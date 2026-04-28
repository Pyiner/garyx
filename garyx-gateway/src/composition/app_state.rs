use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::BridgeError;
use garyx_channels::{
    ChannelDispatcher, ChannelDispatcherImpl, ChannelPluginManager, SwappableDispatcher,
};
use garyx_models::config::GaryxConfig;
use garyx_models::thread_logs::ThreadLogSink;
use garyx_router::{MessageLedgerStore, MessageRouter, ThreadHistoryRepository, ThreadStore};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
#[cfg(test)]
use tokio::sync::broadcast;
use tracing::warn;

use crate::agent_teams::AgentTeamStore;
use crate::api::RestartTracker;
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

pub struct RuntimeState {
    pub start_time: Instant,
    pub health_checker: HealthChecker,
    pub live_config: Arc<LiveConfigCell>,
}

pub struct ThreadState {
    pub thread_store: Arc<dyn ThreadStore>,
    pub history: Arc<ThreadHistoryRepository>,
    pub message_ledger: Arc<MessageLedgerStore>,
    pub router: Arc<Mutex<MessageRouter>>,
}

pub struct OpsState {
    pub events: Arc<EventStreamHub>,
    pub restart_tracker: Mutex<RestartTracker>,
    pub settings_mutex: Mutex<()>,
    pub cron_service: Option<Arc<CronService>>,
    pub heartbeat_service: Option<Arc<HeartbeatService>>,
    pub config_path: Option<PathBuf>,
    pub restart_tokens: Vec<String>,
    pub mcp_tool_metrics: Arc<McpToolMetrics>,
    pub thread_logs: Arc<dyn ThreadLogSink>,
    pub skills: Arc<SkillsService>,
    pub auto_research: Arc<AutoResearchStore>,
    pub custom_agents: Arc<CustomAgentStore>,
    pub agent_teams: Arc<AgentTeamStore>,
    /// Read-only handle to the AgentTeam provider's Group store. Sharing
    /// one instance between the provider and the gateway read path keeps
    /// the in-memory cache coherent — the gateway surfaces the same
    /// `child_thread_ids` the provider is updating.
    pub agent_team_group_store: Arc<dyn garyx_bridge::providers::agent_team::GroupStore>,
    pub wikis: Arc<WikiStore>,
}

pub struct IntegrationState {
    pub bridge: Arc<MultiProviderBridge>,
    pub channel_dispatcher: Arc<ChannelDispatcherCell>,
    /// Concrete handle for the production dispatcher. Written on
    /// config reload (via `SwappableDispatcher::store`) and read by
    /// [`crate::ChannelPluginManager::attach_dispatcher`] /
    /// `respawn_plugin` per §9.4. Tests that build a bespoke
    /// `ChannelDispatcherCell` (e.g. MCP helpers) keep this slot equal
    /// to the cell's initial content; divergence after that point is
    /// intentional (the cell mirrors the mock, the swap mirrors the
    /// real dispatcher state the manager drives).
    pub channel_swap: Arc<SwappableDispatcher>,
    /// Shared handle to the manager so HTTP endpoints can introspect
    /// subprocess plugins (e.g. `GET /api/channels/plugins` returns
    /// the schema-driven catalog the desktop UI renders).
    pub channel_plugin_manager: Arc<Mutex<ChannelPluginManager>>,
}

pub struct AppState {
    pub runtime: RuntimeState,
    pub threads: ThreadState,
    pub ops: OpsState,
    pub integration: IntegrationState,
}

impl AppState {
    pub fn config_snapshot(&self) -> Arc<GaryxConfig> {
        self.runtime.live_config.snapshot()
    }

    pub fn replace_config(&self, config: GaryxConfig) {
        self.runtime.live_config.replace(config);
    }

    pub fn channel_dispatcher(&self) -> Arc<dyn ChannelDispatcher> {
        self.integration.channel_dispatcher.snapshot()
    }

    pub fn replace_channel_dispatcher(&self, dispatcher: Arc<dyn ChannelDispatcher>) {
        self.integration.channel_dispatcher.replace(dispatcher);
    }

    /// Concrete [`SwappableDispatcher`] handle — the
    /// `ChannelPluginManager` uses this (via `attach_dispatcher`) to
    /// publish respawned subprocess-plugin senders per §9.4 without
    /// going through the dyn-trait cell.
    pub fn channel_dispatcher_swap(&self) -> Arc<SwappableDispatcher> {
        self.integration.channel_swap.clone()
    }

    /// Shared manager handle — HTTP endpoints lock this to introspect
    /// registered plugins. Same instance the binary's boot path mutates
    /// during discovery, so changes made via
    /// `register_subprocess_plugin` / `respawn_plugin` are visible
    /// immediately.
    pub fn channel_plugin_manager(&self) -> Arc<Mutex<ChannelPluginManager>> {
        self.integration.channel_plugin_manager.clone()
    }

    pub async fn apply_runtime_config(&self, config: GaryxConfig) -> Result<(), BridgeError> {
        self.integration
            .bridge
            .replace_agent_profiles(self.ops.custom_agents.list_agents().await)
            .await;
        self.integration
            .bridge
            .replace_team_profiles(self.ops.agent_teams.list_teams().await)
            .await;
        self.integration.bridge.reload_from_config(&config).await?;
        // Rebuild the built-in routes from the new config and publish
        // them through the `SwappableDispatcher` so the concrete swap
        // identity is preserved (§9.4: respawning plugins rely on it).
        // Subprocess-plugin senders that were previously forked into
        // the swap are dropped by this store; the plugin manager's
        // hot-reload path is responsible for re-registering them
        // afterwards. Heartbeat / cron hold a cast-to-trait view of
        // the same swap, so their visible dispatcher follows this
        // store with no further plumbing.
        let rebuilt = ChannelDispatcherImpl::from_config(&config.channels);
        self.integration.channel_swap.store(Arc::new(rebuilt));
        let dispatcher: Arc<dyn ChannelDispatcher> = self.integration.channel_swap.clone();
        // Push the new account snapshot to every registered plugin
        // through the channel-blind `ChannelPlugin::reload_accounts`
        // trait method. Built-ins rebuild their `OutboundSender`
        // map in place; subprocess plugins forward an
        // `accounts/reload` JSON-RPC (§6.5). Per-plugin failures
        // are collected + logged but don't abort the outer apply —
        // the plugin stays on its previous state until the next
        // config change retries.
        {
            let manager = self.integration.channel_plugin_manager.lock().await;
            let failures = manager.reload_plugin_accounts(&config.channels).await;
            for (plugin_id, err) in failures {
                tracing::warn!(
                    plugin_id = %plugin_id,
                    error = %err,
                    "reload_accounts failed; plugin left on previous snapshot",
                );
            }
        }
        let managed_mcp_servers = config.mcp_servers.clone();
        self.replace_channel_dispatcher(dispatcher.clone());
        self.replace_config(config.clone());
        {
            let mut router = self.threads.router.lock().await;
            router.update_config(config.clone());
        }
        self.threads
            .history
            .update_conversation_index_config(config.gateway.conversation_index.clone());
        if let Some(heartbeat_service) = &self.ops.heartbeat_service {
            heartbeat_service
                .set_dispatch_runtime(
                    self.threads.router.clone(),
                    self.integration.bridge.clone(),
                    dispatcher.clone(),
                    self.ops.thread_logs.clone(),
                    managed_mcp_servers.clone(),
                )
                .await;
        }
        if let Some(cron_service) = &self.ops.cron_service {
            cron_service
                .set_dispatch_runtime(
                    self.threads.thread_store.clone(),
                    self.threads.router.clone(),
                    self.integration.bridge.clone(),
                    dispatcher,
                    self.ops.thread_logs.clone(),
                    managed_mcp_servers,
                    self.ops.custom_agents.clone(),
                    self.ops.agent_teams.clone(),
                )
                .await;
        }
        Ok(())
    }

    pub fn sync_external_user_skills_before_run(&self, source: &str, thread_id: &str) {
        if let Err(error) = self.ops.skills.sync_external_user_skills() {
            warn!(
                error = %error,
                source,
                thread_id,
                "failed to sync external user skills before provider run"
            );
        }
    }

    #[cfg(test)]
    pub fn clone_for_test(&self) -> Self {
        let (event_tx, _) = broadcast::channel(128);
        Self {
            runtime: RuntimeState {
                start_time: self.runtime.start_time,
                health_checker: HealthChecker::new(self.runtime.start_time),
                live_config: self.runtime.live_config.clone(),
            },
            threads: ThreadState {
                thread_store: self.threads.thread_store.clone(),
                history: self.threads.history.clone(),
                message_ledger: self.threads.message_ledger.clone(),
                router: self.threads.router.clone(),
            },
            ops: OpsState {
                events: EventStreamHub::new(event_tx),
                restart_tracker: Mutex::new(RestartTracker::new()),
                settings_mutex: Mutex::new(()),
                cron_service: self.ops.cron_service.clone(),
                heartbeat_service: self.ops.heartbeat_service.clone(),
                config_path: self.ops.config_path.clone(),
                restart_tokens: self.ops.restart_tokens.clone(),
                mcp_tool_metrics: self.ops.mcp_tool_metrics.clone(),
                thread_logs: self.ops.thread_logs.clone(),
                skills: self.ops.skills.clone(),
                auto_research: self.ops.auto_research.clone(),
                custom_agents: self.ops.custom_agents.clone(),
                agent_teams: self.ops.agent_teams.clone(),
                agent_team_group_store: self.ops.agent_team_group_store.clone(),
                wikis: self.ops.wikis.clone(),
            },
            integration: IntegrationState {
                bridge: self.integration.bridge.clone(),
                channel_dispatcher: self.integration.channel_dispatcher.clone(),
                channel_swap: self.integration.channel_swap.clone(),
                channel_plugin_manager: self.integration.channel_plugin_manager.clone(),
            },
        }
    }
}
#[cfg(test)]
#[path = "../app_state_tests.rs"]
mod app_state_tests;
