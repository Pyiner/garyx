mod index;
mod log;
mod read;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use garyx_models::local_paths::default_meetings_dir;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::garyx_db::{
    DeleteMeetingRowOutcome, GaryxDbError, GaryxDbService, MeetingRecord, normalize_meeting_id,
};
use crate::server::AppState;

pub use log::{MeetingSegment, SegmentDraft, SegmentKind, share_text};
pub use read::{
    ConfirmMeetingReadRequest, MeetingReadMeta, MeetingReadMode, MeetingReadRequest,
    MeetingReadResponse, continuation_mode_unverified,
};

use index::{OffsetIndex, load_index, persist_index};
use log::{LogScan, append_lines_and_sync, checkpoint_line, normalize_page, scan_log};

pub const DEFAULT_READ_PAGE_BYTES: usize = 65_536;
pub const MIN_READ_PAGE_BYTES: usize = 4_096;

#[derive(Debug)]
pub struct MeetingError {
    status: StatusCode,
    code: String,
    message: String,
    restart_command: Option<String>,
}

impl MeetingError {
    pub(crate) fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            restart_command: None,
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub(crate) fn named_bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub(crate) fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "entity_deleted", "entity deleted")
    }

    pub(crate) fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub(crate) fn content_loss() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "snapshot_invalidated_by_content_loss",
            "snapshot invalidated by content loss",
        )
    }

    pub(crate) fn content_lost() -> Self {
        Self::new(StatusCode::GONE, "content_lost", "meeting content is lost")
    }

    pub(crate) fn index_building() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "index_building",
            "meeting offset index is building; retry",
        )
    }

    pub(crate) fn storage(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "storage_error", message)
    }

    pub(crate) fn io(context: &str, error: std::io::Error) -> Self {
        Self::storage(format!("{context}: {error}"))
    }

    pub(crate) fn injected(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "injected_storage_fault",
            message,
        )
    }

    pub(crate) fn with_restart_command(mut self, command: String) -> Self {
        self.restart_command = Some(command);
        self
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn restart_command(&self) -> Option<&str> {
        self.restart_command.as_deref()
    }

    fn envelope(&self) -> Value {
        let mut error = serde_json::Map::new();
        error.insert("code".to_owned(), Value::String(self.code.clone()));
        error.insert("message".to_owned(), Value::String(self.message.clone()));
        if let Some(command) = &self.restart_command {
            error.insert("restart_command".to_owned(), Value::String(command.clone()));
        }
        json!({ "error": error })
    }
}

impl fmt::Display for MeetingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MeetingError {}

impl IntoResponse for MeetingError {
    fn into_response(self) -> Response {
        json_response(self.status, &self.envelope())
    }
}

impl From<serde_json::Error> for MeetingError {
    fn from(error: serde_json::Error) -> Self {
        Self::storage(format!("meeting JSON error: {error}"))
    }
}

impl From<GaryxDbError> for MeetingError {
    fn from(error: GaryxDbError) -> Self {
        match error {
            GaryxDbError::BadRequest(message) => Self::bad_request(message),
            GaryxDbError::ThreadArchived(thread_id) => {
                Self::bad_request(format!("thread is archived: {thread_id}"))
            }
            GaryxDbError::LockPoisoned
            | GaryxDbError::Join(_)
            | GaryxDbError::Configuration(_)
            | GaryxDbError::Io(_)
            | GaryxDbError::Sqlite(_)
            | GaryxDbError::DataDirLocked { .. }
            | GaryxDbError::ParentHandoffTimedOut { .. } => Self::storage(error.to_string()),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct EntityIndexState {
    pub index: Option<OffsetIndex>,
    pub building: bool,
}

#[derive(Debug)]
pub(crate) struct EntityIo {
    pub lock: Arc<RwLock<()>>,
    pub index: Mutex<EntityIndexState>,
}

impl Default for EntityIo {
    fn default() -> Self {
        Self {
            lock: Arc::new(RwLock::new(())),
            index: Mutex::new(EntityIndexState::default()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendPageOutcome {
    pub log_epoch: i64,
    pub generation: i64,
    pub latest_seq: i64,
    pub byte_len: u64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppendTestFault {
    Segment(usize),
    CheckpointBeforeSync,
    SyncBeforeCache,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteTestFault {
    AfterRename,
    AfterDatabaseCommit,
}

pub struct MeetingService {
    pub(crate) db: Arc<GaryxDbService>,
    root: PathBuf,
    read_page_bytes: AtomicUsize,
    entities: Mutex<HashMap<String, Arc<EntityIo>>>,
}

impl MeetingService {
    pub fn new(
        db: Arc<GaryxDbService>,
        root: PathBuf,
        read_page_bytes: usize,
    ) -> Result<Self, MeetingError> {
        let service = Self {
            db,
            root,
            read_page_bytes: AtomicUsize::new(read_page_bytes.max(MIN_READ_PAGE_BYTES)),
            entities: Mutex::new(HashMap::new()),
        };
        service.boot_repair()?;
        Ok(service)
    }

    pub fn production(
        db: Arc<GaryxDbService>,
        read_page_bytes: usize,
    ) -> Result<Self, MeetingError> {
        Self::new(db, default_meetings_dir(), read_page_bytes)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read_page_bytes(&self) -> usize {
        self.read_page_bytes.load(Ordering::Acquire)
    }

    pub fn set_read_page_bytes(&self, read_page_bytes: usize) {
        self.read_page_bytes
            .store(read_page_bytes.max(MIN_READ_PAGE_BYTES), Ordering::Release);
    }

    pub(crate) fn entity_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub(crate) fn log_path(&self, id: &str) -> PathBuf {
        self.entity_dir(id).join("segments.jsonl")
    }

    pub(crate) fn entity_state(&self, id: &str) -> Arc<EntityIo> {
        let mut entities = self
            .entities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entities
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(EntityIo::default()))
            .clone()
    }

    pub fn boot_repair(&self) -> Result<(), MeetingError> {
        self.reconcile_tombstones()?;
        let records = self.db.list_non_terminal_meetings()?;
        for original in records {
            let state = self.entity_state(&original.id);
            let path = self.log_path(&original.id);
            let mut record = original;
            if !path.exists() {
                if record.cache_generation > 0 {
                    warn!(
                        meeting_id = %record.id,
                        epoch = record.log_epoch,
                        generation = record.cache_generation,
                        "meeting log is missing; rolling the read domain to a new epoch"
                    );
                    record = self
                        .db
                        .rollover_missing_meeting_log(&record.id, record.log_epoch)?
                        .ok_or_else(MeetingError::not_found)?;
                }
                let mut index = state
                    .index
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                index.index = Some(OffsetIndex::from_scan(&LogScan::empty(record.log_epoch)));
                continue;
            }
            let scan = scan_log(&path, record.log_epoch, true)?;
            if scan.had_invalid_tail {
                warn!(
                    meeting_id = %record.id,
                    truncated_bytes = scan.truncated_bytes,
                    "truncated uncommitted or invalid meeting log tail during boot repair"
                );
            }
            let updated = self.db.update_meeting_cache_guarded(
                &record.id,
                record.log_epoch,
                scan.generation,
                &scan.cursor,
                scan.latest_seq,
                i64::try_from(scan.byte_len)
                    .map_err(|_| MeetingError::storage("meeting log exceeds i64 byte range"))?,
            )?;
            if updated {
                info!(
                    meeting_id = %record.id,
                    generation = scan.generation,
                    latest_seq = scan.latest_seq,
                    "rebuilt meeting SQLite cache from canonical log"
                );
            }
            let mut index = state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            index.index = Some(OffsetIndex::from_scan(&scan));
        }
        Ok(())
    }

    fn reconcile_tombstones(&self) -> Result<(), MeetingError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(MeetingError::io("scan meeting directory", error)),
        };
        let known_ids = self
            .db
            .list_all_meetings()?
            .into_iter()
            .map(|record| record.id)
            .collect::<HashSet<_>>();
        for entry in entries {
            let entry = entry.map_err(|error| MeetingError::io("scan meeting entry", error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| MeetingError::io("inspect meeting entry", error))?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".tombstone") {
                if normalize_meeting_id(id).is_err() {
                    continue;
                }
                if known_ids.contains(id) {
                    let restored = self.root.join(id);
                    if restored.exists() {
                        std::fs::remove_dir_all(&restored).map_err(|error| {
                            MeetingError::io(
                                "remove orphan meeting directory before restore",
                                error,
                            )
                        })?;
                    }
                    std::fs::rename(entry.path(), &restored).map_err(|error| {
                        MeetingError::io(
                            "restore meeting tombstone after rolled-back delete",
                            error,
                        )
                    })?;
                    info!(
                        meeting_id = id,
                        "restored meeting tombstone with live database row"
                    );
                } else {
                    std::fs::remove_dir_all(entry.path()).map_err(|error| {
                        MeetingError::io("remove committed meeting tombstone", error)
                    })?;
                    info!(meeting_id = id, "removed committed meeting tombstone");
                }
                continue;
            }
            if normalize_meeting_id(&name).is_ok() && !known_ids.contains(&name) {
                std::fs::remove_dir_all(entry.path())
                    .map_err(|error| MeetingError::io("remove orphan meeting directory", error))?;
                warn!(meeting_id = %name, "removed bare meeting directory without database row");
            }
        }
        Ok(())
    }

    pub async fn append_page(
        self: &Arc<Self>,
        id: &str,
        drafts: Vec<SegmentDraft>,
        cursor_out: &str,
    ) -> Result<AppendPageOutcome, MeetingError> {
        self.append_page_inner(id, drafts, cursor_out, None).await
    }

    async fn append_page_inner(
        self: &Arc<Self>,
        id: &str,
        drafts: Vec<SegmentDraft>,
        cursor_out: &str,
        #[cfg(test)] fault: Option<AppendTestFault>,
        #[cfg(not(test))] _fault: Option<()>,
    ) -> Result<AppendPageOutcome, MeetingError> {
        let id = normalize_meeting_id(id)?;
        if cursor_out.len() > 1_024 {
            return Err(MeetingError::bad_request("cursor_out exceeds 1024 bytes"));
        }
        let state = self.entity_state(&id);
        let guard = state.lock.clone().write_owned().await;
        let service = self.clone();
        let cursor_out = cursor_out.to_owned();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            service.append_page_blocking(
                &id,
                &state,
                drafts,
                &cursor_out,
                #[cfg(test)]
                fault,
            )
        })
        .await
        .map_err(|error| MeetingError::storage(format!("meeting append task failed: {error}")))?
    }

    fn append_page_blocking(
        &self,
        id: &str,
        state: &EntityIo,
        drafts: Vec<SegmentDraft>,
        cursor_out: &str,
        #[cfg(test)] fault: Option<AppendTestFault>,
    ) -> Result<AppendPageOutcome, MeetingError> {
        let mut record = self
            .db
            .get_meeting(id)?
            .ok_or_else(MeetingError::not_found)?;
        if record.parsed_status()?.is_terminal() {
            return Err(MeetingError::conflict(
                "meeting_terminal",
                "terminal meeting entities refuse appends",
            ));
        }
        if record.content_state == "lost" {
            return Err(MeetingError::content_lost());
        }
        let path = self.log_path(id);
        if !path.exists() && record.cache_generation > 0 {
            warn!(
                meeting_id = id,
                epoch = record.log_epoch,
                "runtime writer found a missing meeting log; rolling epoch"
            );
            record = self
                .db
                .rollover_missing_meeting_log(id, record.log_epoch)?
                .ok_or_else(MeetingError::not_found)?;
            let mut index = state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            index.index = None;
        }
        log::ensure_parent(&path)?;
        let before = scan_log(&path, record.log_epoch, false)?;
        if before.had_invalid_tail {
            return Err(MeetingError::storage(
                "meeting log has an uncommitted tail outside boot repair",
            ));
        }
        let first_seq = before
            .latest_seq
            .checked_add(1)
            .ok_or_else(|| MeetingError::storage("meeting segment sequence exhausted i64 range"))?;
        let segments = normalize_page(drafts, first_seq)?;
        let checkpoint = checkpoint_line(record.log_epoch, cursor_out, &log::now_timestamp())?;
        #[cfg(test)]
        let stop_after_segments = fault.and_then(|fault| match fault {
            AppendTestFault::Segment(count) => Some(count),
            _ => None,
        });
        #[cfg(not(test))]
        let stop_after_segments = None;
        #[cfg(test)]
        let skip_sync = fault == Some(AppendTestFault::CheckpointBeforeSync);
        #[cfg(not(test))]
        let skip_sync = false;
        append_lines_and_sync(
            &path,
            &segments,
            &checkpoint,
            stop_after_segments,
            skip_sync,
        )?;
        let committed = scan_log(&path, record.log_epoch, false)?;
        if committed.had_invalid_tail {
            return Err(MeetingError::storage(
                "meeting page did not end at a valid checkpoint",
            ));
        }
        {
            let mut index = state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            index.index = Some(OffsetIndex::from_scan(&committed));
        }
        #[cfg(test)]
        if fault == Some(AppendTestFault::SyncBeforeCache) {
            return Err(MeetingError::injected(
                "crash after checkpoint fdatasync before SQLite cache update",
            ));
        }
        let byte_len = i64::try_from(committed.byte_len)
            .map_err(|_| MeetingError::storage("meeting log exceeds i64 byte range"))?;
        let updated = self.db.update_meeting_cache_guarded(
            id,
            record.log_epoch,
            committed.generation,
            &committed.cursor,
            committed.latest_seq,
            byte_len,
        )?;
        if !updated {
            let latest = self
                .db
                .get_meeting(id)?
                .ok_or_else(MeetingError::not_found)?;
            if latest.log_epoch != record.log_epoch {
                return Err(MeetingError::content_loss());
            }
            if latest.cache_generation < committed.generation {
                return Err(MeetingError::storage(
                    "meeting page committed but SQLite cache repair is pending",
                ));
            }
        }
        Ok(AppendPageOutcome {
            log_epoch: record.log_epoch,
            generation: committed.generation,
            latest_seq: committed.latest_seq,
            byte_len: committed.byte_len,
        })
    }

    pub async fn repair_log_cache(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<AppendPageOutcome, MeetingError> {
        let id = normalize_meeting_id(id)?;
        let state = self.entity_state(&id);
        let guard = state.lock.clone().write_owned().await;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let record = service.record_for_locked_log_operation(&id, &state)?;
            let scan = scan_log(&service.log_path(&id), record.log_epoch, true)?;
            service.db.update_meeting_cache_guarded(
                &id,
                record.log_epoch,
                scan.generation,
                &scan.cursor,
                scan.latest_seq,
                i64::try_from(scan.byte_len)
                    .map_err(|_| MeetingError::storage("meeting log exceeds i64 byte range"))?,
            )?;
            state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .index = Some(OffsetIndex::from_scan(&scan));
            Ok(AppendPageOutcome {
                log_epoch: record.log_epoch,
                generation: scan.generation,
                latest_seq: scan.latest_seq,
                byte_len: scan.byte_len,
            })
        })
        .await
        .map_err(|error| MeetingError::storage(format!("cache repair task failed: {error}")))?
    }

    pub async fn persist_terminal_index(self: &Arc<Self>, id: &str) -> Result<(), MeetingError> {
        let id = normalize_meeting_id(id)?;
        let state = self.entity_state(&id);
        let guard = state.lock.clone().write_owned().await;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let record = service.record_for_locked_log_operation(&id, &state)?;
            let path = service.log_path(&id);
            log::ensure_parent(&path)?;
            let scan = scan_log(&path, record.log_epoch, true)?;
            service.db.update_meeting_cache_guarded(
                &id,
                record.log_epoch,
                scan.generation,
                &scan.cursor,
                scan.latest_seq,
                i64::try_from(scan.byte_len)
                    .map_err(|_| MeetingError::storage("meeting log exceeds i64 byte range"))?,
            )?;
            let verified = service
                .db
                .get_meeting(&id)?
                .ok_or_else(MeetingError::not_found)?;
            if verified.log_epoch != scan.epoch
                || verified.cache_generation != scan.generation
                || verified.closed_segment_count != scan.latest_seq
                || verified.byte_size
                    != i64::try_from(scan.byte_len)
                        .map_err(|_| MeetingError::storage("meeting log exceeds i64 byte range"))?
            {
                return Err(MeetingError::storage(
                    "terminal meeting cache read-back did not match canonical log",
                ));
            }
            let index = OffsetIndex::from_scan(&scan);
            persist_index(&service.entity_dir(&id), &index)?;
            state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .index = Some(index);
            Ok(())
        })
        .await
        .map_err(|error| MeetingError::storage(format!("index persist task failed: {error}")))?
    }

    fn record_for_locked_log_operation(
        &self,
        id: &str,
        state: &EntityIo,
    ) -> Result<MeetingRecord, MeetingError> {
        let mut record = self
            .db
            .get_meeting(id)?
            .ok_or_else(MeetingError::not_found)?;
        if self.log_path(id).exists() || record.cache_generation == 0 {
            return Ok(record);
        }
        if record.parsed_status()?.is_terminal() {
            self.db
                .mark_meeting_content_lost(id, &log::now_timestamp())?
                .ok_or_else(MeetingError::not_found)?;
            return Err(MeetingError::content_lost());
        }
        warn!(
            meeting_id = id,
            epoch = record.log_epoch,
            "locked meeting log operation found missing content; rolling epoch"
        );
        record = self
            .db
            .rollover_missing_meeting_log(id, record.log_epoch)?
            .ok_or_else(MeetingError::not_found)?;
        state
            .index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .index = None;
        Ok(record)
    }

    pub async fn delete(self: &Arc<Self>, id: &str) -> Result<(), MeetingError> {
        self.delete_inner(id, None).await
    }

    async fn delete_inner(
        self: &Arc<Self>,
        id: &str,
        #[cfg(test)] fault: Option<DeleteTestFault>,
        #[cfg(not(test))] _fault: Option<()>,
    ) -> Result<(), MeetingError> {
        let id = normalize_meeting_id(id)?;
        let state = self.entity_state(&id);
        let guard = state.lock.clone().write_owned().await;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let record = service
                .db
                .get_meeting(&id)?
                .ok_or_else(MeetingError::not_found)?;
            if !record.parsed_status()?.is_terminal() {
                return Err(MeetingError::conflict(
                    "meeting_not_terminal",
                    "only terminal meeting entities can be deleted",
                ));
            }
            let entity_dir = service.entity_dir(&id);
            let tombstone = service.root.join(format!("{id}.tombstone"));
            let renamed = if entity_dir.exists() {
                std::fs::rename(&entity_dir, &tombstone)
                    .map_err(|error| MeetingError::io("rename meeting tombstone", error))?;
                true
            } else {
                false
            };
            #[cfg(test)]
            if fault == Some(DeleteTestFault::AfterRename) {
                return Err(MeetingError::injected("crash after tombstone rename"));
            }
            let deleted = service.db.delete_terminal_meeting_row(&id);
            let outcome = match deleted {
                Ok(outcome) => outcome,
                Err(error) => {
                    if renamed && tombstone.exists() {
                        let _ = std::fs::rename(&tombstone, &entity_dir);
                    }
                    return Err(error.into());
                }
            };
            match outcome {
                DeleteMeetingRowOutcome::Deleted => {}
                DeleteMeetingRowOutcome::NotFound => return Err(MeetingError::not_found()),
                DeleteMeetingRowOutcome::NotTerminal => {
                    if renamed && tombstone.exists() {
                        let _ = std::fs::rename(&tombstone, &entity_dir);
                    }
                    return Err(MeetingError::conflict(
                        "meeting_not_terminal",
                        "only terminal meeting entities can be deleted",
                    ));
                }
            }
            #[cfg(test)]
            if fault == Some(DeleteTestFault::AfterDatabaseCommit) {
                return Err(MeetingError::injected(
                    "crash after meeting database delete",
                ));
            }
            if renamed && tombstone.exists() {
                std::fs::remove_dir_all(&tombstone)
                    .map_err(|error| MeetingError::io("remove meeting tombstone", error))?;
            }
            let mut index = state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            index.index = None;
            index.building = false;
            Ok(())
        })
        .await
        .map_err(|error| MeetingError::storage(format!("meeting delete task failed: {error}")))?
    }

    pub(crate) async fn get_record(&self, id: &str) -> Result<MeetingRecord, MeetingError> {
        let id = normalize_meeting_id(id)?;
        let db = self.db.clone();
        db.run_blocking(move |db| db.get_meeting(&id))
            .await?
            .ok_or_else(MeetingError::not_found)
    }

    pub(crate) fn install_index(&self, id: &str, index: OffsetIndex) {
        self.entity_state(id)
            .index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .index = Some(index);
    }

    pub(crate) fn cached_or_disk_index(
        &self,
        id: &str,
        record: &MeetingRecord,
    ) -> Result<Option<OffsetIndex>, MeetingError> {
        let state = self.entity_state(id);
        if let Some(index) = state
            .index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .index
            .clone()
            && index.log_epoch == record.log_epoch
            && index.log_byte_len == record.byte_size as u64
            && index.latest_seq == record.closed_segment_count
        {
            return Ok(Some(index));
        }
        let loaded = load_index(
            &self.entity_dir(id),
            record.log_epoch,
            u64::try_from(record.byte_size)
                .map_err(|_| MeetingError::storage("negative meeting byte size"))?,
            record.closed_segment_count,
        )?;
        if let Some(index) = loaded.clone() {
            self.install_index(id, index);
        }
        Ok(loaded)
    }
}

#[derive(Debug, Deserialize)]
pub struct ListMeetingsQuery {
    limit: Option<usize>,
    page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ListPageToken {
    created_at: String,
    id: String,
}

pub async fn list_meetings(
    Query(query): Query<ListMeetingsQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return MeetingError::named_bad_request("invalid_limit", "limit must be between 1 and 100")
            .into_response();
    }
    let after = match query.page_token.as_deref() {
        Some(token) => match decode_list_token(token) {
            Ok(token) => Some((token.created_at, token.id)),
            Err(error) => return error.into_response(),
        },
        None => None,
    };
    let db = state.ops.garyx_db.clone();
    let result = db
        .run_blocking(move |db| db.list_meetings_page(limit + 1, after))
        .await;
    let mut records = match result {
        Ok(records) => records,
        Err(error) => return MeetingError::from(error).into_response(),
    };
    let has_more = records.len() > limit;
    records.truncate(limit);
    let next_page_token = if has_more {
        records.last().and_then(|record| {
            encode_list_token(&ListPageToken {
                created_at: record.created_at.clone(),
                id: record.id.clone(),
            })
            .ok()
        })
    } else {
        None
    };
    let meetings = records.into_iter().map(meeting_view).collect::<Vec<_>>();
    json_response(
        StatusCode::OK,
        &json!({
            "meetings": meetings,
            "next_page_token": next_page_token,
        }),
    )
}

pub async fn get_meeting(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    match state.ops.meetings.get_record(&id).await {
        Ok(record) => json_response(StatusCode::OK, &json!({ "meeting": meeting_view(record) })),
        Err(error) => error.into_response(),
    }
}

pub async fn delete_meeting(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    match state.ops.meetings.delete(&id).await {
        Ok(()) => json_response(StatusCode::OK, &json!({ "deleted": true })),
        Err(error) => error.into_response(),
    }
}

pub use read::{confirm_meeting_read, read_meeting};

pub(crate) fn meeting_view(record: MeetingRecord) -> Value {
    let mut value = serde_json::to_value(record).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert("stalled_reason".to_owned(), Value::String(String::new()));
    }
    value
}

fn encode_list_token(token: &ListPageToken) -> Result<String, MeetingError> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(token)?))
}

fn decode_list_token(token: &str) -> Result<ListPageToken, MeetingError> {
    let raw = URL_SAFE_NO_PAD.decode(token).map_err(|_| {
        MeetingError::named_bad_request("invalid_page_token", "page_token is invalid")
    })?;
    let token: ListPageToken = serde_json::from_slice(&raw).map_err(|_| {
        MeetingError::named_bad_request("invalid_page_token", "page_token is invalid")
    })?;
    normalize_meeting_id(&token.id).map_err(|_| {
        MeetingError::named_bad_request("invalid_page_token", "page_token is invalid")
    })?;
    if log::normalize_timestamp("page_token.created_at", &token.created_at).is_err() {
        return Err(MeetingError::named_bad_request(
            "invalid_page_token",
            "page_token is invalid",
        ));
    }
    Ok(token)
}

pub(crate) fn json_response(status: StatusCode, value: &impl Serialize) -> Response {
    match serde_json::to_vec(value) {
        Ok(bytes) => Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .expect("meeting JSON response is valid"),
        Err(error) => {
            warn!(error = %error, "failed to serialize meeting response");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"error":{"code":"storage_error","message":"failed to serialize meeting response"}}"#,
                ))
                .expect("static meeting error response is valid")
        }
    }
}

#[cfg(test)]
mod tests;
