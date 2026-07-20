use super::*;

pub(crate) const PROMPT_ATTACHMENT_MIGRATION_NAME: &str = "prompt_attachment_lifecycle_v1";
pub(crate) const PROMPT_ATTACHMENT_MIGRATION_VERSION: i64 = 1;

const PROMPT_ATTACHMENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS prompt_attachments (
    attachment_id TEXT PRIMARY KEY,
    scope_identity TEXT NOT NULL,
    scope_epoch INTEGER NOT NULL CHECK (scope_epoch >= 0),
    relative_path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('image', 'file')),
    original_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('ready', 'claimed', 'delete_pending')),
    expires_at TEXT NOT NULL,
    lease_expires_at TEXT,
    owner_thread_id TEXT,
    owner_kind TEXT CHECK (owner_kind IS NULL OR owner_kind IN ('chat_start', 'stream_input')),
    owner_client_intent_id TEXT,
    owner_requested_run_id TEXT,
    owner_effective_run_id TEXT,
    delete_attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (delete_attempt_count >= 0),
    next_delete_at TEXT,
    last_delete_error TEXT,
    created_at TEXT NOT NULL,
    claimed_at TEXT,
    delete_pending_at TEXT,
    updated_at TEXT NOT NULL,
    CHECK (
        (scope_epoch = 0 AND scope_identity = '__legacy_api__')
        OR (scope_epoch > 0 AND scope_identity <> '__legacy_api__')
    ),
    CHECK (
        state <> 'ready'
        OR (
            lease_expires_at IS NULL AND owner_thread_id IS NULL AND owner_kind IS NULL
            AND owner_client_intent_id IS NULL AND owner_requested_run_id IS NULL
            AND owner_effective_run_id IS NULL AND claimed_at IS NULL
        )
    ),
    CHECK (
        state <> 'claimed'
        OR (
            lease_expires_at IS NOT NULL AND owner_thread_id IS NOT NULL
            AND owner_kind IS NOT NULL AND owner_effective_run_id IS NOT NULL
            AND claimed_at IS NOT NULL
        )
    )
) STRICT;
"#;

const PROMPT_ATTACHMENTS_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_prompt_attachments_ready_expiry
    ON prompt_attachments(state, expires_at) WHERE state = 'ready';
CREATE INDEX IF NOT EXISTS idx_prompt_attachments_claim_lease
    ON prompt_attachments(state, lease_expires_at) WHERE state = 'claimed';
CREATE INDEX IF NOT EXISTS idx_prompt_attachments_owner_run
    ON prompt_attachments(owner_effective_run_id, state) WHERE state = 'claimed';
CREATE INDEX IF NOT EXISTS idx_prompt_attachments_delete_pending
    ON prompt_attachments(state, next_delete_at) WHERE state = 'delete_pending';
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptAttachmentState {
    Ready,
    Claimed,
    DeletePending,
}

impl PromptAttachmentState {
    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "ready" => Ok(Self::Ready),
            "claimed" => Ok(Self::Claimed),
            "delete_pending" => Ok(Self::DeletePending),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid prompt attachment state '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PromptAttachmentRecord {
    pub attachment_id: String,
    pub scope_identity: String,
    pub scope_epoch: i64,
    pub relative_path: String,
    pub kind: String,
    pub original_name: String,
    pub media_type: String,
    pub byte_size: i64,
    pub sha256: String,
    pub state: PromptAttachmentState,
    pub expires_at: String,
    pub lease_expires_at: Option<String>,
    pub owner_thread_id: Option<String>,
    pub owner_kind: Option<String>,
    pub owner_client_intent_id: Option<String>,
    pub owner_requested_run_id: Option<String>,
    pub owner_effective_run_id: Option<String>,
    pub delete_attempt_count: i64,
    pub next_delete_at: Option<String>,
    pub last_delete_error: Option<String>,
    pub created_at: String,
    pub claimed_at: Option<String>,
    pub delete_pending_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NewPromptAttachment {
    pub attachment_id: String,
    pub scope_identity: String,
    pub scope_epoch: i64,
    pub relative_path: String,
    pub kind: String,
    pub original_name: String,
    pub media_type: String,
    pub byte_size: i64,
    pub sha256: String,
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptAttachmentClaim {
    pub attachment_id: String,
    pub expected_relative_path: String,
    pub expected_kind: String,
    pub expected_sha256: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptAttachmentOwner<'a> {
    pub scope_identity: &'a str,
    pub scope_epoch: i64,
    pub thread_id: &'a str,
    pub kind: DispatchAdmissionKind,
    pub client_intent_id: Option<&'a str>,
    pub requested_run_id: Option<&'a str>,
    pub effective_run_id: &'a str,
    pub lease_expires_at: &'a str,
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

fn validate_prompt_attachment_schema(tx: &Transaction<'_>) -> GaryxDbResult<()> {
    let table = tx
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'prompt_attachments'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            GaryxDbError::Configuration("prompt attachment table is missing".to_owned())
        })?;
    if canonical_schema_sql(&table) != canonical_schema_sql(PROMPT_ATTACHMENTS_TABLE_SQL) {
        return Err(GaryxDbError::Configuration(
            "prompt attachment table does not match the committed v1 schema".to_owned(),
        ));
    }
    for (name, fragment) in [
        (
            "idx_prompt_attachments_ready_expiry",
            "CREATE INDEX IF NOT EXISTS idx_prompt_attachments_ready_expiry ON prompt_attachments(state, expires_at) WHERE state = 'ready';",
        ),
        (
            "idx_prompt_attachments_claim_lease",
            "CREATE INDEX IF NOT EXISTS idx_prompt_attachments_claim_lease ON prompt_attachments(state, lease_expires_at) WHERE state = 'claimed';",
        ),
        (
            "idx_prompt_attachments_owner_run",
            "CREATE INDEX IF NOT EXISTS idx_prompt_attachments_owner_run ON prompt_attachments(owner_effective_run_id, state) WHERE state = 'claimed';",
        ),
        (
            "idx_prompt_attachments_delete_pending",
            "CREATE INDEX IF NOT EXISTS idx_prompt_attachments_delete_pending ON prompt_attachments(state, next_delete_at) WHERE state = 'delete_pending';",
        ),
    ] {
        let actual = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![name],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                GaryxDbError::Configuration(format!(
                    "prompt attachment schema is missing index '{name}'"
                ))
            })?;
        if canonical_schema_sql(&actual) != canonical_schema_sql(fragment) {
            return Err(GaryxDbError::Configuration(format!(
                "prompt attachment index '{name}' does not match the committed v1 schema"
            )));
        }
    }
    Ok(())
}

fn parse_prompt_attachment_timestamp(field: &str, value: &str) -> GaryxDbResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            GaryxDbError::Configuration(format!(
                "invalid prompt attachment {field} timestamp '{value}': {error}"
            ))
        })
}

fn decode_prompt_attachment(row: &rusqlite::Row<'_>) -> rusqlite::Result<PromptAttachmentRecord> {
    let state = row.get::<_, String>(9)?;
    Ok(PromptAttachmentRecord {
        attachment_id: row.get(0)?,
        scope_identity: row.get(1)?,
        scope_epoch: row.get(2)?,
        relative_path: row.get(3)?,
        kind: row.get(4)?,
        original_name: row.get(5)?,
        media_type: row.get(6)?,
        byte_size: row.get(7)?,
        sha256: row.get(8)?,
        state: PromptAttachmentState::parse(&state).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        expires_at: row.get(10)?,
        lease_expires_at: row.get(11)?,
        owner_thread_id: row.get(12)?,
        owner_kind: row.get(13)?,
        owner_client_intent_id: row.get(14)?,
        owner_requested_run_id: row.get(15)?,
        owner_effective_run_id: row.get(16)?,
        delete_attempt_count: row.get(17)?,
        next_delete_at: row.get(18)?,
        last_delete_error: row.get(19)?,
        created_at: row.get(20)?,
        claimed_at: row.get(21)?,
        delete_pending_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

const PROMPT_ATTACHMENT_SELECT_COLUMNS: &str = "
    attachment_id, scope_identity, scope_epoch, relative_path, kind,
    original_name, media_type, byte_size, sha256, state, expires_at,
    lease_expires_at, owner_thread_id, owner_kind, owner_client_intent_id,
    owner_requested_run_id, owner_effective_run_id, delete_attempt_count,
    next_delete_at, last_delete_error, created_at, claimed_at,
    delete_pending_at, updated_at";

pub(super) fn claim_prompt_attachments_tx(
    tx: &Transaction<'_>,
    claims: &[PromptAttachmentClaim],
    owner: &PromptAttachmentOwner<'_>,
    now: &str,
) -> GaryxDbResult<()> {
    let claim_time = parse_prompt_attachment_timestamp("claim time", now)?;
    for claim in claims {
        let record = tx
            .query_row(
                &format!(
                    "SELECT {PROMPT_ATTACHMENT_SELECT_COLUMNS} FROM prompt_attachments WHERE attachment_id = ?1"
                ),
                params![claim.attachment_id],
                decode_prompt_attachment,
            )
            .optional()?
            .ok_or_else(|| {
                GaryxDbError::BadRequest(format!(
                    "managed prompt attachment not found: {}",
                    claim.attachment_id
                ))
            })?;
        if record.relative_path != claim.expected_relative_path
            || record.kind != claim.expected_kind
            || record.sha256 != claim.expected_sha256
        {
            return Err(GaryxDbError::BadRequest(format!(
                "managed prompt attachment metadata mismatch: {}",
                claim.attachment_id
            )));
        }
        if record.state == PromptAttachmentState::Claimed
            && record.scope_identity == owner.scope_identity
            && record.scope_epoch == owner.scope_epoch
            && record.owner_thread_id.as_deref() == Some(owner.thread_id)
            && record.owner_kind.as_deref() == Some(owner.kind.as_str())
            && record.owner_client_intent_id.as_deref() == owner.client_intent_id
            && record.owner_requested_run_id.as_deref() == owner.requested_run_id
            && record.owner_effective_run_id.as_deref() == Some(owner.effective_run_id)
        {
            continue;
        }
        if record.state != PromptAttachmentState::Ready {
            return Err(GaryxDbError::BadRequest(format!(
                "prompt attachment already claimed: {}",
                claim.attachment_id
            )));
        }
        if parse_prompt_attachment_timestamp("expiry", &record.expires_at)? <= claim_time {
            return Err(GaryxDbError::BadRequest(format!(
                "prompt attachment expired: {}",
                claim.attachment_id
            )));
        }
        let same_scope = record.scope_identity == owner.scope_identity
            && record.scope_epoch == owner.scope_epoch;
        let legacy_upgrade = record.scope_identity == "__legacy_api__"
            && record.scope_epoch == 0
            && owner.scope_epoch > 0
            && owner.scope_identity != "__legacy_api__";
        if !same_scope && !legacy_upgrade {
            return Err(GaryxDbError::BadRequest(format!(
                "prompt attachment scope mismatch: {}",
                claim.attachment_id
            )));
        }
        let updated = tx.execute(
            "UPDATE prompt_attachments
                SET scope_identity = ?2, scope_epoch = ?3, state = 'claimed',
                    lease_expires_at = ?4, owner_thread_id = ?5, owner_kind = ?6,
                    owner_client_intent_id = ?7, owner_requested_run_id = ?8,
                    owner_effective_run_id = ?9, claimed_at = ?10, updated_at = ?10
              WHERE attachment_id = ?1 AND state = 'ready'",
            params![
                claim.attachment_id,
                owner.scope_identity,
                owner.scope_epoch,
                owner.lease_expires_at,
                owner.thread_id,
                owner.kind.as_str(),
                owner.client_intent_id,
                owner.requested_run_id,
                owner.effective_run_id,
                now,
            ],
        )?;
        if updated != 1 {
            return Err(GaryxDbError::BadRequest(format!(
                "prompt attachment claim raced: {}",
                claim.attachment_id
            )));
        }
    }
    Ok(())
}

impl GaryxDbService {
    pub(crate) fn migrate_prompt_attachment_lifecycle_v1(&self) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let marker = tx
            .query_row(
                "SELECT projection_version FROM projection_states WHERE projection_name = ?1",
                params![PROMPT_ATTACHMENT_MIGRATION_NAME],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if marker.is_some_and(|version| version != PROMPT_ATTACHMENT_MIGRATION_VERSION) {
            return Err(GaryxDbError::Configuration(format!(
                "prompt attachment marker version mismatch: expected {}, found {}",
                PROMPT_ATTACHMENT_MIGRATION_VERSION,
                marker.unwrap_or_default()
            )));
        }
        if marker.is_none() {
            tx.execute_batch(PROMPT_ATTACHMENTS_TABLE_SQL)?;
            tx.execute_batch(PROMPT_ATTACHMENTS_INDEX_SQL)?;
        }
        validate_prompt_attachment_schema(&tx)?;
        if marker.is_none() {
            record_projection_state_tx(
                &tx,
                PROMPT_ATTACHMENT_MIGRATION_NAME,
                PROMPT_ATTACHMENT_MIGRATION_VERSION,
                0,
                None,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn insert_ready_prompt_attachments(
        &self,
        attachments: &[NewPromptAttachment],
    ) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for input in attachments {
            tx.execute(
                "INSERT INTO prompt_attachments (
                    attachment_id, scope_identity, scope_epoch, relative_path, kind,
                    original_name, media_type, byte_size, sha256, state,
                    expires_at, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ready', ?10, ?11, ?11)",
                params![
                    input.attachment_id,
                    input.scope_identity,
                    input.scope_epoch,
                    input.relative_path,
                    input.kind,
                    input.original_name,
                    input.media_type,
                    input.byte_size,
                    input.sha256,
                    input.expires_at,
                    input.created_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn prompt_attachment_by_id(
        &self,
        attachment_id: &str,
    ) -> GaryxDbResult<Option<PromptAttachmentRecord>> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {PROMPT_ATTACHMENT_SELECT_COLUMNS} FROM prompt_attachments WHERE attachment_id = ?1"
                ),
                params![attachment_id],
                decode_prompt_attachment,
            )
            .optional()?)
    }

    pub(crate) fn prompt_attachment_by_relative_path(
        &self,
        relative_path: &str,
    ) -> GaryxDbResult<Option<PromptAttachmentRecord>> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {PROMPT_ATTACHMENT_SELECT_COLUMNS} FROM prompt_attachments WHERE relative_path = ?1"
                ),
                params![relative_path],
                decode_prompt_attachment,
            )
            .optional()?)
    }

    pub(crate) fn claim_prompt_attachments(
        &self,
        claims: &[PromptAttachmentClaim],
        owner: PromptAttachmentOwner<'_>,
        now: &str,
    ) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        claim_prompt_attachments_tx(&tx, claims, &owner, now)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn renew_prompt_attachment_lease(
        &self,
        effective_run_id: &str,
        lease_expires_at: &str,
        now: &str,
    ) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE prompt_attachments SET lease_expires_at = ?2, updated_at = ?3
              WHERE owner_effective_run_id = ?1 AND state = 'claimed'",
            params![effective_run_id, lease_expires_at, now],
        )
        .map_err(Into::into)
    }

    pub(crate) fn mark_prompt_attachments_delete_pending_for_run(
        &self,
        effective_run_id: &str,
        now: &str,
    ) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE prompt_attachments
                SET state = 'delete_pending', delete_pending_at = ?2,
                    next_delete_at = ?2, updated_at = ?2
              WHERE owner_effective_run_id = ?1 AND state = 'claimed'",
            params![effective_run_id, now],
        )
        .map_err(Into::into)
    }

    pub(crate) fn expire_prompt_attachments(&self, now: &str) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE prompt_attachments
                SET state = 'delete_pending', delete_pending_at = ?1,
                    next_delete_at = ?1, updated_at = ?1
              WHERE (state = 'ready' AND expires_at <= ?1)
                 OR (state = 'claimed' AND lease_expires_at <= ?1)",
            params![now],
        )
        .map_err(Into::into)
    }

    pub(crate) fn due_prompt_attachment_deletions(
        &self,
        now: &str,
        limit: usize,
    ) -> GaryxDbResult<Vec<PromptAttachmentRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROMPT_ATTACHMENT_SELECT_COLUMNS}
               FROM prompt_attachments
              WHERE state = 'delete_pending'
                AND (next_delete_at IS NULL OR next_delete_at <= ?1)
              ORDER BY COALESCE(next_delete_at, delete_pending_at), attachment_id
              LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![now, limit as i64], decode_prompt_attachment)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn finish_prompt_attachment_deletion(
        &self,
        attachment_id: &str,
    ) -> GaryxDbResult<bool> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM prompt_attachments WHERE attachment_id = ?1 AND state = 'delete_pending'",
            params![attachment_id],
        )? == 1)
    }

    pub(crate) fn fail_prompt_attachment_deletion(
        &self,
        attachment_id: &str,
        next_delete_at: &str,
        error: &str,
        now: &str,
    ) -> GaryxDbResult<()> {
        let bounded_error = error.chars().take(2048).collect::<String>();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE prompt_attachments
                SET delete_attempt_count = delete_attempt_count + 1,
                    next_delete_at = ?2, last_delete_error = ?3, updated_at = ?4
              WHERE attachment_id = ?1 AND state = 'delete_pending'",
            params![attachment_id, next_delete_at, bounded_error, now],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(id: &str, expires_at: &str) -> NewPromptAttachment {
        NewPromptAttachment {
            attachment_id: id.to_owned(),
            scope_identity: "attachment-test".to_owned(),
            scope_epoch: 1,
            relative_path: format!("{id}/payload"),
            kind: "file".to_owned(),
            original_name: "note.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            byte_size: 4,
            sha256: "abcd".to_owned(),
            expires_at: expires_at.to_owned(),
            created_at: "2026-07-20T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn marker_is_strict_and_claim_is_single_owner() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_lifecycle_v1().unwrap();
        db.insert_ready_prompt_attachments(&[ready("attachment:one", "2026-07-21T00:00:00Z")])
            .unwrap();
        let claim = PromptAttachmentClaim {
            attachment_id: "attachment:one".to_owned(),
            expected_relative_path: "attachment:one/payload".to_owned(),
            expected_kind: "file".to_owned(),
            expected_sha256: "abcd".to_owned(),
        };
        db.claim_prompt_attachments(
            std::slice::from_ref(&claim),
            PromptAttachmentOwner {
                scope_identity: "attachment-test",
                scope_epoch: 1,
                thread_id: "thread::one",
                kind: DispatchAdmissionKind::ChatStart,
                client_intent_id: Some("intent-one"),
                requested_run_id: Some("run-one"),
                effective_run_id: "run-one",
                lease_expires_at: "2026-07-20T02:00:00Z",
            },
            "2026-07-20T00:00:01Z",
        )
        .unwrap();
        let error = db
            .claim_prompt_attachments(
                &[claim],
                PromptAttachmentOwner {
                    scope_identity: "attachment-test",
                    scope_epoch: 1,
                    thread_id: "thread::two",
                    kind: DispatchAdmissionKind::ChatStart,
                    client_intent_id: Some("intent-two"),
                    requested_run_id: Some("run-two"),
                    effective_run_id: "run-two",
                    lease_expires_at: "2026-07-20T02:00:00Z",
                },
                "2026-07-20T00:00:02Z",
            )
            .unwrap_err();
        assert!(error.to_string().contains("already claimed"));
    }

    #[test]
    fn committed_marker_does_not_recreate_a_missing_attachment_index() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_lifecycle_v1().unwrap();
        db.conn()
            .unwrap()
            .execute_batch("DROP INDEX idx_prompt_attachments_owner_run")
            .unwrap();

        let error = db
            .migrate_prompt_attachment_lifecycle_v1()
            .expect_err("a committed marker must not repair a missing attachment index");
        assert!(matches!(error, GaryxDbError::Configuration(_)));
        let index_count: i64 = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'index' AND name = 'idx_prompt_attachments_owner_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 0, "marked schema drift must remain unrepaired");
    }

    #[test]
    fn claim_expiry_compares_rfc3339_instants_at_submillisecond_precision() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_lifecycle_v1().unwrap();
        db.insert_ready_prompt_attachments(&[
            ready(
                "attachment:precision-valid",
                "2026-07-21T00:00:00.123456+00:00",
            ),
            ready(
                "attachment:precision-expired",
                "2026-07-21T00:00:00.123456+00:00",
            ),
        ])
        .unwrap();
        let claim = |attachment_id: &str| PromptAttachmentClaim {
            attachment_id: attachment_id.to_owned(),
            expected_relative_path: format!("{attachment_id}/payload"),
            expected_kind: "file".to_owned(),
            expected_sha256: "abcd".to_owned(),
        };
        fn owner<'a>(intent: &'a str, run: &'a str) -> PromptAttachmentOwner<'a> {
            PromptAttachmentOwner {
                scope_identity: "attachment-test",
                scope_epoch: 1,
                thread_id: "thread::precision",
                kind: DispatchAdmissionKind::ChatStart,
                client_intent_id: Some(intent),
                requested_run_id: Some(run),
                effective_run_id: run,
                lease_expires_at: "2026-07-21T02:00:00Z",
            }
        }

        db.claim_prompt_attachments(
            &[claim("attachment:precision-valid")],
            owner("intent-valid", "run-valid"),
            "2026-07-21T00:00:00.123Z",
        )
        .expect("123.456ms expiry remains valid at 123ms");
        let error = db
            .claim_prompt_attachments(
                &[claim("attachment:precision-expired")],
                owner("intent-expired", "run-expired"),
                "2026-07-21T00:00:00.124Z",
            )
            .expect_err("123.456ms expiry is expired at 124ms");
        assert!(error.to_string().contains("expired"));
    }

    #[test]
    fn ready_ttl_and_claim_lease_converge_to_delete_pending() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_lifecycle_v1().unwrap();
        db.insert_ready_prompt_attachments(&[
            ready("attachment:ready", "2026-07-20T00:00:10Z"),
            ready("attachment:claimed", "2026-07-21T00:00:00Z"),
        ])
        .unwrap();
        db.claim_prompt_attachments(
            &[PromptAttachmentClaim {
                attachment_id: "attachment:claimed".to_owned(),
                expected_relative_path: "attachment:claimed/payload".to_owned(),
                expected_kind: "file".to_owned(),
                expected_sha256: "abcd".to_owned(),
            }],
            PromptAttachmentOwner {
                scope_identity: "attachment-test",
                scope_epoch: 1,
                thread_id: "thread::one",
                kind: DispatchAdmissionKind::ChatStart,
                client_intent_id: Some("intent"),
                requested_run_id: Some("run"),
                effective_run_id: "run",
                lease_expires_at: "2026-07-20T00:00:10Z",
            },
            "2026-07-20T00:00:01Z",
        )
        .unwrap();
        assert_eq!(
            db.expire_prompt_attachments("2026-07-20T00:00:11Z")
                .unwrap(),
            2
        );
        assert_eq!(
            db.due_prompt_attachment_deletions("2026-07-20T00:00:11Z", 10)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn legacy_chat_starts_with_distinct_requested_runs_cannot_share_one_attachment() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_lifecycle_v1().unwrap();
        db.insert_ready_prompt_attachments(&[ready(
            "attachment:legacy-owner",
            "2026-07-21T00:00:00Z",
        )])
        .unwrap();
        let claim = PromptAttachmentClaim {
            attachment_id: "attachment:legacy-owner".to_owned(),
            expected_relative_path: "attachment:legacy-owner/payload".to_owned(),
            expected_kind: "file".to_owned(),
            expected_sha256: "abcd".to_owned(),
        };
        let owner = |requested_run_id| PromptAttachmentOwner {
            scope_identity: "attachment-test",
            scope_epoch: 1,
            thread_id: "thread::same",
            kind: DispatchAdmissionKind::ChatStart,
            client_intent_id: None,
            requested_run_id: Some(requested_run_id),
            effective_run_id: "run-active",
            lease_expires_at: "2026-07-20T02:00:00Z",
        };
        db.claim_prompt_attachments(
            std::slice::from_ref(&claim),
            owner("run-request-one"),
            "2026-07-20T00:00:01Z",
        )
        .unwrap();

        let error = db
            .claim_prompt_attachments(&[claim], owner("run-request-two"), "2026-07-20T00:00:02Z")
            .unwrap_err();
        assert!(error.to_string().contains("already claimed"));
    }
}
