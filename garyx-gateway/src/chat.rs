//! API chat channel routes.
//!
//! Rust port of `src/garyx/plugins/channels/api.py`.
//! Provides HTTP endpoints for sending messages and receiving responses.

use std::sync::{Arc, Mutex};

use crate::application::chat::contracts::{
    ChatRequest, InterruptRequest, StreamInputRequest, resolve_existing_thread_key,
};
use crate::chat_application::{ChatPreparationError, prepare_chat_request};
use crate::chat_control::{execute_chat_interrupt, execute_chat_stream_input};
use crate::chat_delivery::{BoundThreadDeliveryBuffer, message_tool_mirror_text};
use axum::Json;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use garyx_models::MessageLifecycleStatus;
use garyx_models::provider::{AgentRunRequest, StreamBoundaryKind, StreamEvent};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent_team_provider::AGENT_TEAM_PROVIDER_KEY;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// GET /api/chat/health - API channel health check.
pub async fn chat_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // `bridge_ready` reports whether any *user-visible* provider is wired
    // up; the AgentTeam meta-provider is registered unconditionally at boot
    // and is an internal dispatch target, not a user-facing provider — so
    // exclude it here so "no providers configured" still reports as not
    // ready. See `agent_team_provider::AGENT_TEAM_PROVIDER_KEY`.
    let bridge_ready = state
        .integration
        .bridge
        .provider_keys()
        .await
        .iter()
        .any(|key| key != AGENT_TEAM_PROVIDER_KEY);
    Json(json!({
        "status": "ok",
        "channel": "api",
        "bridge_ready": bridge_ready,
    }))
}

/// GET /api/chat/ws - unified websocket chat/control channel.
///
/// Client sends JSON messages with:
/// - `{ "op": "start", ...ChatRequest fields... }`
/// - `{ "op": "input", "threadId": "...", "message": "...", "images": [] }`
/// - `{ "op": "interrupt", "threadId": "..." }`
/// - `{ "op": "recover", "threadId": "...", "limit": 200 }`
///
/// Server responds with JSON events:
/// - `accepted`, `assistant_delta`, `assistant_boundary`, `tool_use`, `tool_result`,
///   `user_ack`, `done`, `stream_input`, `interrupt`, `snapshot`, `error`.
pub async fn chat_ws(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_chat_socket(state, socket))
}

async fn handle_chat_socket(state: Arc<AppState>, socket: WebSocket) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<serde_json::Value>();

    let writer = tokio::spawn(async move {
        while let Some(payload) = out_rx.recv().await {
            let Ok(text) = serde_json::to_string(&payload) else {
                continue;
            };
            if ws_sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    let keepalive_tx = out_tx.clone();
    let keepalive = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
        // Skip the initial immediate tick; keepalive should start after the interval.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if keepalive_tx
                .send(json!({
                    "type": "ping"
                }))
                .is_err()
            {
                break;
            }
        }
    });

    while let Some(next) = ws_receiver.next().await {
        let Ok(message) = next else {
            break;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": "invalid websocket json payload"
            }));
            continue;
        };
        let Some(op) = value
            .get("op")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        else {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": "missing op"
            }));
            continue;
        };
        if let Some(map) = value.as_object_mut() {
            map.remove("op");
        }

        match op.as_str() {
            "start" => {
                handle_chat_ws_start(&state, &out_tx, value).await;
            }
            "input" => {
                handle_chat_ws_input(&state, &out_tx, value).await;
            }
            "interrupt" => {
                handle_chat_ws_interrupt(&state, &out_tx, value).await;
            }
            "recover" => {
                handle_chat_ws_recover(&state, &out_tx, value).await;
            }
            _ => {
                let _ = out_tx.send(json!({
                    "type": "error",
                    "error": format!("unsupported op: {op}")
                }));
            }
        }
    }

    keepalive.abort();
    writer.abort();
}

async fn handle_chat_ws_start(
    state: &Arc<AppState>,
    out_tx: &mpsc::UnboundedSender<serde_json::Value>,
    value: serde_json::Value,
) {
    let request = match serde_json::from_value::<ChatRequest>(value) {
        Ok(request) => request,
        Err(error) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": format!("invalid start payload: {error}")
            }));
            return;
        }
    };

    let prepared = match prepare_chat_request(state, request).await {
        Ok(prepared) => prepared,
        Err(ChatPreparationError::InvalidRequest(_status, payload)) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": payload.0.get("error").and_then(serde_json::Value::as_str).unwrap_or("invalid request")
            }));
            return;
        }
        Err(ChatPreparationError::ThreadUpdateConflict { thread_id, error }) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "threadId": thread_id,
                "error": error
            }));
            return;
        }
    };

    let config = state.config_snapshot();
    let run_id = Uuid::new_v4().to_string();
    let thread_id = prepared.thread_id.clone();
    let metadata = crate::chat_application::build_provider_run_metadata(
        &config,
        prepared.metadata,
        prepared.provider_metadata,
        &prepared.account_id,
        &prepared.from_id,
        &run_id,
    );

    let callback = build_chat_ws_stream_callback(out_tx.clone(), state, &run_id, &thread_id);
    state.sync_external_user_skills_before_run("api_chat_ws_start", &thread_id);
    let start_result = state
        .integration
        .bridge
        .start_agent_run(
            AgentRunRequest::new(
                &thread_id,
                &prepared.effective_message,
                &run_id,
                "api",
                &prepared.account_id,
                metadata,
            )
            .with_images(Some(prepared.images))
            .with_workspace_dir(prepared.workspace_path)
            .with_requested_provider(prepared.provider_type),
            Some(callback),
        )
        .await;

    match start_result {
        Ok(()) => {
            crate::runtime_diagnostics::record_message_ledger_event(
                state,
                MessageLifecycleStatus::RunStarted,
                crate::runtime_diagnostics::RuntimeDiagnosticContext {
                    thread_id: Some(thread_id.clone()),
                    run_id: Some(run_id.clone()),
                    channel: Some("api".to_owned()),
                    account_id: Some(prepared.account_id.clone()),
                    from_id: Some(prepared.from_id.clone()),
                    text_excerpt: Some(prepared.effective_message.chars().take(200).collect()),
                    metadata: Some(json!({
                        "source": "api_chat_ws_start",
                    })),
                    ..Default::default()
                },
            )
            .await;
            let _ = out_tx.send(json!({
                "type": "accepted",
                "runId": run_id,
                "threadId": thread_id
            }));
        }
        Err(error) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "runId": run_id,
                "threadId": thread_id,
                "error": error.to_string()
            }));
        }
    }
}

async fn handle_chat_ws_input(
    state: &Arc<AppState>,
    out_tx: &mpsc::UnboundedSender<serde_json::Value>,
    value: serde_json::Value,
) {
    let request = match serde_json::from_value::<StreamInputRequest>(value) {
        Ok(request) => request,
        Err(error) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": format!("invalid input payload: {error}")
            }));
            return;
        }
    };
    let thread_id = match resolve_existing_thread_key(request.thread_id) {
        Ok(thread_id) => thread_id,
        Err((_status, payload)) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": payload.0.get("error").and_then(serde_json::Value::as_str).unwrap_or("invalid thread id")
            }));
            return;
        }
    };
    let (_status, payload) = execute_chat_stream_input(
        state,
        thread_id,
        request.message,
        request.attachments,
        request.images,
        request.files,
    )
    .await;
    let _ = out_tx.send(json!({
        "type": "stream_input",
        "status": payload.status,
        "threadStatus": payload.thread_status,
        "pendingInputId": payload.pending_input_id,
        "threadId": payload.thread_id
    }));
}

async fn handle_chat_ws_interrupt(
    state: &Arc<AppState>,
    out_tx: &mpsc::UnboundedSender<serde_json::Value>,
    value: serde_json::Value,
) {
    let request = match serde_json::from_value::<InterruptRequest>(value) {
        Ok(request) => request,
        Err(error) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": format!("invalid interrupt payload: {error}")
            }));
            return;
        }
    };
    let thread_id = match resolve_existing_thread_key(request.thread_id) {
        Ok(thread_id) => thread_id,
        Err((_status, payload)) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": payload.0.get("error").and_then(serde_json::Value::as_str).unwrap_or("invalid thread id")
            }));
            return;
        }
    };
    let payload = execute_chat_interrupt(state, thread_id).await;
    let _ = out_tx.send(json!({
        "type": "interrupt",
        "status": payload.status,
        "threadId": payload.thread_id,
        "abortedRuns": payload.aborted_runs
    }));
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoverRequest {
    #[serde(default, alias = "threadId", alias = "thread_id")]
    thread_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default = "default_true_include_tools")]
    include_tool_messages: bool,
}

fn default_true_include_tools() -> bool {
    true
}

async fn handle_chat_ws_recover(
    state: &Arc<AppState>,
    out_tx: &mpsc::UnboundedSender<serde_json::Value>,
    value: serde_json::Value,
) {
    let request = match serde_json::from_value::<RecoverRequest>(value) {
        Ok(request) => request,
        Err(error) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": format!("invalid recover payload: {error}")
            }));
            return;
        }
    };
    let thread_id = match resolve_existing_thread_key(request.thread_id) {
        Ok(thread_id) => thread_id,
        Err((_status, payload)) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": payload.0.get("error").and_then(serde_json::Value::as_str).unwrap_or("invalid thread id")
            }));
            return;
        }
    };

    let limit = request.limit.unwrap_or(200).clamp(1, 500);
    let snapshot =
        crate::api::thread_history_for_key(state, &thread_id, limit, request.include_tool_messages)
            .await;

    let _ = out_tx.send(json!({
        "type": "snapshot",
        "threadId": thread_id,
        "limit": limit,
        "payload": snapshot
    }));
}

#[derive(Clone, Debug)]
struct AssistantSpeaker {
    agent_id: String,
    agent_display_name: String,
}

impl AssistantSpeaker {
    fn to_metadata_value(&self) -> Value {
        json!({
            "agent_id": self.agent_id,
            "agent_display_name": self.agent_display_name,
        })
    }
}

fn parse_agent_team_delta_prefix(text: &str) -> Option<(AssistantSpeaker, String)> {
    let stripped = text.strip_prefix('[')?;
    let close_index = stripped.find(']')?;
    let label = stripped[..close_index].trim();
    if label.is_empty() {
        return None;
    }
    let delta = stripped[close_index + 1..].trim_start().to_owned();
    Some((
        AssistantSpeaker {
            agent_id: label.to_owned(),
            agent_display_name: label.to_owned(),
        },
        delta,
    ))
}

fn build_chat_ws_stream_callback(
    out_tx: mpsc::UnboundedSender<serde_json::Value>,
    state: &Arc<AppState>,
    run_id: &str,
    thread_id: &str,
) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let callback_run_id = run_id.to_owned();
    let callback_thread_id = thread_id.to_owned();
    let callback_state = state.clone();
    let bound_delivery = BoundThreadDeliveryBuffer::default();
    let bound_delivery_state = state.clone();
    let bound_delivery_thread_id = thread_id.to_owned();
    let bound_delivery_run_id = run_id.to_owned();
    let bound_delivery_callback = bound_delivery.clone();
    let current_speaker: Arc<Mutex<Option<AssistantSpeaker>>> = Arc::new(Mutex::new(None));
    let current_speaker_for_delta = Arc::clone(&current_speaker);
    let current_speaker_for_tool = Arc::clone(&current_speaker);
    let current_speaker_for_done = Arc::clone(&current_speaker);

    Arc::new(move |event| match event {
        StreamEvent::Delta { text } => {
            if !text.is_empty() {
                bound_delivery_callback.push_delta(&text, "api ws bound delivery");
                let (delta, metadata) =
                    if let Some((speaker, cleaned_delta)) = parse_agent_team_delta_prefix(&text) {
                        *current_speaker_for_delta.lock().unwrap() = Some(speaker.clone());
                        (cleaned_delta, Some(speaker.to_metadata_value()))
                    } else {
                        let speaker = current_speaker_for_delta.lock().unwrap().clone();
                        (text, speaker.map(|entry| entry.to_metadata_value()))
                    };
                if delta.is_empty() {
                    return;
                }
                let _ = out_tx.send(json!({
                    "type": "assistant_delta",
                    "runId": callback_run_id,
                    "threadId": callback_thread_id,
                    "delta": delta,
                    "metadata": metadata,
                }));
            }
        }
        StreamEvent::ToolUse { message } => {
            let metadata = message.metadata.clone();
            if let Some(agent_id) = metadata
                .get("agent_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let display_name = metadata
                    .get("agent_display_name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(agent_id);
                *current_speaker_for_tool.lock().unwrap() = Some(AssistantSpeaker {
                    agent_id: agent_id.to_owned(),
                    agent_display_name: display_name.to_owned(),
                });
            }
            let _ = out_tx.send(json!({
                "type": "tool_use",
                "runId": callback_run_id,
                "threadId": callback_thread_id,
                "message": message
            }));
        }
        StreamEvent::ToolResult { message } => {
            let metadata = message.metadata.clone();
            if let Some(agent_id) = metadata
                .get("agent_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let display_name = metadata
                    .get("agent_display_name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(agent_id);
                *current_speaker_for_tool.lock().unwrap() = Some(AssistantSpeaker {
                    agent_id: agent_id.to_owned(),
                    agent_display_name: display_name.to_owned(),
                });
            }
            let mirrored_text = message_tool_mirror_text(&message);
            let _ = out_tx.send(json!({
                "type": "tool_result",
                "runId": callback_run_id,
                "threadId": callback_thread_id,
                "message": message
            }));
            if let Some(text) = mirrored_text {
                bound_delivery_callback.suppress();
                let _ = out_tx.send(json!({
                    "type": "assistant_delta",
                    "runId": callback_run_id,
                    "threadId": callback_thread_id,
                    "delta": text
                }));
            }
        }
        StreamEvent::Boundary {
            kind,
            pending_input_id,
        } => match kind {
            StreamBoundaryKind::AssistantSegment => {
                bound_delivery_callback.push_separator("api ws bound delivery");
                let _ = out_tx.send(json!({
                    "type": "assistant_boundary",
                    "runId": callback_run_id,
                    "threadId": callback_thread_id
                }));
            }
            StreamBoundaryKind::UserAck => {
                bound_delivery_callback.finish(
                    bound_delivery_state.clone(),
                    bound_delivery_thread_id.clone(),
                    bound_delivery_run_id.clone(),
                    "api ws bound delivery",
                );
                let _ = out_tx.send(json!({
                    "type": "user_ack",
                    "runId": callback_run_id,
                    "threadId": callback_thread_id,
                    "pendingInputId": pending_input_id
                }));
            }
        },
        StreamEvent::Done => {
            *current_speaker_for_done.lock().unwrap() = None;
            bound_delivery_callback.finish(
                callback_state.clone(),
                callback_thread_id.clone(),
                callback_run_id.clone(),
                "api ws bound delivery",
            );
            let _ = out_tx.send(json!({
                "type": "done",
                "runId": callback_run_id,
                "threadId": callback_thread_id
            }));
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "chat_tests.rs"]
mod chat_tests;
