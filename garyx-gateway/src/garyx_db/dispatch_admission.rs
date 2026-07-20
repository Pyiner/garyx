use super::*;

pub(crate) const DISPATCH_ADMISSION_MIGRATION_NAME: &str = "dispatch_admission_ledger_v1";
pub(crate) const DISPATCH_ADMISSION_MIGRATION_VERSION: i64 = 1;

const DISPATCH_ADMISSIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS dispatch_admissions (
    scope_identity TEXT NOT NULL,
    scope_epoch INTEGER NOT NULL CHECK (scope_epoch >= 0),
    thread_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('chat_start', 'stream_input')),
    client_intent_id TEXT NOT NULL,
    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version = 1),
    request_fingerprint TEXT NOT NULL,
    admission_state TEXT NOT NULL CHECK (admission_state IN (
        'admitted', 'handoff_started', 'accepted', 'not_dispatched', 'rejected', 'ambiguous'
    )),
    handoff_attempt INTEGER NOT NULL DEFAULT 0 CHECK (handoff_attempt >= 0),
    outcome TEXT CHECK (outcome IS NULL OR outcome IN (
        'started', 'queued_to_active_run', 'no_active_session'
    )),
    requested_run_id TEXT,
    effective_run_id TEXT,
    pending_input_id TEXT,
    result_http_status INTEGER,
    result_error_code TEXT,
    result_error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    handoff_started_at TEXT,
    settled_at TEXT,
    CHECK (
        (scope_epoch = 0 AND scope_identity = '__legacy_api__')
        OR (scope_epoch > 0 AND scope_identity <> '__legacy_api__')
    ),
    CHECK (kind <> 'chat_start' OR requested_run_id IS NOT NULL),
    CHECK (
        outcome <> 'queued_to_active_run'
        OR (effective_run_id IS NOT NULL AND pending_input_id IS NOT NULL)
    ),
    CHECK (outcome <> 'started' OR requested_run_id IS NOT NULL),
    CHECK (
        outcome <> 'no_active_session'
        OR (kind = 'stream_input' AND admission_state = 'not_dispatched')
    ),
    CHECK (
        admission_state NOT IN ('accepted', 'not_dispatched') OR outcome IS NOT NULL
    ),
    CHECK (admission_state <> 'not_dispatched' OR outcome = 'no_active_session'),
    CHECK (
        admission_state <> 'accepted' OR outcome IN ('started', 'queued_to_active_run')
    ),
    CHECK (
        admission_state NOT IN ('handoff_started', 'accepted', 'ambiguous')
        OR handoff_attempt > 0
    ),
    PRIMARY KEY (scope_identity, scope_epoch, thread_id, kind, client_intent_id)
) STRICT;
"#;

const DISPATCH_ADMISSIONS_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_dispatch_admissions_thread_state
    ON dispatch_admissions(thread_id, admission_state);
"#;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DispatchAdmissionKey {
    pub scope_identity: String,
    pub scope_epoch: i64,
    pub thread_id: String,
    pub kind: DispatchAdmissionKind,
    pub client_intent_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DispatchAdmissionKind {
    ChatStart,
    StreamInput,
}

impl DispatchAdmissionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatStart => "chat_start",
            Self::StreamInput => "stream_input",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "chat_start" => Ok(Self::ChatStart),
            "stream_input" => Ok(Self::StreamInput),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid dispatch admission kind '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchAdmissionState {
    Admitted,
    HandoffStarted,
    Accepted,
    NotDispatched,
    Rejected,
    Ambiguous,
}

impl DispatchAdmissionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::HandoffStarted => "handoff_started",
            Self::Accepted => "accepted",
            Self::NotDispatched => "not_dispatched",
            Self::Rejected => "rejected",
            Self::Ambiguous => "ambiguous",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "admitted" => Ok(Self::Admitted),
            "handoff_started" => Ok(Self::HandoffStarted),
            "accepted" => Ok(Self::Accepted),
            "not_dispatched" => Ok(Self::NotDispatched),
            "rejected" => Ok(Self::Rejected),
            "ambiguous" => Ok(Self::Ambiguous),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid dispatch admission state '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DispatchOutcome {
    Started,
    QueuedToActiveRun,
    NoActiveSession,
}

impl DispatchOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::QueuedToActiveRun => "queued_to_active_run",
            Self::NoActiveSession => "no_active_session",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "started" => Ok(Self::Started),
            "queued_to_active_run" => Ok(Self::QueuedToActiveRun),
            "no_active_session" => Ok(Self::NoActiveSession),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid dispatch admission outcome '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DispatchAdmissionRecord {
    pub key: DispatchAdmissionKey,
    pub fingerprint_version: i64,
    pub request_fingerprint: String,
    pub state: DispatchAdmissionState,
    pub handoff_attempt: i64,
    pub outcome: Option<DispatchOutcome>,
    pub requested_run_id: Option<String>,
    pub effective_run_id: Option<String>,
    pub pending_input_id: Option<String>,
    pub result_http_status: Option<i64>,
    pub result_error_code: Option<String>,
    pub result_error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub handoff_started_at: Option<String>,
    pub settled_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewDispatchAdmission<'a> {
    pub key: &'a DispatchAdmissionKey,
    pub request_fingerprint: &'a str,
    pub requested_run_id: Option<&'a str>,
    pub effective_run_id: Option<&'a str>,
    pub pending_input_id: Option<&'a str>,
    pub outcome: Option<DispatchOutcome>,
}

fn canonical_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("CREATE TABLE IF NOT EXISTS", "CREATE TABLE")
        .replace("CREATE INDEX IF NOT EXISTS", "CREATE INDEX")
        .trim_end_matches(';')
        .to_owned()
}

fn validate_dispatch_admission_schema(tx: &Transaction<'_>) -> GaryxDbResult<()> {
    for (kind, name, expected) in [
        (
            "table",
            "dispatch_admissions",
            DISPATCH_ADMISSIONS_TABLE_SQL,
        ),
        (
            "index",
            "idx_dispatch_admissions_thread_state",
            DISPATCH_ADMISSIONS_INDEX_SQL,
        ),
    ] {
        let actual = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
                params![kind, name],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                GaryxDbError::Configuration(format!(
                    "dispatch admission schema is missing {kind} '{name}'"
                ))
            })?;
        if canonical_schema_sql(&actual) != canonical_schema_sql(expected) {
            return Err(GaryxDbError::Configuration(format!(
                "dispatch admission {kind} '{name}' does not match the committed v1 schema"
            )));
        }
    }
    Ok(())
}

pub(super) fn read_dispatch_admission_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(
    String,
    i64,
    String,
    String,
    String,
    i64,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
        row.get(17)?,
        row.get(18)?,
        row.get(19)?,
    ))
}

pub(super) type DispatchAdmissionSqlRow = (
    String,
    i64,
    String,
    String,
    String,
    i64,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
);

pub(super) fn decode_dispatch_admission(
    row: DispatchAdmissionSqlRow,
) -> GaryxDbResult<DispatchAdmissionRecord> {
    Ok(DispatchAdmissionRecord {
        key: DispatchAdmissionKey {
            scope_identity: row.0,
            scope_epoch: row.1,
            thread_id: row.2,
            kind: DispatchAdmissionKind::parse(&row.3)?,
            client_intent_id: row.4,
        },
        fingerprint_version: row.5,
        request_fingerprint: row.6,
        state: DispatchAdmissionState::parse(&row.7)?,
        handoff_attempt: row.8,
        outcome: row.9.as_deref().map(DispatchOutcome::parse).transpose()?,
        requested_run_id: row.10,
        effective_run_id: row.11,
        pending_input_id: row.12,
        result_http_status: row.13,
        result_error_code: row.14,
        result_error_message: row.15,
        created_at: row.16,
        updated_at: row.17,
        handoff_started_at: row.18,
        settled_at: row.19,
    })
}

pub(super) const DISPATCH_ADMISSION_SELECT: &str = "
    SELECT scope_identity, scope_epoch, thread_id, kind, client_intent_id,
           fingerprint_version, request_fingerprint, admission_state,
           handoff_attempt, outcome, requested_run_id, effective_run_id,
           pending_input_id, result_http_status, result_error_code,
           result_error_message, created_at, updated_at, handoff_started_at,
           settled_at
      FROM dispatch_admissions
    WHERE scope_identity = ?1 AND scope_epoch = ?2 AND thread_id = ?3
      AND kind = ?4 AND client_intent_id = ?5";

pub(super) fn insert_dispatch_admission_tx(
    tx: &Transaction<'_>,
    input: &NewDispatchAdmission<'_>,
    now: &str,
) -> GaryxDbResult<()> {
    tx.execute(
        "INSERT INTO dispatch_admissions (
            scope_identity, scope_epoch, thread_id, kind, client_intent_id,
            fingerprint_version, request_fingerprint, admission_state,
            handoff_attempt, outcome, requested_run_id, effective_run_id,
            pending_input_id, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 'admitted', 0, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT(scope_identity, scope_epoch, thread_id, kind, client_intent_id)
         DO NOTHING",
        params![
            input.key.scope_identity,
            input.key.scope_epoch,
            input.key.thread_id,
            input.key.kind.as_str(),
            input.key.client_intent_id,
            input.request_fingerprint,
            input.outcome.map(DispatchOutcome::as_str),
            input.requested_run_id,
            input.effective_run_id,
            input.pending_input_id,
            now,
        ],
    )?;
    Ok(())
}

impl GaryxDbService {
    pub(crate) fn migrate_dispatch_admission_ledger_v1(&self) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let marker = tx
            .query_row(
                "SELECT projection_version FROM projection_states WHERE projection_name = ?1",
                params![DISPATCH_ADMISSION_MIGRATION_NAME],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if marker.is_some_and(|version| version != DISPATCH_ADMISSION_MIGRATION_VERSION) {
            return Err(GaryxDbError::Configuration(format!(
                "dispatch admission marker version mismatch: expected {}, found {}",
                DISPATCH_ADMISSION_MIGRATION_VERSION,
                marker.unwrap_or_default()
            )));
        }
        if marker.is_none() {
            tx.execute_batch(DISPATCH_ADMISSIONS_TABLE_SQL)?;
            tx.execute_batch(DISPATCH_ADMISSIONS_INDEX_SQL)?;
        }
        validate_dispatch_admission_schema(&tx)?;
        if marker.is_none() {
            record_projection_state_tx(
                &tx,
                DISPATCH_ADMISSION_MIGRATION_NAME,
                DISPATCH_ADMISSION_MIGRATION_VERSION,
                0,
                None,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn recover_stale_dispatch_admissions(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        let now = now_string();
        conn.execute(
            "UPDATE dispatch_admissions
                SET admission_state = 'ambiguous', updated_at = ?1, settled_at = ?1,
                    result_http_status = 409,
                    result_error_code = 'dispatch_ambiguous',
                    result_error_message = 'provider handoff outcome is unknown after gateway restart'
              WHERE admission_state = 'handoff_started'",
            params![now],
        )
        .map_err(Into::into)
    }

    pub(crate) fn dispatch_admission(
        &self,
        key: &DispatchAdmissionKey,
    ) -> GaryxDbResult<Option<DispatchAdmissionRecord>> {
        let conn = self.read_conn()?;
        let row = conn
            .query_row(
                DISPATCH_ADMISSION_SELECT,
                params![
                    key.scope_identity,
                    key.scope_epoch,
                    key.thread_id,
                    key.kind.as_str(),
                    key.client_intent_id,
                ],
                read_dispatch_admission_row,
            )
            .optional()?;
        row.map(decode_dispatch_admission).transpose()
    }

    #[cfg(test)]
    pub(crate) fn insert_dispatch_admission(
        &self,
        input: NewDispatchAdmission<'_>,
    ) -> GaryxDbResult<DispatchAdmissionRecord> {
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        insert_dispatch_admission_tx(&tx, &input, &now)?;
        tx.commit()?;
        drop(conn);
        self.dispatch_admission(input.key)?.ok_or_else(|| {
            GaryxDbError::Configuration("dispatch admission insert was not readable".to_owned())
        })
    }

    /// Admit provider handoff for an existing canonical thread in the same
    /// writer transaction that rechecks the truth record and lifecycle
    /// tombstone. The caller already owns the coordinator request lease; this
    /// transaction is its durable linearization point.
    pub(crate) fn insert_dispatch_admission_for_existing_thread(
        &self,
        input: NewDispatchAdmission<'_>,
        attachment_claims: &[PromptAttachmentClaim],
    ) -> GaryxDbResult<DispatchAdmissionRecord> {
        self.insert_dispatch_admission_with_records_for_existing_thread(
            input,
            Vec::new(),
            attachment_claims,
        )
    }

    /// Atomically publish prepared existing-thread record/projection changes,
    /// the durable provider plan, and every managed attachment claim. A
    /// duplicate admission is returned without replaying the record changes.
    pub(crate) fn insert_dispatch_admission_with_records_for_existing_thread(
        &self,
        input: NewDispatchAdmission<'_>,
        records: Vec<ThreadRecordWrite>,
        attachment_claims: &[PromptAttachmentClaim],
    ) -> GaryxDbResult<DispatchAdmissionRecord> {
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing = tx
            .query_row(
                DISPATCH_ADMISSION_SELECT,
                params![
                    input.key.scope_identity,
                    input.key.scope_epoch,
                    input.key.thread_id,
                    input.key.kind.as_str(),
                    input.key.client_intent_id,
                ],
                read_dispatch_admission_row,
            )
            .optional()?
            .map(decode_dispatch_admission)
            .transpose()?;
        if let Some(existing) = existing {
            tx.commit()?;
            return Ok(existing);
        }
        let archived = tx
            .query_row(
                "SELECT 1 FROM archived_threads WHERE thread_id = ?1",
                params![input.key.thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if archived {
            return Err(GaryxDbError::ThreadArchived(input.key.thread_id.clone()));
        }
        let exists = tx
            .query_row(
                "SELECT 1 FROM thread_records WHERE key = ?1",
                params![input.key.thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(GaryxDbError::BadRequest(format!(
                "thread not found: {}",
                input.key.thread_id
            )));
        }
        for record in records {
            write_thread_record_with_projections_tx(
                &tx,
                &record.key,
                &record.body,
                record.updated_at.as_deref(),
                record.projections,
                &now,
            )?;
        }
        insert_dispatch_admission_tx(&tx, &input, &now)?;
        if !attachment_claims.is_empty() {
            let effective_run_id = input.effective_run_id.ok_or_else(|| {
                GaryxDbError::BadRequest(
                    "managed attachment admission requires an effective run id".to_owned(),
                )
            })?;
            let lease_expires_at = (Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
            claim_prompt_attachments_tx(
                &tx,
                attachment_claims,
                &PromptAttachmentOwner {
                    scope_identity: &input.key.scope_identity,
                    scope_epoch: input.key.scope_epoch,
                    thread_id: &input.key.thread_id,
                    kind: input.key.kind,
                    client_intent_id: Some(&input.key.client_intent_id),
                    requested_run_id: input.requested_run_id,
                    effective_run_id,
                    lease_expires_at: &lease_expires_at,
                },
                &now,
            )?;
        }
        tx.commit()?;
        drop(conn);
        self.dispatch_admission(input.key)?.ok_or_else(|| {
            GaryxDbError::Configuration("dispatch admission insert was not readable".to_owned())
        })
    }

    pub(crate) fn insert_no_active_dispatch_admission(
        &self,
        key: &DispatchAdmissionKey,
        request_fingerprint: &str,
    ) -> GaryxDbResult<DispatchAdmissionRecord> {
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO dispatch_admissions (
                scope_identity, scope_epoch, thread_id, kind, client_intent_id,
                fingerprint_version, request_fingerprint, admission_state,
                handoff_attempt, outcome, result_http_status, created_at, updated_at, settled_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 'not_dispatched', 0,
                       'no_active_session', 200, ?7, ?7, ?7)
             ON CONFLICT(scope_identity, scope_epoch, thread_id, kind, client_intent_id)
             DO NOTHING",
            params![
                key.scope_identity,
                key.scope_epoch,
                key.thread_id,
                key.kind.as_str(),
                key.client_intent_id,
                request_fingerprint,
                now,
            ],
        )?;
        drop(conn);
        self.dispatch_admission(key)?.ok_or_else(|| {
            GaryxDbError::Configuration("dispatch admission insert was not readable".to_owned())
        })
    }

    pub(crate) fn start_dispatch_handoff(
        &self,
        key: &DispatchAdmissionKey,
    ) -> GaryxDbResult<Option<DispatchAdmissionRecord>> {
        let now = now_string();
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE dispatch_admissions
                SET admission_state = 'handoff_started',
                    handoff_attempt = handoff_attempt + 1,
                    handoff_started_at = ?1, updated_at = ?1
              WHERE scope_identity = ?2 AND scope_epoch = ?3 AND thread_id = ?4
                AND kind = ?5 AND client_intent_id = ?6
                AND admission_state = 'admitted'",
            params![
                now,
                key.scope_identity,
                key.scope_epoch,
                key.thread_id,
                key.kind.as_str(),
                key.client_intent_id,
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        drop(conn);
        self.dispatch_admission(key)
    }

    pub(crate) fn settle_dispatch_admission(
        &self,
        key: &DispatchAdmissionKey,
        state: DispatchAdmissionState,
        outcome: Option<DispatchOutcome>,
        effective_run_id: Option<&str>,
        pending_input_id: Option<&str>,
        http_status: i64,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> GaryxDbResult<DispatchAdmissionRecord> {
        if !matches!(
            state,
            DispatchAdmissionState::Accepted
                | DispatchAdmissionState::Rejected
                | DispatchAdmissionState::Ambiguous
        ) {
            return Err(GaryxDbError::BadRequest(format!(
                "invalid terminal dispatch state '{}'",
                state.as_str()
            )));
        }
        let bounded_error = error_message.map(|value| value.chars().take(2048).collect::<String>());
        let now = now_string();
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE dispatch_admissions
                SET admission_state = ?1,
                    outcome = COALESCE(?2, outcome),
                    effective_run_id = COALESCE(?3, effective_run_id),
                    pending_input_id = COALESCE(?4, pending_input_id),
                    result_http_status = ?5,
                    result_error_code = ?6,
                    result_error_message = ?7,
                    updated_at = ?8, settled_at = ?8
              WHERE scope_identity = ?9 AND scope_epoch = ?10 AND thread_id = ?11
                AND kind = ?12 AND client_intent_id = ?13
                AND admission_state = 'handoff_started'",
            params![
                state.as_str(),
                outcome.map(DispatchOutcome::as_str),
                effective_run_id,
                pending_input_id,
                http_status,
                error_code,
                bounded_error,
                now,
                key.scope_identity,
                key.scope_epoch,
                key.thread_id,
                key.kind.as_str(),
                key.client_intent_id,
            ],
        )?;
        if changed != 1 {
            return Err(GaryxDbError::Configuration(format!(
                "dispatch admission {} could not settle from handoff_started",
                key.client_intent_id
            )));
        }
        drop(conn);
        self.dispatch_admission(key)?.ok_or_else(|| {
            GaryxDbError::Configuration("settled dispatch admission was not readable".to_owned())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kind: DispatchAdmissionKind, intent: &str) -> DispatchAdmissionKey {
        DispatchAdmissionKey {
            scope_identity: "test-client".to_owned(),
            scope_epoch: 1,
            thread_id: "thread::dispatch-admission-test".to_owned(),
            kind,
            client_intent_id: intent.to_owned(),
        }
    }

    #[test]
    fn migration_marker_is_versioned_and_schema_drift_fails_closed() {
        let db = GaryxDbService::memory().expect("memory db");
        db.migrate_dispatch_admission_ledger_v1()
            .expect("first migration");
        db.migrate_dispatch_admission_ledger_v1()
            .expect("idempotent migration");
        let marker: (i64, i64) = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT projection_version, source_row_count
                   FROM projection_states WHERE projection_name = ?1",
                params![DISPATCH_ADMISSION_MIGRATION_NAME],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(marker, (1, 0));

        db.conn()
            .unwrap()
            .execute_batch(
                "DROP INDEX idx_dispatch_admissions_thread_state;
                 CREATE INDEX idx_dispatch_admissions_thread_state
                     ON dispatch_admissions(admission_state, thread_id);",
            )
            .unwrap();
        let error = db
            .migrate_dispatch_admission_ledger_v1()
            .expect_err("committed marker must validate its physical schema");
        assert!(matches!(error, GaryxDbError::Configuration(_)));
    }

    #[test]
    fn committed_marker_does_not_recreate_a_missing_ledger() {
        let db = GaryxDbService::memory().expect("memory db");
        db.migrate_dispatch_admission_ledger_v1()
            .expect("first migration");
        db.conn()
            .unwrap()
            .execute_batch("DROP TABLE dispatch_admissions")
            .unwrap();

        let error = db
            .migrate_dispatch_admission_ledger_v1()
            .expect_err("a committed marker must not repair a missing ledger");
        assert!(matches!(error, GaryxDbError::Configuration(_)));
        let table_count: i64 = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'table' AND name = 'dispatch_admissions'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0, "marked schema drift must remain unrepaired");
    }

    #[test]
    fn admission_gate_replays_ids_and_restart_makes_inflight_ambiguous() {
        let db = GaryxDbService::memory().expect("memory db");
        db.migrate_dispatch_admission_ledger_v1().unwrap();

        let accepted_key = key(DispatchAdmissionKind::ChatStart, "intent-accepted");
        let inserted = db
            .insert_dispatch_admission(NewDispatchAdmission {
                key: &accepted_key,
                request_fingerprint: "fingerprint-a",
                requested_run_id: Some("run-requested"),
                effective_run_id: Some("run-requested"),
                pending_input_id: None,
                outcome: Some(DispatchOutcome::Started),
            })
            .unwrap();
        assert_eq!(inserted.state, DispatchAdmissionState::Admitted);
        assert_eq!(inserted.handoff_attempt, 0);
        assert!(db.start_dispatch_handoff(&accepted_key).unwrap().is_some());
        assert!(db.start_dispatch_handoff(&accepted_key).unwrap().is_none());
        let settled = db
            .settle_dispatch_admission(
                &accepted_key,
                DispatchAdmissionState::Accepted,
                Some(DispatchOutcome::Started),
                Some("run-requested"),
                None,
                200,
                None,
                None,
            )
            .unwrap();
        assert_eq!(settled.state, DispatchAdmissionState::Accepted);
        assert_eq!(settled.requested_run_id.as_deref(), Some("run-requested"));
        assert_eq!(settled.effective_run_id.as_deref(), Some("run-requested"));

        let duplicate = db
            .insert_dispatch_admission(NewDispatchAdmission {
                key: &accepted_key,
                request_fingerprint: "different-fingerprint",
                requested_run_id: Some("different-run"),
                effective_run_id: Some("different-run"),
                pending_input_id: None,
                outcome: Some(DispatchOutcome::Started),
            })
            .unwrap();
        assert_eq!(duplicate, settled, "primary key preserves the first result");

        let inflight_key = key(DispatchAdmissionKind::ChatStart, "intent-inflight");
        db.insert_dispatch_admission(NewDispatchAdmission {
            key: &inflight_key,
            request_fingerprint: "fingerprint-b",
            requested_run_id: Some("run-new"),
            effective_run_id: Some("run-active"),
            pending_input_id: Some("queued_input:stable"),
            outcome: Some(DispatchOutcome::QueuedToActiveRun),
        })
        .unwrap();
        db.start_dispatch_handoff(&inflight_key).unwrap().unwrap();
        assert_eq!(db.recover_stale_dispatch_admissions().unwrap(), 1);
        let recovered = db.dispatch_admission(&inflight_key).unwrap().unwrap();
        assert_eq!(recovered.state, DispatchAdmissionState::Ambiguous);
        assert_eq!(recovered.effective_run_id.as_deref(), Some("run-active"));
        assert_eq!(
            recovered.pending_input_id.as_deref(),
            Some("queued_input:stable")
        );
        assert_eq!(recovered.result_http_status, Some(409));
    }

    #[test]
    fn no_active_stream_input_is_terminal_without_crossing_the_gate() {
        let db = GaryxDbService::memory().expect("memory db");
        db.migrate_dispatch_admission_ledger_v1().unwrap();
        let stream_key = key(DispatchAdmissionKind::StreamInput, "intent-no-active");
        let record = db
            .insert_no_active_dispatch_admission(&stream_key, "fingerprint-stream")
            .unwrap();
        assert_eq!(record.state, DispatchAdmissionState::NotDispatched);
        assert_eq!(record.outcome, Some(DispatchOutcome::NoActiveSession));
        assert_eq!(record.handoff_attempt, 0);
        assert!(db.start_dispatch_handoff(&stream_key).unwrap().is_none());
    }

    #[test]
    fn existing_record_patch_rolls_back_when_attachment_claim_fails() {
        let db = GaryxDbService::memory().expect("memory db");
        db.run_thread_data_startup_migrations().unwrap();
        let admission_key = key(DispatchAdmissionKind::ChatStart, "intent-atomic-rollback");
        db.write_thread_record_with_projections(
            &admission_key.thread_id,
            r#"{"thread_id":"thread::dispatch-admission-test","label":"before"}"#,
            None,
            None,
        )
        .unwrap();

        let error = db
            .insert_dispatch_admission_with_records_for_existing_thread(
                NewDispatchAdmission {
                    key: &admission_key,
                    request_fingerprint: "fingerprint-atomic",
                    requested_run_id: Some("run-atomic"),
                    effective_run_id: Some("run-atomic"),
                    pending_input_id: None,
                    outcome: Some(DispatchOutcome::Started),
                },
                vec![ThreadRecordWrite {
                    key: admission_key.thread_id.clone(),
                    body: r#"{"thread_id":"thread::dispatch-admission-test","label":"after"}"#
                        .to_owned(),
                    updated_at: None,
                    projections: None,
                }],
                &[PromptAttachmentClaim {
                    attachment_id: "attachment:missing".to_owned(),
                    expected_relative_path: "attachment-missing/payload".to_owned(),
                    expected_kind: "file".to_owned(),
                    expected_sha256: "missing".to_owned(),
                }],
            )
            .unwrap_err();
        assert!(error.to_string().contains("attachment"));
        assert!(db.dispatch_admission(&admission_key).unwrap().is_none());
        let body = db
            .get_thread_record_body(&admission_key.thread_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["label"],
            "before"
        );
    }
}
