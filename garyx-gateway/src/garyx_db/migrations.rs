//! Legacy boot import markers and versioned one-shot migrations.

use super::*;

pub(crate) const THREAD_META_SUMMARY_MIGRATION_NAME: &str = "thread_meta_summary_v1";

pub(super) const THREAD_META_SUMMARY_MIGRATION_VERSION: i64 = 1;

pub(crate) const THREAD_META_SCHEMA_MIGRATION_NAME: &str = "thread_meta_schema_v2";

pub(super) const THREAD_META_SCHEMA_MIGRATION_VERSION: i64 = 2;

pub(crate) const RECENT_TASK_THREAD_KIND_MIGRATION_NAME: &str = "recent_task_thread_kind_v1";

pub(super) const RECENT_TASK_THREAD_KIND_MIGRATION_VERSION: i64 = 1;

pub(crate) const ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME: &str = "endpoint_holder_dedup_v1";

pub(super) const ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION: i64 = 1;

pub(crate) const THREAD_PIN_SORT_ORDER_MIGRATION_NAME: &str = "thread_pin_sort_order_v1";

pub(super) const THREAD_PIN_SORT_ORDER_MIGRATION_VERSION: i64 = 1;

pub(crate) const DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME: &str = "drop_thread_message_routes_v1";

pub(super) const DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION: i64 = 1;

pub(crate) const RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME: &str = "recent_thread_activity_seq_v1";

pub(super) const RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION: i64 = 1;

pub(crate) const RECENT_MEMBERSHIP_MIGRATION_NAME: &str = "recent_membership_v2";

pub(super) const RECENT_MEMBERSHIP_MIGRATION_VERSION: i64 = 2;

pub(crate) const CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME: &str = "canonical_exclusion_strip_v3";

pub(super) const CANONICAL_EXCLUSION_STRIP_MIGRATION_VERSION: i64 = 3;

pub(super) const LEGACY_IMPORT_GENERATION_NAME: &str = "legacy_import_generation";

pub(super) const LEGACY_IMPORT_GENERATION_VERSION: i64 = 1;

pub(super) const THREAD_META_SCHEMA_V2_COLUMNS: &[&str] = &[
    "thread_id",
    "workspace_dir",
    "thread_type",
    "thread_label",
    "agent_id",
    "provider_type",
    "created_at",
    "updated_at",
    "message_count",
    "last_user_message",
    "last_assistant_message",
    "last_message_preview",
    "recent_run_id",
    "active_run_id",
    "worktree_json",
    "last_delivery_context_json",
    "last_delivery_updated_at",
    "default_list_hidden",
    "sort_updated_at_us",
    "search_text",
    "provider_key",
    "selected_model",
    "selected_model_reasoning_effort",
    "selected_model_service_tier",
    "sdk_session_id",
    "projection_version",
    "projected_at",
];

pub(super) const THREAD_META_SCHEMA_V2_RETIRED_COLUMNS: &[&str] = &[
    "excluded_from_recent",
    "legacy_account_id",
    "legacy_channel",
    "legacy_has_account",
    "legacy_thread_binding_key",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OneShotMigrationSummary {
    pub source_row_count: usize,
    pub updated_row_count: usize,
    pub already_completed: bool,
}

pub(super) fn legacy_import_generation_row_tx(tx: &Transaction<'_>) -> GaryxDbResult<Option<i64>> {
    let row = tx
        .query_row(
            "SELECT projection_version, source_row_count
               FROM projection_states
              WHERE projection_name = ?1",
            params![LEGACY_IMPORT_GENERATION_NAME],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((version, generation)) = row else {
        return Ok(None);
    };
    if version != LEGACY_IMPORT_GENERATION_VERSION || generation < 0 {
        return Err(GaryxDbError::Configuration(format!(
            "invalid legacy import generation row: version={version}, generation={generation}"
        )));
    }
    Ok(Some(generation))
}

pub(super) fn legacy_import_compat_generation_tx(tx: &Transaction<'_>) -> GaryxDbResult<i64> {
    let imported = tx
        .query_row(
            "SELECT 1 FROM projection_states
              WHERE projection_name = ?1 AND projection_version = ?2",
            params![
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(if imported { 1 } else { 0 })
}

pub(super) fn record_projection_state_tx(
    tx: &Transaction<'_>,
    projection_name: &str,
    projection_version: i64,
    source_row_count: i64,
    based_on_import_generation: Option<i64>,
) -> GaryxDbResult<()> {
    tx.execute(
        "INSERT INTO projection_states (
            projection_name, projection_version, source_row_count, projected_at,
            based_on_import_generation
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(projection_name) DO UPDATE SET
            projection_version = excluded.projection_version,
            source_row_count = excluded.source_row_count,
            projected_at = excluded.projected_at,
            based_on_import_generation = excluded.based_on_import_generation",
        params![
            projection_name,
            projection_version,
            source_row_count,
            now_string(),
            based_on_import_generation,
        ],
    )?;
    Ok(())
}

pub(super) fn insert_thread_id(thread_ids: &mut BTreeSet<String>, value: &str) {
    let value = value.trim();
    if is_thread_key(value) {
        thread_ids.insert(value.to_owned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrozenRecentMembershipRow {
    thread_id: String,
    last_active_at: String,
}

/// S5 canonical normalization. Side-chat identity is intentionally narrow:
/// only a top-level or metadata `source == "side_chat"` hides the record.
/// Parent markers and exclusion flags alone never classify a side chat, but
/// every retired exclusion spelling is stripped from object-shaped records.
pub(super) fn normalize_recent_membership_canonical_record(data: &mut Value) -> bool {
    let side_chat = data.get("source").and_then(Value::as_str) == Some("side_chat")
        || data
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("source"))
            .and_then(Value::as_str)
            == Some("side_chat");
    let mut changed = strip_retired_recent_exclusion_fields(data);
    let Some(object) = data.as_object_mut() else {
        return changed;
    };

    if side_chat && object.get("hidden") != Some(&Value::Bool(true)) {
        object.insert("hidden".to_owned(), Value::Bool(true));
        changed = true;
    }
    changed
}

pub(super) fn recent_membership_timestamp(value: &str) -> (i64, u32) {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|timestamp| (timestamp.timestamp(), timestamp.timestamp_subsec_nanos()))
        .unwrap_or((0, 0))
}

pub(super) fn compare_recent_membership_order(
    left_timestamp: &str,
    left_thread_id: &str,
    right_timestamp: &str,
    right_thread_id: &str,
) -> CmpOrdering {
    recent_membership_timestamp(left_timestamp)
        .cmp(&recent_membership_timestamp(right_timestamp))
        .then_with(|| left_thread_id.as_bytes().cmp(right_thread_id.as_bytes()))
}

/// Cutover-only insert. Unlike the runtime upsert, this never allocates a
/// sequence: all new members intentionally share zero until step d assigns the
/// frozen H-based contiguous range with the activity indexes absent.
pub(super) fn insert_recent_membership_placeholder_tx(
    tx: &Transaction<'_>,
    draft: &RecentThreadDraft,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let thread_id = normalize_thread_id(&draft.thread_id)?;
    let thread_type = normalize_required("thread_type", &draft.thread_type)?;
    let run_state = normalize_required("run_state", &draft.run_state)?;
    let last_active_at = normalize_required("last_active_at", &draft.last_active_at)?;
    let title = draft.title.trim().to_owned();
    let workspace_dir = normalize_optional(draft.workspace_dir.as_deref());
    let provider_type = normalize_optional(draft.provider_type.as_deref());
    let agent_id = normalize_optional(draft.agent_id.as_deref());
    let last_message_preview = draft.last_message_preview.trim().to_owned();
    let recent_run_id = normalize_optional(draft.recent_run_id.as_deref());
    let active_run_id = normalize_optional(draft.active_run_id.as_deref());
    let updated_at = normalize_optional(draft.updated_at.as_deref());
    let inserted = tx.execute(
        "INSERT INTO recent_threads (
            thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
            message_count, last_message_preview, recent_run_id, active_run_id, run_state,
            updated_at, last_active_at, activity_seq, recorded_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, ?14)",
        params![
            thread_id,
            title,
            workspace_dir,
            thread_type,
            provider_type,
            agent_id,
            draft.message_count,
            last_message_preview,
            recent_run_id,
            active_run_id,
            run_state,
            updated_at,
            last_active_at,
            recorded_at,
        ],
    )?;
    if inserted != 1 {
        return Err(GaryxDbError::Configuration(
            "recent membership cutover could not insert a target row".to_owned(),
        ));
    }
    Ok(())
}

/// Enforce all six directions of canonical-visible/thread-meta/recent parity
/// before the durable marker commits. These are SQL EXCEPT checks rather than
/// counts, so equal-sized but different member sets cannot pass.
pub(super) fn assert_recent_membership_parity_tx(tx: &Transaction<'_>) -> GaryxDbResult<()> {
    let checks = [
        (
            "canonical visible minus thread_meta visible",
            "SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             EXCEPT
             SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             LIMIT 1",
        ),
        (
            "thread_meta visible minus canonical visible",
            "SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             EXCEPT
             SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             LIMIT 1",
        ),
        (
            "canonical visible minus recent",
            "SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             EXCEPT
             SELECT thread_id FROM recent_threads
             LIMIT 1",
        ),
        (
            "recent minus canonical visible",
            "SELECT thread_id FROM recent_threads
             EXCEPT
             SELECT record.key
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1 FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND COALESCE(json_type(record.body, '$.hidden'), '') <> 'true'
             LIMIT 1",
        ),
        (
            "thread_meta visible minus recent",
            "SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             EXCEPT
             SELECT thread_id FROM recent_threads
             LIMIT 1",
        ),
        (
            "recent minus thread_meta visible",
            "SELECT thread_id FROM recent_threads
             EXCEPT
             SELECT thread_id FROM thread_meta WHERE default_list_hidden = 0
             LIMIT 1",
        ),
    ];
    for (label, sql) in checks {
        if let Some(thread_id) = tx
            .query_row(sql, [], |row| row.get::<_, String>(0))
            .optional()?
        {
            return Err(GaryxDbError::Configuration(format!(
                "recent membership parity failed ({label}): {thread_id}"
            )));
        }
    }
    Ok(())
}

impl GaryxDbService {
    /// Read the import and retirement markers in one SQL query. The boot
    /// importer double-checks this pair after taking the lifecycle lock.
    pub(crate) fn legacy_import_marker_pair(&self) -> GaryxDbResult<(bool, bool)> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyMarkerPairRead)?;
        let conn = self.read_conn()?;
        let pair = conn.query_row(
            "SELECT
                 COALESCE(MAX(CASE
                     WHEN projection_name = ?1 AND projection_version = ?2 THEN 1 ELSE 0
                 END), 0),
                 COALESCE(MAX(CASE
                     WHEN projection_name = ?3 AND projection_version = ?4 THEN 1 ELSE 0
                 END), 0)
               FROM projection_states",
            params![
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
                crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
                crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_NAME,
                crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_VERSION,
            ],
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? != 0)),
        )?;
        Ok(pair)
    }

    /// Commit the frozen import marker and the next monotonic import
    /// generation together. Recovery also clears the retirement marker in
    /// this transaction, making `(0,1) -> (1,0)` atomic.
    pub(crate) fn commit_legacy_import(
        &self,
        source_row_count: usize,
        recovery: bool,
    ) -> GaryxDbResult<i64> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyImportCommit)?;
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let generation = match legacy_import_generation_row_tx(&tx)? {
            Some(generation) => generation,
            None => legacy_import_compat_generation_tx(&tx)?,
        };
        let next_generation = generation.checked_add(1).ok_or_else(|| {
            GaryxDbError::Configuration("legacy import generation overflow".to_owned())
        })?;
        record_projection_state_tx(
            &tx,
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
            source_row_count,
            None,
        )?;
        record_projection_state_tx(
            &tx,
            LEGACY_IMPORT_GENERATION_NAME,
            LEGACY_IMPORT_GENERATION_VERSION,
            next_generation,
            None,
        )?;
        if recovery {
            rotate_store_incarnation_tx(&tx)?;
            #[cfg(any(test, feature = "test-seams"))]
            self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyImportAfterIncarnationRotation)?;
            tx.execute(
                "DELETE FROM projection_states WHERE projection_name = ?1",
                params![crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_NAME],
            )?;
        }
        tx.commit()?;
        Ok(next_generation)
    }

    pub(crate) fn record_legacy_archive_retirement(&self) -> GaryxDbResult<()> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyRetirementMarkerWrite)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        record_projection_state_tx(
            &tx,
            crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_NAME,
            crate::legacy_boot_import::LEGACY_ARCHIVE_RETIREMENT_VERSION,
            0,
            None,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Current generation for cutover gating. A pre-generation database with
    /// the frozen import marker is lazily seeded to generation 1; a builder
    /// that never ran the boot importer observes generation 0 without
    /// creating a generation row.
    #[cfg(any(test, feature = "test-seams"))]
    pub(crate) fn current_legacy_import_generation(&self) -> GaryxDbResult<i64> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let generation = self.current_legacy_import_generation_tx(&tx)?;
        tx.commit()?;
        Ok(generation)
    }

    fn current_legacy_import_generation_tx(&self, tx: &Transaction<'_>) -> GaryxDbResult<i64> {
        if let Some(generation) = legacy_import_generation_row_tx(tx)? {
            return Ok(generation);
        }
        let generation = legacy_import_compat_generation_tx(tx)?;
        if generation == 1 {
            #[cfg(any(test, feature = "test-seams"))]
            self.maybe_fail_test_db_call(TestDbFaultPoint::LegacyGenerationSeedWrite)?;
            record_projection_state_tx(
                tx,
                LEGACY_IMPORT_GENERATION_NAME,
                LEGACY_IMPORT_GENERATION_VERSION,
                generation,
                None,
            )?;
        }
        Ok(generation)
    }

    fn import_generation_cutover_gate(
        &self,
        tx: &Transaction<'_>,
        migration_name: &str,
        migration_version: i64,
    ) -> GaryxDbResult<(i64, Option<i64>)> {
        let generation = self.current_legacy_import_generation_tx(tx)?;
        let completed = tx
            .query_row(
                "SELECT source_row_count,
                        COALESCE(based_on_import_generation, 1)
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![migration_name, migration_version],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let completed_source_count = completed
            .and_then(|(source_count, based_on)| (based_on == generation).then_some(source_count));
        Ok((generation, completed_source_count))
    }

    /// Run every versioned thread-data migration that must complete after
    /// the one-shot archive import and before the gateway starts serving.
    pub(crate) fn run_thread_data_startup_migrations(&self) -> GaryxDbResult<()> {
        // Destructive cleanup belongs after the boot import, not in schema
        // initialization. GaryxDbService's process-lifetime data-dir lock is
        // already held, and RuntimeAssembler runs this before listener bind.
        {
            let conn = self.conn()?;
            purge_retired_workflow_state(&conn)?;
        }
        self.drop_thread_message_routes_v1()?;
        self.migrate_thread_pin_sort_order_v1()?;
        self.migrate_recent_task_thread_kind_v1()?;
        self.migrate_thread_meta_summary_v1()?;
        self.migrate_recent_thread_activity_seq_v1()?;
        self.migrate_recent_membership_v2()?;
        self.migrate_canonical_exclusion_strip_v3()?;
        self.migrate_thread_meta_schema_v2()?;
        self.migrate_endpoint_holder_dedup_v1()?;
        Ok(())
    }

    /// Backfill the monotonic recent-thread ordering key exactly once. This
    /// marker is intentionally independent of legacy import generations:
    /// recovery imports use the normal allocator and must never reset either
    /// the marker or the meta high-water mark.
    pub(crate) fn migrate_recent_thread_activity_seq_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let completed_source_count = tx
            .query_row(
                "SELECT source_row_count
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_row_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM recent_threads", [], |row| row.get(0))?;
        let meta_activity_seq: i64 = tx.query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let existing_max: i64 = tx.query_row(
            "SELECT COALESCE(MAX(activity_seq), 0) FROM recent_threads",
            [],
            |row| row.get(0),
        )?;
        let starting_activity_seq = meta_activity_seq.max(existing_max);
        let final_activity_seq = starting_activity_seq
            .checked_add(source_row_count)
            .filter(|value| *value < MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE)
            .ok_or_else(|| {
                GaryxDbError::Configuration(
                    "recent thread activity sequence space is exhausted".to_owned(),
                )
            })?;

        // Re-running after an explicitly cleared marker remains deterministic
        // and safe even if the prior unique index is still present.
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_recent_threads_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_task_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_non_task_activity_seq;",
        )?;

        let thread_ids = {
            let mut stmt = tx.prepare(
                "SELECT thread_id
                   FROM recent_threads
                  ORDER BY last_active_at ASC, thread_id DESC",
            )?;
            stmt.query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for (offset, thread_id) in thread_ids.iter().enumerate() {
            let offset = i64::try_from(offset).unwrap_or(i64::MAX);
            let activity_seq = starting_activity_seq
                .checked_add(offset)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| {
                    GaryxDbError::Configuration(
                        "recent thread activity sequence space is exhausted".to_owned(),
                    )
                })?;
            // Pre-bind one-shot migration: this direct UPDATE is the sole
            // backfill allow-list entry in addition to pre-bind orphan/type
            // cleanup. Runtime projection writes always use the allocator.
            tx.execute(
                "UPDATE recent_threads SET activity_seq = ?1 WHERE thread_id = ?2",
                params![activity_seq, thread_id],
            )?;
        }
        tx.execute(
            "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
            params![final_activity_seq],
        )?;

        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_recent_threads_last_active;
             DROP INDEX IF EXISTS idx_recent_threads_task_last_active;
             DROP INDEX IF EXISTS idx_recent_threads_non_task_last_active;
             CREATE UNIQUE INDEX idx_recent_threads_activity_seq
                 ON recent_threads(activity_seq DESC);
             CREATE INDEX idx_recent_threads_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type = 'task';
             CREATE INDEX idx_recent_threads_non_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type <> 'task';",
        )?;
        record_projection_state_tx(
            &tx,
            RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
            RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION,
            source_row_count,
            None,
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count: thread_ids.len(),
            already_completed: false,
        })
    }

    /// Make hidden the sole recent-membership predicate exactly once per
    /// legacy-import generation. The cutover deliberately bypasses the normal
    /// recent allocator: it freezes the old order, repairs canonical/meta
    /// state, rebuilds the exact visible-live member set, and assigns one new
    /// contiguous sequence range in the same transaction as its marker.
    pub(crate) fn migrate_recent_membership_v2(&self) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            RECENT_MEMBERSHIP_MIGRATION_NAME,
            RECENT_MEMBERSHIP_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        // Registration contract: summary derivation and the first monotonic
        // activity sequence must both precede this membership rewrite. The
        // summary marker is generation-aware, so recovery cannot run S5 over
        // a stale generation's thread_meta rows.
        let summary_prerequisite: bool = tx
            .query_row(
                "SELECT 1
                   FROM projection_states
                  WHERE projection_name = ?1
                    AND projection_version = ?2
                    AND COALESCE(based_on_import_generation, 1) = ?3",
                params![
                    THREAD_META_SUMMARY_MIGRATION_NAME,
                    THREAD_META_SUMMARY_MIGRATION_VERSION,
                    import_generation,
                ],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        let activity_prerequisite: bool = tx
            .query_row(
                "SELECT 1
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
                    RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION,
                ],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !summary_prerequisite || !activity_prerequisite {
            return Err(GaryxDbError::Configuration(
                "recent_membership_v2 must run after thread_meta_summary_v1 and recent_thread_activity_seq_v1"
                    .to_owned(),
            ));
        }

        // a. Freeze the pre-cutover membership, its exact ascending sequence
        // order, and H before any insertion. Indexes go immediately because
        // step c intentionally gives every new member the same placeholder.
        let frozen_recent = {
            let mut stmt = tx.prepare(
                "SELECT thread_id, last_active_at
                   FROM recent_threads
                  ORDER BY activity_seq ASC, thread_id ASC",
            )?;
            stmt.query_map([], |row| {
                Ok(FrozenRecentMembershipRow {
                    thread_id: row.get(0)?,
                    last_active_at: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let pre_cutover_recent_ids = frozen_recent
            .iter()
            .map(|row| row.thread_id.clone())
            .collect::<HashSet<_>>();
        let meta_high_water: i64 = tx.query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let row_high_water: i64 = tx.query_row(
            "SELECT COALESCE(MAX(activity_seq), 0) FROM recent_threads",
            [],
            |row| row.get(0),
        )?;
        let frozen_high_water = meta_high_water.max(row_high_water);
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_recent_threads_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_task_activity_seq;
             DROP INDEX IF EXISTS idx_recent_threads_non_task_activity_seq;",
        )?;

        // b. Normalize every live canonical record: hide side chats and strip
        // all four retired recent-exclusion paths. Rederive every thread_meta
        // row even when the body is byte-identical. Do not touch
        // recent_threads in this phase.
        let canonical_rows = {
            let mut stmt = tx.prepare(
                "SELECT record.key, record.body
                   FROM thread_records AS record
                  WHERE substr(record.key, 1, 8) = 'thread::'
                    AND NOT EXISTS (
                        SELECT 1
                          FROM archived_threads AS archived
                         WHERE archived.thread_id = record.key
                    )
                  ORDER BY record.key ASC",
            )?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let source_row_count = i64::try_from(canonical_rows.len()).unwrap_or(i64::MAX);
        let projected_at = now_string();
        let mut canonical_updated_count = 0usize;
        let mut target_drafts = Vec::with_capacity(canonical_rows.len());
        for (thread_id, body) in canonical_rows {
            let mut data: Value = serde_json::from_str(&body).map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "recent membership cutover could not decode {thread_id}: {error}"
                ))
            })?;
            let canonical_changed = normalize_recent_membership_canonical_record(&mut data);
            if canonical_changed {
                let normalized_body = serde_json::to_string(&data).map_err(|error| {
                    GaryxDbError::Configuration(format!(
                        "recent membership cutover could not encode {thread_id}: {error}"
                    ))
                })?;
                canonical_updated_count += tx.execute(
                    "UPDATE thread_records SET body = ?1 WHERE key = ?2",
                    params![normalized_body, thread_id],
                )?;
            }

            let projection = crate::thread_meta_projection::
                thread_meta_projection_from_thread_data_with_active_run(&thread_id, &data, None)
                .ok_or_else(|| {
                    GaryxDbError::Configuration(format!(
                        "recent membership cutover rejected canonical id {thread_id}"
                    ))
                })?;
            upsert_thread_meta(&tx, &projection.thread_meta, &projected_at)?;
            if let Some(draft) = crate::recent_thread_projection::
                recent_thread_draft_from_thread_data_with_active_run(&thread_id, &data, None)
            {
                target_drafts.push(draft);
            }
        }
        let target_ids = target_drafts
            .iter()
            .map(|draft| draft.thread_id.clone())
            .collect::<HashSet<_>>();
        let target_count = i64::try_from(target_drafts.len()).unwrap_or(i64::MAX);
        let final_high_water = frozen_high_water
            .checked_add(target_count)
            .filter(|value| *value < MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE)
            .ok_or_else(|| {
                GaryxDbError::Configuration(
                    "recent thread activity sequence space is exhausted".to_owned(),
                )
            })?;

        // c. Exact target membership: add target-pre with a shared placeholder
        // and remove every pre-target orphan/hidden row. The indexes were
        // already dropped in step a, so multiple placeholder zeroes are valid.
        let mut new_drafts = target_drafts
            .iter()
            .filter(|draft| !pre_cutover_recent_ids.contains(&draft.thread_id))
            .cloned()
            .collect::<Vec<_>>();
        for draft in &new_drafts {
            insert_recent_membership_placeholder_tx(&tx, draft, &projected_at)?;
        }
        let mut removed_recent_count = 0usize;
        for row in &frozen_recent {
            if !target_ids.contains(&row.thread_id) {
                removed_recent_count += tx.execute(
                    "DELETE FROM recent_threads WHERE thread_id = ?1",
                    params![row.thread_id],
                )?;
            }
        }

        // d. Existing members retain their exact frozen relative order.
        // New members are bucketed by the count of retained rows whose
        // timestamp/id key is smaller; the retained rows are never sorted by
        // timestamp, which avoids cycles when old timestamps are inverted.
        let retained_existing_order = frozen_recent
            .iter()
            .filter(|row| target_ids.contains(&row.thread_id))
            .cloned()
            .collect::<Vec<_>>();
        new_drafts.sort_by(|left, right| {
            compare_recent_membership_order(
                &left.last_active_at,
                &left.thread_id,
                &right.last_active_at,
                &right.thread_id,
            )
        });
        let mut insertion_buckets = vec![Vec::<String>::new(); retained_existing_order.len() + 1];
        for draft in &new_drafts {
            let insertion_index = retained_existing_order
                .iter()
                .filter(|existing| {
                    compare_recent_membership_order(
                        &existing.last_active_at,
                        &existing.thread_id,
                        &draft.last_active_at,
                        &draft.thread_id,
                    ) == CmpOrdering::Less
                })
                .count();
            insertion_buckets[insertion_index].push(draft.thread_id.clone());
        }
        let mut final_order = Vec::with_capacity(target_drafts.len());
        for index in 0..=retained_existing_order.len() {
            final_order.append(&mut insertion_buckets[index]);
            if let Some(existing) = retained_existing_order.get(index) {
                final_order.push(existing.thread_id.clone());
            }
        }
        if final_order.len() != target_drafts.len() {
            return Err(GaryxDbError::Configuration(
                "recent membership cutover produced an incomplete order".to_owned(),
            ));
        }
        for (offset, thread_id) in final_order.iter().enumerate() {
            let offset = i64::try_from(offset).unwrap_or(i64::MAX);
            let activity_seq = frozen_high_water
                .checked_add(offset)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| {
                    GaryxDbError::Configuration(
                        "recent thread activity sequence space is exhausted".to_owned(),
                    )
                })?;
            let updated = tx.execute(
                "UPDATE recent_threads SET activity_seq = ?1 WHERE thread_id = ?2",
                params![activity_seq, thread_id],
            )?;
            if updated != 1 {
                return Err(GaryxDbError::Configuration(format!(
                    "recent membership cutover lost target row {thread_id}"
                )));
            }
        }

        // e. Publish the new high-water mark, recreate all three ordering
        // indexes, and verify the row/max invariants before the marker lands.
        let updated_meta = tx.execute(
            "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
            params![final_high_water],
        )?;
        if updated_meta != 1 {
            return Err(GaryxDbError::Configuration(
                "recent_threads_meta singleton is missing".to_owned(),
            ));
        }
        tx.execute_batch(
            "CREATE UNIQUE INDEX idx_recent_threads_activity_seq
                 ON recent_threads(activity_seq DESC);
             CREATE INDEX idx_recent_threads_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type = 'task';
             CREATE INDEX idx_recent_threads_non_task_activity_seq
                 ON recent_threads(activity_seq DESC)
                 WHERE thread_type <> 'task';",
        )?;
        let (actual_count, actual_max): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(MAX(activity_seq), 0) FROM recent_threads",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if actual_count != target_count
            || (actual_count > 0 && actual_max != final_high_water)
            || (actual_count == 0 && final_high_water != frozen_high_water)
        {
            return Err(GaryxDbError::Configuration(
                "recent membership cutover high-water/member count mismatch".to_owned(),
            ));
        }
        assert_recent_membership_parity_tx(&tx)?;

        // f. Data and generation-aware marker commit atomically.
        record_projection_state_tx(
            &tx,
            RECENT_MEMBERSHIP_MIGRATION_NAME,
            RECENT_MEMBERSHIP_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count: canonical_updated_count
                .saturating_add(new_drafts.len())
                .saturating_add(removed_recent_count)
                .saturating_add(final_order.len()),
            already_completed: false,
        })
    }

    /// Repair canonical records whose generation already committed the
    /// historical recent_membership_v2 marker after its strip step was
    /// accidentally removed. The independent generation-aware marker is
    /// required because rewriting a committed v2 marker would strand every
    /// database that already skipped it.
    pub(crate) fn migrate_canonical_exclusion_strip_v3(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME,
            CANONICAL_EXCLUSION_STRIP_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let membership_prerequisite: bool = tx
            .query_row(
                "SELECT 1
                   FROM projection_states
                  WHERE projection_name = ?1
                    AND projection_version = ?2
                    AND COALESCE(based_on_import_generation, 1) = ?3",
                params![
                    RECENT_MEMBERSHIP_MIGRATION_NAME,
                    RECENT_MEMBERSHIP_MIGRATION_VERSION,
                    import_generation,
                ],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !membership_prerequisite {
            return Err(GaryxDbError::Configuration(
                "canonical_exclusion_strip_v3 must run after recent_membership_v2".to_owned(),
            ));
        }

        let canonical_rows = {
            let mut stmt = tx.prepare(
                "SELECT record.key, record.body
                   FROM thread_records AS record
                  WHERE substr(record.key, 1, 8) = 'thread::'
                    AND NOT EXISTS (
                        SELECT 1
                          FROM archived_threads AS archived
                         WHERE archived.thread_id = record.key
                    )
                  ORDER BY record.key ASC",
            )?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let source_row_count = i64::try_from(canonical_rows.len()).unwrap_or(i64::MAX);
        let mut updated_row_count = 0usize;
        for (thread_id, body) in canonical_rows {
            let mut data: Value = serde_json::from_str(&body).map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "canonical exclusion strip could not decode {thread_id}: {error}"
                ))
            })?;
            if !strip_retired_recent_exclusion_fields(&mut data) {
                continue;
            }
            let normalized_body = serde_json::to_string(&data).map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "canonical exclusion strip could not encode {thread_id}: {error}"
                ))
            })?;
            let updated = tx.execute(
                "UPDATE thread_records SET body = ?1 WHERE key = ?2",
                params![normalized_body, thread_id],
            )?;
            if updated != 1 {
                return Err(GaryxDbError::Configuration(format!(
                    "canonical exclusion strip lost target row {thread_id}"
                )));
            }
            updated_row_count = updated_row_count.saturating_add(updated);
        }

        let residual_count: i64 = tx.query_row(
            "SELECT COUNT(*)
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND NOT EXISTS (
                    SELECT 1
                      FROM archived_threads AS archived
                     WHERE archived.thread_id = record.key
                )
                AND (
                    json_type(record.body, '$.exclude_from_recent') IS NOT NULL
                    OR json_type(record.body, '$.excludeFromRecent') IS NOT NULL
                    OR json_type(record.body, '$.metadata.exclude_from_recent') IS NOT NULL
                    OR json_type(record.body, '$.metadata.excludeFromRecent') IS NOT NULL
                )",
            [],
            |row| row.get(0),
        )?;
        if residual_count != 0 {
            return Err(GaryxDbError::Configuration(format!(
                "canonical exclusion strip left {residual_count} live records with retired fields"
            )));
        }

        record_projection_state_tx(
            &tx,
            CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME,
            CANONICAL_EXCLUSION_STRIP_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    pub(crate) fn drop_thread_message_routes_v1(&self) -> GaryxDbResult<OneShotMigrationSummary> {
        self.drop_thread_message_routes_v1_inner(|_| Ok(()))
    }

    pub(super) fn drop_thread_message_routes_v1_inner<F>(
        &self,
        after_drop: F,
    ) -> GaryxDbResult<OneShotMigrationSummary>
    where
        F: FnOnce(&Transaction<'_>) -> GaryxDbResult<()>,
    {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let completed_source_count = tx
            .query_row(
                "SELECT source_row_count
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
                    DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let table_exists = tx
            .query_row(
                "SELECT 1 FROM sqlite_master
                  WHERE type = 'table' AND name = 'thread_message_routes'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let source_row_count = if table_exists { 1 } else { 0 };

        tx.execute("DROP TABLE IF EXISTS thread_message_routes", [])?;
        after_drop(&tx)?;
        record_projection_state_tx(
            &tx,
            DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
            DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION,
            source_row_count,
            None,
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count: usize::from(table_exists),
            already_completed: false,
        })
    }

    pub(crate) fn migrate_thread_pin_sort_order_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        self.migrate_thread_pin_sort_order_v1_inner(|_| Ok(()))
    }

    pub(super) fn migrate_thread_pin_sort_order_v1_inner<F>(
        &self,
        after_backfill: F,
    ) -> GaryxDbResult<OneShotMigrationSummary>
    where
        F: FnOnce(&Transaction<'_>) -> GaryxDbResult<()>,
    {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let completed_source_count = tx
            .query_row(
                "SELECT source_row_count
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
                    THREAD_PIN_SORT_ORDER_MIGRATION_VERSION
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_row_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM thread_pins", [], |row| row.get(0))?;
        let updated_row_count = tx.execute(
            "WITH ranked AS (
                 SELECT thread_id,
                        ROW_NUMBER() OVER (
                            ORDER BY pinned_at DESC, thread_id ASC
                        ) - 1 AS next_sort_order
                   FROM thread_pins
             )
             UPDATE thread_pins
                SET sort_order = (
                    SELECT next_sort_order
                      FROM ranked
                     WHERE ranked.thread_id = thread_pins.thread_id
                )",
            [],
        )?;

        after_backfill(&tx)?;

        tx.execute(
            "INSERT INTO projection_states (
                projection_name, projection_version, source_row_count, projected_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(projection_name) DO UPDATE SET
                projection_version = excluded.projection_version,
                source_row_count = excluded.source_row_count,
                projected_at = excluded.projected_at",
            params![
                THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
                THREAD_PIN_SORT_ORDER_MIGRATION_VERSION,
                source_row_count,
                now_string(),
            ],
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    /// Establish the canonical invariant that one endpoint appears on at
    /// most one thread record. Winner selection exactly follows the existing
    /// preference order: parsed timestamp, raw timestamp, then thread id.
    /// Canonical JSON and the endpoint projection are rewritten in one
    /// transaction so no ghost holder can survive the cutover to point reads.
    pub(crate) fn migrate_endpoint_holder_dedup_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        tx.execute_batch(
            "DROP TABLE IF EXISTS temp.endpoint_holder_dedup_rows;
             DROP TABLE IF EXISTS temp.endpoint_holder_dedup_winners;
             CREATE TEMP TABLE endpoint_holder_dedup_rows (
                 thread_id TEXT NOT NULL,
                 binding_index INTEGER NOT NULL,
                 endpoint_key TEXT NOT NULL,
                 channel TEXT NOT NULL,
                 account_id TEXT NOT NULL,
                 binding_key TEXT NOT NULL,
                 chat_id TEXT NOT NULL,
                 delivery_target_type TEXT NOT NULL,
                 delivery_target_id TEXT NOT NULL,
                 display_label TEXT NOT NULL,
                 last_inbound_at TEXT,
                 last_delivery_at TEXT,
                 thread_label TEXT,
                 workspace_dir TEXT,
                 thread_updated_at TEXT NOT NULL
             ) STRICT;
             CREATE TEMP TABLE endpoint_holder_dedup_winners (
                 endpoint_key TEXT PRIMARY KEY,
                 thread_id TEXT NOT NULL
             ) STRICT;",
        )?;
        tx.execute(
            "INSERT INTO endpoint_holder_dedup_rows (
                 thread_id, binding_index, endpoint_key, channel, account_id,
                 binding_key, chat_id, delivery_target_type, delivery_target_id,
                 display_label, last_inbound_at, last_delivery_at, thread_label,
                 workspace_dir, thread_updated_at
             )
             SELECT record.key,
                    CAST(binding.key AS INTEGER),
                    COALESCE(json_extract(binding.value, '$.channel'), '') || '::' ||
                    COALESCE(json_extract(binding.value, '$.account_id'), '') || '::' ||
                    trim(CASE
                        WHEN json_type(binding.value, '$.binding_key') = 'text'
                            THEN json_extract(binding.value, '$.binding_key')
                        WHEN json_type(binding.value, '$.thread_scope') = 'text'
                            THEN json_extract(binding.value, '$.thread_scope')
                        WHEN json_type(binding.value, '$.peer_id') = 'text'
                            THEN json_extract(binding.value, '$.peer_id')
                        ELSE ''
                    END),
                    COALESCE(json_extract(binding.value, '$.channel'), ''),
                    COALESCE(json_extract(binding.value, '$.account_id'), ''),
                    trim(CASE
                        WHEN json_type(binding.value, '$.binding_key') = 'text'
                            THEN json_extract(binding.value, '$.binding_key')
                        WHEN json_type(binding.value, '$.thread_scope') = 'text'
                            THEN json_extract(binding.value, '$.thread_scope')
                        WHEN json_type(binding.value, '$.peer_id') = 'text'
                            THEN json_extract(binding.value, '$.peer_id')
                        ELSE ''
                    END),
                    trim(COALESCE(json_extract(binding.value, '$.chat_id'), '')),
                    trim(COALESCE(json_extract(binding.value, '$.delivery_target_type'), '')),
                    trim(COALESCE(json_extract(binding.value, '$.delivery_target_id'), '')),
                    trim(COALESCE(json_extract(binding.value, '$.display_label'), '')),
                    CASE WHEN json_type(binding.value, '$.last_inbound_at') = 'text'
                         THEN json_extract(binding.value, '$.last_inbound_at') END,
                    CASE WHEN json_type(binding.value, '$.last_delivery_at') = 'text'
                         THEN json_extract(binding.value, '$.last_delivery_at') END,
                    CASE WHEN json_type(record.body, '$.label') = 'text'
                         THEN json_extract(record.body, '$.label') END,
                    CASE WHEN json_type(record.body, '$.workspace_dir') = 'text'
                         THEN json_extract(record.body, '$.workspace_dir') END,
                    CASE WHEN json_type(record.body, '$.updated_at') = 'text'
                         THEN json_extract(record.body, '$.updated_at') ELSE '' END
               FROM thread_records AS record,
                    json_each(json_extract(record.body, '$.channel_bindings')) AS binding
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND json_type(binding.value) = 'object'
                AND (json_type(binding.value, '$.channel') IS NULL OR
                     json_type(binding.value, '$.channel') = 'text')
                AND (json_type(binding.value, '$.account_id') IS NULL OR
                     json_type(binding.value, '$.account_id') = 'text')
                AND (json_type(binding.value, '$.binding_key') IS NULL OR
                     json_type(binding.value, '$.binding_key') = 'text')
                AND (json_type(binding.value, '$.chat_id') IS NULL OR
                     json_type(binding.value, '$.chat_id') = 'text')
                AND (json_type(binding.value, '$.delivery_target_type') IS NULL OR
                     json_type(binding.value, '$.delivery_target_type') = 'text')
                AND (json_type(binding.value, '$.delivery_target_id') IS NULL OR
                     json_type(binding.value, '$.delivery_target_id') = 'text')
                AND (json_type(binding.value, '$.display_label') IS NULL OR
                     json_type(binding.value, '$.display_label') = 'text')
                AND (json_type(binding.value, '$.last_inbound_at') IS NULL OR
                     json_type(binding.value, '$.last_inbound_at') = 'text')
                AND (json_type(binding.value, '$.last_delivery_at') IS NULL OR
                     json_type(binding.value, '$.last_delivery_at') = 'text')",
            [],
        )?;
        tx.execute(
            "INSERT INTO endpoint_holder_dedup_winners (endpoint_key, thread_id)
             SELECT endpoint_key, thread_id
               FROM (
                   SELECT endpoint_key,
                          thread_id,
                          ROW_NUMBER() OVER (
                              PARTITION BY endpoint_key
                              ORDER BY
                                  CASE
                                      WHEN thread_updated_at GLOB
                                           '????-??-??T??:??:??*'
                                           AND julianday(thread_updated_at) IS NOT NULL
                                      THEN 1 ELSE 0
                                  END DESC,
                                  CASE
                                      WHEN thread_updated_at GLOB
                                           '????-??-??T??:??:??*'
                                      THEN julianday(thread_updated_at)
                                  END DESC,
                                  thread_updated_at DESC,
                                  thread_id DESC
                          ) AS preference_rank
                     FROM endpoint_holder_dedup_rows
               )
              WHERE preference_rank = 1",
            [],
        )?;
        let source_row_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM endpoint_holder_dedup_rows",
            [],
            |row| row.get(0),
        )?;

        let updated_row_count = tx.execute(
            "UPDATE thread_records
                SET body = json_set(
                    body,
                    '$.channel_bindings',
                    json(COALESCE((
                        SELECT json_group_array(json(binding.value))
                          FROM json_each(
                              json_extract(thread_records.body, '$.channel_bindings')
                          ) AS binding
                         WHERE NOT EXISTS (
                             SELECT 1
                               FROM endpoint_holder_dedup_rows AS holder
                               JOIN endpoint_holder_dedup_winners AS winner
                                 ON winner.endpoint_key = holder.endpoint_key
                              WHERE holder.thread_id = thread_records.key
                                AND holder.binding_index = CAST(binding.key AS INTEGER)
                                AND winner.thread_id <> holder.thread_id
                         )
                    ), '[]'))
                )
              WHERE key IN (
                  SELECT DISTINCT holder.thread_id
                    FROM endpoint_holder_dedup_rows AS holder
                    JOIN endpoint_holder_dedup_winners AS winner
                      ON winner.endpoint_key = holder.endpoint_key
                   WHERE winner.thread_id <> holder.thread_id
              )",
            [],
        )?;

        tx.execute("DELETE FROM thread_channel_endpoints", [])?;
        tx.execute(
            "INSERT OR REPLACE INTO thread_channel_endpoints (
                 endpoint_key, channel, account_id, binding_key, chat_id,
                 delivery_target_type, delivery_target_id, display_label,
                 thread_id, thread_label, workspace_dir, thread_updated_at,
                 last_inbound_at, last_delivery_at, projected_at
             )
             SELECT holder.endpoint_key,
                    holder.channel,
                    holder.account_id,
                    holder.binding_key,
                    holder.chat_id,
                    CASE
                        WHEN holder.delivery_target_id <> '' THEN
                            CASE WHEN holder.delivery_target_type = 'open_id'
                                 THEN 'open_id' ELSE 'chat_id' END
                        WHEN holder.channel = 'feishu'
                             AND holder.chat_id <> ''
                             AND holder.chat_id = holder.binding_key
                             AND holder.chat_id LIKE 'ou_%'
                        THEN 'open_id'
                        ELSE 'chat_id'
                    END,
                    CASE
                        WHEN holder.delivery_target_id <> ''
                        THEN holder.delivery_target_id
                        WHEN holder.channel = 'feishu'
                             AND holder.chat_id <> ''
                             AND holder.chat_id = holder.binding_key
                             AND holder.chat_id LIKE 'ou_%'
                        THEN CASE WHEN holder.binding_key <> ''
                                  THEN holder.binding_key ELSE holder.chat_id END
                        ELSE CASE WHEN holder.chat_id <> ''
                                  THEN holder.chat_id ELSE holder.binding_key END
                    END,
                    holder.display_label,
                    holder.thread_id,
                    NULLIF(trim(holder.thread_label), ''),
                    NULLIF(trim(holder.workspace_dir), ''),
                    NULLIF(trim(holder.thread_updated_at), ''),
                    NULLIF(trim(holder.last_inbound_at), ''),
                    NULLIF(trim(holder.last_delivery_at), ''),
                    ?1
               FROM endpoint_holder_dedup_rows AS holder
               JOIN endpoint_holder_dedup_winners AS winner
                 ON winner.endpoint_key = holder.endpoint_key
                AND winner.thread_id = holder.thread_id
              ORDER BY holder.thread_id ASC, holder.binding_index ASC",
            params![now_string()],
        )?;
        record_projection_state_tx(
            &tx,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.execute_batch(
            "DROP TABLE endpoint_holder_dedup_winners;
             DROP TABLE endpoint_holder_dedup_rows;",
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    /// Persist task identity on legacy backing threads. The migration is a
    /// one-shot, set-based transaction: canonical bodies and both type
    /// projections move together, while activity timestamps and titles stay
    /// untouched.
    pub(crate) fn migrate_recent_task_thread_kind_v1(
        &self,
    ) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
            RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_row_count: i64 = tx.query_row(
            "SELECT COUNT(*)
               FROM thread_records AS record
              WHERE substr(record.key, 1, 8) = 'thread::'
                AND (
                    json_extract(record.body, '$.thread_kind') = 'task'
                    OR json_extract(record.body, '$.thread_title_source') = 'task'
                    OR EXISTS (
                        SELECT 1
                          FROM task_projection AS task
                         WHERE task.thread_id = record.key
                    )
                )",
            [],
            |row| row.get(0),
        )?;

        let updated_row_count = tx.execute(
            "UPDATE thread_records
                SET body = json_set(body, '$.thread_kind', 'task')
              WHERE substr(key, 1, 8) = 'thread::'
                AND (
                    json_extract(body, '$.thread_kind') = 'task'
                    OR json_extract(body, '$.thread_title_source') = 'task'
                    OR EXISTS (
                        SELECT 1
                          FROM task_projection AS task
                         WHERE task.thread_id = thread_records.key
                    )
                )
                AND COALESCE(json_extract(body, '$.thread_kind'), '') <> 'task'",
            [],
        )?;
        // Pre-bind one-shot projection correction. Changing the persisted
        // thread kind is not user activity, so it intentionally preserves
        // activity_seq rather than moving the row to the head.
        tx.execute(
            "UPDATE recent_threads
                SET thread_type = 'task'
              WHERE thread_id IN (
                    SELECT key
                      FROM thread_records
                     WHERE substr(key, 1, 8) = 'thread::'
                       AND json_extract(body, '$.thread_kind') = 'task'
                )
                AND thread_type <> 'task'",
            [],
        )?;
        tx.execute(
            "UPDATE thread_meta
                SET thread_type = 'task'
              WHERE thread_id IN (
                    SELECT key
                      FROM thread_records
                     WHERE substr(key, 1, 8) = 'thread::'
                       AND json_extract(body, '$.thread_kind') = 'task'
                )
                AND thread_type <> 'task'",
            [],
        )?;
        record_projection_state_tx(
            &tx,
            RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
            RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    /// Backfill the retained list-summary columns from canonical thread records
    /// exactly once per legacy-import generation. Normal writes derive the
    /// same fields before entering the record/projection transaction.
    pub(crate) fn migrate_thread_meta_summary_v1(&self) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (import_generation, completed_source_count) = self.import_generation_cutover_gate(
            &tx,
            THREAD_META_SUMMARY_MIGRATION_NAME,
            THREAD_META_SUMMARY_MIGRATION_VERSION,
        )?;
        if let Some(source_row_count) = completed_source_count {
            tx.commit()?;
            return Ok(OneShotMigrationSummary {
                source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                updated_row_count: 0,
                already_completed: true,
            });
        }

        let source_rows = {
            let mut stmt = tx.prepare(
                "SELECT meta.thread_id, record.body
                   FROM thread_meta AS meta
                   LEFT JOIN thread_records AS record ON record.key = meta.thread_id
                  ORDER BY meta.thread_id ASC",
            )?;
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        let source_row_count = i64::try_from(source_rows.len()).unwrap_or(i64::MAX);

        let mut updated_row_count = 0usize;
        for (thread_id, body) in source_rows {
            let body = body.ok_or_else(|| {
                GaryxDbError::Configuration(
                    "thread_meta summary cutover found a projection without a canonical record"
                        .to_owned(),
                )
            })?;
            let data: Value = serde_json::from_str(&body).map_err(|error| {
                GaryxDbError::Configuration(format!(
                    "thread_meta summary cutover could not decode {thread_id}: {error}"
                ))
            })?;
            let projection = crate::thread_meta_projection::
                thread_meta_projection_from_thread_data_with_active_run(&thread_id, &data, None)
                .ok_or_else(|| {
                    GaryxDbError::Configuration(format!(
                        "thread_meta summary cutover rejected canonical id {thread_id}"
                    ))
                })?;
            updated_row_count += tx.execute(
                "UPDATE thread_meta
                    SET sort_updated_at_us = ?1,
                        search_text = ?2
                  WHERE thread_id = ?3",
                params![
                    projection.thread_meta.sort_updated_at_us,
                    projection.thread_meta.search_text,
                    thread_id,
                ],
            )?;
        }
        record_projection_state_tx(
            &tx,
            THREAD_META_SUMMARY_MIGRATION_NAME,
            THREAD_META_SUMMARY_MIGRATION_VERSION,
            source_row_count,
            Some(import_generation),
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    /// Advance the physical thread-summary projection schema without
    /// reusing the generation-aware v1 backfill marker. Depending on their
    /// upgrade path, legacy databases have the retained columns in an older
    /// order and may have one extra column. Rebuilding from the explicit v2
    /// set canonicalizes every known legacy shape while preserving every live
    /// value. Unknown columns still stop the migration instead of being
    /// silently discarded.
    pub(crate) fn migrate_thread_meta_schema_v2(&self) -> GaryxDbResult<OneShotMigrationSummary> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let completed_source_count = tx
            .query_row(
                "SELECT source_row_count
                   FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![
                    THREAD_META_SCHEMA_MIGRATION_NAME,
                    THREAD_META_SCHEMA_MIGRATION_VERSION
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let actual_columns = thread_meta_column_names(&tx)?;
        let expected_columns = THREAD_META_SCHEMA_V2_COLUMNS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        if actual_columns == expected_columns {
            if let Some(source_row_count) = completed_source_count {
                tx.commit()?;
                return Ok(OneShotMigrationSummary {
                    source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
                    updated_row_count: 0,
                    already_completed: true,
                });
            }
        } else {
            let actual = actual_columns.iter().collect::<BTreeSet<_>>();
            let expected = expected_columns.iter().collect::<BTreeSet<_>>();
            let missing = expected
                .difference(&actual)
                .copied()
                .cloned()
                .collect::<Vec<_>>();
            let extra = actual
                .difference(&expected)
                .copied()
                .cloned()
                .collect::<Vec<_>>();
            let has_unknown_extra = extra
                .iter()
                .any(|name| !THREAD_META_SCHEMA_V2_RETIRED_COLUMNS.contains(&name.as_str()));
            if !missing.is_empty() || has_unknown_extra {
                return Err(GaryxDbError::Configuration(format!(
                    "thread_meta schema v2 found an unsupported legacy shape; missing={missing:?}, extra={extra:?}"
                )));
            }
        }

        let source_row_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM thread_meta", [], |row| row.get(0))?;
        let updated_row_count = if actual_columns == expected_columns {
            tx.execute(
                "UPDATE thread_meta
                    SET projection_version = ?1
                  WHERE projection_version != ?1",
                params![CURRENT_THREAD_META_PROJECTION_VERSION],
            )?
        } else {
            tx.execute_batch(
                "DROP TABLE IF EXISTS thread_meta_schema_v2;
                 CREATE TABLE thread_meta_schema_v2 (
                    thread_id TEXT PRIMARY KEY,
                    workspace_dir TEXT,
                    thread_type TEXT NOT NULL DEFAULT 'chat',
                    thread_label TEXT,
                    agent_id TEXT,
                    provider_type TEXT,
                    created_at TEXT,
                    updated_at TEXT,
                    message_count INTEGER NOT NULL DEFAULT 0,
                    last_user_message TEXT,
                    last_assistant_message TEXT,
                    last_message_preview TEXT,
                    recent_run_id TEXT,
                    active_run_id TEXT,
                    worktree_json TEXT,
                    last_delivery_context_json TEXT,
                    last_delivery_updated_at TEXT,
                    default_list_hidden INTEGER NOT NULL DEFAULT 0,
                    sort_updated_at_us INTEGER NOT NULL DEFAULT 0,
                    search_text TEXT NOT NULL DEFAULT '',
                    provider_key TEXT,
                    selected_model TEXT,
                    selected_model_reasoning_effort TEXT,
                    selected_model_service_tier TEXT,
                    sdk_session_id TEXT,
                    projection_version INTEGER NOT NULL DEFAULT 6,
                    projected_at TEXT NOT NULL
                 ) STRICT;",
            )?;
            tx.execute(
                "INSERT INTO thread_meta_schema_v2 (
                    thread_id, workspace_dir, thread_type, thread_label, agent_id,
                    provider_type, created_at, updated_at, message_count,
                    last_user_message, last_assistant_message, last_message_preview,
                    recent_run_id, active_run_id, worktree_json,
                    last_delivery_context_json, last_delivery_updated_at,
                    default_list_hidden, sort_updated_at_us, search_text,
                    provider_key, selected_model, selected_model_reasoning_effort,
                    selected_model_service_tier, sdk_session_id, projection_version,
                    projected_at
                 )
                 SELECT thread_id, workspace_dir, thread_type, thread_label, agent_id,
                        provider_type, created_at, updated_at, message_count,
                        last_user_message, last_assistant_message, last_message_preview,
                        recent_run_id, active_run_id, worktree_json,
                        last_delivery_context_json, last_delivery_updated_at,
                        default_list_hidden, sort_updated_at_us, search_text,
                        provider_key, selected_model, selected_model_reasoning_effort,
                        selected_model_service_tier, sdk_session_id, ?1, projected_at
                   FROM thread_meta",
                params![CURRENT_THREAD_META_PROJECTION_VERSION],
            )?;
            tx.execute_batch(
                "DROP TABLE thread_meta;
                 ALTER TABLE thread_meta_schema_v2 RENAME TO thread_meta;",
            )?;
            ensure_thread_meta_indexes(&tx)?;
            usize::try_from(source_row_count).unwrap_or(usize::MAX)
        };
        record_projection_state_tx(
            &tx,
            THREAD_META_SCHEMA_MIGRATION_NAME,
            THREAD_META_SCHEMA_MIGRATION_VERSION,
            source_row_count,
            None,
        )?;
        tx.commit()?;

        Ok(OneShotMigrationSummary {
            source_row_count: usize::try_from(source_row_count).unwrap_or(usize::MAX),
            updated_row_count,
            already_completed: false,
        })
    }

    pub fn projection_state_matches(
        &self,
        projection_name: &str,
        projection_version: i64,
        source_row_count: usize,
    ) -> GaryxDbResult<bool> {
        let projection_name = normalize_required("projection_name", projection_name)?;
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let conn = self.read_conn()?;
        let row = conn
            .query_row(
                "SELECT projection_version, source_row_count
                 FROM projection_states
                 WHERE projection_name = ?1",
                params![projection_name],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        Ok(row.is_some_and(|(version, count)| {
            version == projection_version && count == source_row_count
        }))
    }

    /// Whether a projection/migration state row exists at the given
    /// version, regardless of its recorded source count. The sqlite
    /// thread-record import gates on existence alone: in steady state new
    /// threads change the key count, and a count-sensitive gate would
    /// re-import on every boot — flowing the stale file archive back over
    /// the SQL truth (#TASK-1864 batch 2 on-device finding). Clearing the
    /// state row is the only event that forces a re-import.
    pub fn projection_state_exists(&self, name: &str, version: i64) -> GaryxDbResult<bool> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM projection_states
                  WHERE projection_name = ?1 AND projection_version = ?2",
                params![name, version],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Drop a projection/migration state row so its one-shot job runs
    /// again on the next eligible boot. Manual recovery hook: clearing
    /// the thread-records import row forces a fresh boot import from the
    /// archived source (review #TASK-1901: a same-key-count rewrite must
    /// not be skipped by the next import).
    pub fn clear_projection_state(&self, name: &str) -> GaryxDbResult<bool> {
        if name == LEGACY_IMPORT_GENERATION_NAME {
            return Err(GaryxDbError::BadRequest(
                "legacy_import_generation is monotonic and cannot be cleared".to_owned(),
            ));
        }
        let conn = self.conn()?;
        let removed = conn.execute(
            "DELETE FROM projection_states WHERE projection_name = ?1",
            params![name],
        )?;
        Ok(removed > 0)
    }

    pub fn record_projection_state(
        &self,
        projection_name: &str,
        projection_version: i64,
        source_row_count: usize,
    ) -> GaryxDbResult<()> {
        let projection_name = normalize_required("projection_name", projection_name)?;
        if projection_name == LEGACY_IMPORT_GENERATION_NAME {
            return Err(GaryxDbError::BadRequest(
                "legacy_import_generation is owned by the boot importer".to_owned(),
            ));
        }
        let source_row_count = i64::try_from(source_row_count).unwrap_or(i64::MAX);
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        record_projection_state_tx(
            &tx,
            &projection_name,
            projection_version,
            source_row_count,
            None,
        )?;
        tx.commit()?;
        Ok(())
    }
}
