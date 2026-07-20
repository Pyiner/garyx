use super::*;

pub(crate) const CREATE_INTENT_MIGRATION_NAME: &str = "thread_create_intent_claim_v1";
pub(crate) const CREATE_INTENT_MIGRATION_VERSION: i64 = 1;

const CREATE_INTENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS thread_create_intents (
    id INTEGER PRIMARY KEY,
    scope_identity TEXT NOT NULL,
    scope_epoch INTEGER NOT NULL CHECK (scope_epoch >= 0),
    create_intent_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    fingerprint_version INTEGER NOT NULL CHECK (fingerprint_version = 1),
    request_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'reserved', 'preparing', 'committed', 'failed_before_commit'
    )),
    command_kind TEXT NOT NULL CHECK (command_kind IN (
        'create_only', 'create_and_dispatch'
    )),
    dispatch_client_intent_id TEXT,
    owner_boot_id TEXT,
    lease_expires_at TEXT,
    failure_code TEXT,
    failure_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    committed_at TEXT,
    CHECK (
        (scope_epoch = 0 AND scope_identity = '__legacy_api__')
        OR (scope_epoch > 0 AND scope_identity <> '__legacy_api__')
    ),
    CHECK (
        (command_kind = 'create_only' AND dispatch_client_intent_id IS NULL)
        OR
        (command_kind = 'create_and_dispatch'
            AND dispatch_client_intent_id IS NOT NULL)
    ),
    CHECK (state <> 'preparing' OR owner_boot_id IS NOT NULL),
    CHECK (state <> 'preparing' OR lease_expires_at IS NOT NULL),
    CHECK ((state = 'committed') = (committed_at IS NOT NULL)),
    UNIQUE (thread_id)
) STRICT;
"#;

const CREATE_INTENTS_INDEX_SQL: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_thread_create_intents_scope_intent
    ON thread_create_intents(scope_identity, scope_epoch, create_intent_id);
"#;

const CREATE_RESOURCES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS thread_create_resources (
    create_intent_row_id INTEGER NOT NULL,
    resource_kind TEXT NOT NULL CHECK (resource_kind IN (
        'managed_workspace', 'worktree', 'imported_transcript'
    )),
    resource_path TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'reserved', 'materializing', 'materialized',
        'adopted', 'delete_pending', 'deleted'
    )),
    owner_marker TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (create_intent_row_id, resource_kind, resource_path),
    FOREIGN KEY (create_intent_row_id)
        REFERENCES thread_create_intents(id) ON DELETE RESTRICT
) STRICT;
"#;

const CREATE_RESOURCES_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_thread_create_resources_cleanup
    ON thread_create_resources(state, next_attempt_at)
    WHERE state = 'delete_pending';
"#;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CreateIntentKey {
    pub scope_identity: String,
    pub scope_epoch: i64,
    pub create_intent_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateIntentState {
    Reserved,
    Preparing,
    Committed,
    FailedBeforeCommit,
}

impl CreateIntentState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Preparing => "preparing",
            Self::Committed => "committed",
            Self::FailedBeforeCommit => "failed_before_commit",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "reserved" => Ok(Self::Reserved),
            "preparing" => Ok(Self::Preparing),
            "committed" => Ok(Self::Committed),
            "failed_before_commit" => Ok(Self::FailedBeforeCommit),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid create-intent state '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateCommandKind {
    CreateOnly,
    CreateAndDispatch,
}

impl CreateCommandKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateOnly => "create_only",
            Self::CreateAndDispatch => "create_and_dispatch",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "create_only" => Ok(Self::CreateOnly),
            "create_and_dispatch" => Ok(Self::CreateAndDispatch),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid create command kind '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreateIntentRecord {
    pub id: i64,
    pub key: CreateIntentKey,
    pub thread_id: String,
    pub fingerprint_version: i64,
    pub request_fingerprint: String,
    pub state: CreateIntentState,
    pub command_kind: CreateCommandKind,
    pub dispatch_client_intent_id: Option<String>,
    pub owner_boot_id: Option<String>,
    pub lease_expires_at: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub committed_at: Option<String>,
}

pub(crate) struct NewCreateIntent<'a> {
    pub key: &'a CreateIntentKey,
    pub thread_id: &'a str,
    pub request_fingerprint: &'a str,
    pub command_kind: CreateCommandKind,
    pub dispatch_client_intent_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateResourceKind {
    ManagedWorkspace,
    Worktree,
    ImportedTranscript,
}

impl CreateResourceKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ManagedWorkspace => "managed_workspace",
            Self::Worktree => "worktree",
            Self::ImportedTranscript => "imported_transcript",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "managed_workspace" => Ok(Self::ManagedWorkspace),
            "worktree" => Ok(Self::Worktree),
            "imported_transcript" => Ok(Self::ImportedTranscript),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid create-resource kind '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateResourceState {
    Reserved,
    Materializing,
    Materialized,
    Adopted,
    DeletePending,
    Deleted,
}

impl CreateResourceState {
    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "reserved" => Ok(Self::Reserved),
            "materializing" => Ok(Self::Materializing),
            "materialized" => Ok(Self::Materialized),
            "adopted" => Ok(Self::Adopted),
            "delete_pending" => Ok(Self::DeletePending),
            "deleted" => Ok(Self::Deleted),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid create-resource state '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreateResourceRecord {
    pub create_intent_row_id: i64,
    pub kind: CreateResourceKind,
    pub resource_path: String,
    pub state: CreateResourceState,
    pub owner_marker: String,
    pub attempt_count: i64,
    pub next_attempt_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub(crate) struct CreateIntentSnapshot {
    pub claim: CreateIntentRecord,
    pub record_body: Option<String>,
    pub lifecycle: Option<ThreadTerminalState>,
    pub dispatch: Option<DispatchAdmissionRecord>,
}

fn canonical_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("CREATE TABLE IF NOT EXISTS", "CREATE TABLE")
        .replace("CREATE UNIQUE INDEX IF NOT EXISTS", "CREATE UNIQUE INDEX")
        .replace("CREATE INDEX IF NOT EXISTS", "CREATE INDEX")
        .trim_end_matches(';')
        .to_owned()
}

fn validate_schema(tx: &Transaction<'_>) -> GaryxDbResult<()> {
    for (kind, name, expected) in [
        ("table", "thread_create_intents", CREATE_INTENTS_TABLE_SQL),
        (
            "index",
            "idx_thread_create_intents_scope_intent",
            CREATE_INTENTS_INDEX_SQL,
        ),
        (
            "table",
            "thread_create_resources",
            CREATE_RESOURCES_TABLE_SQL,
        ),
        (
            "index",
            "idx_thread_create_resources_cleanup",
            CREATE_RESOURCES_INDEX_SQL,
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
                    "create-intent schema is missing {kind} '{name}'"
                ))
            })?;
        if canonical_schema_sql(&actual) != canonical_schema_sql(expected) {
            return Err(GaryxDbError::Configuration(format!(
                "create-intent {kind} '{name}' does not match the committed v1 schema"
            )));
        }
    }
    Ok(())
}

fn read_create_intent_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(
    i64,
    String,
    i64,
    String,
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
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
    ))
}

fn decode_create_intent(
    row: (
        i64,
        String,
        i64,
        String,
        String,
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
        Option<String>,
    ),
) -> GaryxDbResult<CreateIntentRecord> {
    Ok(CreateIntentRecord {
        id: row.0,
        key: CreateIntentKey {
            scope_identity: row.1,
            scope_epoch: row.2,
            create_intent_id: row.3,
        },
        thread_id: row.4,
        fingerprint_version: row.5,
        request_fingerprint: row.6,
        state: CreateIntentState::parse(&row.7)?,
        command_kind: CreateCommandKind::parse(&row.8)?,
        dispatch_client_intent_id: row.9,
        owner_boot_id: row.10,
        lease_expires_at: row.11,
        failure_code: row.12,
        failure_message: row.13,
        created_at: row.14,
        updated_at: row.15,
        committed_at: row.16,
    })
}

const CREATE_INTENT_SELECT: &str = "
    SELECT id, scope_identity, scope_epoch, create_intent_id, thread_id,
           fingerprint_version, request_fingerprint, state, command_kind,
           dispatch_client_intent_id, owner_boot_id, lease_expires_at,
           failure_code, failure_message, created_at, updated_at, committed_at
      FROM thread_create_intents
     WHERE scope_identity = ?1 AND scope_epoch = ?2 AND create_intent_id = ?3";

fn decode_create_resource_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CreateResourceRecord> {
    let kind = row.get::<_, String>(1)?;
    let state = row.get::<_, String>(3)?;
    let kind = CreateResourceKind::parse(&kind).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let state = CreateResourceState::parse(&state).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(CreateResourceRecord {
        create_intent_row_id: row.get(0)?,
        kind,
        resource_path: row.get(2)?,
        state,
        owner_marker: row.get(4)?,
        attempt_count: row.get(5)?,
        next_attempt_at: row.get(6)?,
        last_error: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn read_create_resource(
    conn: &Connection,
    create_intent_row_id: i64,
    kind: CreateResourceKind,
    resource_path: &str,
) -> GaryxDbResult<Option<CreateResourceRecord>> {
    Ok(conn
        .query_row(
            "SELECT create_intent_row_id, resource_kind, resource_path, state,
                    owner_marker, attempt_count, next_attempt_at, last_error,
                    created_at, updated_at
               FROM thread_create_resources
              WHERE create_intent_row_id = ?1 AND resource_kind = ?2 AND resource_path = ?3",
            params![create_intent_row_id, kind.as_str(), resource_path],
            decode_create_resource_row,
        )
        .optional()?)
}

impl GaryxDbService {
    pub(crate) fn migrate_thread_create_intent_claim_v1(&self) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let marker = tx
            .query_row(
                "SELECT projection_version FROM projection_states WHERE projection_name = ?1",
                params![CREATE_INTENT_MIGRATION_NAME],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if marker.is_some_and(|version| version != CREATE_INTENT_MIGRATION_VERSION) {
            return Err(GaryxDbError::Configuration(format!(
                "create-intent marker version mismatch: expected {}, found {}",
                CREATE_INTENT_MIGRATION_VERSION,
                marker.unwrap_or_default()
            )));
        }
        let foreign_keys: i64 = tx.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        if foreign_keys != 1 {
            return Err(GaryxDbError::Configuration(
                "create-intent migration requires PRAGMA foreign_keys=ON".to_owned(),
            ));
        }
        if marker.is_none() {
            tx.execute_batch(CREATE_INTENTS_TABLE_SQL)?;
            tx.execute_batch(CREATE_INTENTS_INDEX_SQL)?;
            tx.execute_batch(CREATE_RESOURCES_TABLE_SQL)?;
            tx.execute_batch(CREATE_RESOURCES_INDEX_SQL)?;
        }
        validate_schema(&tx)?;
        if marker.is_none() {
            record_projection_state_tx(
                &tx,
                CREATE_INTENT_MIGRATION_NAME,
                CREATE_INTENT_MIGRATION_VERSION,
                0,
                None,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn recover_stale_create_intents(&self) -> GaryxDbResult<usize> {
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE thread_create_resources
                SET state = 'delete_pending', next_attempt_at = ?1, updated_at = ?1
              WHERE create_intent_row_id IN (
                    SELECT id FROM thread_create_intents WHERE state = 'preparing'
              ) AND state IN ('materializing', 'materialized')",
            params![now],
        )?;
        let changed = tx.execute(
            "UPDATE thread_create_intents
                SET state = 'reserved', owner_boot_id = NULL, lease_expires_at = NULL,
                    updated_at = ?1
              WHERE state = 'preparing'",
            params![now],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    pub(crate) fn reserve_create_intent(
        &self,
        input: NewCreateIntent<'_>,
    ) -> GaryxDbResult<CreateIntentRecord> {
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO thread_create_intents (
                scope_identity, scope_epoch, create_intent_id, thread_id,
                fingerprint_version, request_fingerprint, state, command_kind,
                dispatch_client_intent_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5, 'reserved', ?6, ?7, ?8, ?8)
             ON CONFLICT(scope_identity, scope_epoch, create_intent_id) DO NOTHING",
            params![
                input.key.scope_identity,
                input.key.scope_epoch,
                input.key.create_intent_id,
                input.thread_id,
                input.request_fingerprint,
                input.command_kind.as_str(),
                input.dispatch_client_intent_id,
                now,
            ],
        )?;
        drop(conn);
        self.create_intent(input.key)?.ok_or_else(|| {
            GaryxDbError::Configuration("create-intent reservation was not readable".to_owned())
        })
    }

    pub(crate) fn create_intent(
        &self,
        key: &CreateIntentKey,
    ) -> GaryxDbResult<Option<CreateIntentRecord>> {
        let conn = self.read_conn()?;
        conn.query_row(
            CREATE_INTENT_SELECT,
            params![key.scope_identity, key.scope_epoch, key.create_intent_id],
            read_create_intent_row,
        )
        .optional()?
        .map(decode_create_intent)
        .transpose()
    }

    pub(crate) fn mark_create_intent_preparing(
        &self,
        key: &CreateIntentKey,
        owner_boot_id: &str,
        lease_expires_at: &str,
    ) -> GaryxDbResult<bool> {
        let now = now_string();
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE thread_create_intents
                SET state = 'preparing', owner_boot_id = ?1, lease_expires_at = ?2,
                    failure_code = NULL, failure_message = NULL, updated_at = ?3
              WHERE scope_identity = ?4 AND scope_epoch = ?5 AND create_intent_id = ?6
                AND state = 'reserved'",
            params![
                owner_boot_id,
                lease_expires_at,
                now,
                key.scope_identity,
                key.scope_epoch,
                key.create_intent_id,
            ],
        )? == 1)
    }

    pub(crate) fn renew_create_intent_lease(
        &self,
        key: &CreateIntentKey,
        owner_boot_id: &str,
        lease_expires_at: &str,
    ) -> GaryxDbResult<bool> {
        let now = now_string();
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE thread_create_intents
                SET lease_expires_at = ?1, updated_at = ?2
              WHERE scope_identity = ?3 AND scope_epoch = ?4 AND create_intent_id = ?5
                AND state = 'preparing' AND owner_boot_id = ?6",
            params![
                lease_expires_at,
                now,
                key.scope_identity,
                key.scope_epoch,
                key.create_intent_id,
                owner_boot_id,
            ],
        )? == 1)
    }

    pub(crate) fn begin_create_resource_materialization(
        &self,
        key: &CreateIntentKey,
        kind: CreateResourceKind,
        resource_path: &str,
        owner_marker: &str,
    ) -> GaryxDbResult<CreateResourceRecord> {
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claim = tx
            .query_row(
                CREATE_INTENT_SELECT,
                params![key.scope_identity, key.scope_epoch, key.create_intent_id],
                read_create_intent_row,
            )
            .optional()?
            .map(decode_create_intent)
            .transpose()?
            .ok_or_else(|| GaryxDbError::BadRequest("create intent not found".to_owned()))?;
        if claim.state != CreateIntentState::Preparing {
            return Err(GaryxDbError::BadRequest(format!(
                "create intent is not preparing: {}",
                claim.state.as_str()
            )));
        }
        let existing = tx
            .query_row(
                "SELECT state, owner_marker FROM thread_create_resources
                  WHERE create_intent_row_id = ?1 AND resource_kind = ?2 AND resource_path = ?3",
                params![claim.id, kind.as_str(), resource_path],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((state, marker)) = existing {
            let state = CreateResourceState::parse(&state)?;
            match state {
                CreateResourceState::Adopted => {
                    return Err(GaryxDbError::BadRequest(
                        "create resource is already adopted".to_owned(),
                    ));
                }
                CreateResourceState::DeletePending => {
                    return Err(GaryxDbError::BadRequest(
                        "create resource still requires cleanup".to_owned(),
                    ));
                }
                CreateResourceState::Materializing | CreateResourceState::Materialized
                    if marker == owner_marker => {}
                _ => {
                    tx.execute(
                        "UPDATE thread_create_resources
                            SET state = 'materializing', owner_marker = ?4,
                                attempt_count = attempt_count + 1, next_attempt_at = NULL,
                                last_error = NULL, updated_at = ?5
                          WHERE create_intent_row_id = ?1 AND resource_kind = ?2
                            AND resource_path = ?3",
                        params![claim.id, kind.as_str(), resource_path, owner_marker, now],
                    )?;
                }
            }
        } else {
            tx.execute(
                "INSERT INTO thread_create_resources (
                    create_intent_row_id, resource_kind, resource_path, state,
                    owner_marker, attempt_count, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'materializing', ?4, 1, ?5, ?5)",
                params![claim.id, kind.as_str(), resource_path, owner_marker, now],
            )?;
        }
        let record = read_create_resource(&tx, claim.id, kind, resource_path)?
            .ok_or_else(|| GaryxDbError::Configuration("create resource disappeared".to_owned()))?;
        tx.commit()?;
        Ok(record)
    }

    pub(crate) fn mark_create_resource_materialized(
        &self,
        key: &CreateIntentKey,
        kind: CreateResourceKind,
        resource_path: &str,
        owner_marker: &str,
    ) -> GaryxDbResult<bool> {
        let now = now_string();
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE thread_create_resources
                SET state = 'materialized', updated_at = ?1
              WHERE create_intent_row_id = (
                    SELECT id FROM thread_create_intents
                     WHERE scope_identity = ?2 AND scope_epoch = ?3 AND create_intent_id = ?4
              ) AND resource_kind = ?5 AND resource_path = ?6
                AND owner_marker = ?7 AND state IN ('materializing', 'materialized')",
            params![
                now,
                key.scope_identity,
                key.scope_epoch,
                key.create_intent_id,
                kind.as_str(),
                resource_path,
                owner_marker,
            ],
        )? == 1)
    }

    pub(crate) fn create_resources_for_intent(
        &self,
        key: &CreateIntentKey,
    ) -> GaryxDbResult<Vec<CreateResourceRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.create_intent_row_id, r.resource_kind, r.resource_path, r.state,
                    r.owner_marker, r.attempt_count, r.next_attempt_at, r.last_error,
                    r.created_at, r.updated_at
               FROM thread_create_resources r
               JOIN thread_create_intents i ON i.id = r.create_intent_row_id
              WHERE i.scope_identity = ?1 AND i.scope_epoch = ?2 AND i.create_intent_id = ?3
              ORDER BY r.resource_kind, r.resource_path",
        )?;
        let rows = stmt.query_map(
            params![key.scope_identity, key.scope_epoch, key.create_intent_id],
            decode_create_resource_row,
        )?;
        rows.map(|row| row.map_err(Into::into)).collect()
    }

    pub(crate) fn due_create_resource_cleanup_intents(
        &self,
        now: &str,
        limit: usize,
    ) -> GaryxDbResult<Vec<(CreateIntentKey, String)>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT i.scope_identity, i.scope_epoch, i.create_intent_id, i.thread_id
               FROM thread_create_resources r
               JOIN thread_create_intents i ON i.id = r.create_intent_row_id
              WHERE r.state = 'delete_pending'
                AND (r.next_attempt_at IS NULL OR r.next_attempt_at <= ?1)
              ORDER BY r.updated_at, i.id
              LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now, limit], |row| {
            Ok((
                CreateIntentKey {
                    scope_identity: row.get(0)?,
                    scope_epoch: row.get(1)?,
                    create_intent_id: row.get(2)?,
                },
                row.get(3)?,
            ))
        })?;
        rows.map(|row| row.map_err(Into::into)).collect()
    }

    pub(crate) fn mark_create_resource_deleted(
        &self,
        create_intent_row_id: i64,
        kind: CreateResourceKind,
        resource_path: &str,
        owner_marker: &str,
    ) -> GaryxDbResult<bool> {
        let now = now_string();
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE thread_create_resources
                SET state = 'deleted', next_attempt_at = NULL, last_error = NULL, updated_at = ?1
              WHERE create_intent_row_id = ?2 AND resource_kind = ?3
                AND resource_path = ?4 AND owner_marker = ?5
                AND state IN ('delete_pending', 'materializing', 'materialized')",
            params![
                now,
                create_intent_row_id,
                kind.as_str(),
                resource_path,
                owner_marker,
            ],
        )? == 1)
    }

    pub(crate) fn fail_create_resource_cleanup(
        &self,
        create_intent_row_id: i64,
        kind: CreateResourceKind,
        resource_path: &str,
        owner_marker: &str,
        error: &str,
    ) -> GaryxDbResult<()> {
        let now = now_string();
        let next = (Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
        let error = error.chars().take(2048).collect::<String>();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE thread_create_resources
                SET state = 'delete_pending', next_attempt_at = ?1,
                    last_error = ?2, updated_at = ?3
              WHERE create_intent_row_id = ?4 AND resource_kind = ?5
                AND resource_path = ?6 AND owner_marker = ?7",
            params![
                next,
                error,
                now,
                create_intent_row_id,
                kind.as_str(),
                resource_path,
                owner_marker,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn fail_create_intent_before_commit(
        &self,
        key: &CreateIntentKey,
        code: &str,
        message: &str,
    ) -> GaryxDbResult<bool> {
        let now = now_string();
        let bounded = message.chars().take(2048).collect::<String>();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let id = tx
            .query_row(
                "SELECT id FROM thread_create_intents
                  WHERE scope_identity = ?1 AND scope_epoch = ?2 AND create_intent_id = ?3
                    AND state IN ('reserved', 'preparing')",
                params![key.scope_identity, key.scope_epoch, key.create_intent_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(false);
        };
        tx.execute(
            "UPDATE thread_create_resources
                SET state = 'delete_pending', next_attempt_at = ?1,
                    last_error = ?2, updated_at = ?1
              WHERE create_intent_row_id = ?3
                AND state IN ('reserved', 'materializing', 'materialized')",
            params![now, bounded, id],
        )?;
        let changed = tx.execute(
            "UPDATE thread_create_intents
                SET state = 'failed_before_commit', owner_boot_id = NULL,
                    lease_expires_at = NULL, failure_code = ?1,
                    failure_message = ?2, updated_at = ?3
              WHERE id = ?4 AND state IN ('reserved', 'preparing')",
            params![code, bounded, now, id],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub(crate) fn release_create_intent_after_preparation_failure(
        &self,
        key: &CreateIntentKey,
        error: &str,
    ) -> GaryxDbResult<bool> {
        let now = now_string();
        let bounded = error.chars().take(2048).collect::<String>();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let id = tx
            .query_row(
                "SELECT id FROM thread_create_intents
                  WHERE scope_identity = ?1 AND scope_epoch = ?2 AND create_intent_id = ?3
                    AND state = 'preparing'",
                params![key.scope_identity, key.scope_epoch, key.create_intent_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(false);
        };
        tx.execute(
            "UPDATE thread_create_resources
                SET state = 'delete_pending', next_attempt_at = ?1,
                    last_error = ?2, updated_at = ?1
              WHERE create_intent_row_id = ?3
                AND state IN ('reserved', 'materializing', 'materialized')",
            params![now, bounded, id],
        )?;
        let changed = tx.execute(
            "UPDATE thread_create_intents
                SET state = 'reserved', owner_boot_id = NULL, lease_expires_at = NULL,
                    updated_at = ?1
              WHERE id = ?2 AND state = 'preparing'",
            params![now, id],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub(crate) fn commit_create_intent_records(
        &self,
        key: &CreateIntentKey,
        request_fingerprint: &str,
        target_thread_id: &str,
        records: Vec<ThreadRecordWrite>,
        dispatch: Option<NewDispatchAdmission<'_>>,
        attachment_claims: &[PromptAttachmentClaim],
    ) -> GaryxDbResult<()> {
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claim = tx
            .query_row(
                CREATE_INTENT_SELECT,
                params![key.scope_identity, key.scope_epoch, key.create_intent_id],
                read_create_intent_row,
            )
            .optional()?
            .map(decode_create_intent)
            .transpose()?
            .ok_or_else(|| GaryxDbError::BadRequest("create intent not found".to_owned()))?;
        if claim.fingerprint_version != 1 || claim.request_fingerprint != request_fingerprint {
            return Err(GaryxDbError::BadRequest(
                "create intent fingerprint conflict".to_owned(),
            ));
        }
        if claim.thread_id != target_thread_id {
            return Err(GaryxDbError::Configuration(
                "create-intent thread id does not match record write".to_owned(),
            ));
        }
        if !matches!(
            claim.state,
            CreateIntentState::Reserved | CreateIntentState::Preparing
        ) {
            return Err(GaryxDbError::BadRequest(format!(
                "create intent is already {}",
                claim.state.as_str()
            )));
        }
        let record_exists = tx
            .query_row(
                "SELECT 1 FROM thread_records WHERE key = ?1",
                params![target_thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if record_exists {
            return Err(GaryxDbError::Configuration(format!(
                "reserved thread id already exists: {}",
                target_thread_id
            )));
        }
        if !records.iter().any(|record| record.key == target_thread_id) {
            return Err(GaryxDbError::Configuration(
                "atomic create write set omitted its target thread".to_owned(),
            ));
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
        match (&dispatch, claim.command_kind) {
            (Some(dispatch), CreateCommandKind::CreateAndDispatch) => {
                if dispatch.key.thread_id != claim.thread_id
                    || Some(dispatch.key.client_intent_id.as_str())
                        != claim.dispatch_client_intent_id.as_deref()
                {
                    return Err(GaryxDbError::Configuration(
                        "create-intent dispatch identity does not match claim".to_owned(),
                    ));
                }
                insert_dispatch_admission_tx(&tx, dispatch, &now)?;
                if !attachment_claims.is_empty() {
                    let effective_run_id = dispatch.effective_run_id.ok_or_else(|| {
                        GaryxDbError::BadRequest(
                            "managed attachment admission requires an effective run id".to_owned(),
                        )
                    })?;
                    claim_prompt_attachments_tx(
                        &tx,
                        attachment_claims,
                        &PromptAttachmentOwner {
                            scope_identity: &dispatch.key.scope_identity,
                            scope_epoch: dispatch.key.scope_epoch,
                            thread_id: &dispatch.key.thread_id,
                            kind: dispatch.key.kind,
                            client_intent_id: Some(&dispatch.key.client_intent_id),
                            requested_run_id: dispatch.requested_run_id,
                            effective_run_id,
                        },
                        &now,
                    )?;
                }
            }
            (None, CreateCommandKind::CreateOnly) => {}
            _ => {
                return Err(GaryxDbError::Configuration(
                    "create command kind does not match dispatch payload".to_owned(),
                ));
            }
        }
        let changed = tx.execute(
            "UPDATE thread_create_intents
                SET state = 'committed', owner_boot_id = NULL,
                    lease_expires_at = NULL, committed_at = ?1, updated_at = ?1
              WHERE id = ?2 AND state IN ('reserved', 'preparing')",
            params![now, claim.id],
        )?;
        if changed != 1 {
            return Err(GaryxDbError::Configuration(
                "create-intent commit lost its reservation".to_owned(),
            ));
        }
        tx.execute(
            "UPDATE thread_create_resources
                SET state = 'adopted', updated_at = ?1
              WHERE create_intent_row_id = ?2 AND state = 'materialized'",
            params![now, claim.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn create_intent_snapshot(
        &self,
        key: &CreateIntentKey,
    ) -> GaryxDbResult<Option<CreateIntentSnapshot>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let Some(claim) = tx
            .query_row(
                CREATE_INTENT_SELECT,
                params![key.scope_identity, key.scope_epoch, key.create_intent_id],
                read_create_intent_row,
            )
            .optional()?
            .map(decode_create_intent)
            .transpose()?
        else {
            return Ok(None);
        };
        let record_body = tx
            .query_row(
                "SELECT body FROM thread_records WHERE key = ?1",
                params![claim.thread_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let lifecycle = read_thread_terminal_state(&tx, &claim.thread_id)?;
        let dispatch = match claim.dispatch_client_intent_id.as_deref() {
            Some(client_intent_id) => tx
                .query_row(
                    DISPATCH_ADMISSION_SELECT,
                    params![
                        claim.key.scope_identity,
                        claim.key.scope_epoch,
                        claim.thread_id,
                        DispatchAdmissionKind::ChatStart.as_str(),
                        client_intent_id,
                    ],
                    read_dispatch_admission_row,
                )
                .optional()?
                .map(decode_dispatch_admission)
                .transpose()?,
            None => None,
        };
        tx.commit()?;
        Ok(Some(CreateIntentSnapshot {
            claim,
            record_body,
            lifecycle,
            dispatch,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(intent: &str) -> CreateIntentKey {
        CreateIntentKey {
            scope_identity: "create-test-client".to_owned(),
            scope_epoch: 1,
            create_intent_id: intent.to_owned(),
        }
    }

    #[test]
    fn marker_and_unique_claim_survive_reopen_semantics() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_thread_create_intent_claim_v1().unwrap();
        db.migrate_thread_create_intent_claim_v1().unwrap();
        let key = key("same-intent");
        let first = db
            .reserve_create_intent(NewCreateIntent {
                key: &key,
                thread_id: "thread::fixed-create-id",
                request_fingerprint: "fingerprint-a",
                command_kind: CreateCommandKind::CreateOnly,
                dispatch_client_intent_id: None,
            })
            .unwrap();
        let replay = db
            .reserve_create_intent(NewCreateIntent {
                key: &key,
                thread_id: "thread::must-not-win",
                request_fingerprint: "fingerprint-a",
                command_kind: CreateCommandKind::CreateOnly,
                dispatch_client_intent_id: None,
            })
            .unwrap();
        assert_eq!(first.thread_id, "thread::fixed-create-id");
        assert_eq!(replay.thread_id, first.thread_id);
        assert_eq!(replay.request_fingerprint, "fingerprint-a");

        let query_plan = db
            .conn()
            .unwrap()
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM thread_create_intents
                  WHERE scope_identity = ?1 AND scope_epoch = ?2 AND create_intent_id = ?3",
                params![key.scope_identity, key.scope_epoch, key.create_intent_id],
                |row| row.get::<_, String>(3),
            )
            .unwrap();
        assert!(
            query_plan.contains("idx_thread_create_intents_scope_intent"),
            "create-intent recovery must remain a point lookup: {query_plan}"
        );
    }

    #[test]
    fn committed_marker_does_not_recreate_a_missing_claim_index() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_thread_create_intent_claim_v1().unwrap();
        db.conn()
            .unwrap()
            .execute_batch("DROP INDEX idx_thread_create_intents_scope_intent")
            .unwrap();

        let error = db
            .migrate_thread_create_intent_claim_v1()
            .expect_err("a committed marker must not repair a missing claim index");
        assert!(matches!(error, GaryxDbError::Configuration(_)));
        let index_count: i64 = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'index' AND name = 'idx_thread_create_intents_scope_intent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 0, "marked schema drift must remain unrepaired");
    }

    #[test]
    fn migration_requires_foreign_keys_and_resource_owner_is_restricted() {
        let disabled = GaryxDbService::memory().unwrap();
        disabled
            .conn()
            .unwrap()
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        let error = disabled
            .migrate_thread_create_intent_claim_v1()
            .expect_err("create-intent ownership requires foreign keys");
        assert!(matches!(error, GaryxDbError::Configuration(_)));

        let db = GaryxDbService::memory().unwrap();
        db.migrate_thread_create_intent_claim_v1().unwrap();
        let key = key("restricted-resource-owner");
        let claim = db
            .reserve_create_intent(NewCreateIntent {
                key: &key,
                thread_id: "thread::restricted-resource-owner",
                request_fingerprint: "fingerprint",
                command_kind: CreateCommandKind::CreateOnly,
                dispatch_client_intent_id: None,
            })
            .unwrap();
        assert!(
            db.mark_create_intent_preparing(&key, "test-boot", "2099-01-01T00:00:00Z")
                .unwrap()
        );
        db.begin_create_resource_materialization(
            &key,
            CreateResourceKind::ManagedWorkspace,
            "/tmp/test-restricted-thread-workspace",
            "owner",
        )
        .unwrap();

        let error = db
            .conn()
            .unwrap()
            .execute(
                "DELETE FROM thread_create_intents WHERE id = ?1",
                params![claim.id],
            )
            .expect_err("ON DELETE RESTRICT must preserve a referenced create intent");
        match error {
            rusqlite::Error::SqliteFailure(code, _) => {
                assert_eq!(code.code, rusqlite::ErrorCode::ConstraintViolation)
            }
            other => panic!("expected a SQLite constraint violation, got {other:?}"),
        }
        assert!(db.create_intent(&key).unwrap().is_some());
    }

    #[test]
    fn old_boot_preparation_returns_to_reserved() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_thread_create_intent_claim_v1().unwrap();
        let key = key("recover-preparing");
        db.reserve_create_intent(NewCreateIntent {
            key: &key,
            thread_id: "thread::recover-preparing",
            request_fingerprint: "fingerprint",
            command_kind: CreateCommandKind::CreateOnly,
            dispatch_client_intent_id: None,
        })
        .unwrap();
        assert!(
            db.mark_create_intent_preparing(&key, "old-boot", "2099-01-01T00:00:00Z")
                .unwrap()
        );
        assert_eq!(db.recover_stale_create_intents().unwrap(), 1);
        assert_eq!(
            db.create_intent(&key).unwrap().unwrap().state,
            CreateIntentState::Reserved
        );
    }

    #[test]
    fn old_boot_materialized_resource_requires_cleanup_before_reuse() {
        let db = GaryxDbService::memory().unwrap();
        db.migrate_thread_create_intent_claim_v1().unwrap();
        let key = key("recover-resource");
        db.reserve_create_intent(NewCreateIntent {
            key: &key,
            thread_id: "thread::recover-resource",
            request_fingerprint: "fingerprint",
            command_kind: CreateCommandKind::CreateOnly,
            dispatch_client_intent_id: None,
        })
        .unwrap();
        db.mark_create_intent_preparing(&key, "old-boot", "2099-01-01T00:00:00Z")
            .unwrap();
        db.begin_create_resource_materialization(
            &key,
            CreateResourceKind::ManagedWorkspace,
            "/tmp/test-thread-workspace",
            "owner-one",
        )
        .unwrap();
        assert!(
            db.mark_create_resource_materialized(
                &key,
                CreateResourceKind::ManagedWorkspace,
                "/tmp/test-thread-workspace",
                "owner-one",
            )
            .unwrap()
        );

        assert_eq!(db.recover_stale_create_intents().unwrap(), 1);
        let resources = db.create_resources_for_intent(&key).unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].state, CreateResourceState::DeletePending);
        let error = db
            .begin_create_resource_materialization(
                &key,
                CreateResourceKind::ManagedWorkspace,
                "/tmp/test-thread-workspace",
                "owner-two",
            )
            .unwrap_err();
        assert!(error.to_string().contains("not preparing"));
    }
}
