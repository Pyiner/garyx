//! Composition root for the long-running gateway process.
//!
//! This module owns the complete startup and shutdown story of `garyx
//! gateway run`: exclusive data-directory ownership, store assembly, the
//! legacy boot import, AppState construction, bridge/provider wiring, channel
//! plugin (re)builds, config hot-reload, listener binding, deferred startup,
//! and the shutdown sequence. The CLI command is a thin shell over
//! [`run`]; `AppStateBuilder` (garyx-gateway) stays the DI constructor for
//! the state object graph and is fed only from here.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use garyx_bridge::MultiProviderBridge;
use garyx_channels::{BuiltInPluginDiscoverer, ChannelPluginManager};
use garyx_gateway::server::{AppState, AppStateBuilder, Gateway};
use garyx_gateway::{CronService, ThreadFileLogger, default_thread_log_dir};
use garyx_models::config::GaryxConfig;
use garyx_models::config_loader::{
    ConfigHotReloadOptions, ConfigHotReloader, ConfigLoadOptions, ConfigRuntimeOverrides,
};
use garyx_models::local_paths::{
    default_session_data_dir, message_ledger_dir_for_data_dir, thread_transcripts_dir_for_data_dir,
};
use garyx_router::{
    MessageLedgerStore, ThreadHistoryRepository, ThreadStore, ThreadTranscriptStore,
};

use crate::commands::VERSION;
use crate::config_support::{default_config_path_buf, load_config_or_default, print_diagnostics};

pub struct RuntimeAssembly {
    pub state: Arc<AppState>,
    pub bridge: Arc<MultiProviderBridge>,
    pub cron_service: Arc<CronService>,
}

pub struct RuntimeAssembler {
    config_path: PathBuf,
    config: GaryxConfig,
}

impl RuntimeAssembler {
    pub fn new(config_path: impl AsRef<Path>, config: GaryxConfig) -> Self {
        Self {
            config_path: config_path.as_ref().to_path_buf(),
            config,
        }
    }

    pub async fn assemble(self) -> Result<RuntimeAssembly, Box<dyn std::error::Error>> {
        let session_data_dir = self
            .config
            .sessions
            .data_dir
            .clone()
            .unwrap_or_else(|| default_session_data_dir().to_string_lossy().to_string());

        // Take exclusive ownership of the data directory before opening any
        // other persistent runtime store. GaryxDbService holds the lock for
        // the lifetime of AppState and performs schema initialization only
        // after the pre-R5 parent handoff barrier has cleared.
        let garyx_db = Arc::new(garyx_gateway::garyx_db::GaryxDbService::open(
            garyx_models::local_paths::garyx_database_path_for_data_dir(Path::new(
                &session_data_dir,
            )),
        )?);
        let transcript_root = thread_transcripts_dir_for_data_dir(Path::new(&session_data_dir));
        let transcript_store = Arc::new(ThreadTranscriptStore::file(&transcript_root).await?);
        let message_ledger = Arc::new(
            MessageLedgerStore::file(message_ledger_dir_for_data_dir(Path::new(
                &session_data_dir,
            )))
            .await?,
        );

        let bridge = Arc::new(MultiProviderBridge::new());

        // Thread records live in SQLite, full stop (#TASK-1864). The file
        // archive survives only as the one-shot boot-import source for
        // upgrades from pre-SQLite installs; there is no runtime file mode
        // and no dual-write mirror. Emergency recovery = the archived
        // backups plus a fresh boot import, not a mode switch.
        tracing::info!("thread store backend: sqlite");
        // One-shot migration of the retired file-based task counter into the
        // SQLite allocator row (no-op once the row exists).
        garyx_gateway::seed_task_counter_from_legacy(&garyx_db, Path::new(&session_data_dir));
        let thread_store: Arc<dyn ThreadStore> = garyx_gateway::assemble_sqlite_thread_store(
            garyx_db.clone(),
            transcript_store.clone(),
            &bridge,
        )?;
        garyx_gateway::run_legacy_boot_import(
            &garyx_db,
            &thread_store,
            &transcript_store,
            Path::new(&session_data_dir),
        )
        .await?;
        let thread_history = Arc::new(ThreadHistoryRepository::new(
            thread_store.clone(),
            transcript_store,
        ));

        let (event_tx, _) = tokio::sync::broadcast::channel(128);

        let mut cron_service_raw = CronService::new(PathBuf::from(&session_data_dir));
        let cron_boot_config = self.config.cron.clone();
        match cron_service_raw.load(&cron_boot_config).await {
            Ok(()) => {
                cron_service_raw.start();
                tracing::info!(
                    configured_jobs = cron_boot_config.jobs.len(),
                    "Cron service started"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "Failed to initialize cron service");
            }
        }
        let cron_service = Arc::new(cron_service_raw);

        let restart_tokens = parse_restart_tokens_from_env();
        if !restart_tokens.is_empty() {
            tracing::info!(
                count = restart_tokens.len(),
                "Restart auth tokens loaded from GARYX_RESTART_TOKENS"
            );
        }
        let thread_logs = Arc::new(ThreadFileLogger::new(default_thread_log_dir()));

        let builder = AppStateBuilder::new(self.config.clone())
            .with_persistent_local_stores(garyx_db.clone());
        let state = builder
            .with_thread_store(thread_store.clone())
            .with_thread_history(thread_history.clone())
            .with_message_ledger(message_ledger)
            .with_bridge(bridge.clone())
            .with_provider_runtime_ready(false)
            .with_event_tx(event_tx)
            .with_cron_service(cron_service.clone())
            .with_config_path(self.config_path)
            .with_restart_tokens(restart_tokens)
            .with_thread_log_sink(thread_logs.clone())
            .build();

        // Crash recovery is a destructive projection update and must finish
        // under the data-dir lock before Gateway binds its listener. It must
        // not allocate new activity ordering: completed runs stay in place.
        state.ops.garyx_db.clear_stale_active_runs()?;
        let recovered_inputs = state.ops.garyx_db.recover_orphaned_pending_user_inputs()?;
        if recovered_inputs > 0 {
            tracing::info!(
                recovered_inputs,
                "settled orphaned queued user inputs during startup"
            );
        }
        let lifecycle_cutoff = (chrono::Utc::now() - chrono::Duration::days(7))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let (pruned_operations, pruned_cleanup_jobs) = state
            .ops
            .garyx_db
            .prune_lifecycle_history(&lifecycle_cutoff)?;
        if pruned_operations > 0 || pruned_cleanup_jobs > 0 {
            tracing::info!(
                pruned_operations,
                pruned_cleanup_jobs,
                "pruned expired lifecycle operation history"
            );
        }

        // Bind the bridge to AppState's final SQLite store handle so provider
        // persistence, routing, and history all share the same truth source.
        bridge
            .set_thread_store(state.threads.thread_store.clone())
            .await;
        bridge.set_thread_history(state.threads.history.clone());
        bridge.set_event_tx(state.ops.events.sender()).await;
        cron_service
            .set_dispatch_runtime(
                state.threads.thread_store.clone(),
                state.threads.router.clone(),
                bridge.clone(),
                state.channel_dispatcher(),
                state.ops.thread_logs.clone(),
                self.config.mcp_servers.clone(),
                state.ops.custom_agents.clone(),
            )
            .await;
        Ok(RuntimeAssembly {
            state,
            bridge,
            cron_service,
        })
    }
}

fn parse_restart_tokens_from_env() -> Vec<String> {
    std::env::var("GARYX_RESTART_TOKENS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn routing_rebuild_channels(config: &GaryxConfig) -> Vec<String> {
    let mut channels: Vec<String> = Vec::new();

    if config.channels.api.accounts.values().any(|acc| acc.enabled) {
        channels.push("api".to_owned());
    }
    for (plugin_id, plugin_cfg) in &config.channels.plugins {
        if plugin_cfg.accounts.values().any(|entry| entry.enabled) {
            channels.push(plugin_id.clone());
        }
    }

    channels
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

pub(crate) async fn rebuild_channel_plugins(
    plugin_manager: &std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
    config: &GaryxConfig,
    state: &Arc<AppState>,
    bridge: &Arc<MultiProviderBridge>,
    no_channels: bool,
) -> Result<(), String> {
    rebuild_channel_plugins_with_factory(plugin_manager, config, state, bridge, no_channels, None)
        .await
}

pub(crate) type RebuildDiscovererFactory = Box<
    dyn FnOnce(
            Arc<dyn garyx_channels::MeetingEventSink>,
        ) -> Box<dyn garyx_channels::plugin::PluginDiscoverer>
        + Send,
>;

pub(crate) async fn rebuild_channel_plugins_with_factory(
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

pub(crate) async fn run(
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
