//! One-shot legacy archive import lifecycle.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use garyx_router::{
    FileThreadStore, ThreadStore, ThreadStoreError, ThreadTranscriptStore, is_thread_key,
};
use serde_json::Value;

use crate::garyx_db::{GaryxDbError, GaryxDbService, is_retired_workflow_thread_record};

pub(crate) const THREAD_RECORDS_IMPORT_NAME: &str = "thread_records_import";
pub(crate) const THREAD_RECORDS_IMPORT_VERSION: i64 = 1;
pub(crate) const LEGACY_ARCHIVE_RETIREMENT_NAME: &str = "legacy_archive_retirement";
pub(crate) const LEGACY_ARCHIVE_RETIREMENT_VERSION: i64 = 1;

const ARCHIVE_DIR_NAMES: [&str; 2] = ["threads", "sessions"];
const RETIREMENT_BACKUP_DIR: &str = "legacy-archive-v1";
const LIFECYCLE_LOCK_FILE: &str = "legacy-boot-import.lock";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadRecordImportSummary {
    pub source_keys: usize,
    pub imported: usize,
    pub discarded: usize,
    pub failed: usize,
    pub transcripts_backfilled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyBootImportOutcome {
    Complete,
    ImportedAndRetired(ThreadRecordImportSummary),
    ImportedRetirementPending(ThreadRecordImportSummary),
    RetirementOnly { pending: bool },
    NothingToImport,
}

#[derive(Debug, thiserror::Error)]
pub enum LegacyBootImportError {
    #[error(transparent)]
    Database(#[from] GaryxDbError),
    #[error("legacy archive {operation} failed for {}: {source}", path.display())]
    ArchiveIo {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("legacy boot import lock is already held: {}", .0.display())]
    LockBusy(PathBuf),
    #[error("failed to open the legacy thread archive at {}: {source}", data_dir.display())]
    SourceOpen {
        data_dir: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to list the legacy thread archive: {0}")]
    SourceList(ThreadStoreError),
    #[error("recovery intent but no archive restored under {}", .0.display())]
    RecoveryArchiveMissing(PathBuf),
    #[error(
        "recovery restore is incomplete: backup destination still exists at {} (move, do not copy, both archive directories back before rebooting)",
        .0.display()
    )]
    RecoveryArchiveIncomplete(PathBuf),
    #[error("legacy thread archive import failed: {0:?}")]
    ImportFailed(ThreadRecordImportSummary),
}

#[async_trait]
pub(crate) trait LegacyArchiveReader: Send + Sync {
    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError>;
    async fn get(&self, key: &str) -> Result<Option<Value>, ThreadStoreError>;
}

#[async_trait]
impl LegacyArchiveReader for FileThreadStore {
    async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
        ThreadStore::list_keys(self, prefix).await
    }

    async fn get(&self, key: &str) -> Result<Option<Value>, ThreadStoreError> {
        ThreadStore::get(self, key).await
    }
}

trait ArchiveLockHandle: Send + Sync {
    fn try_lock_exclusive(&self) -> std::io::Result<()>;
}

#[async_trait]
trait ArchiveFs: std::fmt::Debug + Send + Sync {
    async fn open_lock(&self, path: &Path) -> std::io::Result<Box<dyn ArchiveLockHandle>>;
    async fn exists(&self, path: &Path) -> std::io::Result<bool>;
    async fn rename_archive_dir(&self, source: &Path, destination: &Path) -> std::io::Result<()>;
}

#[derive(Debug, Default)]
struct RealArchiveFs;

struct RealArchiveLockHandle {
    file: std::fs::File,
}

impl ArchiveLockHandle for RealArchiveLockHandle {
    fn try_lock_exclusive(&self) -> std::io::Result<()> {
        Ok(self.file.try_lock()?)
    }
}

#[async_trait]
impl ArchiveFs for RealArchiveFs {
    async fn open_lock(&self, path: &Path) -> std::io::Result<Box<dyn ArchiveLockHandle>> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
            .await?
            .into_std()
            .await;
        Ok(Box::new(RealArchiveLockHandle { file }))
    }

    async fn exists(&self, path: &Path) -> std::io::Result<bool> {
        match tokio::fs::symlink_metadata(path).await {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn rename_archive_dir(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(source, destination).await
    }
}

#[async_trait]
trait ArchiveSourceFactory: std::fmt::Debug + Send + Sync {
    async fn open(&self, data_dir: &Path) -> std::io::Result<Arc<dyn LegacyArchiveReader>>;
}

#[derive(Debug, Default)]
struct FileArchiveSourceFactory;

#[async_trait]
impl ArchiveSourceFactory for FileArchiveSourceFactory {
    async fn open(&self, data_dir: &Path) -> std::io::Result<Arc<dyn LegacyArchiveReader>> {
        Ok(Arc::new(FileThreadStore::new(data_dir).await?))
    }
}

pub async fn run_legacy_boot_import(
    garyx_db: &Arc<GaryxDbService>,
    sqlite_store: &Arc<dyn ThreadStore>,
    transcript_store: &Arc<ThreadTranscriptStore>,
    data_dir: &Path,
) -> Result<LegacyBootImportOutcome, LegacyBootImportError> {
    run_legacy_boot_import_with(
        garyx_db,
        sqlite_store,
        transcript_store,
        data_dir,
        &RealArchiveFs,
        &FileArchiveSourceFactory,
    )
    .await
}

async fn run_legacy_boot_import_with(
    garyx_db: &Arc<GaryxDbService>,
    sqlite_store: &Arc<dyn ThreadStore>,
    transcript_store: &Arc<ThreadTranscriptStore>,
    data_dir: &Path,
    archive_fs: &dyn ArchiveFs,
    source_factory: &dyn ArchiveSourceFactory,
) -> Result<LegacyBootImportOutcome, LegacyBootImportError> {
    let initial_markers = garyx_db.legacy_import_marker_pair()?;
    if initial_markers == (true, true) {
        return Ok(LegacyBootImportOutcome::Complete);
    }

    let lock_path = data_dir.join(LIFECYCLE_LOCK_FILE);
    let lifecycle_lock = archive_fs.open_lock(&lock_path).await.map_err(|source| {
        LegacyBootImportError::ArchiveIo {
            operation: "lock open",
            path: lock_path.clone(),
            source,
        }
    })?;
    match lifecycle_lock.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            return Err(LegacyBootImportError::LockBusy(lock_path));
        }
        Err(source) => {
            return Err(LegacyBootImportError::ArchiveIo {
                operation: "lock acquire",
                path: lock_path,
                source,
            });
        }
    }

    let markers = garyx_db.legacy_import_marker_pair()?;
    if markers == (true, true) {
        return Ok(LegacyBootImportOutcome::Complete);
    }
    if markers == (true, false) {
        let pending = retire_archive(garyx_db, data_dir, archive_fs).await;
        return Ok(LegacyBootImportOutcome::RetirementOnly { pending });
    }

    let recovery = markers == (false, true);
    let probe = probe_archive(data_dir, recovery, archive_fs).await?;
    if recovery {
        if let Some(destination) = probe
            .backup_paths
            .iter()
            .zip(probe.backup_exists)
            .find_map(|(path, exists)| exists.then_some(path))
        {
            return Err(LegacyBootImportError::RecoveryArchiveIncomplete(
                destination.clone(),
            ));
        }
        if !probe.source_exists.iter().any(|exists| *exists) {
            return Err(LegacyBootImportError::RecoveryArchiveMissing(
                data_dir.to_path_buf(),
            ));
        }
    } else if !probe.source_exists.iter().any(|exists| *exists) {
        garyx_db.commit_legacy_import(0, false)?;
        let pending = retire_archive(garyx_db, data_dir, archive_fs).await;
        if pending {
            tracing::warn!("fresh-install legacy archive retirement marker remains pending");
        }
        return Ok(LegacyBootImportOutcome::NothingToImport);
    }

    let source = source_factory.open(data_dir).await.map_err(|source| {
        LegacyBootImportError::SourceOpen {
            data_dir: data_dir.to_path_buf(),
            source,
        }
    })?;
    let summary =
        import_thread_records(garyx_db, source.as_ref(), sqlite_store, transcript_store).await?;
    garyx_db.commit_legacy_import(summary.source_keys, recovery)?;

    let pending = retire_archive(garyx_db, data_dir, archive_fs).await;
    tracing::info!(
        source_keys = summary.source_keys,
        imported = summary.imported,
        discarded = summary.discarded,
        transcripts_backfilled = summary.transcripts_backfilled,
        retirement_pending = pending,
        "legacy thread archive import completed"
    );
    Ok(if pending {
        LegacyBootImportOutcome::ImportedRetirementPending(summary)
    } else {
        LegacyBootImportOutcome::ImportedAndRetired(summary)
    })
}

struct ArchiveProbe {
    source_exists: [bool; 2],
    backup_exists: [bool; 2],
    backup_paths: [PathBuf; 2],
}

async fn probe_archive(
    data_dir: &Path,
    recovery: bool,
    archive_fs: &dyn ArchiveFs,
) -> Result<ArchiveProbe, LegacyBootImportError> {
    let source_paths = ARCHIVE_DIR_NAMES.map(|name| data_dir.join(name));
    let backup_root = data_dir.join("backups").join(RETIREMENT_BACKUP_DIR);
    let backup_paths = ARCHIVE_DIR_NAMES.map(|name| backup_root.join(name));
    let mut source_exists = [false; 2];
    let mut backup_exists = [false; 2];
    for (index, path) in source_paths.iter().enumerate() {
        source_exists[index] = archive_exists(archive_fs, path, "archive probe").await?;
    }
    if recovery {
        for (index, path) in backup_paths.iter().enumerate() {
            backup_exists[index] = archive_exists(archive_fs, path, "backup probe").await?;
        }
    }
    Ok(ArchiveProbe {
        source_exists,
        backup_exists,
        backup_paths,
    })
}

async fn archive_exists(
    archive_fs: &dyn ArchiveFs,
    path: &Path,
    operation: &'static str,
) -> Result<bool, LegacyBootImportError> {
    archive_fs
        .exists(path)
        .await
        .map_err(|source| LegacyBootImportError::ArchiveIo {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

async fn import_thread_records(
    garyx_db: &Arc<GaryxDbService>,
    source: &dyn LegacyArchiveReader,
    sqlite_store: &Arc<dyn ThreadStore>,
    transcript_store: &Arc<ThreadTranscriptStore>,
) -> Result<ThreadRecordImportSummary, LegacyBootImportError> {
    let source_keys = source
        .list_keys(None)
        .await
        .map_err(LegacyBootImportError::SourceList)?;
    let mut summary = ThreadRecordImportSummary {
        source_keys: source_keys.len(),
        ..Default::default()
    };

    for key in source_keys {
        if is_thread_key(&key) && garyx_db.is_thread_archived(&key)? {
            match transcript_store.delete(&key).await {
                Ok(()) => summary.discarded += 1,
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(key, error = %error, "failed to delete archived-thread transcript during legacy import");
                }
            }
            continue;
        }

        let mut data = match source.get(&key).await {
            Ok(Some(data)) => data,
            Ok(None) => {
                summary.failed += 1;
                tracing::warn!(key, "legacy archive key disappeared during import");
                continue;
            }
            Err(error) => {
                summary.failed += 1;
                tracing::warn!(key, error = %error, "failed to read legacy archive record");
                continue;
            }
        };

        if is_thread_key(&key) && is_retired_workflow_thread_record(&data) {
            match transcript_store.delete(&key).await {
                Ok(()) => summary.discarded += 1,
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(key, error = %error, "failed to delete retired-workflow transcript during legacy import");
                }
            }
            continue;
        }

        let legacy_messages = data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if is_thread_key(&key) && !legacy_messages.is_empty() {
            match transcript_store
                .ensure_transcript_backfilled(&key, &legacy_messages)
                .await
            {
                Ok(outcome) => {
                    if outcome.wrote_transcript() {
                        summary.transcripts_backfilled += 1;
                    }
                }
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(key, error = %error, "failed to backfill transcript during legacy import");
                    continue;
                }
            }
            seed_legacy_message_fields(&mut data, &legacy_messages);
        }

        match sqlite_store.set(&key, data).await {
            Ok(()) => summary.imported += 1,
            Err(ThreadStoreError::Archived(_)) => match transcript_store.delete(&key).await {
                Ok(()) => summary.discarded += 1,
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(key, error = %error, "failed to clean transcript after archived write rejection");
                }
            },
            Err(error) => {
                summary.failed += 1;
                tracing::warn!(key, error = %error, "failed to write imported thread record");
            }
        }
    }

    if summary.failed > 0 {
        tracing::warn!(
            source_keys = summary.source_keys,
            imported = summary.imported,
            discarded = summary.discarded,
            failed = summary.failed,
            transcripts_backfilled = summary.transcripts_backfilled,
            "legacy thread archive import failed"
        );
        return Err(LegacyBootImportError::ImportFailed(summary));
    }
    Ok(summary)
}

fn seed_legacy_message_fields(data: &mut Value, legacy_messages: &[Value]) {
    let Some(object) = data.as_object_mut() else {
        return;
    };
    for role in ["user", "assistant"] {
        if let Some(field) = garyx_models::message_preview::preview_field_for_role(role)
            && !object.contains_key(field)
            && let Some(preview) = garyx_models::message_preview::last_message_preview_for_role(
                legacy_messages.iter(),
                role,
            )
        {
            object.insert(field.to_owned(), Value::String(preview));
        }
    }
    if let Some(task) = object.get_mut("task").and_then(Value::as_object_mut)
        && task
            .get("body")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        && let Some(first_user) = legacy_messages.iter().find_map(|message| {
            if message.get("role").and_then(Value::as_str) != Some("user") {
                return None;
            }
            message
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
    {
        task.insert("body".to_owned(), Value::String(first_user));
    }
}

/// Retirement is post-marker and therefore deliberately best effort. `true`
/// means a move or retirement-marker write remains pending for the next boot.
async fn retire_archive(
    garyx_db: &Arc<GaryxDbService>,
    data_dir: &Path,
    archive_fs: &dyn ArchiveFs,
) -> bool {
    let backup_root = data_dir.join("backups").join(RETIREMENT_BACKUP_DIR);
    let mut pending = false;
    for name in ARCHIVE_DIR_NAMES {
        let source = data_dir.join(name);
        let destination = backup_root.join(name);
        let source_exists = match archive_fs.exists(&source).await {
            Ok(exists) => exists,
            Err(error) => {
                pending = true;
                tracing::warn!(path = %source.display(), error = %error, "failed to inspect legacy archive during retirement");
                continue;
            }
        };
        if !source_exists {
            continue;
        }
        match archive_fs.exists(&destination).await {
            Ok(true) => {
                pending = true;
                tracing::warn!(
                    source = %source.display(),
                    destination = %destination.display(),
                    "legacy archive retirement destination already exists; leaving both trees intact"
                );
            }
            Ok(false) => {
                if let Err(error) = archive_fs.rename_archive_dir(&source, &destination).await {
                    pending = true;
                    tracing::warn!(
                        source = %source.display(),
                        destination = %destination.display(),
                        error = %error,
                        "failed to retire legacy archive directory"
                    );
                }
            }
            Err(error) => {
                pending = true;
                tracing::warn!(path = %destination.display(), error = %error, "failed to inspect legacy archive retirement destination");
            }
        }
    }
    if pending {
        return true;
    }
    if let Err(error) = garyx_db.record_legacy_archive_retirement() {
        tracing::warn!(error = %error, "failed to record legacy archive retirement marker");
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fmt;
    use std::io::ErrorKind;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::garyx_db::TestDbFaultPoint;
    use crate::recent_thread_projection::AlwaysActiveRunProbe;
    use crate::sqlite_thread_store::SqliteThreadStore;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ArchiveCall {
        OpenLock(PathBuf),
        AcquireLock,
        Exists(PathBuf),
        Rename {
            source: PathBuf,
            destination: PathBuf,
        },
    }

    #[derive(Debug, Default)]
    struct RecordingArchiveFs {
        inner: RealArchiveFs,
        calls: Arc<StdMutex<Vec<ArchiveCall>>>,
        fail_open_once: AtomicBool,
        acquire_error_once: Arc<StdMutex<Option<ErrorKind>>>,
        exists_errors_once: StdMutex<HashSet<PathBuf>>,
        rename_errors_once: StdMutex<HashSet<PathBuf>>,
    }

    impl RecordingArchiveFs {
        fn calls(&self) -> Vec<ArchiveCall> {
            self.calls.lock().expect("archive calls").clone()
        }

        fn fail_open_once(&self) {
            self.fail_open_once.store(true, Ordering::SeqCst);
        }

        fn fail_acquire_once(&self, kind: ErrorKind) {
            *self.acquire_error_once.lock().expect("acquire fault") = Some(kind);
        }

        fn fail_exists_once(&self, path: impl Into<PathBuf>) {
            self.exists_errors_once
                .lock()
                .expect("exists faults")
                .insert(path.into());
        }

        fn fail_rename_once(&self, source: impl Into<PathBuf>) {
            self.rename_errors_once
                .lock()
                .expect("rename faults")
                .insert(source.into());
        }
    }

    struct RecordingArchiveLock {
        inner: Box<dyn ArchiveLockHandle>,
        calls: Arc<StdMutex<Vec<ArchiveCall>>>,
        acquire_error_once: Arc<StdMutex<Option<ErrorKind>>>,
    }

    impl ArchiveLockHandle for RecordingArchiveLock {
        fn try_lock_exclusive(&self) -> std::io::Result<()> {
            self.calls
                .lock()
                .expect("archive calls")
                .push(ArchiveCall::AcquireLock);
            if let Some(kind) = self
                .acquire_error_once
                .lock()
                .expect("acquire fault")
                .take()
            {
                return Err(std::io::Error::from(kind));
            }
            self.inner.try_lock_exclusive()
        }
    }

    #[async_trait]
    impl ArchiveFs for RecordingArchiveFs {
        async fn open_lock(&self, path: &Path) -> std::io::Result<Box<dyn ArchiveLockHandle>> {
            self.calls
                .lock()
                .expect("archive calls")
                .push(ArchiveCall::OpenLock(path.to_path_buf()));
            if self.fail_open_once.swap(false, Ordering::SeqCst) {
                return Err(std::io::Error::other("injected lock-open failure"));
            }
            let inner = self.inner.open_lock(path).await?;
            Ok(Box::new(RecordingArchiveLock {
                inner,
                calls: Arc::clone(&self.calls),
                acquire_error_once: Arc::clone(&self.acquire_error_once),
            }))
        }

        async fn exists(&self, path: &Path) -> std::io::Result<bool> {
            self.calls
                .lock()
                .expect("archive calls")
                .push(ArchiveCall::Exists(path.to_path_buf()));
            if self
                .exists_errors_once
                .lock()
                .expect("exists faults")
                .remove(path)
            {
                return Err(std::io::Error::other("injected metadata failure"));
            }
            self.inner.exists(path).await
        }

        async fn rename_archive_dir(
            &self,
            source: &Path,
            destination: &Path,
        ) -> std::io::Result<()> {
            self.calls
                .lock()
                .expect("archive calls")
                .push(ArchiveCall::Rename {
                    source: source.to_path_buf(),
                    destination: destination.to_path_buf(),
                });
            if self
                .rename_errors_once
                .lock()
                .expect("rename faults")
                .remove(source)
            {
                return Err(std::io::Error::other("injected rename failure"));
            }
            self.inner.rename_archive_dir(source, destination).await
        }
    }

    struct StaticSourceFactory {
        source: Arc<dyn LegacyArchiveReader>,
        fail_once: AtomicBool,
        calls: AtomicUsize,
    }

    impl fmt::Debug for StaticSourceFactory {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("StaticSourceFactory")
                .field("calls", &self.calls.load(Ordering::SeqCst))
                .finish_non_exhaustive()
        }
    }

    impl StaticSourceFactory {
        fn new(source: Arc<dyn LegacyArchiveReader>) -> Self {
            Self {
                source,
                fail_once: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
            }
        }

        fn open_calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ArchiveSourceFactory for StaticSourceFactory {
        async fn open(&self, _data_dir: &Path) -> std::io::Result<Arc<dyn LegacyArchiveReader>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_once.swap(false, Ordering::SeqCst) {
                return Err(std::io::Error::other("injected source-open failure"));
            }
            Ok(Arc::clone(&self.source))
        }
    }

    #[derive(Default)]
    struct TestLegacyArchiveReader {
        records: StdMutex<HashMap<String, Value>>,
        fail_list_once: AtomicBool,
        fail_get_once: StdMutex<HashSet<String>>,
        miss_get_once: StdMutex<HashSet<String>>,
    }

    impl fmt::Debug for TestLegacyArchiveReader {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("TestLegacyArchiveReader")
                .finish_non_exhaustive()
        }
    }

    impl TestLegacyArchiveReader {
        fn seed(&self, key: impl Into<String>, data: Value) {
            self.records
                .lock()
                .expect("legacy archive records")
                .insert(key.into(), data);
        }

        fn fail_list_once(&self) {
            self.fail_list_once.store(true, Ordering::SeqCst);
        }

        fn fail_get_once(&self, key: &str) {
            self.fail_get_once
                .lock()
                .expect("get faults")
                .insert(key.to_owned());
        }

        fn miss_get_once(&self, key: &str) {
            self.miss_get_once
                .lock()
                .expect("get misses")
                .insert(key.to_owned());
        }
    }

    #[async_trait]
    impl LegacyArchiveReader for TestLegacyArchiveReader {
        async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
            if self.fail_list_once.swap(false, Ordering::SeqCst) {
                return Err(ThreadStoreError::Backend(
                    "injected list failure".to_owned(),
                ));
            }
            let records = self.records.lock().expect("legacy archive records");
            Ok(records
                .keys()
                .filter(|key| prefix.is_none_or(|prefix| key.starts_with(prefix)))
                .cloned()
                .collect())
        }

        async fn get(&self, key: &str) -> Result<Option<Value>, ThreadStoreError> {
            if self.fail_get_once.lock().expect("get faults").remove(key) {
                return Err(ThreadStoreError::Backend("injected get failure".to_owned()));
            }
            if self.miss_get_once.lock().expect("get misses").remove(key) {
                return Ok(None);
            }
            Ok(self
                .records
                .lock()
                .expect("legacy archive records")
                .get(key)
                .cloned())
        }
    }

    struct FailNthSetStore {
        inner: Arc<dyn ThreadStore>,
        fail_on: usize,
        calls: AtomicUsize,
        failed: AtomicBool,
    }

    impl fmt::Debug for FailNthSetStore {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("FailNthSetStore")
                .finish_non_exhaustive()
        }
    }

    struct ArchiveDuringFirstSetStore {
        inner: Arc<dyn ThreadStore>,
        db: Arc<GaryxDbService>,
        transcripts: Arc<ThreadTranscriptStore>,
        fired: AtomicBool,
    }

    impl fmt::Debug for ArchiveDuringFirstSetStore {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("ArchiveDuringFirstSetStore")
                .finish_non_exhaustive()
        }
    }

    #[async_trait]
    impl ThreadStore for ArchiveDuringFirstSetStore {
        async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
            self.inner.get(thread_id).await
        }

        async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
            if !self.fired.swap(true, Ordering::SeqCst) {
                self.db
                    .archive_thread_record(thread_id)
                    .map_err(|error| ThreadStoreError::Backend(error.to_string()))?;
                let path = self
                    .transcripts
                    .transcript_path(thread_id)
                    .expect("file transcript path");
                tokio::fs::remove_file(&path)
                    .await
                    .expect("remove just-backfilled transcript");
                tokio::fs::create_dir(&path)
                    .await
                    .expect("block residual cleanup");
            }
            self.inner.set(thread_id, data).await
        }

        async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.delete(thread_id).await
        }

        async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
            self.inner.list_keys(prefix).await
        }

        async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.exists(thread_id).await
        }

        async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
            self.inner.update(thread_id, updates).await
        }

        fn channel_endpoint_projection(
            &self,
        ) -> Option<Arc<dyn garyx_router::ChannelEndpointProjection>> {
            self.inner.channel_endpoint_projection()
        }

        fn task_projection(&self) -> Option<Arc<dyn garyx_router::tasks::TaskProjectionReader>> {
            self.inner.task_projection()
        }
    }

    struct BlockingListStore {
        inner: TestLegacyArchiveReader,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        blocked: AtomicBool,
    }

    impl fmt::Debug for BlockingListStore {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("BlockingListStore")
                .finish_non_exhaustive()
        }
    }

    #[async_trait]
    impl LegacyArchiveReader for BlockingListStore {
        async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
            if !self.blocked.swap(true, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.list_keys(prefix).await
        }

        async fn get(&self, key: &str) -> Result<Option<Value>, ThreadStoreError> {
            self.inner.get(key).await
        }
    }

    #[async_trait]
    impl ThreadStore for FailNthSetStore {
        async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
            self.inner.get(thread_id).await
        }

        async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.fail_on && !self.failed.swap(true, Ordering::SeqCst) {
                return Err(ThreadStoreError::Backend("injected set failure".to_owned()));
            }
            self.inner.set(thread_id, data).await
        }

        async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.delete(thread_id).await
        }

        async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
            self.inner.list_keys(prefix).await
        }

        async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.exists(thread_id).await
        }

        async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
            self.inner.update(thread_id, updates).await
        }

        fn channel_endpoint_projection(
            &self,
        ) -> Option<Arc<dyn garyx_router::ChannelEndpointProjection>> {
            self.inner.channel_endpoint_projection()
        }

        fn task_projection(&self) -> Option<Arc<dyn garyx_router::tasks::TaskProjectionReader>> {
            self.inner.task_projection()
        }
    }

    fn test_db() -> Arc<GaryxDbService> {
        Arc::new(GaryxDbService::memory().expect("memory db"))
    }

    fn sqlite_target(
        db: &Arc<GaryxDbService>,
        transcripts: &Arc<ThreadTranscriptStore>,
    ) -> Arc<dyn ThreadStore> {
        Arc::new(SqliteThreadStore::new(
            Arc::clone(db),
            Arc::clone(transcripts),
            Arc::new(AlwaysActiveRunProbe),
        ))
    }

    async fn file_transcripts(data_dir: &Path) -> Arc<ThreadTranscriptStore> {
        Arc::new(
            ThreadTranscriptStore::file(data_dir.join("test-transcripts"))
                .await
                .expect("file transcripts"),
        )
    }

    async fn prepare_probe_dir(data_dir: &Path) {
        tokio::fs::create_dir_all(data_dir.join("threads"))
            .await
            .expect("probe directory");
    }

    async fn put(store: &dyn ThreadStore, key: &str, data: Value) {
        store.set(key, data).await.expect("seed source record");
    }

    fn assert_markers_and_generation(db: &GaryxDbService, pair: (bool, bool), generation: i64) {
        assert_eq!(db.legacy_import_marker_pair().expect("marker pair"), pair);
        assert_eq!(
            db.current_legacy_import_generation()
                .expect("import generation"),
            generation
        );
    }

    async fn run_with_fakes(
        db: &Arc<GaryxDbService>,
        target: &Arc<dyn ThreadStore>,
        transcripts: &Arc<ThreadTranscriptStore>,
        data_dir: &Path,
        archive_fs: &dyn ArchiveFs,
        source_factory: &dyn ArchiveSourceFactory,
    ) -> Result<LegacyBootImportOutcome, LegacyBootImportError> {
        run_legacy_boot_import_with(
            db,
            target,
            transcripts,
            data_dir,
            archive_fs,
            source_factory,
        )
        .await
    }

    #[tokio::test]
    async fn lifecycle_complete_reads_both_markers_and_touches_no_archive_fs() {
        let data_dir = TempDir::new().expect("temp data dir");
        let db = test_db();
        db.commit_legacy_import(0, false).expect("import marker");
        db.record_legacy_archive_retirement()
            .expect("retirement marker");
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let source_factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
        let archive_fs = RecordingArchiveFs::default();

        let outcome = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &source_factory,
        )
        .await
        .expect("complete lifecycle");

        assert_eq!(outcome, LegacyBootImportOutcome::Complete);
        assert!(archive_fs.calls().is_empty());
        assert_eq!(source_factory.open_calls(), 0);
    }

    #[tokio::test]
    async fn fresh_install_commits_generation_one_without_creating_archive_dirs() {
        let data_dir = TempDir::new().expect("temp data dir");
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let source_factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
        let archive_fs = RecordingArchiveFs::default();

        let first = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &source_factory,
        )
        .await
        .expect("fresh install");
        assert_eq!(first, LegacyBootImportOutcome::NothingToImport);
        assert_markers_and_generation(&db, (true, true), 1);
        assert!(!data_dir.path().join("threads").exists());
        assert!(!data_dir.path().join("sessions").exists());
        assert_eq!(source_factory.open_calls(), 0);

        let calls_after_first = archive_fs.calls().len();
        let second = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &source_factory,
        )
        .await
        .expect("second boot");
        assert_eq!(second, LegacyBootImportOutcome::Complete);
        assert_eq!(archive_fs.calls().len(), calls_after_first);
    }

    #[tokio::test]
    async fn marker_pair_failures_abort_both_before_and_after_lock() {
        for occurrence in [1, 2] {
            let data_dir = TempDir::new().expect("temp data dir");
            let db = test_db();
            db.fail_test_db_call(TestDbFaultPoint::LegacyMarkerPairRead, occurrence);
            let transcripts = Arc::new(ThreadTranscriptStore::memory());
            let target = sqlite_target(&db, &transcripts);
            let source_factory =
                StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
            let archive_fs = RecordingArchiveFs::default();

            assert!(
                run_with_fakes(
                    &db,
                    &target,
                    &transcripts,
                    data_dir.path(),
                    &archive_fs,
                    &source_factory,
                )
                .await
                .is_err(),
                "marker read {occurrence} must abort"
            );
            let calls = archive_fs.calls();
            if occurrence == 1 {
                assert!(calls.is_empty());
            } else {
                assert_eq!(
                    calls,
                    vec![
                        ArchiveCall::OpenLock(data_dir.path().join(LIFECYCLE_LOCK_FILE)),
                        ArchiveCall::AcquireLock,
                    ]
                );
            }
            assert_eq!(source_factory.open_calls(), 0);
        }
    }

    #[tokio::test]
    async fn import_commit_failure_retries_and_advances_generation_exactly_once() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        db.fail_test_db_call(TestDbFaultPoint::LegacyImportCommit, 1);
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let source = Arc::new(TestLegacyArchiveReader::default());
        source.seed(
            "thread::commit-retry",
            json!({"thread_id": "thread::commit-retry", "label": "legacy"}),
        );
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_err()
        );
        assert_markers_and_generation(&db, (false, false), 0);
        assert!(data_dir.path().join("threads").exists());

        let retry = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect("retry import");
        assert!(matches!(
            retry,
            LegacyBootImportOutcome::ImportedAndRetired(_)
        ));
        assert_markers_and_generation(&db, (true, true), 1);
        assert_eq!(factory.open_calls(), 2);
    }

    #[tokio::test]
    async fn fresh_install_commit_failure_retries_without_false_success() {
        let data_dir = TempDir::new().expect("temp data dir");
        let db = test_db();
        db.fail_test_db_call(TestDbFaultPoint::LegacyImportCommit, 1);
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
        let archive_fs = RecordingArchiveFs::default();

        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_err()
        );
        assert_markers_and_generation(&db, (false, false), 0);
        assert!(!data_dir.path().join("threads").exists());

        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .expect("fresh retry"),
            LegacyBootImportOutcome::NothingToImport
        );
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn retirement_marker_failure_degrades_then_retirement_only_retries() {
        let data_dir = TempDir::new().expect("temp data dir");
        let db = test_db();
        db.fail_test_db_call(TestDbFaultPoint::LegacyRetirementMarkerWrite, 1);
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
        let archive_fs = RecordingArchiveFs::default();

        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .expect("retirement write degrades"),
            LegacyBootImportOutcome::NothingToImport
        );
        assert_markers_and_generation(&db, (true, false), 1);

        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .expect("retirement retry"),
            LegacyBootImportOutcome::RetirementOnly { pending: false }
        );
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn lock_open_and_busy_fail_before_probe_or_source_construction() {
        for (open_failure, acquire_kind) in [(true, None), (false, Some(ErrorKind::WouldBlock))] {
            let data_dir = TempDir::new().expect("temp data dir");
            let db = test_db();
            let transcripts = Arc::new(ThreadTranscriptStore::memory());
            let target = sqlite_target(&db, &transcripts);
            let factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
            let archive_fs = RecordingArchiveFs::default();
            if open_failure {
                archive_fs.fail_open_once();
            }
            if let Some(kind) = acquire_kind {
                archive_fs.fail_acquire_once(kind);
            }

            assert!(
                run_with_fakes(
                    &db,
                    &target,
                    &transcripts,
                    data_dir.path(),
                    &archive_fs,
                    &factory,
                )
                .await
                .is_err()
            );
            let mut expected = vec![ArchiveCall::OpenLock(
                data_dir.path().join(LIFECYCLE_LOCK_FILE),
            )];
            if !open_failure {
                expected.push(ArchiveCall::AcquireLock);
            }
            assert_eq!(archive_fs.calls(), expected);
            assert_eq!(factory.open_calls(), 0);
            assert_markers_and_generation(&db, (false, false), 0);
        }
    }

    #[tokio::test]
    async fn archive_probe_metadata_failure_aborts_before_source_open() {
        let data_dir = TempDir::new().expect("temp data dir");
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
        let archive_fs = RecordingArchiveFs::default();
        archive_fs.fail_exists_once(data_dir.path().join("threads"));

        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_err()
        );
        assert_eq!(factory.open_calls(), 0);
        assert_markers_and_generation(&db, (false, false), 0);
    }

    #[tokio::test]
    async fn file_thread_store_open_failure_preserves_archive_and_retry_completes() {
        let data_dir = TempDir::new().expect("temp data dir");
        tokio::fs::write(data_dir.path().join("threads"), b"not a directory")
            .await
            .expect("poison threads path");
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let archive_fs = RecordingArchiveFs::default();

        assert!(matches!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await,
            Err(LegacyBootImportError::SourceOpen { .. })
        ));
        assert_markers_and_generation(&db, (false, false), 0);
        assert!(data_dir.path().join("threads").is_file());

        tokio::fs::remove_file(data_dir.path().join("threads"))
            .await
            .expect("repair threads path");
        tokio::fs::create_dir(data_dir.path().join("threads"))
            .await
            .expect("restore threads dir");
        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await
            .is_ok()
        );
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn source_list_failure_preserves_archive_then_clean_retry_commits() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let source = Arc::new(TestLegacyArchiveReader::default());
        source.fail_list_once();
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        assert!(matches!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await,
            Err(LegacyBootImportError::SourceList(_))
        ));
        assert_markers_and_generation(&db, (false, false), 0);
        assert!(data_dir.path().join("threads").exists());

        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_ok()
        );
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn per_key_get_error_and_missing_are_failures_then_retry_fully_imports() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let source = Arc::new(TestLegacyArchiveReader::default());
        for key in ["thread::get-error", "thread::get-missing"] {
            source.seed(key, json!({"thread_id": key, "label": key}));
        }
        source.fail_get_once("thread::get-error");
        source.miss_get_once("thread::get-missing");
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        let error = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect_err("get failures abort");
        let LegacyBootImportError::ImportFailed(summary) = error else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(summary.failed, 2);
        assert_eq!(summary.imported, 0);
        assert_markers_and_generation(&db, (false, false), 0);

        let outcome = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect("clean retry");
        let LegacyBootImportOutcome::ImportedAndRetired(summary) = outcome else {
            panic!("unexpected outcome: {outcome:?}");
        };
        assert_eq!(summary.imported, 2);
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn retired_workflow_delete_failure_retries_as_discarded_and_commits() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = file_transcripts(data_dir.path()).await;
        let target = sqlite_target(&db, &transcripts);
        let source = Arc::new(TestLegacyArchiveReader::default());
        let key = "thread::retired-workflow";
        source.seed(
            key,
            json!({"thread_id": key, "thread_kind": "workflow_run"}),
        );
        let transcript_path = transcripts.transcript_path(key).expect("transcript path");
        tokio::fs::create_dir(&transcript_path)
            .await
            .expect("delete-blocking directory");
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        let error = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect_err("delete failure aborts");
        let LegacyBootImportError::ImportFailed(summary) = error else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(summary.failed, 1);
        assert_markers_and_generation(&db, (false, false), 0);

        tokio::fs::remove_dir(&transcript_path)
            .await
            .expect("repair transcript path");
        let outcome = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect("clean retry");
        let LegacyBootImportOutcome::ImportedAndRetired(summary) = outcome else {
            panic!("unexpected outcome: {outcome:?}");
        };
        assert_eq!(summary.discarded, 1);
        assert_eq!(summary.imported, 0);
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn backfill_failure_skips_record_and_retry_lands_transcript_and_record() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = file_transcripts(data_dir.path()).await;
        let target = sqlite_target(&db, &transcripts);
        let source = Arc::new(TestLegacyArchiveReader::default());
        let key = "thread::backfill-retry";
        source.seed(
            key,
            json!({
                "thread_id": key,
                "messages": [{"role": "user", "content": "archive remains authoritative"}],
            }),
        );
        let path = transcripts.transcript_path(key).expect("transcript path");
        tokio::fs::write(&path, b"structurally invalid transcript\n")
            .await
            .expect("invalid transcript");
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_err()
        );
        assert!(target.get(key).await.expect("target read").is_none());
        assert_eq!(
            tokio::fs::read(&path).await.expect("preserved transcript"),
            b"structurally invalid transcript\n"
        );
        assert_markers_and_generation(&db, (false, false), 0);

        tokio::fs::remove_file(&path)
            .await
            .expect("repair transcript");
        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_ok()
        );
        let record = target
            .get(key)
            .await
            .expect("target read")
            .expect("imported record");
        assert!(record.get("messages").is_none());
        assert_eq!(
            transcripts
                .provider_session_tail(key, 10)
                .await
                .expect("transcript tail")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn nth_record_write_failure_full_retry_matches_clean_single_pass() {
        async fn execute(fail_on: Option<usize>) -> (Vec<(String, Value)>, Vec<Vec<Value>>) {
            let data_dir = TempDir::new().expect("temp data dir");
            prepare_probe_dir(data_dir.path()).await;
            let db = test_db();
            let transcripts = Arc::new(ThreadTranscriptStore::memory());
            let base_target = sqlite_target(&db, &transcripts);
            let target: Arc<dyn ThreadStore> = match fail_on {
                Some(fail_on) => Arc::new(FailNthSetStore {
                    inner: Arc::clone(&base_target),
                    fail_on,
                    calls: AtomicUsize::new(0),
                    failed: AtomicBool::new(false),
                }),
                None => Arc::clone(&base_target),
            };
            let source = Arc::new(TestLegacyArchiveReader::default());
            for index in 0..3 {
                let key = format!("thread::batch-{index}");
                source.seed(
                    &key,
                    json!({
                        "thread_id": key,
                        "updated_at": "2026-07-01T00:00:00Z",
                        "messages": [{"role": "user", "content": format!("message-{index}")}],
                    }),
                );
            }
            let factory = StaticSourceFactory::new(source);
            let archive_fs = RecordingArchiveFs::default();
            if fail_on.is_some() {
                assert!(
                    run_with_fakes(
                        &db,
                        &target,
                        &transcripts,
                        data_dir.path(),
                        &archive_fs,
                        &factory,
                    )
                    .await
                    .is_err()
                );
                assert_markers_and_generation(&db, (false, false), 0);
                assert!(data_dir.path().join("threads").exists());
            }
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .expect("completed import");
            assert_markers_and_generation(&db, (true, true), 1);

            let mut keys = base_target.list_keys(None).await.expect("target keys");
            keys.sort();
            let mut records = Vec::new();
            let mut message_sets = Vec::new();
            for key in keys {
                records.push((
                    key.clone(),
                    base_target
                        .get(&key)
                        .await
                        .expect("target read")
                        .expect("record"),
                ));
                message_sets.push(
                    transcripts
                        .provider_session_tail(&key, 10)
                        .await
                        .expect("transcript"),
                );
            }
            (records, message_sets)
        }

        let retried = execute(Some(2)).await;
        let clean = execute(None).await;
        assert_eq!(retried, clean);
    }

    #[tokio::test]
    async fn tombstone_precheck_deletes_orphan_and_discards_with_or_without_transcript() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = file_transcripts(data_dir.path()).await;
        let target = sqlite_target(&db, &transcripts);
        let source = Arc::new(TestLegacyArchiveReader::default());
        let orphan = "thread::archived-orphan";
        let no_transcript = "thread::archived-without-transcript";
        for key in [orphan, no_transcript] {
            source.seed(
                key,
                json!({
                    "thread_id": key,
                    "messages": [{"role": "user", "content": "must not resurrect"}],
                }),
            );
            assert!(!db.archive_thread_record(key).expect("seed tombstone"));
        }
        transcripts
            .append_committed_messages(
                orphan,
                Some("run-orphan"),
                &[json!({"role": "user", "content": "orphan transcript"})],
            )
            .await
            .expect("seed orphan transcript");
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        let outcome = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect("tombstone import");
        let LegacyBootImportOutcome::ImportedAndRetired(summary) = outcome else {
            panic!("unexpected outcome: {outcome:?}");
        };
        assert_eq!(summary.discarded, 2);
        assert_eq!(summary.imported, 0);
        for key in [orphan, no_transcript] {
            assert!(target.get(key).await.expect("target read").is_none());
            assert!(!transcripts.exists(key).await);
            assert!(db.is_thread_archived(key).expect("tombstone remains"));
        }
        assert!(
            db.list_recent_threads(20, 0)
                .expect("recent rows")
                .iter()
                .all(|row| row.thread_id != orphan && row.thread_id != no_transcript)
        );
        assert!(
            db.list_thread_meta()
                .expect("meta rows")
                .iter()
                .all(|row| row.thread_id != orphan && row.thread_id != no_transcript)
        );
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn tombstone_read_and_transcript_delete_failures_abort_then_retry_cleanly() {
        async fn run_case(fail_read: bool) {
            let data_dir = TempDir::new().expect("temp data dir");
            prepare_probe_dir(data_dir.path()).await;
            let db = test_db();
            let transcripts = file_transcripts(data_dir.path()).await;
            let target = sqlite_target(&db, &transcripts);
            let source = Arc::new(TestLegacyArchiveReader::default());
            let key = if fail_read {
                "thread::tombstone-read-error"
            } else {
                "thread::tombstone-delete-error"
            };
            source.seed(
                key,
                json!({"thread_id": key, "messages": [{"role": "user", "content": "dead"}]}),
            );
            db.archive_thread_record(key).expect("seed tombstone");
            let path = transcripts.transcript_path(key).expect("transcript path");
            if fail_read {
                transcripts
                    .append_committed_messages(
                        key,
                        None,
                        &[json!({"role": "user", "content": "orphan"})],
                    )
                    .await
                    .expect("orphan transcript");
                db.fail_test_db_call(TestDbFaultPoint::ArchivedThreadRead, 1);
            } else {
                tokio::fs::create_dir(&path)
                    .await
                    .expect("delete-blocking directory");
            }
            let factory = StaticSourceFactory::new(source);
            let archive_fs = RecordingArchiveFs::default();

            assert!(
                run_with_fakes(
                    &db,
                    &target,
                    &transcripts,
                    data_dir.path(),
                    &archive_fs,
                    &factory,
                )
                .await
                .is_err()
            );
            assert_markers_and_generation(&db, (false, false), 0);
            assert!(data_dir.path().join("threads").exists());
            assert!(target.get(key).await.expect("target read").is_none());

            if !fail_read {
                tokio::fs::remove_dir(&path)
                    .await
                    .expect("repair transcript path");
            }
            let outcome = run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .expect("clean retry");
            let LegacyBootImportOutcome::ImportedAndRetired(summary) = outcome else {
                panic!("unexpected outcome: {outcome:?}");
            };
            assert_eq!(summary.discarded, 1);
            assert!(!transcripts.exists(key).await);
            assert_markers_and_generation(&db, (true, true), 1);
        }

        run_case(true).await;
        run_case(false).await;
    }

    #[tokio::test]
    async fn residual_archived_write_cleanup_failure_retries_via_tombstone_precheck() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = file_transcripts(data_dir.path()).await;
        let base_target = sqlite_target(&db, &transcripts);
        let target: Arc<dyn ThreadStore> = Arc::new(ArchiveDuringFirstSetStore {
            inner: Arc::clone(&base_target),
            db: Arc::clone(&db),
            transcripts: Arc::clone(&transcripts),
            fired: AtomicBool::new(false),
        });
        let source = Arc::new(TestLegacyArchiveReader::default());
        let key = "thread::residual-archive-race";
        source.seed(
            key,
            json!({
                "thread_id": key,
                "messages": [{"role": "user", "content": "backfilled before tombstone race"}],
            }),
        );
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        let error = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect_err("residual cleanup failure aborts");
        let LegacyBootImportError::ImportFailed(summary) = error else {
            panic!("unexpected error: {error}");
        };
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.discarded, 0);
        assert!(db.is_thread_archived(key).expect("race tombstone"));
        assert_markers_and_generation(&db, (false, false), 0);

        let path = transcripts.transcript_path(key).expect("transcript path");
        tokio::fs::remove_dir(&path)
            .await
            .expect("repair cleanup path");
        tokio::fs::write(&path, b"leftover transcript")
            .await
            .expect("recreate orphan for precheck retry");
        let outcome = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect("precheck retry");
        let LegacyBootImportOutcome::ImportedAndRetired(summary) = outcome else {
            panic!("unexpected outcome: {outcome:?}");
        };
        assert_eq!(summary.discarded, 1);
        assert!(base_target.get(key).await.expect("target read").is_none());
        assert!(!transcripts.exists(key).await);
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn full_success_retires_both_directories_and_reboot_is_fs_free() {
        let data_dir = TempDir::new().expect("temp data dir");
        let source = FileThreadStore::new(data_dir.path())
            .await
            .expect("legacy file store");
        put(
            &source,
            "thread::retire-success",
            json!({"thread_id": "thread::retire-success", "label": "legacy"}),
        )
        .await;
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let archive_fs = RecordingArchiveFs::default();

        let outcome = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &FileArchiveSourceFactory,
        )
        .await
        .expect("full import");
        let LegacyBootImportOutcome::ImportedAndRetired(summary) = outcome else {
            panic!("unexpected outcome: {outcome:?}");
        };
        assert_eq!(summary.imported, 1);
        assert_markers_and_generation(&db, (true, true), 1);
        let backup = data_dir.path().join("backups").join(RETIREMENT_BACKUP_DIR);
        assert!(backup.join("threads").is_dir());
        assert!(backup.join("sessions").is_dir());
        assert!(!data_dir.path().join("threads").exists());
        assert!(!data_dir.path().join("sessions").exists());

        let calls_after_first = archive_fs.calls().len();
        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await
            .expect("complete reboot"),
            LegacyBootImportOutcome::Complete
        );
        assert_eq!(archive_fs.calls().len(), calls_after_first);
        assert!(!data_dir.path().join("threads").exists());
        assert!(!data_dir.path().join("sessions").exists());
    }

    #[tokio::test]
    async fn existing_machine_is_retirement_only_and_preserves_evolved_sqlite_truth() {
        let data_dir = TempDir::new().expect("temp data dir");
        let source = FileThreadStore::new(data_dir.path())
            .await
            .expect("legacy file store");
        let key = "thread::evolved";
        put(
            &source,
            key,
            json!({"thread_id": key, "label": "stale archive"}),
        )
        .await;
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        put(
            target.as_ref(),
            key,
            json!({"thread_id": key, "label": "evolved sqlite"}),
        )
        .await;
        db.commit_legacy_import(1, false).expect("pre-v4 marker");
        db.run_thread_data_startup_migrations()
            .expect("seed generation-one cutovers");
        let archive_fs = RecordingArchiveFs::default();

        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await
            .expect("retirement-only boot"),
            LegacyBootImportOutcome::RetirementOnly { pending: false }
        );
        assert_eq!(
            target
                .get(key)
                .await
                .expect("target read")
                .expect("evolved record")["label"],
            "evolved sqlite"
        );
        assert_markers_and_generation(&db, (true, true), 1);
        assert!(
            db.migrate_recent_task_thread_kind_v1()
                .expect("task cutover gate")
                .already_completed
        );
        assert!(
            db.migrate_endpoint_holder_dedup_v1()
                .expect("endpoint cutover gate")
                .already_completed
        );
    }

    #[tokio::test]
    async fn partial_retirement_moves_first_dir_then_retries_only_remainder() {
        let data_dir = TempDir::new().expect("temp data dir");
        FileThreadStore::new(data_dir.path())
            .await
            .expect("legacy file store");
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let archive_fs = RecordingArchiveFs::default();
        archive_fs.fail_rename_once(data_dir.path().join("sessions"));

        assert!(matches!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await
            .expect("pending retirement"),
            LegacyBootImportOutcome::ImportedRetirementPending(_)
        ));
        assert_markers_and_generation(&db, (true, false), 1);
        let backup = data_dir.path().join("backups").join(RETIREMENT_BACKUP_DIR);
        assert!(backup.join("threads").exists());
        assert!(!data_dir.path().join("threads").exists());
        assert!(data_dir.path().join("sessions").exists());

        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await
            .expect("retirement retry"),
            LegacyBootImportOutcome::RetirementOnly { pending: false }
        );
        assert_markers_and_generation(&db, (true, true), 1);
        assert!(backup.join("threads").exists());
        assert!(backup.join("sessions").exists());
        let thread_moves = archive_fs
            .calls()
            .into_iter()
            .filter(|call| {
                matches!(call, ArchiveCall::Rename { source, .. } if source.ends_with("threads"))
            })
            .count();
        assert_eq!(
            thread_moves, 1,
            "the already-moved directory is not retried"
        );
    }

    #[tokio::test]
    async fn retirement_destination_conflict_preserves_both_trees_without_merge() {
        let data_dir = TempDir::new().expect("temp data dir");
        let source = FileThreadStore::new(data_dir.path())
            .await
            .expect("legacy file store");
        put(
            &source,
            "thread::conflict",
            json!({"thread_id": "thread::conflict", "label": "source"}),
        )
        .await;
        let backup = data_dir.path().join("backups").join(RETIREMENT_BACKUP_DIR);
        for name in ARCHIVE_DIR_NAMES {
            tokio::fs::create_dir_all(backup.join(name))
                .await
                .expect("conflict destination");
            tokio::fs::write(backup.join(name).join("sentinel"), name.as_bytes())
                .await
                .expect("destination sentinel");
        }
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let archive_fs = RecordingArchiveFs::default();

        assert!(matches!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await
            .expect("conflict degrades"),
            LegacyBootImportOutcome::ImportedRetirementPending(_)
        ));
        assert_markers_and_generation(&db, (true, false), 1);
        for name in ARCHIVE_DIR_NAMES {
            assert!(data_dir.path().join(name).exists());
            assert_eq!(
                tokio::fs::read(backup.join(name).join("sentinel"))
                    .await
                    .expect("preserved sentinel"),
                name.as_bytes()
            );
        }
        assert!(
            archive_fs
                .calls()
                .iter()
                .all(|call| !matches!(call, ArchiveCall::Rename { .. }))
        );
    }

    #[tokio::test]
    async fn recovery_missing_archive_aborts_with_sticky_intent_and_untouched_sqlite() {
        let data_dir = TempDir::new().expect("temp data dir");
        let db = test_db();
        db.commit_legacy_import(0, false).expect("generation one");
        db.record_legacy_archive_retirement()
            .expect("retirement marker");
        db.clear_projection_state(THREAD_RECORDS_IMPORT_NAME)
            .expect("clear import marker");
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        put(
            target.as_ref(),
            "thread::sqlite-sentinel",
            json!({"thread_id": "thread::sqlite-sentinel", "label": "untouched"}),
        )
        .await;
        let factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
        let archive_fs = RecordingArchiveFs::default();

        assert!(matches!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await,
            Err(LegacyBootImportError::RecoveryArchiveMissing(_))
        ));
        assert_markers_and_generation(&db, (false, true), 1);
        assert_eq!(
            target
                .get("thread::sqlite-sentinel")
                .await
                .expect("target read")
                .expect("sentinel")["label"],
            "untouched"
        );
        assert_eq!(factory.open_calls(), 0);
    }

    #[tokio::test]
    async fn recovery_partial_restore_shapes_abort_without_state_or_directory_changes() {
        enum Shape {
            ThreadsOnly,
            SessionsOnly,
            Copied,
        }

        for shape in [Shape::ThreadsOnly, Shape::SessionsOnly, Shape::Copied] {
            let data_dir = TempDir::new().expect("temp data dir");
            let backup = data_dir.path().join("backups").join(RETIREMENT_BACKUP_DIR);
            match shape {
                Shape::ThreadsOnly => {
                    tokio::fs::create_dir_all(data_dir.path().join("threads"))
                        .await
                        .expect("restored threads");
                    tokio::fs::create_dir_all(backup.join("sessions"))
                        .await
                        .expect("unrestored sessions");
                }
                Shape::SessionsOnly => {
                    tokio::fs::create_dir_all(data_dir.path().join("sessions"))
                        .await
                        .expect("restored sessions");
                    tokio::fs::create_dir_all(backup.join("threads"))
                        .await
                        .expect("unrestored threads");
                }
                Shape::Copied => {
                    for name in ARCHIVE_DIR_NAMES {
                        tokio::fs::create_dir_all(data_dir.path().join(name))
                            .await
                            .expect("copied source");
                        tokio::fs::create_dir_all(backup.join(name))
                            .await
                            .expect("copy destination");
                    }
                }
            }
            let before = ARCHIVE_DIR_NAMES.map(|name| {
                (
                    data_dir.path().join(name).exists(),
                    backup.join(name).exists(),
                )
            });
            let db = test_db();
            db.commit_legacy_import(0, false).expect("generation one");
            db.record_legacy_archive_retirement()
                .expect("retirement marker");
            db.clear_projection_state(THREAD_RECORDS_IMPORT_NAME)
                .expect("clear import marker");
            let transcripts = Arc::new(ThreadTranscriptStore::memory());
            let target = sqlite_target(&db, &transcripts);
            put(
                target.as_ref(),
                "thread::sentinel",
                json!({"thread_id": "thread::sentinel", "label": "untouched"}),
            )
            .await;
            let factory = StaticSourceFactory::new(Arc::new(TestLegacyArchiveReader::default()));
            let archive_fs = RecordingArchiveFs::default();

            assert!(matches!(
                run_with_fakes(
                    &db,
                    &target,
                    &transcripts,
                    data_dir.path(),
                    &archive_fs,
                    &factory,
                )
                .await,
                Err(LegacyBootImportError::RecoveryArchiveIncomplete(_))
            ));
            assert_markers_and_generation(&db, (false, true), 1);
            assert_eq!(
                target
                    .get("thread::sentinel")
                    .await
                    .expect("target read")
                    .expect("sentinel")["label"],
                "untouched"
            );
            assert_eq!(factory.open_calls(), 0);
            let after = ARCHIVE_DIR_NAMES.map(|name| {
                (
                    data_dir.path().join(name).exists(),
                    backup.join(name).exists(),
                )
            });
            assert_eq!(after, before, "probe must not create or move directories");
        }
    }

    #[tokio::test]
    async fn recovery_commit_crash_preserves_intent_then_retry_advances_once() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        db.commit_legacy_import(0, false).expect("generation one");
        db.record_legacy_archive_retirement()
            .expect("retirement marker");
        db.clear_projection_state(THREAD_RECORDS_IMPORT_NAME)
            .expect("recovery intent");
        db.fail_test_db_call(TestDbFaultPoint::LegacyImportCommit, 2);
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let source = Arc::new(TestLegacyArchiveReader::default());
        source.seed(
            "thread::recovery-crash",
            json!({"thread_id": "thread::recovery-crash", "label": "restored"}),
        );
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_err()
        );
        assert_markers_and_generation(&db, (false, true), 1);
        assert!(data_dir.path().join("threads").exists());

        assert!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &factory,
            )
            .await
            .is_ok()
        );
        assert_markers_and_generation(&db, (true, true), 2);
    }

    #[tokio::test]
    async fn pre_v4_upgrade_then_manual_recovery_reruns_cutovers_at_generation_two() {
        let data_dir = TempDir::new().expect("temp data dir");
        let task_key = "thread::recovered-legacy-task";
        let endpoint_loser = "thread::endpoint-loser";
        let endpoint_winner = "thread::endpoint-winner";
        let archived_key = "thread::archived-stays-dead";
        let duplicate_binding = json!({
            "channel": "telegram",
            "account_id": "main",
            "binding_key": "synthetic-chat",
            "chat_id": "synthetic-chat",
            "delivery_target_type": "chat_id",
            "delivery_target_id": "synthetic-chat",
            "display_label": "Synthetic chat",
            "last_inbound_at": "2026-07-01T00:00:00Z",
        });

        {
            let archive = FileThreadStore::new(data_dir.path())
                .await
                .expect("legacy archive");
            put(
                &archive,
                task_key,
                json!({
                    "thread_id": task_key,
                    "thread_title_source": "task",
                    "task": {
                        "number": 77,
                        "title": "Recovered legacy task",
                        "status": "done",
                    },
                    "messages": [{"role": "user", "content": "restored task body"}],
                    "updated_at": "2026-07-01T00:00:00Z",
                }),
            )
            .await;
            put(
                &archive,
                endpoint_loser,
                json!({
                    "thread_id": endpoint_loser,
                    "label": "Older endpoint holder",
                    "updated_at": "2026-07-01T00:00:00Z",
                    "channel_bindings": [duplicate_binding.clone()],
                }),
            )
            .await;
            put(
                &archive,
                endpoint_winner,
                json!({
                    "thread_id": endpoint_winner,
                    "label": "Newer endpoint holder",
                    "updated_at": "2026-07-02T00:00:00Z",
                    "channel_bindings": [duplicate_binding],
                }),
            )
            .await;
            put(
                &archive,
                archived_key,
                json!({
                    "thread_id": archived_key,
                    "messages": [{"role": "user", "content": "must remain archived"}],
                    "updated_at": "2026-07-03T00:00:00Z",
                }),
            )
            .await;
        }

        let db = test_db();
        let transcripts = file_transcripts(data_dir.path()).await;
        let target = sqlite_target(&db, &transcripts);
        put(
            target.as_ref(),
            task_key,
            json!({
                "thread_id": task_key,
                "thread_title_source": "task",
                "task": {
                    "number": 77,
                    "title": "Pre-v4 SQLite task",
                    "status": "done",
                },
                "updated_at": "2026-07-04T00:00:00Z",
            }),
        )
        .await;
        put(
            target.as_ref(),
            archived_key,
            json!({"thread_id": archived_key, "label": "removed before recovery"}),
        )
        .await;
        assert!(
            db.archive_thread_record(archived_key)
                .expect("archive target")
        );
        transcripts
            .append_committed_messages(
                archived_key,
                None,
                &[json!({"role": "user", "content": "orphan after best-effort delete"})],
            )
            .await
            .expect("seed archived orphan transcript");

        db.record_projection_state(THREAD_RECORDS_IMPORT_NAME, THREAD_RECORDS_IMPORT_VERSION, 4)
            .expect("pre-v4 import marker");
        db.record_projection_state(
            crate::garyx_db::RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
            1,
            1,
        )
        .expect("pre-v4 task cutover marker");
        db.record_projection_state(crate::garyx_db::ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME, 1, 0)
            .expect("pre-v4 endpoint cutover marker");
        assert_eq!(db.legacy_import_marker_pair().unwrap(), (true, false));

        let archive_fs = RecordingArchiveFs::default();
        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                data_dir.path(),
                &archive_fs,
                &FileArchiveSourceFactory,
            )
            .await
            .expect("pre-v4 retirement-only boot"),
            LegacyBootImportOutcome::RetirementOnly { pending: false }
        );
        assert!(
            db.migrate_recent_task_thread_kind_v1()
                .expect("compatible task marker")
                .already_completed,
            "generation-one compatibility must pin the old marker"
        );
        assert!(
            db.migrate_endpoint_holder_dedup_v1()
                .expect("compatible endpoint marker")
                .already_completed,
            "generation-one compatibility must pin the old marker"
        );
        assert_markers_and_generation(&db, (true, true), 1);
        assert!(
            target
                .get(task_key)
                .await
                .expect("pre-recovery task read")
                .expect("pre-v4 task")
                .get("thread_kind")
                .is_none(),
            "pre-v4 cutover marker must not rerun at generation one"
        );

        let backup = data_dir.path().join("backups").join(RETIREMENT_BACKUP_DIR);
        for name in ARCHIVE_DIR_NAMES {
            tokio::fs::rename(backup.join(name), data_dir.path().join(name))
                .await
                .expect("move archive directory back for recovery");
        }
        db.clear_projection_state(THREAD_RECORDS_IMPORT_NAME)
            .expect("declare recovery intent");
        assert_markers_and_generation(&db, (false, true), 1);

        let recovery = run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &FileArchiveSourceFactory,
        )
        .await
        .expect("recovery import");
        let LegacyBootImportOutcome::ImportedAndRetired(summary) = recovery else {
            panic!("unexpected recovery outcome: {recovery:?}");
        };
        assert_eq!(summary.source_keys, 4);
        assert_eq!(summary.imported, 3);
        assert_eq!(summary.discarded, 1);
        assert_markers_and_generation(&db, (true, true), 2);
        assert!(
            target
                .get(task_key)
                .await
                .expect("reimported task read")
                .expect("reimported task")
                .get("thread_kind")
                .is_none(),
            "import must finish before the generation-two cutover"
        );
        assert_eq!(
            target
                .get(endpoint_loser)
                .await
                .expect("loser before cutover")
                .expect("loser record")["channel_bindings"]
                .as_array()
                .expect("loser bindings")
                .len(),
            1
        );

        let task_cutover = db
            .migrate_recent_task_thread_kind_v1()
            .expect("generation-two task cutover");
        let endpoint_cutover = db
            .migrate_endpoint_holder_dedup_v1()
            .expect("generation-two endpoint cutover");
        assert!(!task_cutover.already_completed);
        assert!(task_cutover.updated_row_count >= 1);
        assert!(!endpoint_cutover.already_completed);
        assert!(endpoint_cutover.updated_row_count >= 1);
        assert_eq!(
            target
                .get(task_key)
                .await
                .expect("task after cutover")
                .expect("task record")["thread_kind"],
            "task"
        );
        assert_eq!(
            db.list_recent_threads(20, 0)
                .expect("recent rows")
                .into_iter()
                .find(|row| row.thread_id == task_key)
                .expect("task recent projection")
                .thread_type,
            "task"
        );
        assert_eq!(
            db.list_thread_meta()
                .expect("meta rows")
                .into_iter()
                .find(|row| row.thread_id == task_key)
                .expect("task meta projection")
                .thread_type,
            "task"
        );
        assert_eq!(
            target
                .get(endpoint_loser)
                .await
                .expect("loser after cutover")
                .expect("loser record")["channel_bindings"]
                .as_array()
                .expect("loser bindings")
                .len(),
            0
        );
        assert_eq!(
            target
                .get(endpoint_winner)
                .await
                .expect("winner after cutover")
                .expect("winner record")["channel_bindings"]
                .as_array()
                .expect("winner bindings")
                .len(),
            1
        );
        assert!(
            target
                .get(archived_key)
                .await
                .expect("archived read")
                .is_none()
        );
        assert!(!transcripts.exists(archived_key).await);
        assert!(db.is_thread_archived(archived_key).expect("tombstone"));
        assert!(
            db.list_recent_threads(20, 0)
                .expect("recent rows")
                .iter()
                .all(|row| row.thread_id != archived_key)
        );
        assert!(
            db.list_thread_meta()
                .expect("meta rows")
                .iter()
                .all(|row| row.thread_id != archived_key)
        );
        assert!(backup.join("threads").exists());
        assert!(backup.join("sessions").exists());
        assert!(!data_dir.path().join("threads").exists());
        assert!(!data_dir.path().join("sessions").exists());
    }

    #[tokio::test]
    async fn concurrent_second_boot_is_lock_busy_then_retry_observes_complete() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let target = sqlite_target(&db, &transcripts);
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let source = Arc::new(BlockingListStore {
            inner: TestLegacyArchiveReader::default(),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            blocked: AtomicBool::new(false),
        });
        source.inner.seed(
            "thread::concurrent",
            json!({"thread_id": "thread::concurrent", "label": "legacy"}),
        );
        let factory = Arc::new(StaticSourceFactory::new(source));
        let archive_fs = Arc::new(RecordingArchiveFs::default());
        let data_path = data_dir.path().to_path_buf();

        let first = tokio::spawn({
            let db = Arc::clone(&db);
            let target = Arc::clone(&target);
            let transcripts = Arc::clone(&transcripts);
            let factory = Arc::clone(&factory);
            let archive_fs = Arc::clone(&archive_fs);
            let data_path = data_path.clone();
            async move {
                run_with_fakes(
                    &db,
                    &target,
                    &transcripts,
                    &data_path,
                    archive_fs.as_ref(),
                    factory.as_ref(),
                )
                .await
            }
        });
        entered.notified().await;

        assert!(matches!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                &data_path,
                archive_fs.as_ref(),
                factory.as_ref(),
            )
            .await,
            Err(LegacyBootImportError::LockBusy(_))
        ));
        release.notify_one();
        assert!(first.await.expect("first boot task").is_ok());
        assert_eq!(
            run_with_fakes(
                &db,
                &target,
                &transcripts,
                &data_path,
                archive_fs.as_ref(),
                factory.as_ref(),
            )
            .await
            .expect("post-completion retry"),
            LegacyBootImportOutcome::Complete
        );
        assert_markers_and_generation(&db, (true, true), 1);
    }

    #[tokio::test]
    async fn assembly_migrates_task_kind_only_after_boot_import() {
        let data_dir = TempDir::new().expect("temp data dir");
        prepare_probe_dir(data_dir.path()).await;
        let db = test_db();
        let transcripts = Arc::new(ThreadTranscriptStore::memory());
        let bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
        let target =
            crate::assemble_sqlite_thread_store(Arc::clone(&db), Arc::clone(&transcripts), &bridge)
                .expect("pure sqlite constructor");
        let key = "thread::legacy-task-order";
        let source = Arc::new(TestLegacyArchiveReader::default());
        source.seed(
            key,
            json!({
                "thread_id": key,
                "thread_title_source": "task",
                "task": {"number": 42, "title": "Imported legacy task", "status": "done"},
                "updated_at": "2026-07-01T00:00:00Z",
            }),
        );
        let factory = StaticSourceFactory::new(source);
        let archive_fs = RecordingArchiveFs::default();

        assert!(target.get(key).await.expect("pre-import read").is_none());
        assert!(
            !db.projection_state_exists(
                crate::garyx_db::RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                1,
            )
            .expect("pre-import cutover marker")
        );
        run_with_fakes(
            &db,
            &target,
            &transcripts,
            data_dir.path(),
            &archive_fs,
            &factory,
        )
        .await
        .expect("boot import");
        let imported = target
            .get(key)
            .await
            .expect("post-import read")
            .expect("imported task");
        assert!(imported.get("thread_kind").is_none());
        assert!(
            !db.projection_state_exists(
                crate::garyx_db::RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                1,
            )
            .expect("cutover still pending")
        );

        db.run_thread_data_startup_migrations()
            .expect("post-import cutovers");
        assert_eq!(
            target
                .get(key)
                .await
                .expect("post-cutover read")
                .expect("task record")["thread_kind"],
            "task"
        );
        assert_eq!(
            db.list_recent_threads(20, 0)
                .expect("recent rows")
                .into_iter()
                .find(|row| row.thread_id == key)
                .expect("recent task")
                .thread_type,
            "task"
        );
        assert_eq!(
            db.list_thread_meta()
                .expect("meta rows")
                .into_iter()
                .find(|row| row.thread_id == key)
                .expect("meta task")
                .thread_type,
            "task"
        );
    }
}
