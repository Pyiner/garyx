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
use crate::chat_application::{
    ChatPreparationError, prepare_chat_request, prepare_durable_chat_request,
};
use crate::chat_application::{ThreadlessCorrelationTarget, resolve_threadless_correlation_target};
use crate::chat_control::{execute_chat_interrupt, execute_chat_stream_input};
use crate::chat_delivery::build_bound_response_callback;
use crate::conversation_admission::{
    AdmissionOperationResult, AdmissionRegistration, AdmissionRegistrationError,
    chat_request_fingerprint, resolve_dispatch_correlation,
};
use crate::garyx_db::{
    DispatchAdmissionKey, DispatchAdmissionKind, DispatchAdmissionRecord, DispatchAdmissionState,
    DispatchOutcome,
};
use crate::prompt_attachment_lifecycle::PromptAttachmentLifecycleError;
use crate::sqlite_thread_store::{AtomicCreateDispatchLedger, AtomicExistingDispatchCommit};
use axum::Json;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use garyx_models::MessageLifecycleStatus;
use garyx_models::provider::{AgentDispatchOutcome, AgentRunRequest, StreamEvent};
use garyx_router::{
    AdmittedRun, AgentDispatcher, RunAdmissionError, THREAD_TRANSCRIPT_REPLAY_CAP,
    ThreadTranscriptRecord,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::server::AppState;

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// GET /api/chat/health - API channel health check.
pub async fn chat_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bridge_ready = !state.integration.bridge.provider_keys().await.is_empty();
    let mut payload = json!({
        "status": "ok",
        "channel": "api",
        "bridge_ready": bridge_ready,
    });
    // AppStateBuilder only retains this concrete store handle when the thread
    // truth source, sorted record-lock domain, and admission repository share
    // the same Garyx DB. Custom stores remain healthy but must not advertise a
    // durable protocol they cannot atomically uphold.
    if state.threads.sqlite_thread_store.is_some() {
        payload["deliveryCapabilities"] = json!({
            "dispatchAdmission": 1,
            "atomicCreateDispatch": 1,
            "createIntentClaim": 1,
            "promptAttachmentLifecycle": 1,
            "explicitScopeRequiredForRecovery": true,
        });
    }
    Json(payload)
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
    let thread_id = match resolve_existing_thread_key(request.thread_id.clone()) {
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
    let thread_id = match resolve_existing_thread_key(request.thread_id.clone()) {
        Ok(thread_id) => thread_id,
        Err((status, payload)) => return (status, payload).into_response(),
    };
    let (status, payload) = execute_chat_stream_input(&state, thread_id, request).await;
    (status, Json(payload)).into_response()
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
                    "threadId": response.thread_id,
                    "dispatchOutcome": response.dispatch_outcome,
                    "effectiveRunId": response.effective_run_id,
                    "pendingInputId": response.pending_input_id,
                    "deliveryState": response.delivery_state,
                    "idempotencyReplay": response.idempotency_replay
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
                    "error": error,
                    "dispatchOutcome": body.get("dispatchOutcome").cloned().unwrap_or(Value::Null),
                    "effectiveRunId": body.get("effectiveRunId").cloned().unwrap_or(Value::Null),
                    "pendingInputId": body.get("pendingInputId").cloned().unwrap_or(Value::Null),
                    "deliveryState": body.get("deliveryState").cloned().unwrap_or(Value::Null),
                    "idempotencyReplay": body.get("idempotencyReplay").cloned().unwrap_or(Value::Null)
                }));
            }
        }
    });
}

pub(crate) struct ChatStreamCallbackAttachment {
    pub(crate) callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    pub(crate) task: Option<JoinHandle<()>>,
}

pub(crate) type ChatStreamCallbackBuilder =
    Box<dyn Fn(&str, &str) -> ChatStreamCallbackAttachment + Send + Sync>;

pub(crate) fn compose_stream_callbacks(
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

fn internal_admission_error(error: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "dispatch_admission_storage_error",
            "message": error,
        })),
    )
}

fn registration_error(error: AdmissionRegistrationError) -> (StatusCode, Json<Value>) {
    match error {
        AdmissionRegistrationError::FingerprintConflict => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": error.to_string(),
            })),
        ),
        AdmissionRegistrationError::Overloaded => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "dispatch_admission_overloaded",
                "message": error.to_string(),
            })),
        ),
    }
}

fn ensure_matching_fingerprint(
    record: &DispatchAdmissionRecord,
    fingerprint: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    if record.fingerprint_version == 1 && record.request_fingerprint == fingerprint {
        return Ok(());
    }
    Err((
        StatusCode::CONFLICT,
        Json(json!({
            "error": "idempotency_conflict",
            "message": "clientIntentId was reused with a different request",
            "threadId": record.key.thread_id,
            "runId": record.requested_run_id,
        })),
    ))
}

fn status_from_record(record: &DispatchAdmissionRecord, fallback: StatusCode) -> StatusCode {
    record
        .result_http_status
        .and_then(|value| u16::try_from(value).ok())
        .and_then(|value| StatusCode::from_u16(value).ok())
        .unwrap_or(fallback)
}

pub(crate) fn start_response_from_record(
    record: DispatchAdmissionRecord,
    replay: bool,
) -> Result<StartChatResponse, (StatusCode, Json<Value>)> {
    let delivery_state = record.state.as_str().to_owned();
    let dispatch_outcome = record.outcome.map(|value| value.as_str().to_owned());
    let run_id = record.requested_run_id.clone().unwrap_or_default();
    let effective_run_id = record
        .effective_run_id
        .clone()
        .or_else(|| (!run_id.is_empty()).then(|| run_id.clone()));
    match record.state {
        DispatchAdmissionState::Accepted => Ok(StartChatResponse {
            status: "accepted".to_owned(),
            run_id,
            thread_id: record.key.thread_id,
            dispatch_outcome,
            effective_run_id,
            pending_input_id: record.pending_input_id,
            delivery_state: Some(delivery_state),
            idempotency_replay: Some(replay),
        }),
        DispatchAdmissionState::Rejected
        | DispatchAdmissionState::Ambiguous
        | DispatchAdmissionState::HandoffStarted
        | DispatchAdmissionState::Admitted
        | DispatchAdmissionState::NotDispatched => {
            let (fallback_status, fallback_code, fallback_message) = match record.state {
                DispatchAdmissionState::Rejected => (
                    StatusCode::CONFLICT,
                    "dispatch_rejected",
                    "dispatch was rejected before provider handoff",
                ),
                DispatchAdmissionState::Admitted => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "dispatch_admission_pending",
                    "dispatch is durably admitted and can be resumed",
                ),
                DispatchAdmissionState::NotDispatched => (
                    StatusCode::CONFLICT,
                    "dispatch_not_started",
                    "chat start was not dispatched",
                ),
                DispatchAdmissionState::Ambiguous | DispatchAdmissionState::HandoffStarted => (
                    StatusCode::CONFLICT,
                    "dispatch_ambiguous",
                    "provider handoff outcome is unknown and will not be reissued",
                ),
                DispatchAdmissionState::Accepted => unreachable!(),
            };
            let status = status_from_record(&record, fallback_status);
            Err((
                status,
                Json(json!({
                    "error": record.result_error_code.as_deref().unwrap_or(fallback_code),
                    "message": record.result_error_message.as_deref().unwrap_or(fallback_message),
                    "runId": run_id,
                    "threadId": record.key.thread_id,
                    "dispatchOutcome": dispatch_outcome,
                    "effectiveRunId": effective_run_id,
                    "pendingInputId": record.pending_input_id,
                    "deliveryState": delivery_state,
                    "idempotencyReplay": replay,
                })),
            ))
        }
    }
}

fn preparation_failure(error: ChatPreparationError) -> AdmissionOperationResult {
    match error {
        ChatPreparationError::InvalidRequest(status, payload) => AdmissionOperationResult::Failed {
            status,
            payload: payload.0,
        },
        ChatPreparationError::ThreadUpdateConflict { thread_id, error } => {
            AdmissionOperationResult::Failed {
                status: StatusCode::CONFLICT,
                payload: json!({"threadId": thread_id, "error": error}),
            }
        }
        ChatPreparationError::Storage { thread_id, error } => AdmissionOperationResult::Failed {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            payload: json!({"threadId": thread_id, "error": error}),
        },
    }
}

fn durable_failure(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> AdmissionOperationResult {
    AdmissionOperationResult::Failed {
        status,
        payload: json!({
            "error": code,
            "message": message.into(),
        }),
    }
}

fn durable_outcome_fields(
    outcome: &AgentDispatchOutcome,
    requested_run_id: &str,
) -> (DispatchOutcome, String, Option<String>) {
    match outcome {
        AgentDispatchOutcome::Started => {
            (DispatchOutcome::Started, requested_run_id.to_owned(), None)
        }
        AgentDispatchOutcome::QueuedToActiveRun {
            effective_run_id,
            pending_input_id,
        } => (
            DispatchOutcome::QueuedToActiveRun,
            effective_run_id.clone(),
            Some(pending_input_id.clone()),
        ),
    }
}

fn admission_plan_matches(
    record: &DispatchAdmissionRecord,
    requested_run_id: &str,
    outcome: DispatchOutcome,
    effective_run_id: &str,
    pending_input_id: Option<&str>,
) -> bool {
    record.requested_run_id.as_deref() == Some(requested_run_id)
        && record.outcome == Some(outcome)
        && record.effective_run_id.as_deref() == Some(effective_run_id)
        && record.pending_input_id.as_deref() == pending_input_id
}

async fn run_durable_chat_start(
    state: Arc<AppState>,
    request: ChatRequest,
    key: DispatchAdmissionKey,
    fingerprint: String,
    callback_builder: Option<ChatStreamCallbackBuilder>,
) -> AdmissionOperationResult {
    let existing = match state.ops.conversation_admission.read(key.clone()).await {
        Ok(record) => record,
        Err(error) => {
            return durable_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_admission_storage_error",
                error,
            );
        }
    };
    if let Some(record) = existing.as_ref() {
        if let Err((_status, payload)) = ensure_matching_fingerprint(record, &fingerprint) {
            return AdmissionOperationResult::Failed {
                status: StatusCode::CONFLICT,
                payload: payload.0,
            };
        }
        if !matches!(record.state, DispatchAdmissionState::Admitted) {
            return AdmissionOperationResult::Ready;
        }
    }
    if !state.provider_runtime_ready() {
        return durable_failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway_provider_runtime_starting",
            "Gateway provider runtime is still starting; retry shortly.",
        );
    }

    let prepared = match prepare_durable_chat_request(&state, request).await {
        Ok(prepared) => prepared,
        Err(error) => return preparation_failure(error),
    };
    if prepared.thread_id != key.thread_id {
        return durable_failure(
            StatusCode::CONFLICT,
            "dispatch_thread_changed",
            "request routing changed the durable thread identity",
        );
    }
    let managed_attachment_claims = prepared.managed_attachment_claims.clone();
    let record_patch = prepared.record_patch.clone();
    let binding_plan = prepared.binding_plan.clone();
    let cache_changes_after_commit = prepared.cache_changes_after_commit;

    let run_id = existing
        .as_ref()
        .and_then(|record| record.requested_run_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let pending_input_id = existing
        .as_ref()
        .and_then(|record| record.pending_input_id.clone())
        .unwrap_or_else(|| format!("queued_input:{}", Uuid::new_v4()));
    let config = state.config_snapshot();
    let metadata = crate::chat_application::build_provider_run_metadata(
        &config,
        prepared.metadata,
        &prepared.channel,
        &prepared.account_id,
        &prepared.from_id,
        &run_id,
    );
    let admitted = match AdmittedRun::thread_bound(
        state.threads.thread_store.clone(),
        AgentRunRequest::new(
            &prepared.thread_id,
            &prepared.effective_message,
            &run_id,
            &prepared.channel,
            &prepared.account_id,
            metadata,
        )
        .with_images(Some(prepared.images))
        .with_workspace_dir(prepared.workspace_path.clone())
        .with_requested_provider(prepared.provider_type),
    )
    .await
    {
        Ok(run) => run,
        Err(error) => {
            let status = match error {
                RunAdmissionError::NotFound(_) => StatusCode::NOT_FOUND,
                RunAdmissionError::Archived(_) | RunAdmissionError::Stale(_) => {
                    StatusCode::CONFLICT
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return durable_failure(status, "run_admission_failed", error.to_string());
        }
    };
    let plan = match state
        .integration
        .bridge
        .prepare_durable_dispatch(admitted, pending_input_id)
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            return durable_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_plan_failed",
                error,
            );
        }
    };
    let requested_run_id = plan.requested_run_id().to_owned();
    let planned_outcome = plan.outcome();
    let (outcome, effective_run_id, planned_pending_input_id) =
        durable_outcome_fields(&planned_outcome, &requested_run_id);

    let record = match existing {
        Some(record) => record,
        None => match {
            let command = AtomicExistingDispatchCommit {
                target_thread_id: key.thread_id.clone(),
                target_patch: record_patch,
                merges: Vec::new(),
                dispatch: AtomicCreateDispatchLedger {
                    key: key.clone(),
                    request_fingerprint: fingerprint.clone(),
                    requested_run_id: requested_run_id.clone(),
                    effective_run_id: effective_run_id.clone(),
                    pending_input_id: planned_pending_input_id.clone(),
                    outcome,
                    attachment_claims: managed_attachment_claims,
                },
            };
            match binding_plan {
                Some(binding) => state
                    .ops
                    .endpoint_binding_mutator
                    .commit_existing_dispatch_with_binding(command, binding)
                    .await
                    .map_err(|error| error.to_string()),
                None => match state.threads.sqlite_thread_store.as_ref() {
                    Some(store) => store
                        .commit_existing_dispatch_atomic(command)
                        .await
                        .map_err(|error| error.to_string()),
                    None => Err(
                        "durable dispatch is unsupported by the configured thread store".to_owned(),
                    ),
                },
            }
        } {
            Ok(record) => record,
            Err(error) => {
                return durable_failure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "dispatch_admission_storage_error",
                    error,
                );
            }
        },
    };
    if let Err((_status, payload)) = ensure_matching_fingerprint(&record, &fingerprint) {
        return AdmissionOperationResult::Failed {
            status: StatusCode::CONFLICT,
            payload: payload.0,
        };
    }
    if !matches!(record.state, DispatchAdmissionState::Admitted) {
        return AdmissionOperationResult::Ready;
    }
    if !admission_plan_matches(
        &record,
        &requested_run_id,
        outcome,
        &effective_run_id,
        planned_pending_input_id.as_deref(),
    ) {
        return durable_failure(
            StatusCode::CONFLICT,
            "dispatch_plan_changed",
            "the durable dispatch plan no longer matches the admitted identifiers",
        );
    }
    state
        .integration
        .bridge
        .set_thread_workspace_binding(&key.thread_id, prepared.workspace_path.clone())
        .await;
    if cache_changes_after_commit {
        state.invalidate_gateway_sync_caches().await;
    }

    let mut callbacks = Vec::new();
    let mut stream_tasks = Vec::<JoinHandle<()>>::new();
    if let Some(builder) = callback_builder {
        let mut attachment = builder(&requested_run_id, &key.thread_id);
        if let Some(callback) = attachment.callback.take() {
            callbacks.push(callback);
        }
        if let Some(task) = attachment.task.take() {
            stream_tasks.push(task);
        }
    }
    let bound_stream = match build_bound_response_callback(
        &state,
        &key.thread_id,
        &requested_run_id,
        None,
    )
    .await
    {
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
            return durable_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "response_stream_attach_failed",
                error.to_string(),
            );
        }
    };
    let callback = compose_stream_callbacks(callbacks);
    state.sync_external_user_skills_before_run("api_chat_start", &key.thread_id);
    match state
        .ops
        .conversation_admission
        .start_handoff(key.clone())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            bound_stream.abort();
            for task in stream_tasks {
                task.abort();
            }
            return AdmissionOperationResult::Ready;
        }
        Err(error) => {
            bound_stream.abort();
            for task in stream_tasks {
                task.abort();
            }
            return durable_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_admission_storage_error",
                error,
            );
        }
    }

    let dispatch_result = state
        .integration
        .bridge
        .execute_durable_dispatch(plan, callback)
        .await;
    match dispatch_result {
        Ok(actual_outcome) => {
            let (actual, actual_effective_run_id, actual_pending_input_id) =
                durable_outcome_fields(&actual_outcome, &requested_run_id);
            let settled = state
                .ops
                .conversation_admission
                .settle(
                    key.clone(),
                    DispatchAdmissionState::Accepted,
                    Some(actual),
                    Some(actual_effective_run_id.clone()),
                    actual_pending_input_id.clone(),
                    200,
                    None,
                    None,
                )
                .await;
            if let Err(error) = settled {
                bound_stream.abort();
                for task in stream_tasks {
                    task.abort();
                }
                return durable_failure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "dispatch_settlement_failed",
                    error,
                );
            }
            match actual_outcome {
                AgentDispatchOutcome::Started => bound_stream.detach(),
                AgentDispatchOutcome::QueuedToActiveRun { .. } => {
                    bound_stream.abort();
                    for task in &stream_tasks {
                        task.abort();
                    }
                }
            }
            crate::runtime_diagnostics::record_message_ledger_event(
                &state,
                MessageLifecycleStatus::RunStarted,
                crate::runtime_diagnostics::RuntimeDiagnosticContext {
                    thread_id: Some(key.thread_id.clone()),
                    run_id: Some(requested_run_id),
                    channel: Some(prepared.channel),
                    account_id: Some(prepared.account_id),
                    from_id: Some(prepared.from_id),
                    text_excerpt: Some(prepared.effective_message.chars().take(200).collect()),
                    metadata: Some(json!({"source": "api_chat_start"})),
                    ..Default::default()
                },
            )
            .await;
            AdmissionOperationResult::Ready
        }
        Err(error) => {
            bound_stream.abort();
            for task in stream_tasks {
                task.abort();
            }
            match state
                .ops
                .conversation_admission
                .settle(
                    key,
                    DispatchAdmissionState::Ambiguous,
                    Some(outcome),
                    Some(effective_run_id),
                    planned_pending_input_id,
                    409,
                    Some("dispatch_ambiguous".to_owned()),
                    Some(error),
                )
                .await
            {
                Ok(_) => AdmissionOperationResult::Ready,
                Err(error) => durable_failure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "dispatch_settlement_failed",
                    error,
                ),
            }
        }
    }
}

async fn start_chat_run(
    state: &Arc<AppState>,
    mut request: ChatRequest,
    callback_builder: Option<ChatStreamCallbackBuilder>,
) -> Result<StartChatResponse, (StatusCode, Json<Value>)> {
    let correlation = resolve_dispatch_correlation(&mut request)
        .map_err(|(status, payload)| (status, Json(payload)))?;
    let Some(correlation) = correlation else {
        return start_chat_run_legacy(state, request, callback_builder).await;
    };

    let thread_id = match request.thread_id.as_deref() {
        Some(raw_thread_id) => {
            let thread_id = resolve_existing_thread_key(Some(raw_thread_id.to_owned()))?;
            request.thread_id = Some(thread_id.clone());
            thread_id
        }
        None => match resolve_threadless_correlation_target(state, &request).await {
            Ok(ThreadlessCorrelationTarget::Existing { thread_id }) => thread_id,
            Ok(ThreadlessCorrelationTarget::Create(plan)) => {
                return crate::create_dispatch::create_implicit_and_dispatch(
                    state,
                    correlation.scope_identity,
                    correlation.scope_epoch,
                    correlation.client_intent_id,
                    request,
                    plan,
                    callback_builder,
                )
                .await;
            }
            Err(ChatPreparationError::InvalidRequest(status, payload)) => {
                return Err((status, payload));
            }
            Err(ChatPreparationError::ThreadUpdateConflict { thread_id, error }) => {
                return Err((
                    StatusCode::CONFLICT,
                    Json(json!({"threadId": thread_id, "error": error})),
                ));
            }
            Err(ChatPreparationError::Storage { thread_id, error }) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"threadId": thread_id, "error": error})),
                ));
            }
        },
    };
    let fingerprint = chat_request_fingerprint(&request);
    let key = DispatchAdmissionKey {
        scope_identity: correlation.scope_identity,
        scope_epoch: correlation.scope_epoch,
        thread_id,
        kind: DispatchAdmissionKind::ChatStart,
        client_intent_id: correlation.client_intent_id,
    };

    if let Some(record) = state
        .ops
        .conversation_admission
        .read(key.clone())
        .await
        .map_err(internal_admission_error)?
    {
        ensure_matching_fingerprint(&record, &fingerprint)?;
        if !matches!(record.state, DispatchAdmissionState::Admitted) {
            return start_response_from_record(record, true);
        }
    }

    let registration = state
        .ops
        .conversation_admission
        .register(key.clone(), &fingerprint)
        .map_err(registration_error)?;
    let response_key = key.clone();
    let (join, replay) = match registration {
        AdmissionRegistration::Join(join) => (join, true),
        AdmissionRegistration::Owner(owner) => {
            let join = owner.join_handle();
            let owner_state = Arc::clone(state);
            let owner_key = key;
            tokio::spawn(async move {
                let result = run_durable_chat_start(
                    owner_state,
                    request,
                    owner_key,
                    fingerprint,
                    callback_builder,
                )
                .await;
                owner.publish(result);
            });
            (join, false)
        }
    };
    match join.wait().await.as_ref() {
        AdmissionOperationResult::Ready => {
            let record = state
                .ops
                .conversation_admission
                .read(response_key)
                .await
                .map_err(internal_admission_error)?
                .ok_or_else(|| {
                    internal_admission_error(
                        "dispatch owner completed without a durable admission row".to_owned(),
                    )
                })?;
            start_response_from_record(record, replay)
        }
        AdmissionOperationResult::Failed { status, payload } => {
            Err((*status, Json(payload.clone())))
        }
    }
}

async fn start_chat_run_legacy(
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
        Err(ChatPreparationError::Storage { thread_id, error }) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
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
    let managed_attachment_claims = prepared.managed_attachment_claims.clone();
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
    let admitted = match AdmittedRun::thread_bound(
        state.threads.thread_store.clone(),
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
    )
    .await
    {
        Ok(run) => run,
        Err(error) => {
            for task in stream_tasks {
                task.abort();
            }
            bound_stream.abort();
            let status = match error {
                RunAdmissionError::NotFound(_) => StatusCode::NOT_FOUND,
                RunAdmissionError::Archived(_) | RunAdmissionError::Stale(_) => {
                    StatusCode::CONFLICT
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return Err((
                status,
                Json(json!({
                    "runId": run_id,
                    "threadId": thread_id,
                    "error": error.to_string(),
                })),
            ));
        }
    };
    let start_result = if managed_attachment_claims.is_empty() {
        state.integration.bridge.dispatch(admitted, callback).await
    } else {
        let plan = match state
            .integration
            .bridge
            .prepare_durable_dispatch(admitted, format!("queued_input:{}", Uuid::new_v4()))
            .await
        {
            Ok(plan) => plan,
            Err(error) => {
                bound_stream.abort();
                for task in stream_tasks {
                    task.abort();
                }
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "runId": run_id,
                        "threadId": thread_id,
                        "error": error,
                    })),
                ));
            }
        };
        let effective_run_id = plan.effective_run_id().to_owned();
        if let Err(error) = state
            .ops
            .prompt_attachments
            .claim_standalone(
                ("__legacy_api__", 0),
                &thread_id,
                DispatchAdmissionKind::ChatStart,
                None,
                Some(&run_id),
                &effective_run_id,
                &managed_attachment_claims,
            )
            .await
        {
            bound_stream.abort();
            for task in stream_tasks {
                task.abort();
            }
            let status = match error {
                PromptAttachmentLifecycleError::Invalid(_) => StatusCode::BAD_REQUEST,
                PromptAttachmentLifecycleError::Conflict(_) => StatusCode::CONFLICT,
                PromptAttachmentLifecycleError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return Err((
                status,
                Json(json!({
                    "runId": run_id,
                    "threadId": thread_id,
                    "error": error.to_string(),
                })),
            ));
        }
        state
            .integration
            .bridge
            .execute_durable_dispatch(plan, callback)
            .await
    };

    match start_result {
        Ok(outcome) => {
            match &outcome {
                AgentDispatchOutcome::Started => bound_stream.detach(),
                AgentDispatchOutcome::QueuedToActiveRun { .. } => {
                    // The reply belongs to the already-active run, which the
                    // fresh-run-keyed subscriptions will never see; keeping
                    // them alive leaks until process exit. That covers both
                    // the bound channel stream and any caller-attached
                    // committed-stream task (the chat WS handler's per-start
                    // subscriber waiting on this run id).
                    bound_stream.abort();
                    for task in &stream_tasks {
                        task.abort();
                    }
                }
            }
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
                dispatch_outcome: None,
                effective_run_id: None,
                pending_input_id: None,
                delivery_state: None,
                idempotency_replay: None,
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
    let thread_id = match resolve_existing_thread_key(request.thread_id.clone()) {
        Ok(thread_id) => thread_id,
        Err((_status, payload)) => {
            let _ = out_tx.send(json!({
                "type": "error",
                "error": payload.0.get("error").and_then(serde_json::Value::as_str).unwrap_or("invalid thread id")
            }));
            return;
        }
    };
    let (_status, payload) = execute_chat_stream_input(state, thread_id, request).await;
    let _ = out_tx.send(json!({
        "type": "stream_input",
        "status": payload.status,
        "threadStatus": payload.thread_status,
        "clientIntentId": payload.client_intent_id,
        "pendingInputId": payload.pending_input_id,
        "effectiveRunId": payload.effective_run_id,
        "threadId": payload.thread_id,
        "deliveryState": payload.delivery_state,
        "idempotencyReplay": payload.idempotency_replay,
        "error": payload.error,
        "message": payload.message
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
    let snapshot = crate::routes::thread_history_for_key(
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
