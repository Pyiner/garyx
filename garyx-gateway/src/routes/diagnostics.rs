//! Per-thread diagnostics handler.

use super::*;
use crate::server::AppState;
use crate::thread_runtime::build_thread_runtime_summary;
use axum::Json;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use garyx_router::ThreadStoreExt;
use garyx_router::bindings_from_value;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ThreadDiagnosticsParams {
    pub thread_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

pub async fn thread_diagnostics(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ThreadDiagnosticsParams>,
) -> impl IntoResponse {
    let thread_id = params.thread_id.trim();
    if thread_id.is_empty() {
        return Json(json!({
            "ok": false,
            "reason": "missing-thread-id",
        }));
    }

    let limit = params.limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    let thread_value = state.threads.thread_store.get_logged(thread_id).await;
    let bindings = thread_value
        .as_ref()
        .map(bindings_from_value)
        .unwrap_or_default()
        .into_iter()
        .map(|binding| {
            let delivery_thread_id =
                crate::routes::binding_delivery_thread_id(&binding.binding_key, &binding.chat_id);
            json!({
                "channel": binding.channel,
                "account_id": binding.account_id,
                "chat_id": binding.chat_id,
                "binding_key": binding.binding_key,
                "peer_id": binding.binding_key,
                "thread_binding_key": binding.binding_key,
                "thread_scope": delivery_thread_id,
                "delivery_target_type": binding.delivery_target_type,
                "delivery_target_id": binding.delivery_target_id,
            })
        })
        .collect::<Vec<_>>();

    let transcript_path = state
        .threads
        .history
        .transcript_store()
        .transcript_path(thread_id)
        .map(|path| path.display().to_string());

    let ledger = match state
        .threads
        .message_ledger
        .records_for_thread(thread_id, limit)
        .await
    {
        Ok(records) => json!({
            "ok": true,
            "records": records,
        }),
        Err(error) => json!({
            "ok": false,
            "error": error.to_string(),
            "records": [],
        }),
    };

    Json(json!({
        "ok": true,
        "thread_id": thread_id,
        "thread": thread_value,
        "thread_runtime": build_thread_runtime_summary(&state, thread_value.as_ref()).await,
        "bindings": bindings,
        "history": thread_history_for_key(&state, thread_id, limit, true, None, None, None).await,
        "message_ledger": ledger,
        "transcript_path": transcript_path,
    }))
}
