use std::sync::Arc;

use axum::http::StatusCode;

use crate::application::chat::contracts::{InterruptResponse, StreamInputResponse};
use crate::chat_shared::{interrupt_response, stream_input_response};
use crate::server::AppState;

pub(crate) async fn execute_chat_interrupt(
    state: &Arc<AppState>,
    thread_id: String,
) -> InterruptResponse {
    let bridge = &state.integration.bridge;
    let streaming_interrupted = bridge.interrupt_streaming_session(&thread_id).await;

    if streaming_interrupted {
        return interrupt_response("interrupted", thread_id, vec![]);
    }

    let (aborted_any, aborted_runs) = bridge.abort_thread_runs(&thread_id).await;
    let status = if aborted_any {
        "interrupted"
    } else {
        "not_found"
    };

    interrupt_response(status, thread_id, aborted_runs)
}

pub(crate) async fn execute_chat_stream_input(
    state: &Arc<AppState>,
    thread_id: String,
    message: String,
    attachments: Vec<garyx_models::provider::PromptAttachment>,
    images: Vec<garyx_models::provider::ImagePayload>,
    files: Vec<garyx_models::provider::FilePayload>,
) -> (StatusCode, StreamInputResponse) {
    let bridge = &state.integration.bridge;
    let effective_message = state
        .config_snapshot()
        .resolve_slash_command(&message)
        .and_then(|command| command.prompt)
        .unwrap_or(message);

    let queued = bridge
        .add_streaming_input(
            &thread_id,
            &effective_message,
            Some(images),
            Some(files),
            Some(attachments),
        )
        .await;

    let (status, thread_status, pending_input_id) = if let Some(result) = queued {
        let pending_input_id = if result.pending_input_id.trim().is_empty() {
            None
        } else {
            Some(result.pending_input_id)
        };
        ("queued", "queued", pending_input_id)
    } else {
        ("no_active_session", "no_active_thread", None)
    };
    (
        StatusCode::OK,
        stream_input_response(
            status,
            Some(thread_status.to_owned()),
            pending_input_id,
            thread_id,
        ),
    )
}
