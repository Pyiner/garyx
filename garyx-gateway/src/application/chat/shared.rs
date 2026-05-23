use std::sync::Arc;

use garyx_models::thread_logs::ThreadLogEvent;
use serde_json::json;

use crate::application::chat::contracts::{InterruptResponse, StreamInputResponse};
use crate::server::AppState;

pub(crate) fn stream_input_response(
    status: impl Into<String>,
    thread_status: Option<String>,
    client_intent_id: Option<String>,
    pending_input_id: Option<String>,
    thread_id: String,
) -> StreamInputResponse {
    StreamInputResponse {
        status: status.into(),
        thread_status,
        client_intent_id,
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

pub(crate) fn emit_thread_title_updated_event(
    state: &Arc<AppState>,
    thread_id: &str,
    run_id: Option<&str>,
    title: &str,
) {
    let thread_id = thread_id.trim();
    let title = title.trim();
    if thread_id.is_empty() || title.is_empty() {
        return;
    }
    let _ = state.ops.events.sender().send(
        json!({
            "type": "thread_title_updated",
            "thread_id": thread_id,
            "run_id": run_id.unwrap_or_default(),
            "title": title,
        })
        .to_string(),
    );
}
