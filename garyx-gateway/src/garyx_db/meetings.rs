use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{GaryxDbError, GaryxDbResult, GaryxDbService, now_string};

pub(crate) const MEETINGS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS meetings (
  id                   TEXT NOT NULL PRIMARY KEY,      -- uuid_v7
  account_id           TEXT NOT NULL,
  meeting_no           TEXT NOT NULL,
  feishu_meeting_id    TEXT NOT NULL DEFAULT '',
  invite_event_id      TEXT NOT NULL,
  topic                TEXT NOT NULL DEFAULT '',        -- normalized: ≤256 bytes, UTF-8 boundary
  invited_by           TEXT NOT NULL DEFAULT '',        -- ≤128 bytes
  status               TEXT NOT NULL CHECK (status IN
    ('joining','live','finalizing','aborting','finalized','aborted')),
  status_detail        TEXT NOT NULL DEFAULT '',        -- ≤256 bytes
  content_state        TEXT NOT NULL DEFAULT 'ok' CHECK (content_state IN ('ok','lost')),
  content_lost_at      TEXT,
  -- pairing enforced below with: CHECK ((content_state = 'lost') = (content_lost_at IS NOT NULL))
  failure_kind         TEXT NOT NULL DEFAULT '' CHECK (failure_kind IN
    ('','auth','transport')),
  failure_since        TEXT,
  log_epoch            INTEGER NOT NULL DEFAULT 0 CHECK (log_epoch >= 0),
  cache_generation     INTEGER NOT NULL DEFAULT 0 CHECK (cache_generation >= 0),
  end_source           TEXT NOT NULL DEFAULT '' CHECK (end_source IN
    ('','push','participant_left')),
  join_deadline_at     TEXT NOT NULL,
  grace_deadline_at    TEXT,
  closed_segment_count INTEGER NOT NULL DEFAULT 0 CHECK (closed_segment_count >= 0),
  byte_size            INTEGER NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
  started_at           TEXT NOT NULL,
  ended_at             TEXT,
  finalized_at         TEXT,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL,
  CHECK ((failure_kind = '') = (failure_since IS NULL)),
  CHECK ((content_state = 'lost') = (content_lost_at IS NOT NULL))
) STRICT;
CREATE TABLE IF NOT EXISTS meeting_invite_keys (
  invite_event_id TEXT NOT NULL PRIMARY KEY,
  meeting_id      TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  observed_at     TEXT NOT NULL
) STRICT;
-- meetings.invite_event_id remains as "first admitting event" provenance;
-- durable admission idempotency is owned by meeting_invite_keys (RR14-02).
CREATE UNIQUE INDEX IF NOT EXISTS idx_meetings_active_no
  ON meetings(account_id, meeting_no)
  WHERE status IN ('joining','live','finalizing','aborting');
CREATE UNIQUE INDEX IF NOT EXISTS idx_meetings_active_fid
  ON meetings(account_id, feishu_meeting_id)
  WHERE feishu_meeting_id <> ''
    AND status IN ('joining','live','finalizing','aborting');
CREATE INDEX IF NOT EXISTS idx_meetings_created ON meetings(created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_meetings_status  ON meetings(status);

CREATE TABLE IF NOT EXISTS meeting_read_cursors (
  meeting_id    TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  reader_id     TEXT NOT NULL
    CHECK (length(CAST(reader_id AS BLOB)) BETWEEN 1 AND 128),  -- bytes, not chars
  log_epoch     INTEGER NOT NULL CHECK (log_epoch >= 0),  -- no default: always explicitly inserted from meetings.log_epoch (RR10-01)
  confirmed_seq INTEGER NOT NULL DEFAULT 0 CHECK (confirmed_seq >= 0),
  pending_from  INTEGER,
  pending_to    INTEGER,
  receipt       TEXT,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (meeting_id, reader_id),
  CHECK ((pending_from IS NULL) = (pending_to IS NULL)
     AND (pending_from IS NULL) = (receipt IS NULL)
     AND (pending_from IS NULL OR
          (pending_from > confirmed_seq AND pending_to >= pending_from)))
) STRICT;
"#;

const MEETING_COLUMNS: &str = "
    id, account_id, meeting_no, feishu_meeting_id, invite_event_id,
    topic, invited_by, status, status_detail, content_state, content_lost_at,
    failure_kind, failure_since, log_epoch, cache_generation, end_source,
    join_deadline_at, grace_deadline_at, closed_segment_count,
    byte_size, started_at, ended_at, finalized_at, created_at, updated_at";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Joining,
    Live,
    Finalizing,
    Aborting,
    Finalized,
    Aborted,
}

impl MeetingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Joining => "joining",
            Self::Live => "live",
            Self::Finalizing => "finalizing",
            Self::Aborting => "aborting",
            Self::Finalized => "finalized",
            Self::Aborted => "aborted",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Finalized | Self::Aborted)
    }
}

impl FromStr for MeetingStatus {
    type Err = GaryxDbError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "joining" => Ok(Self::Joining),
            "live" => Ok(Self::Live),
            "finalizing" => Ok(Self::Finalizing),
            "aborting" => Ok(Self::Aborting),
            "finalized" => Ok(Self::Finalized),
            "aborted" => Ok(Self::Aborted),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid meeting status in database: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingRecord {
    pub id: String,
    pub account_id: String,
    pub meeting_no: String,
    pub feishu_meeting_id: String,
    pub invite_event_id: String,
    pub topic: String,
    pub invited_by: String,
    pub status: String,
    pub status_detail: String,
    pub content_state: String,
    pub content_lost_at: Option<String>,
    pub failure_kind: String,
    pub failure_since: Option<String>,
    pub log_epoch: i64,
    pub cache_generation: i64,
    pub end_source: String,
    pub join_deadline_at: String,
    pub grace_deadline_at: Option<String>,
    pub closed_segment_count: i64,
    pub byte_size: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub finalized_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl MeetingRecord {
    pub fn parsed_status(&self) -> GaryxDbResult<MeetingStatus> {
        self.status.parse()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingCreateDraft {
    pub id: Option<String>,
    pub account_id: String,
    pub meeting_no: String,
    pub feishu_meeting_id: String,
    pub invite_event_id: String,
    pub topic: String,
    pub invited_by: String,
    pub status: MeetingStatus,
    pub status_detail: String,
    pub join_deadline_at: String,
    pub grace_deadline_at: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub finalized_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingReadCursor {
    pub meeting_id: String,
    pub reader_id: String,
    pub log_epoch: i64,
    pub confirmed_seq: i64,
    pub pending_from: Option<i64>,
    pub pending_to: Option<i64>,
    pub receipt: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedMeetingCursor {
    pub cursor: MeetingReadCursor,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingCursorDomainError {
    Deleted,
    ContentLoss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingCursorClaimOutcome {
    Claimed(MeetingReadCursor),
    Winner(MeetingReadCursor),
    Retry(MeetingReadCursor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingConfirmOutcome {
    Confirmed,
    AlreadyConfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMeetingRowOutcome {
    Deleted,
    NotFound,
    NotTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingAdmissionDraft {
    pub account_id: String,
    pub meeting_no: String,
    pub invite_event_id: String,
    pub topic: String,
    pub invited_by: String,
    pub join_deadline_at: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingAdmissionOutcome {
    Existing(MeetingRecord),
    Created(MeetingRecord),
}

impl GaryxDbService {
    pub fn create_meeting(&self, draft: MeetingCreateDraft) -> GaryxDbResult<MeetingRecord> {
        let id = match draft.id.as_deref() {
            Some(id) => normalize_meeting_id(id)?,
            None => Uuid::now_v7().to_string(),
        };
        let account_id = normalize_required("account_id", &draft.account_id)?;
        let meeting_no = normalize_required("meeting_no", &draft.meeting_no)?;
        let invite_event_id = normalize_required("invite_event_id", &draft.invite_event_id)?;
        let feishu_meeting_id = draft.feishu_meeting_id.trim().to_owned();
        let topic = truncate_utf8(draft.topic.trim(), 256);
        let invited_by = truncate_utf8(draft.invited_by.trim(), 128);
        let status_detail = truncate_utf8(draft.status_detail.trim(), 256);
        let join_deadline_at = normalize_timestamp("join_deadline_at", &draft.join_deadline_at)?;
        let grace_deadline_at =
            normalize_optional_timestamp("grace_deadline_at", draft.grace_deadline_at.as_deref())?;
        let started_at = normalize_timestamp("started_at", &draft.started_at)?;
        let ended_at = normalize_optional_timestamp("ended_at", draft.ended_at.as_deref())?;
        let finalized_at =
            normalize_optional_timestamp("finalized_at", draft.finalized_at.as_deref())?;
        let created_at = normalize_timestamp("created_at", &draft.created_at)?;

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO meetings (
                id, account_id, meeting_no, feishu_meeting_id, invite_event_id,
                topic, invited_by, status, status_detail,
                join_deadline_at, grace_deadline_at, started_at, ended_at,
                finalized_at, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?15
             )",
            params![
                id,
                account_id,
                meeting_no,
                feishu_meeting_id,
                invite_event_id,
                topic,
                invited_by,
                draft.status.as_str(),
                status_detail,
                join_deadline_at,
                grace_deadline_at,
                started_at,
                ended_at,
                finalized_at,
                created_at,
            ],
        )?;
        tx.execute(
            "INSERT INTO meeting_invite_keys (invite_event_id, meeting_id, observed_at)
             VALUES (?1, ?2, ?3)",
            params![invite_event_id, id, created_at],
        )?;
        let record = meeting_by_id_tx(&tx, &id)?.ok_or_else(|| {
            GaryxDbError::Configuration("meeting insert did not produce a row".to_owned())
        })?;
        tx.commit()?;
        Ok(record)
    }

    /// Atomically records one invitation delivery and either links it to an
    /// already-active entity or creates a new joining entity.
    pub fn admit_meeting_invite(
        &self,
        draft: MeetingAdmissionDraft,
    ) -> GaryxDbResult<MeetingAdmissionOutcome> {
        let account_id = normalize_required("account_id", &draft.account_id)?;
        let meeting_no = normalize_required("meeting_no", &draft.meeting_no)?;
        let invite_event_id = normalize_required("invite_event_id", &draft.invite_event_id)?;
        let topic = truncate_utf8(draft.topic.trim(), 256);
        let invited_by = truncate_utf8(draft.invited_by.trim(), 128);
        let join_deadline_at = normalize_timestamp("join_deadline_at", &draft.join_deadline_at)?;
        let observed_at = normalize_timestamp("observed_at", &draft.observed_at)?;

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        if let Some(record) = meeting_by_invite_key_tx(&tx, &invite_event_id)? {
            tx.commit()?;
            return Ok(MeetingAdmissionOutcome::Existing(record));
        }

        let active_sql = format!(
            "SELECT {MEETING_COLUMNS} FROM meetings
             WHERE account_id = ?1 AND meeting_no = ?2
               AND status IN ('joining','live','finalizing','aborting')
             LIMIT 1"
        );
        if let Some(record) = tx
            .query_row(
                &active_sql,
                params![account_id, meeting_no],
                meeting_from_row,
            )
            .optional()?
        {
            tx.execute(
                "INSERT INTO meeting_invite_keys (invite_event_id, meeting_id, observed_at)
                 VALUES (?1, ?2, ?3)",
                params![invite_event_id, record.id, observed_at],
            )?;
            tx.commit()?;
            return Ok(MeetingAdmissionOutcome::Existing(record));
        }

        let id = Uuid::now_v7().to_string();
        tx.execute(
            "INSERT INTO meetings (
                id, account_id, meeting_no, invite_event_id, topic, invited_by,
                status, join_deadline_at, started_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'joining', ?7, ?8, ?8, ?8)",
            params![
                id,
                account_id,
                meeting_no,
                invite_event_id,
                topic,
                invited_by,
                join_deadline_at,
                observed_at,
            ],
        )?;
        tx.execute(
            "INSERT INTO meeting_invite_keys (invite_event_id, meeting_id, observed_at)
             VALUES (?1, ?2, ?3)",
            params![invite_event_id, id, observed_at],
        )?;
        let record = meeting_by_id_tx(&tx, &id)?.ok_or_else(|| {
            GaryxDbError::Configuration("meeting admission insert produced no row".to_owned())
        })?;
        tx.commit()?;
        Ok(MeetingAdmissionOutcome::Created(record))
    }

    pub fn insert_meeting_invite_key(
        &self,
        meeting_id: &str,
        invite_event_id: &str,
        observed_at: &str,
    ) -> GaryxDbResult<bool> {
        let meeting_id = normalize_meeting_id(meeting_id)?;
        let invite_event_id = normalize_required("invite_event_id", invite_event_id)?;
        let observed_at = normalize_timestamp("observed_at", observed_at)?;
        let conn = self.conn()?;
        let inserted = conn.execute(
            "INSERT INTO meeting_invite_keys (invite_event_id, meeting_id, observed_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(invite_event_id) DO NOTHING",
            params![invite_event_id, meeting_id, observed_at],
        )?;
        Ok(inserted > 0)
    }

    pub fn get_meeting(&self, id: &str) -> GaryxDbResult<Option<MeetingRecord>> {
        let id = normalize_meeting_id(id)?;
        let conn = self.read_conn()?;
        meeting_by_id_conn(&conn, &id)
    }

    pub fn get_meeting_by_invite_event_id(
        &self,
        invite_event_id: &str,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        let invite_event_id = normalize_required("invite_event_id", invite_event_id)?;
        let conn = self.read_conn()?;
        let sql = format!(
            "SELECT {MEETING_COLUMNS}
               FROM meetings
              WHERE id = (
                    SELECT meeting_id
                      FROM meeting_invite_keys
                     WHERE invite_event_id = ?1
              )"
        );
        conn.query_row(&sql, params![invite_event_id], meeting_from_row)
            .optional()
            .map_err(Into::into)
    }

    pub fn list_all_meetings(&self) -> GaryxDbResult<Vec<MeetingRecord>> {
        let conn = self.read_conn()?;
        let sql =
            format!("SELECT {MEETING_COLUMNS} FROM meetings ORDER BY created_at DESC, id DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], meeting_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_non_terminal_meetings(&self) -> GaryxDbResult<Vec<MeetingRecord>> {
        let conn = self.read_conn()?;
        let sql = format!(
            "SELECT {MEETING_COLUMNS}
               FROM meetings
              WHERE status IN ('joining','live','finalizing','aborting')
              ORDER BY created_at DESC, id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], meeting_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_non_terminal_meetings_for_account(
        &self,
        account_id: &str,
    ) -> GaryxDbResult<Vec<MeetingRecord>> {
        let account_id = normalize_required("account_id", account_id)?;
        let conn = self.read_conn()?;
        let sql = format!(
            "SELECT {MEETING_COLUMNS} FROM meetings
             WHERE account_id = ?1
               AND status IN ('joining','live','finalizing','aborting')
             ORDER BY created_at DESC, id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account_id], meeting_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_active_meeting_by_feishu_id(
        &self,
        account_id: &str,
        feishu_meeting_id: &str,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        let account_id = normalize_required("account_id", account_id)?;
        let feishu_meeting_id = normalize_required("feishu_meeting_id", feishu_meeting_id)?;
        let conn = self.read_conn()?;
        let sql = format!(
            "SELECT {MEETING_COLUMNS} FROM meetings
             WHERE account_id = ?1 AND feishu_meeting_id = ?2
               AND status IN ('joining','live','finalizing','aborting')
             LIMIT 1"
        );
        conn.query_row(
            &sql,
            params![account_id, feishu_meeting_id],
            meeting_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_meetings_page(
        &self,
        limit: usize,
        after: Option<(String, String)>,
    ) -> GaryxDbResult<Vec<MeetingRecord>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let conn = self.read_conn()?;
        if let Some((created_at, id)) = after {
            let created_at = normalize_timestamp("page_token.created_at", &created_at)?;
            let id = normalize_meeting_id(&id)?;
            let sql = format!(
                "SELECT {MEETING_COLUMNS}
                   FROM meetings
                  WHERE created_at < ?1 OR (created_at = ?1 AND id < ?2)
                  ORDER BY created_at DESC, id DESC
                  LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![created_at, id, limit], meeting_from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let sql = format!(
                "SELECT {MEETING_COLUMNS}
                   FROM meetings
                  ORDER BY created_at DESC, id DESC
                  LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![limit], meeting_from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn transition_meeting_status(
        &self,
        id: &str,
        expected_status: MeetingStatus,
        status: MeetingStatus,
        status_detail: &str,
        end_source: &str,
        ended_at: Option<&str>,
        finalized_at: Option<&str>,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        let id = normalize_meeting_id(id)?;
        let status_detail = truncate_utf8(status_detail.trim(), 256);
        let end_source = normalize_end_source(end_source)?;
        let ended_at = normalize_optional_timestamp("ended_at", ended_at)?;
        let finalized_at = normalize_optional_timestamp("finalized_at", finalized_at)?;
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE meetings
                SET status = ?2,
                    status_detail = ?3,
                    end_source = ?4,
                    ended_at = ?5,
                    finalized_at = ?6,
                    failure_kind = CASE
                        WHEN ?2 IN ('aborting','finalized','aborted') THEN ''
                        ELSE failure_kind
                    END,
                    failure_since = CASE
                        WHEN ?2 IN ('aborting','finalized','aborted') THEN NULL
                        ELSE failure_since
                    END,
                    updated_at = ?7
              WHERE id = ?1 AND status = ?8",
            params![
                id,
                status.as_str(),
                status_detail,
                end_source,
                ended_at,
                finalized_at,
                now_string(),
                expected_status.as_str(),
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        meeting_by_id_conn(&conn, &id)
    }

    pub fn mark_meeting_live(
        &self,
        id: &str,
        feishu_meeting_id: &str,
        topic: &str,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        let id = normalize_meeting_id(id)?;
        let feishu_meeting_id = normalize_required("feishu_meeting_id", feishu_meeting_id)?;
        let topic = truncate_utf8(topic.trim(), 256);
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE meetings
                SET status = 'live',
                    feishu_meeting_id = ?2,
                    topic = CASE WHEN ?3 = '' THEN topic ELSE ?3 END,
                    failure_kind = '',
                    failure_since = NULL,
                    updated_at = ?4
              WHERE id = ?1 AND status = 'joining'",
            params![id, feishu_meeting_id, topic, now_string()],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        meeting_by_id_conn(&conn, &id)
    }

    pub fn begin_meeting_finalizing(
        &self,
        id: &str,
        source: &str,
        ended_at: &str,
        grace_deadline_at: &str,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        let id = normalize_meeting_id(id)?;
        let source = normalize_end_source(source)?;
        if source.is_empty() {
            return Err(GaryxDbError::BadRequest(
                "finalizing requires an end source".to_owned(),
            ));
        }
        let ended_at = normalize_timestamp("ended_at", ended_at)?;
        let grace_deadline_at = normalize_timestamp("grace_deadline_at", grace_deadline_at)?;
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE meetings
                SET status = 'finalizing',
                    end_source = ?2,
                    ended_at = ?3,
                    grace_deadline_at = ?4,
                    updated_at = ?3
              WHERE id = ?1 AND status = 'live'",
            params![id, source, ended_at, grace_deadline_at],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        meeting_by_id_conn(&conn, &id)
    }

    pub fn begin_meeting_abort(
        &self,
        id: &str,
        expected: MeetingStatus,
        detail: &str,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        if !matches!(expected, MeetingStatus::Joining | MeetingStatus::Live) {
            return Err(GaryxDbError::BadRequest(
                "abort may start only from joining or live".to_owned(),
            ));
        }
        self.transition_meeting_status(
            id,
            expected,
            MeetingStatus::Aborting,
            detail,
            "",
            None,
            None,
        )
    }

    pub fn complete_meeting_terminal(
        &self,
        id: &str,
        expected: MeetingStatus,
        terminal: MeetingStatus,
        finalized_at: &str,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        if !matches!(
            (expected, terminal),
            (MeetingStatus::Finalizing, MeetingStatus::Finalized)
                | (MeetingStatus::Aborting, MeetingStatus::Aborted)
        ) {
            return Err(GaryxDbError::BadRequest(
                "invalid terminal meeting transition".to_owned(),
            ));
        }
        let current = self
            .get_meeting(id)?
            .ok_or_else(|| GaryxDbError::BadRequest("meeting entity was deleted".to_owned()))?;
        self.transition_meeting_status(
            id,
            expected,
            terminal,
            &current.status_detail,
            &current.end_source,
            current.ended_at.as_deref(),
            Some(finalized_at),
        )
    }

    pub fn record_meeting_failure(&self, id: &str, kind: &str) -> GaryxDbResult<()> {
        let id = normalize_meeting_id(id)?;
        if !matches!(kind, "auth" | "transport") {
            return Err(GaryxDbError::BadRequest(
                "failure kind must be auth or transport".to_owned(),
            ));
        }
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE meetings
                SET failure_since = CASE WHEN failure_kind = ?2 THEN failure_since ELSE ?3 END,
                    failure_kind = ?2,
                    updated_at = ?3
              WHERE id = ?1
                AND status IN ('joining','live','finalizing')",
            params![id, kind, now],
        )?;
        Ok(())
    }

    pub fn clear_meeting_failure(&self, id: &str) -> GaryxDbResult<()> {
        let id = normalize_meeting_id(id)?;
        let conn = self.conn()?;
        conn.execute(
            "UPDATE meetings
                SET failure_kind = '', failure_since = NULL, updated_at = ?2
              WHERE id = ?1 AND (failure_kind <> '' OR failure_since IS NOT NULL)",
            params![id, now_string()],
        )?;
        Ok(())
    }

    pub fn clear_meeting_failures_for_account(&self, account_id: &str) -> GaryxDbResult<()> {
        let account_id = normalize_required("account_id", account_id)?;
        let conn = self.conn()?;
        conn.execute(
            "UPDATE meetings
                SET failure_kind = '', failure_since = NULL, updated_at = ?2
              WHERE account_id = ?1
                AND status IN ('joining','live','finalizing')
                AND (failure_kind <> '' OR failure_since IS NOT NULL)",
            params![account_id, now_string()],
        )?;
        Ok(())
    }

    pub fn update_meeting_cache_guarded(
        &self,
        id: &str,
        epoch: i64,
        generation: i64,
        closed_segment_count: i64,
        byte_size: i64,
    ) -> GaryxDbResult<bool> {
        let id = normalize_meeting_id(id)?;
        validate_non_negative("log_epoch", epoch)?;
        validate_non_negative("cache_generation", generation)?;
        validate_non_negative("closed_segment_count", closed_segment_count)?;
        validate_non_negative("byte_size", byte_size)?;
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE meetings
                SET closed_segment_count = ?4,
                    byte_size = ?5,
                    cache_generation = ?3,
                    updated_at = ?6
              WHERE id = ?1
                AND log_epoch = ?2
                AND cache_generation < ?3",
            params![
                id,
                epoch,
                generation,
                closed_segment_count,
                byte_size,
                now_string(),
            ],
        )?;
        Ok(updated > 0)
    }

    pub fn rollover_missing_meeting_log(
        &self,
        id: &str,
        expected_epoch: i64,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        let id = normalize_meeting_id(id)?;
        validate_non_negative("expected_epoch", expected_epoch)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let Some(current) = meeting_by_id_tx(&tx, &id)? else {
            tx.commit()?;
            return Ok(None);
        };
        if current.log_epoch != expected_epoch || current.cache_generation == 0 {
            tx.commit()?;
            return Ok(Some(current));
        }
        let new_epoch = expected_epoch.checked_add(1).ok_or_else(|| {
            GaryxDbError::Configuration(format!("meeting {id} exhausted log epochs"))
        })?;
        let updated = tx.execute(
            "UPDATE meetings
                SET log_epoch = ?3,
                    cache_generation = 0,
                    closed_segment_count = 0,
                    byte_size = 0,
                    updated_at = ?4
              WHERE id = ?1
                AND log_epoch = ?2
                AND cache_generation > 0",
            params![id, expected_epoch, new_epoch, now_string()],
        )?;
        if updated > 0 {
            tx.execute(
                "UPDATE meeting_read_cursors
                    SET log_epoch = ?2,
                        confirmed_seq = 0,
                        pending_from = NULL,
                        pending_to = NULL,
                        receipt = NULL,
                        updated_at = ?3
                  WHERE meeting_id = ?1",
                params![id, new_epoch, now_string()],
            )?;
        }
        let result = meeting_by_id_tx(&tx, &id)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn mark_meeting_content_lost(
        &self,
        id: &str,
        lost_at: &str,
    ) -> GaryxDbResult<Option<MeetingRecord>> {
        let id = normalize_meeting_id(id)?;
        let lost_at = normalize_timestamp("content_lost_at", lost_at)?;
        let conn = self.conn()?;
        conn.execute(
            "UPDATE meetings
                SET content_state = 'lost',
                    content_lost_at = COALESCE(content_lost_at, ?2),
                    updated_at = CASE
                        WHEN content_lost_at IS NULL THEN ?2
                        ELSE updated_at
                    END
              WHERE id = ?1",
            params![id, lost_at],
        )?;
        meeting_by_id_conn(&conn, &id)
    }

    pub fn delete_terminal_meeting_row(&self, id: &str) -> GaryxDbResult<DeleteMeetingRowOutcome> {
        let id = normalize_meeting_id(id)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM meetings WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(status) = status else {
            tx.commit()?;
            return Ok(DeleteMeetingRowOutcome::NotFound);
        };
        if !MeetingStatus::from_str(&status)?.is_terminal() {
            tx.commit()?;
            return Ok(DeleteMeetingRowOutcome::NotTerminal);
        }
        tx.execute("DELETE FROM meetings WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(DeleteMeetingRowOutcome::Deleted)
    }

    pub fn prepare_meeting_cursor(
        &self,
        meeting_id: &str,
        reader_id: &str,
        snapshot_epoch: i64,
    ) -> GaryxDbResult<Result<PreparedMeetingCursor, MeetingCursorDomainError>> {
        let meeting_id = normalize_meeting_id(meeting_id)?;
        let reader_id = normalize_reader_id(reader_id)?;
        validate_non_negative("snapshot_epoch", snapshot_epoch)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let Some(entity_epoch) = meeting_epoch_tx(&tx, &meeting_id)? else {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::Deleted));
        };
        if entity_epoch != snapshot_epoch {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::ContentLoss));
        }

        if let Some(cursor) = cursor_by_id_tx(&tx, &meeting_id, &reader_id)? {
            let result = if cursor.log_epoch == snapshot_epoch {
                Ok(PreparedMeetingCursor {
                    cursor,
                    created: false,
                })
            } else {
                Err(MeetingCursorDomainError::ContentLoss)
            };
            tx.commit()?;
            return Ok(result);
        }

        let inserted = tx.execute(
            "INSERT INTO meeting_read_cursors (
                meeting_id, reader_id, log_epoch, confirmed_seq, updated_at
             )
             SELECT ?1, ?2, log_epoch, 0, ?4
               FROM meetings
              WHERE id = ?1 AND log_epoch = ?3
             ON CONFLICT DO NOTHING",
            params![meeting_id, reader_id, snapshot_epoch, now_string()],
        )?;
        let Some(cursor) = cursor_by_id_tx(&tx, &meeting_id, &reader_id)? else {
            tx.commit()?;
            return Ok(Err(if inserted == 0 {
                MeetingCursorDomainError::ContentLoss
            } else {
                MeetingCursorDomainError::Deleted
            }));
        };
        if cursor.log_epoch != snapshot_epoch {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::ContentLoss));
        }
        tx.commit()?;
        Ok(Ok(PreparedMeetingCursor {
            cursor,
            created: inserted > 0,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim_meeting_cursor(
        &self,
        meeting_id: &str,
        reader_id: &str,
        snapshot_epoch: i64,
        expected_confirmed: i64,
        pending_from: i64,
        pending_to: i64,
        receipt: &str,
    ) -> GaryxDbResult<Result<MeetingCursorClaimOutcome, MeetingCursorDomainError>> {
        let meeting_id = normalize_meeting_id(meeting_id)?;
        let reader_id = normalize_reader_id(reader_id)?;
        validate_non_negative("snapshot_epoch", snapshot_epoch)?;
        validate_non_negative("expected_confirmed", expected_confirmed)?;
        if pending_from <= expected_confirmed || pending_to < pending_from {
            return Err(GaryxDbError::BadRequest(
                "invalid pending cursor span".to_owned(),
            ));
        }
        let receipt = normalize_required("receipt", receipt)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let Some(entity_epoch) = meeting_epoch_tx(&tx, &meeting_id)? else {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::Deleted));
        };
        if entity_epoch != snapshot_epoch {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::ContentLoss));
        }
        let updated = tx.execute(
            "UPDATE meeting_read_cursors
                SET pending_from = ?5,
                    pending_to = ?6,
                    receipt = ?7,
                    updated_at = ?8
              WHERE meeting_id = ?1
                AND reader_id = ?2
                AND log_epoch = ?3
                AND confirmed_seq = ?4
                AND pending_from IS NULL",
            params![
                meeting_id,
                reader_id,
                snapshot_epoch,
                expected_confirmed,
                pending_from,
                pending_to,
                receipt,
                now_string(),
            ],
        )?;
        let Some(cursor) = cursor_by_id_tx(&tx, &meeting_id, &reader_id)? else {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::Deleted));
        };
        if cursor.log_epoch != snapshot_epoch {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::ContentLoss));
        }
        let outcome = if updated > 0 {
            MeetingCursorClaimOutcome::Claimed(cursor)
        } else if cursor.pending_from.is_some() {
            MeetingCursorClaimOutcome::Winner(cursor)
        } else {
            MeetingCursorClaimOutcome::Retry(cursor)
        };
        tx.commit()?;
        Ok(Ok(outcome))
    }

    pub fn confirm_meeting_cursor(
        &self,
        meeting_id: &str,
        reader_id: &str,
        receipt: &str,
        request_epoch: i64,
    ) -> GaryxDbResult<Result<MeetingConfirmOutcome, MeetingCursorDomainError>> {
        let meeting_id = normalize_meeting_id(meeting_id)?;
        let reader_id = normalize_reader_id(reader_id)?;
        let receipt = normalize_required("receipt", receipt)?;
        validate_non_negative("log_epoch", request_epoch)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let Some(entity_epoch) = meeting_epoch_tx(&tx, &meeting_id)? else {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::Deleted));
        };
        if entity_epoch != request_epoch {
            tx.commit()?;
            return Ok(Err(MeetingCursorDomainError::ContentLoss));
        }
        let updated = tx.execute(
            "UPDATE meeting_read_cursors
                SET confirmed_seq = pending_to,
                    pending_from = NULL,
                    pending_to = NULL,
                    receipt = NULL,
                    updated_at = ?5
              WHERE meeting_id = ?1
                AND reader_id = ?2
                AND receipt = ?3
                AND log_epoch = ?4",
            params![meeting_id, reader_id, receipt, request_epoch, now_string(),],
        )?;
        tx.commit()?;
        Ok(Ok(if updated > 0 {
            MeetingConfirmOutcome::Confirmed
        } else {
            MeetingConfirmOutcome::AlreadyConfirmed
        }))
    }

    pub fn get_meeting_cursor(
        &self,
        meeting_id: &str,
        reader_id: &str,
    ) -> GaryxDbResult<Option<MeetingReadCursor>> {
        let meeting_id = normalize_meeting_id(meeting_id)?;
        let reader_id = normalize_reader_id(reader_id)?;
        let conn = self.read_conn()?;
        cursor_by_id_conn(&conn, &meeting_id, &reader_id)
    }

    pub fn count_meeting_cursors(&self, meeting_id: &str) -> GaryxDbResult<usize> {
        let meeting_id = normalize_meeting_id(meeting_id)?;
        let conn = self.read_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_read_cursors WHERE meeting_id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn count_meeting_invite_keys(&self, meeting_id: &str) -> GaryxDbResult<usize> {
        let meeting_id = normalize_meeting_id(meeting_id)?;
        let conn = self.read_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_invite_keys WHERE meeting_id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }
}

fn meeting_by_id_conn(
    conn: &rusqlite::Connection,
    id: &str,
) -> GaryxDbResult<Option<MeetingRecord>> {
    let sql = format!("SELECT {MEETING_COLUMNS} FROM meetings WHERE id = ?1");
    conn.query_row(&sql, params![id], meeting_from_row)
        .optional()
        .map_err(Into::into)
}

fn meeting_by_id_tx(tx: &Transaction<'_>, id: &str) -> GaryxDbResult<Option<MeetingRecord>> {
    let sql = format!("SELECT {MEETING_COLUMNS} FROM meetings WHERE id = ?1");
    tx.query_row(&sql, params![id], meeting_from_row)
        .optional()
        .map_err(Into::into)
}

fn meeting_by_invite_key_tx(
    tx: &Transaction<'_>,
    invite_event_id: &str,
) -> GaryxDbResult<Option<MeetingRecord>> {
    let sql = format!(
        "SELECT {MEETING_COLUMNS} FROM meetings
         WHERE id = (SELECT meeting_id FROM meeting_invite_keys WHERE invite_event_id = ?1)"
    );
    tx.query_row(&sql, params![invite_event_id], meeting_from_row)
        .optional()
        .map_err(Into::into)
}

fn meeting_epoch_tx(tx: &Transaction<'_>, id: &str) -> GaryxDbResult<Option<i64>> {
    tx.query_row(
        "SELECT log_epoch FROM meetings WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn meeting_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingRecord> {
    Ok(MeetingRecord {
        id: row.get(0)?,
        account_id: row.get(1)?,
        meeting_no: row.get(2)?,
        feishu_meeting_id: row.get(3)?,
        invite_event_id: row.get(4)?,
        topic: row.get(5)?,
        invited_by: row.get(6)?,
        status: row.get(7)?,
        status_detail: row.get(8)?,
        content_state: row.get(9)?,
        content_lost_at: row.get(10)?,
        failure_kind: row.get(11)?,
        failure_since: row.get(12)?,
        log_epoch: row.get(13)?,
        cache_generation: row.get(14)?,
        end_source: row.get(15)?,
        join_deadline_at: row.get(16)?,
        grace_deadline_at: row.get(17)?,
        closed_segment_count: row.get(18)?,
        byte_size: row.get(19)?,
        started_at: row.get(20)?,
        ended_at: row.get(21)?,
        finalized_at: row.get(22)?,
        created_at: row.get(23)?,
        updated_at: row.get(24)?,
    })
}

fn cursor_by_id_conn(
    conn: &rusqlite::Connection,
    meeting_id: &str,
    reader_id: &str,
) -> GaryxDbResult<Option<MeetingReadCursor>> {
    conn.query_row(
        "SELECT meeting_id, reader_id, log_epoch, confirmed_seq,
                pending_from, pending_to, receipt, updated_at
           FROM meeting_read_cursors
          WHERE meeting_id = ?1 AND reader_id = ?2",
        params![meeting_id, reader_id],
        cursor_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn cursor_by_id_tx(
    tx: &Transaction<'_>,
    meeting_id: &str,
    reader_id: &str,
) -> GaryxDbResult<Option<MeetingReadCursor>> {
    tx.query_row(
        "SELECT meeting_id, reader_id, log_epoch, confirmed_seq,
                pending_from, pending_to, receipt, updated_at
           FROM meeting_read_cursors
          WHERE meeting_id = ?1 AND reader_id = ?2",
        params![meeting_id, reader_id],
        cursor_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn cursor_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingReadCursor> {
    Ok(MeetingReadCursor {
        meeting_id: row.get(0)?,
        reader_id: row.get(1)?,
        log_epoch: row.get(2)?,
        confirmed_seq: row.get(3)?,
        pending_from: row.get(4)?,
        pending_to: row.get(5)?,
        receipt: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

pub(crate) fn normalize_meeting_id(id: &str) -> GaryxDbResult<String> {
    let trimmed = id.trim();
    Uuid::parse_str(trimmed)
        .map(|uuid| uuid.to_string())
        .map_err(|_| GaryxDbError::BadRequest("meeting id must be a UUID".to_owned()))
}

pub(crate) fn normalize_reader_id(reader_id: &str) -> GaryxDbResult<String> {
    let trimmed = reader_id.trim();
    if !(1..=128).contains(&trimmed.len()) {
        return Err(GaryxDbError::BadRequest(
            "reader_id must be between 1 and 128 bytes".to_owned(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn normalize_required(field: &str, value: &str) -> GaryxDbResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(GaryxDbError::BadRequest(format!(
            "{field} must not be empty"
        )));
    }
    Ok(trimmed.to_owned())
}

fn normalize_timestamp(field: &str, value: &str) -> GaryxDbResult<String> {
    let trimmed = value.trim();
    if trimmed.len() != 24 || !trimmed.ends_with('Z') {
        return Err(GaryxDbError::BadRequest(format!(
            "{field} must be fixed-width RFC3339 milliseconds UTC"
        )));
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed).map_err(|_| {
        GaryxDbError::BadRequest(format!(
            "{field} must be fixed-width RFC3339 milliseconds UTC"
        ))
    })?;
    Ok(parsed
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn normalize_optional_timestamp(field: &str, value: Option<&str>) -> GaryxDbResult<Option<String>> {
    value
        .map(|value| normalize_timestamp(field, value))
        .transpose()
}

fn normalize_end_source(value: &str) -> GaryxDbResult<String> {
    match value.trim() {
        "" | "push" | "participant_left" => Ok(value.trim().to_owned()),
        _ => Err(GaryxDbError::BadRequest(
            "end_source must be empty, push, or participant_left".to_owned(),
        )),
    }
}

fn validate_non_negative(field: &str, value: i64) -> GaryxDbResult<()> {
    if value < 0 {
        return Err(GaryxDbError::BadRequest(format!(
            "{field} must be non-negative"
        )));
    }
    Ok(())
}

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;

    fn timestamp(second: u8) -> String {
        format!("2026-07-16T02:35:{second:02}.123Z")
    }

    fn draft(id: Option<String>, invite: &str) -> MeetingCreateDraft {
        MeetingCreateDraft {
            id,
            account_id: "test-account".to_owned(),
            meeting_no: "123456789".to_owned(),
            feishu_meeting_id: String::new(),
            invite_event_id: invite.to_owned(),
            topic: "Synthetic meeting".to_owned(),
            invited_by: "Test User".to_owned(),
            status: MeetingStatus::Joining,
            status_detail: String::new(),
            join_deadline_at: timestamp(59),
            grace_deadline_at: None,
            started_at: timestamp(0),
            ended_at: None,
            finalized_at: None,
            created_at: timestamp(0),
        }
    }

    fn admission(event_id: &str) -> MeetingAdmissionDraft {
        MeetingAdmissionDraft {
            account_id: "admission-account".to_owned(),
            meeting_no: "123456789".to_owned(),
            invite_event_id: event_id.to_owned(),
            topic: "Synthetic admission".to_owned(),
            invited_by: "Test User".to_owned(),
            join_deadline_at: timestamp(59),
            observed_at: timestamp(0),
        }
    }

    fn ddl_from_design() -> String {
        let design = include_str!("../../../docs/design/feishu-meeting-entity.md");
        let section = design
            .split("### 4.1 SQLite DDL (normative)")
            .nth(1)
            .expect("DDL section");
        section
            .split("```sql")
            .nth(1)
            .expect("SQL fence")
            .split("```")
            .next()
            .expect("SQL body")
            .trim()
            .to_owned()
    }

    #[test]
    fn normative_design_ddl_executes_twice_and_service_reopens_with_cascades() {
        let temp = tempdir().expect("temp dir");
        let path = temp.path().join("garyx-db.sqlite3");
        let ddl = ddl_from_design();
        assert_eq!(
            MEETINGS_DDL.trim(),
            ddl,
            "the runtime DDL must remain byte-for-byte aligned with the normative design block"
        );
        {
            let conn = Connection::open(&path).expect("open raw database");
            conn.pragma_update(None, "foreign_keys", "ON")
                .expect("foreign keys");
            conn.execute_batch(&ddl).expect("first exact DDL execution");
            conn.execute_batch(&ddl)
                .expect("second exact DDL execution");
        }

        let id = Uuid::now_v7().to_string();
        let db = GaryxDbService::open(&path).expect("service reopen");
        db.create_meeting(draft(Some(id.clone()), "invite-exact-ddl"))
            .expect("meeting");
        db.prepare_meeting_cursor(&id, "reader", 0)
            .expect("prepare")
            .expect("domain");
        db.insert_meeting_invite_key(&id, "invite-second", &timestamp(1))
            .expect("second invite");
        assert_eq!(
            db.get_meeting_by_invite_event_id("invite-exact-ddl")
                .expect("initial invite lookup")
                .expect("initial invite row")
                .id,
            id
        );
        assert_eq!(
            db.get_meeting_by_invite_event_id("invite-second")
                .expect("folded invite lookup")
                .expect("folded invite row")
                .id,
            id
        );
        db.transition_meeting_status(
            &id,
            MeetingStatus::Joining,
            MeetingStatus::Aborted,
            "synthetic abort",
            "",
            Some(&timestamp(2)),
            Some(&timestamp(3)),
        )
        .expect("terminal status");
        assert_eq!(
            db.delete_terminal_meeting_row(&id).expect("delete"),
            DeleteMeetingRowOutcome::Deleted
        );
        assert_eq!(db.count_meeting_cursors(&id).expect("cursor count"), 0);
        assert_eq!(db.count_meeting_invite_keys(&id).expect("invite count"), 0);
        assert!(
            db.get_meeting_by_invite_event_id("invite-second")
                .expect("deleted invite lookup")
                .is_none()
        );

        drop(db);
        let reopened = GaryxDbService::open(&path).expect("second service reopen");
        assert!(reopened.get_meeting(&id).expect("lookup").is_none());
        assert!(fs::metadata(path).is_ok());
    }

    #[test]
    fn normative_checks_reject_pairing_and_measure_reader_ids_as_bytes() {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.pragma_update(None, "foreign_keys", "ON")
            .expect("foreign keys");
        conn.execute_batch(&ddl_from_design()).expect("DDL");
        let id = Uuid::now_v7().to_string();
        let base = [
            id.as_str(),
            "test-account",
            "123456789",
            "invite-check",
            "joining",
            "2026-07-16T02:40:00.000Z",
            "2026-07-16T02:35:00.000Z",
            "2026-07-16T02:35:00.000Z",
        ];
        conn.execute(
            "INSERT INTO meetings (
                id, account_id, meeting_no, invite_event_id, status,
                join_deadline_at, started_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                base[0], base[1], base[2], base[3], base[4], base[5], base[6], base[7]
            ],
        )
        .expect("meeting");
        assert!(
            conn.execute(
                "UPDATE meetings SET content_state='lost', content_lost_at=NULL WHERE id=?1",
                params![id],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE meetings SET content_state='ok', content_lost_at=?2 WHERE id=?1",
                params![id, timestamp(1)],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO meeting_read_cursors (
                    meeting_id, reader_id, log_epoch, updated_at
                 ) VALUES (?1, ?2, 0, ?3)",
                params![id, "😀".repeat(33), timestamp(1)],
            )
            .is_err(),
            "33 emoji are 132 bytes and must violate the 128-byte CHECK"
        );
        conn.execute(
            "INSERT INTO meeting_read_cursors (
                meeting_id, reader_id, log_epoch, updated_at
             ) VALUES (?1, ?2, 0, ?3)",
            params![id, "😀".repeat(32), timestamp(1)],
        )
        .expect("exactly 128 UTF-8 bytes");
    }

    #[test]
    fn meeting_metadata_truncates_every_bounded_field_on_utf8_boundaries() {
        let db = GaryxDbService::memory().expect("db");
        let mut input = draft(None, "invite-bounds");
        input.topic = "😀".repeat(100);
        input.invited_by = "界".repeat(100);
        input.status_detail = "é".repeat(200);
        let record = db.create_meeting(input).expect("meeting");
        for (value, bound) in [
            (&record.topic, 256usize),
            (&record.invited_by, 128usize),
            (&record.status_detail, 256usize),
        ] {
            assert!(value.len() <= bound);
            assert!(std::str::from_utf8(value.as_bytes()).is_ok());
        }
    }

    #[test]
    fn preflight_rollover_claim_is_trichotomized_as_content_loss() {
        let db = GaryxDbService::memory().expect("db");
        let meeting = db
            .create_meeting(draft(None, "invite-epoch-race"))
            .expect("meeting");
        db.update_meeting_cache_guarded(&meeting.id, 0, 1, 1, 100)
            .expect("cache");
        db.prepare_meeting_cursor(&meeting.id, "reader", 0)
            .expect("prepare")
            .expect("prepare domain");
        let rolled = db
            .rollover_missing_meeting_log(&meeting.id, 0)
            .expect("rollover")
            .expect("meeting");
        assert_eq!(rolled.log_epoch, 1);
        let claim = db
            .claim_meeting_cursor(&meeting.id, "reader", 0, 0, 1, 1, "receipt")
            .expect("claim transaction");
        assert_eq!(claim, Err(MeetingCursorDomainError::ContentLoss));
        let cursor = db
            .get_meeting_cursor(&meeting.id, "reader")
            .expect("cursor")
            .expect("recognition");
        assert_eq!(cursor.log_epoch, 1);
        assert_eq!(cursor.confirmed_seq, 0);
        assert!(cursor.pending_from.is_none());
    }

    #[test]
    fn active_identity_indexes_release_only_after_terminal_transition() {
        let db = GaryxDbService::memory().expect("db");
        let mut first = draft(None, "invite-active-one");
        first.feishu_meeting_id = "feishu-meeting".to_owned();
        let first = db.create_meeting(first).expect("first active");
        let mut duplicate = draft(None, "invite-active-two");
        duplicate.feishu_meeting_id = "feishu-meeting".to_owned();
        assert!(
            db.create_meeting(duplicate.clone()).is_err(),
            "an active account/meeting identity must be unique"
        );
        db.transition_meeting_status(
            &first.id,
            MeetingStatus::Joining,
            MeetingStatus::Aborted,
            "done",
            "",
            Some(&timestamp(2)),
            Some(&timestamp(3)),
        )
        .expect("terminal transition");
        let replacement = db
            .create_meeting(duplicate)
            .expect("terminal rows release both partial unique indexes");
        assert_ne!(replacement.id, first.id);
    }

    #[test]
    fn platform_failure_clock_resets_on_kind_change_and_abort_clears_the_pair() {
        let db = GaryxDbService::memory().expect("db");
        let meeting = db
            .create_meeting(draft(None, "invite-failure-clock"))
            .expect("meeting");

        db.record_meeting_failure(&meeting.id, "transport")
            .expect("transport failure");
        let transport = db
            .get_meeting(&meeting.id)
            .expect("record")
            .expect("meeting");
        let transport_since = transport.failure_since.expect("transport clock");
        std::thread::sleep(std::time::Duration::from_millis(5));

        db.record_meeting_failure(&meeting.id, "auth")
            .expect("auth failure");
        let auth = db
            .get_meeting(&meeting.id)
            .expect("record")
            .expect("meeting");
        assert_eq!(auth.failure_kind, "auth");
        assert!(
            auth.failure_since
                .as_deref()
                .is_some_and(|since| since > transport_since.as_str())
        );

        db.begin_meeting_abort(&meeting.id, MeetingStatus::Joining, "synthetic abort")
            .expect("abort CAS")
            .expect("aborting row");
        let aborting = db
            .get_meeting(&meeting.id)
            .expect("record")
            .expect("meeting");
        assert_eq!(aborting.failure_kind, "");
        assert!(aborting.failure_since.is_none());
    }

    #[test]
    fn admission_transaction_links_distinct_active_keys_and_delete_resets_idempotency() {
        let db = GaryxDbService::memory().expect("db");
        let first = match db
            .admit_meeting_invite(admission("invite-admission-one"))
            .expect("first admission")
        {
            MeetingAdmissionOutcome::Created(record) => record,
            MeetingAdmissionOutcome::Existing(_) => panic!("first invite must create"),
        };
        let exact = db
            .admit_meeting_invite(admission("invite-admission-one"))
            .expect("exact redelivery");
        assert!(
            matches!(exact, MeetingAdmissionOutcome::Existing(ref record) if record.id == first.id)
        );
        let distinct = db
            .admit_meeting_invite(admission("invite-admission-two"))
            .expect("distinct delivery for active meeting");
        assert!(
            matches!(distinct, MeetingAdmissionOutcome::Existing(ref record) if record.id == first.id)
        );
        assert_eq!(db.count_meeting_invite_keys(&first.id).expect("keys"), 2);

        db.transition_meeting_status(
            &first.id,
            MeetingStatus::Joining,
            MeetingStatus::Aborted,
            "synthetic terminal",
            "",
            None,
            Some(&timestamp(3)),
        )
        .expect("terminal first");
        let terminal_redelivery = db
            .admit_meeting_invite(admission("invite-admission-one"))
            .expect("terminal redelivery");
        assert!(
            matches!(terminal_redelivery, MeetingAdmissionOutcome::Existing(ref record) if record.id == first.id)
        );

        let second = match db
            .admit_meeting_invite(admission("invite-admission-three"))
            .expect("reinvite after terminal")
        {
            MeetingAdmissionOutcome::Created(record) => record,
            MeetingAdmissionOutcome::Existing(_) => {
                panic!("terminal row must not collapse reinvite")
            }
        };
        db.transition_meeting_status(
            &second.id,
            MeetingStatus::Joining,
            MeetingStatus::Aborted,
            "synthetic terminal",
            "",
            None,
            Some(&timestamp(3)),
        )
        .expect("terminal second");
        assert_eq!(
            db.delete_terminal_meeting_row(&second.id)
                .expect("delete second"),
            DeleteMeetingRowOutcome::Deleted
        );
        assert_eq!(
            db.delete_terminal_meeting_row(&first.id)
                .expect("delete first"),
            DeleteMeetingRowOutcome::Deleted
        );
        let reset = db
            .admit_meeting_invite(admission("invite-admission-one"))
            .expect("same delivery after delete");
        assert!(matches!(reset, MeetingAdmissionOutcome::Created(_)));
    }
}
