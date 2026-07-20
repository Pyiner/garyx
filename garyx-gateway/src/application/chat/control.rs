use std::sync::Arc;

use axum::http::StatusCode;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::application::chat::contracts::{
    InterruptResponse, StreamInputRequest, StreamInputResponse,
};
use crate::chat_shared::{interrupt_response, stream_input_response};
use crate::conversation_admission::{
    AdmissionOperationResult, AdmissionRegistration, AdmissionRegistrationError,
    resolve_stream_input_correlation, stream_input_request_fingerprint,
};
use crate::garyx_db::{
    DispatchAdmissionKey, DispatchAdmissionKind, DispatchAdmissionRecord, DispatchAdmissionState,
    DispatchOutcome,
};
use crate::prompt_attachment_lifecycle::PromptAttachmentLifecycleError;
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
    mut request: StreamInputRequest,
) -> (StatusCode, StreamInputResponse) {
    let correlation = match resolve_stream_input_correlation(&mut request) {
        Ok(correlation) => correlation,
        Err((status, payload)) => {
            return stream_input_error_response(status, thread_id, payload, false);
        }
    };
    let Some(correlation) = correlation else {
        return execute_chat_stream_input_legacy(state, thread_id, request).await;
    };
    let fingerprint = stream_input_request_fingerprint(&thread_id, &request);
    let key = DispatchAdmissionKey {
        scope_identity: correlation.scope_identity,
        scope_epoch: correlation.scope_epoch,
        thread_id: thread_id.clone(),
        kind: DispatchAdmissionKind::StreamInput,
        client_intent_id: correlation.client_intent_id,
    };
    match state.ops.conversation_admission.read(key.clone()).await {
        Ok(Some(record)) => {
            if record.request_fingerprint != fingerprint {
                return stream_input_error_response(
                    StatusCode::CONFLICT,
                    thread_id,
                    json!({
                        "error": "idempotency_conflict",
                        "message": "clientIntentId was reused with a different request",
                    }),
                    true,
                );
            }
            if !matches!(record.state, DispatchAdmissionState::Admitted) {
                return stream_input_response_from_record(record, true);
            }
        }
        Ok(None) => {}
        Err(error) => {
            return stream_input_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                thread_id,
                json!({
                    "error": "dispatch_admission_storage_error",
                    "message": error,
                }),
                false,
            );
        }
    }

    let registration = match state
        .ops
        .conversation_admission
        .register(key.clone(), &fingerprint)
    {
        Ok(registration) => registration,
        Err(error) => {
            let (status, code) = match error {
                AdmissionRegistrationError::FingerprintConflict => {
                    (StatusCode::CONFLICT, "idempotency_conflict")
                }
                AdmissionRegistrationError::Overloaded => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "dispatch_admission_overloaded",
                ),
            };
            return stream_input_error_response(
                status,
                thread_id,
                json!({"error": code, "message": error.to_string()}),
                false,
            );
        }
    };
    let response_key = key.clone();
    let (join, replay) = match registration {
        AdmissionRegistration::Join(join) => (join, true),
        AdmissionRegistration::Owner(owner) => {
            let join = owner.join_handle();
            let owner_state = Arc::clone(state);
            tokio::spawn(async move {
                let result = run_durable_stream_input(owner_state, request, key, fingerprint).await;
                owner.publish(result);
            });
            (join, false)
        }
    };
    match join.wait().await.as_ref() {
        AdmissionOperationResult::Ready => {
            match state.ops.conversation_admission.read(response_key).await {
                Ok(Some(record)) => stream_input_response_from_record(record, replay),
                Ok(None) => stream_input_error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    thread_id,
                    json!({
                        "error": "dispatch_admission_storage_error",
                        "message": "dispatch owner completed without a durable admission row",
                    }),
                    replay,
                ),
                Err(error) => stream_input_error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    thread_id,
                    json!({
                        "error": "dispatch_admission_storage_error",
                        "message": error,
                    }),
                    replay,
                ),
            }
        }
        AdmissionOperationResult::Failed { status, payload } => {
            stream_input_error_response(*status, thread_id, payload.clone(), replay)
        }
    }
}

async fn execute_chat_stream_input_legacy(
    state: &Arc<AppState>,
    thread_id: String,
    mut request: StreamInputRequest,
) -> (StatusCode, StreamInputResponse) {
    let bridge = &state.integration.bridge;
    let attachment_claims = match state
        .ops
        .prompt_attachments
        .prepare_claims(("__legacy_api__", 0), &mut request.attachments)
        .await
    {
        Ok(claims) => claims,
        Err(PromptAttachmentLifecycleError::Invalid(message)) => {
            return stream_input_error_response(
                StatusCode::BAD_REQUEST,
                thread_id,
                json!({"error": "invalid_prompt_attachment", "message": message}),
                false,
            );
        }
        Err(PromptAttachmentLifecycleError::Conflict(message)) => {
            return stream_input_error_response(
                StatusCode::CONFLICT,
                thread_id,
                json!({"error": "prompt_attachment_conflict", "message": message}),
                false,
            );
        }
        Err(PromptAttachmentLifecycleError::Storage(message)) => {
            return stream_input_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                thread_id,
                json!({"error": "prompt_attachment_storage_error", "message": message}),
                false,
            );
        }
    };
    let effective_message = state
        .config_snapshot()
        .resolve_slash_command(&request.message)
        .and_then(|command| command.prompt)
        .unwrap_or(request.message);

    let queued = if attachment_claims.is_empty() {
        bridge
            .add_streaming_input(
                &thread_id,
                &effective_message,
                Some(request.images),
                Some(request.files),
                Some(request.attachments),
                request.client_intent_id.clone(),
            )
            .await
    } else {
        let plan = bridge
            .prepare_durable_stream_input(
                thread_id.clone(),
                effective_message,
                Some(request.images),
                Some(request.files),
                Some(request.attachments),
                request.client_intent_id.clone(),
                format!("queued_input:{}", Uuid::new_v4()),
            )
            .await;
        let Some(effective_run_id) = plan.effective_run_id().map(ToOwned::to_owned) else {
            return (
                StatusCode::OK,
                stream_input_response(
                    "no_active_session",
                    Some("no_active_thread".to_owned()),
                    request.client_intent_id,
                    None,
                    thread_id,
                ),
            );
        };
        if let Err(error) = state
            .ops
            .prompt_attachments
            .claim_standalone(
                ("__legacy_api__", 0),
                &thread_id,
                DispatchAdmissionKind::StreamInput,
                request.client_intent_id.as_deref(),
                None,
                &effective_run_id,
                &attachment_claims,
            )
            .await
        {
            let (status, code) = match error {
                PromptAttachmentLifecycleError::Invalid(_) => {
                    (StatusCode::BAD_REQUEST, "invalid_prompt_attachment")
                }
                PromptAttachmentLifecycleError::Conflict(_) => {
                    (StatusCode::CONFLICT, "prompt_attachment_conflict")
                }
                PromptAttachmentLifecycleError::Storage(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "prompt_attachment_storage_error",
                ),
            };
            return stream_input_error_response(
                status,
                thread_id,
                json!({"error": code, "message": error.to_string()}),
                false,
            );
        }
        match bridge.execute_durable_stream_input(plan).await {
            Ok(queued) => queued,
            Err(error) => {
                return stream_input_error_response(
                    StatusCode::CONFLICT,
                    thread_id,
                    json!({"error": "dispatch_ambiguous", "message": error}),
                    false,
                );
            }
        }
    };

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
            request.client_intent_id,
            pending_input_id,
            thread_id,
        ),
    )
}

fn stream_input_error_response(
    status: StatusCode,
    thread_id: String,
    payload: Value,
    replay: bool,
) -> (StatusCode, StreamInputResponse) {
    let error = payload
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("stream_input_failed")
        .to_owned();
    let message = payload
        .get("message")
        .or_else(|| payload.get("error"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    (
        status,
        StreamInputResponse {
            status: "error".to_owned(),
            thread_status: None,
            client_intent_id: None,
            pending_input_id: None,
            effective_run_id: None,
            thread_id,
            delivery_state: None,
            idempotency_replay: Some(replay),
            error: Some(error),
            message,
        },
    )
}

fn stream_input_response_from_record(
    record: DispatchAdmissionRecord,
    replay: bool,
) -> (StatusCode, StreamInputResponse) {
    let (status, thread_status, http_status, error, message) = match record.state {
        DispatchAdmissionState::Accepted => (
            "queued",
            Some("queued".to_owned()),
            StatusCode::OK,
            None,
            None,
        ),
        DispatchAdmissionState::NotDispatched => (
            "no_active_session",
            Some("no_active_thread".to_owned()),
            StatusCode::OK,
            None,
            None,
        ),
        DispatchAdmissionState::Rejected => (
            "error",
            None,
            record
                .result_http_status
                .and_then(|value| u16::try_from(value).ok())
                .and_then(|value| StatusCode::from_u16(value).ok())
                .unwrap_or(StatusCode::CONFLICT),
            Some(
                record
                    .result_error_code
                    .clone()
                    .unwrap_or_else(|| "dispatch_rejected".to_owned()),
            ),
            record.result_error_message.clone(),
        ),
        DispatchAdmissionState::Admitted => (
            "error",
            None,
            StatusCode::SERVICE_UNAVAILABLE,
            Some("dispatch_admission_pending".to_owned()),
            Some("dispatch is durably admitted and can be resumed".to_owned()),
        ),
        DispatchAdmissionState::HandoffStarted | DispatchAdmissionState::Ambiguous => (
            "ambiguous",
            None,
            StatusCode::CONFLICT,
            Some("dispatch_ambiguous".to_owned()),
            record.result_error_message.clone().or_else(|| {
                Some("provider handoff outcome is unknown and will not be reissued".to_owned())
            }),
        ),
    };
    (
        http_status,
        StreamInputResponse {
            status: status.to_owned(),
            thread_status,
            client_intent_id: Some(record.key.client_intent_id),
            pending_input_id: record.pending_input_id,
            effective_run_id: record.effective_run_id,
            thread_id: record.key.thread_id,
            delivery_state: Some(record.state.as_str().to_owned()),
            idempotency_replay: Some(replay),
            error,
            message,
        },
    )
}

fn stream_input_operation_failure(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> AdmissionOperationResult {
    AdmissionOperationResult::Failed {
        status,
        payload: json!({"error": code, "message": message.into()}),
    }
}

async fn run_durable_stream_input(
    state: Arc<AppState>,
    mut request: StreamInputRequest,
    key: DispatchAdmissionKey,
    fingerprint: String,
) -> AdmissionOperationResult {
    let existing = match state.ops.conversation_admission.read(key.clone()).await {
        Ok(record) => record,
        Err(error) => {
            return stream_input_operation_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_admission_storage_error",
                error,
            );
        }
    };
    if let Some(record) = existing.as_ref() {
        if record.fingerprint_version != 1 || record.request_fingerprint != fingerprint {
            return stream_input_operation_failure(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "clientIntentId was reused with a different request",
            );
        }
        if !matches!(record.state, DispatchAdmissionState::Admitted) {
            return AdmissionOperationResult::Ready;
        }
    }

    let effective_message = state
        .config_snapshot()
        .resolve_slash_command(&request.message)
        .and_then(|command| command.prompt)
        .unwrap_or(request.message);
    let pending_input_id = existing
        .as_ref()
        .and_then(|record| record.pending_input_id.clone())
        .unwrap_or_else(|| format!("queued_input:{}", Uuid::new_v4()));
    let attachment_claims = match state
        .ops
        .prompt_attachments
        .prepare_claims(
            (key.scope_identity.as_str(), key.scope_epoch),
            &mut request.attachments,
        )
        .await
    {
        Ok(claims) => claims,
        Err(PromptAttachmentLifecycleError::Invalid(message)) => {
            return stream_input_operation_failure(
                StatusCode::BAD_REQUEST,
                "invalid_prompt_attachment",
                message,
            );
        }
        Err(PromptAttachmentLifecycleError::Conflict(message)) => {
            return stream_input_operation_failure(
                StatusCode::CONFLICT,
                "prompt_attachment_conflict",
                message,
            );
        }
        Err(PromptAttachmentLifecycleError::Storage(message)) => {
            return stream_input_operation_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "prompt_attachment_storage_error",
                message,
            );
        }
    };
    let plan = state
        .integration
        .bridge
        .prepare_durable_stream_input(
            key.thread_id.clone(),
            effective_message,
            Some(request.images),
            Some(request.files),
            Some(request.attachments),
            Some(key.client_intent_id.clone()),
            pending_input_id,
        )
        .await;
    let planned_effective_run_id = plan.effective_run_id().map(ToOwned::to_owned);
    let planned_pending_input_id = plan.pending_input_id().map(ToOwned::to_owned);

    let Some(effective_run_id) = planned_effective_run_id else {
        if existing.is_some() {
            return stream_input_operation_failure(
                StatusCode::CONFLICT,
                "dispatch_plan_changed",
                "the active run ended before an admitted stream-input could resume",
            );
        }
        return match state
            .ops
            .conversation_admission
            .insert_no_active(key, fingerprint.clone())
            .await
        {
            Ok(record) if record.request_fingerprint == fingerprint => {
                AdmissionOperationResult::Ready
            }
            Ok(_) => stream_input_operation_failure(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "clientIntentId was reused with a different request",
            ),
            Err(error) => stream_input_operation_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_admission_storage_error",
                error,
            ),
        };
    };
    let pending_input_id =
        planned_pending_input_id.expect("active durable stream-input plan always has a pending id");
    let record = match existing {
        Some(record) => record,
        None => match state
            .ops
            .conversation_admission
            .insert(
                key.clone(),
                fingerprint.clone(),
                None,
                Some(effective_run_id.clone()),
                Some(pending_input_id.clone()),
                Some(DispatchOutcome::QueuedToActiveRun),
                attachment_claims,
            )
            .await
        {
            Ok(record) => record,
            Err(error) => {
                return stream_input_operation_failure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "dispatch_admission_storage_error",
                    error,
                );
            }
        },
    };
    if record.request_fingerprint != fingerprint {
        return stream_input_operation_failure(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "clientIntentId was reused with a different request",
        );
    }
    if !matches!(record.state, DispatchAdmissionState::Admitted) {
        return AdmissionOperationResult::Ready;
    }
    if record.outcome != Some(DispatchOutcome::QueuedToActiveRun)
        || record.effective_run_id.as_deref() != Some(effective_run_id.as_str())
        || record.pending_input_id.as_deref() != Some(pending_input_id.as_str())
    {
        return stream_input_operation_failure(
            StatusCode::CONFLICT,
            "dispatch_plan_changed",
            "the durable stream-input plan no longer matches the admitted identifiers",
        );
    }
    match state
        .ops
        .conversation_admission
        .start_handoff(key.clone())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return AdmissionOperationResult::Ready,
        Err(error) => {
            return stream_input_operation_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_admission_storage_error",
                error,
            );
        }
    }
    match state
        .integration
        .bridge
        .execute_durable_stream_input(plan)
        .await
    {
        Ok(Some(queued)) => match state
            .ops
            .conversation_admission
            .settle(
                key,
                DispatchAdmissionState::Accepted,
                Some(DispatchOutcome::QueuedToActiveRun),
                Some(queued.run_id),
                Some(queued.pending_input_id),
                200,
                None,
                None,
            )
            .await
        {
            Ok(_) => AdmissionOperationResult::Ready,
            Err(error) => stream_input_operation_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_settlement_failed",
                error,
            ),
        },
        Ok(None) => stream_input_operation_failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "dispatch_plan_invalid",
            "active stream-input plan executed as no-active",
        ),
        Err(error) => match state
            .ops
            .conversation_admission
            .settle(
                key,
                DispatchAdmissionState::Ambiguous,
                Some(DispatchOutcome::QueuedToActiveRun),
                Some(effective_run_id),
                Some(pending_input_id),
                409,
                Some("dispatch_ambiguous".to_owned()),
                Some(error),
            )
            .await
        {
            Ok(_) => AdmissionOperationResult::Ready,
            Err(error) => stream_input_operation_failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_settlement_failed",
                error,
            ),
        },
    }
}
