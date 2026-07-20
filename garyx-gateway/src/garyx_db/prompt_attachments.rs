use super::*;

pub(crate) const PROMPT_ATTACHMENT_MIGRATION_NAME: &str = "prompt_attachment_lifecycle_v1";
pub(crate) const PROMPT_ATTACHMENT_MIGRATION_VERSION: i64 = 1;
pub(crate) const PROMPT_ATTACHMENT_OWNERSHIP_MIGRATION_NAME: &str =
    "prompt_attachment_thread_ownership_v2";
pub(crate) const PROMPT_ATTACHMENT_OWNERSHIP_MIGRATION_VERSION: i64 = 2;

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

const PROMPT_ATTACHMENTS_V2_TABLE_SQL: &str = r#"
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
    state TEXT NOT NULL CHECK (state IN ('staged', 'owned')),
    owner_thread_id TEXT,
    owner_kind TEXT CHECK (owner_kind IS NULL OR owner_kind IN ('chat_start', 'stream_input')),
    owner_client_intent_id TEXT,
    owner_requested_run_id TEXT,
    owner_effective_run_id TEXT,
    created_at TEXT NOT NULL,
    owned_at TEXT,
    updated_at TEXT NOT NULL,
    CHECK (
        (scope_epoch = 0 AND scope_identity = '__legacy_api__')
        OR (scope_epoch > 0 AND scope_identity <> '__legacy_api__')
    ),
    CHECK (
        state <> 'staged'
        OR (
            owner_thread_id IS NULL AND owner_kind IS NULL
            AND owner_client_intent_id IS NULL AND owner_requested_run_id IS NULL
            AND owner_effective_run_id IS NULL AND owned_at IS NULL
        )
    ),
    CHECK (
        state <> 'owned'
        OR (
            owner_thread_id IS NOT NULL AND owner_kind IS NOT NULL
            AND owner_effective_run_id IS NOT NULL AND owned_at IS NOT NULL
        )
    )
) STRICT;
"#;

const PROMPT_ATTACHMENTS_V2_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_prompt_attachments_owner_thread
    ON prompt_attachments(owner_thread_id, attachment_id) WHERE state = 'owned';
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptAttachmentState {
    Staged,
    Owned,
}

impl PromptAttachmentState {
    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "staged" => Ok(Self::Staged),
            "owned" => Ok(Self::Owned),
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
    pub owner_thread_id: Option<String>,
    pub owner_kind: Option<String>,
    pub owner_client_intent_id: Option<String>,
    pub owner_requested_run_id: Option<String>,
    pub owner_effective_run_id: Option<String>,
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

fn validate_prompt_attachment_v1_schema(tx: &Transaction<'_>) -> GaryxDbResult<()> {
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

fn validate_prompt_attachment_v2_schema(tx: &Transaction<'_>) -> GaryxDbResult<()> {
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
    if canonical_schema_sql(&table) != canonical_schema_sql(PROMPT_ATTACHMENTS_V2_TABLE_SQL) {
        return Err(GaryxDbError::Configuration(
            "prompt attachment table does not match the committed v2 schema".to_owned(),
        ));
    }
    let index = tx
        .query_row(
            "SELECT sql FROM sqlite_master
              WHERE type = 'index' AND name = 'idx_prompt_attachments_owner_thread'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            GaryxDbError::Configuration(
                "prompt attachment schema is missing owner-thread index".to_owned(),
            )
        })?;
    if canonical_schema_sql(&index)
        != canonical_schema_sql(
            "CREATE INDEX IF NOT EXISTS idx_prompt_attachments_owner_thread ON prompt_attachments(owner_thread_id, attachment_id) WHERE state = 'owned';",
        )
    {
        return Err(GaryxDbError::Configuration(
            "prompt attachment owner-thread index does not match the committed v2 schema"
                .to_owned(),
        ));
    }
    Ok(())
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
        owner_thread_id: row.get(10)?,
        owner_kind: row.get(11)?,
        owner_client_intent_id: row.get(12)?,
        owner_requested_run_id: row.get(13)?,
        owner_effective_run_id: row.get(14)?,
    })
}

const PROMPT_ATTACHMENT_SELECT_COLUMNS: &str = "
    attachment_id, scope_identity, scope_epoch, relative_path, kind,
    original_name, media_type, byte_size, sha256, state, owner_thread_id,
    owner_kind, owner_client_intent_id, owner_requested_run_id,
    owner_effective_run_id";

pub(super) fn claim_prompt_attachments_tx(
    tx: &Transaction<'_>,
    claims: &[PromptAttachmentClaim],
    owner: &PromptAttachmentOwner<'_>,
    now: &str,
) -> GaryxDbResult<()> {
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
        if record.state == PromptAttachmentState::Owned
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
        if record.state != PromptAttachmentState::Staged {
            return Err(GaryxDbError::BadRequest(format!(
                "prompt attachment already claimed: {}",
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
                SET scope_identity = ?2, scope_epoch = ?3, state = 'owned',
                    owner_thread_id = ?4, owner_kind = ?5,
                    owner_client_intent_id = ?6, owner_requested_run_id = ?7,
                    owner_effective_run_id = ?8, owned_at = ?9, updated_at = ?9
              WHERE attachment_id = ?1 AND state = 'staged'",
            params![
                claim.attachment_id,
                owner.scope_identity,
                owner.scope_epoch,
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
        validate_prompt_attachment_v1_schema(&tx)?;
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

    pub(crate) fn migrate_prompt_attachment_thread_ownership_v2(&self) -> GaryxDbResult<()> {
        self.migrate_cleanup_outbox_prompt_attachments_v1()?;
        let completed = {
            let conn = self.conn()?;
            conn.query_row(
                "SELECT projection_version FROM projection_states WHERE projection_name = ?1",
                params![PROMPT_ATTACHMENT_OWNERSHIP_MIGRATION_NAME],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        };
        if let Some(version) = completed {
            if version != PROMPT_ATTACHMENT_OWNERSHIP_MIGRATION_VERSION {
                return Err(GaryxDbError::Configuration(format!(
                    "prompt attachment ownership marker version mismatch: expected {}, found {version}",
                    PROMPT_ATTACHMENT_OWNERSHIP_MIGRATION_VERSION
                )));
            }
            let mut conn = self.conn()?;
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let v1_version = tx
                .query_row(
                    "SELECT projection_version FROM projection_states WHERE projection_name = ?1",
                    params![PROMPT_ATTACHMENT_MIGRATION_NAME],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if v1_version != Some(PROMPT_ATTACHMENT_MIGRATION_VERSION) {
                return Err(GaryxDbError::Configuration(
                    "prompt attachment v2 marker requires the committed v1 marker".to_owned(),
                ));
            }
            validate_prompt_attachment_v2_schema(&tx)?;
            tx.commit()?;
            return Ok(());
        }

        // The v1 marker and its physical schema remain a strict prerequisite.
        // A separate v2 cutover owns the intentional retirement of TTL/lease
        // fields instead of weakening the already-committed v1 contract.
        self.migrate_prompt_attachment_lifecycle_v1()?;

        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        validate_prompt_attachment_v1_schema(&tx)?;
        let invalid_delete_pending: i64 = tx.query_row(
            "SELECT COUNT(*) FROM prompt_attachments
              WHERE state = 'delete_pending'
                AND NOT (
                    (owner_thread_id IS NULL AND owner_kind IS NULL
                     AND owner_client_intent_id IS NULL
                     AND owner_requested_run_id IS NULL
                     AND owner_effective_run_id IS NULL AND claimed_at IS NULL)
                    OR
                    (owner_thread_id IS NOT NULL AND owner_kind IS NOT NULL
                     AND owner_effective_run_id IS NOT NULL AND claimed_at IS NOT NULL)
                )",
            [],
            |row| row.get(0),
        )?;
        if invalid_delete_pending != 0 {
            return Err(GaryxDbError::Configuration(format!(
                "prompt attachment v1 contains {invalid_delete_pending} ambiguous delete-pending ownership rows"
            )));
        }
        let source_row_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM prompt_attachments", [], |row| {
                row.get(0)
            })?;
        tx.execute_batch("ALTER TABLE prompt_attachments RENAME TO prompt_attachments_v1")?;
        tx.execute_batch(PROMPT_ATTACHMENTS_V2_TABLE_SQL)?;
        tx.execute(
            "INSERT INTO prompt_attachments (
                attachment_id, scope_identity, scope_epoch, relative_path, kind,
                original_name, media_type, byte_size, sha256, state,
                owner_thread_id, owner_kind, owner_client_intent_id,
                owner_requested_run_id, owner_effective_run_id, created_at,
                owned_at, updated_at
             )
             SELECT attachment_id, scope_identity, scope_epoch, relative_path, kind,
                    original_name, media_type, byte_size, sha256,
                    CASE WHEN owner_thread_id IS NULL THEN 'staged' ELSE 'owned' END,
                    owner_thread_id, owner_kind, owner_client_intent_id,
                    owner_requested_run_id, owner_effective_run_id, created_at,
                    claimed_at, updated_at
               FROM prompt_attachments_v1",
            [],
        )?;
        tx.execute_batch("DROP TABLE prompt_attachments_v1")?;
        tx.execute_batch(PROMPT_ATTACHMENTS_V2_INDEX_SQL)?;
        tx.execute(
            "INSERT INTO cleanup_outbox (
                thread_id, step, payload, status, attempt_count,
                next_attempt_at, created_at, settled_at
             )
             SELECT DISTINCT attachment.owner_thread_id,
                    'prompt_attachments_remove', NULL, 'pending', 0,
                    NULL, ?1, NULL
               FROM prompt_attachments AS attachment
               JOIN archived_threads AS terminal
                 ON terminal.thread_id = attachment.owner_thread_id
              WHERE attachment.state = 'owned'
                AND terminal.kind = 'deleted'
                AND NOT EXISTS (
                    SELECT 1 FROM cleanup_outbox AS existing
                     WHERE existing.thread_id = attachment.owner_thread_id
                       AND existing.step = 'prompt_attachments_remove'
                )",
            params![now_string()],
        )?;
        validate_prompt_attachment_v2_schema(&tx)?;
        record_projection_state_tx(
            &tx,
            PROMPT_ATTACHMENT_OWNERSHIP_MIGRATION_NAME,
            PROMPT_ATTACHMENT_OWNERSHIP_MIGRATION_VERSION,
            source_row_count,
            None,
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn insert_staged_prompt_attachments(
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
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'staged', ?10, ?10)",
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

    pub(crate) fn owned_prompt_attachments_for_thread(
        &self,
        thread_id: &str,
    ) -> GaryxDbResult<Vec<PromptAttachmentRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROMPT_ATTACHMENT_SELECT_COLUMNS}
               FROM prompt_attachments
              WHERE state = 'owned' AND owner_thread_id = ?1
              ORDER BY attachment_id"
        ))?;
        let rows = stmt.query_map(params![thread_id], decode_prompt_attachment)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn delete_owned_prompt_attachment(
        &self,
        attachment_id: &str,
        thread_id: &str,
    ) -> GaryxDbResult<bool> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM prompt_attachments
              WHERE attachment_id = ?1 AND state = 'owned' AND owner_thread_id = ?2",
            params![attachment_id, thread_id],
        )? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged(id: &str) -> NewPromptAttachment {
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
            created_at: "2026-07-20T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn ownership_marker_is_strict_and_claim_is_single_owner() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        db.insert_staged_prompt_attachments(&[staged("attachment:one")])
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
    fn committed_v2_marker_does_not_recreate_a_missing_owner_index() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        db.conn()
            .unwrap()
            .execute_batch("DROP INDEX idx_prompt_attachments_owner_thread")
            .unwrap();

        let error = db
            .migrate_prompt_attachment_thread_ownership_v2()
            .expect_err("a committed v2 marker must not repair a missing owner index");
        assert!(matches!(error, GaryxDbError::Configuration(_)));
        let index_count: i64 = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'index' AND name = 'idx_prompt_attachments_owner_thread'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 0, "marked v2 drift must remain unrepaired");
    }

    #[test]
    fn v2_migration_preserves_staging_and_committed_v1_rows_without_timers() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_lifecycle_v1().unwrap();
        db.conn()
            .unwrap()
            .execute_batch(
                "INSERT INTO archived_threads (thread_id, archived_at, kind)
                 VALUES ('thread::migration-owned', '2026-07-01T00:00:00Z', 'deleted');
                 INSERT INTO prompt_attachments (
                    attachment_id, scope_identity, scope_epoch, relative_path, kind,
                    original_name, media_type, byte_size, sha256, state, expires_at,
                    created_at, updated_at
                 ) VALUES (
                    'attachment:staged', 'attachment-test', 1,
                    'attachment:staged/payload', 'image', 'staged.jpg', 'image/jpeg',
                    4, 'staged-sha', 'ready', '2026-07-01T00:00:00Z',
                    '2026-06-30T00:00:00Z', '2026-06-30T00:00:00Z'
                 );
                 INSERT INTO prompt_attachments (
                    attachment_id, scope_identity, scope_epoch, relative_path, kind,
                    original_name, media_type, byte_size, sha256, state, expires_at,
                    lease_expires_at, owner_thread_id, owner_kind,
                    owner_client_intent_id, owner_requested_run_id,
                    owner_effective_run_id, created_at, claimed_at, updated_at
                 ) VALUES (
                    'attachment:owned', 'attachment-test', 1,
                    'attachment:owned/payload', 'image', 'owned.jpg', 'image/jpeg',
                    4, 'owned-sha', 'claimed', '2026-07-01T00:00:00Z',
                    '2026-07-01T01:00:00Z', 'thread::migration-owned', 'chat_start',
                    'intent-owned', 'run-owned', 'run-owned',
                    '2026-06-30T00:00:00Z', '2026-06-30T00:01:00Z',
                    '2026-06-30T00:01:00Z'
                 );
                 INSERT INTO prompt_attachments (
                    attachment_id, scope_identity, scope_epoch, relative_path, kind,
                    original_name, media_type, byte_size, sha256, state, expires_at,
                    lease_expires_at, owner_thread_id, owner_kind,
                    owner_client_intent_id, owner_requested_run_id,
                    owner_effective_run_id, created_at, claimed_at,
                    delete_pending_at, next_delete_at, updated_at
                 ) VALUES (
                    'attachment:pending-owned', 'attachment-test', 1,
                    'attachment:pending-owned/payload', 'image', 'pending.jpg', 'image/jpeg',
                    4, 'pending-sha', 'delete_pending', '2026-07-01T00:00:00Z',
                    '2026-07-01T01:00:00Z', 'thread::migration-owned', 'chat_start',
                    'intent-pending', 'run-pending', 'run-pending',
                    '2026-06-30T00:00:00Z', '2026-06-30T00:01:00Z',
                    '2026-07-01T01:00:00Z', '2026-07-01T01:00:00Z',
                    '2026-07-01T01:00:00Z'
                 );",
            )
            .unwrap();

        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();

        assert_eq!(
            db.prompt_attachment_by_id("attachment:staged")
                .unwrap()
                .unwrap()
                .state,
            PromptAttachmentState::Staged
        );
        assert_eq!(
            db.prompt_attachment_by_id("attachment:owned")
                .unwrap()
                .unwrap()
                .state,
            PromptAttachmentState::Owned
        );
        let pending = db
            .prompt_attachment_by_id("attachment:pending-owned")
            .unwrap()
            .unwrap();
        assert_eq!(pending.state, PromptAttachmentState::Owned);
        assert_eq!(
            pending.owner_thread_id.as_deref(),
            Some("thread::migration-owned")
        );
        let cleanup = db
            .next_cleanup_outbox_job("2999-01-01T00:00:00Z")
            .unwrap()
            .unwrap();
        assert_eq!(cleanup.thread_id, "thread::migration-owned");
        assert_eq!(cleanup.step, CleanupOutboxStep::PromptAttachmentsRemove);
        let columns = {
            let conn = db.conn().unwrap();
            let mut statement = conn
                .prepare("PRAGMA table_info(prompt_attachments)")
                .unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(!columns.iter().any(|column| {
            matches!(
                column.as_str(),
                "expires_at" | "lease_expires_at" | "delete_pending_at" | "next_delete_at"
            )
        }));
    }

    #[test]
    fn staging_uploads_remain_claimable_after_the_retired_expiry_timestamp() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        db.insert_staged_prompt_attachments(&[staged("attachment:retained-staging")])
            .unwrap();
        let claim = PromptAttachmentClaim {
            attachment_id: "attachment:retained-staging".to_owned(),
            expected_relative_path: "attachment:retained-staging/payload".to_owned(),
            expected_kind: "file".to_owned(),
            expected_sha256: "abcd".to_owned(),
        };
        db.claim_prompt_attachments(
            &[claim],
            PromptAttachmentOwner {
                scope_identity: "attachment-test",
                scope_epoch: 1,
                thread_id: "thread::retained-staging",
                kind: DispatchAdmissionKind::ChatStart,
                client_intent_id: Some("intent-retained-staging"),
                requested_run_id: Some("run-retained-staging"),
                effective_run_id: "run-retained-staging",
            },
            "2999-07-21T00:00:00Z",
        )
        .expect("staging uploads have no time-based expiry");
    }

    #[test]
    fn staging_and_owned_rows_have_distinct_durable_ownership() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        db.insert_staged_prompt_attachments(&[
            staged("attachment:staged"),
            staged("attachment:owned"),
        ])
        .unwrap();
        db.claim_prompt_attachments(
            &[PromptAttachmentClaim {
                attachment_id: "attachment:owned".to_owned(),
                expected_relative_path: "attachment:owned/payload".to_owned(),
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
            },
            "2026-07-20T00:00:01Z",
        )
        .unwrap();
        assert_eq!(
            db.prompt_attachment_by_id("attachment:staged")
                .unwrap()
                .unwrap()
                .state,
            PromptAttachmentState::Staged
        );
        let owned = db
            .owned_prompt_attachments_for_thread("thread::one")
            .unwrap();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].attachment_id, "attachment:owned");
    }

    #[test]
    fn legacy_chat_starts_with_distinct_requested_runs_cannot_share_one_attachment() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_prompt_attachment_thread_ownership_v2().unwrap();
        db.insert_staged_prompt_attachments(&[staged("attachment:legacy-owner")])
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
