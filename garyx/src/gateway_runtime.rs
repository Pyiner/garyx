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

/// LIFO stack of labeled async teardown actions. Each startup phase registers
/// its inverse as it completes; shutdown drains the stack in reverse
/// registration order, so startup and shutdown can never drift apart.
struct TeardownStack(
    Vec<(
        &'static str,
        Box<dyn FnOnce() -> futures_boxed::BoxFuture + Send>,
    )>,
);

mod futures_boxed {
    pub(super) type BoxFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
}

impl TeardownStack {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn push<F, Fut>(&mut self, label: &'static str, teardown: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.0.push((label, Box::new(move || Box::pin(teardown()))));
    }

    async fn drain(self) -> Vec<&'static str> {
        let mut executed = Vec::with_capacity(self.0.len());
        for (label, teardown) in self.0.into_iter().rev() {
            tracing::debug!(teardown = label, "running shutdown teardown");
            teardown().await;
            executed.push(label);
        }
        executed
    }
}

/// The single production registration point for runtime teardowns. Reversing
/// the registration order here flips the drain order asserted by
/// `shutdown_teardowns_drain_plugins_before_cron`, so startup/shutdown
/// mirroring is pinned through the same path production uses.
fn register_shutdown_teardowns(
    teardown: &mut TeardownStack,
    cron_service: Arc<CronService>,
    plugin_manager: std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
) {
    // Move the assembly's cron handle into its teardown so the drain-time
    // Arc::try_unwrap sees exactly the same outstanding references as the
    // pre-stack shutdown sequence did.
    teardown.push("cron_service", move || async move {
        match Arc::try_unwrap(cron_service) {
            Ok(mut svc) => svc.stop().await,
            Err(_) => tracing::debug!("Cron service still has outstanding references on shutdown"),
        }
    });
    teardown.push("channel_plugins", move || async move {
        let mut plugin_manager = plugin_manager.lock().await;
        plugin_manager.stop_all().await;
        plugin_manager.cleanup_all().await;
    });
}

/// Phase 3 witness: the fully wired state graph. Serving requires this type.
pub struct RuntimeAssembly {
    pub state: Arc<AppState>,
    pub bridge: Arc<MultiProviderBridge>,
    pub cron_service: Arc<CronService>,
}

pub struct RuntimeAssembler {
    config_path: PathBuf,
    config: GaryxConfig,
    no_channels: bool,
}

impl RuntimeAssembler {
    pub fn new(config_path: impl AsRef<Path>, config: GaryxConfig) -> Self {
        Self {
            config_path: config_path.as_ref().to_path_buf(),
            config,
            no_channels: false,
        }
    }

    /// Mirror of the `--no-channels` boot flag: threaded into the
    /// assembly phase so the channel-plugin rebuilder hook it installs
    /// carries the same channel policy as the boot path.
    pub fn with_channels_disabled(mut self, no_channels: bool) -> Self {
        self.no_channels = no_channels;
        self
    }

    /// The typed startup chain: each phase consumes the previous phase's
    /// witness, so an illegal ordering does not compile. The witnesses and
    /// their only constructors live in the sealed [`phases`] module, so no
    /// code in this crate — including submodules of this file — can forge a
    /// phase output and skip a step.
    pub async fn assemble(self) -> Result<RuntimeAssembly, Box<dyn std::error::Error>> {
        let locked = phases::acquire_locked_stores(&self.config).await?;
        let imported = phases::import_thread_data(locked).await?;
        phases::assemble_runtime(imported, self.config, self.config_path, self.no_channels).await
    }
}

/// Sealed startup phases. The witness types' fields are private to this
/// module and the three phase functions are their only constructors, so the
/// typed chain cannot be bypassed from anywhere else in the crate —
/// including submodules of the composition root.
mod phases {
    use super::*;

    /// Phase 1 witness: exclusive ownership of the data directory plus the raw
    /// persistent stores. Nothing else may open runtime storage before this
    /// exists, and later phases can only be reached through it.
    pub(super) struct LockedStores {
        session_data_dir: String,
        garyx_db: Arc<garyx_gateway::garyx_db::GaryxDbService>,
        transcript_store: Arc<ThreadTranscriptStore>,
        message_ledger: Arc<MessageLedgerStore>,
    }

    /// Phase 2 witness: canonical thread data imported (legacy boot import and
    /// one-shot cutovers complete) and the store handles bound for the bridge.
    /// Listener binding is unreachable without passing through this type.
    pub(super) struct ImportedStores {
        session_data_dir: String,
        garyx_db: Arc<garyx_gateway::garyx_db::GaryxDbService>,
        message_ledger: Arc<MessageLedgerStore>,
        bridge: Arc<MultiProviderBridge>,
        thread_store: Arc<dyn ThreadStore>,
        thread_history: Arc<ThreadHistoryRepository>,
    }

    /// Phase 1: take exclusive ownership of the data directory before opening any
    /// other persistent runtime store. GaryxDbService holds the lock for the
    /// lifetime of AppState and performs schema initialization only after the
    /// pre-R5 parent handoff barrier has cleared.
    pub(super) async fn acquire_locked_stores(
        config: &GaryxConfig,
    ) -> Result<LockedStores, Box<dyn std::error::Error>> {
        let session_data_dir = config
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
        Ok(LockedStores {
            session_data_dir,
            garyx_db,
            transcript_store,
            message_ledger,
        })
    }

    /// Phase 2: import canonical thread data and bind the store handles the
    /// bridge and history layers share.
    pub(super) async fn import_thread_data(
        stores: LockedStores,
    ) -> Result<ImportedStores, Box<dyn std::error::Error>> {
        let LockedStores {
            session_data_dir,
            garyx_db,
            transcript_store,
            message_ledger,
        } = stores;
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
        Ok(ImportedStores {
            session_data_dir,
            garyx_db,
            message_ledger,
            bridge,
            thread_store,
            thread_history,
        })
    }

    /// Phase 3: build the fully wired state graph — cron, AppState, crash
    /// recovery under the data-dir lock, and the bridge/cron runtime bindings.
    pub(super) async fn assemble_runtime(
        imported: ImportedStores,
        config: GaryxConfig,
        config_path: PathBuf,
        no_channels: bool,
    ) -> Result<RuntimeAssembly, Box<dyn std::error::Error>> {
        let ImportedStores {
            session_data_dir,
            garyx_db,
            message_ledger,
            bridge,
            thread_store,
            thread_history,
        } = imported;
        let (event_tx, _) = tokio::sync::broadcast::channel(128);

        let mut cron_service_raw = CronService::new(PathBuf::from(&session_data_dir));
        let cron_boot_config = config.cron.clone();
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

        let builder =
            AppStateBuilder::new(config.clone()).with_persistent_local_stores(garyx_db.clone());
        let state = builder
            .with_thread_store(thread_store.clone())
            .with_thread_history(thread_history.clone())
            .with_message_ledger(message_ledger)
            .with_bridge(bridge.clone())
            .with_provider_runtime_ready(false)
            .with_event_tx(event_tx)
            .with_cron_service(cron_service.clone())
            .with_config_path(config_path)
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
                config.mcp_servers.clone(),
                state.ops.custom_agents.clone(),
            )
            .await;
        // Phase-7: installing the channel-plugin rebuilder is a
        // mandatory assembly step, not an optional call sites may
        // forget — the only way to obtain a RuntimeAssembly is through
        // this phase, so production and tests get the hook installed
        // by construction.
        install_channel_plugin_rebuilder(
            &state,
            &state.channel_plugin_manager(),
            &bridge,
            no_channels,
        );

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

/// The production `HostDeps` every subprocess plugin handler is built
/// from. Extracted so the shared-switch contract is pinned by a test:
/// `plugin_auto_update_enabled` MUST be the process-wide Arc owned by
/// AppState (apply_runtime_config refreshes it on every config apply),
/// never a per-registration snapshot of the config value — a snapshot
/// leaves live handlers on a stale switch (Phase-7 review round 2).
pub(crate) fn manifest_host_deps(
    state: &Arc<AppState>,
    bridge: &Arc<MultiProviderBridge>,
    plugin_manager: &std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
) -> crate::channel_plugin_host::HostDeps {
    crate::channel_plugin_host::HostDeps {
        router: state.threads.router.clone(),
        bridge: bridge.clone(),
        swap: state.channel_dispatcher_swap(),
        // Weak handle so the `request_self_replace` host RPC can drive
        // respawn after a successful swap without forming a
        // manager↔handler reference cycle that would survive gateway
        // shutdown.
        plugin_manager: std::sync::Arc::downgrade(plugin_manager),
        // Plugin-side master kill switch: the process-wide shared
        // handle owned by AppState.
        plugin_auto_update_enabled: state.plugin_auto_update_enabled(),
    }
}

/// Install the production channel-plugin rebuilder hook on `state`
/// (Phase-7 single derivation point). Captures a `Weak<AppState>` so
/// the hook never forms an `AppState -> hook -> AppState` strong
/// reference cycle: if the gateway is tearing down when a rebuild
/// fires, the upgrade fails and the rebuild is skipped.
pub(crate) fn install_channel_plugin_rebuilder(
    state: &Arc<AppState>,
    plugin_manager: &std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
    bridge: &Arc<MultiProviderBridge>,
    no_channels: bool,
) {
    let weak_state = Arc::downgrade(state);
    let plugin_manager = plugin_manager.clone();
    let bridge = bridge.clone();
    state.set_channel_plugin_rebuilder(Arc::new(move |config: GaryxConfig| {
        let weak_state = weak_state.clone();
        let plugin_manager = plugin_manager.clone();
        let bridge = bridge.clone();
        Box::pin(async move {
            let Some(state) = weak_state.upgrade() else {
                // Gateway shutting down; nothing to rebuild against.
                return Ok(());
            };
            rebuild_channel_plugins(&plugin_manager, &config, &state, &bridge, no_channels).await
        })
    }));
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
            manifest_host_deps(state, bridge, plugin_manager),
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
        .with_channels_disabled(no_channels)
        .assemble()
        .await?;

    // 3.0 Share the channel plugin manager with AppState so HTTP
    // endpoints (`GET /api/channels/plugins`) see the same
    // registrations the boot path creates. Single source of truth.
    let plugin_manager = state.channel_plugin_manager();
    register_plugin_state_logging(&mut *plugin_manager.lock().await);

    // Teardown mirrors startup: the single production registration point
    // records each phase's inverse and shutdown drains the stack in reverse
    // order, so the two sequences cannot drift apart.
    let mut teardown = TeardownStack::new();
    register_shutdown_teardowns(&mut teardown, cron_service, plugin_manager.clone());

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
        let tokio_handle = tokio::runtime::Handle::current();
        reloader.register_callback(move |new_config, diagnostics| {
            print_diagnostics(&diagnostics);
            let state = state_cb.clone();
            tokio_handle.spawn(async move {
                let _settings_guard = state.ops.settings_mutex.lock().await;
                if let Err(error) = state.apply_runtime_config(new_config).await {
                    tracing::warn!(
                        error = %error,
                        "Failed to fully apply hot-reloaded config"
                    );
                    return;
                }

                // The channel plugin rebuild now runs inside
                // apply_runtime_config through the assembly-installed
                // rebuilder hook (Phase-7 single derivation point),
                // gated on the rebuild-inputs projection (channels
                // section + gateway.public_url) — so an external edit
                // of unrelated config sections no longer bounces live
                // channel connections.
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

    // Always run the shutdown sequence, even when startup/serve fails:
    // drain the teardown stack registered during startup (LIFO), i.e.
    // channel plugins first, then the cron service.
    let _ = teardown.drain().await;
    run_result
}

#[cfg(test)]
mod tests {
    use super::{RuntimeAssembler, RuntimeAssembly};
    use super::{TeardownStack, register_shutdown_teardowns};
    use garyx_channels::ChannelPluginManager;
    use garyx_gateway::CronService;
    use std::sync::Arc;

    /// Shared-switch contract (Phase-7 review round 2): the production
    /// HostDeps constructor must hand every subprocess handler the
    /// process-wide AppState switch — reverting to a per-registration
    /// `Arc::new(AtomicBool::new(config…))` snapshot breaks ptr
    /// identity AND flip visibility below.
    #[tokio::test]
    async fn manifest_host_deps_share_the_app_state_auto_update_switch() {
        use garyx_gateway::server::AppStateBuilder;
        use garyx_models::config::GaryxConfig;
        use std::sync::atomic::Ordering;

        let bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
        let state = AppStateBuilder::new(GaryxConfig::default())
            .with_bridge(bridge.clone())
            .build();
        let plugin_manager = state.channel_plugin_manager();

        let deps = super::manifest_host_deps(&state, &bridge, &plugin_manager);
        assert!(
            Arc::ptr_eq(
                &deps.plugin_auto_update_enabled,
                &state.plugin_auto_update_enabled()
            ),
            "handlers must hold the process-wide switch, not a snapshot"
        );
        state
            .plugin_auto_update_enabled()
            .store(true, Ordering::Release);
        assert!(
            deps.plugin_auto_update_enabled.load(Ordering::Acquire),
            "a flip on the AppState switch must be visible through HostDeps"
        );
    }

    /// Production-wiring guard (Phase-7 review rounds 1-2): consumes the
    /// REAL RuntimeAssembler output and drives the hook the assembly
    /// phase installed — the only way to obtain a RuntimeAssembly — so
    /// removing the installation turns this red with no synthetic
    /// re-wiring in the test body. Channels are disabled for the
    /// assembly so the rebuild skips discovery (plugin_root_paths
    /// always includes the machine-global ~/.garyx/plugins install
    /// root; a test must never spawn the real subprocess plugins). The
    /// rebuild's signature move — stop_all() replacing the old
    /// manager — is observed via a pre-registered stub plugin's state
    /// transitions, which the accounts-only reload path never emits.
    #[tokio::test]
    async fn assembled_runtime_rebuilds_the_real_manager_on_rebuild_input_change() {
        use garyx_channels::channel_trait::{Channel, ChannelError};
        use garyx_channels::plugin::{ManagedChannelPlugin, PluginMetadata};
        use garyx_models::config::GaryxConfig;

        struct NoopChannel;
        #[async_trait::async_trait]
        impl Channel for NoopChannel {
            fn name(&self) -> &str {
                "phase7-probe"
            }
            async fn start(&mut self) -> Result<(), ChannelError> {
                Ok(())
            }
            async fn stop(&mut self) -> Result<(), ChannelError> {
                Ok(())
            }
            fn is_running(&self) -> bool {
                false
            }
        }

        let temp = tempfile::TempDir::new().expect("temp dir");
        let mut config = GaryxConfig::default();
        config.sessions.data_dir = Some(temp.path().join("data").to_string_lossy().into_owned());

        let RuntimeAssembly {
            state,
            bridge: _bridge,
            cron_service: _cron_service,
        } = RuntimeAssembler::new(temp.path().join("garyx.json"), config.clone())
            .with_channels_disabled(true)
            .assemble()
            .await
            .expect("assembly");
        let plugin_manager = state.channel_plugin_manager();

        // Stub entry whose Stopped transition is the rebuild sentinel.
        let observed = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        {
            let mut manager = plugin_manager.lock().await;
            manager
                .register_plugin(Box::new(ManagedChannelPlugin::new(
                    PluginMetadata {
                        id: "phase7-probe".to_owned(),
                        aliases: Vec::new(),
                        display_name: "phase7-probe".to_owned(),
                        version: "0.0.0".to_owned(),
                        description: String::new(),
                        source: "test".to_owned(),
                        config_methods: Vec::new(),
                    },
                    Box::new(NoopChannel),
                )))
                .expect("register probe plugin");
            let observed = observed.clone();
            manager.register_state_hook(move |status| {
                observed
                    .lock()
                    .expect("sentinel lock")
                    .push(format!("{}:{:?}", status.metadata.id, status.state));
            });
        }

        // Drive the assembly-installed hook directly. The other half
        // of the chain — apply_runtime_config invoking whatever hook is
        // installed, gated on the rebuild-inputs projection — is pinned
        // by the garyx-gateway contract tests
        // (test_apply_runtime_config_invokes_rebuilder_only_on_channels_change,
        // public_url_change_triggers_plugin_rebuild). Composing the two
        // keeps this test free of apply's unrelated provider reload,
        // whose cold-start cost is minutes on a fresh machine.
        let rebuilder = state
            .integration
            .channel_plugin_rebuilder
            .get()
            .expect("the assembly phase must install the production rebuilder")
            .clone();
        rebuilder(config).await.expect("real rebuild must succeed");

        let events = observed.lock().expect("sentinel lock").clone();
        assert!(
            events
                .iter()
                .any(|event| event.starts_with("phase7-probe:") && event.contains("Stopped")),
            "the assembly-installed hook must run the REAL rebuild; observed: {events:?}"
        );
        state.ops.meetings.shutdown_ingestion();
    }

    /// Mutation guard for startup/shutdown mirroring: this drives the SAME
    /// registration function production `run` uses, with real (inert)
    /// runtime objects, and asserts the drained order. Reversing the
    /// registration order inside `register_shutdown_teardowns` turns this
    /// test red.
    #[tokio::test]
    async fn shutdown_teardowns_drain_plugins_before_cron() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let cron_service = Arc::new(CronService::new(tmp.path().to_path_buf()));
        let plugin_manager = Arc::new(tokio::sync::Mutex::new(ChannelPluginManager::new()));

        let mut teardown = TeardownStack::new();
        register_shutdown_teardowns(&mut teardown, cron_service, plugin_manager);

        let executed = teardown.drain().await;
        assert_eq!(
            executed,
            vec!["channel_plugins", "cron_service"],
            "shutdown must mirror startup: plugins stop before the cron service"
        );
    }
}
