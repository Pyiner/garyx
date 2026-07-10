use std::sync::Arc;

use crate::server::AppState;

pub(crate) fn start_gateway_runtime(state: Arc<AppState>) {
    crate::task_notifications::spawn_listener(state.clone());
    crate::quota_resend::spawn_reactor(state);
}
