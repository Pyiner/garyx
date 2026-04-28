use std::sync::Arc;

use crate::server::AppState;

pub(crate) fn start_gateway_runtime(state: Arc<AppState>) {
    crate::loop_continuation::spawn_listener(state.clone());
    crate::restart_resume::spawn_replay(state);
}
