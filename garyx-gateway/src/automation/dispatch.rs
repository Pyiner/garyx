//! Execution-time contracts between the automation engine and the assembled
//! gateway state.
//!
//! [`AutomationDispatchPort`] is the narrow set of gateway-state operations a
//! firing job needs; [`AutomationExecEnv`] bundles the stable runtime handles
//! plus that port. Both are implemented/constructed at the composition seam
//! (`composition::automation_wiring`) — the engine itself has no `AppState`
//! knowledge.

use std::collections::HashMap;
use std::sync::Arc;

use garyx_bridge::MultiProviderBridge;
use garyx_models::provider::AgentDispatchOutcome;
use garyx_models::thread_logs::ThreadLogSink;
use garyx_router::{MessageRouter, ThreadStore};

use crate::custom_agents::CustomAgentStore;
use crate::garyx_db::GaryxDbService;

/// Error from [`AutomationDispatchPort::dispatch_internal_message`].
#[derive(Debug)]
pub(crate) enum AutomationDispatchError {
    /// The owning gateway state is gone (shutdown); non-retryable.
    StateUnavailable,
    /// The front-door dispatch itself failed; the string is the routed error.
    Dispatch(String),
}

/// Narrow gateway-state operations the scheduler needs at execution time.
///
/// The engine has no `AppState` knowledge: this port is implemented at the
/// composition layer (`composition::automation_wiring`), which is the only
/// place allowed to hold a handle back to the assembled application state.
#[async_trait::async_trait]
pub(crate) trait AutomationDispatchPort: Send + Sync {
    /// Whether the provider runtime finished starting. Gates execution of
    /// jobs that need a live provider runtime.
    fn provider_runtime_ready(&self) -> bool;

    /// Invalidate gateway sync caches after an automation thread was created.
    async fn invalidate_gateway_sync_caches(&self);

    /// Inject a synthetic user turn into `thread_id` through the
    /// internal-inbound front door (router inbound semantics, transcript user
    /// turn, busy queueing, channel echo).
    async fn dispatch_internal_message(
        &self,
        thread_id: &str,
        run_id: &str,
        message: &str,
        extra_metadata: HashMap<String, serde_json::Value>,
    ) -> Result<AgentDispatchOutcome, AutomationDispatchError>;
}

/// Execution environment for scheduled jobs.
///
/// Constructed once from the assembled application state
/// (`composition::automation_wiring::automation_exec_env`) and handed to the
/// scheduler loop at [`CronService::start`] — or per call for
/// [`CronService::run_now`]. Replaces the retired trio of late-injection
/// channels (`set_app_state` / `set_garyx_db` / `set_dispatch_runtime`): a
/// scheduler that could observe a half-injected runtime no longer exists by
/// construction.
#[derive(Clone)]
pub(crate) struct AutomationExecEnv {
    pub(crate) thread_store: Arc<dyn ThreadStore>,
    pub(crate) router: Arc<tokio::sync::Mutex<MessageRouter>>,
    pub(crate) bridge: Arc<MultiProviderBridge>,
    pub(crate) thread_logs: Arc<dyn ThreadLogSink>,
    pub(crate) custom_agents: Arc<CustomAgentStore>,
    /// `None` skips automation thread-run association recording (tests
    /// without a gateway database).
    pub(crate) garyx_db: Option<Arc<GaryxDbService>>,
    pub(crate) port: Arc<dyn AutomationDispatchPort>,
}
