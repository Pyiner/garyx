use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::AgentLoopProvider;
use garyx_bridge::providers::agent_team::{
    AgentTeamProvider, FileGroupStore, GroupStore, SubAgentDispatcher, TeamProfileResolver,
};
use garyx_channels::plugin::PluginDiscoverer;
use garyx_channels::{
    BuiltInPluginDiscoverer, ChannelDispatcher, ChannelDispatcherImpl, ChannelPluginManager,
    SwappableDispatcher,
};
use garyx_models::config::GaryxConfig;
use garyx_models::local_paths::default_agent_team_groups_dir;
use garyx_models::local_paths::default_agent_teams_state_path;
use garyx_models::local_paths::default_auto_research_state_path;
use garyx_models::local_paths::default_custom_agents_state_path;
use garyx_models::local_paths::default_wikis_state_path;
use garyx_models::thread_logs::{NoopThreadLogSink, ThreadLogSink};
use garyx_models::validate_agent_team_registry_uniqueness;
use garyx_router::{
    InMemoryThreadStore, MessageLedgerStore, MessageRouter, ThreadCreator, ThreadHistoryRepository,
    ThreadStore, ThreadTranscriptStore,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, broadcast};
use tracing::warn;

use crate::agent_identity::GatewayThreadCreator;
use crate::agent_team_provider::{
    AGENT_TEAM_PROVIDER_KEY, GatewaySubAgentDispatcher, GatewayTeamProfileResolver,
};
use crate::agent_teams::AgentTeamStore;
use crate::api::RestartTracker;
use crate::app_state::{AppState, IntegrationState, OpsState, RuntimeState, ThreadState};
use crate::auto_research::AutoResearchStore;
use crate::cron::CronService;
use crate::custom_agents::CustomAgentStore;
use crate::event_stream_hub::EventStreamHub;
use crate::health::HealthChecker;
use crate::heartbeat::HeartbeatService;
use crate::mcp_metrics::McpToolMetrics;
use crate::runtime_cells::{ChannelDispatcherCell, LiveConfigCell};
use crate::skills::SkillsService;
use crate::wikis::WikiStore;

/// Load a persistent `Store` from the given on-disk path, falling back to an
/// empty in-memory instance **only** if loading fails — but shout about the
/// failure via `tracing::warn!` so silent disk corruption cannot quietly
/// vaporize user state across restarts.
///
/// This replaces the old `Store::file(path).unwrap_or_else(|_| Store::new())`
/// pattern which swallowed parse errors and left operators wondering why their
/// teams / agents / wikis had disappeared.
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
    thread_store: Arc<dyn ThreadStore>,
    thread_history: Arc<ThreadHistoryRepository>,
    message_ledger: Arc<MessageLedgerStore>,
    bridge: Arc<MultiProviderBridge>,
    event_tx: broadcast::Sender<String>,
    cron_service: Option<Arc<CronService>>,
    heartbeat_service: Option<Arc<HeartbeatService>>,
    config_path: Option<PathBuf>,
    restart_tokens: Vec<String>,
    channel_dispatcher: Arc<dyn ChannelDispatcher>,
    channel_swap: Arc<SwappableDispatcher>,
    channel_plugin_manager: Arc<Mutex<ChannelPluginManager>>,
    thread_logs: Arc<dyn ThreadLogSink>,
    skills: Arc<SkillsService>,
    auto_research: Arc<AutoResearchStore>,
    custom_agents: Arc<CustomAgentStore>,
    agent_teams: Arc<AgentTeamStore>,
    wikis: Arc<WikiStore>,
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
        let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        let thread_history = Arc::new(ThreadHistoryRepository::new(
            thread_store.clone(),
            Arc::new(ThreadTranscriptStore::memory()),
        ));
        let skills = Arc::new(SkillsService::new(
            SkillsService::default_user_dir(),
            SkillsService::default_project_dir(),
        ));
        if let Err(error) = skills.seed_builtin_skills() {
            warn!(error = %error, "failed to seed built-in skills during startup");
        }
        if let Err(error) = skills.sync_external_user_skills() {
            warn!(error = %error, "failed to sync external user skills during startup");
        }
        Self {
            config,
            thread_store,
            thread_history,
            message_ledger: Arc::new(MessageLedgerStore::memory()),
            bridge: Arc::new(MultiProviderBridge::new()),
            event_tx,
            cron_service: None,
            heartbeat_service: None,
            config_path: None,
            restart_tokens: Vec::new(),
            channel_dispatcher,
            channel_swap,
            channel_plugin_manager,
            thread_logs: Arc::new(NoopThreadLogSink),
            skills,
            auto_research: Arc::new(load_store_or_warn(
                "auto_research",
                default_auto_research_state_path(),
                AutoResearchStore::file,
                AutoResearchStore::new,
            )),
            custom_agents: Arc::new(load_store_or_warn(
                "custom_agents",
                default_custom_agents_state_path(),
                CustomAgentStore::file,
                CustomAgentStore::new,
            )),
            agent_teams: Arc::new(load_store_or_warn(
                "agent_teams",
                default_agent_teams_state_path(),
                AgentTeamStore::file,
                AgentTeamStore::new,
            )),
            wikis: Arc::new(load_store_or_warn(
                "wikis",
                default_wikis_state_path(),
                WikiStore::file,
                WikiStore::new,
            )),
        }
    }

    pub fn with_thread_store(mut self, thread_store: Arc<dyn ThreadStore>) -> Self {
        self.thread_store = thread_store;
        self.thread_history = Arc::new(ThreadHistoryRepository::new(
            self.thread_store.clone(),
            self.thread_history.transcript_store(),
        ));
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

    pub fn with_event_tx(mut self, event_tx: broadcast::Sender<String>) -> Self {
        self.event_tx = event_tx;
        self
    }

    pub fn with_cron_service(mut self, cron_service: Arc<CronService>) -> Self {
        self.cron_service = Some(cron_service);
        self
    }

    pub fn with_heartbeat_service(mut self, heartbeat_service: Arc<HeartbeatService>) -> Self {
        self.heartbeat_service = Some(heartbeat_service);
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
    /// need both a trait-object view (for the `channel_dispatcher`
    /// cell) and the concrete handle (for
    /// `ChannelPluginManager::attach_dispatcher`) should provide this
    /// alongside [`Self::with_channel_dispatcher`] so the two slots
    /// agree on the initial dispatcher identity.
    pub fn with_channel_swap(mut self, channel_swap: Arc<SwappableDispatcher>) -> Self {
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

    pub fn with_auto_research_store(mut self, auto_research: Arc<AutoResearchStore>) -> Self {
        self.auto_research = auto_research;
        self
    }

    pub fn with_custom_agent_store(mut self, custom_agents: Arc<CustomAgentStore>) -> Self {
        self.custom_agents = custom_agents;
        self
    }

    pub fn with_agent_team_store(mut self, agent_teams: Arc<AgentTeamStore>) -> Self {
        self.agent_teams = agent_teams;
        self
    }

    pub fn with_wiki_store(mut self, wikis: Arc<WikiStore>) -> Self {
        self.wikis = wikis;
        self
    }

    pub fn build(self) -> Arc<AppState> {
        if let Ok(recovered) = self.auto_research.recover_interrupted_runs_blocking() {
            if !recovered.is_empty() {
                warn!(
                    recovered_count = recovered.len(),
                    "recovered interrupted auto research runs during startup"
                );
            }
        }

        // Teams and standalone agents share one agent_id namespace — a team_id
        // collision with an existing agent_id would make `agent_id` ambiguous
        // on threads. Surface the conflict fatally at boot instead of silently
        // picking one resolution at runtime.
        let boot_agents = self.custom_agents.list_agents_blocking();
        let boot_teams = self.agent_teams.list_teams_blocking();
        if let Err(error) = validate_agent_team_registry_uniqueness(&boot_agents, &boot_teams) {
            panic!("agent_team registry uniqueness check failed during startup: {error}");
        }
        let start_time = Instant::now();
        let mut router = MessageRouter::new(self.thread_store.clone(), self.config.clone());
        let thread_creator: Arc<dyn ThreadCreator> = Arc::new(GatewayThreadCreator::new(
            self.bridge.clone(),
            self.custom_agents.clone(),
            self.agent_teams.clone(),
        ));
        router.set_thread_creator(thread_creator.clone());
        router.set_thread_history_repository(self.thread_history.clone());
        router.set_thread_log_sink(self.thread_logs.clone());
        router.set_message_ledger_store(self.message_ledger.clone());
        self.bridge.set_thread_log_sink(self.thread_logs.clone());
        self.bridge.set_thread_history(self.thread_history.clone());

        // Wire the AgentTeam meta-provider into the bridge. This is the
        // production implementation of the two DI traits the provider needs:
        //   - `GatewayTeamProfileResolver` reads from the gateway's
        //     `AgentTeamStore` so teams loaded at boot are visible to the
        //     provider.
        //   - `GatewaySubAgentDispatcher` holds a `Weak<MultiProviderBridge>`
        //     (avoiding the Bridge → Provider → Dispatcher → Bridge cycle)
        //     plus the thread store and `ThreadCreator` so it can lazily
        //     spawn per-sub-agent threads and drive their runs via whichever
        //     concrete provider is registered for the child's provider_type.
        //
        // Registration goes through `register_provider_blocking` because
        // `AppStateBuilder::build` is synchronous (see the same pattern used
        // by `auto_research::recover_interrupted_runs_blocking` above and
        // `MultiProviderBridge::set_thread_history` in `multi_provider.rs`).
        // At boot the topology lock is uncontended; any contention here
        // indicates a wiring bug and should be surfaced loudly.
        // Share ONE Group store instance between the AgentTeam provider
        // (which writes as sub-agents get dispatched) and the gateway read
        // path (which surfaces `child_thread_ids` in the thread metadata
        // response — see `routes.rs::team_block_for_thread`). Constructing
        // a second `FileGroupStore` here would give the gateway a cold
        // cache that lags behind the provider's writes until the next file
        // read, producing stale UI state.
        let group_store: Arc<dyn GroupStore> =
            Arc::new(FileGroupStore::new(default_agent_team_groups_dir()));
        let provider_group_store = group_store.clone();
        let team_resolver: Arc<dyn TeamProfileResolver> =
            Arc::new(GatewayTeamProfileResolver::new(self.agent_teams.clone()));
        let sub_agent_dispatcher: Arc<dyn SubAgentDispatcher> =
            Arc::new(GatewaySubAgentDispatcher::new(
                Arc::downgrade(&self.bridge),
                self.thread_store.clone(),
                thread_creator.clone(),
                self.custom_agents.clone(),
            ));
        let agent_team_provider: Arc<dyn AgentLoopProvider> = Arc::new(AgentTeamProvider::new(
            provider_group_store,
            team_resolver,
            sub_agent_dispatcher,
        ));
        if let Err(error) = self
            .bridge
            .register_provider_blocking(AGENT_TEAM_PROVIDER_KEY, agent_team_provider)
        {
            panic!("failed to register AgentTeam provider during startup: {error}");
        }

        // The bridge needs the current Team registry snapshot so dispatch can
        // resolve `agent_id -> team` for thread routing. `apply_runtime_config`
        // also does this on config reload; we mirror it here so the very first
        // `build()` already has teams loaded before any request arrives.
        // Use `list_teams_blocking`/`list_agents_blocking` and reuse the
        // bridge's existing async `replace_*` helpers via `try_write` inside
        // their impls — those helpers hold their own tokio RwLock so we call
        // them from a synchronous context via a tiny block_on escape hatch.
        // This mirrors the intent of `apply_runtime_config` but is safe in
        // sync-build paths because `replace_*_profiles` only acquires a
        // tokio::sync::RwLock::write for a trivial map swap.
        let boot_agent_profiles = self.custom_agents.list_agents_blocking();
        let boot_team_profiles = self.agent_teams.list_teams_blocking();
        let bridge_for_profiles = self.bridge.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                bridge_for_profiles
                    .replace_agent_profiles(boot_agent_profiles)
                    .await;
                bridge_for_profiles
                    .replace_team_profiles(boot_team_profiles)
                    .await;
            });
        } else {
            warn!(
                "AppStateBuilder::build invoked outside a tokio runtime; \
                 agent/team profiles will be pushed to bridge on first apply_runtime_config"
            );
        }

        let router = Arc::new(Mutex::new(router));
        {
            let mut manager = self
                .channel_plugin_manager
                .try_lock()
                .expect("channel plugin manager lock must be uncontended during build");
            manager.attach_dispatcher(self.channel_swap.clone());
            let discoverer = BuiltInPluginDiscoverer::new(
                self.config.channels.clone(),
                router.clone(),
                self.bridge.clone(),
                String::new(),
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
        Arc::new(AppState {
            runtime: RuntimeState {
                start_time,
                health_checker: HealthChecker::new(start_time),
                live_config,
            },
            threads: ThreadState {
                thread_store: self.thread_store,
                history: self.thread_history,
                message_ledger: self.message_ledger,
                router,
            },
            ops: OpsState {
                events,
                restart_tracker: Mutex::new(RestartTracker::new()),
                settings_mutex: Mutex::new(()),
                cron_service: self.cron_service,
                heartbeat_service: self.heartbeat_service,
                config_path: self.config_path,
                restart_tokens: self.restart_tokens,
                mcp_tool_metrics: Arc::new(McpToolMetrics::default()),
                thread_logs: self.thread_logs,
                skills: self.skills,
                auto_research: self.auto_research,
                custom_agents: self.custom_agents,
                agent_teams: self.agent_teams,
                agent_team_group_store: group_store,
                wikis: self.wikis,
            },
            integration: IntegrationState {
                bridge: self.bridge,
                channel_dispatcher: Arc::new(ChannelDispatcherCell::new(self.channel_dispatcher)),
                channel_swap: self.channel_swap,
                channel_plugin_manager: self.channel_plugin_manager,
            },
        })
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
