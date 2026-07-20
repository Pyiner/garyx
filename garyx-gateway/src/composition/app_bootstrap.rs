use garyx_bridge::MultiProviderBridge;
use garyx_channels::plugin::PluginDiscoverer;
use garyx_channels::{
    BuiltInPluginDiscoverer, ChannelDispatcher, ChannelDispatcherImpl, ChannelPluginManager,
    SwappableDispatcher,
};
use garyx_models::config::GaryxConfig;
use garyx_models::local_paths::{default_custom_agents_state_path, default_session_data_dir};
use garyx_models::thread_logs::{NoopThreadLogSink, ThreadLogSink};
use garyx_router::{
    InMemoryThreadStore, MessageLedgerStore, MessageRouter, ThreadCreator, ThreadHistoryRepository,
    ThreadStore, ThreadTranscriptStore,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tokio::sync::{Mutex, Notify, broadcast};
use tracing::warn;

use crate::agent_identity::GatewayThreadCreator;
use crate::app_state::{AppState, IntegrationState, OpsState, RuntimeState, ThreadState};
use crate::composition::runtime_config_projection::RuntimeConfigProjection;
use crate::conversation_admission::ConversationAdmissionService;
use crate::cron::CronService;
use crate::custom_agents::CustomAgentStore;
use crate::endpoint_binding_mutator::SqlEndpointBindingMutator;
use crate::event_stream_hub::EventStreamHub;
use crate::garyx_db::GaryxDbService;
use crate::health::HealthChecker;
use crate::mcp_metrics::McpToolMetrics;
use crate::meetings::MeetingService;
use crate::prompt_attachment_lifecycle::PromptAttachmentLifecycle;
use crate::provider_auth::ClaudeAuthSessionStore;
use crate::recent_thread_projection::{ActiveRunProbe, BridgeActiveRunProbe};
use crate::recent_thread_reader::SqlRecentThreadPageReader;
use crate::routes::RestartTracker;
use crate::runtime_cells::{ChannelDispatcherCell, LiveConfigCell};
use crate::skills::SkillsService;
use crate::sqlite_thread_store::{SqliteThreadStore, SqliteThreadStoreHandle};
use crate::thread_lifecycle::LifecycleService;

enum ThreadStoreBinding {
    Automatic,
    Custom(Arc<dyn ThreadStore>),
    Sqlite(SqliteThreadStoreHandle),
}

/// Load a persistent `Store` from the given on-disk path, falling back to an
/// empty in-memory instance **only** if loading fails — but shout about the
/// failure via `tracing::warn!` so silent disk corruption cannot quietly
/// vaporize user state across restarts.
///
/// This replaces the old `Store::file(path).unwrap_or_else(|_| Store::new())`
/// pattern which swallowed parse errors and left operators wondering why their
/// agents had disappeared.
fn load_store_or_warn<T>(
    store_name: &'static str,
    path: PathBuf,
    load: impl FnOnce(PathBuf) -> Result<T, String>,
    default: impl FnOnce() -> T,
) -> T {
    let path_display = path.display().to_string();
    match load(path) {
        Ok(store) => store,
        Err(error) => {
            warn!(
                store = store_name,
                path = %path_display,
                error = %error,
                "failed to load persistent store from disk; falling back to empty in-memory instance. Inspect and repair the file before the next write, otherwise the on-disk state will be overwritten."
            );
            default()
        }
    }
}

/// Builder that owns gateway dependency injection and emits a fully wired [`AppState`].
pub struct AppStateBuilder {
    config: GaryxConfig,
    thread_store: ThreadStoreBinding,
    thread_history: Arc<ThreadHistoryRepository>,
    message_ledger: Arc<MessageLedgerStore>,
    bridge: Arc<MultiProviderBridge>,
    event_tx: broadcast::Sender<String>,
    cron_service: Option<Arc<CronService>>,
    config_path: Option<PathBuf>,
    restart_tokens: Vec<String>,
    channel_dispatcher: Arc<dyn ChannelDispatcher>,
    channel_swap: Arc<SwappableDispatcher>,
    channel_plugin_manager: Arc<Mutex<ChannelPluginManager>>,
    thread_logs: Arc<dyn ThreadLogSink>,
    skills: Arc<SkillsService>,
    custom_agents: Arc<CustomAgentStore>,
    garyx_db: Arc<GaryxDbService>,
    meetings_dir: PathBuf,
    session_data_dir: PathBuf,
    /// Optional override for the active-run probe. Production leaves this `None`
    /// and `build` wires a bridge-backed probe; tests inject a fake to control
    /// which runs count as live.
    active_run_probe: Option<Arc<dyn ActiveRunProbe>>,
    provider_runtime_ready: bool,
}

impl AppStateBuilder {
    pub fn new(config: GaryxConfig) -> Self {
        let (event_tx, _) = broadcast::channel(128);
        // The production dispatcher is a `SwappableDispatcher` wrapping
        // a `ChannelDispatcherImpl`; §9.4 respawn needs the stable
        // swap identity. The builder stores both: a trait-object
        // `channel_dispatcher` (for tests that mock via cell replace)
        // and the concrete `channel_swap` (for the plugin manager's
        // attach path).
        let channel_swap = Arc::new(SwappableDispatcher::new(
            ChannelDispatcherImpl::from_config(&config.channels),
        ));
        let channel_dispatcher: Arc<dyn ChannelDispatcher> = channel_swap.clone();
        // Default to an empty manager; the binary's boot path replaces
        // it after running discovery. Tests using `AppStateBuilder`
        // directly keep the empty default.
        let channel_plugin_manager = Arc::new(Mutex::new(ChannelPluginManager::new()));
        let garyx_db_default = Arc::new(
            GaryxDbService::memory()
                .unwrap_or_else(|error| panic!("failed to open garyx database: {error}")),
        );
        // The default store is built in `build()` — the real
        // SqliteThreadStore over the in-memory garyx database, wired to
        // the resolved active-run probe (#TASK-1864 closing batch): tests
        // run on the same truth-table + same-transaction-projection
        // semantics as production, and `with_active_run_probe` still
        // applies. This history handle is a placeholder carrying the
        // transcript store; `build()` rebuilds it over the final store.
        let thread_history = Arc::new(ThreadHistoryRepository::new(
            Arc::new(InMemoryThreadStore::new()),
            Arc::new(ThreadTranscriptStore::memory()),
        ));
        let skills = Arc::new(SkillsService::new(
            SkillsService::default_user_dir(),
            SkillsService::default_project_dir(),
        ));
        let session_data_dir = config
            .sessions
            .data_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir()
                    .join(format!("garyx-gateway-test-data-{}", uuid::Uuid::new_v4()))
            });
        Self {
            config,
            thread_store: ThreadStoreBinding::Automatic,
            thread_history,
            message_ledger: Arc::new(MessageLedgerStore::memory()),
            bridge: Arc::new(MultiProviderBridge::new()),
            event_tx,
            cron_service: None,
            config_path: None,
            restart_tokens: Vec::new(),
            channel_dispatcher,
            channel_swap,
            channel_plugin_manager,
            thread_logs: Arc::new(NoopThreadLogSink),
            skills,
            custom_agents: Arc::new(CustomAgentStore::new()),
            garyx_db: garyx_db_default,
            meetings_dir: std::env::temp_dir()
                .join(format!("garyx-meetings-test-{}", uuid::Uuid::new_v4())),
            session_data_dir,
            active_run_probe: None,
            provider_runtime_ready: true,
        }
    }

    /// Bind the real on-disk `~/.garyx` state: the persistent custom-agent
    /// store, the garyx database, and built-in skill seeding.
    ///
    /// This is the production boot path's explicit opt-in. `new()`
    /// deliberately stays fully in-memory so that tests (unit *and*
    /// integration, where `cfg(test)` gating on the library does not apply)
    /// can never read or clobber live user data by default — a test's
    /// whole-file persist through these defaults is what erased a
    /// real custom agent on 2026-07-06.
    pub fn with_persistent_local_stores(mut self, garyx_db: Arc<GaryxDbService>) -> Self {
        if let Err(error) = self.skills.seed_builtin_skills() {
            warn!(error = %error, "failed to seed built-in skills during startup");
        }
        if let Err(error) = self.skills.sync_external_user_skills() {
            warn!(error = %error, "failed to sync external user skills during startup");
        }
        self.custom_agents = Arc::new(load_store_or_warn(
            "custom_agents",
            default_custom_agents_state_path(),
            CustomAgentStore::file,
            CustomAgentStore::new,
        ));
        // RuntimeAssembler owns the one and only on-disk database open. In
        // particular, a configured custom sessions.data_dir must never make
        // the builder initialize (or purge) the default data directory first.
        self.garyx_db = garyx_db;
        self.meetings_dir = garyx_models::local_paths::default_meetings_dir();
        self.session_data_dir = self
            .config
            .sessions
            .data_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_session_data_dir);
        self
    }

    pub fn with_thread_store(mut self, thread_store: Arc<dyn ThreadStore>) -> Self {
        self.thread_history = Arc::new(ThreadHistoryRepository::new(
            thread_store.clone(),
            self.thread_history.transcript_store(),
        ));
        self.thread_store = ThreadStoreBinding::Custom(thread_store);
        self
    }

    /// Install the typed SQLite store assembled by the production runtime.
    /// The handle preserves the concrete capability needed for atomic durable
    /// dispatch while exposing the same instance through the generic router
    /// and history interfaces.
    pub fn with_sqlite_thread_store(
        mut self,
        sqlite_thread_store: SqliteThreadStoreHandle,
    ) -> Self {
        let thread_store = sqlite_thread_store.thread_store();
        self.thread_history = Arc::new(ThreadHistoryRepository::new(
            thread_store,
            self.thread_history.transcript_store(),
        ));
        self.thread_store = ThreadStoreBinding::Sqlite(sqlite_thread_store);
        self
    }

    pub fn with_thread_history(mut self, thread_history: Arc<ThreadHistoryRepository>) -> Self {
        self.thread_history = thread_history;
        self
    }

    pub fn with_message_ledger(mut self, message_ledger: Arc<MessageLedgerStore>) -> Self {
        self.message_ledger = message_ledger;
        self
    }

    pub fn with_bridge(mut self, bridge: Arc<MultiProviderBridge>) -> Self {
        self.bridge = bridge;
        self
    }

    pub fn with_provider_runtime_ready(mut self, ready: bool) -> Self {
        self.provider_runtime_ready = ready;
        self
    }

    /// Override the active-run probe (tests). Production wiring derives the
    /// probe from the bridge in `build`.
    #[cfg(test)]
    pub(crate) fn with_active_run_probe(mut self, probe: Arc<dyn ActiveRunProbe>) -> Self {
        self.active_run_probe = Some(probe);
        self
    }

    pub fn with_event_tx(mut self, event_tx: broadcast::Sender<String>) -> Self {
        self.event_tx = event_tx;
        self
    }

    pub fn with_cron_service(mut self, cron_service: Arc<CronService>) -> Self {
        self.cron_service = Some(cron_service);
        self
    }

    pub fn with_config_path(mut self, config_path: PathBuf) -> Self {
        self.config_path = Some(config_path);
        self
    }

    pub fn with_restart_tokens(mut self, restart_tokens: Vec<String>) -> Self {
        self.restart_tokens = restart_tokens;
        self
    }

    pub fn with_channel_dispatcher(
        mut self,
        channel_dispatcher: Arc<dyn ChannelDispatcher>,
    ) -> Self {
        self.channel_dispatcher = channel_dispatcher;
        self
    }

    /// Install the production [`SwappableDispatcher`]. Callers that
    /// need a custom trait-object-only test double can still call
    /// [`Self::with_channel_dispatcher`] after this method; production
    /// paths should keep the trait-object view and concrete swap in
    /// sync.
    pub fn with_channel_swap(mut self, channel_swap: Arc<SwappableDispatcher>) -> Self {
        self.channel_dispatcher = channel_swap.clone();
        self.channel_swap = channel_swap;
        self
    }

    /// Install the shared [`ChannelPluginManager`] so HTTP routes and
    /// the binary's boot path share one instance. Defaults to an
    /// empty manager; the binary replaces it immediately after
    /// discovery.
    pub fn with_channel_plugin_manager(
        mut self,
        manager: Arc<Mutex<ChannelPluginManager>>,
    ) -> Self {
        self.channel_plugin_manager = manager;
        self
    }

    pub fn with_thread_log_sink(mut self, thread_logs: Arc<dyn ThreadLogSink>) -> Self {
        self.thread_logs = thread_logs;
        self
    }

    pub fn with_skills_service(mut self, skills: Arc<SkillsService>) -> Self {
        self.skills = skills;
        self
    }

    pub fn with_custom_agent_store(mut self, custom_agents: Arc<CustomAgentStore>) -> Self {
        self.custom_agents = custom_agents;
        self
    }

    pub fn with_garyx_db(mut self, garyx_db: Arc<GaryxDbService>) -> Self {
        self.garyx_db = garyx_db;
        self
    }

    pub fn with_meetings_dir(mut self, meetings_dir: PathBuf) -> Self {
        self.meetings_dir = meetings_dir;
        self
    }

    pub fn build(self) -> Arc<AppState> {
        let start_time = Instant::now();
        // Runtime assembly runs the legacy boot import before reaching this
        // point. Direct builder construction in tests intentionally skips it:
        // the shared cutover gate then uses import generation 0.
        self.garyx_db
            .run_thread_data_startup_migrations()
            .unwrap_or_else(|error| {
                panic!("failed to run thread-data startup migrations: {error}")
            });
        let active_run_probe: Arc<dyn ActiveRunProbe> = self
            .active_run_probe
            .clone()
            .unwrap_or_else(|| Arc::new(BridgeActiveRunProbe::new(Arc::downgrade(&self.bridge))));
        // The store arrives final (#TASK-1864 closing batch): SQLite
        // backends derive projections inside their own write transaction,
        // so there is nothing left for a wrapper to do. The file archive is
        // no longer a primary backend; the former projecting wrapper and
        // its startup reconciliation are retired with it.
        let (thread_store, sqlite_thread_store): (
            Arc<dyn ThreadStore>,
            Option<Arc<SqliteThreadStore>>,
        ) = match &self.thread_store {
            ThreadStoreBinding::Custom(store) => (store.clone(), None),
            ThreadStoreBinding::Sqlite(handle) => {
                let store = handle.concrete_store();
                (store.clone(), Some(store))
            }
            ThreadStoreBinding::Automatic => {
                let store = Arc::new(SqliteThreadStore::new(
                    self.garyx_db.clone(),
                    self.thread_history.transcript_store(),
                    active_run_probe,
                ));
                (store.clone(), Some(store))
            }
        };
        let thread_history = ThreadHistoryRepository::new(
            thread_store.clone(),
            self.thread_history.transcript_store(),
        );
        let thread_history = Arc::new(thread_history);
        let mut router = MessageRouter::new(thread_store.clone(), self.config.clone());
        router.set_recent_thread_page_reader(Arc::new(SqlRecentThreadPageReader::new(
            self.garyx_db.clone(),
        )));
        let endpoint_binding_mutator = Arc::new(SqlEndpointBindingMutator::new_with_sqlite_store(
            thread_store.clone(),
            self.garyx_db.clone(),
            sqlite_thread_store.clone(),
        ));
        router.set_endpoint_binding_mutator(endpoint_binding_mutator.clone());
        let thread_creator: Arc<dyn ThreadCreator> = Arc::new(GatewayThreadCreator::new(
            self.bridge.clone(),
            self.custom_agents.clone(),
        ));
        router.set_thread_creator(thread_creator.clone());
        router.set_thread_history_repository(thread_history.clone());
        router.set_thread_log_sink(self.thread_logs.clone());
        router.set_message_ledger_store(self.message_ledger.clone());
        self.bridge.set_thread_log_sink(self.thread_logs.clone());
        self.bridge.set_thread_store_blocking(thread_store.clone());
        self.bridge.set_thread_history(thread_history.clone());

        // Seed the bridge with the current agent registry before the first
        // request. Runtime config reloads refresh the same snapshot.
        let boot_agent_snapshot = self.custom_agents.snapshot_blocking();
        let bridge_for_profiles = self.bridge.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                bridge_for_profiles
                    .replace_agent_profiles(boot_agent_snapshot)
                    .await;
            });
        } else {
            warn!(
                "AppStateBuilder::build invoked outside a tokio runtime; \
                 agent profiles will be pushed to bridge on first apply_runtime_config"
            );
        }

        let router = Arc::new(Mutex::new(router));
        let projection = RuntimeConfigProjection::from_config(&self.config);
        let meetings = Arc::new(
            MeetingService::new(
                self.garyx_db.clone(),
                self.meetings_dir,
                projection.meeting_read_page_bytes,
            )
            .unwrap_or_else(|error| panic!("failed to initialize meeting service: {error}")),
        );
        meetings.start_ingestion(projection.meeting_join_retry_window_secs);
        {
            let mut manager = self
                .channel_plugin_manager
                .try_lock()
                .expect("channel plugin manager lock must be uncontended during build");
            manager.attach_dispatcher(self.channel_swap.clone());
            let discoverer = BuiltInPluginDiscoverer::with_dispatcher_and_meeting_sink(
                self.config.channels.clone(),
                router.clone(),
                self.bridge.clone(),
                self.channel_swap.clone(),
                String::new(),
                meetings.clone(),
            );
            let discovered = discoverer.discover().unwrap_or_else(|error| {
                panic!("failed to discover built-in channel plugins during build: {error}")
            });
            for plugin in discovered {
                let plugin_id = plugin.metadata().id.clone();
                if manager.plugin(&plugin_id).is_some() {
                    continue;
                }
                if let Err(error) = manager.register_plugin(plugin) {
                    panic!(
                        "failed to register built-in channel plugin '{plugin_id}' during build: {error}"
                    );
                }
            }
        }

        let live_config = Arc::new(LiveConfigCell::new(self.config.clone()));
        let events = EventStreamHub::new(self.event_tx);
        let lifecycle = LifecycleService::new(self.garyx_db.clone());
        let conversation_admission = ConversationAdmissionService::new(self.garyx_db.clone());
        let prompt_attachments =
            PromptAttachmentLifecycle::new(self.garyx_db.clone(), self.session_data_dir.clone());
        let state = Arc::new(AppState {
            runtime: RuntimeState {
                start_time,
                server_boot_id: uuid::Uuid::new_v4().to_string(),
                health_checker: HealthChecker::new(start_time),
                live_config,
                provider_runtime_ready: Arc::new(AtomicBool::new(self.provider_runtime_ready)),
                provider_runtime_ready_notify: Arc::new(Notify::new()),
            },
            threads: ThreadState {
                thread_store,
                sqlite_thread_store,
                history: thread_history,
                message_ledger: self.message_ledger,
                router,
            },
            ops: OpsState {
                events,
                restart_tracker: Mutex::new(RestartTracker::new()),
                settings_mutex: Mutex::new(()),
                cron_service: self.cron_service,
                config_path: self.config_path,
                restart_tokens: self.restart_tokens,
                mcp_tool_metrics: Arc::new(McpToolMetrics::default()),
                thread_logs: self.thread_logs,
                skills: self.skills,
                custom_agents: self.custom_agents,
                garyx_db: self.garyx_db,
                conversation_admission,
                prompt_attachments,
                meetings,
                provider_auth_sessions: Arc::new(ClaudeAuthSessionStore::default()),
                channel_endpoint_snapshot: Mutex::new(None),
                endpoint_binding_mutator,
                lifecycle: lifecycle.clone(),
            },
            integration: IntegrationState {
                bridge: self.bridge,
                channel_dispatcher: Arc::new(ChannelDispatcherCell::new(self.channel_dispatcher)),
                channel_swap: self.channel_swap,
                channel_plugin_manager: self.channel_plugin_manager,
                channel_plugin_rebuilder: Arc::new(std::sync::OnceLock::new()),
                plugin_auto_update_enabled: Arc::new(std::sync::atomic::AtomicBool::new(
                    self.config.plugins.auto_update,
                )),
            },
        });

        lifecycle.attach_state(Arc::downgrade(&state));
        crate::create_resources::spawn_create_resource_cleanup_worker(&state);

        state
    }
}

pub fn create_app_state(config: GaryxConfig) -> Arc<AppState> {
    AppStateBuilder::new(config).build()
}

pub fn create_app_state_with_bridge(
    config: GaryxConfig,
    bridge: Arc<MultiProviderBridge>,
) -> Arc<AppState> {
    AppStateBuilder::new(config).with_bridge(bridge).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the 2026-07-06 gary incident: the builder used to
    /// bind the real `~/.garyx` stores (custom agents / Garyx DB) by default,
    /// so every test constructing an `AppState` read and
    /// *wrote* live user data — a test's whole-file persist
    /// overwrote `custom-agents.json` and vaporized a real agent definition.
    ///
    /// Defaults must be in-memory. Production opts into disk-backed stores
    /// explicitly via `with_persistent_local_stores(db)`.
    #[test]
    fn builder_defaults_stay_off_real_user_state() {
        let builder = AppStateBuilder::new(GaryxConfig::default());
        assert!(
            builder.custom_agents.persistence_path().is_none(),
            "default custom-agent store must not persist to real user files"
        );
    }
}
