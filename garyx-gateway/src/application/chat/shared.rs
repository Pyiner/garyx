use std::sync::Arc;

use garyx_models::thread_logs::ThreadLogEvent;

use crate::application::chat::contracts::{InterruptResponse, StreamInputResponse};
use crate::server::AppState;

pub(crate) fn stream_input_response(
    status: impl Into<String>,
    thread_status: Option<String>,
    pending_input_id: Option<String>,
    thread_id: String,
) -> StreamInputResponse {
    StreamInputResponse {
        status: status.into(),
        thread_status,
        pending_input_id,
        thread_id,
    }
}

pub(crate) fn interrupt_response(
    status: impl Into<String>,
    thread_id: String,
    aborted_runs: Vec<String>,
) -> InterruptResponse {
    InterruptResponse {
        status: status.into(),
        thread_id,
        aborted_runs,
    }
}

pub(crate) async fn record_api_thread_log(state: &Arc<AppState>, event: ThreadLogEvent) {
    state.ops.thread_logs.record_event(event).await;
}
