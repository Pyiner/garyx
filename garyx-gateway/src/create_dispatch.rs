use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, AgentDispatchOutcome, AgentRunRequest,
    FORK_FROM_PROVIDER_TYPE_METADATA_KEY, FORK_FROM_SDK_SESSION_ID_METADATA_KEY,
    FORK_FROM_THREAD_ID_METADATA_KEY, FilePayload, ImagePayload, MODEL_METADATA_KEY,
    MODEL_REASONING_EFFORT_METADATA_KEY, MODEL_SERVICE_TIER_METADATA_KEY, PromptAttachment,
    SDK_SESSION_FORK_METADATA_KEY, attachments_to_metadata_value, stage_file_payloads_for_prompt,
    stage_image_payloads_for_prompt,
};
use garyx_models::{MessageLifecycleStatus, strip_server_owned_agent_metadata};
use garyx_router::{
    AdmittedRun, ChannelBinding, EndpointBindingMutationError, ThreadCreationError,
    ThreadEnsureOptions, ThreadTerminalState, build_runtime_context_metadata, is_thread_key,
    new_thread_key, planned_thread_worktree_path, workspace_dir_from_value,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::agent_identity::{AgentBindingIntent, prepare_thread_for_agent_reference};
use crate::application::chat::contracts::IdempotencyScope;
use crate::application::chat::contracts::{ChatRequest, StartChatResponse};
use crate::application::chat::prepare::ImplicitThreadCreatePlan;
use crate::chat::{
    ChatStreamCallbackBuilder, compose_stream_callbacks, start_response_from_record,
};
use crate::chat_delivery::build_bound_response_callback;
use crate::conversation_admission::{
    AdmissionOperationResult, AdmissionRegistration, AdmissionRegistrationError,
    stable_json_fingerprint, validate_explicit_idempotency_scope, validate_intent_id,
};
use crate::create_resources::{
    begin_create_resource, cleanup_unadopted_create_resources, create_lease_expires_at,
    mark_create_resource_materialized, materialize_managed_workspace,
    remove_adopted_resource_markers, start_create_lease_heartbeat,
};
use crate::garyx_db::{
    CreateCommandKind, CreateIntentKey, CreateIntentSnapshot, CreateIntentState,
    CreateResourceKind, DispatchAdmissionKey, DispatchAdmissionKind, DispatchAdmissionRecord,
    DispatchAdmissionState, DispatchOutcome, NewCreateIntent,
};
use crate::prompt_attachment_lifecycle::PromptAttachmentLifecycleError;
use crate::provider_session_locator::recover_local_provider_session;
use crate::routes::{
    CreateThreadBody, fork_source_sdk_session_id, is_resume_provider,
    materialize_imported_thread_history, parse_sdk_session_provider_hint, provider_hint_label,
    provider_type_from_thread_value, thread_summary,
};
use crate::server::AppState;
use crate::sqlite_thread_store::{AtomicCreateCommit, AtomicCreateDispatchLedger};
use crate::workspace_mode::worktree_base_dir_for_data_dir;

#[cfg(test)]
#[derive(Clone)]
struct CreateCommitBarrier {
    create_intent_id: String,
    committed: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
static CREATE_COMMIT_BARRIER: std::sync::Mutex<Option<CreateCommitBarrier>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
static STOP_AFTER_CREATE_COMMIT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(test)]
async fn maybe_block_after_create_commit(create_intent_id: &str) {
    let barrier = CREATE_COMMIT_BARRIER
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_ref()
        .filter(|barrier| barrier.create_intent_id == create_intent_id)
        .cloned();
    if let Some(barrier) = barrier {
        barrier.committed.notify_one();
        barrier.release.notified().await;
    }
}

#[cfg(not(test))]
async fn maybe_block_after_create_commit(_create_intent_id: &str) {}

#[cfg(test)]
fn stop_after_create_commit(create_intent_id: &str) -> bool {
    STOP_AFTER_CREATE_COMMIT
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_deref()
        == Some(create_intent_id)
}

#[cfg(not(test))]
fn stop_after_create_commit(_create_intent_id: &str) -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtomicCreateBinding {
    pub bot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtomicDispatchBody {
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
    #[serde(default)]
    pub images: Vec<ImagePayload>,
    #[serde(default)]
    pub files: Vec<FilePayload>,
    #[serde(default = "default_account_id")]
    pub account_id: String,
    #[serde(default = "default_from_id")]
    pub from_id: String,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

fn default_account_id() -> String {
    "main".to_owned()
}

fn default_from_id() -> String {
    "api-user".to_owned()
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAndDispatchBody {
    pub idempotency_scope: IdempotencyScope,
    pub create_intent_id: String,
    pub client_intent_id: String,
    pub thread: CreateThreadBody,
    #[serde(default)]
    pub binding: Option<AtomicCreateBinding>,
    pub dispatch: AtomicDispatchBody,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIntentQuery {
    pub scope_identity: String,
    pub scope_epoch: i64,
    pub create_intent_id: String,
}

struct ValidatedCreateCommand {
    body: CreateAndDispatchBody,
    key: CreateIntentKey,
    fingerprint: String,
    dispatch_channel: String,
    resolved_binding: Option<ChannelBinding>,
    origin_channel: Option<String>,
    origin_account_id: Option<String>,
    origin_from_id: Option<String>,
    dispatch_fingerprint: Option<String>,
    callback_builder: Option<ChatStreamCallbackBuilder>,
}

enum RequestedCreateBinding {
    Resolved(ChannelBinding),
    PublicBot {
        bot_id: String,
        channel: String,
        account_id: String,
    },
}

fn validate_create_command(
    mut body: CreateAndDispatchBody,
) -> Result<ValidatedCreateCommand, (StatusCode, Json<Value>)> {
    if body.thread.idempotency_scope.is_some() || body.thread.create_intent_id.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "nested thread create intent fields are not allowed"
            })),
        ));
    }
    let (scope_identity, scope_epoch) =
        validate_explicit_idempotency_scope(&body.idempotency_scope)
            .map_err(|(status, payload)| (status, Json(payload)))?;
    let create_intent_id = validate_intent_id("createIntentId", &body.create_intent_id, true)
        .map_err(|(status, payload)| (status, Json(payload)))?;
    let client_intent_id = validate_intent_id("clientIntentId", &body.client_intent_id, false)
        .map_err(|(status, payload)| (status, Json(payload)))?;
    body.create_intent_id = create_intent_id.clone();
    body.client_intent_id = client_intent_id;
    strip_server_owned_agent_metadata(&mut body.thread.metadata);
    strip_server_owned_agent_metadata(&mut body.dispatch.metadata);
    let fingerprint = stable_json_fingerprint(json!({
        "fingerprint_version": 1,
        "command_kind": "create_and_dispatch",
        "thread": &body.thread,
        "binding": &body.binding,
        "dispatch": &body.dispatch,
    }));
    Ok(ValidatedCreateCommand {
        body,
        key: CreateIntentKey {
            scope_identity,
            scope_epoch,
            create_intent_id,
        },
        fingerprint,
        dispatch_channel: "api".to_owned(),
        resolved_binding: None,
        origin_channel: None,
        origin_account_id: None,
        origin_from_id: None,
        dispatch_fingerprint: None,
        callback_builder: None,
    })
}

fn operation_error(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> AdmissionOperationResult {
    AdmissionOperationResult::Failed {
        status,
        payload: json!({"error": code, "message": message.into()}),
    }
}

fn registration_error(error: AdmissionRegistrationError) -> (StatusCode, Json<Value>) {
    let (status, code) = match error {
        AdmissionRegistrationError::FingerprintConflict => {
            (StatusCode::CONFLICT, "idempotency_conflict")
        }
        AdmissionRegistrationError::Overloaded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dispatch_admission_overloaded",
        ),
    };
    (
        status,
        Json(json!({"error": code, "message": error.to_string()})),
    )
}

fn ensure_claim_matches(
    claim: &crate::garyx_db::CreateIntentRecord,
    fingerprint: &str,
    client_intent_id: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    if claim.fingerprint_version == 1
        && claim.request_fingerprint == fingerprint
        && claim.command_kind == CreateCommandKind::CreateAndDispatch
        && claim.dispatch_client_intent_id.as_deref() == Some(client_intent_id)
    {
        return Ok(());
    }
    Err((
        StatusCode::CONFLICT,
        Json(json!({
            "error": "idempotency_conflict",
            "message": "createIntentId was reused with a different command",
            "createIntentId": claim.key.create_intent_id,
            "threadId": claim.thread_id,
        })),
    ))
}

pub async fn create_and_dispatch(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateAndDispatchBody>,
) -> impl IntoResponse {
    let validated = match validate_create_command(body) {
        Ok(validated) => validated,
        Err(error) => return error.into_response(),
    };
    let response_key = validated.key.clone();
    match execute_create_and_dispatch(Arc::clone(&state), validated).await {
        Ok(replay) => create_response_from_snapshot(&state, &response_key, replay).await,
        Err((status, payload)) => (status, payload).into_response(),
    }
}

fn implicit_create_request_fingerprint(dispatch_fingerprint: &str) -> String {
    stable_json_fingerprint(json!({
        "fingerprint_version": 1,
        "command_kind": "implicit_create_and_dispatch",
        "dispatch_fingerprint": dispatch_fingerprint,
    }))
}

async fn execute_create_and_dispatch(
    state: Arc<AppState>,
    validated: ValidatedCreateCommand,
) -> Result<bool, (StatusCode, Json<Value>)> {
    let candidate_thread_id = new_thread_key();
    let key_for_reserve = validated.key.clone();
    let fingerprint_for_reserve = validated.fingerprint.clone();
    let client_intent_for_reserve = validated.body.client_intent_id.clone();
    let claim = match state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            db.reserve_create_intent(NewCreateIntent {
                key: &key_for_reserve,
                thread_id: &candidate_thread_id,
                request_fingerprint: &fingerprint_for_reserve,
                command_kind: CreateCommandKind::CreateAndDispatch,
                dispatch_client_intent_id: Some(&client_intent_for_reserve),
            })
        })
        .await
    {
        Ok(claim) => claim,
        Err(error) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "create_intent_storage_error",
                    "message": error.to_string(),
                })),
            ));
        }
    };
    if let Err(error) = ensure_claim_matches(
        &claim,
        &validated.fingerprint,
        &validated.body.client_intent_id,
    ) {
        return Err(error);
    }
    let resume_committed = if claim.state == CreateIntentState::Committed {
        let snapshot_key = validated.key.clone();
        let db = Arc::clone(&state.ops.garyx_db);
        let snapshot = db
            .run_blocking(move |db| db.create_intent_snapshot(&snapshot_key))
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": "create_intent_storage_error",
                        "message": error.to_string(),
                    })),
                )
            })?
            .ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "committed create intent disappeared"})),
                )
            })?;
        match snapshot.dispatch.as_ref().map(|row| row.state) {
            Some(DispatchAdmissionState::Admitted) => true,
            _ => return Ok(true),
        }
    } else {
        false
    };
    if claim.state == CreateIntentState::FailedBeforeCommit {
        if let Err(error) =
            cleanup_unadopted_create_resources(&state, &validated.key, &claim.thread_id).await
        {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "create_resource_cleanup_failed",
                    "message": error,
                    "createIntentId": claim.key.create_intent_id,
                    "threadId": claim.thread_id,
                    "state": claim.state.as_str(),
                    "idempotencyReplay": true,
                })),
            ));
        }
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": claim.failure_code.unwrap_or_else(|| "create_failed_before_commit".to_owned()),
                "message": claim.failure_message,
                "createIntentId": claim.key.create_intent_id,
                "threadId": claim.thread_id,
                "state": claim.state.as_str(),
                "idempotencyReplay": true,
            })),
        ));
    }

    let registration = match state
        .ops
        .conversation_admission
        .register_create(validated.key.clone(), &validated.fingerprint)
    {
        Ok(registration) => registration,
        Err(error) => return Err(registration_error(error)),
    };
    let (join, replay) = match registration {
        AdmissionRegistration::Join(join) => (join, true),
        AdmissionRegistration::Owner(owner) => {
            let join = owner.join_handle();
            let owner_state = Arc::clone(&state);
            tokio::spawn(async move {
                let result = if resume_committed {
                    resume_committed_create_dispatch(owner_state, validated, claim).await
                } else {
                    run_create_and_dispatch(owner_state, validated, claim).await
                };
                owner.publish(result);
            });
            (join, false)
        }
    };
    match join.wait().await.as_ref() {
        AdmissionOperationResult::Ready => Ok(replay || resume_committed),
        AdmissionOperationResult::Failed { status, payload } => {
            Err((*status, Json(payload.clone())))
        }
    }
}

pub(crate) async fn create_implicit_and_dispatch(
    state: &Arc<AppState>,
    scope_identity: String,
    scope_epoch: i64,
    client_intent_id: String,
    mut request: ChatRequest,
    plan: ImplicitThreadCreatePlan,
    callback_builder: Option<ChatStreamCallbackBuilder>,
) -> Result<StartChatResponse, (StatusCode, Json<Value>)> {
    let create_intent_id = format!("implicit:{:x}", Sha256::digest(client_intent_id.as_bytes()));
    let dispatch_fingerprint = crate::conversation_admission::chat_request_fingerprint(&request);
    // The implicit create claim represents the raw client operation. Do not
    // include the endpoint's server timestamp or defaults resolved from the
    // current config: either can legitimately differ after a pre-commit
    // restart while the client request is byte-for-byte the same.
    let fingerprint = implicit_create_request_fingerprint(&dispatch_fingerprint);
    request.metadata.extend(plan.dispatch_metadata.clone());
    let label = crate::chat_application::prompt_derived_thread_label(&request.message)
        .unwrap_or_else(|| plan.label.clone());
    let thread = CreateThreadBody {
        no_workspace: false,
        idempotency_scope: None,
        create_intent_id: None,
        label: Some(label),
        workspace_dir: plan.workspace_dir.clone(),
        workspace_mode: plan.workspace_mode,
        metadata: HashMap::new(),
        agent_id: plan.agent_id.clone(),
        model: None,
        model_reasoning_effort: None,
        model_service_tier: None,
        sdk_session_id: None,
        sdk_session_provider_hint: None,
        fork_from_thread_id: None,
    };
    let body = CreateAndDispatchBody {
        idempotency_scope: IdempotencyScope {
            identity: scope_identity.clone(),
            epoch: scope_epoch,
        },
        create_intent_id: create_intent_id.clone(),
        client_intent_id: client_intent_id.clone(),
        thread,
        binding: None,
        dispatch: AtomicDispatchBody {
            message: request.message,
            attachments: request.attachments,
            images: request.images,
            files: request.files,
            account_id: plan.account_id.clone(),
            from_id: plan.from_id.clone(),
            metadata: request.metadata,
        },
    };
    let key = CreateIntentKey {
        scope_identity,
        scope_epoch,
        create_intent_id,
    };
    let validated = ValidatedCreateCommand {
        body,
        key: key.clone(),
        fingerprint,
        dispatch_channel: plan.channel.clone(),
        resolved_binding: Some(plan.binding),
        origin_channel: Some(plan.channel),
        origin_account_id: Some(plan.account_id),
        origin_from_id: Some(plan.from_id),
        dispatch_fingerprint: Some(dispatch_fingerprint),
        callback_builder,
    };
    let replay = execute_create_and_dispatch(Arc::clone(state), validated).await?;
    let db = Arc::clone(&state.ops.garyx_db);
    let snapshot = db
        .run_blocking(move |db| db.create_intent_snapshot(&key))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "create_intent_storage_error",
                    "message": error.to_string(),
                })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "committed create intent disappeared"})),
            )
        })?;
    let dispatch = snapshot.dispatch.ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "atomic create completed without dispatch admission"})),
        )
    })?;
    start_response_from_record(dispatch, replay)
}

pub(crate) async fn create_only_thread(
    state: Arc<AppState>,
    mut body: CreateThreadBody,
) -> axum::response::Response {
    let scope = body
        .idempotency_scope
        .take()
        .expect("create-only wrapper validates scope pair");
    let raw_create_intent_id = body
        .create_intent_id
        .take()
        .expect("create-only wrapper validates scope pair");
    let (scope_identity, scope_epoch) = match validate_explicit_idempotency_scope(&scope) {
        Ok(scope) => scope,
        Err((status, payload)) => return (status, Json(payload)).into_response(),
    };
    let create_intent_id = match validate_intent_id("createIntentId", &raw_create_intent_id, true) {
        Ok(intent) => intent,
        Err((status, payload)) => return (status, Json(payload)).into_response(),
    };
    strip_server_owned_agent_metadata(&mut body.metadata);
    let fingerprint = stable_json_fingerprint(json!({
        "fingerprint_version": 1,
        "command_kind": "create_only",
        "thread": &body,
    }));
    let key = CreateIntentKey {
        scope_identity,
        scope_epoch,
        create_intent_id,
    };
    let candidate_thread_id = new_thread_key();
    let reserve_key = key.clone();
    let reserve_fingerprint = fingerprint.clone();
    let claim = match state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            db.reserve_create_intent(NewCreateIntent {
                key: &reserve_key,
                thread_id: &candidate_thread_id,
                request_fingerprint: &reserve_fingerprint,
                command_kind: CreateCommandKind::CreateOnly,
                dispatch_client_intent_id: None,
            })
        })
        .await
    {
        Ok(claim) => claim,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "create_intent_storage_error",
                    "message": error.to_string(),
                })),
            )
                .into_response();
        }
    };
    if claim.fingerprint_version != 1
        || claim.request_fingerprint != fingerprint
        || claim.command_kind != CreateCommandKind::CreateOnly
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": "createIntentId was reused with a different command",
                "threadId": claim.thread_id,
            })),
        )
            .into_response();
    }
    if claim.state == CreateIntentState::Committed {
        return create_response_from_snapshot(&state, &key, true).await;
    }
    if claim.state == CreateIntentState::FailedBeforeCommit {
        if let Err(error) = cleanup_unadopted_create_resources(&state, &key, &claim.thread_id).await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "create_resource_cleanup_failed",
                    "message": error,
                    "threadId": claim.thread_id,
                    "idempotencyReplay": true,
                })),
            )
                .into_response();
        }
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": claim.failure_code.unwrap_or_else(|| "create_failed_before_commit".to_owned()),
                "message": claim.failure_message,
                "threadId": claim.thread_id,
                "idempotencyReplay": true,
            })),
        )
            .into_response();
    }
    let registration = match state
        .ops
        .conversation_admission
        .register_create(key.clone(), &fingerprint)
    {
        Ok(registration) => registration,
        Err(error) => return registration_error(error).into_response(),
    };
    let response_key = key.clone();
    let (join, replay) = match registration {
        AdmissionRegistration::Join(join) => (join, true),
        AdmissionRegistration::Owner(owner) => {
            let join = owner.join_handle();
            let owner_state = Arc::clone(&state);
            tokio::spawn(async move {
                let result = run_create_only(owner_state, key, fingerprint, claim, body).await;
                owner.publish(result);
            });
            (join, false)
        }
    };
    match join.wait().await.as_ref() {
        AdmissionOperationResult::Ready => {
            create_response_from_snapshot(&state, &response_key, replay).await
        }
        AdmissionOperationResult::Failed { status, payload } => {
            (*status, Json(payload.clone())).into_response()
        }
    }
}

async fn release_preparation_claim(state: &Arc<AppState>, key: &CreateIntentKey, message: &str) {
    let db = Arc::clone(&state.ops.garyx_db);
    let key = key.clone();
    let message = message.to_owned();
    if let Err(error) = db
        .run_blocking(move |db| {
            db.release_create_intent_after_preparation_failure(&key, &message)
                .map(|_| ())
        })
        .await
    {
        tracing::error!(%error, "failed to release create-intent preparation lease");
    }
}

async fn definitive_preparation_error(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    code: &str,
    message: impl Into<String>,
) -> AdmissionOperationResult {
    let message = message.into();
    fail_claim(state, key, code, &message).await;
    operation_error(StatusCode::BAD_REQUEST, code, message)
}

async fn transient_preparation_error(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    code: &str,
    message: impl Into<String>,
) -> AdmissionOperationResult {
    let message = message.into();
    release_preparation_claim(state, key, &message).await;
    operation_error(StatusCode::INTERNAL_SERVER_ERROR, code, message)
}

async fn prepare_atomic_thread_record(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    thread_id: &str,
    body: &CreateThreadBody,
    origin_channel: Option<String>,
    origin_account_id: Option<String>,
    origin_from_id: Option<String>,
) -> Result<Value, AdmissionOperationResult> {
    let requested_session_id = body
        .sdk_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let requested_fork_thread_id = body
        .fork_from_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested_session_id.is_some() && requested_fork_thread_id.is_some() {
        return Err(definitive_preparation_error(
            state,
            key,
            "invalid_session_source",
            "sdkSessionId resume cannot be combined with forkFromThreadId",
        )
        .await);
    }
    if (requested_session_id.is_some() || requested_fork_thread_id.is_some())
        && body.workspace_mode.is_worktree()
    {
        return Err(definitive_preparation_error(
            state,
            key,
            "invalid_workspace_mode",
            "workspaceMode=worktree cannot be combined with session resume or fork",
        )
        .await);
    }
    let provider_hint =
        match parse_sdk_session_provider_hint(body.sdk_session_provider_hint.as_deref()) {
            Ok(hint) => hint,
            Err(error) => {
                return Err(definitive_preparation_error(
                    state,
                    key,
                    "invalid_provider_hint",
                    error,
                )
                .await);
            }
        };
    if requested_session_id.is_some()
        && provider_hint
            .as_ref()
            .is_some_and(|provider| !is_resume_provider(provider))
    {
        return Err(definitive_preparation_error(
            state,
            key,
            "invalid_provider_hint",
            "sdkSessionId resume is only supported for Claude or Codex",
        )
        .await);
    }
    let recovered_session = match requested_session_id {
        Some(session_id) => match recover_local_provider_session(session_id, provider_hint.clone())
        {
            Ok(Some(recovered)) => Some(recovered),
            Ok(None) => {
                let provider = provider_hint
                    .as_ref()
                    .map(provider_hint_label)
                    .unwrap_or("Claude or Codex");
                return Err(definitive_preparation_error(
                    state,
                    key,
                    "sdk_session_not_found",
                    format!("No local {provider} session was found for session id '{session_id}'"),
                )
                .await);
            }
            Err(error) => {
                return Err(
                    definitive_preparation_error(state, key, "sdk_session_invalid", error).await,
                );
            }
        },
        None => None,
    };
    let fork_source = match requested_fork_thread_id {
        Some(source_thread_id) => {
            if !is_thread_key(source_thread_id) {
                return Err(definitive_preparation_error(
                    state,
                    key,
                    "fork_source_not_found",
                    "fork source thread not found",
                )
                .await);
            }
            let source = match state.threads.thread_store.get(source_thread_id).await {
                Ok(Some(source)) => source,
                Ok(None) => {
                    return Err(definitive_preparation_error(
                        state,
                        key,
                        "fork_source_not_found",
                        "fork source thread not found",
                    )
                    .await);
                }
                Err(error) => {
                    return Err(transient_preparation_error(
                        state,
                        key,
                        "fork_source_storage_error",
                        error.to_string(),
                    )
                    .await);
                }
            };
            let Some(provider_type) = provider_type_from_thread_value(&source) else {
                return Err(definitive_preparation_error(
                    state,
                    key,
                    "fork_source_missing_provider",
                    "fork source thread has no provider type",
                )
                .await);
            };
            if !is_resume_provider(&provider_type) {
                return Err(definitive_preparation_error(
                    state,
                    key,
                    "fork_source_provider_unsupported",
                    "forkFromThreadId is only supported for Claude or Codex provider sessions",
                )
                .await);
            }
            let Some(session_id) = fork_source_sdk_session_id(&source, &provider_type) else {
                return Err(definitive_preparation_error(
                    state,
                    key,
                    "fork_source_missing_session",
                    "fork source thread has no provider session id yet",
                )
                .await);
            };
            Some((
                source_thread_id.to_owned(),
                source,
                provider_type,
                session_id,
            ))
        }
        None => None,
    };

    let mut metadata = body.metadata.clone();
    for (metadata_key, requested) in [
        (MODEL_METADATA_KEY, body.model.as_deref()),
        (
            MODEL_REASONING_EFFORT_METADATA_KEY,
            body.model_reasoning_effort.as_deref(),
        ),
        (
            MODEL_SERVICE_TIER_METADATA_KEY,
            body.model_service_tier.as_deref(),
        ),
    ] {
        if let Some(value) = requested.map(str::trim).filter(|value| !value.is_empty()) {
            metadata.insert(metadata_key.to_owned(), Value::String(value.to_owned()));
        }
    }
    if let Some((source_thread_id, _, provider_type, session_id)) = fork_source.as_ref() {
        metadata.insert(
            FORK_FROM_THREAD_ID_METADATA_KEY.to_owned(),
            Value::String(source_thread_id.clone()),
        );
        metadata.insert(
            FORK_FROM_SDK_SESSION_ID_METADATA_KEY.to_owned(),
            Value::String(session_id.clone()),
        );
        metadata.insert(
            FORK_FROM_PROVIDER_TYPE_METADATA_KEY.to_owned(),
            serde_json::to_value(provider_type).unwrap_or(Value::Null),
        );
        metadata.insert(SDK_SESSION_FORK_METADATA_KEY.to_owned(), Value::Bool(true));
    }
    let fork_workspace_origin = fork_source.as_ref().map(|(source_thread_id, source, _, _)| {
        crate::workspace_mode::fork_inherited_workspace_origin(source_thread_id, source)
    });
    let mut options = ThreadEnsureOptions {
        no_workspace: body.no_workspace
            && recovered_session.is_none()
            && fork_source.is_none(),
        workspace_origin: fork_workspace_origin,
        label: body.label.clone(),
        workspace_dir: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.workspace_dir.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .and_then(|(_, source, _, _)| workspace_dir_from_value(source))
            })
            .or_else(|| body.workspace_dir.clone()),
        workspace_mode: body.workspace_mode,
        worktree_base_dir: Some(worktree_base_dir_for_data_dir(
            state.ops.prompt_attachments.data_dir(),
        )),
        agent_id: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.agent_id.clone())
            .or_else(|| {
                fork_source.as_ref().and_then(|(_, source, _, _)| {
                    source
                        .get("agent_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                })
            })
            .or_else(|| body.agent_id.clone()),
        metadata,
        provider_type: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.provider_type.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .map(|(_, _, provider_type, _)| provider_type.clone())
            }),
        sdk_session_id: body.sdk_session_id.clone(),
        thread_kind: None,
        origin_channel,
        origin_account_id,
        origin_from_id,
        is_group: None,
    };
    let binding_intent = if recovered_session.is_some() {
        AgentBindingIntent::RecoverExistingSession
    } else if fork_source.is_some() {
        AgentBindingIntent::Fork
    } else {
        AgentBindingIntent::Fresh
    };

    let worktree_lease = if options.workspace_mode.is_worktree() {
        let workspace = options.workspace_dir.as_deref().unwrap_or_default();
        let planned = match planned_thread_worktree_path(
            thread_id,
            workspace,
            options.worktree_base_dir.as_deref(),
        )
        .await
        {
            Ok(path) => path,
            Err(error) => {
                return Err(definitive_preparation_error(
                    state,
                    key,
                    "worktree_preparation_invalid",
                    error,
                )
                .await);
            }
        };
        match begin_create_resource(state, key, thread_id, CreateResourceKind::Worktree, planned)
            .await
        {
            Ok(lease) => Some(lease),
            Err(error) => {
                return Err(transient_preparation_error(
                    state,
                    key,
                    "worktree_reservation_failed",
                    error,
                )
                .await);
            }
        }
    } else {
        None
    };

    let (mut thread_data, _) = match prepare_thread_for_agent_reference(
        thread_id,
        state.ops.custom_agents.as_ref(),
        options.clone(),
        binding_intent,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(ThreadCreationError::AgentBinding(error)) => {
            return Err(definitive_preparation_error(
                state,
                key,
                "agent_binding_invalid",
                error.to_string(),
            )
            .await);
        }
        Err(ThreadCreationError::Other(error)) => {
            return Err(
                definitive_preparation_error(state, key, "thread_creation_invalid", error).await,
            );
        }
        Err(ThreadCreationError::Storage(error)) => {
            return Err(transient_preparation_error(
                state,
                key,
                "thread_creation_storage_error",
                error,
            )
            .await);
        }
    };
    if let Some(lease) = worktree_lease.as_ref() {
        if workspace_dir_from_value(&thread_data).as_deref()
            != Some(lease.path.to_string_lossy().as_ref())
        {
            return Err(transient_preparation_error(
                state,
                key,
                "worktree_path_mismatch",
                "materialized worktree path differs from its reservation",
            )
            .await);
        }
        if let Err(error) = mark_create_resource_materialized(state, key, lease).await {
            return Err(transient_preparation_error(
                state,
                key,
                "worktree_materialization_failed",
                error,
            )
            .await);
        }
    }
    if workspace_dir_from_value(&thread_data).is_none() {
        let workspace = match materialize_managed_workspace(state, key, thread_id).await {
            Ok(workspace) => workspace,
            Err(error) => {
                return Err(transient_preparation_error(
                    state,
                    key,
                    "managed_workspace_preparation_failed",
                    error,
                )
                .await);
            }
        };
        options.workspace_dir = Some(workspace);
        // The managed workspace is the implicit No-workspace directory; the
        // re-prepared record must carry implicit provenance, not the
        // explicit default.
        options.workspace_origin = Some("implicit".to_owned());
        thread_data = match prepare_thread_for_agent_reference(
            thread_id,
            state.ops.custom_agents.as_ref(),
            options,
            binding_intent,
        )
        .await
        {
            Ok((thread_data, _)) => thread_data,
            Err(error) => {
                return Err(transient_preparation_error(
                    state,
                    key,
                    "managed_workspace_record_failed",
                    error.to_string(),
                )
                .await);
            }
        };
    }
    if let Some(recovered) = recovered_session.as_ref()
        && !recovered.messages.is_empty()
    {
        let Some(transcript_path) = state
            .threads
            .history
            .transcript_store()
            .transcript_path(thread_id)
        else {
            return Err(transient_preparation_error(
                state,
                key,
                "imported_transcript_unavailable",
                "atomic session resume requires a file-backed transcript store",
            )
            .await);
        };
        let lease = match begin_create_resource(
            state,
            key,
            thread_id,
            CreateResourceKind::ImportedTranscript,
            transcript_path,
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                return Err(transient_preparation_error(
                    state,
                    key,
                    "imported_transcript_reservation_failed",
                    error,
                )
                .await);
            }
        };
        if let Err(error) = materialize_imported_thread_history(
            state,
            thread_id,
            &mut thread_data,
            &recovered.messages,
        )
        .await
        {
            return Err(transient_preparation_error(
                state,
                key,
                "imported_transcript_materialization_failed",
                error,
            )
            .await);
        }
        if let Err(error) = mark_create_resource_materialized(state, key, &lease).await {
            return Err(transient_preparation_error(
                state,
                key,
                "imported_transcript_materialization_failed",
                error,
            )
            .await);
        }
    }
    crate::sqlite_thread_store::strip_retired_record_fields(&mut thread_data);
    Ok(thread_data)
}

async fn run_create_only(
    state: Arc<AppState>,
    key: CreateIntentKey,
    fingerprint: String,
    claim: crate::garyx_db::CreateIntentRecord,
    body: CreateThreadBody,
) -> AdmissionOperationResult {
    if let Err(error) = cleanup_unadopted_create_resources(&state, &key, &claim.thread_id).await {
        return operation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "create_resource_cleanup_failed",
            error,
        );
    }
    let preparing_key = key.clone();
    let boot_id = state.server_boot_id().to_owned();
    let lease_expires_at = create_lease_expires_at();
    let db = Arc::clone(&state.ops.garyx_db);
    match db
        .run_blocking(move |db| {
            db.mark_create_intent_preparing(&preparing_key, &boot_id, &lease_expires_at)
        })
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return operation_error(
                StatusCode::CONFLICT,
                "create_intent_not_reservable",
                "create intent is no longer reservable",
            );
        }
        Err(error) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "create_intent_storage_error",
                error.to_string(),
            );
        }
    }
    let _create_lease = start_create_lease_heartbeat(&state, key.clone());
    let thread_data =
        match prepare_atomic_thread_record(&state, &key, &claim.thread_id, &body, None, None, None)
            .await
        {
            Ok(thread_data) => thread_data,
            Err(error) => return error,
        };
    let coordinator = state.threads.thread_store.run_coordinator();
    let mut creation_reservation = match coordinator
        .reserve_creation(state.threads.thread_store.as_ref(), &claim.thread_id)
        .await
    {
        Ok(reservation) => reservation,
        Err(error) => {
            return transient_preparation_error(
                &state,
                &key,
                "thread_creation_reservation_failed",
                error.to_string(),
            )
            .await;
        }
    };
    let Some(store) = state.threads.sqlite_thread_store.as_ref() else {
        return transient_preparation_error(
            &state,
            &key,
            "create_commit_failed",
            "idempotent create requires the SQLite thread store",
        )
        .await;
    };
    if let Err(error) = store
        .commit_create_intent_atomic(AtomicCreateCommit {
            create_key: key.clone(),
            create_request_fingerprint: fingerprint,
            target_thread_id: claim.thread_id.clone(),
            target_data: thread_data.clone(),
            merges: Vec::new(),
            dispatch: None,
        })
        .await
    {
        return transient_preparation_error(
            &state,
            &key,
            "create_commit_failed",
            error.to_string(),
        )
        .await;
    }
    creation_reservation.settle_committed(None);
    remove_adopted_resource_markers(&state, &key).await;
    maybe_block_after_create_commit(&key.create_intent_id).await;
    state
        .integration
        .bridge
        .set_thread_workspace_binding(&claim.thread_id, workspace_dir_from_value(&thread_data))
        .await;
    state.invalidate_gateway_sync_caches().await;
    AdmissionOperationResult::Ready
}

async fn fail_claim(state: &Arc<AppState>, key: &CreateIntentKey, code: &str, message: &str) {
    let db = Arc::clone(&state.ops.garyx_db);
    let db_key = key.clone();
    let code = code.to_owned();
    let message = message.to_owned();
    let failed = db
        .run_blocking(move |db| db.fail_create_intent_before_commit(&db_key, &code, &message))
        .await
        .unwrap_or(false);
    if !failed {
        return;
    }
    let db = Arc::clone(&state.ops.garyx_db);
    let query_key = key.clone();
    if let Ok(Some(claim)) = db
        .run_blocking(move |db| db.create_intent(&query_key))
        .await
    {
        let _ = cleanup_unadopted_create_resources(state, key, &claim.thread_id).await;
    }
}

fn create_dispatch_fingerprint(validated: &ValidatedCreateCommand, thread_id: &str) -> String {
    validated.dispatch_fingerprint.clone().unwrap_or_else(|| {
        stable_json_fingerprint(json!({
            "fingerprint_version": 1,
            "message": &validated.body.dispatch.message,
            "thread_id": thread_id,
            "channel": &validated.dispatch_channel,
            "account_id": &validated.body.dispatch.account_id,
            "from_id": &validated.body.dispatch.from_id,
            "attachments": &validated.body.dispatch.attachments,
            "images": &validated.body.dispatch.images,
            "files": &validated.body.dispatch.files,
            "metadata": &validated.body.dispatch.metadata,
        }))
    })
}

async fn resume_committed_create_dispatch(
    state: Arc<AppState>,
    validated: ValidatedCreateCommand,
    claim: crate::garyx_db::CreateIntentRecord,
) -> AdmissionOperationResult {
    if !state.provider_runtime_ready() {
        return operation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway_provider_runtime_starting",
            "Gateway provider runtime is still starting; retry shortly.",
        );
    }
    let snapshot_key = validated.key.clone();
    let db = Arc::clone(&state.ops.garyx_db);
    let snapshot = match db
        .run_blocking(move |db| db.create_intent_snapshot(&snapshot_key))
        .await
    {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "create_intent_missing",
                "committed create intent disappeared",
            );
        }
        Err(error) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "create_intent_storage_error",
                error.to_string(),
            );
        }
    };
    let Some(dispatch) = snapshot.dispatch else {
        return operation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "dispatch_admission_missing",
            "committed create intent has no dispatch admission",
        );
    };
    if dispatch.state != DispatchAdmissionState::Admitted {
        return AdmissionOperationResult::Ready;
    }
    let expected_fingerprint = create_dispatch_fingerprint(&validated, &claim.thread_id);
    if dispatch.fingerprint_version != 1 || dispatch.request_fingerprint != expected_fingerprint {
        return operation_error(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "clientIntentId was reused with a different request",
        );
    }
    let Some(run_id) = dispatch.requested_run_id.clone() else {
        return operation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "dispatch_admission_invalid",
            "chat-start admission has no requested run id",
        );
    };
    let Some(record_body) = snapshot.record_body else {
        return operation_error(
            StatusCode::CONFLICT,
            "created_thread_unavailable",
            "the claimed thread is no longer live",
        );
    };
    let thread_data = match serde_json::from_str::<Value>(&record_body) {
        Ok(value) => value,
        Err(error) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "created_thread_invalid",
                error.to_string(),
            );
        }
    };

    let config = state.config_snapshot();
    let effective_message = config
        .resolve_slash_command(&validated.body.dispatch.message)
        .and_then(|command| command.prompt)
        .unwrap_or_else(|| validated.body.dispatch.message.clone());
    let mut dispatch_metadata = validated.body.dispatch.metadata.clone();
    dispatch_metadata.insert(
        "client_intent_id".to_owned(),
        Value::String(validated.body.client_intent_id.clone()),
    );
    let mut staged_attachments = validated.body.dispatch.attachments.clone();
    staged_attachments.extend(stage_image_payloads_for_prompt(
        "garyx-gateway",
        &validated.body.dispatch.images,
    ));
    staged_attachments.extend(stage_file_payloads_for_prompt(
        "garyx-gateway",
        &validated.body.dispatch.files,
    ));
    let _attachment_claims = match state
        .ops
        .prompt_attachments
        .prepare_claims(
            (&validated.key.scope_identity, validated.key.scope_epoch),
            &mut staged_attachments,
        )
        .await
    {
        Ok(claims) => claims,
        Err(PromptAttachmentLifecycleError::Invalid(message)) => {
            return operation_error(StatusCode::BAD_REQUEST, "invalid_attachment", message);
        }
        Err(PromptAttachmentLifecycleError::Conflict(message)) => {
            return operation_error(StatusCode::CONFLICT, "attachment_conflict", message);
        }
        Err(PromptAttachmentLifecycleError::Storage(message)) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "attachment_storage_error",
                message,
            );
        }
    };
    if !staged_attachments.is_empty() {
        dispatch_metadata.insert(
            ATTACHMENTS_METADATA_KEY.to_owned(),
            attachments_to_metadata_value(&staged_attachments),
        );
    }
    dispatch_metadata.insert(
        "runtime_context".to_owned(),
        build_runtime_context_metadata(
            &claim.thread_id,
            Some(&thread_data),
            &dispatch_metadata,
            &validated.dispatch_channel,
            &validated.body.dispatch.account_id,
            &validated.body.dispatch.from_id,
            workspace_dir_from_value(&thread_data).as_deref(),
        ),
    );
    let run_metadata = crate::chat_application::build_provider_run_metadata(
        &config,
        dispatch_metadata,
        &validated.dispatch_channel,
        &validated.body.dispatch.account_id,
        &validated.body.dispatch.from_id,
        &run_id,
    );
    let requested_provider = thread_data
        .get("provider_type")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    let request = AgentRunRequest::new(
        &claim.thread_id,
        &effective_message,
        &run_id,
        &validated.dispatch_channel,
        &validated.body.dispatch.account_id,
        run_metadata,
    )
    .with_images(Some(validated.body.dispatch.images.clone()))
    .with_workspace_dir(workspace_dir_from_value(&thread_data))
    .with_requested_provider(requested_provider);
    state
        .integration
        .bridge
        .set_thread_workspace_binding(&claim.thread_id, workspace_dir_from_value(&thread_data))
        .await;
    let admitted =
        match AdmittedRun::thread_bound(state.threads.thread_store.clone(), request).await {
            Ok(admitted) => admitted,
            Err(error) => {
                return operation_error(
                    StatusCode::CONFLICT,
                    "created_run_admission_failed",
                    error.to_string(),
                );
            }
        };
    handoff_created_dispatch(
        state,
        validated,
        claim.thread_id,
        dispatch.key,
        run_id,
        effective_message,
        admitted,
    )
    .await
}

async fn run_create_and_dispatch(
    state: Arc<AppState>,
    validated: ValidatedCreateCommand,
    claim: crate::garyx_db::CreateIntentRecord,
) -> AdmissionOperationResult {
    if !state.provider_runtime_ready() {
        return operation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway_provider_runtime_starting",
            "Gateway provider runtime is still starting; retry shortly.",
        );
    }
    if let Err(error) =
        cleanup_unadopted_create_resources(&state, &validated.key, &claim.thread_id).await
    {
        return operation_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "create_resource_cleanup_failed",
            error,
        );
    }

    let key_for_preparing = validated.key.clone();
    let boot_id = state.server_boot_id().to_owned();
    let lease_expires_at = create_lease_expires_at();
    let db = Arc::clone(&state.ops.garyx_db);
    match db
        .run_blocking(move |db| {
            db.mark_create_intent_preparing(&key_for_preparing, &boot_id, &lease_expires_at)
        })
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return operation_error(
                StatusCode::CONFLICT,
                "create_intent_not_reservable",
                "create intent is no longer reservable",
            );
        }
        Err(error) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "create_intent_storage_error",
                error.to_string(),
            );
        }
    }
    let _create_lease = start_create_lease_heartbeat(&state, validated.key.clone());

    let thread_data = match prepare_atomic_thread_record(
        &state,
        &validated.key,
        &claim.thread_id,
        &validated.body.thread,
        validated.origin_channel.clone(),
        validated.origin_account_id.clone(),
        validated.origin_from_id.clone(),
    )
    .await
    {
        Ok(thread_data) => thread_data,
        Err(error) => return error,
    };

    let coordinator = state.threads.thread_store.run_coordinator();
    let creation_reservation = match coordinator
        .reserve_creation(state.threads.thread_store.as_ref(), &claim.thread_id)
        .await
    {
        Ok(reservation) => reservation,
        Err(error) => {
            return transient_preparation_error(
                &state,
                &validated.key,
                "thread_creation_reservation_failed",
                error.to_string(),
            )
            .await;
        }
    };

    let config = state.config_snapshot();
    let effective_message = config
        .resolve_slash_command(&validated.body.dispatch.message)
        .and_then(|command| command.prompt)
        .unwrap_or_else(|| validated.body.dispatch.message.clone());
    let mut dispatch_metadata = validated.body.dispatch.metadata.clone();
    dispatch_metadata.insert(
        "client_intent_id".to_owned(),
        Value::String(validated.body.client_intent_id.clone()),
    );
    let mut staged_attachments = validated.body.dispatch.attachments.clone();
    staged_attachments.extend(stage_image_payloads_for_prompt(
        "garyx-gateway",
        &validated.body.dispatch.images,
    ));
    staged_attachments.extend(stage_file_payloads_for_prompt(
        "garyx-gateway",
        &validated.body.dispatch.files,
    ));
    let attachment_claims = match state
        .ops
        .prompt_attachments
        .prepare_claims(
            (&validated.key.scope_identity, validated.key.scope_epoch),
            &mut staged_attachments,
        )
        .await
    {
        Ok(claims) => claims,
        Err(PromptAttachmentLifecycleError::Invalid(message)) => {
            return operation_error(StatusCode::BAD_REQUEST, "invalid_attachment", message);
        }
        Err(PromptAttachmentLifecycleError::Conflict(message)) => {
            return operation_error(StatusCode::CONFLICT, "attachment_conflict", message);
        }
        Err(PromptAttachmentLifecycleError::Storage(message)) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "attachment_storage_error",
                message,
            );
        }
    };
    if !staged_attachments.is_empty() {
        dispatch_metadata.insert(
            ATTACHMENTS_METADATA_KEY.to_owned(),
            attachments_to_metadata_value(&staged_attachments),
        );
    }
    dispatch_metadata.insert(
        "runtime_context".to_owned(),
        build_runtime_context_metadata(
            &claim.thread_id,
            Some(&thread_data),
            &dispatch_metadata,
            &validated.dispatch_channel,
            &validated.body.dispatch.account_id,
            &validated.body.dispatch.from_id,
            workspace_dir_from_value(&thread_data).as_deref(),
        ),
    );
    let run_id = Uuid::new_v4().to_string();
    let run_metadata = crate::chat_application::build_provider_run_metadata(
        &config,
        dispatch_metadata,
        &validated.dispatch_channel,
        &validated.body.dispatch.account_id,
        &validated.body.dispatch.from_id,
        &run_id,
    );
    let requested_provider = thread_data
        .get("provider_type")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    let request = AgentRunRequest::new(
        &claim.thread_id,
        &effective_message,
        &run_id,
        &validated.dispatch_channel,
        &validated.body.dispatch.account_id,
        run_metadata,
    )
    .with_images(Some(validated.body.dispatch.images.clone()))
    .with_workspace_dir(workspace_dir_from_value(&thread_data))
    .with_requested_provider(requested_provider);

    let dispatch_key = DispatchAdmissionKey {
        scope_identity: validated.key.scope_identity.clone(),
        scope_epoch: validated.key.scope_epoch,
        thread_id: claim.thread_id.clone(),
        kind: DispatchAdmissionKind::ChatStart,
        client_intent_id: validated.body.client_intent_id.clone(),
    };
    let dispatch_fingerprint = create_dispatch_fingerprint(&validated, &claim.thread_id);
    let command = AtomicCreateCommit {
        create_key: validated.key.clone(),
        create_request_fingerprint: validated.fingerprint.clone(),
        target_thread_id: claim.thread_id.clone(),
        target_data: thread_data.clone(),
        merges: Vec::new(),
        dispatch: Some(AtomicCreateDispatchLedger {
            key: dispatch_key.clone(),
            request_fingerprint: dispatch_fingerprint,
            requested_run_id: run_id.clone(),
            effective_run_id: run_id.clone(),
            pending_input_id: None,
            outcome: DispatchOutcome::Started,
            attachment_claims,
        }),
    };
    let requested_binding = match validated.resolved_binding.clone() {
        Some(binding) => Some(RequestedCreateBinding::Resolved(binding)),
        None => match validated.body.binding.as_ref() {
            Some(binding) => {
                let Some((channel, account_id)) = binding.bot_id.trim().split_once(':') else {
                    let message = "binding.botId must be `channel:account_id`";
                    fail_claim(&state, &validated.key, "invalid_binding_bot", message).await;
                    return operation_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_binding_bot",
                        message,
                    );
                };
                match crate::routes::resolve_main_endpoint_by_bot_fresh(&state, channel, account_id)
                    .await
                {
                    Ok(Some(_)) => Some(RequestedCreateBinding::PublicBot {
                        bot_id: binding.bot_id.clone(),
                        channel: channel.to_owned(),
                        account_id: account_id.to_owned(),
                    }),
                    Ok(None) => {
                        let message =
                            format!("bot '{}' has no resolved main endpoint", binding.bot_id);
                        fail_claim(&state, &validated.key, "binding_bot_not_found", &message).await;
                        return operation_error(
                            StatusCode::NOT_FOUND,
                            "binding_bot_not_found",
                            message,
                        );
                    }
                    Err(error) => {
                        return operation_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "binding_resolution_failed",
                            error.to_string(),
                        );
                    }
                }
            }
            None => None,
        },
    };
    let commit_result = match requested_binding {
        Some(RequestedCreateBinding::Resolved(binding)) => state
            .ops
            .endpoint_binding_mutator
            .commit_created_thread_with_binding(command, binding)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Some(RequestedCreateBinding::PublicBot {
            bot_id,
            channel,
            account_id,
        }) => {
            let resolver_state = Arc::clone(&state);
            state
                .ops
                .endpoint_binding_mutator
                .commit_created_thread_with_binding_resolver(command, move || async move {
                    match crate::routes::resolve_main_endpoint_by_bot_fresh(
                        &resolver_state,
                        &channel,
                        &account_id,
                    )
                    .await
                    {
                        Ok(Some(endpoint)) => {
                            let mut binding = endpoint.to_binding();
                            binding.last_inbound_at = Some(Utc::now().to_rfc3339());
                            Ok(binding)
                        }
                        Ok(None) => Err(EndpointBindingMutationError::Incompatible(format!(
                            "bot '{bot_id}' has no resolved main endpoint"
                        ))),
                        Err(error) => {
                            Err(EndpointBindingMutationError::Projection(error.to_string()))
                        }
                    }
                })
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        None => match state.threads.sqlite_thread_store.as_ref() {
            Some(store) => store
                .commit_create_intent_atomic(command)
                .await
                .map_err(|error| error.to_string()),
            None => Err("atomic create requires the SQLite thread store".to_owned()),
        },
    };
    if let Err(error) = commit_result {
        return transient_preparation_error(&state, &validated.key, "create_commit_failed", error)
            .await;
    }

    remove_adopted_resource_markers(&state, &validated.key).await;
    maybe_block_after_create_commit(&validated.key.create_intent_id).await;
    if stop_after_create_commit(&validated.key.create_intent_id) {
        return operation_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "injected_stop_after_create_commit",
            "injected process stop after commit and before provider gate",
        );
    }

    state
        .integration
        .bridge
        .set_thread_workspace_binding(&claim.thread_id, workspace_dir_from_value(&thread_data))
        .await;
    state.invalidate_gateway_sync_caches().await;
    let admitted = match AdmittedRun::thread_bound_from_creation(
        state.threads.thread_store.clone(),
        request,
        creation_reservation,
    )
    .await
    {
        Ok(admitted) => admitted,
        Err(error) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "created_run_admission_failed",
                error.to_string(),
            );
        }
    };
    handoff_created_dispatch(
        state,
        validated,
        claim.thread_id,
        dispatch_key,
        run_id,
        effective_message,
        admitted,
    )
    .await
}

async fn handoff_created_dispatch(
    state: Arc<AppState>,
    mut validated: ValidatedCreateCommand,
    thread_id: String,
    dispatch_key: DispatchAdmissionKey,
    run_id: String,
    effective_message: String,
    admitted: AdmittedRun,
) -> AdmissionOperationResult {
    let plan = match state
        .integration
        .bridge
        .prepare_durable_dispatch(admitted, format!("queued_input:{}", Uuid::new_v4()))
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_plan_failed",
                error,
            );
        }
    };
    if plan.outcome() != AgentDispatchOutcome::Started || plan.requested_run_id() != run_id {
        return operation_error(
            StatusCode::CONFLICT,
            "dispatch_plan_changed",
            "new thread acquired an unexpected active run before handoff",
        );
    }
    let mut callbacks = Vec::new();
    let mut stream_tasks = Vec::new();
    if let Some(builder) = validated.callback_builder.take() {
        let mut attachment = builder(&run_id, &thread_id);
        if let Some(callback) = attachment.callback.take() {
            callbacks.push(callback);
        }
        if let Some(task) = attachment.task.take() {
            stream_tasks.push(task);
        }
    }
    let bound_stream = match build_bound_response_callback(&state, &thread_id, &run_id, None).await
    {
        Ok(stream) => stream,
        Err(error) => {
            for task in stream_tasks {
                task.abort();
            }
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "response_stream_attach_failed",
                error.to_string(),
            );
        }
    };
    if let Some(callback) = bound_stream.callback() {
        callbacks.push(callback);
    }
    let callback = compose_stream_callbacks(callbacks);
    match state
        .ops
        .conversation_admission
        .start_handoff(dispatch_key.clone())
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
            return operation_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "dispatch_admission_storage_error",
                error,
            );
        }
    }
    state.sync_external_user_skills_before_run("atomic_create_dispatch", &thread_id);
    match state
        .integration
        .bridge
        .execute_durable_dispatch(plan, callback)
        .await
    {
        Ok(AgentDispatchOutcome::Started) => {
            match state
                .ops
                .conversation_admission
                .settle(
                    dispatch_key,
                    DispatchAdmissionState::Accepted,
                    Some(DispatchOutcome::Started),
                    Some(run_id.clone()),
                    None,
                    200,
                    None,
                    None,
                )
                .await
            {
                Ok(_) => {
                    bound_stream.detach();
                    crate::runtime_diagnostics::record_message_ledger_event(
                        &state,
                        MessageLifecycleStatus::RunStarted,
                        crate::runtime_diagnostics::RuntimeDiagnosticContext {
                            thread_id: Some(thread_id),
                            run_id: Some(run_id),
                            channel: Some(validated.dispatch_channel),
                            account_id: Some(validated.body.dispatch.account_id),
                            from_id: Some(validated.body.dispatch.from_id),
                            text_excerpt: Some(effective_message.chars().take(200).collect()),
                            metadata: Some(json!({"source": "atomic_create_dispatch"})),
                            ..Default::default()
                        },
                    )
                    .await;
                    AdmissionOperationResult::Ready
                }
                Err(error) => operation_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "dispatch_settlement_failed",
                    error,
                ),
            }
        }
        Ok(AgentDispatchOutcome::QueuedToActiveRun { .. }) => {
            bound_stream.abort();
            for task in stream_tasks {
                task.abort();
            }
            operation_error(
                StatusCode::CONFLICT,
                "dispatch_plan_changed",
                "new thread dispatch unexpectedly queued to another run",
            )
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
                    dispatch_key,
                    DispatchAdmissionState::Ambiguous,
                    Some(DispatchOutcome::Started),
                    Some(run_id),
                    None,
                    409,
                    Some("dispatch_ambiguous".to_owned()),
                    Some(error),
                )
                .await
            {
                Ok(_) => AdmissionOperationResult::Ready,
                Err(error) => operation_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "dispatch_settlement_failed",
                    error,
                ),
            }
        }
    }
}

fn dispatch_json(record: &DispatchAdmissionRecord) -> Value {
    json!({
        "deliveryState": record.state.as_str(),
        "dispatchOutcome": record.outcome.map(DispatchOutcome::as_str),
        "runId": record.requested_run_id,
        "effectiveRunId": record.effective_run_id,
        "pendingInputId": record.pending_input_id,
        "error": record.result_error_code,
        "message": record.result_error_message,
    })
}

fn snapshot_json(snapshot: CreateIntentSnapshot, replay: bool) -> Value {
    let lifecycle = match snapshot.lifecycle {
        Some(ThreadTerminalState::Archived) => "archived",
        Some(ThreadTerminalState::Deleted) => "deleted",
        None if snapshot.claim.state == CreateIntentState::Committed => "live",
        None => "not_committed",
    };
    let thread = snapshot
        .record_body
        .as_deref()
        .and_then(|body| serde_json::from_str::<Value>(body).ok())
        .map(|body| thread_summary(&snapshot.claim.thread_id, &body));
    json!({
        "createIntentId": snapshot.claim.key.create_intent_id,
        "threadId": snapshot.claim.thread_id,
        "state": snapshot.claim.state.as_str(),
        "threadLifecycle": lifecycle,
        "thread": thread,
        "dispatch": snapshot.dispatch.as_ref().map(dispatch_json),
        "idempotencyReplay": replay,
    })
}

async fn create_response_from_snapshot(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    replay: bool,
) -> axum::response::Response {
    let db = Arc::clone(&state.ops.garyx_db);
    let key = key.clone();
    match db
        .run_blocking(move |db| db.create_intent_snapshot(&key))
        .await
    {
        Ok(Some(snapshot)) => {
            let status = match snapshot.dispatch.as_ref().map(|row| row.state) {
                Some(
                    DispatchAdmissionState::Ambiguous | DispatchAdmissionState::HandoffStarted,
                ) => StatusCode::CONFLICT,
                _ if replay => StatusCode::OK,
                _ => StatusCode::CREATED,
            };
            (status, Json(snapshot_json(snapshot, replay))).into_response()
        }
        Ok(None) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "committed create intent disappeared"})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "create_intent_storage_error",
                "message": error.to_string(),
            })),
        )
            .into_response(),
    }
}

pub async fn get_by_create_intent(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CreateIntentQuery>,
) -> impl IntoResponse {
    let scope = IdempotencyScope {
        identity: query.scope_identity,
        epoch: query.scope_epoch,
    };
    let (scope_identity, scope_epoch) = match validate_explicit_idempotency_scope(&scope) {
        Ok(scope) => scope,
        Err((status, payload)) => return (status, Json(payload)).into_response(),
    };
    let create_intent_id = match validate_intent_id("createIntentId", &query.create_intent_id, true)
    {
        Ok(intent) => intent,
        Err((status, payload)) => return (status, Json(payload)).into_response(),
    };
    let key = CreateIntentKey {
        scope_identity,
        scope_epoch,
        create_intent_id,
    };
    let db = Arc::clone(&state.ops.garyx_db);
    match db
        .run_blocking(move |db| db.create_intent_snapshot(&key))
        .await
    {
        Ok(Some(snapshot)) => (StatusCode::OK, Json(snapshot_json(snapshot, true))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "create intent not found"})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "create_intent_storage_error",
                "message": error.to_string(),
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use garyx_bridge::MultiProviderBridge;
    use garyx_bridge::provider_trait::{BridgeError, ProviderRuntime, StreamCallback};
    use garyx_models::config::{ApiAccount, GaryxConfig};
    use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType};
    use tempfile::tempdir;
    use tower::ServiceExt;

    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ProviderRuntime for CountingProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::ClaudeCode
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            _on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ProviderRunResult {
                run_id: "atomic-create-provider".to_owned(),
                thread_id: options.thread_id.clone(),
                response: String::new(),
                session_messages: Vec::new(),
                sdk_session_id: None,
                actual_model: None,
                thread_title: None,
                success: true,
                error: None,
                input_tokens: 0,
                output_tokens: 0,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{thread_id}"))
        }
    }

    async fn state(calls: Arc<AtomicUsize>) -> Arc<AppState> {
        let mut config = GaryxConfig::default();
        config.channels.api.accounts.insert(
            "main".to_owned(),
            ApiAccount {
                enabled: true,
                name: None,
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                workspace_mode: None,
            },
        );
        let bridge = Arc::new(MultiProviderBridge::new());
        bridge
            .register_provider("atomic-provider", Arc::new(CountingProvider { calls }))
            .await;
        bridge.set_route("api", "main", "atomic-provider").await;
        bridge.set_default_provider_key("atomic-provider").await;
        crate::server::AppStateBuilder::new(config)
            .with_bridge(bridge)
            .build()
    }

    fn router(state: Arc<AppState>) -> Router {
        Router::new()
            .route(
                "/api/threads",
                axum::routing::post(crate::routes::create_thread),
            )
            .route(
                "/api/threads/create-and-dispatch",
                axum::routing::post(create_and_dispatch),
            )
            .route(
                "/api/threads/by-create-intent",
                axum::routing::get(get_by_create_intent),
            )
            .with_state(state)
    }

    async fn send(router: Router, payload: &Value) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/threads/create-and-dispatch")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn atomic_create_replay_claims_one_thread_and_dispatches_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = state(calls.clone()).await;
        let workspace = tempdir().unwrap();
        let router = router(state.clone());
        let payload = json!({
            "idempotencyScope": {"identity": "atomic-test", "epoch": 1},
            "createIntentId": "create-once",
            "clientIntentId": "dispatch-once",
            "thread": {
                "workspaceDir": workspace.path().to_string_lossy(),
                "agentId": "claude"
            },
            "dispatch": {
                "message": "create and send",
                "accountId": "main",
                "fromId": "api-user"
            }
        });

        let (first_status, first) = send(router.clone(), &payload).await;
        let (second_status, second) = send(router.clone(), &payload).await;
        assert_eq!(first_status, StatusCode::CREATED, "{first}");
        assert_eq!(second_status, StatusCode::OK, "{second}");
        assert_eq!(first["threadId"], second["threadId"]);
        assert_eq!(first["dispatch"]["runId"], second["dispatch"]["runId"]);
        assert_eq!(first["idempotencyReplay"], false);
        assert_eq!(second["idempotencyReplay"], true);
        assert_eq!(
            state.thread_record_count().await.unwrap(),
            1,
            "one create intent must publish exactly one canonical thread"
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/threads/by-create-intent?scopeIdentity=atomic-test&scopeEpoch=1&createIntentId=create-once")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let query: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(query["threadId"], first["threadId"]);
        assert_eq!(query["state"], "committed");
        assert_eq!(query["threadLifecycle"], "live");
    }

    #[tokio::test]
    async fn create_intent_query_recovers_reserved_claim_without_enumeration() {
        let state = state(Arc::new(AtomicUsize::new(0))).await;
        let router = router(state.clone());
        let key = CreateIntentKey {
            scope_identity: "query-recovery-test".to_owned(),
            scope_epoch: 7,
            create_intent_id: "reserved-before-response".to_owned(),
        };
        let query_key = key.clone();
        state
            .ops
            .garyx_db
            .run_blocking(move |db| {
                db.reserve_create_intent(NewCreateIntent {
                    key: &query_key,
                    thread_id: "thread::query-recovery-fixed",
                    request_fingerprint: "query-recovery-fingerprint",
                    command_kind: CreateCommandKind::CreateOnly,
                    dispatch_client_intent_id: None,
                })
            })
            .await
            .unwrap();

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/threads/by-create-intent?scopeIdentity=query-recovery-test&scopeEpoch=7&createIntentId=reserved-before-response")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["threadId"], "thread::query-recovery-fixed");
        assert_eq!(body["state"], "reserved");
        assert_eq!(body["threadLifecycle"], "not_committed");
        assert!(body["thread"].is_null());

        let missing = router
            .oneshot(
                Request::builder()
                    .uri("/api/threads/by-create-intent?scopeIdentity=query-recovery-test&scopeEpoch=7&createIntentId=missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn requester_killed_after_commit_replay_cannot_create_a_ghost_duplicate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = state(calls.clone()).await;
        let workspace = tempdir().unwrap();
        let router = router(state.clone());
        let payload = json!({
            "idempotencyScope": {"identity": "kill-test", "epoch": 1},
            "createIntentId": "commit-before-response",
            "clientIntentId": "commit-before-response-dispatch",
            "thread": {
                "workspaceDir": workspace.path().to_string_lossy(),
                "agentId": "claude"
            },
            "dispatch": {"message": "survive requester death"}
        });
        let committed = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *CREATE_COMMIT_BARRIER
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(CreateCommitBarrier {
            create_intent_id: "commit-before-response".to_owned(),
            committed: committed.clone(),
            release: release.clone(),
        });

        let first_router = router.clone();
        let first_payload = payload.clone();
        let requester = tokio::spawn(async move { send(first_router, &first_payload).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), committed.notified())
            .await
            .expect("atomic commit barrier");
        requester.abort();
        let _ = requester.await;
        assert_eq!(state.thread_record_count().await.unwrap(), 1);

        let query_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/threads/by-create-intent?scopeIdentity=kill-test&scopeEpoch=1&createIntentId=commit-before-response")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(query_response.status(), StatusCode::OK);
        let query_bytes = axum::body::to_bytes(query_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let before_release: Value = serde_json::from_slice(&query_bytes).unwrap();
        assert_eq!(before_release["state"], "committed");
        assert_eq!(before_release["dispatch"]["deliveryState"], "admitted");
        let committed_thread_id = before_release["threadId"].clone();

        release.notify_waiters();
        let (replay_status, replay) = send(router, &payload).await;
        assert_eq!(replay_status, StatusCode::OK, "{replay}");
        assert_eq!(replay["threadId"], committed_thread_id);
        assert_eq!(state.thread_record_count().await.unwrap(), 1);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        *CREATE_COMMIT_BARRIER
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    }

    #[tokio::test]
    async fn committed_admission_replay_resumes_before_provider_gate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = state(calls.clone()).await;
        let workspace = tempdir().unwrap();
        let router = router(state.clone());
        let payload = json!({
            "idempotencyScope": {"identity": "resume-test", "epoch": 1},
            "createIntentId": "resume-after-commit",
            "clientIntentId": "resume-after-commit-dispatch",
            "thread": {
                "workspaceDir": workspace.path().to_string_lossy(),
                "agentId": "claude"
            },
            "dispatch": {"message": "resume the safe admitted row"}
        });
        *STOP_AFTER_CREATE_COMMIT
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some("resume-after-commit".to_owned());

        let (first_status, first) = send(router.clone(), &payload).await;
        assert_eq!(first_status, StatusCode::SERVICE_UNAVAILABLE, "{first}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let query_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/threads/by-create-intent?scopeIdentity=resume-test&scopeEpoch=1&createIntentId=resume-after-commit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(query_response.status(), StatusCode::OK);
        let query_bytes = axum::body::to_bytes(query_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let before_replay: Value = serde_json::from_slice(&query_bytes).unwrap();
        assert_eq!(before_replay["state"], "committed");
        assert_eq!(before_replay["dispatch"]["deliveryState"], "admitted");
        let thread_id = before_replay["threadId"].clone();
        let run_id = before_replay["dispatch"]["runId"].clone();

        *STOP_AFTER_CREATE_COMMIT
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        let (replay_status, replay) = send(router, &payload).await;
        assert_eq!(replay_status, StatusCode::OK, "{replay}");
        assert_eq!(replay["threadId"], thread_id);
        assert_eq!(replay["dispatch"]["runId"], run_id);
        assert_eq!(replay["dispatch"]["deliveryState"], "accepted");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.thread_record_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn threadless_precommit_claim_replays_across_server_timestamp_change() {
        let calls = Arc::new(AtomicUsize::new(0));
        let state = state(calls.clone()).await;
        let workspace = tempdir().unwrap();
        let client_intent_id = "threadless-precommit-recovery";
        let scope_identity = "threadless-precommit-test";
        let request = ChatRequest {
            message: "recover the reserved implicit create".to_owned(),
            attachments: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            thread_id: None,
            client_intent_id: Some(client_intent_id.to_owned()),
            idempotency_scope: Some(IdempotencyScope {
                identity: scope_identity.to_owned(),
                epoch: 1,
            }),
            bot: None,
            from_id: "api-user".to_owned(),
            account_id: "main".to_owned(),
            wait_for_response: false,
            workspace_path: Some(workspace.path().to_string_lossy().into_owned()),
            provider_type: None,
            metadata: HashMap::new(),
        };
        let dispatch_fingerprint =
            crate::conversation_admission::chat_request_fingerprint(&request);
        let create_intent_id =
            format!("implicit:{:x}", Sha256::digest(client_intent_id.as_bytes()));
        let create_key = CreateIntentKey {
            scope_identity: scope_identity.to_owned(),
            scope_epoch: 1,
            create_intent_id,
        };
        let reserved_thread_id = new_thread_key();
        let reserve_key = create_key.clone();
        let reserve_fingerprint = implicit_create_request_fingerprint(&dispatch_fingerprint);
        let reserve_thread_id = reserved_thread_id.clone();
        state
            .ops
            .garyx_db
            .run_blocking(move |db| {
                db.reserve_create_intent(NewCreateIntent {
                    key: &reserve_key,
                    thread_id: &reserve_thread_id,
                    request_fingerprint: &reserve_fingerprint,
                    command_kind: CreateCommandKind::CreateAndDispatch,
                    dispatch_client_intent_id: Some(client_intent_id),
                })
            })
            .await
            .unwrap();

        let response = create_implicit_and_dispatch(
            &state,
            scope_identity.to_owned(),
            1,
            client_intent_id.to_owned(),
            request,
            ImplicitThreadCreatePlan {
                channel: "api".to_owned(),
                account_id: "main".to_owned(),
                from_id: "api-user".to_owned(),
                binding: ChannelBinding {
                    channel: "api".to_owned(),
                    account_id: "main".to_owned(),
                    binding_key: "api-user".to_owned(),
                    chat_id: "api-user".to_owned(),
                    delivery_target_type: garyx_models::routing::DELIVERY_TARGET_TYPE_CHAT_ID
                        .to_owned(),
                    delivery_target_id: "api-user".to_owned(),
                    display_label: "api/main/api-user".to_owned(),
                    // This value represents a later server preparation and
                    // must not participate in the existing create claim.
                    last_inbound_at: Some("2099-01-01T00:00:00Z".to_owned()),
                    last_delivery_at: None,
                },
                label: "api/main/api-user".to_owned(),
                workspace_dir: Some(workspace.path().to_string_lossy().into_owned()),
                workspace_mode: garyx_router::WorkspaceMode::Local,
                agent_id: Some("claude".to_owned()),
                dispatch_metadata: HashMap::new(),
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(response.thread_id, reserved_thread_id);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn legacy_create_route_claims_explicit_create_intent_once() {
        let state = state(Arc::new(AtomicUsize::new(0))).await;
        let workspace = tempdir().unwrap();
        let router = router(state.clone());
        let payload = json!({
            "idempotencyScope": {"identity": "create-only-test", "epoch": 1},
            "createIntentId": "empty-thread-once",
            "workspaceDir": workspace.path().to_string_lossy(),
            "agentId": "claude"
        });

        let send_create = |router: Router, payload: Value| async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/threads")
                        .header("content-type", "application/json")
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        };

        let (first_status, first) = send_create(router.clone(), payload.clone()).await;
        let (second_status, second) = send_create(router.clone(), payload).await;
        assert_eq!(first_status, StatusCode::CREATED, "{first}");
        assert_eq!(second_status, StatusCode::OK, "{second}");
        assert_eq!(first["threadId"], second["threadId"]);
        assert_eq!(first["idempotencyReplay"], false);
        assert_eq!(second["idempotencyReplay"], true);
        assert!(first["dispatch"].is_null());
        assert_eq!(state.thread_record_count().await.unwrap(), 1);

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/threads/by-create-intent?scopeIdentity=create-only-test&scopeEpoch=1&createIntentId=empty-thread-once")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let query: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(query["threadId"], first["threadId"]);
        assert_eq!(query["state"], "committed");
        assert!(query["dispatch"].is_null());

        let thread_id = first["threadId"].as_str().unwrap().to_owned();
        let db = Arc::clone(&state.ops.garyx_db);
        let archived_thread_id = thread_id.clone();
        db.run_blocking(move |db| db.archive_thread_record(&archived_thread_id))
            .await
            .unwrap();
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/threads/by-create-intent?scopeIdentity=create-only-test&scopeEpoch=1&createIntentId=empty-thread-once")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let archived: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(archived["threadId"], thread_id);
        assert_eq!(archived["state"], "committed");
        assert_eq!(archived["threadLifecycle"], "archived");
        assert!(archived["thread"].is_null());
    }

    #[tokio::test]
    async fn create_only_adopts_deterministic_managed_workspace_resource() {
        let state = state(Arc::new(AtomicUsize::new(0))).await;
        let router = router(state.clone());
        let payload = json!({
            "idempotencyScope": {"identity": "managed-workspace-test", "epoch": 1},
            "createIntentId": "managed-workspace-once",
            "agentId": "claude"
        });

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/threads")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        let thread_id = response["threadId"].as_str().unwrap();
        let expected_workspace = crate::workspace_mode::implicit_thread_workspace_dir_for_data_dir(
            state.ops.prompt_attachments.data_dir(),
            thread_id,
        );
        assert!(expected_workspace.is_dir());
        assert_eq!(
            response["thread"]["workspace_dir"].as_str(),
            Some(expected_workspace.to_string_lossy().as_ref())
        );

        let key = CreateIntentKey {
            scope_identity: "managed-workspace-test".to_owned(),
            scope_epoch: 1,
            create_intent_id: "managed-workspace-once".to_owned(),
        };
        let resources = state
            .ops
            .garyx_db
            .run_blocking(move |db| db.create_resources_for_intent(&key))
            .await
            .unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].kind, CreateResourceKind::ManagedWorkspace);
        assert_eq!(
            resources[0].state,
            crate::garyx_db::CreateResourceState::Adopted
        );
        assert_eq!(
            PathBuf::from(&resources[0].resource_path),
            expected_workspace
        );
        let marker = PathBuf::from(format!(
            "{}.garyx-create-owner",
            expected_workspace.to_string_lossy()
        ));
        assert!(!marker.exists(), "adopted resource marker must be removed");

        tokio::fs::remove_dir_all(&expected_workspace)
            .await
            .unwrap();
    }
}
