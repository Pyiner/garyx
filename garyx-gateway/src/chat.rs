//! API chat channel routes.
//!
//! Rust port of `src/garyx/plugins/channels/api.py`.
//! Provides HTTP endpoints for sending messages and receiving responses.

use std::collections::HashMap;
use std::sync::Arc;

use crate::application::chat::contracts::{
    ChatRequest, InterruptRequest, StartChatResponse, StreamInputRequest,
    resolve_existing_thread_key,
};
use crate::chat_application::{ChatPreparationError, prepare_chat_request};
use crate::chat_control::{execute_chat_interrupt, execute_chat_stream_input};
use crate::chat_delivery::build_bound_response_callback;
use axum::Json;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use garyx_models::MessageLifecycleStatus;
use garyx_models::provider::{AgentRunRequest, StreamEvent};
use garyx_router::{THREAD_TRANSCRIPT_REPLAY_CAP, ThreadTranscriptRecord};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
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
/// - `accepted`, `committed_message`, `stream_input`, `interrupt`, `snapshot`,
///   `error`.
pub async fn chat_ws(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_chat_socket(state, socket))
}

/// POST /api/chat/start - start a chat run via HTTP.
pub async fn chat_start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> impl IntoResponse {
    match start_chat_run(&state, request, None).await {
        Ok(response) => Json(response).into_response(),
        Err(payload) => payload.into_response(),
    }
}

/// POST /api/chat/interrupt - interrupt an active run for a thread.
pub async fn chat_interrupt(
    State(state): State<Arc<AppState>>,
    Json(request): Json<InterruptRequest>,
) -> impl IntoResponse {
    let thread_id = match resolve_existing_thread_key(request.thread_id) {
        Ok(thread_id) => thread_id,
        Err((status, payload)) => return (status, payload).into_response(),
    };
    let payload = execute_chat_interrupt(&state, thread_id).await;
    Json(payload).into_response()
}

/// POST /api/chat/stream-input - add input to an active streaming run.
pub async fn chat_stream_input(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StreamInputRequest>,
) -> impl IntoResponse {
    let thread_id = match resolve_existing_thread_key(request.thread_id) {
        Ok(thread_id) => thread_id,
        Err((status, payload)) => return (status, payload).into_response(),
    };
    let (_status, payload) = execute_chat_stream_input(
        &state,
        thread_id,
        request.client_intent_id,
        request.message,
        request.attachments,
        request.images,
        request.files,
    )
    .await;
    Json(payload).into_response()
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
    let run_state = state.clone();
    let run_out_tx = out_tx.clone();
    tokio::spawn(async move {
        let callback_state = run_state.clone();
        let callback_out_tx = run_out_tx.clone();
        let callback_builder = move |run_id: &str, thread_id: &str| {
            let task = spawn_chat_ws_committed_stream(
                callback_state.clone(),
                callback_out_tx.clone(),
                thread_id.to_owned(),
                run_id.to_owned(),
            );
            ChatStreamCallbackAttachment {
                callback: None,
                task: Some(task),
            }
        };
        match start_chat_run(&run_state, request, Some(Box::new(callback_builder))).await {
            Ok(response) => {
                let _ = run_out_tx.send(json!({
                    "type": "accepted",
                    "runId": response.run_id,
                    "threadId": response.thread_id
                }));
            }
            Err((_status, payload)) => {
                let body = payload.0;
                let error = body
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("agent run failed");
                let run_id = body
                    .get("runId")
                    .or_else(|| body.get("run_id"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let thread_id = body
                    .get("threadId")
                    .or_else(|| body.get("thread_id"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let _ = run_out_tx.send(json!({
                    "type": "error",
                    "runId": run_id,
                    "threadId": thread_id,
                    "error": error
                }));
            }
        }
    });
}

struct ChatStreamCallbackAttachment {
    callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    task: Option<JoinHandle<()>>,
}

type ChatStreamCallbackBuilder =
    Box<dyn Fn(&str, &str) -> ChatStreamCallbackAttachment + Send + Sync>;

fn compose_stream_callbacks(
    callbacks: Vec<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
    if callbacks.is_empty() {
        return None;
    }
    Some(Arc::new(move |event: StreamEvent| {
        for callback in &callbacks {
            callback(event.clone());
        }
    }))
}

async fn start_chat_run(
    state: &Arc<AppState>,
    request: ChatRequest,
    callback_builder: Option<ChatStreamCallbackBuilder>,
) -> Result<StartChatResponse, (StatusCode, Json<Value>)> {
    if !state.provider_runtime_ready() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "gateway_provider_runtime_starting",
                "message": "Gateway provider runtime is still starting; retry shortly."
            })),
        ));
    }

    let prepared = match prepare_chat_request(state, request).await {
        Ok(prepared) => prepared,
        Err(ChatPreparationError::InvalidRequest(status, payload)) => {
            return Err((status, payload));
        }
        Err(ChatPreparationError::ThreadUpdateConflict { thread_id, error }) => {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "threadId": thread_id,
                    "error": error
                })),
            ));
        }
    };

    let config = state.config_snapshot();
    let run_id = Uuid::new_v4().to_string();
    let thread_id = prepared.thread_id.clone();
    let metadata = crate::chat_application::build_provider_run_metadata(
        &config,
        prepared.metadata,
        &prepared.channel,
        &prepared.account_id,
        &prepared.from_id,
        &run_id,
    );

    let mut callbacks = Vec::new();
    let mut stream_tasks = Vec::<JoinHandle<()>>::new();
    if let Some(builder) = callback_builder {
        let mut attachment = builder(&run_id, &thread_id);
        if let Some(callback) = attachment.callback.take() {
            callbacks.push(callback);
        }
        if let Some(task) = attachment.task.take() {
            stream_tasks.push(task);
        }
    }
    let bound_stream = match build_bound_response_callback(state, &thread_id, &run_id, None).await {
        Ok(stream) => {
            if let Some(callback) = stream.callback() {
                callbacks.push(callback);
            }
            stream
        }
        Err(error) => {
            for task in stream_tasks {
                task.abort();
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "runId": run_id,
                    "threadId": thread_id,
                    "error": format!("failed to attach bound channel response stream: {error}")
                })),
            ));
        }
    };
    let callback = compose_stream_callbacks(callbacks);
    state.sync_external_user_skills_before_run("api_chat_start", &thread_id);
    let start_result = state
        .integration
        .bridge
        .start_agent_run(
            AgentRunRequest::new(
                &thread_id,
                &prepared.effective_message,
                &run_id,
                &prepared.channel,
                &prepared.account_id,
                metadata,
            )
            .with_images(Some(prepared.images))
            .with_workspace_dir(prepared.workspace_path)
            .with_requested_provider(prepared.provider_type),
            callback,
        )
        .await;

    match start_result {
        Ok(()) => {
            bound_stream.detach();
            crate::runtime_diagnostics::record_message_ledger_event(
                state,
                MessageLifecycleStatus::RunStarted,
                crate::runtime_diagnostics::RuntimeDiagnosticContext {
                    thread_id: Some(thread_id.clone()),
                    run_id: Some(run_id.clone()),
                    channel: Some(prepared.channel.clone()),
                    account_id: Some(prepared.account_id.clone()),
                    from_id: Some(prepared.from_id.clone()),
                    text_excerpt: Some(prepared.effective_message.chars().take(200).collect()),
                    metadata: Some(json!({
                        "source": "api_chat_start",
                    })),
                    ..Default::default()
                },
            )
            .await;
            Ok(StartChatResponse {
                status: "accepted".to_owned(),
                run_id,
                thread_id,
            })
        }
        Err(error) => {
            bound_stream.abort();
            for task in stream_tasks {
                task.abort();
            }
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "runId": run_id,
                    "threadId": thread_id,
                    "error": error.to_string()
                })),
            ))
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
        request.client_intent_id.clone(),
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
        "clientIntentId": payload.client_intent_id,
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
    let snapshot = crate::api::thread_history_for_key(
        state,
        &thread_id,
        limit,
        request.include_tool_messages,
        None,
        None,
        None,
    )
    .await;

    let _ = out_tx.send(json!({
        "type": "snapshot",
        "threadId": thread_id,
        "limit": limit,
        "payload": snapshot
    }));
}

fn spawn_chat_ws_committed_stream(
    state: Arc<AppState>,
    out_tx: mpsc::UnboundedSender<Value>,
    thread_id: String,
    run_id: String,
) -> JoinHandle<()> {
    let mut rx = state.ops.events.subscribe();
    let transcript_store = state.threads.history.transcript_store();
    tokio::spawn(async move {
        let mut sent_payloads: HashMap<u64, String> = HashMap::new();
        let mut last_sent_seq = 0u64;
        loop {
            match rx.recv().await {
                Ok(raw) => {
                    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
                        continue;
                    };
                    if is_committed_ws_record_for_run(&value, &thread_id, &run_id) {
                        let terminal = is_terminal_bus_record_for_run(&value, &run_id);
                        match forward_chat_ws_committed_value(
                            &out_tx,
                            &value,
                            &mut sent_payloads,
                            &mut last_sent_seq,
                        ) {
                            ChatWsForwardOutcome::SentOrSkipped => {
                                if terminal
                                    && !backfill_chat_ws_committed(
                                        transcript_store.clone(),
                                        &out_tx,
                                        &thread_id,
                                        &run_id,
                                        &mut sent_payloads,
                                        &mut last_sent_seq,
                                    )
                                    .await
                                {
                                    break;
                                }
                                if terminal {
                                    break;
                                }
                                continue;
                            }
                            ChatWsForwardOutcome::Closed => break,
                            ChatWsForwardOutcome::Gap => {}
                        }
                        if !backfill_chat_ws_committed(
                            transcript_store.clone(),
                            &out_tx,
                            &thread_id,
                            &run_id,
                            &mut sent_payloads,
                            &mut last_sent_seq,
                        )
                        .await
                        {
                            break;
                        }
                        match forward_chat_ws_committed_value(
                            &out_tx,
                            &value,
                            &mut sent_payloads,
                            &mut last_sent_seq,
                        ) {
                            ChatWsForwardOutcome::Closed => break,
                            ChatWsForwardOutcome::SentOrSkipped | ChatWsForwardOutcome::Gap => {}
                        }
                        if terminal {
                            if !backfill_chat_ws_committed(
                                transcript_store.clone(),
                                &out_tx,
                                &thread_id,
                                &run_id,
                                &mut sent_payloads,
                                &mut last_sent_seq,
                            )
                            .await
                            {
                                break;
                            }
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    state.ops.events.record_drop();
                    if !backfill_chat_ws_committed(
                        transcript_store.clone(),
                        &out_tx,
                        &thread_id,
                        &run_id,
                        &mut sent_payloads,
                        &mut last_sent_seq,
                    )
                    .await
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    let _ = backfill_chat_ws_committed(
                        transcript_store.clone(),
                        &out_tx,
                        &thread_id,
                        &run_id,
                        &mut sent_payloads,
                        &mut last_sent_seq,
                    )
                    .await;
                    break;
                }
            }
        }
    })
}

fn is_committed_ws_record_for_run(value: &Value, thread_id: &str, run_id: &str) -> bool {
    if value.get("type").and_then(Value::as_str) != Some("committed_message")
        || value.get("thread_id").and_then(Value::as_str) != Some(thread_id)
    {
        return false;
    }
    value.get("run_id").and_then(Value::as_str) == Some(run_id)
        || (value.get("run_id").is_none()
            && value
                .pointer("/message/control/kind")
                .and_then(Value::as_str)
                .is_some())
}

fn is_terminal_bus_record_for_run(value: &Value, run_id: &str) -> bool {
    value.get("type").and_then(Value::as_str) == Some("committed_message")
        && value.get("run_id").and_then(Value::as_str) == Some(run_id)
        && matches!(
            value
                .pointer("/message/control/kind")
                .and_then(Value::as_str),
            Some("run_complete" | "run_error")
        )
}

fn committed_ws_payload(record: ThreadTranscriptRecord) -> Value {
    json!({
        "type": "committed_message",
        "thread_id": record.thread_id,
        "run_id": record.run_id,
        "seq": record.seq,
        "message": record.message,
    })
}

fn forward_chat_ws_committed_record(
    out_tx: &mpsc::UnboundedSender<Value>,
    record: ThreadTranscriptRecord,
    sent_payloads: &mut HashMap<u64, String>,
    last_sent_seq: &mut u64,
) -> ChatWsForwardOutcome {
    let payload = committed_ws_payload(record);
    forward_chat_ws_committed_value(out_tx, &payload, sent_payloads, last_sent_seq)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatWsForwardOutcome {
    SentOrSkipped,
    Gap,
    Closed,
}

fn forward_chat_ws_committed_value(
    out_tx: &mpsc::UnboundedSender<Value>,
    value: &Value,
    sent_payloads: &mut HashMap<u64, String>,
    last_sent_seq: &mut u64,
) -> ChatWsForwardOutcome {
    let seq = value.get("seq").and_then(Value::as_u64).unwrap_or(0);
    if seq == 0 {
        return ChatWsForwardOutcome::SentOrSkipped;
    }
    let payload = value.to_string();
    if sent_payloads.get(&seq).is_some_and(|sent| sent == &payload) {
        return ChatWsForwardOutcome::SentOrSkipped;
    }
    if *last_sent_seq != 0 && seq > *last_sent_seq + 1 {
        return ChatWsForwardOutcome::Gap;
    }
    if seq > *last_sent_seq {
        *last_sent_seq = seq;
    }
    sent_payloads.insert(seq, payload);
    if out_tx.send(value.clone()).is_err() {
        return ChatWsForwardOutcome::Closed;
    }
    ChatWsForwardOutcome::SentOrSkipped
}

async fn backfill_chat_ws_committed(
    transcript_store: Arc<garyx_router::ThreadTranscriptStore>,
    out_tx: &mpsc::UnboundedSender<Value>,
    thread_id: &str,
    run_id: &str,
    sent_payloads: &mut HashMap<u64, String>,
    last_sent_seq: &mut u64,
) -> bool {
    loop {
        let cursor = *last_sent_seq;
        let records = transcript_store
            .records_for_run_after_seq(thread_id, run_id, cursor, THREAD_TRANSCRIPT_REPLAY_CAP)
            .await
            .unwrap_or_default();
        if records.is_empty() {
            break;
        }
        let page_len = records.len();
        for record in records {
            if matches!(
                forward_chat_ws_committed_record(out_tx, record, sent_payloads, last_sent_seq),
                ChatWsForwardOutcome::Closed
            ) {
                return false;
            }
        }
        if page_len < THREAD_TRANSCRIPT_REPLAY_CAP || *last_sent_seq == cursor {
            break;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "chat_tests.rs"]
mod chat_tests;
