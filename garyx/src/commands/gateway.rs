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
    rebuild_channel_plugins_with_factory(plugin_manager, config, state, bridge, no_channels, None)
        .await
}

type RebuildDiscovererFactory = Box<
    dyn FnOnce(
            Arc<dyn garyx_channels::MeetingEventSink>,
        ) -> Box<dyn garyx_channels::plugin::PluginDiscoverer>
        + Send,
>;

async fn rebuild_channel_plugins_with_factory(
    plugin_manager: &std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
    config: &GaryxConfig,
    state: &Arc<AppState>,
    bridge: &Arc<MultiProviderBridge>,
    no_channels: bool,
    discoverer_factory: Option<RebuildDiscovererFactory>,
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
            state
                .ops
                .meetings
                .start_ingestion(config.gateway.meetings.effective_join_retry_window_secs());
            {
                let meeting_sink: Arc<dyn garyx_channels::MeetingEventSink> =
                    state.ops.meetings.clone();
                let discoverer: Box<dyn garyx_channels::plugin::PluginDiscoverer> =
                    if let Some(factory) = discoverer_factory {
                        factory(meeting_sink)
                    } else {
                        Box::new(BuiltInPluginDiscoverer::with_dispatcher_and_meeting_sink(
                            config.channels.clone(),
                            state.threads.router.clone(),
                            bridge.clone(),
                            state.channel_dispatcher(),
                            config.gateway.public_url.clone(),
                            meeting_sink,
                        ))
                    };
                replacement.discover_and_register(discoverer.as_ref())?;
            }

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
        .replace_agent_profiles(state.ops.custom_agents.snapshot().await)
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

#[derive(Debug)]
struct RestartWakeAllSnapshotSelection {
    snapshot: garyx_gateway::restart_wake::RestartWakeAllSnapshot,
    gateway_fallback_reason: Option<String>,
}

fn configured_session_data_dir(config: &GaryxConfig) -> PathBuf {
    config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(default_session_data_dir)
}

async fn restart_wake_all_snapshot_for_config(
    config_path: &str,
) -> Result<RestartWakeAllSnapshotSelection, Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let data_dir = configured_session_data_dir(&loaded.config);
    let gateway = gateway_endpoint(config_path)?;
    let gateway_snapshot = fetch_gateway_json(
        &gateway,
        garyx_gateway::restart_wake::RESTART_WAKE_ALL_SNAPSHOT_PATH,
    )
    .await
    .and_then(|payload| {
        serde_json::from_value(payload).map_err(|error| Box::<dyn std::error::Error>::from(error))
    });

    match gateway_snapshot {
        Ok(snapshot) => Ok(RestartWakeAllSnapshotSelection {
            snapshot,
            gateway_fallback_reason: None,
        }),
        Err(gateway_error) => {
            let snapshot =
                garyx_gateway::restart_wake::restart_wake_all_snapshot_from_data_dir(&data_dir)
                    .map_err(|local_error| {
                        format!(
                            "failed to capture restart wake-all snapshot from gateway ({gateway_error}) or read-only local database {} ({local_error})",
                            data_dir.display()
                        )
                    })?;
            Ok(RestartWakeAllSnapshotSelection {
                snapshot,
                gateway_fallback_reason: Some(gateway_error.to_string()),
            })
        }
    }
}

pub(crate) async fn cmd_queue_gateway_restart_wake_all(
    config_path: &str,
    message: &str,
) -> Result<garyx_gateway::restart_wake::QueuedRestartWakeAll, Box<dyn std::error::Error>> {
    let selection = restart_wake_all_snapshot_for_config(config_path).await?;
    if let Some(reason) = selection.gateway_fallback_reason.as_deref() {
        eprintln!(
            "warning: gateway restart-wake snapshot unavailable ({reason}); using configured data_dir through a read-only SQLite connection"
        );
    }
    garyx_gateway::restart_wake::queue_pending_restart_wake_all(message, selection.snapshot)
}

pub(crate) async fn cmd_gateway_stop() -> Result<(), Box<dyn std::error::Error>> {
    let manager = crate::service_manager::active_manager()?;
    manager.stop()?;
    println!("Gateway service stopped via {}", manager.backend_name());
    Ok(())
}

pub(crate) fn cmd_gateway_rotate_store_incarnation(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let data_dir = config
        .sessions
        .data_dir
        .map(PathBuf::from)
        .unwrap_or_else(default_session_data_dir);
    let database_path = garyx_models::local_paths::garyx_database_path_for_data_dir(&data_dir);
    // GaryxDbService::open owns the data-dir lock, so a running gateway makes
    // this command fail instead of rotating beneath live requests.
    let database = garyx_gateway::garyx_db::GaryxDbService::open(&database_path)?;
    let previous = database.store_incarnation_id()?;
    let rotated = database.rotate_store_incarnation()?;
    println!(
        "Rotated store incarnation for {}: {} -> {}",
        data_dir.display(),
        previous,
        rotated.store_incarnation_id
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{ENV_LOCK, ScopedEnvVar};
    use axum::routing::get;
    use axum::{Json, Router, http::StatusCode};
    use garyx_channels::plugin::{ManagedChannelPlugin, PluginDiscoverer};
    use garyx_channels::{
        Channel, ChannelError, JoinedMeeting, MeetingApiError, MeetingInvite, MeetingPlatformClient,
    };
    use garyx_gateway::garyx_db::{GaryxDbError, GaryxDbService, MeetingRecord, RecentThreadDraft};
    use garyx_gateway::server::AppStateBuilder;
    use garyx_models::local_paths::garyx_database_path_for_data_dir;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    #[test]
    fn rotate_store_incarnation_command_uses_configured_data_dir() {
        let temp = tempdir().expect("temp dir");
        let custom_data_dir = temp.path().join("restored-data");
        let config_path = temp.path().join("garyx.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "sessions": {"data_dir": custom_data_dir}
            }))
            .unwrap(),
        )
        .unwrap();
        let database_path = garyx_database_path_for_data_dir(&custom_data_dir);
        let database = GaryxDbService::open(&database_path).expect("seed restored database");
        let before = database.store_incarnation_id().unwrap();
        drop(database);

        cmd_gateway_rotate_store_incarnation(config_path.to_str().unwrap())
            .expect("offline rotate command");
        let reopened = GaryxDbService::open(&database_path).expect("reopen rotated database");
        assert_ne!(reopened.store_incarnation_id().unwrap(), before);
        assert!(custom_data_dir.join("garyx.lock").exists());
    }

    #[test]
    fn rotate_store_incarnation_command_refuses_a_live_gateway_lock() {
        let _env_guard = ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("temp dir");
        let custom_data_dir = temp.path().join("live-data");
        let config_path = temp.path().join("garyx.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "sessions": {"data_dir": custom_data_dir}
            }))
            .unwrap(),
        )
        .unwrap();
        let database_path = garyx_database_path_for_data_dir(&custom_data_dir);
        let live_gateway = GaryxDbService::open(&database_path).expect("live gateway owns lock");
        let before = live_gateway.store_incarnation_id().unwrap();
        let _wait = ScopedEnvVar::set_string("GARYX_DATA_LOCK_WAIT_SECS", "0");

        let error = cmd_gateway_rotate_store_incarnation(config_path.to_str().unwrap())
            .expect_err("online rotate must fail on the data-dir lock");
        assert!(matches!(
            error.downcast_ref::<GaryxDbError>(),
            Some(GaryxDbError::DataDirLocked { .. })
        ));
        assert_eq!(
            live_gateway.store_incarnation_id().unwrap(),
            before,
            "failed online rotate must not mutate store identity"
        );
    }

    struct ReloadMeetingClient {
        hang: bool,
        join_calls: AtomicUsize,
    }

    #[async_trait]
    impl MeetingPlatformClient for ReloadMeetingClient {
        async fn join(
            &self,
            _meeting_no: &str,
            _password: Option<&str>,
        ) -> Result<JoinedMeeting, MeetingApiError> {
            let call = self.join_calls.fetch_add(1, Ordering::AcqRel) + 1;
            if self.hang {
                return std::future::pending().await;
            }
            Ok(JoinedMeeting {
                feishu_meeting_id: format!("synthetic-reload-meeting-{call}"),
            })
        }

        async fn leave(&self, _feishu_meeting_id: &str) -> Result<(), MeetingApiError> {
            Ok(())
        }
    }

    struct ReloadProbeChannel {
        label: String,
        account_id: String,
        sink: Arc<dyn garyx_channels::MeetingEventSink>,
        client: Arc<ReloadMeetingClient>,
        invite_on_start: MeetingInvite,
        lifecycle: Arc<std::sync::Mutex<Vec<String>>>,
        running: bool,
    }

    #[async_trait]
    impl Channel for ReloadProbeChannel {
        fn name(&self) -> &str {
            "feishu"
        }

        async fn start(&mut self) -> Result<(), ChannelError> {
            self.sink
                .register_client(&self.account_id, self.client.clone());
            self.lifecycle
                .lock()
                .expect("lifecycle")
                .push(format!("{}:register", self.label));
            self.sink.on_meeting_invited(self.invite_on_start.clone());
            self.running = true;
            Ok(())
        }

        async fn stop(&mut self) -> Result<(), ChannelError> {
            if self.running {
                self.sink.unregister_client(&self.account_id);
                self.lifecycle
                    .lock()
                    .expect("lifecycle")
                    .push(format!("{}:unregister", self.label));
                self.running = false;
            }
            Ok(())
        }

        fn is_running(&self) -> bool {
            self.running
        }
    }

    struct ReloadProbeDiscoverer {
        label: String,
        account_id: String,
        sink: Arc<dyn garyx_channels::MeetingEventSink>,
        client: Arc<ReloadMeetingClient>,
        invite_on_start: MeetingInvite,
        lifecycle: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl PluginDiscoverer for ReloadProbeDiscoverer {
        fn discover(&self) -> Result<Vec<Box<dyn garyx_channels::plugin::ChannelPlugin>>, String> {
            let metadata =
                garyx_channels::builtin_plugin_metadata("feishu").expect("Feishu plugin metadata");
            let channel = ReloadProbeChannel {
                label: self.label.clone(),
                account_id: self.account_id.clone(),
                sink: self.sink.clone(),
                client: self.client.clone(),
                invite_on_start: self.invite_on_start.clone(),
                lifecycle: self.lifecycle.clone(),
                running: false,
            };
            Ok(vec![Box::new(ManagedChannelPlugin::new(
                metadata,
                Box::new(channel),
            ))])
        }
    }

    fn reload_probe_factory(
        label: &str,
        account_id: &str,
        client: Arc<ReloadMeetingClient>,
        invite_on_start: MeetingInvite,
        lifecycle: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> RebuildDiscovererFactory {
        let label = label.to_owned();
        let account_id = account_id.to_owned();
        Box::new(move |sink| {
            Box::new(ReloadProbeDiscoverer {
                label,
                account_id,
                sink,
                client,
                invite_on_start,
                lifecycle,
            })
        })
    }

    fn reload_invite(account_id: &str, event_id: &str, meeting_no: &str) -> MeetingInvite {
        MeetingInvite {
            account_id: account_id.to_owned(),
            event_id: event_id.to_owned(),
            meeting_reference_id: format!("reference-{event_id}"),
            meeting_no: meeting_no.to_owned(),
            topic: format!("Synthetic reload {meeting_no}"),
            bot_id: "bot_1000000001".to_owned(),
            inviter_id: "user_1000000001".to_owned(),
        }
    }

    async fn wait_for_reload_meeting(
        db: &GaryxDbService,
        predicate: impl Fn(&MeetingRecord) -> bool,
    ) -> MeetingRecord {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(record) = db
                    .list_all_meetings()
                    .expect("list meetings")
                    .into_iter()
                    .find(|record| predicate(record))
                {
                    return record;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("meeting lifecycle converged")
    }

    fn seed_running_thread(data_dir: &Path, thread_id: &str) {
        let database = GaryxDbService::open(garyx_database_path_for_data_dir(data_dir))
            .expect("open test database");
        database
            .upsert_recent_thread(RecentThreadDraft {
                thread_id: thread_id.to_owned(),
                title: thread_id.to_owned(),
                workspace_dir: None,
                thread_type: "chat".to_owned(),
                provider_type: None,
                agent_id: None,
                message_count: 0,
                last_message_preview: String::new(),
                recent_run_id: None,
                active_run_id: None,
                run_state: "running".to_owned(),
                updated_at: None,
                last_active_at: "2026-07-14T00:00:00Z".to_owned(),
            })
            .expect("seed running thread");
    }

    async fn spawn_restart_snapshot_server(
        status: StatusCode,
        payload: Value,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route(
            garyx_gateway::restart_wake::RESTART_WAKE_ALL_SNAPSHOT_PATH,
            get(move || {
                let payload = payload.clone();
                async move { (status, Json(payload)) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind snapshot server");
        let address = listener.local_addr().expect("snapshot server address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve snapshot API");
        });
        (format!("http://{address}"), handle)
    }

    fn write_restart_test_config(
        temp: &tempfile::TempDir,
        public_url: &str,
        data_dir: &Path,
    ) -> PathBuf {
        let config_path = temp.path().join("garyx.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "gateway": {"public_url": public_url},
                "sessions": {"data_dir": data_dir},
            }))
            .expect("serialize config"),
        )
        .expect("write config");
        config_path
    }

    #[tokio::test]
    async fn rebuild_channel_plugins_unregisters_then_registers_and_nudges_real_meeting_sink() {
        let temp = tempdir().expect("temp dir");
        let db = Arc::new(GaryxDbService::memory().expect("database"));
        let bridge = Arc::new(MultiProviderBridge::new());
        let config = GaryxConfig::default();
        let state = AppStateBuilder::new(config.clone())
            .with_garyx_db(db.clone())
            .with_meetings_dir(temp.path().join("meetings"))
            .with_bridge(bridge.clone())
            .build();
        let plugin_manager = state.channel_plugin_manager();
        let lifecycle = Arc::new(std::sync::Mutex::new(Vec::new()));
        let account_id = "synthetic-reload-account";

        let old_client = Arc::new(ReloadMeetingClient {
            hang: true,
            join_calls: AtomicUsize::new(0),
        });
        rebuild_channel_plugins_with_factory(
            &plugin_manager,
            &config,
            &state,
            &bridge,
            false,
            Some(reload_probe_factory(
                "old",
                account_id,
                old_client.clone(),
                reload_invite(account_id, "evt-reload-existing", "111111111"),
                lifecycle.clone(),
            )),
        )
        .await
        .expect("initial probe channel build");
        let existing = wait_for_reload_meeting(&db, |record| {
            record.account_id == account_id
                && record.meeting_no == "111111111"
                && record.status == "joining"
        })
        .await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while old_client.join_calls.load(Ordering::Acquire) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("old client join started");

        let new_client = Arc::new(ReloadMeetingClient {
            hang: false,
            join_calls: AtomicUsize::new(0),
        });
        rebuild_channel_plugins_with_factory(
            &plugin_manager,
            &config,
            &state,
            &bridge,
            false,
            Some(reload_probe_factory(
                "new",
                account_id,
                new_client.clone(),
                reload_invite(account_id, "evt-reload-fresh", "222222222"),
                lifecycle.clone(),
            )),
        )
        .await
        .expect("hot-reload probe channel build");

        let resumed = wait_for_reload_meeting(&db, |record| {
            record.id == existing.id && record.status == "live"
        })
        .await;
        let fresh = wait_for_reload_meeting(&db, |record| {
            record.account_id == account_id
                && record.meeting_no == "222222222"
                && record.status == "live"
        })
        .await;
        assert_ne!(resumed.id, fresh.id);
        assert_eq!(old_client.join_calls.load(Ordering::Acquire), 1);
        assert_eq!(new_client.join_calls.load(Ordering::Acquire), 2);
        assert_eq!(
            lifecycle.lock().expect("lifecycle").as_slice(),
            ["old:register", "old:unregister", "new:register"]
        );

        rebuild_channel_plugins(&plugin_manager, &config, &state, &bridge, true)
            .await
            .expect("disable channels after reload test");
        assert_eq!(
            lifecycle.lock().expect("lifecycle").as_slice(),
            [
                "old:register",
                "old:unregister",
                "new:register",
                "new:unregister"
            ]
        );
        state.ops.meetings.shutdown_ingestion();
    }

    #[tokio::test]
    async fn restart_wake_all_prefers_the_running_gateway_snapshot() {
        let temp = tempdir().expect("temp dir");
        let data_dir = temp.path().join("custom-data");
        seed_running_thread(&data_dir, "thread::local-fallback");
        let (base_url, server) = spawn_restart_snapshot_server(
            StatusCode::OK,
            json!({
                "targets": ["thread::gateway-snapshot"],
                "truncated_count": 0,
            }),
        )
        .await;
        let config_path = write_restart_test_config(&temp, &base_url, &data_dir);

        let selection =
            restart_wake_all_snapshot_for_config(config_path.to_str().expect("config path"))
                .await
                .expect("snapshot selection");

        server.abort();
        assert_eq!(selection.snapshot.targets, vec!["thread::gateway-snapshot"]);
        assert!(selection.gateway_fallback_reason.is_none());
    }

    #[tokio::test]
    async fn restart_wake_all_falls_back_to_the_configured_data_dir() {
        let temp = tempdir().expect("temp dir");
        let data_dir = temp.path().join("custom-data");
        seed_running_thread(&data_dir, "thread::custom-data-running");
        let (base_url, server) = spawn_restart_snapshot_server(
            StatusCode::NOT_FOUND,
            json!({"error": "snapshot endpoint unavailable"}),
        )
        .await;
        let config_path = write_restart_test_config(&temp, &base_url, &data_dir);

        let selection =
            restart_wake_all_snapshot_for_config(config_path.to_str().expect("config path"))
                .await
                .expect("read-only fallback snapshot");

        server.abort();
        assert_eq!(
            selection.snapshot.targets,
            vec!["thread::custom-data-running"]
        );
        assert!(selection.gateway_fallback_reason.is_some());
    }
}
