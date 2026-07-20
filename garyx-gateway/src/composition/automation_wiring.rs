//! Automation scheduler wiring: the one place that connects the cron engine
//! to the assembled [`AppState`].
//!
//! The engine (`crate::automation::engine`) has no `AppState` knowledge — it executes
//! against an [`AutomationExecEnv`] whose gateway-state operations go through
//! the narrow [`AutomationDispatchPort`]. This module implements that port
//! over a `Weak<AppState>` and builds the env from live state, so the
//! back-reference is confined to the composition seam and injected exactly
//! once at scheduler start (no `OnceLock` late-mutation channels).

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use garyx_models::provider::AgentDispatchOutcome;

use crate::app_state::AppState;
use crate::automation::dispatch::{
    AutomationDispatchError, AutomationDispatchPort, AutomationExecEnv,
};
use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};

/// [`AutomationDispatchPort`] implementation over the assembled gateway state.
///
/// Holds a `Weak` handle: the scheduler is a driver of the application, not a
/// keep-alive owner. When the state is gone (shutdown), every operation
/// degrades to its explicit unavailable result instead of executing against a
/// half-torn-down gateway.
struct AppStateAutomationPort {
    state: Weak<AppState>,
}

#[async_trait::async_trait]
impl AutomationDispatchPort for AppStateAutomationPort {
    fn provider_runtime_ready(&self) -> bool {
        self.state
            .upgrade()
            .map(|state| state.provider_runtime_ready())
            .unwrap_or(false)
    }

    async fn invalidate_gateway_sync_caches(&self) {
        if let Some(state) = self.state.upgrade() {
            state.invalidate_gateway_sync_caches().await;
        }
    }

    async fn dispatch_internal_message(
        &self,
        thread_id: &str,
        run_id: &str,
        message: &str,
        extra_metadata: HashMap<String, serde_json::Value>,
    ) -> Result<AgentDispatchOutcome, AutomationDispatchError> {
        let Some(state) = self.state.upgrade() else {
            return Err(AutomationDispatchError::StateUnavailable);
        };
        dispatch_internal_message_to_thread(
            &state,
            thread_id,
            run_id,
            message,
            InternalDispatchOptions {
                extra_metadata,
                ..Default::default()
            },
        )
        .await
        .map_err(AutomationDispatchError::Dispatch)
    }
}

/// Build the automation execution environment from the assembled state.
///
/// Every handle is the same stable `Arc` the rest of the gateway uses; the
/// retired `set_dispatch_runtime` re-injection on config apply is unnecessary
/// because none of these identities change across hot reloads.
pub(crate) fn automation_exec_env(state: &Arc<AppState>) -> AutomationExecEnv {
    AutomationExecEnv {
        thread_store: state.threads.thread_store.clone(),
        router: state.threads.router.clone(),
        bridge: state.integration.bridge.clone(),
        thread_logs: state.ops.thread_logs.clone(),
        custom_agents: state.ops.custom_agents.clone(),
        garyx_db: Some(state.ops.garyx_db.clone()),
        port: Arc::new(AppStateAutomationPort {
            state: Arc::downgrade(state),
        }),
    }
}

/// Start the automation scheduler loop against the assembled state.
///
/// Called once after `AppStateBuilder::build` (and a successful
/// `CronService::load`). Returns `false` when no cron service is configured.
pub fn start_automation_scheduler(state: &Arc<AppState>) -> bool {
    match state.ops.cron_service.as_ref() {
        Some(service) => {
            service.start(automation_exec_env(state));
            true
        }
        None => false,
    }
}
