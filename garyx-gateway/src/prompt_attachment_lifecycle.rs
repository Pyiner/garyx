use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use garyx_models::provider::{PromptAttachment, PromptAttachmentKind};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::application::chat::contracts::IdempotencyScope;
use crate::garyx_db::{
    DispatchAdmissionKind, GaryxDbError, GaryxDbService, NewPromptAttachment,
    PromptAttachmentClaim, PromptAttachmentOwner, PromptAttachmentRecord,
};

#[cfg(test)]
pub(crate) const READY_TTL: chrono::Duration = chrono::Duration::hours(24);
#[cfg(test)]
pub(crate) const CLAIM_LEASE: chrono::Duration = chrono::Duration::hours(2);

#[derive(Debug, thiserror::Error)]
pub(crate) enum PromptAttachmentLifecycleError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Storage(String),
}

#[derive(Debug)]
pub(crate) struct PromptAttachmentUpload {
    pub kind: PromptAttachmentKind,
    pub name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedPromptAttachment {
    pub attachment_id: String,
    pub kind: PromptAttachmentKind,
    pub path: String,
    pub name: String,
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedPromptAttachmentPreviewMetadata {
    pub name: String,
    pub media_type: String,
}

#[derive(Clone)]
pub(crate) struct PromptAttachmentLifecycle {
    db: Arc<GaryxDbService>,
    data_dir: Arc<PathBuf>,
    root: Arc<PathBuf>,
}

impl PromptAttachmentLifecycle {
    pub(crate) fn new(db: Arc<GaryxDbService>, data_dir: PathBuf) -> Self {
        let root = data_dir.join("prompt-attachments-v1");
        Self {
            db,
            data_dir: Arc::new(data_dir),
            root: Arc::new(root),
        }
    }

    pub(crate) fn data_dir(&self) -> &Path {
        self.data_dir.as_path()
    }

    pub(crate) fn root(&self) -> &Path {
        self.root.as_path()
    }

    pub(crate) async fn upload(
        &self,
        scope: Option<&IdempotencyScope>,
        uploads: Vec<PromptAttachmentUpload>,
    ) -> Result<Vec<ManagedPromptAttachment>, PromptAttachmentLifecycleError> {
        self.upload_at(scope, uploads, Utc::now()).await
    }

    pub(crate) async fn upload_at(
        &self,
        scope: Option<&IdempotencyScope>,
        uploads: Vec<PromptAttachmentUpload>,
        now: DateTime<Utc>,
    ) -> Result<Vec<ManagedPromptAttachment>, PromptAttachmentLifecycleError> {
        let (scope_identity, scope_epoch) = validated_scope(scope)?;
        ensure_managed_root(self.root()).await?;
        let created_at = now.to_rfc3339();
        let mut rows = Vec::with_capacity(uploads.len());
        let mut results = Vec::with_capacity(uploads.len());
        let mut created_dirs = Vec::with_capacity(uploads.len());

        for upload in uploads {
            let attachment_id = format!("attachment:{}", uuid::Uuid::new_v4());
            let relative_path = format!("{attachment_id}/payload");
            let directory = self.root().join(&attachment_id);
            let staging = directory.join("payload.staging");
            let final_path = directory.join("payload");
            fs::create_dir(&directory).await.map_err(storage_error)?;
            created_dirs.push(directory.clone());
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&staging)
                .await
                .map_err(storage_error)?;
            file.write_all(&upload.bytes).await.map_err(storage_error)?;
            file.sync_all().await.map_err(storage_error)?;
            drop(file);
            fs::rename(&staging, &final_path)
                .await
                .map_err(storage_error)?;
            sync_directory(&directory).await?;
            let digest = format!("{:x}", Sha256::digest(&upload.bytes));
            rows.push(NewPromptAttachment {
                attachment_id: attachment_id.clone(),
                scope_identity: scope_identity.clone(),
                scope_epoch,
                relative_path: relative_path.clone(),
                kind: kind_label(&upload.kind).to_owned(),
                original_name: upload.name.clone(),
                media_type: upload.media_type.clone(),
                byte_size: i64::try_from(upload.bytes.len()).unwrap_or(i64::MAX),
                sha256: digest,
                created_at: created_at.clone(),
            });
            results.push(ManagedPromptAttachment {
                attachment_id,
                kind: upload.kind,
                path: final_path.to_string_lossy().into_owned(),
                name: upload.name,
                media_type: upload.media_type,
            });
        }
        sync_directory(self.root()).await?;
        let db = Arc::clone(&self.db);
        let rows_for_db = rows.clone();
        if let Err(error) = db
            .run_blocking(move |db| db.insert_staged_prompt_attachments(&rows_for_db))
            .await
        {
            for directory in created_dirs {
                let _ = fs::remove_dir_all(directory).await;
            }
            return Err(PromptAttachmentLifecycleError::Storage(error.to_string()));
        }
        Ok(results)
    }

    /// Resolve every attachment that belongs to the managed root and verify
    /// immutable row/file metadata before a database claim is attempted.
    /// Ordinary workspace paths are returned untouched and never become GC
    /// targets.
    pub(crate) async fn prepare_claims(
        &self,
        scope: (&str, i64),
        attachments: &mut [PromptAttachment],
    ) -> Result<Vec<PromptAttachmentClaim>, PromptAttachmentLifecycleError> {
        let mut claims = Vec::new();
        for attachment in attachments {
            let mut supplied_path = PathBuf::from(attachment.path.trim());
            if attachment
                .attachment_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
                && scope.1 > 0
                && scope.0 != "__legacy_api__"
                && let Some(legacy_relative) = legacy_uuid_relative_path(&supplied_path)
            {
                let legacy_root = legacy_prompt_attachment_root();
                let bytes = read_regular_root_confined_file(&legacy_root, &legacy_relative).await?;
                let explicit_scope = IdempotencyScope {
                    identity: scope.0.to_owned(),
                    epoch: scope.1,
                };
                let copied = self
                    .upload(
                        Some(&explicit_scope),
                        vec![PromptAttachmentUpload {
                            kind: attachment.kind.clone(),
                            name: attachment.name.clone(),
                            media_type: attachment.media_type.clone(),
                            bytes,
                        }],
                    )
                    .await?
                    .pop()
                    .expect("one legacy attachment copy");
                attachment.attachment_id = Some(copied.attachment_id);
                attachment.path = copied.path;
                attachment.name = copied.name;
                attachment.media_type = copied.media_type;
                supplied_path = PathBuf::from(&attachment.path);
            }
            let id = attachment
                .attachment_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let managed_relative = lexical_relative_under(self.root(), &supplied_path)?;
            let record = match (id.as_deref(), managed_relative.as_deref()) {
                (Some(id), _) => self.record_by_id(id).await?,
                (None, Some(relative)) => self.record_by_relative_path(relative).await?,
                (None, None) => continue,
            }
            .ok_or_else(|| {
                PromptAttachmentLifecycleError::Invalid(
                    "path inside the managed attachment root has no ownership row".to_owned(),
                )
            })?;
            if let Some(id) = id.as_deref()
                && id != record.attachment_id
            {
                return Err(PromptAttachmentLifecycleError::Invalid(
                    "attachmentId does not match the managed path".to_owned(),
                ));
            }
            let expected_path = self.absolute_path(&record.relative_path)?;
            if !attachment.path.trim().is_empty() && supplied_path != expected_path {
                return Err(PromptAttachmentLifecycleError::Invalid(format!(
                    "managed attachment path mismatch: {}",
                    record.attachment_id
                )));
            }
            if record.scope_identity != scope.0 || record.scope_epoch != scope.1 {
                let legacy_upgrade = record.scope_identity == "__legacy_api__"
                    && record.scope_epoch == 0
                    && scope.1 > 0
                    && scope.0 != "__legacy_api__";
                if !legacy_upgrade {
                    return Err(PromptAttachmentLifecycleError::Conflict(format!(
                        "managed attachment scope mismatch: {}",
                        record.attachment_id
                    )));
                }
            }
            if record.kind != kind_label(&attachment.kind) {
                return Err(PromptAttachmentLifecycleError::Invalid(format!(
                    "managed attachment kind mismatch: {}",
                    record.attachment_id
                )));
            }
            let bytes = read_regular_root_confined_file(self.root(), &record.relative_path).await?;
            let digest = format!("{:x}", Sha256::digest(&bytes));
            if digest != record.sha256
                || i64::try_from(bytes.len()).unwrap_or(i64::MAX) != record.byte_size
            {
                return Err(PromptAttachmentLifecycleError::Invalid(format!(
                    "managed attachment content changed: {}",
                    record.attachment_id
                )));
            }
            attachment.attachment_id = Some(record.attachment_id.clone());
            attachment.path = expected_path.to_string_lossy().into_owned();
            attachment.name = record.original_name.clone();
            attachment.media_type = record.media_type.clone();
            claims.push(PromptAttachmentClaim {
                attachment_id: record.attachment_id,
                expected_relative_path: record.relative_path,
                expected_kind: record.kind,
                expected_sha256: record.sha256,
            });
        }
        Ok(claims)
    }

    /// Claim managed files for an uncorrelated legacy dispatch after its
    /// bridge plan has fixed the effective run identity. The provider is not
    /// called until this transaction commits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn claim_standalone(
        &self,
        scope: (&str, i64),
        thread_id: &str,
        kind: DispatchAdmissionKind,
        client_intent_id: Option<&str>,
        requested_run_id: Option<&str>,
        effective_run_id: &str,
        claims: &[PromptAttachmentClaim],
    ) -> Result<(), PromptAttachmentLifecycleError> {
        if claims.is_empty() {
            return Ok(());
        }
        let scope_identity = scope.0.to_owned();
        let scope_epoch = scope.1;
        let thread_id = thread_id.to_owned();
        let client_intent_id = client_intent_id.map(ToOwned::to_owned);
        let requested_run_id = requested_run_id.map(ToOwned::to_owned);
        let effective_run_id = effective_run_id.to_owned();
        let claims = claims.to_vec();
        let now_string = Utc::now().to_rfc3339();
        let db = Arc::clone(&self.db);
        db.run_blocking(move |db| {
            db.claim_prompt_attachments(
                &claims,
                PromptAttachmentOwner {
                    scope_identity: &scope_identity,
                    scope_epoch,
                    thread_id: &thread_id,
                    kind,
                    client_intent_id: client_intent_id.as_deref(),
                    requested_run_id: requested_run_id.as_deref(),
                    effective_run_id: &effective_run_id,
                },
                &now_string,
            )
        })
        .await
        .map_err(|error| match error {
            GaryxDbError::BadRequest(message) => PromptAttachmentLifecycleError::Conflict(message),
            other => PromptAttachmentLifecycleError::Storage(other.to_string()),
        })
    }

    async fn record_by_id(
        &self,
        id: &str,
    ) -> Result<Option<PromptAttachmentRecord>, PromptAttachmentLifecycleError> {
        let db = Arc::clone(&self.db);
        let id = id.to_owned();
        db.run_blocking(move |db| db.prompt_attachment_by_id(&id))
            .await
            .map_err(|error| PromptAttachmentLifecycleError::Storage(error.to_string()))
    }

    async fn record_by_relative_path(
        &self,
        path: &str,
    ) -> Result<Option<PromptAttachmentRecord>, PromptAttachmentLifecycleError> {
        let db = Arc::clone(&self.db);
        let path = path.to_owned();
        db.run_blocking(move |db| db.prompt_attachment_by_relative_path(&path))
            .await
            .map_err(|error| PromptAttachmentLifecycleError::Storage(error.to_string()))
    }

    fn absolute_path(
        &self,
        relative_path: &str,
    ) -> Result<PathBuf, PromptAttachmentLifecycleError> {
        validate_relative_path(relative_path)?;
        Ok(self.root().join(relative_path))
    }

    /// Return the persisted presentation metadata for a path in managed
    /// attachment storage. A managed payload never falls back to its physical
    /// basename (`payload`) for MIME detection; the catalog row is the single
    /// source of truth.
    pub(crate) async fn preview_metadata_for_path(
        &self,
        canonical_path: &Path,
    ) -> Result<Option<ManagedPromptAttachmentPreviewMetadata>, PromptAttachmentLifecycleError>
    {
        let root_metadata = match fs::symlink_metadata(self.root()).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(storage_error(error)),
        };
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(PromptAttachmentLifecycleError::Invalid(
                "managed attachment root is not a real directory".to_owned(),
            ));
        }
        let canonical_root = fs::canonicalize(self.root()).await.map_err(storage_error)?;
        let Ok(relative) = canonical_path.strip_prefix(&canonical_root) else {
            return Ok(None);
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        validate_relative_path(&relative)?;
        let record = self
            .record_by_relative_path(&relative)
            .await?
            .ok_or_else(|| {
                PromptAttachmentLifecycleError::Invalid(
                    "path inside the managed attachment root has no ownership row".to_owned(),
                )
            })?;
        let expected_path = canonical_root.join(&record.relative_path);
        if canonical_path != expected_path {
            return Err(PromptAttachmentLifecycleError::Invalid(
                "managed attachment path does not match its ownership row".to_owned(),
            ));
        }
        Ok(Some(ManagedPromptAttachmentPreviewMetadata {
            name: record.original_name,
            media_type: record.media_type,
        }))
    }

    /// Delete durable attachment content only as a retryable thread-cleanup
    /// outbox step. Physical deletion precedes row deletion so a crash can
    /// safely retry without losing the ownership record.
    pub(crate) async fn delete_thread_attachments(
        &self,
        thread_id: &str,
    ) -> Result<(), PromptAttachmentLifecycleError> {
        let db = Arc::clone(&self.db);
        let query_thread_id = thread_id.to_owned();
        let records = db
            .run_blocking(move |db| db.owned_prompt_attachments_for_thread(&query_thread_id))
            .await
            .map_err(|error| PromptAttachmentLifecycleError::Storage(error.to_string()))?;
        for record in records {
            self.delete_record_file(&record).await?;
            let db = Arc::clone(&self.db);
            let attachment_id = record.attachment_id;
            let owner_thread_id = thread_id.to_owned();
            let deleted = db
                .run_blocking(move |db| {
                    db.delete_owned_prompt_attachment(&attachment_id, &owner_thread_id)
                })
                .await
                .map_err(|error| PromptAttachmentLifecycleError::Storage(error.to_string()))?;
            if !deleted {
                return Err(PromptAttachmentLifecycleError::Conflict(
                    "managed attachment ownership changed during thread cleanup".to_owned(),
                ));
            }
        }
        Ok(())
    }

    // Phase-1 regression probes still invoke the two retired cleanup triggers.
    // Keep those names test-only and inert: production has no timer/terminal
    // attachment-cleanup API or worker, while the original assertions remain
    // executable against the durable ownership model.
    #[cfg(test)]
    pub(crate) async fn mark_run_terminal(
        &self,
        _effective_run_id: &str,
    ) -> Result<(), PromptAttachmentLifecycleError> {
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn process_cleanup_once_at(
        &self,
        _now: DateTime<Utc>,
    ) -> Result<(), PromptAttachmentLifecycleError> {
        Ok(())
    }

    async fn delete_record_file(
        &self,
        record: &PromptAttachmentRecord,
    ) -> Result<(), PromptAttachmentLifecycleError> {
        let path = self.absolute_path(&record.relative_path)?;
        let parent = path.parent().ok_or_else(|| {
            PromptAttachmentLifecycleError::Invalid("managed attachment has no parent".to_owned())
        })?;
        for candidate in [self.root(), parent, path.as_path()] {
            match fs::symlink_metadata(candidate).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PromptAttachmentLifecycleError::Invalid(
                        "refusing to delete a symlink in the managed attachment root".to_owned(),
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(storage_error(error)),
            }
        }
        match fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
        match fs::remove_dir(parent).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
        Ok(())
    }
}

fn validated_scope(
    scope: Option<&IdempotencyScope>,
) -> Result<(String, i64), PromptAttachmentLifecycleError> {
    match scope {
        None => Ok(("__legacy_api__".to_owned(), 0)),
        Some(scope) => crate::conversation_admission::validate_explicit_idempotency_scope(scope)
            .map_err(|(_, payload)| {
                PromptAttachmentLifecycleError::Invalid(
                    payload
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("invalid idempotency scope")
                        .to_owned(),
                )
            }),
    }
}

fn kind_label(kind: &PromptAttachmentKind) -> &'static str {
    match kind {
        PromptAttachmentKind::Image => "image",
        PromptAttachmentKind::File => "file",
    }
}

fn validate_relative_path(path: &str) -> Result<(), PromptAttachmentLifecycleError> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(PromptAttachmentLifecycleError::Invalid(
            "managed attachment path escapes its root".to_owned(),
        ));
    }
    Ok(())
}

fn lexical_relative_under(
    root: &Path,
    path: &Path,
) -> Result<Option<String>, PromptAttachmentLifecycleError> {
    if path.as_os_str().is_empty() {
        return Ok(None);
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return Ok(None);
    };
    let relative = relative.to_string_lossy().replace('\\', "/");
    validate_relative_path(&relative)?;
    Ok(Some(relative))
}

fn legacy_prompt_attachment_root() -> PathBuf {
    std::env::temp_dir()
        .join("garyx-gateway")
        .join("prompt-attachments")
}

fn legacy_uuid_relative_path(path: &Path) -> Option<String> {
    let root = legacy_prompt_attachment_root();
    let relative = path.strip_prefix(&root).ok()?;
    let mut components = relative.components();
    let Component::Normal(file_name) = components.next()? else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }
    let file_name = file_name.to_str()?;
    if file_name.len() <= 37 || file_name.as_bytes().get(36) != Some(&b'-') {
        return None;
    }
    uuid::Uuid::parse_str(&file_name[..36]).ok()?;
    Some(file_name.to_owned())
}

async fn read_regular_root_confined_file(
    root: &Path,
    relative_path: &str,
) -> Result<Vec<u8>, PromptAttachmentLifecycleError> {
    validate_relative_path(relative_path)?;
    let path = root.join(relative_path);
    let parent = path.parent().ok_or_else(|| {
        PromptAttachmentLifecycleError::Invalid("managed attachment has no parent".to_owned())
    })?;
    for candidate in [root, parent, path.as_path()] {
        let metadata = fs::symlink_metadata(candidate)
            .await
            .map_err(storage_error)?;
        if metadata.file_type().is_symlink() {
            return Err(PromptAttachmentLifecycleError::Invalid(
                "managed attachment path contains a symlink".to_owned(),
            ));
        }
    }
    let metadata = fs::metadata(&path).await.map_err(storage_error)?;
    if !metadata.is_file() {
        return Err(PromptAttachmentLifecycleError::Invalid(
            "managed attachment payload is not a regular file".to_owned(),
        ));
    }
    fs::read(path).await.map_err(storage_error)
}

async fn sync_directory(path: &Path) -> Result<(), PromptAttachmentLifecycleError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .map_err(|error| PromptAttachmentLifecycleError::Storage(error.to_string()))?
        .map_err(storage_error)
}

fn storage_error(error: std::io::Error) -> PromptAttachmentLifecycleError {
    PromptAttachmentLifecycleError::Storage(error.to_string())
}

async fn ensure_managed_root(root: &Path) -> Result<(), PromptAttachmentLifecycleError> {
    fs::create_dir_all(root).await.map_err(storage_error)?;
    let metadata = fs::symlink_metadata(root).await.map_err(storage_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PromptAttachmentLifecycleError::Invalid(
            "managed attachment root is not a real directory".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::tempdir;

    #[tokio::test]
    async fn unreferenced_staging_upload_survives_elapsed_legacy_ttl() {
        let temp = tempdir().unwrap();
        let db = Arc::new(GaryxDbService::memory().unwrap());
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        let lifecycle = PromptAttachmentLifecycle::new(db.clone(), temp.path().to_path_buf());
        let now = DateTime::parse_from_rfc3339("2026-07-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let uploaded = lifecycle
            .upload_at(
                Some(&IdempotencyScope {
                    identity: "attachment-test".to_owned(),
                    epoch: 1,
                }),
                vec![PromptAttachmentUpload {
                    kind: PromptAttachmentKind::File,
                    name: "note.txt".to_owned(),
                    media_type: "text/plain".to_owned(),
                    bytes: b"hello".to_vec(),
                }],
                now,
            )
            .await
            .unwrap();
        let unmanaged = temp.path().join("unmanaged.txt");
        fs::write(&unmanaged, b"keep").await.unwrap();
        assert!(Path::new(&uploaded[0].path).exists());
        lifecycle
            .process_cleanup_once_at(now + READY_TTL + Duration::seconds(1))
            .await
            .unwrap();
        assert!(Path::new(&uploaded[0].path).exists());
        assert!(unmanaged.exists());
        assert!(
            db.prompt_attachment_by_id(&uploaded[0].attachment_id)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn claimed_attachment_is_single_owner_and_survives_run_terminal() {
        let temp = tempdir().unwrap();
        let db = Arc::new(GaryxDbService::memory().unwrap());
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        let lifecycle = PromptAttachmentLifecycle::new(db.clone(), temp.path().to_path_buf());
        let scope = IdempotencyScope {
            identity: "attachment-terminal-test".to_owned(),
            epoch: 1,
        };
        let uploaded = lifecycle
            .upload(
                Some(&scope),
                vec![PromptAttachmentUpload {
                    kind: PromptAttachmentKind::File,
                    name: "note.txt".to_owned(),
                    media_type: "text/plain".to_owned(),
                    bytes: b"terminal".to_vec(),
                }],
            )
            .await
            .unwrap();
        let managed = &uploaded[0];
        let mut attachments = vec![PromptAttachment {
            attachment_id: Some(managed.attachment_id.clone()),
            kind: PromptAttachmentKind::File,
            path: managed.path.clone(),
            name: managed.name.clone(),
            media_type: managed.media_type.clone(),
        }];
        let claims = lifecycle
            .prepare_claims((&scope.identity, scope.epoch), &mut attachments)
            .await
            .unwrap();
        lifecycle
            .claim_standalone(
                (&scope.identity, scope.epoch),
                "thread::attachment-owner",
                DispatchAdmissionKind::ChatStart,
                Some("intent-one"),
                Some("run-one"),
                "run-one",
                &claims,
            )
            .await
            .unwrap();
        let conflict = lifecycle
            .claim_standalone(
                (&scope.identity, scope.epoch),
                "thread::attachment-other",
                DispatchAdmissionKind::ChatStart,
                Some("intent-two"),
                Some("run-two"),
                "run-two",
                &claims,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            PromptAttachmentLifecycleError::Conflict(_)
        ));
        assert!(Path::new(&managed.path).exists());

        lifecycle.mark_run_terminal("run-one").await.unwrap();

        assert!(
            Path::new(&managed.path).exists(),
            "a committed attachment remains conversation content after its provider run terminates"
        );
        assert!(
            matches!(
                db.prompt_attachment_by_id(&managed.attachment_id)
                    .unwrap()
                    .map(|record| (record.state, record.owner_thread_id)),
                Some((
                    crate::garyx_db::PromptAttachmentState::Owned,
                    Some(owner)
                )) if owner == "thread::attachment-owner"
            ),
            "the attachment remains owned by its thread until thread cleanup"
        );
    }

    /// RED reproduction for #TASK-2511's committed-attachment lifetime.
    ///
    /// Once this managed image has been claimed by a chat start, the committed
    /// transcript keeps referencing its path after the provider run is gone.
    /// Expiring that claim therefore turns a previously renderable message into
    /// the permanent filename-only image placeholder seen by the client.
    #[tokio::test]
    async fn committed_chat_image_survives_claim_lease_expiry() {
        let temp = tempdir().unwrap();
        let db = Arc::new(GaryxDbService::memory().unwrap());
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        let lifecycle = PromptAttachmentLifecycle::new(db.clone(), temp.path().to_path_buf());
        let scope = IdempotencyScope {
            identity: "committed-image-lifetime".to_owned(),
            epoch: 1,
        };
        let uploaded = lifecycle
            .upload(
                Some(&scope),
                vec![PromptAttachmentUpload {
                    kind: PromptAttachmentKind::Image,
                    name: "photo-1.jpg".to_owned(),
                    media_type: "image/jpeg".to_owned(),
                    bytes: vec![0xff, 0xd8, 0xff, 0xd9],
                }],
            )
            .await
            .unwrap();
        let managed = uploaded.first().expect("managed image");
        let mut committed_attachments = vec![PromptAttachment {
            attachment_id: Some(managed.attachment_id.clone()),
            kind: PromptAttachmentKind::Image,
            path: managed.path.clone(),
            name: managed.name.clone(),
            media_type: managed.media_type.clone(),
        }];
        let claims = lifecycle
            .prepare_claims((&scope.identity, scope.epoch), &mut committed_attachments)
            .await
            .unwrap();
        let claimed_at = Utc::now();
        lifecycle
            .claim_standalone(
                (&scope.identity, scope.epoch),
                "thread::committed-image-lifetime",
                DispatchAdmissionKind::ChatStart,
                Some("intent-committed-image"),
                Some("run-committed-image"),
                "run-committed-image",
                &claims,
            )
            .await
            .unwrap();

        lifecycle
            .process_cleanup_once_at(claimed_at + CLAIM_LEASE + Duration::seconds(1))
            .await
            .unwrap();

        assert!(
            Path::new(&committed_attachments[0].path).exists(),
            "a chat image remains conversation content after its dispatch lease expires"
        );
        assert!(
            matches!(
                db.prompt_attachment_by_id(&managed.attachment_id)
                    .unwrap()
                    .map(|record| (record.state, record.owner_thread_id)),
                Some((
                    crate::garyx_db::PromptAttachmentState::Owned,
                    Some(owner)
                )) if owner == "thread::committed-image-lifetime"
            ),
            "the durable attachment record must remain owned by its thread"
        );
    }

    #[tokio::test]
    async fn explicit_request_lazy_copies_legacy_uuid_file_without_deleting_source() {
        let temp = tempdir().unwrap();
        let db = Arc::new(GaryxDbService::memory().unwrap());
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        let lifecycle = PromptAttachmentLifecycle::new(db.clone(), temp.path().to_path_buf());
        let legacy_root = legacy_prompt_attachment_root();
        fs::create_dir_all(&legacy_root).await.unwrap();
        let legacy_path = legacy_root.join(format!("{}-notes.txt", uuid::Uuid::new_v4()));
        fs::write(&legacy_path, b"legacy payload").await.unwrap();
        let mut attachments = vec![PromptAttachment {
            attachment_id: None,
            kind: PromptAttachmentKind::File,
            path: legacy_path.to_string_lossy().into_owned(),
            name: "notes.txt".to_owned(),
            media_type: "text/plain".to_owned(),
        }];

        let claims = lifecycle
            .prepare_claims(("authenticated-upgrade", 2), &mut attachments)
            .await
            .unwrap();

        assert_eq!(claims.len(), 1);
        let copied_path = PathBuf::from(&attachments[0].path);
        assert!(copied_path.starts_with(lifecycle.root()));
        assert_eq!(fs::read(&copied_path).await.unwrap(), b"legacy payload");
        assert_eq!(fs::read(&legacy_path).await.unwrap(), b"legacy payload");
        lifecycle
            .claim_standalone(
                ("authenticated-upgrade", 2),
                "thread::legacy-copy",
                DispatchAdmissionKind::ChatStart,
                Some("intent-copy"),
                Some("run-copy"),
                "run-copy",
                &claims,
            )
            .await
            .unwrap();
        lifecycle.mark_run_terminal("run-copy").await.unwrap();
        assert!(copied_path.exists());
        assert!(legacy_path.exists());

        fs::remove_file(legacy_path).await.unwrap();
    }

    #[tokio::test]
    async fn thread_cleanup_deletes_owned_attachment_but_retains_staging_upload() {
        let temp = tempdir().unwrap();
        let db = Arc::new(GaryxDbService::memory().unwrap());
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        let lifecycle = PromptAttachmentLifecycle::new(db.clone(), temp.path().to_path_buf());
        let scope = IdempotencyScope {
            identity: "thread-cleanup-ownership".to_owned(),
            epoch: 1,
        };
        let uploaded = lifecycle
            .upload(
                Some(&scope),
                vec![
                    PromptAttachmentUpload {
                        kind: PromptAttachmentKind::Image,
                        name: "owned.jpg".to_owned(),
                        media_type: "image/jpeg".to_owned(),
                        bytes: vec![0xff, 0xd8, 0xff, 0xd9],
                    },
                    PromptAttachmentUpload {
                        kind: PromptAttachmentKind::File,
                        name: "staged.txt".to_owned(),
                        media_type: "text/plain".to_owned(),
                        bytes: b"staged".to_vec(),
                    },
                ],
            )
            .await
            .unwrap();
        let owned = &uploaded[0];
        let staged = &uploaded[1];
        let mut attachments = vec![PromptAttachment {
            attachment_id: Some(owned.attachment_id.clone()),
            kind: owned.kind.clone(),
            path: owned.path.clone(),
            name: owned.name.clone(),
            media_type: owned.media_type.clone(),
        }];
        let claims = lifecycle
            .prepare_claims((&scope.identity, scope.epoch), &mut attachments)
            .await
            .unwrap();
        lifecycle
            .claim_standalone(
                (&scope.identity, scope.epoch),
                "thread::cleanup-owned",
                DispatchAdmissionKind::ChatStart,
                Some("intent-cleanup"),
                Some("run-cleanup"),
                "run-cleanup",
                &claims,
            )
            .await
            .unwrap();

        lifecycle
            .delete_thread_attachments("thread::cleanup-owned")
            .await
            .unwrap();

        assert!(!Path::new(&owned.path).exists());
        assert!(
            db.prompt_attachment_by_id(&owned.attachment_id)
                .unwrap()
                .is_none()
        );
        assert!(Path::new(&staged.path).exists());
        assert_eq!(
            db.prompt_attachment_by_id(&staged.attachment_id)
                .unwrap()
                .unwrap()
                .state,
            crate::garyx_db::PromptAttachmentState::Staged
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_symlink_is_refused_without_deleting_target() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let db = Arc::new(GaryxDbService::memory().unwrap());
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        let lifecycle = PromptAttachmentLifecycle::new(db.clone(), temp.path().to_path_buf());
        let uploaded = lifecycle
            .upload(
                None,
                vec![PromptAttachmentUpload {
                    kind: PromptAttachmentKind::File,
                    name: "note.txt".to_owned(),
                    media_type: "text/plain".to_owned(),
                    bytes: b"hello".to_vec(),
                }],
            )
            .await
            .unwrap();
        let outside = temp.path().join("outside.txt");
        fs::write(&outside, b"outside").await.unwrap();
        fs::remove_file(&uploaded[0].path).await.unwrap();
        symlink(&outside, &uploaded[0].path).unwrap();
        let row = db
            .prompt_attachment_by_id(&uploaded[0].attachment_id)
            .unwrap()
            .unwrap();
        let error = lifecycle.delete_record_file(&row).await.unwrap_err();
        assert!(error.to_string().contains("symlink"));
        assert_eq!(fs::read(&outside).await.unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn upload_and_cleanup_refuse_a_symlinked_managed_root() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let db = Arc::new(GaryxDbService::memory().unwrap());
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        let lifecycle = PromptAttachmentLifecycle::new(db.clone(), temp.path().to_path_buf());
        let outside = temp.path().join("outside-root-target");
        fs::create_dir(&outside).await.unwrap();
        symlink(&outside, lifecycle.root()).unwrap();

        let upload_error = lifecycle
            .upload(
                None,
                vec![PromptAttachmentUpload {
                    kind: PromptAttachmentKind::File,
                    name: "note.txt".to_owned(),
                    media_type: "text/plain".to_owned(),
                    bytes: b"must not escape".to_vec(),
                }],
            )
            .await
            .unwrap_err();
        assert!(upload_error.to_string().contains("real directory"));
        assert!(
            fs::read_dir(&outside)
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none()
        );

        let attachment_id = "attachment:symlink-root-delete";
        let external_attachment = outside.join(attachment_id);
        fs::create_dir(&external_attachment).await.unwrap();
        let external_payload = external_attachment.join("payload");
        fs::write(&external_payload, b"keep outside").await.unwrap();
        db.insert_staged_prompt_attachments(&[NewPromptAttachment {
            attachment_id: attachment_id.to_owned(),
            scope_identity: "__legacy_api__".to_owned(),
            scope_epoch: 0,
            relative_path: format!("{attachment_id}/payload"),
            kind: "file".to_owned(),
            original_name: "note.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            byte_size: 12,
            sha256: "not-read-for-delete".to_owned(),
            created_at: "2026-07-20T00:00:00Z".to_owned(),
        }])
        .unwrap();
        let row = db.prompt_attachment_by_id(attachment_id).unwrap().unwrap();

        let delete_error = lifecycle.delete_record_file(&row).await.unwrap_err();
        assert!(delete_error.to_string().contains("symlink"));
        assert_eq!(fs::read(external_payload).await.unwrap(), b"keep outside");
    }
}
