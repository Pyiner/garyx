use std::sync::Arc;

use crate::server::AppState;

pub(crate) fn start_gateway_runtime(state: Arc<AppState>) {
    crate::loop_continuation::spawn_listener(state.clone());
    crate::task_notifications::spawn_listener(state);
}
