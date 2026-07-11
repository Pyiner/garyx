use super::*;

#[cfg(test)]
pub(crate) fn routing_rebuild_channels(config: &GaryxConfig) -> Vec<String> {
    crate::runtime_assembler::routing_rebuild_channels(config)
}

fn register_plugin_state_logging(plugin_manager: &mut ChannelPluginManager) {
    plugin_manager.register_state_hook(|status| {
        tracing::info!(
            plugin_id = %status.metadata.id,
            state = ?status.state,
            source = %status.metadata.source,
            error = status.last_error.as_deref().unwrap_or(""),
            "channel plugin state changed"
        );
    });
}

async fn rebuild_channel_plugins(
    plugin_manager: &std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
    config: &GaryxConfig,
    state: &Arc<AppState>,
    bridge: &Arc<MultiProviderBridge>,
    no_channels: bool,
) -> Result<(), String> {
    {
        let mut manager = plugin_manager.lock().await;
        manager.stop_all().await;
        manager.cleanup_all().await;

        let mut replacement = ChannelPluginManager::new();
        register_plugin_state_logging(&mut replacement);
        // Attach the production `SwappableDispatcher` so subprocess
        // plugins registered below can fork-and-store into it. Built-in
        // plugins go through `BuiltInPluginDiscoverer` instead and do
        // not need this handle.
        replacement.attach_dispatcher(state.channel_dispatcher_swap());

        if !no_channels {
            let built_in_discoverer = BuiltInPluginDiscoverer::with_dispatcher(
                config.channels.clone(),
                state.threads.router.clone(),
                bridge.clone(),
                state.channel_dispatcher(),
                config.gateway.public_url.clone(),
            );
            replacement.discover_and_register(&built_in_discoverer)?;

            replacement.initialize_all().await;
            replacement.start_all().await;
        }

        *manager = replacement;
    }

    // Subprocess plugins go through a separate discovery path
    // (`plugin.toml` manifests + preflight + `register_subprocess_plugin`).
    // Done outside the manager lock above because
    // `register_manifest_plugins` itself re-acquires the lock per
    // plugin — async `register_subprocess_plugin` spawns a child and
    // runs the initialize/start handshake under that lock.
    if !no_channels {
        crate::channel_plugin_host::register_manifest_plugins(
            plugin_manager,
            config,
            VERSION,
            crate::channel_plugin_host::HostDeps {
                router: state.threads.router.clone(),
                bridge: bridge.clone(),
                swap: state.channel_dispatcher_swap(),
                // Weak handle so the `request_self_replace` host
                // RPC can drive respawn after a successful swap
                // without forming a manager↔handler reference
                // cycle that would survive gateway shutdown.
                plugin_manager: std::sync::Arc::downgrade(plugin_manager),
                // Plugin-side master kill switch is the existing
                // `plugins.auto_update` config field. Captured here
                // (one-shot read) so handlers see the right
                // initial value; future hot-reload paths flip the
                // AtomicBool without rebuilding handlers.
                plugin_auto_update_enabled: std::sync::Arc::new(
                    std::sync::atomic::AtomicBool::new(config.plugins.auto_update),
                ),
            },
        )
        .await;
    }

    Ok(())
}

async fn initialize_bridge_runtime(
    state: &Arc<AppState>,
    bridge: &Arc<MultiProviderBridge>,
    config: &GaryxConfig,
) -> Result<(), String> {
    bridge
        .replace_agent_profiles(state.ops.custom_agents.list_agents().await)
        .await;
    bridge
        .reload_from_config(config)
        .await
        .map_err(|error| error.to_string())?;
    tracing::info!("MultiProviderBridge initialized");
    Ok(())
}

fn spawn_deferred_gateway_startup(
    state: Arc<AppState>,
    bridge: Arc<MultiProviderBridge>,
    plugin_manager: std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
    no_channels: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        tracing::info!("deferred gateway startup started");

        let (gateway_auto_update, provider_ready) = {
            let _settings_guard = state.ops.settings_mutex.lock().await;
            let config = state.config_snapshot();

            let bridge_init_result = initialize_bridge_runtime(&state, &bridge, &config).await;
            match bridge_init_result {
                Ok(()) => state.mark_provider_runtime_ready(),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "Bridge init failed during deferred startup; continuing with current provider pool"
                    );
                }
            }

            if let Err(error) =
                rebuild_channel_plugins(&plugin_manager, &config, &state, &bridge, no_channels)
                    .await
            {
                tracing::warn!(
                    error = %error,
                    "Failed to initialize channel plugins during deferred startup"
                );
            }

            (
                config.gateway.auto_update.clone(),
                state.provider_runtime_ready(),
            )
        };

        if provider_ready {
            garyx_gateway::restart_wake::drain_pending_restart_wakes(state.clone()).await;
        } else {
            tracing::warn!("skipping restart wake drain because provider runtime is not ready");
        }

        // Architecture C: host no longer runs a periodic plugin
        // update loop. Each plugin owns its own timer + advertised-
        // version source and sends `request_self_replace` reverse
        // RPCs when it decides to upgrade; the host responds with
        // the safe-swap pipeline (idle gate + atomic rename +
        // respawn) inside `plugin_self_replace::handle`. The
        // `plugins.auto_update` config flag now controls whether
        // those RPCs are accepted at all, gated per-call rather
        // than at spawn-time.

        // Spawn the gateway self-updater after the listener is up; update
        // checks are integration work, not control-plane readiness work.
        let _gateway_auto_update_handle =
            crate::gateway_auto_update::spawn(plugin_manager.clone(), gateway_auto_update);

        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "deferred gateway startup completed"
        );
    })
}

pub(crate) async fn run_gateway(
    config_path: &str,
    port_override: Option<u16>,
    host_override: Option<String>,
    no_channels: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Garyx v{} starting...", VERSION);

    let loaded = load_config_or_default(
        config_path,
        ConfigRuntimeOverrides {
            gateway_host: host_override.clone(),
            gateway_port: port_override,
        },
    )?;
    print_diagnostics(&loaded.diagnostics);
    let config_path = loaded.path;
    let config = loaded.config;
    tracing::info!("Loading config from {}", config_path.display());

    let host = config.gateway.host.clone();
    let port = config.gateway.port;

    let RuntimeAssembly {
        state,
        bridge,
        cron_service,
    } = RuntimeAssembler::new(&config_path, config.clone())
        .assemble()
        .await?;

    // 3.0 Share the channel plugin manager with AppState so HTTP
    // endpoints (`GET /api/channels/plugins`) see the same
    // registrations the boot path creates. Single source of truth.
    let plugin_manager = state.channel_plugin_manager();
    register_plugin_state_logging(&mut *plugin_manager.lock().await);

    // 3.1 Start hot reload watcher (best effort)
    let hot_reloader = {
        let options = ConfigHotReloadOptions {
            poll_interval: Duration::from_millis(500),
            debounce: Duration::from_millis(400),
            load_options: ConfigLoadOptions {
                default_path: default_config_path_buf(),
                runtime_overrides: ConfigRuntimeOverrides::default(),
            },
        };

        let reloader = ConfigHotReloader::start(config_path.clone(), config.clone(), options);

        let state_cb = state.clone();
        let bridge_cb = bridge.clone();
        let plugin_manager_cb = plugin_manager.clone();
        let tokio_handle = tokio::runtime::Handle::current();
        reloader.register_callback(move |new_config, diagnostics| {
            print_diagnostics(&diagnostics);
            let state = state_cb.clone();
            let bridge = bridge_cb.clone();
            let plugin_manager = plugin_manager_cb.clone();
            tokio_handle.spawn(async move {
                let _settings_guard = state.ops.settings_mutex.lock().await;
                if let Err(error) = state.apply_runtime_config(new_config.clone()).await {
                    tracing::warn!(
                        error = %error,
                        "Failed to fully apply hot-reloaded config"
                    );
                    return;
                }

                if let Err(error) = rebuild_channel_plugins(
                    &plugin_manager,
                    &new_config,
                    &state,
                    &bridge,
                    no_channels,
                )
                .await
                {
                    tracing::warn!(
                        error = %error,
                        "Failed to rebuild channel plugins after hot-reloaded config"
                    );
                }
            });
        });

        reloader
    };

    // 4. Start the HTTP control plane first. Provider reconciliation,
    // channel/plugin startup, restart-wake drain, and update checks are kicked
    // off only after the listener is bound so slow external integrations cannot
    // make the gateway appear dead during cold start.
    let deferred_startup = Arc::new(std::sync::Mutex::new(None));
    let run_result: Result<(), Box<dyn std::error::Error>> = async {
        let gateway = Gateway::new(state);
        let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
        let startup_slot = deferred_startup.clone();
        let startup_state = gateway.state().clone();
        let startup_bridge = bridge.clone();
        let startup_plugin_manager = plugin_manager.clone();
        let shutdown_slot = deferred_startup.clone();
        gateway
            .serve_with_lifecycle_hooks(
                addr,
                move || {
                    let handle = spawn_deferred_gateway_startup(
                        startup_state,
                        startup_bridge,
                        startup_plugin_manager,
                        no_channels,
                    );
                    *startup_slot
                        .lock()
                        .expect("deferred startup handle lock poisoned") = Some(handle);
                },
                move || {
                    let guard = shutdown_slot
                        .lock()
                        .expect("deferred startup handle lock poisoned");
                    if let Some(handle) = guard.as_ref()
                        && !handle.is_finished()
                    {
                        handle.abort();
                    }
                },
            )
            .await?;
        Ok(())
    }
    .await;

    let metrics = hot_reloader.metrics();
    tracing::info!(
        attempts = metrics.attempts,
        successes = metrics.successes,
        failures = metrics.failures,
        callback_notifications = metrics.callback_notifications,
        "config hot-reload metrics"
    );
    drop(hot_reloader);

    let deferred_startup_handle = deferred_startup
        .lock()
        .expect("deferred startup handle lock poisoned")
        .take();
    if let Some(handle) = deferred_startup_handle {
        if !handle.is_finished() {
            handle.abort();
        }
        let _ = handle.await;
    }

    // Always run shutdown sequence, even when startup/serve fails.
    //
    {
        let mut plugin_manager = plugin_manager.lock().await;
        plugin_manager.stop_all().await;
        plugin_manager.cleanup_all().await;
    }

    match Arc::try_unwrap(cron_service) {
        Ok(mut svc) => svc.stop().await,
        Err(_) => tracing::debug!("Cron service still has outstanding references on shutdown"),
    }
    run_result
}

pub(crate) async fn cmd_status(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let port = config.gateway.port;
    let host = if config.gateway.host == "0.0.0.0" {
        "127.0.0.1"
    } else {
        &config.gateway.host
    };
    let url = format!("http://{}:{}/health", host, port);

    let running = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .is_ok();

    if json {
        let obj = serde_json::json!({
            "running": running,
            "host": config.gateway.host,
            "port": port,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        let status = if running { "running" } else { "not running" };
        println!("Gateway: {} ({}:{})", status, config.gateway.host, port);
    }
    Ok(())
}

pub(crate) async fn cmd_gateway_reload_config(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let response = post_gateway_json(&gateway, "/api/settings/reload", &json!({})).await?;
    print_pretty_json(&response)?;
    Ok(())
}

/// Best-effort reload trigger for CLI flows that mutate the on-disk config
/// (channel add/login). The TOML is written atomically before we get here, so
/// a running gateway just needs to re-read it. Silent when the gateway isn't
/// up — that's the normal "configure before starting" case.
pub(super) async fn notify_gateway_reload(config_path: &Path) {
    notify_gateway_reload_with_output(config_path, true).await;
}

pub(super) async fn notify_gateway_reload_quiet(config_path: &Path) {
    notify_gateway_reload_with_output(config_path, false).await;
}

pub(super) async fn gateway_is_reachable(config_path: &Path) -> bool {
    let path_str = config_path.to_string_lossy();
    let Ok(gateway) = gateway_endpoint(&path_str) else {
        return false;
    };
    let url = format!("{}/health", gateway.base_url);
    let response = gateway_request(
        reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(5)),
        &gateway,
    )
    .send()
    .await;
    matches!(response, Ok(r) if r.status().is_success())
}

async fn notify_gateway_reload_with_output(config_path: &Path, print_success: bool) {
    let path_str = config_path.to_string_lossy();
    let Ok(gateway) = gateway_endpoint(&path_str) else {
        return;
    };
    let url = format!("{}/api/settings/reload", gateway.base_url);
    let response = gateway_request(
        reqwest::Client::new()
            .post(&url)
            .json(&json!({}))
            .timeout(std::time::Duration::from_secs(5)),
        &gateway,
    )
    .send()
    .await;
    match response {
        Ok(r) if r.status().is_success() => {
            if print_success {
                println!("Gateway config reloaded");
            }
        }
        Ok(r) => {
            eprintln!("warning: gateway reload returned HTTP {}", r.status());
        }
        Err(e) if e.is_connect() || e.is_timeout() => {}
        Err(e) => {
            eprintln!("warning: gateway reload failed: {e}");
        }
    }
}

pub(crate) async fn cmd_gateway_token(
    config_path: &str,
    rotate: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    let config_path = prepared.active_path;
    let loaded = load_config_or_default(
        config_path
            .to_str()
            .ok_or("config path must be valid UTF-8")?,
        ConfigRuntimeOverrides::default(),
    )?;
    let mut config = loaded.config;
    let existing = config.gateway.auth_token.trim().to_owned();

    let (token, changed) = if !existing.is_empty() && !rotate {
        (existing, false)
    } else {
        let token = format!("gx_{}", Uuid::new_v4().simple());
        config.gateway.auth_token = token.clone();
        let value = serde_json::to_value(&config)?;
        write_config_value_atomic(&config_path, &value, &ConfigWriteOptions::default())?;
        (token, true)
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "token": token,
                "changed": changed,
                "config_path": config_path.display().to_string(),
            }))?
        );
    } else {
        println!("{token}");
    }
    Ok(())
}

pub(crate) async fn cmd_gateway_install(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = build_service_spec(config_path)?;
    let manager = crate::service_manager::active_manager()?;
    let report = manager.install(&spec)?;
    crate::service_manager::wait_for_port(spec.port, Duration::from_secs(30)).await?;
    println!(
        "Gateway service installed via {}: {} (port {})",
        report.backend,
        report.unit_path.display(),
        spec.port
    );
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
    Ok(())
}

pub(crate) async fn cmd_gateway_uninstall() -> Result<(), Box<dyn std::error::Error>> {
    let manager = crate::service_manager::active_manager()?;
    manager.uninstall()?;
    println!("Gateway service uninstalled via {}", manager.backend_name());
    Ok(())
}

pub(crate) async fn cmd_gateway_start(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let manager = crate::service_manager::active_manager()?;
    if !manager.is_installed() {
        return Err(format!(
            "gateway service is not installed — run `garyx gateway install` first ({} backend)",
            manager.backend_name()
        )
        .into());
    }
    manager.start()?;
    crate::service_manager::wait_for_port(config.gateway.port, Duration::from_secs(30)).await?;
    println!(
        "Gateway service started via {} (port {})",
        manager.backend_name(),
        config.gateway.port
    );
    Ok(())
}

pub(crate) async fn cmd_gateway_restart(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = build_service_spec(config_path)?;
    let manager = crate::service_manager::active_manager()?;
    let report = manager.restart(&spec)?;
    crate::service_manager::wait_for_port(spec.port, Duration::from_secs(30)).await?;
    println!(
        "Gateway service restarted via {}: {} (port {})",
        report.backend,
        report.unit_path.display(),
        spec.port
    );
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
    Ok(())
}

pub(crate) async fn cmd_gateway_stop() -> Result<(), Box<dyn std::error::Error>> {
    let manager = crate::service_manager::active_manager()?;
    manager.stop()?;
    println!("Gateway service stopped via {}", manager.backend_name());
    Ok(())
}

/// Resolve every input the platform backend needs to render a unit / plist.
fn build_service_spec(
    config_path: &str,
) -> Result<crate::service_manager::ServiceSpec, Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let log_dir = crate::service_manager::log_dir_path()?;
    let binary_path = std::env::current_exe()?;
    Ok(crate::service_manager::ServiceSpec {
        binary_path,
        host: config.gateway.host.clone(),
        port: config.gateway.port,
        log_dir,
        workspace_root: detect_workspace_root(),
    })
}

/// If the CLI was invoked from a garyx repo checkout, return its root so the
/// managed service can inherit local development context when needed.
fn detect_workspace_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("Cargo.toml").exists() && cwd.join("garyx").is_dir() {
        Some(cwd)
    } else {
        None
    }
}
