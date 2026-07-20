use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path};
use std::sync::{Arc, Mutex, Weak};

use axum::http::StatusCode;
use garyx_models::strip_server_owned_agent_metadata;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::application::chat::contracts::{ChatRequest, IdempotencyScope, StreamInputRequest};
use crate::garyx_db::{
    CreateIntentKey, DispatchAdmissionKey, DispatchAdmissionRecord, DispatchAdmissionState,
    DispatchOutcome, GaryxDbService, NewDispatchAdmission, PromptAttachmentClaim,
};

const LEGACY_SCOPE_IDENTITY: &str = "__legacy_api__";
const MAX_OPERATION_CELLS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DispatchCorrelation {
    pub scope_identity: String,
    pub scope_epoch: i64,
    pub client_intent_id: String,
}

#[derive(Debug, Clone)]
pub(crate) enum AdmissionOperationResult {
    Ready,
    Failed { status: StatusCode, payload: Value },
}

struct AdmissionOperationCell {
    fingerprint: String,
    result_tx: watch::Sender<Option<Arc<AdmissionOperationResult>>>,
}

impl AdmissionOperationCell {
    fn new(fingerprint: String) -> Arc<Self> {
        let (result_tx, _result_rx) = watch::channel(None);
        Arc::new(Self {
            fingerprint,
            result_tx,
        })
    }

    fn publish(&self, result: AdmissionOperationResult) {
        if self.result_tx.borrow().is_none() {
            self.result_tx.send_replace(Some(Arc::new(result)));
        }
    }

    async fn wait(&self) -> Arc<AdmissionOperationResult> {
        let mut receiver = self.result_tx.subscribe();
        loop {
            if let Some(result) = receiver.borrow().as_ref().cloned() {
                return result;
            }
            if receiver.changed().await.is_err() {
                return Arc::new(owner_lost_result());
            }
        }
    }
}

fn owner_lost_result() -> AdmissionOperationResult {
    AdmissionOperationResult::Failed {
        status: StatusCode::SERVICE_UNAVAILABLE,
        payload: json!({
            "error": "dispatch_admission_owner_lost",
            "message": "dispatch admission owner ended before publishing a result"
        }),
    }
}

#[derive(Default)]
struct AdmissionRegistryInner {
    cells: Mutex<HashMap<AdmissionOperationKey, Arc<AdmissionOperationCell>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AdmissionOperationKey {
    Dispatch(DispatchAdmissionKey),
    Create(CreateIntentKey),
}

#[derive(Clone)]
pub(crate) struct ConversationAdmissionService {
    db: Arc<GaryxDbService>,
    registry: Arc<AdmissionRegistryInner>,
}

pub(crate) enum AdmissionRegistration {
    Owner(AdmissionOwnerGuard),
    Join(AdmissionJoinHandle),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AdmissionRegistrationError {
    #[error("clientIntentId was reused with a different request")]
    FingerprintConflict,
    #[error("too many dispatch admissions are currently in progress")]
    Overloaded,
}

pub(crate) struct AdmissionJoinHandle {
    cell: Arc<AdmissionOperationCell>,
}

impl AdmissionJoinHandle {
    pub async fn wait(&self) -> Arc<AdmissionOperationResult> {
        self.cell.wait().await
    }
}

pub(crate) struct AdmissionOwnerGuard {
    registry: Weak<AdmissionRegistryInner>,
    key: AdmissionOperationKey,
    cell: Arc<AdmissionOperationCell>,
    published: bool,
}

impl AdmissionOwnerGuard {
    pub fn join_handle(&self) -> AdmissionJoinHandle {
        AdmissionJoinHandle {
            cell: Arc::clone(&self.cell),
        }
    }

    pub fn publish(mut self, result: AdmissionOperationResult) {
        self.cell.publish(result);
        self.remove_if_current();
        self.published = true;
    }

    fn remove_if_current(&self) {
        let Some(registry) = self.registry.upgrade() else {
            return;
        };
        let mut cells = registry
            .cells
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if cells
            .get(&self.key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.cell))
        {
            cells.remove(&self.key);
        }
    }
}

impl Drop for AdmissionOwnerGuard {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        self.cell.publish(owner_lost_result());
        self.remove_if_current();
    }
}

impl ConversationAdmissionService {
    pub(crate) fn new(db: Arc<GaryxDbService>) -> Self {
        Self {
            db,
            registry: Arc::new(AdmissionRegistryInner::default()),
        }
    }

    pub(crate) fn register(
        &self,
        key: DispatchAdmissionKey,
        fingerprint: &str,
    ) -> Result<AdmissionRegistration, AdmissionRegistrationError> {
        self.register_operation(AdmissionOperationKey::Dispatch(key), fingerprint)
    }

    pub(crate) fn register_create(
        &self,
        key: CreateIntentKey,
        fingerprint: &str,
    ) -> Result<AdmissionRegistration, AdmissionRegistrationError> {
        self.register_operation(AdmissionOperationKey::Create(key), fingerprint)
    }

    fn register_operation(
        &self,
        key: AdmissionOperationKey,
        fingerprint: &str,
    ) -> Result<AdmissionRegistration, AdmissionRegistrationError> {
        let mut cells = self
            .registry
            .cells
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(cell) = cells.get(&key) {
            if cell.fingerprint != fingerprint {
                return Err(AdmissionRegistrationError::FingerprintConflict);
            }
            return Ok(AdmissionRegistration::Join(AdmissionJoinHandle {
                cell: Arc::clone(cell),
            }));
        }
        if cells.len() >= MAX_OPERATION_CELLS {
            return Err(AdmissionRegistrationError::Overloaded);
        }
        let cell = AdmissionOperationCell::new(fingerprint.to_owned());
        cells.insert(key.clone(), Arc::clone(&cell));
        Ok(AdmissionRegistration::Owner(AdmissionOwnerGuard {
            registry: Arc::downgrade(&self.registry),
            key,
            cell,
            published: false,
        }))
    }

    pub(crate) async fn read(
        &self,
        key: DispatchAdmissionKey,
    ) -> Result<Option<DispatchAdmissionRecord>, String> {
        let db = Arc::clone(&self.db);
        db.run_blocking(move |db| db.dispatch_admission(&key))
            .await
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn insert(
        &self,
        key: DispatchAdmissionKey,
        request_fingerprint: String,
        requested_run_id: Option<String>,
        effective_run_id: Option<String>,
        pending_input_id: Option<String>,
        outcome: Option<DispatchOutcome>,
        attachment_claims: Vec<PromptAttachmentClaim>,
    ) -> Result<DispatchAdmissionRecord, String> {
        let db = Arc::clone(&self.db);
        db.run_blocking(move |db| {
            db.insert_dispatch_admission_for_existing_thread(
                NewDispatchAdmission {
                    key: &key,
                    request_fingerprint: &request_fingerprint,
                    requested_run_id: requested_run_id.as_deref(),
                    effective_run_id: effective_run_id.as_deref(),
                    pending_input_id: pending_input_id.as_deref(),
                    outcome,
                },
                &attachment_claims,
            )
        })
        .await
        .map_err(|error| error.to_string())
    }

    pub(crate) async fn insert_no_active(
        &self,
        key: DispatchAdmissionKey,
        request_fingerprint: String,
    ) -> Result<DispatchAdmissionRecord, String> {
        let db = Arc::clone(&self.db);
        db.run_blocking(move |db| {
            db.insert_no_active_dispatch_admission(&key, &request_fingerprint)
        })
        .await
        .map_err(|error| error.to_string())
    }

    pub(crate) async fn start_handoff(
        &self,
        key: DispatchAdmissionKey,
    ) -> Result<Option<DispatchAdmissionRecord>, String> {
        let db = Arc::clone(&self.db);
        db.run_blocking(move |db| db.start_dispatch_handoff(&key))
            .await
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn settle(
        &self,
        key: DispatchAdmissionKey,
        state: DispatchAdmissionState,
        outcome: Option<DispatchOutcome>,
        effective_run_id: Option<String>,
        pending_input_id: Option<String>,
        http_status: i64,
        error_code: Option<String>,
        error_message: Option<String>,
    ) -> Result<DispatchAdmissionRecord, String> {
        let db = Arc::clone(&self.db);
        db.run_blocking(move |db| {
            db.settle_dispatch_admission(
                &key,
                state,
                outcome,
                effective_run_id.as_deref(),
                pending_input_id.as_deref(),
                http_status,
                error_code.as_deref(),
                error_message.as_deref(),
            )
        })
        .await
        .map_err(|error| error.to_string())
    }
}

pub(crate) fn resolve_dispatch_correlation(
    request: &mut ChatRequest,
) -> Result<Option<DispatchCorrelation>, (StatusCode, Value)> {
    let metadata_intent = match request.metadata.get("client_intent_id") {
        Some(Value::String(value)) => Some(value.trim().to_owned()),
        Some(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                json!({"error": "metadata.client_intent_id must be a string"}),
            ));
        }
        None => None,
    };
    let top_intent = request
        .client_intent_id
        .as_deref()
        .map(str::trim)
        .map(ToOwned::to_owned);
    if let (Some(top), Some(metadata)) = (&top_intent, &metadata_intent)
        && top != metadata
    {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": "clientIntentId conflicts with metadata.client_intent_id"}),
        ));
    }
    let Some(client_intent_id) = top_intent.or(metadata_intent) else {
        if request.idempotency_scope.is_some() {
            return Err((
                StatusCode::BAD_REQUEST,
                json!({"error": "idempotencyScope requires clientIntentId"}),
            ));
        }
        return Ok(None);
    };
    validate_wire_id("clientIntentId", &client_intent_id)?;
    request.client_intent_id = Some(client_intent_id.clone());
    request.metadata.insert(
        "client_intent_id".to_owned(),
        Value::String(client_intent_id.clone()),
    );

    let (scope_identity, scope_epoch) = match request.idempotency_scope.as_ref() {
        Some(scope) => validate_explicit_scope(scope)?,
        None => (LEGACY_SCOPE_IDENTITY.to_owned(), 0),
    };
    Ok(Some(DispatchCorrelation {
        scope_identity,
        scope_epoch,
        client_intent_id,
    }))
}

pub(crate) fn resolve_stream_input_correlation(
    request: &mut StreamInputRequest,
) -> Result<Option<DispatchCorrelation>, (StatusCode, Value)> {
    let Some(client_intent_id) = request
        .client_intent_id
        .as_deref()
        .map(str::trim)
        .map(ToOwned::to_owned)
    else {
        if request.idempotency_scope.is_some() {
            return Err((
                StatusCode::BAD_REQUEST,
                json!({"error": "idempotencyScope requires clientIntentId"}),
            ));
        }
        return Ok(None);
    };
    validate_wire_id("clientIntentId", &client_intent_id)?;
    request.client_intent_id = Some(client_intent_id.clone());
    let (scope_identity, scope_epoch) = match request.idempotency_scope.as_ref() {
        Some(scope) => validate_explicit_scope(scope)?,
        None => (LEGACY_SCOPE_IDENTITY.to_owned(), 0),
    };
    Ok(Some(DispatchCorrelation {
        scope_identity,
        scope_epoch,
        client_intent_id,
    }))
}

fn validate_wire_id(name: &str, value: &str) -> Result<(), (StatusCode, Value)> {
    if value.is_empty() || value.len() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": format!("{name} must be 1..256 UTF-8 bytes")}),
        ));
    }
    Ok(())
}

pub(crate) fn validate_intent_id(
    name: &str,
    value: &str,
    reject_implicit_prefix: bool,
) -> Result<String, (StatusCode, Value)> {
    let value = value.trim();
    validate_wire_id(name, value)?;
    if reject_implicit_prefix && value.starts_with("implicit:") {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": format!("{name} uses the reserved implicit: prefix")}),
        ));
    }
    Ok(value.to_owned())
}

pub(crate) fn validate_explicit_idempotency_scope(
    scope: &IdempotencyScope,
) -> Result<(String, i64), (StatusCode, Value)> {
    validate_explicit_scope(scope)
}

fn validate_explicit_scope(scope: &IdempotencyScope) -> Result<(String, i64), (StatusCode, Value)> {
    let identity = scope.identity.trim();
    validate_wire_id("idempotencyScope.identity", identity)?;
    if identity == LEGACY_SCOPE_IDENTITY {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": "idempotencyScope.identity uses a reserved value"}),
        ));
    }
    if scope.epoch <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": "idempotencyScope.epoch must be positive"}),
        ));
    }
    Ok((identity.to_owned(), scope.epoch))
}

fn lexical_path(path: &str) -> String {
    let mut components = Vec::new();
    let absolute = Path::new(path).is_absolute();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                components.pop();
            }
            Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
        }
    }
    let joined = components.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

fn content_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        other => other,
    }
}

pub(crate) fn stable_json_fingerprint(value: Value) -> String {
    let bytes =
        serde_json::to_vec(&canonicalize_json(value)).expect("fingerprint JSON is serializable");
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn chat_request_fingerprint(request: &ChatRequest) -> String {
    let mut metadata = request.metadata.clone();
    strip_server_owned_agent_metadata(&mut metadata);
    metadata.remove("client_intent_id");
    let attachments = request
        .attachments
        .iter()
        .map(|attachment| {
            json!({
                "attachment_id": attachment.attachment_id.as_deref().map(str::trim),
                "kind": attachment.kind,
                "path": lexical_path(&attachment.path),
                "name": attachment.name,
                "media_type": attachment.media_type,
            })
        })
        .collect::<Vec<_>>();
    let images = request
        .images
        .iter()
        .map(|image| {
            json!({
                "name": image.name,
                "media_type": image.media_type,
                "content_sha256": content_hash(&image.data),
            })
        })
        .collect::<Vec<_>>();
    let files = request
        .files
        .iter()
        .map(|file| {
            json!({
                "name": file.name,
                "media_type": file.media_type,
                "content_sha256": content_hash(&file.data),
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "fingerprint_version": 1,
        "message": request.message,
        "thread_id": request.thread_id.as_deref().map(str::trim),
        "channel": "api",
        "account_id": request.account_id,
        "from_id": request.from_id,
        "bot": request.bot,
        "workspace_path": request.workspace_path.as_deref().map(lexical_path),
        "provider_type": request.provider_type,
        "attachments": attachments,
        "images": images,
        "files": files,
        "metadata": metadata,
    });
    stable_json_fingerprint(value)
}

pub(crate) fn stream_input_request_fingerprint(
    thread_id: &str,
    request: &StreamInputRequest,
) -> String {
    let attachments = request
        .attachments
        .iter()
        .map(|attachment| {
            json!({
                "attachment_id": attachment.attachment_id.as_deref().map(str::trim),
                "kind": attachment.kind,
                "path": lexical_path(&attachment.path),
                "name": attachment.name,
                "media_type": attachment.media_type,
            })
        })
        .collect::<Vec<_>>();
    let images = request
        .images
        .iter()
        .map(|image| {
            json!({
                "name": image.name,
                "media_type": image.media_type,
                "content_sha256": content_hash(&image.data),
            })
        })
        .collect::<Vec<_>>();
    let files = request
        .files
        .iter()
        .map(|file| {
            json!({
                "name": file.name,
                "media_type": file.media_type,
                "content_sha256": content_hash(&file.data),
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "fingerprint_version": 1,
        "message": request.message,
        "thread_id": thread_id,
        "attachments": attachments,
        "images": images,
        "files": files,
    });
    stable_json_fingerprint(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use garyx_models::provider::{PromptAttachment, PromptAttachmentKind};

    fn chat_with_attachment(attachment_id: &str) -> ChatRequest {
        ChatRequest {
            message: "fingerprint attachment".to_owned(),
            attachments: vec![PromptAttachment {
                attachment_id: Some(attachment_id.to_owned()),
                kind: PromptAttachmentKind::File,
                path: String::new(),
                name: "notes.txt".to_owned(),
                media_type: "text/plain".to_owned(),
            }],
            images: Vec::new(),
            files: Vec::new(),
            thread_id: Some("thread::fingerprint-test".to_owned()),
            client_intent_id: Some("fingerprint-intent".to_owned()),
            idempotency_scope: Some(IdempotencyScope {
                identity: "fingerprint-test".to_owned(),
                epoch: 1,
            }),
            bot: None,
            from_id: "api-user".to_owned(),
            account_id: "main".to_owned(),
            wait_for_response: false,
            workspace_path: None,
            provider_type: None,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn managed_attachment_identity_is_part_of_dispatch_fingerprint() {
        let first = chat_request_fingerprint(&chat_with_attachment("attachment:first"));
        let second = chat_request_fingerprint(&chat_with_attachment("attachment:second"));
        assert_ne!(first, second);

        let first = StreamInputRequest {
            thread_id: Some("thread::fingerprint-test".to_owned()),
            client_intent_id: Some("fingerprint-intent".to_owned()),
            idempotency_scope: Some(IdempotencyScope {
                identity: "fingerprint-test".to_owned(),
                epoch: 1,
            }),
            message: "fingerprint attachment".to_owned(),
            attachments: chat_with_attachment("attachment:first").attachments,
            images: Vec::new(),
            files: Vec::new(),
        };
        let mut second = StreamInputRequest {
            thread_id: first.thread_id.clone(),
            client_intent_id: first.client_intent_id.clone(),
            idempotency_scope: first.idempotency_scope.clone(),
            message: first.message.clone(),
            attachments: first.attachments.clone(),
            images: Vec::new(),
            files: Vec::new(),
        };
        second.attachments[0].attachment_id = Some("attachment:second".to_owned());
        assert_ne!(
            stream_input_request_fingerprint("thread::fingerprint-test", &first),
            stream_input_request_fingerprint("thread::fingerprint-test", &second)
        );
    }
}
