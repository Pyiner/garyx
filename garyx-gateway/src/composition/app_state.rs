use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::BridgeError;
use garyx_channels::{
    ChannelDispatcher, ChannelDispatcherImpl, ChannelPluginManager, SwappableDispatcher,
};
use garyx_models::config::GaryxConfig;
use garyx_models::thread_logs::ThreadLogSink;
use garyx_router::{
    KnownChannelEndpoint, MessageLedgerStore, MessageRouter, ThreadHistoryRepository, ThreadStore,
};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::sync::Notify;
#[cfg(test)]
use tokio::sync::broadcast;
use tracing::{debug, warn};

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
use crate::routes::RestartTracker;
use crate::runtime_cells::{ChannelDispatcherCell, LiveConfigCell};
use crate::skills::SkillsService;
use crate::sqlite_thread_store::SqliteThreadStore;
use crate::thread_lifecycle::LifecycleService;

pub struct RuntimeState {
    pub start_time: Instant,
    pub server_boot_id: String,
    pub health_checker: HealthChecker,
    pub live_config: Arc<LiveConfigCell>,
    pub provider_runtime_ready: Arc<AtomicBool>,
    pub provider_runtime_ready_notify: Arc<Notify>,
}

pub struct ThreadState {
    pub thread_store: Arc<dyn ThreadStore>,
    pub(crate) sqlite_thread_store: Option<Arc<SqliteThreadStore>>,
    pub history: Arc<ThreadHistoryRepository>,
    pub message_ledger: Arc<MessageLedgerStore>,
    pub router: Arc<Mutex<MessageRouter>>,
}

pub struct OpsState {
    pub events: Arc<EventStreamHub>,
    pub restart_tracker: Mutex<RestartTracker>,
    pub settings_mutex: Mutex<()>,
    pub cron_service: Option<Arc<CronService>>,
    pub config_path: Option<PathBuf>,
    pub restart_tokens: Vec<String>,
    pub mcp_tool_metrics: Arc<McpToolMetrics>,
    pub thread_logs: Arc<dyn ThreadLogSink>,
    pub skills: Arc<SkillsService>,
    pub custom_agents: Arc<CustomAgentStore>,
    pub garyx_db: Arc<GaryxDbService>,
    pub(crate) conversation_admission: ConversationAdmissionService,
    pub(crate) prompt_attachments: PromptAttachmentLifecycle,
    pub meetings: Arc<MeetingService>,
    pub provider_auth_sessions: Arc<ClaudeAuthSessionStore>,
    pub channel_endpoint_snapshot: Mutex<Option<ChannelEndpointSnapshotCache>>,
    pub(crate) endpoint_binding_mutator: Arc<SqlEndpointBindingMutator>,
    pub(crate) lifecycle: Arc<LifecycleService>,
}

pub struct ChannelEndpointSnapshotCache {
    endpoints: Vec<KnownChannelEndpoint>,
    expires_at: Instant,
}

const GATEWAY_SYNC_SNAPSHOT_TTL: Duration = Duration::from_secs(5);

/// Boot-installed hook that rebuilds the channel plugin manager
/// (built-in runtime restart + subprocess discovery/respawn) for a
/// freshly applied config. The rebuild implementation lives in the
/// binary crate; injecting it here makes `apply_runtime_config` the
/// single derivation point both the HTTP settings path and the config
/// file watcher converge on (Phase-7).
pub type ChannelPluginRebuilder = Arc<
    dyn Fn(GaryxConfig) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync,
>;

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
    /// See [`ChannelPluginRebuilder`]. `None` until the binary installs
    /// it at boot; embedded/test states without a plugin runtime simply
    /// never set it. Shared across state re-compositions.
    pub channel_plugin_rebuilder: Arc<std::sync::OnceLock<ChannelPluginRebuilder>>,
    /// Process-wide plugin self-update master switch. Handlers hold
    /// this exact Arc, and `apply_runtime_config` refreshes it on
    /// every apply — a `plugins.auto_update` edit takes effect without
    /// any channel-plugin rebuild (Phase-7 review F1).
    pub plugin_auto_update_enabled: Arc<AtomicBool>,
}

pub struct AppState {
    pub runtime: RuntimeState,
    pub threads: ThreadState,
    pub ops: OpsState,
    pub integration: IntegrationState,
}

impl AppState {
    pub fn server_boot_id(&self) -> &str {
        &self.runtime.server_boot_id
    }

    pub fn config_snapshot(&self) -> Arc<GaryxConfig> {
        self.runtime.live_config.snapshot()
    }

    pub fn replace_config(&self, config: GaryxConfig) {
        self.runtime.live_config.replace(config);
    }

    /// Install the boot-time channel plugin rebuilder. First caller
    /// wins; later calls are ignored (the hook is process-lifetime).
    pub fn set_channel_plugin_rebuilder(&self, rebuilder: ChannelPluginRebuilder) {
        let _ = self.integration.channel_plugin_rebuilder.set(rebuilder);
    }

    /// The process-wide plugin self-update master switch handlers
    /// share. See [`IntegrationState::plugin_auto_update_enabled`].
    pub fn plugin_auto_update_enabled(&self) -> Arc<AtomicBool> {
        self.integration.plugin_auto_update_enabled.clone()
    }

    /// The explicit projection of config inputs whose change requires a
    /// channel-plugin rebuild (built-in runtime restart + subprocess
    /// discovery/respawn): the channels section itself and the public
    /// URL baked into each subprocess plugin's HostContext. Everything
    /// else is either hot (dispatcher/bridge/meeting knobs, the plugin
    /// auto-update switch, per-account reload_accounts) or explicitly
    /// restart-required or currently unconsumed at runtime
    /// (e.g. plugins.auto_update_check_interval, which today has no
    /// runtime consumer beyond configuration display).
    fn channel_plugin_rebuild_inputs(config: &GaryxConfig) -> serde_json::Value {
        serde_json::json!({
            "channels": serde_json::to_value(&config.channels).ok(),
            "public_url": config.gateway.public_url,
        })
    }

    pub fn provider_runtime_ready(&self) -> bool {
        self.runtime.provider_runtime_ready.load(Ordering::Acquire)
    }

    pub fn mark_provider_runtime_ready(&self) {
        self.runtime
            .provider_runtime_ready
            .store(true, Ordering::Release);
        self.runtime.provider_runtime_ready_notify.notify_waiters();
    }

    pub async fn wait_for_provider_runtime_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.provider_runtime_ready() {
                return true;
            }

            let notified = self.runtime.provider_runtime_ready_notify.notified();
            if self.provider_runtime_ready() {
                return true;
            }

            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(now);
            if tokio::time::timeout(remaining, notified).await.is_err() {
                return self.provider_runtime_ready();
            }
        }
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

    /// Count thread records with a SQL COUNT over the `thread::` prefix —
    /// no key listing and no record bodies (#TASK-2099).
    pub async fn thread_record_count(&self) -> Result<usize, garyx_router::ThreadStoreError> {
        self.threads.thread_store.count_keys(Some("thread::")).await
    }

    /// Fresh, uncached endpoint read for request boundaries whose
    /// response IS the endpoint data: a live store/projection failure
    /// must surface even when a snapshot was cached moments ago
    /// (#TASK-2134). On success the snapshot cache is refreshed so hot
    /// resolution paths benefit.
    pub async fn channel_endpoints_fresh(
        &self,
    ) -> Result<Vec<KnownChannelEndpoint>, garyx_router::ThreadStoreError> {
        let endpoints =
            garyx_router::list_known_channel_endpoints(&self.threads.thread_store).await?;
        let mut cache = self.ops.channel_endpoint_snapshot.lock().await;
        *cache = Some(ChannelEndpointSnapshotCache {
            endpoints: endpoints.clone(),
            expires_at: Instant::now() + GATEWAY_SYNC_SNAPSHOT_TTL,
        });
        Ok(endpoints)
    }

    /// Snapshot-cached endpoint listing for hot resolution paths
    /// (message dispatch, bind/unbind target resolution): a cache hit
    /// may serve a snapshot up to the TTL old, and the actions taken on
    /// it surface storage failures themselves when they touch the
    /// store. Failures are never cached and propagate as `Err`
    /// (#TASK-2128). Request boundaries that RETURN endpoint data must
    /// use `channel_endpoints_fresh` instead (#TASK-2134); fire-and-
    /// forget callers opt into degradation explicitly at their own call
    /// sites.
    pub async fn cached_channel_endpoints(
        &self,
    ) -> Result<Vec<KnownChannelEndpoint>, garyx_router::ThreadStoreError> {
        let now = Instant::now();
        let mut cache = self.ops.channel_endpoint_snapshot.lock().await;
        if let Some(snapshot) = cache.as_ref()
            && snapshot.expires_at > now
        {
            debug!(
                endpoint_count = snapshot.endpoints.len(),
                "channel endpoint snapshot cache hit"
            );
            return Ok(snapshot.endpoints.clone());
        }

        let started = Instant::now();
        // Projection rows + known-endpoint registry, via the router's
        // projection-backed listing (the SQL endpoint projection is
        // store-owned for the SQLite store).
        let endpoints =
            garyx_router::list_known_channel_endpoints(&self.threads.thread_store).await?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        debug!(
            elapsed_ms,
            endpoint_count = endpoints.len(),
            "channel endpoint snapshot refreshed"
        );
        *cache = Some(ChannelEndpointSnapshotCache {
            endpoints: endpoints.clone(),
            expires_at: Instant::now() + GATEWAY_SYNC_SNAPSHOT_TTL,
        });
        Ok(endpoints)
    }

    pub async fn invalidate_channel_endpoint_cache(&self) {
        let mut cache = self.ops.channel_endpoint_snapshot.lock().await;
        if cache.take().is_some() {
            debug!("channel endpoint snapshot cache invalidated");
        }
    }

    pub async fn invalidate_gateway_sync_caches(&self) {
        self.invalidate_channel_endpoint_cache().await;
    }

    pub fn spawn_gateway_sync_cache_warmup(self: &Arc<Self>) {
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let started = Instant::now();
            // No startup index rebuild/reconciliation: the router's
            // endpoint routing map is a lazy per-endpoint cache over the
            // SQL endpoint projection, and projections derive inside the
            // same transaction as every record write, so a repair pass has
            // nothing left to repair (#TASK-2099).
            // RuntimeAssembler already settled orphaned running rows while
            // holding the data-dir lock and before listener bind. A read-side
            // warmup must never repeat destructive startup work.
            let threads = state.thread_record_count().await.unwrap_or_else(|error| {
                warn!(error = %error, "failed to count thread records at startup");
                0
            });
            let endpoints = match state.cached_channel_endpoints().await {
                Ok(endpoints) => endpoints.len(),
                Err(error) => {
                    warn!(error = %error, "failed to warm channel endpoint snapshot at startup");
                    0
                }
            };
            debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                thread_count = threads,
                endpoint_count = endpoints,
                "gateway sync snapshots warmed"
            );
        });
    }

    pub async fn apply_runtime_config(&self, config: GaryxConfig) -> Result<(), BridgeError> {
        // Rebuild-inputs gate for the plugin rebuild below: unrelated
        // config saves (shortcuts, MCP servers, …) must not bounce
        // live channel connections. The gate compares the explicit
        // rebuild-input projection, not just `channels`.
        let rebuild_inputs_changed =
            Self::channel_plugin_rebuild_inputs(self.config_snapshot().as_ref())
                != Self::channel_plugin_rebuild_inputs(&config);
        let projection = RuntimeConfigProjection::from_config(&config);
        self.integration
            .bridge
            .replace_agent_profiles(self.ops.custom_agents.snapshot().await)
            .await;
        self.integration.bridge.reload_from_config(&config).await?;
        // Publish the meeting knobs only after the fallible bridge reconcile
        // above: a rejected candidate config must not leak partial runtime
        // state. The join-retry window used to refresh only on the
        // file-watcher path (via the channel-plugin rebuild), so API-driven
        // settings saves left it stale until restart.
        self.ops
            .meetings
            .set_read_page_bytes(projection.meeting_read_page_bytes);
        self.ops
            .meetings
            .set_ingestion_join_retry_window(projection.meeting_join_retry_window_secs);
        // Hot knob: handlers share this Arc, so a plugins.auto_update
        // edit takes effect immediately with zero channel bounce.
        self.integration
            .plugin_auto_update_enabled
            .store(projection.plugin_auto_update, Ordering::Release);
        // Rebuild the built-in routes from the new config and publish
        // them through the `SwappableDispatcher` so the concrete swap
        // identity is preserved (§9.4: respawning plugins rely on it).
        // `from_config` only seeds built-in channels declared in
        // `GaryxConfig`, so any subprocess-plugin sender previously
        // forked into the swap via `register_subprocess_plugin` /
        // `respawn_plugin` would be silently dropped by the store.
        // Snapshot them first and re-seed the rebuilt dispatcher before
        // publishing, so outbound traffic to subprocess plugins keeps
        // working across config reloads. Cron holds a cast-to-trait
        // view of the same swap, so its visible dispatcher follows this
        // store with no further plumbing.
        let dispatcher_snapshot = self.integration.channel_swap.load();
        let preserved_plugin_senders = dispatcher_snapshot.plugin_senders_snapshot();
        let mut rebuilt = match dispatcher_snapshot.channel_running_handle("weixin") {
            Some(running) => {
                ChannelDispatcherImpl::from_config_with_weixin_running(&config.channels, running)
            }
            None => ChannelDispatcherImpl::from_config(&config.channels),
        };
        for sender in preserved_plugin_senders {
            let plugin_id = sender.plugin_id().to_owned();
            if let Err(error) = rebuilt.register_plugin(sender) {
                tracing::warn!(
                    plugin_id = %plugin_id,
                    error = %error,
                    "failed to preserve subprocess plugin sender across config reload"
                );
            }
        }
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
        self.replace_channel_dispatcher(dispatcher);
        self.replace_config(config.clone());
        {
            let mut router = self.threads.router.lock().await;
            router.update_config(config.clone());
        }
        // The automation scheduler needs no re-injection here: its execution
        // environment is built once from these same stable Arcs at scheduler
        // start (composition::automation_wiring), and the readiness gate below
        // is observed live through its dispatch port.
        self.mark_provider_runtime_ready();
        // Phase-7 single derivation point: when the rebuild-inputs
        // projection (channels section + gateway.public_url, see
        // channel_plugin_rebuild_inputs) actually changed, run the
        // assembly-installed plugin-manager rebuild (built-in runtime
        // restart + subprocess discovery/respawn). Both the HTTP
        // settings path and the file watcher reach it through this
        // one hook; failures are
        // logged, matching the historical watcher-path semantics —
        // the applied dispatcher/bridge state above is already live.
        if rebuild_inputs_changed
            && let Some(rebuilder) = self.integration.channel_plugin_rebuilder.get()
            && let Err(error) = rebuilder(config).await
        {
            tracing::warn!(
                error = %error,
                "channel plugin rebuild after config apply failed"
            );
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
                server_boot_id: self.runtime.server_boot_id.clone(),
                health_checker: HealthChecker::new(self.runtime.start_time),
                live_config: self.runtime.live_config.clone(),
                provider_runtime_ready: self.runtime.provider_runtime_ready.clone(),
                provider_runtime_ready_notify: self.runtime.provider_runtime_ready_notify.clone(),
            },
            threads: ThreadState {
                thread_store: self.threads.thread_store.clone(),
                sqlite_thread_store: self.threads.sqlite_thread_store.clone(),
                history: self.threads.history.clone(),
                message_ledger: self.threads.message_ledger.clone(),
                router: self.threads.router.clone(),
            },
            ops: OpsState {
                events: EventStreamHub::new(event_tx),
                restart_tracker: Mutex::new(RestartTracker::new()),
                settings_mutex: Mutex::new(()),
                cron_service: self.ops.cron_service.clone(),
                config_path: self.ops.config_path.clone(),
                restart_tokens: self.ops.restart_tokens.clone(),
                mcp_tool_metrics: self.ops.mcp_tool_metrics.clone(),
                thread_logs: self.ops.thread_logs.clone(),
                skills: self.ops.skills.clone(),
                custom_agents: self.ops.custom_agents.clone(),
                garyx_db: self.ops.garyx_db.clone(),
                conversation_admission: self.ops.conversation_admission.clone(),
                prompt_attachments: self.ops.prompt_attachments.clone(),
                meetings: self.ops.meetings.clone(),
                provider_auth_sessions: self.ops.provider_auth_sessions.clone(),
                channel_endpoint_snapshot: Mutex::new(None),
                endpoint_binding_mutator: self.ops.endpoint_binding_mutator.clone(),
                lifecycle: self.ops.lifecycle.clone(),
            },
            integration: IntegrationState {
                bridge: self.integration.bridge.clone(),
                channel_dispatcher: self.integration.channel_dispatcher.clone(),
                channel_swap: self.integration.channel_swap.clone(),
                channel_plugin_manager: self.integration.channel_plugin_manager.clone(),
                channel_plugin_rebuilder: self.integration.channel_plugin_rebuilder.clone(),
                plugin_auto_update_enabled: self.integration.plugin_auto_update_enabled.clone(),
            },
        }
    }
}
#[cfg(test)]
#[path = "../app_state_tests.rs"]
mod app_state_tests;
