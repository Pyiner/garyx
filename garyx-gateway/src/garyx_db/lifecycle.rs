//! Thread lifecycle operations, terminal states, and the cleanup outbox.

use super::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOperationKind {
    Archive,
    Delete,
}

impl LifecycleOperationKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Delete => "delete",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "archive" => Ok(Self::Archive),
            "delete" => Ok(Self::Delete),
            other => Err(GaryxDbError::Configuration(format!(
                "invalid lifecycle operation kind '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOperationOutcome {
    AppliedChanged,
    AppliedNoop,
    RejectedConflict,
    RejectedNotFound,
}

impl LifecycleOperationOutcome {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::AppliedChanged => "applied_changed",
            Self::AppliedNoop => "applied_noop",
            Self::RejectedConflict => "rejected_conflict",
            Self::RejectedNotFound => "rejected_not_found",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "applied_changed" => Ok(Self::AppliedChanged),
            "applied_noop" => Ok(Self::AppliedNoop),
            "rejected_conflict" => Ok(Self::RejectedConflict),
            "rejected_not_found" => Ok(Self::RejectedNotFound),
            other => Err(GaryxDbError::Configuration(format!(
                "invalid lifecycle operation outcome '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleOperationRecord {
    pub store_incarnation: String,
    pub operation_id: String,
    pub kind: LifecycleOperationKind,
    pub thread_id: String,
    pub fingerprint: String,
    pub outcome: LifecycleOperationOutcome,
    pub result_payload: Option<Value>,
    pub detail: Option<Value>,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LifecycleOperationLookup {
    Current(Option<LifecycleOperationRecord>),
    WrongIncarnation { current_store_incarnation: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupOutboxStep {
    EndpointRuntimeInvalidate,
    RuntimeTeardown,
    TranscriptRemove,
    ThreadLogRemove,
    PromptAttachmentsRemove,
}

impl CleanupOutboxStep {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::EndpointRuntimeInvalidate => "endpoint_runtime_invalidate",
            Self::RuntimeTeardown => "runtime_teardown",
            Self::TranscriptRemove => "transcript_remove",
            Self::ThreadLogRemove => "thread_log_remove",
            Self::PromptAttachmentsRemove => "prompt_attachments_remove",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "endpoint_runtime_invalidate" => Ok(Self::EndpointRuntimeInvalidate),
            "runtime_teardown" => Ok(Self::RuntimeTeardown),
            "transcript_remove" => Ok(Self::TranscriptRemove),
            "thread_log_remove" => Ok(Self::ThreadLogRemove),
            "prompt_attachments_remove" => Ok(Self::PromptAttachmentsRemove),
            other => Err(GaryxDbError::Configuration(format!(
                "invalid cleanup outbox step '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupOutboxJob {
    pub job_id: i64,
    pub thread_id: String,
    pub step: CleanupOutboxStep,
    pub payload: Option<Value>,
    pub attempt_count: u32,
    pub next_attempt_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleMutationInput {
    pub expected_store_incarnation: String,
    pub operation_id: String,
    pub kind: LifecycleOperationKind,
    pub thread_id: String,
    pub fingerprint: String,
    pub endpoint_keys: Vec<String>,
    pub enabled_channel_accounts: BTreeSet<(String, String)>,
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleDecisionInput {
    pub expected_store_incarnation: String,
    pub operation_id: String,
    pub kind: LifecycleOperationKind,
    pub thread_id: String,
    pub fingerprint: String,
    pub outcome: LifecycleOperationOutcome,
    pub detail: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LifecycleTransactionResult {
    Completed {
        operation: LifecycleOperationRecord,
        durable_terminal: Option<ThreadTerminalState>,
    },
    Existing {
        operation: LifecycleOperationRecord,
        durable_terminal: Option<ThreadTerminalState>,
    },
    WrongIncarnation {
        current_store_incarnation: String,
    },
}

pub(super) enum LifecycleMatrixAction {
    ProceedActive,
    ApplyChanged(ThreadTerminalState),
    ApplyNoop(ThreadTerminalState, bool),
    RejectNotFound(Option<ThreadTerminalState>),
}

pub(super) fn lifecycle_matrix(
    kind: LifecycleOperationKind,
    record_exists: bool,
    terminal: Option<ThreadTerminalState>,
) -> LifecycleMatrixAction {
    match (kind, record_exists, terminal) {
        (LifecycleOperationKind::Archive, true, None)
        | (LifecycleOperationKind::Delete, true, None) => LifecycleMatrixAction::ProceedActive,
        (LifecycleOperationKind::Archive, false, None) => {
            LifecycleMatrixAction::ApplyNoop(ThreadTerminalState::Archived, true)
        }
        (LifecycleOperationKind::Delete, false, None) => {
            LifecycleMatrixAction::RejectNotFound(None)
        }
        (LifecycleOperationKind::Archive, _, Some(ThreadTerminalState::Archived)) => {
            LifecycleMatrixAction::ApplyNoop(ThreadTerminalState::Archived, false)
        }
        (LifecycleOperationKind::Delete, _, Some(ThreadTerminalState::Archived)) => {
            LifecycleMatrixAction::ApplyChanged(ThreadTerminalState::Deleted)
        }
        (LifecycleOperationKind::Archive, _, Some(ThreadTerminalState::Deleted)) => {
            LifecycleMatrixAction::RejectNotFound(Some(ThreadTerminalState::Deleted))
        }
        (LifecycleOperationKind::Delete, _, Some(ThreadTerminalState::Deleted)) => {
            LifecycleMatrixAction::ApplyNoop(ThreadTerminalState::Deleted, false)
        }
    }
}

pub(super) fn json_detail(code: &str, message: &str) -> Value {
    serde_json::json!({ "code": code, "message": message })
}

pub(super) fn lifecycle_not_found_detail(
    thread_id: &str,
    terminal: Option<ThreadTerminalState>,
) -> Value {
    serde_json::json!({
        "code": "thread_not_found",
        "message": format!("thread not found: {thread_id}"),
        "terminal_kind": terminal.map(ThreadTerminalState::as_str),
    })
}

pub(super) fn read_thread_terminal_state(
    conn: &Connection,
    thread_id: &str,
) -> GaryxDbResult<Option<ThreadTerminalState>> {
    conn.query_row(
        "SELECT kind FROM archived_threads WHERE thread_id = ?1",
        params![thread_id],
        |row| row.get::<_, String>(0),
    )
    .optional()?
    .map(|kind| parse_terminal_state(&kind))
    .transpose()
}

pub(super) fn parse_terminal_state(value: &str) -> GaryxDbResult<ThreadTerminalState> {
    ThreadTerminalState::parse(value).ok_or_else(|| {
        GaryxDbError::Configuration(format!("invalid terminal thread tombstone kind '{value}'"))
    })
}

pub(super) fn read_lifecycle_operation(
    conn: &Connection,
    store_incarnation: &str,
    operation_id: &str,
) -> GaryxDbResult<Option<LifecycleOperationRecord>> {
    let row = conn
        .query_row(
            "SELECT kind, thread_id, fingerprint, outcome, result_payload,
                    detail, completed_at
               FROM lifecycle_operations
              WHERE store_incarnation = ?1 AND operation_id = ?2",
            params![store_incarnation, operation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, thread_id, fingerprint, outcome, result_payload, detail, completed_at)) = row
    else {
        return Ok(None);
    };
    Ok(Some(LifecycleOperationRecord {
        store_incarnation: store_incarnation.to_owned(),
        operation_id: operation_id.to_owned(),
        kind: LifecycleOperationKind::parse(&kind)?,
        thread_id,
        fingerprint,
        outcome: LifecycleOperationOutcome::parse(&outcome)?,
        result_payload: result_payload
            .map(|payload| serde_json::from_str(&payload))
            .transpose()
            .map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "invalid lifecycle result payload for operation '{operation_id}': {error}"
                ))
            })?,
        detail: detail
            .map(|payload| serde_json::from_str(&payload))
            .transpose()
            .map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "invalid lifecycle detail for operation '{operation_id}': {error}"
                ))
            })?,
        completed_at,
    }))
}

pub(super) fn insert_lifecycle_operation(
    tx: &Transaction<'_>,
    operation: &LifecycleOperationRecord,
) -> GaryxDbResult<()> {
    let result_payload = operation
        .result_payload
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| GaryxDbError::Configuration(error.to_string()))?;
    let detail = operation
        .detail
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| GaryxDbError::Configuration(error.to_string()))?;
    tx.execute(
        "INSERT INTO lifecycle_operations (
            store_incarnation, operation_id, kind, thread_id, fingerprint,
            outcome, result_payload, detail, completed_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            operation.store_incarnation,
            operation.operation_id,
            operation.kind.as_str(),
            operation.thread_id,
            operation.fingerprint,
            operation.outcome.as_str(),
            result_payload,
            detail,
            operation.completed_at,
        ],
    )?;
    Ok(())
}

pub(super) fn enqueue_cleanup_job(
    tx: &Transaction<'_>,
    thread_id: &str,
    step: CleanupOutboxStep,
    payload: Option<&Value>,
) -> GaryxDbResult<()> {
    let payload = payload
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| GaryxDbError::Configuration(error.to_string()))?;
    tx.execute(
        "INSERT INTO cleanup_outbox (
            thread_id, step, payload, status, attempt_count, next_attempt_at,
            created_at, settled_at
         ) VALUES (?1, ?2, ?3, 'pending', 0, NULL, ?4, NULL)",
        params![thread_id, step.as_str(), payload, now_string()],
    )?;
    Ok(())
}

pub(super) fn cleanup_job_from_row(
    row: (
        i64,
        String,
        String,
        Option<String>,
        i64,
        Option<String>,
        String,
    ),
) -> GaryxDbResult<CleanupOutboxJob> {
    let (job_id, thread_id, step, payload, attempt_count, next_attempt_at, created_at) = row;
    Ok(CleanupOutboxJob {
        job_id,
        thread_id,
        step: CleanupOutboxStep::parse(&step)?,
        payload: payload
            .map(|payload| serde_json::from_str(&payload))
            .transpose()
            .map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "invalid cleanup payload for job {job_id}: {error}"
                ))
            })?,
        attempt_count: u32::try_from(attempt_count).map_err(|_| {
            GaryxDbError::Configuration(format!(
                "invalid cleanup attempt_count {attempt_count} for job {job_id}"
            ))
        })?,
        next_attempt_at,
        created_at,
    })
}

impl GaryxDbService {
    /// Product archive semantics in one transaction: write the tombstone
    /// and delete the record, its projection rows, pin, and favorite together.
    /// Returns whether a record existed. Nothing is left to repair on any
    /// other path — a write racing this transaction either lands before
    /// the tombstone (and is deleted here) or is rejected by the in-tx
    /// tombstone check in `write_thread_record_with_projections`.
    ///
    /// Test-only tombstone seeding helper: the production archive path is
    /// `execute_lifecycle_mutation`. The `cfg(test)` gate is the structural
    /// replacement for the retired source-scan guard — a production call
    /// site simply does not compile.
    #[cfg(test)]
    pub(crate) fn archive_thread_record(&self, thread_id: &str) -> GaryxDbResult<bool> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_block_test_db_mutation(TestDbMutationPoint::ArchiveThreadRecord);
        let thread_id = normalize_thread_id(thread_id)?;
        let archived_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let terminal = read_thread_terminal_state(&tx, &thread_id)?;
        if terminal == Some(ThreadTerminalState::Deleted)
            || terminal == Some(ThreadTerminalState::Archived)
        {
            return Ok(false);
        }
        let record_exists = tx
            .query_row(
                "SELECT 1 FROM thread_records WHERE key = ?1",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        tx.execute(
            "INSERT INTO archived_threads (thread_id, archived_at, kind)
             VALUES (?1, ?2, 'archived')
             ON CONFLICT(thread_id) DO UPDATE SET
                archived_at = excluded.archived_at,
                kind = CASE
                    WHEN archived_threads.kind = 'deleted' THEN 'deleted'
                    ELSE excluded.kind
                END",
            params![thread_id, archived_at],
        )?;
        if !record_exists {
            tx.commit()?;
            return Ok(false);
        }
        let removed = tx.execute(
            "DELETE FROM thread_records WHERE key = ?1",
            params![thread_id],
        )? > 0;
        remove_thread_meta_projection_tx(&tx, &thread_id)?;
        remove_task_projection_tx(&tx, &thread_id)?;
        remove_recent_thread_tx(&tx, &thread_id)?;
        let removed_pin = tx.execute(
            "DELETE FROM thread_pins WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        bump_thread_pins_revision_if_changed_tx(&tx, removed_pin)?;
        let removed_favorite = tx.execute(
            "DELETE FROM thread_favorites WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        // Archive tombstones prevent record resurrection, so a missing
        // favorite needs no extra fence; only a changed collection bumps.
        bump_thread_favorites_revision_if_changed_tx(&tx, removed_favorite)?;
        tx.commit()?;
        Ok(removed)
    }

    pub fn is_thread_archived(&self, thread_id: &str) -> GaryxDbResult<bool> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::ArchivedThreadRead)?;
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        let archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM archived_threads WHERE thread_id = ?1",
                params![thread_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(archived.is_some())
    }

    pub fn thread_terminal_state(
        &self,
        thread_id: &str,
    ) -> GaryxDbResult<Option<ThreadTerminalState>> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::ArchivedThreadRead)?;
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT kind FROM archived_threads WHERE thread_id = ?1",
            params![thread_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|kind| parse_terminal_state(&kind))
        .transpose()
    }

    pub fn lifecycle_operation(
        &self,
        store_incarnation: &str,
        operation_id: &str,
    ) -> GaryxDbResult<Option<LifecycleOperationRecord>> {
        let conn = self.read_conn()?;
        read_lifecycle_operation(&conn, store_incarnation, operation_id)
    }

    /// Identity-first completed lookup under one SQLite read transaction.
    /// A store rotation can therefore never slip between the incarnation
    /// check and the ledger point read.
    pub(crate) fn lookup_lifecycle_operation(
        &self,
        expected_store_incarnation: &str,
        operation_id: &str,
    ) -> GaryxDbResult<LifecycleOperationLookup> {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let current_store_incarnation = read_store_incarnation_id(&tx)?;
        if current_store_incarnation != expected_store_incarnation {
            return Ok(LifecycleOperationLookup::WrongIncarnation {
                current_store_incarnation,
            });
        }
        let operation = read_lifecycle_operation(&tx, &current_store_incarnation, operation_id)?;
        tx.commit()?;
        Ok(LifecycleOperationLookup::Current(operation))
    }

    pub(crate) fn execute_lifecycle_mutation(
        &self,
        input: LifecycleMutationInput,
    ) -> GaryxDbResult<LifecycleTransactionResult> {
        self.execute_lifecycle_transaction(
            input.expected_store_incarnation,
            input.operation_id,
            input.kind,
            input.thread_id,
            input.fingerprint,
            input.endpoint_keys,
            input.enabled_channel_accounts,
            None,
        )
    }

    pub(crate) fn execute_lifecycle_decision(
        &self,
        input: LifecycleDecisionInput,
    ) -> GaryxDbResult<LifecycleTransactionResult> {
        self.execute_lifecycle_transaction(
            input.expected_store_incarnation,
            input.operation_id,
            input.kind,
            input.thread_id,
            input.fingerprint,
            Vec::new(),
            BTreeSet::new(),
            Some((input.outcome, input.detail)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_lifecycle_transaction(
        &self,
        expected_store_incarnation: String,
        operation_id: String,
        kind: LifecycleOperationKind,
        thread_id: String,
        fingerprint: String,
        endpoint_keys: Vec<String>,
        enabled_channel_accounts: BTreeSet<(String, String)>,
        forced_decision: Option<(LifecycleOperationOutcome, Value)>,
    ) -> GaryxDbResult<LifecycleTransactionResult> {
        let expected_store_incarnation =
            normalize_required("expected_store_incarnation", &expected_store_incarnation)?;
        let operation_id = normalize_required("operation_id", &operation_id)?;
        let thread_id = normalize_thread_id(&thread_id)?;
        let fingerprint = normalize_required("fingerprint", &fingerprint)?;
        #[cfg(any(test, feature = "test-seams"))]
        match kind {
            LifecycleOperationKind::Archive => {
                self.maybe_block_test_db_mutation(TestDbMutationPoint::ArchiveThreadRecord);
            }
            LifecycleOperationKind::Delete => {
                self.maybe_block_test_db_mutation(TestDbMutationPoint::DeleteThreadRecord);
                self.maybe_fail_test_db_call(TestDbFaultPoint::DeleteThreadRecord)?;
            }
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current_store_incarnation = read_store_incarnation_id(&tx)?;
        if current_store_incarnation != expected_store_incarnation {
            return Ok(LifecycleTransactionResult::WrongIncarnation {
                current_store_incarnation,
            });
        }
        if let Some(existing) =
            read_lifecycle_operation(&tx, &expected_store_incarnation, &operation_id)?
        {
            let durable_terminal = read_thread_terminal_state(&tx, &thread_id)?;
            return Ok(LifecycleTransactionResult::Existing {
                operation: existing,
                durable_terminal,
            });
        }

        let record_body = tx
            .query_row(
                "SELECT body FROM thread_records WHERE key = ?1",
                params![thread_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let terminal = read_thread_terminal_state(&tx, &thread_id)?;
        let record_value = record_body
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "thread record '{thread_id}' contains invalid JSON: {error}"
                ))
            })?;

        let matrix = lifecycle_matrix(kind, record_value.is_some(), terminal);
        let (outcome, detail, durable_terminal, mutates_terminal) = match matrix {
            LifecycleMatrixAction::ApplyChanged(next) => (
                LifecycleOperationOutcome::AppliedChanged,
                None,
                Some(next),
                true,
            ),
            LifecycleMatrixAction::ApplyNoop(next, write_terminal) => (
                LifecycleOperationOutcome::AppliedNoop,
                None,
                Some(next),
                write_terminal,
            ),
            LifecycleMatrixAction::RejectNotFound(terminal) => (
                LifecycleOperationOutcome::RejectedNotFound,
                Some(lifecycle_not_found_detail(&thread_id, terminal)),
                terminal,
                false,
            ),
            LifecycleMatrixAction::ProceedActive => {
                if let Some((outcome, detail)) = forced_decision {
                    let outcome = match outcome {
                        LifecycleOperationOutcome::RejectedConflict
                        | LifecycleOperationOutcome::RejectedNotFound => outcome,
                        _ => {
                            return Err(GaryxDbError::BadRequest(
                                "a lifecycle decision must be rejected_conflict or rejected_not_found"
                                    .to_owned(),
                            ));
                        }
                    };
                    (outcome, Some(detail), terminal, false)
                } else {
                    let has_enabled_binding = kind == LifecycleOperationKind::Delete
                        && record_value.as_ref().is_some_and(|record| {
                            bindings_from_value(record).iter().any(|binding| {
                                enabled_channel_accounts.contains(&(
                                    binding.channel.clone(),
                                    binding.account_id.clone(),
                                ))
                            })
                        });
                    if has_enabled_binding {
                        (
                            LifecycleOperationOutcome::RejectedConflict,
                            Some(json_detail(
                                "active_channel_binding",
                                "cannot delete thread with active channel bindings",
                            )),
                            None,
                            false,
                        )
                    } else {
                        let next = match kind {
                            LifecycleOperationKind::Archive => ThreadTerminalState::Archived,
                            LifecycleOperationKind::Delete => ThreadTerminalState::Deleted,
                        };
                        (
                            LifecycleOperationOutcome::AppliedChanged,
                            None,
                            Some(next),
                            true,
                        )
                    }
                }
            }
        };

        let mut detached_endpoint_keys = BTreeSet::new();
        let should_cleanup = outcome == LifecycleOperationOutcome::AppliedChanged;
        if mutates_terminal {
            tx.execute(
                "INSERT INTO archived_threads (thread_id, archived_at, kind)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(thread_id) DO UPDATE SET
                    archived_at = excluded.archived_at,
                    kind = CASE
                        WHEN archived_threads.kind = 'deleted' THEN 'deleted'
                        ELSE excluded.kind
                    END",
                params![
                    thread_id,
                    now_string(),
                    durable_terminal
                        .expect("terminal mutation must carry a durable state")
                        .as_str()
                ],
            )?;
        }

        if should_cleanup {
            let mut stmt = tx.prepare(
                "SELECT endpoint_key FROM thread_channel_endpoints
                 WHERE thread_id = ?1 ORDER BY endpoint_key ASC",
            )?;
            let rows = stmt.query_map(params![thread_id], |row| row.get::<_, String>(0))?;
            for row in rows {
                detached_endpoint_keys.insert(row?);
            }
            drop(stmt);
            for endpoint_key in endpoint_keys {
                let endpoint_key = endpoint_key.trim();
                if endpoint_key.is_empty() {
                    continue;
                }
                // Client-carried endpoint keys remain part of the canonical
                // result payload even when their persistent row is already
                // absent. The volatile invalidation is conditional on this
                // thread id, so replay cannot detach a replacement owner.
                detached_endpoint_keys.insert(endpoint_key.to_owned());
            }

            tx.execute(
                "DELETE FROM thread_records WHERE key = ?1",
                params![thread_id],
            )?;
            remove_thread_meta_projection_tx(&tx, &thread_id)?;
            remove_task_projection_tx(&tx, &thread_id)?;
            remove_recent_thread_tx(&tx, &thread_id)?;
            let removed_pin = tx.execute(
                "DELETE FROM thread_pins WHERE thread_id = ?1",
                params![thread_id],
            )? > 0;
            bump_thread_pins_revision_if_changed_tx(&tx, removed_pin)?;
            let removed_favorite = tx.execute(
                "DELETE FROM thread_favorites WHERE thread_id = ?1",
                params![thread_id],
            )? > 0;
            bump_thread_favorites_revision_if_changed_tx(&tx, removed_favorite)?;

            for endpoint_key in &detached_endpoint_keys {
                enqueue_cleanup_job(
                    &tx,
                    &thread_id,
                    CleanupOutboxStep::EndpointRuntimeInvalidate,
                    Some(&serde_json::json!({
                        "endpoint_key": endpoint_key,
                        "expected_thread_id": thread_id,
                    })),
                )?;
            }
            let provider_key = record_value.as_ref().and_then(|record| {
                record
                    .get("provider_key")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            });
            enqueue_cleanup_job(
                &tx,
                &thread_id,
                CleanupOutboxStep::RuntimeTeardown,
                Some(&serde_json::json!({ "provider_key": provider_key })),
            )?;
            enqueue_cleanup_job(&tx, &thread_id, CleanupOutboxStep::TranscriptRemove, None)?;
            enqueue_cleanup_job(&tx, &thread_id, CleanupOutboxStep::ThreadLogRemove, None)?;
            if kind == LifecycleOperationKind::Delete {
                enqueue_cleanup_job(
                    &tx,
                    &thread_id,
                    CleanupOutboxStep::PromptAttachmentsRemove,
                    None,
                )?;
            }
        }

        let result_payload = matches!(
            outcome,
            LifecycleOperationOutcome::AppliedChanged | LifecycleOperationOutcome::AppliedNoop
        )
        .then(|| {
            serde_json::json!({
                "detached_endpoint_keys": detached_endpoint_keys.into_iter().collect::<Vec<_>>(),
            })
        });
        let operation = LifecycleOperationRecord {
            store_incarnation: expected_store_incarnation,
            operation_id,
            kind,
            thread_id,
            fingerprint,
            outcome,
            result_payload,
            detail,
            completed_at: now_string(),
        };
        insert_lifecycle_operation(&tx, &operation)?;
        tx.commit()?;
        Ok(LifecycleTransactionResult::Completed {
            operation,
            durable_terminal,
        })
    }

    pub fn next_cleanup_outbox_job(&self, now: &str) -> GaryxDbResult<Option<CleanupOutboxJob>> {
        let conn = self.read_conn()?;
        let row = conn
            .query_row(
                "SELECT job.job_id, job.thread_id, job.step, job.payload,
                        job.attempt_count, job.next_attempt_at, job.created_at
                   FROM cleanup_outbox AS job
                  WHERE job.status = 'pending'
                    AND (job.next_attempt_at IS NULL OR job.next_attempt_at <= ?1)
                    AND NOT EXISTS (
                        SELECT 1 FROM cleanup_outbox AS earlier
                         WHERE earlier.thread_id = job.thread_id
                           AND earlier.job_id < job.job_id
                           AND earlier.status <> 'done'
                    )
                  ORDER BY job.job_id ASC
                  LIMIT 1",
                params![now],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?;
        row.map(cleanup_job_from_row).transpose()
    }

    pub fn mark_cleanup_outbox_done(&self, job_id: i64) -> GaryxDbResult<bool> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE cleanup_outbox
                SET status = 'done', settled_at = ?2, next_attempt_at = NULL
              WHERE job_id = ?1 AND status = 'pending'",
            params![job_id, now_string()],
        )? > 0)
    }

    pub fn retry_cleanup_outbox_job(
        &self,
        job_id: i64,
        next_attempt_at: &str,
    ) -> GaryxDbResult<bool> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE cleanup_outbox
                SET attempt_count = attempt_count + 1,
                    next_attempt_at = ?2
              WHERE job_id = ?1 AND status = 'pending'",
            params![job_id, next_attempt_at],
        )? > 0)
    }

    pub fn pending_cleanup_outbox_count(&self) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM cleanup_outbox WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?;
        usize::try_from(count).map_err(|_| {
            GaryxDbError::Configuration("cleanup outbox count exceeds usize".to_owned())
        })
    }

    pub fn prune_lifecycle_history(&self, completed_before: &str) -> GaryxDbResult<(usize, usize)> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let operations = tx.execute(
            "DELETE FROM lifecycle_operations WHERE completed_at < ?1",
            params![completed_before],
        )?;
        let jobs = tx.execute(
            "DELETE FROM cleanup_outbox
              WHERE status = 'done' AND settled_at < ?1",
            params![completed_before],
        )?;
        tx.commit()?;
        Ok((operations, jobs))
    }

    /// At boot the bridge run index is empty, so every still-queued input is
    /// necessarily orphaned.  Settle it once under the writer transaction;
    /// history GET remains a pure projection after this pass.
    pub fn recover_orphaned_pending_user_inputs(&self) -> GaryxDbResult<usize> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "SELECT key, body FROM thread_records
              WHERE key LIKE 'thread::%'
                AND instr(body, '\"pending_user_inputs\"') > 0",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        drop(stmt);

        let mut settled = 0usize;
        for (thread_id, body) in records {
            let mut value: Value = serde_json::from_str(&body).map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "thread record '{thread_id}' contains invalid JSON: {error}"
                ))
            })?;
            let Some(inputs) = value
                .get_mut("pending_user_inputs")
                .and_then(Value::as_array_mut)
            else {
                continue;
            };
            let mut changed = false;
            for input in inputs {
                let Some(input) = input.as_object_mut() else {
                    continue;
                };
                let queued = input
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("queued")
                    .eq_ignore_ascii_case("queued");
                if queued {
                    input.insert("status".to_owned(), Value::String("abandoned".to_owned()));
                    settled += 1;
                    changed = true;
                }
            }
            if changed {
                tx.execute(
                    "UPDATE thread_records SET body = ?2 WHERE key = ?1",
                    params![
                        thread_id,
                        serde_json::to_string(&value).map_err(|error| {
                            GaryxDbError::Configuration(format!(
                                "failed to serialize recovered thread '{thread_id}': {error}"
                            ))
                        })?
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(settled)
    }
}
