use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::OwnedRwLockReadGuard;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::garyx_db::{
    MeetingConfirmOutcome, MeetingCursorClaimOutcome, MeetingCursorDomainError, MeetingReadCursor,
    MeetingRecord,
};
use crate::server::AppState;

use super::index::{OffsetIndex, persist_index};
use super::log::{MeetingSegment, read_segments, scan_log};
use super::{EntityIo, MIN_READ_PAGE_BYTES, MeetingError, MeetingService, json_response};

const TOKEN_INACTIVITY_MILLIS: i64 = 10 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingReadMode {
    Incremental,
    Full,
    Range,
}

impl MeetingReadMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Incremental => "incremental",
            Self::Full => "full",
            Self::Range => "range",
        }
    }

    fn is_stateless(self) -> bool {
        matches!(self, Self::Full | Self::Range)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingReadRequest {
    pub mode: MeetingReadMode,
    #[serde(default)]
    pub reader_id: Option<String>,
    #[serde(default)]
    pub range_start: Option<i64>,
    #[serde(default)]
    pub range_end: Option<i64>,
    #[serde(default)]
    pub epoch: Option<i64>,
    #[serde(default)]
    pub continue_token: Option<String>,
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmMeetingReadRequest {
    pub reader_id: String,
    pub receipt: String,
    pub log_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingReadMeta {
    pub mode: String,
    pub entity_id: String,
    pub log_epoch: i64,
    pub status: String,
    pub status_detail: String,
    pub end_source: String,
    pub stalled_reason: String,
    pub content_state: String,
    pub topic: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub finalized_at: Option<String>,
    pub content_lost_at: Option<String>,
    pub updated_at: String,
    pub span_from: Option<i64>,
    pub span_to: Option<i64>,
    pub closed_total: i64,
    pub receipt: Option<String>,
    pub continue_token: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingReadResponse {
    pub meta: MeetingReadMeta,
    pub segments: Vec<MeetingSegment>,
}

#[derive(Debug, Clone)]
enum ValidatedRead {
    Incremental {
        reader_id: String,
        requested_budget: usize,
    },
    StatelessFirst {
        mode: MeetingReadMode,
        range_start: Option<i64>,
        range_end: Option<i64>,
        epoch: Option<i64>,
        requested_budget: usize,
    },
    StatelessContinue {
        mode: MeetingReadMode,
        token: SnapshotToken,
        requested_budget: usize,
    },
}

#[derive(Debug)]
struct ReadPreflight {
    _guard: OwnedRwLockReadGuard<()>,
    record: MeetingRecord,
    snapshot: ReadSnapshot,
    index: OffsetIndex,
    log_path: std::path::PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ReadSnapshot {
    closed_latest: i64,
    log_byte_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnsignedSnapshotToken {
    entity_id: String,
    log_epoch: i64,
    snapshot: ReadSnapshot,
    next_seq: i64,
    mode: MeetingReadMode,
    origin_range_start: i64,
    range_end: i64,
    issued_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotToken {
    entity_id: String,
    log_epoch: i64,
    snapshot: ReadSnapshot,
    next_seq: i64,
    mode: MeetingReadMode,
    origin_range_start: i64,
    range_end: i64,
    checksum: String,
    issued_at: i64,
}

impl SnapshotToken {
    fn unsigned(&self) -> UnsignedSnapshotToken {
        UnsignedSnapshotToken {
            entity_id: self.entity_id.clone(),
            log_epoch: self.log_epoch,
            snapshot: self.snapshot,
            next_seq: self.next_seq,
            mode: self.mode,
            origin_range_start: self.origin_range_start,
            range_end: self.range_end,
            issued_at: self.issued_at,
        }
    }
}

impl MeetingService {
    pub async fn read(
        self: &Arc<Self>,
        id: &str,
        request: MeetingReadRequest,
    ) -> Result<MeetingReadResponse, MeetingError> {
        let id = crate::garyx_db::normalize_meeting_id(id)?;
        let validated = validate_request(&id, request, self.read_page_bytes())?;
        match validated {
            ValidatedRead::Incremental {
                reader_id,
                requested_budget,
            } => {
                self.read_incremental(&id, &reader_id, requested_budget)
                    .await
            }
            ValidatedRead::StatelessFirst {
                mode,
                range_start,
                range_end,
                epoch,
                requested_budget,
            } => {
                self.read_stateless_first(
                    &id,
                    mode,
                    range_start,
                    range_end,
                    epoch,
                    requested_budget,
                )
                .await
            }
            ValidatedRead::StatelessContinue {
                mode,
                token,
                requested_budget,
            } => {
                self.read_stateless_continue(&id, mode, token, requested_budget)
                    .await
            }
        }
    }

    pub async fn confirm(
        self: &Arc<Self>,
        id: &str,
        request: ConfirmMeetingReadRequest,
    ) -> Result<MeetingConfirmOutcome, MeetingError> {
        let id = crate::garyx_db::normalize_meeting_id(id)?;
        let reader_id = normalize_reader(&request.reader_id)?;
        if request.receipt.trim().is_empty() {
            return Err(MeetingError::named_bad_request(
                "invalid_receipt",
                "receipt must not be empty",
            ));
        }
        if request.log_epoch < 0 {
            return Err(MeetingError::named_bad_request(
                "invalid_log_epoch",
                "log_epoch must be non-negative",
            ));
        }
        let state = self.entity_state(&id);
        let _guard = state.lock.clone().read_owned().await;
        let db = self.db.clone();
        let receipt = request.receipt;
        let epoch = request.log_epoch;
        let result = db
            .run_blocking(move |db| db.confirm_meeting_cursor(&id, &reader_id, &receipt, epoch))
            .await?;
        map_cursor_domain(result)
    }

    async fn read_incremental(
        self: &Arc<Self>,
        id: &str,
        reader_id: &str,
        requested_budget: usize,
    ) -> Result<MeetingReadResponse, MeetingError> {
        loop {
            let preflight = self.preflight(id, None).await?;
            let epoch = preflight.record.log_epoch;
            let id_owned = id.to_owned();
            let reader_owned = reader_id.to_owned();
            let db = self.db.clone();
            let prepared = db
                .run_blocking(move |db| db.prepare_meeting_cursor(&id_owned, &reader_owned, epoch))
                .await?;
            let prepared = map_cursor_domain(prepared)?;
            let mut cursor = prepared.cursor;
            let first_read = prepared.created;

            loop {
                if let Some(pending_to) = cursor.pending_to {
                    if pending_to > preflight.snapshot.closed_latest {
                        // Defense in depth for a winner that claimed against a
                        // newer snapshot (e.g. a stale cross-process reader).
                        drop(preflight);
                        tokio::task::yield_now().await;
                        break;
                    }
                    return pending_response(
                        self,
                        &preflight,
                        cursor,
                        requested_budget,
                        first_read,
                    );
                }

                if cursor.confirmed_seq >= preflight.snapshot.closed_latest {
                    let mut notes = Vec::new();
                    if first_read {
                        notes.push("first read for this reader".to_owned());
                    }
                    notes.push(format!("no new segments since {}", cursor.confirmed_seq));
                    return Ok(MeetingReadResponse {
                        meta: response_meta(
                            self,
                            MeetingReadMode::Incremental,
                            &preflight.record,
                            preflight.snapshot.closed_latest,
                            None,
                            None,
                            None,
                            None,
                            notes,
                        ),
                        segments: Vec::new(),
                    });
                }

                let from = cursor
                    .confirmed_seq
                    .checked_add(1)
                    .ok_or_else(|| MeetingError::storage("meeting cursor exhausted i64 range"))?;
                let all = read_segments(
                    &self.log_path(id),
                    &preflight.index.offsets,
                    from,
                    preflight.snapshot.closed_latest,
                    preflight.snapshot.log_byte_offset,
                    preflight.record.log_epoch,
                )?;
                if all.is_empty() {
                    return Err(MeetingError::storage(
                        "meeting log snapshot did not contain the expected next segment",
                    ));
                }
                let receipt = Uuid::new_v4().to_string();
                let (segments, notes) = select_incremental_segments(
                    self,
                    &preflight.record,
                    preflight.snapshot.closed_latest,
                    all,
                    &receipt,
                    requested_budget.min(self.read_page_bytes()),
                    first_read,
                )?;
                let to = segments
                    .last()
                    .map(|segment| segment.seq)
                    .ok_or_else(|| MeetingError::storage("meeting read made no progress"))?;
                let id_owned = id.to_owned();
                let reader_owned = reader_id.to_owned();
                let receipt_owned = receipt.clone();
                let db = self.db.clone();
                let expected = cursor.confirmed_seq;
                let claim = db
                    .run_blocking(move |db| {
                        db.claim_meeting_cursor(
                            &id_owned,
                            &reader_owned,
                            epoch,
                            expected,
                            from,
                            to,
                            &receipt_owned,
                        )
                    })
                    .await?;
                match map_cursor_domain(claim)? {
                    MeetingCursorClaimOutcome::Claimed(_claimed) => {
                        return Ok(MeetingReadResponse {
                            meta: response_meta(
                                self,
                                MeetingReadMode::Incremental,
                                &preflight.record,
                                preflight.snapshot.closed_latest,
                                Some(from),
                                Some(to),
                                Some(receipt),
                                None,
                                notes,
                            ),
                            segments,
                        });
                    }
                    MeetingCursorClaimOutcome::Winner(winner) => {
                        cursor = winner;
                    }
                    MeetingCursorClaimOutcome::Retry(current) => {
                        cursor = current;
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn read_stateless_first(
        self: &Arc<Self>,
        id: &str,
        mode: MeetingReadMode,
        range_start: Option<i64>,
        range_end: Option<i64>,
        epoch: Option<i64>,
        requested_budget: usize,
    ) -> Result<MeetingReadResponse, MeetingError> {
        let preflight = self.preflight(id, epoch).await?;
        let (origin_start, range_end) = match mode {
            MeetingReadMode::Full => (1, preflight.snapshot.closed_latest),
            MeetingReadMode::Range => (
                range_start.expect("validated range start"),
                range_end.expect("validated range end"),
            ),
            MeetingReadMode::Incremental => unreachable!("validated stateless mode"),
        };
        stateless_page(
            self,
            id,
            mode,
            &preflight,
            origin_start,
            range_end,
            origin_start,
            requested_budget.min(self.read_page_bytes()),
        )
    }

    async fn read_stateless_continue(
        self: &Arc<Self>,
        id: &str,
        mode: MeetingReadMode,
        token: SnapshotToken,
        requested_budget: usize,
    ) -> Result<MeetingReadResponse, MeetingError> {
        let preflight = self.preflight(id, Some(token.log_epoch)).await?;
        if token.snapshot.log_byte_offset > preflight.snapshot.log_byte_offset
            || token.snapshot.closed_latest > preflight.snapshot.closed_latest
        {
            return Err(MeetingError::content_loss());
        }
        let token_preflight = ReadPreflight {
            _guard: preflight._guard,
            record: preflight.record,
            snapshot: token.snapshot,
            index: preflight.index,
            log_path: preflight.log_path,
        };
        stateless_page(
            self,
            id,
            mode,
            &token_preflight,
            token.origin_range_start,
            token.range_end,
            token.next_seq,
            requested_budget.min(self.read_page_bytes()),
        )
    }

    async fn preflight(
        self: &Arc<Self>,
        id: &str,
        expected_epoch: Option<i64>,
    ) -> Result<ReadPreflight, MeetingError> {
        loop {
            let state = self.entity_state(id);
            let guard = state.lock.clone().read_owned().await;
            let id_owned = id.to_owned();
            let db = self.db.clone();
            let Some(record) = db.run_blocking(move |db| db.get_meeting(&id_owned)).await? else {
                return Err(MeetingError::not_found());
            };
            if record.content_state == "lost" {
                return Err(MeetingError::content_lost());
            }
            if expected_epoch.is_some_and(|epoch| epoch != record.log_epoch) {
                return Err(MeetingError::content_loss());
            }
            let terminal = record.parsed_status()?.is_terminal();
            let path = self.log_path(id);
            if !path.exists()
                && (record.byte_size > 0
                    || record.closed_segment_count > 0
                    || record.cache_generation > 0)
            {
                drop(guard);
                let write_guard = state.lock.clone().write_owned().await;
                let service = self.clone();
                let id_owned = id.to_owned();
                let resolution = tokio::task::spawn_blocking(move || {
                    let _guard = write_guard;
                    let Some(current) = service.db.get_meeting(&id_owned)? else {
                        return Err(MeetingError::not_found());
                    };
                    if service.log_path(&id_owned).exists() {
                        return Ok(None);
                    }
                    let terminal_now = current.parsed_status()?.is_terminal();
                    if terminal_now {
                        service
                            .db
                            .mark_meeting_content_lost(&id_owned, &super::log::now_timestamp())?
                            .ok_or_else(MeetingError::not_found)?;
                        warn!(
                            meeting_id = %id_owned,
                            "terminal meeting log disappeared; marked content lost"
                        );
                    } else {
                        service
                            .db
                            .rollover_missing_meeting_log(&id_owned, current.log_epoch)?
                            .ok_or_else(MeetingError::not_found)?;
                        service
                            .entity_state(&id_owned)
                            .index
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .index = None;
                        warn!(
                            meeting_id = %id_owned,
                            "live meeting log disappeared; rolled content epoch"
                        );
                    }
                    Ok(Some(terminal_now))
                })
                .await
                .map_err(|error| {
                    MeetingError::storage(format!("content-loss detection task failed: {error}"))
                })??;
                if let Some(terminal_now) = resolution {
                    return Err(if terminal_now {
                        MeetingError::content_lost()
                    } else {
                        MeetingError::content_loss()
                    });
                }
                continue;
            }

            if terminal {
                if let Some(index) = self.cached_or_disk_index(id, &record)? {
                    return Ok(ReadPreflight {
                        _guard: guard,
                        snapshot: ReadSnapshot {
                            closed_latest: record.closed_segment_count,
                            log_byte_offset: u64::try_from(record.byte_size)
                                .map_err(|_| MeetingError::storage("negative meeting byte size"))?,
                        },
                        record,
                        index,
                        log_path: path,
                    });
                }
                drop(guard);
                self.start_index_rebuild(id, state, record).await;
                return Err(MeetingError::index_building());
            }

            let epoch = record.log_epoch;
            let scan_path = path.clone();
            let scan = tokio::task::spawn_blocking(move || scan_log(&scan_path, epoch, false))
                .await
                .map_err(|error| {
                    MeetingError::storage(format!("meeting snapshot task failed: {error}"))
                })??;
            if scan.had_invalid_tail {
                return Err(MeetingError::storage(
                    "meeting log contains an invalid tail outside boot repair",
                ));
            }
            let index = OffsetIndex::from_scan(&scan);
            self.install_index(id, index.clone());
            return Ok(ReadPreflight {
                _guard: guard,
                record,
                snapshot: ReadSnapshot {
                    closed_latest: scan.latest_seq,
                    log_byte_offset: scan.byte_len,
                },
                index,
                log_path: path,
            });
        }
    }

    async fn start_index_rebuild(
        self: &Arc<Self>,
        id: &str,
        state: Arc<EntityIo>,
        record: MeetingRecord,
    ) {
        {
            let mut index = state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if index.building {
                return;
            }
            index.building = true;
        }
        let service = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            let guard = state.lock.clone().read_owned().await;
            let service_for_build = service.clone();
            let id_for_build = id.clone();
            let state_for_build = state.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                let path = service_for_build.log_path(&id_for_build);
                let scan = scan_log(&path, record.log_epoch, false)?;
                if scan.had_invalid_tail
                    || scan.byte_len
                        != u64::try_from(record.byte_size)
                            .map_err(|_| MeetingError::storage("negative meeting byte size"))?
                    || scan.latest_seq != record.closed_segment_count
                {
                    return Err(MeetingError::storage(
                        "terminal meeting log does not match its verified cache",
                    ));
                }
                let current = service_for_build
                    .db
                    .get_meeting(&id_for_build)?
                    .ok_or_else(MeetingError::not_found)?;
                if current.log_epoch != record.log_epoch
                    || service_for_build
                        .root()
                        .join(format!("{id_for_build}.tombstone"))
                        .exists()
                {
                    return Err(MeetingError::content_loss());
                }
                let index = OffsetIndex::from_scan(&scan);
                persist_index(&service_for_build.entity_dir(&id_for_build), &index)?;
                state_for_build
                    .index
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .index = Some(index);
                Ok(())
            })
            .await;
            match result {
                Ok(Ok(())) => {
                    debug!(meeting_id = %id, "rebuilt terminal meeting offset index");
                }
                Ok(Err(error)) => {
                    warn!(meeting_id = %id, error = %error, "meeting index rebuild failed");
                }
                Err(error) => {
                    warn!(meeting_id = %id, error = %error, "meeting index rebuild task failed");
                }
            }
            state
                .index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .building = false;
        });
    }
}

fn validate_request(
    entity_id: &str,
    request: MeetingReadRequest,
    default_budget: usize,
) -> Result<ValidatedRead, MeetingError> {
    let requested_budget = request.max_bytes.unwrap_or(default_budget);
    if requested_budget < MIN_READ_PAGE_BYTES {
        return Err(MeetingError::named_bad_request(
            "invalid_max_bytes",
            "max_bytes must be at least 4096",
        ));
    }
    if let Some(token) = request.continue_token {
        if !request.mode.is_stateless() {
            return Err(MeetingError::named_bad_request(
                "invalid_continue_token",
                "continue_token is valid only for full or range mode",
            ));
        }
        if request.reader_id.is_some() {
            return Err(MeetingError::named_bad_request(
                "invalid_reader_id",
                "reader_id is not valid with continue_token",
            ));
        }
        if request.range_start.is_some() || request.range_end.is_some() || request.epoch.is_some() {
            return Err(MeetingError::named_bad_request(
                "invalid_continue_token",
                "continue_token is mutually exclusive with range_start, range_end, and epoch",
            ));
        }
        let token = decode_and_validate_token(&token, entity_id, request.mode, now_millis())?;
        return Ok(ValidatedRead::StatelessContinue {
            mode: request.mode,
            token,
            requested_budget,
        });
    }

    match request.mode {
        MeetingReadMode::Incremental => {
            if request.range_start.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_range_start",
                    "range_start is valid only in range mode",
                ));
            }
            if request.range_end.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_range_end",
                    "range_end is valid only in range mode",
                ));
            }
            if request.epoch.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_epoch",
                    "epoch is valid only in range mode",
                ));
            }
            let reader_id = request.reader_id.ok_or_else(|| {
                MeetingError::named_bad_request(
                    "missing_reader_id",
                    "reader_id is required for incremental mode",
                )
            })?;
            Ok(ValidatedRead::Incremental {
                reader_id: normalize_reader(&reader_id)?,
                requested_budget,
            })
        }
        MeetingReadMode::Full => {
            if request.reader_id.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_reader_id",
                    "reader_id is not valid in full mode",
                ));
            }
            if request.range_start.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_range_start",
                    "range_start is not valid in full mode",
                ));
            }
            if request.range_end.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_range_end",
                    "range_end is not valid in full mode",
                ));
            }
            if request.epoch.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_epoch",
                    "epoch is not valid in full mode",
                ));
            }
            Ok(ValidatedRead::StatelessFirst {
                mode: request.mode,
                range_start: None,
                range_end: None,
                epoch: None,
                requested_budget,
            })
        }
        MeetingReadMode::Range => {
            if request.reader_id.is_some() {
                return Err(MeetingError::named_bad_request(
                    "invalid_reader_id",
                    "reader_id is not valid in range mode",
                ));
            }
            let start = request.range_start.ok_or_else(|| {
                MeetingError::named_bad_request(
                    "missing_range_start",
                    "range_start is required in range mode",
                )
            })?;
            let end = request.range_end.ok_or_else(|| {
                MeetingError::named_bad_request(
                    "missing_range_end",
                    "range_end is required in range mode",
                )
            })?;
            if start <= 0 {
                return Err(MeetingError::named_bad_request(
                    "invalid_range_start",
                    "range_start must be positive",
                ));
            }
            if end < start {
                return Err(MeetingError::named_bad_request(
                    "invalid_range_end",
                    "range_end must be at least range_start",
                ));
            }
            if request.epoch.is_some_and(|epoch| epoch < 0) {
                return Err(MeetingError::named_bad_request(
                    "invalid_epoch",
                    "epoch must be non-negative",
                ));
            }
            Ok(ValidatedRead::StatelessFirst {
                mode: request.mode,
                range_start: Some(start),
                range_end: Some(end),
                epoch: request.epoch,
                requested_budget,
            })
        }
    }
}

fn normalize_reader(reader_id: &str) -> Result<String, MeetingError> {
    let trimmed = reader_id.trim();
    if !(1..=128).contains(&trimmed.len()) {
        return Err(MeetingError::named_bad_request(
            "invalid_reader_id",
            "reader_id must be between 1 and 128 bytes",
        ));
    }
    Ok(trimmed.to_owned())
}

fn pending_response(
    service: &MeetingService,
    preflight: &ReadPreflight,
    cursor: MeetingReadCursor,
    requested_budget: usize,
    first_read: bool,
) -> Result<MeetingReadResponse, MeetingError> {
    let from = cursor
        .pending_from
        .ok_or_else(|| MeetingError::storage("pending cursor is incomplete"))?;
    let to = cursor
        .pending_to
        .ok_or_else(|| MeetingError::storage("pending cursor is incomplete"))?;
    let receipt = cursor
        .receipt
        .ok_or_else(|| MeetingError::storage("pending cursor is missing its receipt"))?;
    let segments = read_segments(
        &preflight.log_path,
        &preflight.index.offsets,
        from,
        to,
        preflight.snapshot.log_byte_offset,
        preflight.record.log_epoch,
    )?;
    if segments.first().map(|segment| segment.seq) != Some(from)
        || segments.last().map(|segment| segment.seq) != Some(to)
    {
        return Err(MeetingError::storage(
            "pending meeting span is not fully present in the snapshot",
        ));
    }
    let mut notes = Vec::new();
    if first_read {
        notes.push("first read for this reader".to_owned());
    }
    notes.push("pending span replay; confirmation still pending".to_owned());
    let mut response = MeetingReadResponse {
        meta: response_meta(
            service,
            MeetingReadMode::Incremental,
            &preflight.record,
            preflight.snapshot.closed_latest,
            Some(from),
            Some(to),
            Some(receipt),
            None,
            notes,
        ),
        segments,
    };
    if serde_json::to_vec(&response)?.len() > requested_budget {
        response
            .meta
            .notes
            .push("pending replay exceeds requested budget".to_owned());
    }
    Ok(response)
}

fn select_incremental_segments(
    service: &MeetingService,
    record: &MeetingRecord,
    closed_total: i64,
    all: Vec<MeetingSegment>,
    receipt: &str,
    budget: usize,
    first_read: bool,
) -> Result<(Vec<MeetingSegment>, Vec<String>), MeetingError> {
    let mut selected = Vec::new();
    let mut notes = if first_read {
        vec!["first read for this reader".to_owned()]
    } else {
        Vec::new()
    };
    notes.push("confirmation pending; CLI confirms after stdout flush".to_owned());
    for segment in all {
        selected.push(segment);
        let candidate = MeetingReadResponse {
            meta: response_meta(
                service,
                MeetingReadMode::Incremental,
                record,
                closed_total,
                selected.first().map(|segment| segment.seq),
                selected.last().map(|segment| segment.seq),
                Some(receipt.to_owned()),
                None,
                notes.clone(),
            ),
            segments: selected.clone(),
        };
        if serde_json::to_vec(&candidate)?.len() > budget {
            if selected.len() == 1 {
                notes.push("single-segment minimum-progress overshoot".to_owned());
            } else {
                selected.pop();
            }
            break;
        }
    }
    if selected.is_empty() {
        return Err(MeetingError::storage(
            "budget selection returned a zero-progress meeting span",
        ));
    }
    Ok((selected, notes))
}

#[allow(clippy::too_many_arguments)]
fn stateless_page(
    service: &MeetingService,
    id: &str,
    mode: MeetingReadMode,
    preflight: &ReadPreflight,
    origin_start: i64,
    requested_range_end: i64,
    next_seq: i64,
    budget: usize,
) -> Result<MeetingReadResponse, MeetingError> {
    let range_end = requested_range_end.min(preflight.snapshot.closed_latest);
    if next_seq > range_end {
        return Ok(MeetingReadResponse {
            meta: response_meta(
                service,
                mode,
                &preflight.record,
                preflight.snapshot.closed_latest,
                None,
                None,
                None,
                None,
                vec![format!(
                    "snapshot contains no segments in {}..{}",
                    next_seq, requested_range_end
                )],
            ),
            segments: Vec::new(),
        });
    }
    let all = read_segments(
        &preflight.log_path,
        &preflight.index.offsets,
        next_seq,
        range_end,
        preflight.snapshot.log_byte_offset,
        preflight.record.log_epoch,
    )?;
    select_stateless_page(
        service,
        id,
        mode,
        preflight,
        origin_start,
        requested_range_end,
        all,
        budget,
    )
}

fn select_stateless_page(
    service: &MeetingService,
    id: &str,
    mode: MeetingReadMode,
    preflight: &ReadPreflight,
    origin_start: i64,
    requested_range_end: i64,
    all: Vec<MeetingSegment>,
    budget: usize,
) -> Result<MeetingReadResponse, MeetingError> {
    if all.is_empty() {
        return Err(MeetingError::storage(
            "snapshot read made zero progress before its range end",
        ));
    }
    let effective_end = requested_range_end.min(preflight.snapshot.closed_latest);
    let mut selected = Vec::new();
    let mut notes = Vec::new();
    for segment in all {
        selected.push(segment);
        let next = selected
            .last()
            .expect("selected segment")
            .seq
            .checked_add(1)
            .ok_or_else(|| {
                MeetingError::storage("meeting snapshot sequence exhausted i64 range")
            })?;
        let token = if next <= effective_end {
            Some(encode_token(SnapshotToken {
                entity_id: id.to_owned(),
                log_epoch: preflight.record.log_epoch,
                snapshot: preflight.snapshot,
                next_seq: next,
                mode,
                origin_range_start: origin_start,
                range_end: requested_range_end,
                checksum: String::new(),
                issued_at: now_millis(),
            })?)
        } else {
            None
        };
        let candidate = MeetingReadResponse {
            meta: response_meta(
                service,
                mode,
                &preflight.record,
                preflight.snapshot.closed_latest,
                selected.first().map(|segment| segment.seq),
                selected.last().map(|segment| segment.seq),
                None,
                token,
                notes.clone(),
            ),
            segments: selected.clone(),
        };
        if serde_json::to_vec(&candidate)?.len() > budget {
            if selected.len() == 1 {
                notes.push("single-segment minimum-progress overshoot".to_owned());
            } else {
                selected.pop();
            }
            break;
        }
    }
    let next = selected
        .last()
        .ok_or_else(|| MeetingError::storage("snapshot read made zero progress"))?
        .seq
        .checked_add(1)
        .ok_or_else(|| MeetingError::storage("meeting snapshot sequence exhausted i64 range"))?;
    let continue_token = if next <= effective_end {
        Some(encode_token(SnapshotToken {
            entity_id: id.to_owned(),
            log_epoch: preflight.record.log_epoch,
            snapshot: preflight.snapshot,
            next_seq: next,
            mode,
            origin_range_start: origin_start,
            range_end: requested_range_end,
            checksum: String::new(),
            issued_at: now_millis(),
        })?)
    } else {
        None
    };
    Ok(MeetingReadResponse {
        meta: response_meta(
            service,
            mode,
            &preflight.record,
            preflight.snapshot.closed_latest,
            selected.first().map(|segment| segment.seq),
            selected.last().map(|segment| segment.seq),
            None,
            continue_token,
            notes,
        ),
        segments: selected,
    })
}

#[allow(clippy::too_many_arguments)]
fn response_meta(
    service: &MeetingService,
    mode: MeetingReadMode,
    record: &MeetingRecord,
    closed_total: i64,
    span_from: Option<i64>,
    span_to: Option<i64>,
    receipt: Option<String>,
    continue_token: Option<String>,
    notes: Vec<String>,
) -> MeetingReadMeta {
    MeetingReadMeta {
        mode: mode.as_str().to_owned(),
        entity_id: record.id.clone(),
        log_epoch: record.log_epoch,
        status: record.status.clone(),
        status_detail: record.status_detail.clone(),
        end_source: record.end_source.clone(),
        stalled_reason: service.stalled_reason(record),
        content_state: record.content_state.clone(),
        topic: record.topic.clone(),
        started_at: record.started_at.clone(),
        ended_at: record.ended_at.clone(),
        finalized_at: record.finalized_at.clone(),
        content_lost_at: record.content_lost_at.clone(),
        updated_at: record.updated_at.clone(),
        span_from,
        span_to,
        closed_total,
        receipt,
        continue_token,
        notes,
    }
}

fn encode_token(mut token: SnapshotToken) -> Result<String, MeetingError> {
    token.checksum = token_checksum(&token.unsigned())?;
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&token)?))
}

fn decode_token(raw: &str) -> Result<SnapshotToken, MeetingError> {
    let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| {
        MeetingError::named_bad_request("invalid_continue_token", "continue_token is invalid")
    })?;
    serde_json::from_slice(&decoded).map_err(|_| {
        MeetingError::named_bad_request("invalid_continue_token", "continue_token is invalid")
    })
}

fn decode_and_validate_token(
    raw: &str,
    path_entity: &str,
    body_mode: MeetingReadMode,
    now: i64,
) -> Result<SnapshotToken, MeetingError> {
    let token = decode_token(raw)?;
    if token.checksum != token_checksum(&token.unsigned())? {
        return Err(MeetingError::named_bad_request(
            "invalid_continue_token",
            "continue_token checksum is invalid",
        ));
    }
    if token.entity_id != path_entity {
        return Err(MeetingError::named_bad_request(
            "token_entity_mismatch",
            "continue_token belongs to another meeting entity",
        ));
    }
    if token.mode != body_mode {
        return Err(MeetingError::named_bad_request(
            "token_mode_mismatch",
            "continue_token mode does not match request mode",
        ));
    }
    let mode_fields_valid = match token.mode {
        MeetingReadMode::Full => {
            token.origin_range_start == 1
                && token.range_end == token.snapshot.closed_latest
                && token.next_seq <= token.snapshot.closed_latest
        }
        MeetingReadMode::Range => {
            token.next_seq <= token.range_end && token.next_seq <= token.snapshot.closed_latest
        }
        MeetingReadMode::Incremental => false,
    };
    if !mode_fields_valid
        || token.log_epoch < 0
        || token.snapshot.closed_latest <= 0
        || token.snapshot.log_byte_offset == 0
        || token.next_seq <= 0
        || token.origin_range_start <= 0
        || token.range_end < token.origin_range_start
        || token.next_seq < token.origin_range_start
        || token.issued_at < 0
    {
        return Err(MeetingError::named_bad_request(
            "invalid_continue_token",
            "continue_token fields are invalid",
        ));
    }
    if now.saturating_sub(token.issued_at) > TOKEN_INACTIVITY_MILLIS
        || token.issued_at > now.saturating_add(60_000)
    {
        let restart = match token.mode {
            MeetingReadMode::Full => {
                format!("garyx meeting read {} --full", token.entity_id)
            }
            MeetingReadMode::Range => format!(
                "garyx meeting read {} --range {}..{} --epoch {}",
                token.entity_id, token.origin_range_start, token.range_end, token.log_epoch
            ),
            MeetingReadMode::Incremental => unreachable!(),
        };
        return Err(MeetingError::named_bad_request(
            "token_expired",
            "continue_token expired after 10 minutes of inactivity",
        )
        .with_restart_command(restart));
    }
    Ok(token)
}

fn token_checksum(token: &UnsignedSnapshotToken) -> Result<String, MeetingError> {
    let mut hasher = Sha256::new();
    hasher.update(b"garyx-meeting-snapshot-v1\0");
    hasher.update(serde_json::to_vec(token)?);
    Ok(format!("{:x}", hasher.finalize()))
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn continuation_mode_unverified(token: &str) -> Result<MeetingReadMode, MeetingError> {
    Ok(decode_token(token)?.mode)
}

fn map_cursor_domain<T>(result: Result<T, MeetingCursorDomainError>) -> Result<T, MeetingError> {
    result.map_err(|error| match error {
        MeetingCursorDomainError::Deleted => MeetingError::not_found(),
        MeetingCursorDomainError::ContentLoss => MeetingError::content_loss(),
    })
}

pub async fn read_meeting(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Response {
    let request: MeetingReadRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return MeetingError::named_bad_request(
                "invalid_body",
                format!("meeting read body is invalid: {error}"),
            )
            .into_response();
        }
    };
    match state.ops.meetings.read(&id, request).await {
        Ok(response) => json_response(StatusCode::OK, &response),
        Err(error) => error.into_response(),
    }
}

pub async fn confirm_meeting_read(
    AxumPath(id): AxumPath<String>,
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Response {
    let request: ConfirmMeetingReadRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return MeetingError::named_bad_request(
                "invalid_body",
                format!("meeting confirm body is invalid: {error}"),
            )
            .into_response();
        }
    };
    match state.ops.meetings.confirm(&id, request).await {
        Ok(outcome) => json_response(
            StatusCode::OK,
            &serde_json::json!({
                "confirmed": matches!(outcome, MeetingConfirmOutcome::Confirmed),
                "already_confirmed": matches!(outcome, MeetingConfirmOutcome::AlreadyConfirmed),
            }),
        ),
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::garyx_db::{GaryxDbService, MeetingCreateDraft, MeetingStatus};
    use crate::meetings::{SegmentDraft, SegmentKind};

    use super::*;

    #[test]
    fn token_validation_checks_checksum_path_and_mode_before_expiry() {
        let entity = Uuid::now_v7().to_string();
        let old = now_millis() - TOKEN_INACTIVITY_MILLIS - 1;
        let encoded = encode_token(SnapshotToken {
            entity_id: entity.clone(),
            log_epoch: 7,
            snapshot: ReadSnapshot {
                closed_latest: 200,
                log_byte_offset: 10_000,
            },
            next_seq: 150,
            mode: MeetingReadMode::Range,
            origin_range_start: 100,
            range_end: 200,
            checksum: String::new(),
            issued_at: old,
        })
        .expect("token");
        let wrong_path = decode_and_validate_token(
            &encoded,
            &Uuid::now_v7().to_string(),
            MeetingReadMode::Range,
            now_millis(),
        )
        .expect_err("path mismatch");
        assert_eq!(wrong_path.code(), "token_entity_mismatch");
        let wrong_mode =
            decode_and_validate_token(&encoded, &entity, MeetingReadMode::Full, now_millis())
                .expect_err("mode mismatch");
        assert_eq!(wrong_mode.code(), "token_mode_mismatch");
        let expired =
            decode_and_validate_token(&encoded, &entity, MeetingReadMode::Range, now_millis())
                .expect_err("expired");
        assert_eq!(expired.code(), "token_expired");
        assert_eq!(
            expired.restart_command(),
            Some(format!("garyx meeting read {entity} --range 100..200 --epoch 7").as_str())
        );
        assert!(
            encoded
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );

        let non_advancing = encode_token(SnapshotToken {
            entity_id: entity.clone(),
            log_epoch: 7,
            snapshot: ReadSnapshot {
                closed_latest: 200,
                log_byte_offset: 10_000,
            },
            next_seq: 201,
            mode: MeetingReadMode::Range,
            origin_range_start: 100,
            range_end: 200,
            checksum: String::new(),
            issued_at: now_millis(),
        })
        .expect("self-consistent checksum");
        let invalid = decode_and_validate_token(
            &non_advancing,
            &entity,
            MeetingReadMode::Range,
            now_millis(),
        )
        .expect_err("a continuation must always have progress remaining");
        assert_eq!(invalid.code(), "invalid_continue_token");
    }

    #[test]
    fn request_combinations_are_named_errors() {
        let id = Uuid::now_v7().to_string();
        let full_epoch = validate_request(
            &id,
            MeetingReadRequest {
                mode: MeetingReadMode::Full,
                reader_id: None,
                range_start: None,
                range_end: None,
                epoch: Some(0),
                continue_token: None,
                max_bytes: None,
            },
            65_536,
        )
        .expect_err("full epoch");
        assert_eq!(full_epoch.code(), "invalid_epoch");
        let incremental_without_reader = validate_request(
            &id,
            MeetingReadRequest {
                mode: MeetingReadMode::Incremental,
                reader_id: None,
                range_start: None,
                range_end: None,
                epoch: None,
                continue_token: None,
                max_bytes: None,
            },
            65_536,
        )
        .expect_err("reader");
        assert_eq!(incremental_without_reader.code(), "missing_reader_id");

        let stateless_reader = validate_request(
            &id,
            MeetingReadRequest {
                mode: MeetingReadMode::Full,
                reader_id: Some("reader".to_owned()),
                range_start: None,
                range_end: None,
                epoch: None,
                continue_token: None,
                max_bytes: None,
            },
            65_536,
        )
        .expect_err("stateless reader");
        assert_eq!(stateless_reader.code(), "invalid_reader_id");

        let token_with_explicit_range = validate_request(
            &id,
            MeetingReadRequest {
                mode: MeetingReadMode::Range,
                reader_id: None,
                range_start: Some(1),
                range_end: Some(2),
                epoch: Some(0),
                continue_token: Some("opaque".to_owned()),
                max_bytes: None,
            },
            65_536,
        )
        .expect_err("token with explicit range");
        assert_eq!(token_with_explicit_range.code(), "invalid_continue_token");

        for max_bytes in [0, 1, 4_095] {
            let floor = validate_request(
                &id,
                MeetingReadRequest {
                    mode: MeetingReadMode::Full,
                    reader_id: None,
                    range_start: None,
                    range_end: None,
                    epoch: None,
                    continue_token: None,
                    max_bytes: Some(max_bytes),
                },
                65_536,
            )
            .expect_err("budget floor");
            assert_eq!(floor.code(), "invalid_max_bytes");
        }
    }

    #[test]
    fn read_and_confirm_wire_bodies_reject_unknown_fields() {
        assert!(
            serde_json::from_value::<MeetingReadRequest>(serde_json::json!({
                "mode": "full",
                "render_mode": "human"
            }))
            .is_err(),
            "the server has no render-mode input"
        );
        assert!(
            serde_json::from_value::<ConfirmMeetingReadRequest>(serde_json::json!({
                "reader_id": "reader",
                "receipt": "receipt",
                "log_epoch": 0,
                "extra": true
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn stateless_continuation_renews_the_sliding_inactivity_window() {
        let temp = tempfile::tempdir().expect("temp");
        let db = Arc::new(GaryxDbService::memory().expect("db"));
        let service = Arc::new(
            MeetingService::new(db.clone(), temp.path().join("meetings"), 4_096).expect("service"),
        );
        let entity = Uuid::now_v7().to_string();
        db.create_meeting(MeetingCreateDraft {
            id: Some(entity.clone()),
            account_id: "test-account".to_owned(),
            meeting_no: "123456789".to_owned(),
            feishu_meeting_id: String::new(),
            invite_event_id: "invite-sliding-token".to_owned(),
            topic: "Sliding token".to_owned(),
            invited_by: "Test User".to_owned(),
            status: MeetingStatus::Live,
            status_detail: String::new(),
            join_deadline_at: "2026-07-16T02:40:00.000Z".to_owned(),
            grace_deadline_at: None,
            started_at: "2026-07-16T02:35:00.000Z".to_owned(),
            ended_at: None,
            finalized_at: None,
            created_at: "2026-07-16T02:35:00.000Z".to_owned(),
        })
        .expect("meeting");
        service
            .append_batch(
                &entity,
                (0..4)
                    .map(|index| SegmentDraft {
                        kind: SegmentKind::Chat,
                        speaker: "Test Speaker".to_owned(),
                        start: "2026-07-16T02:35:00.000Z".to_owned(),
                        end: "2026-07-16T02:35:01.000Z".to_owned(),
                        text: format!("{index}-{}", "x".repeat(2_000)),
                        source_id: format!("source-{index}"),
                    })
                    .collect(),
                "cursor",
            )
            .await
            .expect("content");
        let first = service
            .read(
                &entity,
                MeetingReadRequest {
                    mode: MeetingReadMode::Full,
                    reader_id: None,
                    range_start: None,
                    range_end: None,
                    epoch: None,
                    continue_token: None,
                    max_bytes: Some(4_096),
                },
            )
            .await
            .expect("first page");
        let mut old = decode_token(
            first
                .meta
                .continue_token
                .as_deref()
                .expect("first continuation"),
        )
        .expect("decode first token");
        old.issued_at = now_millis() - (9 * 60 * 1_000);
        let old_issued_at = old.issued_at;
        let old_next = old.next_seq;
        let old_snapshot = old.snapshot;
        let old = encode_token(old).expect("aged but live token");

        let continued = service
            .read(
                &entity,
                MeetingReadRequest {
                    mode: MeetingReadMode::Full,
                    reader_id: None,
                    range_start: None,
                    range_end: None,
                    epoch: None,
                    continue_token: Some(old),
                    max_bytes: Some(4_096),
                },
            )
            .await
            .expect("continuation within inactivity window");
        let fresh = decode_token(
            continued
                .meta
                .continue_token
                .as_deref()
                .expect("renewed continuation"),
        )
        .expect("decode renewed token");
        assert_eq!(fresh.snapshot, old_snapshot);
        assert!(fresh.next_seq > old_next);
        assert!(fresh.issued_at > old_issued_at);
        assert!(now_millis() - fresh.issued_at < 5_000);
    }
}
