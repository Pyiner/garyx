//! Canonical `thread_records` bodies and same-transaction projection writes.

use super::*;

pub(super) fn thread_record_exists_tx(conn: &Connection, thread_id: &str) -> GaryxDbResult<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM thread_records WHERE key = ?1",
            params![thread_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Projection writes derived from one thread record, applied inside the
/// same transaction as the record upsert (#TASK-1864 batch 2, D2). Each
/// `Some` upserts that projection; `None` removes it.
pub struct ThreadRecordProjections {
    pub thread_meta: Option<ThreadMetaProjectionDraft>,
    pub task: Option<TaskProjectionDraft>,
    pub recent: Option<RecentThreadDraft>,
}

/// One record write inside an atomic multi-record batch.
pub struct ThreadRecordWrite {
    pub key: String,
    pub body: String,
    pub updated_at: Option<String>,
    pub projections: Option<ThreadRecordProjections>,
}

pub(super) fn write_thread_record_with_projections_tx(
    tx: &Transaction<'_>,
    key: &str,
    body: &str,
    updated_at: Option<&str>,
    projections: Option<ThreadRecordProjections>,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let key = normalize_required("key", key)?;
    // Archived threads reject writes inside the same transaction that
    // would persist them — a tombstone committed by a racing archive
    // can never be overtaken by a write that passed an earlier check.
    if garyx_router::is_thread_key(&key) {
        let archived: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM archived_threads WHERE thread_id = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        if archived.is_some() {
            return Err(GaryxDbError::ThreadArchived(key));
        }
    }
    tx.execute(
        "INSERT INTO thread_records (key, body, updated_at, recorded_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET
            body = excluded.body,
            updated_at = excluded.updated_at,
            recorded_at = excluded.recorded_at",
        params![key, body, updated_at, recorded_at],
    )?;
    if let Some(projections) = projections {
        match projections.thread_meta {
            Some(draft) => replace_thread_meta_projection_tx(tx, draft, recorded_at)?,
            None => {
                remove_thread_meta_projection_tx(tx, &key)?;
            }
        }
        match projections.task {
            Some(mut draft) => {
                draft.thread_id = normalize_thread_id(&draft.thread_id)?;
                task_forest::upsert_task_projection(tx, &draft, recorded_at)?;
            }
            None => {
                remove_task_projection_tx(tx, &key)?;
            }
        }
        match projections.recent {
            Some(draft) => {
                upsert_recent_thread_tx(tx, draft, recorded_at)?;
            }
            None => {
                remove_recent_thread_tx(tx, &key)?;
            }
        }
    }
    Ok(())
}

/// Escape `%`/`_`/`\` so a caller-supplied prefix matches literally in a
/// LIKE pattern (used with `ESCAPE '\'`).
pub(super) fn escape_like_pattern(prefix: &str) -> String {
    prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(super) fn remove_task_projection_tx(conn: &Connection, thread_id: &str) -> GaryxDbResult<bool> {
    let removed = conn.execute(
        "DELETE FROM task_projection WHERE thread_id = ?1",
        params![thread_id],
    )?;
    Ok(removed > 0)
}

impl GaryxDbService {
    /// Single-transaction write of a thread record plus its derived
    /// projections (#TASK-1864 batch 2, D2): the record and the five
    /// projection tables commit or roll back together, so projection drift
    /// is structurally impossible. `projections: None` writes the record
    /// only (non-thread keys such as `meta::`/`cron::`/`tool::`).
    pub fn write_thread_record_with_projections(
        &self,
        key: &str,
        body: &str,
        updated_at: Option<&str>,
        projections: Option<ThreadRecordProjections>,
    ) -> GaryxDbResult<()> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        write_thread_record_with_projections_tx(
            &tx,
            key,
            body,
            updated_at,
            projections,
            &recorded_at,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// All-or-nothing write of MULTIPLE thread records plus their derived
    /// projections in one transaction (#TASK-2099 root final review):
    /// endpoint binding mutations touch the previous owner, the target,
    /// and the known-endpoint registry together — either every record and
    /// projection commits or none do, so a mid-mutation storage failure
    /// can never lose the active binding.
    pub fn write_thread_records_with_projections_atomic(
        &self,
        entries: Vec<ThreadRecordWrite>,
    ) -> GaryxDbResult<()> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for entry in entries {
            write_thread_record_with_projections_tx(
                &tx,
                &entry.key,
                &entry.body,
                entry.updated_at.as_deref(),
                entry.projections,
                &recorded_at,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Single-transaction terminal delete of a thread record, all its
    /// projection rows, pin, and favorite. Deleting an existing record or
    /// upgrading an archived tombstone leaves `deleted`; a genuinely missing
    /// thread with no tombstone remains missing, matching the lifecycle
    /// result matrix's rejected-not-found branch.
    ///
    /// The [`garyx_router::DrainedDeleteReservation`] parameter is a
    /// typestate witness minted only by the coordinator's delete abort/drain
    /// barrier (or the inert `test-seams` constructor in tests). The delete
    /// target derives from the witness itself — there is no separate key
    /// parameter to point at another thread — and the witness is consumed
    /// here into its settle-only stage, so one witness admits at most one
    /// raw destructive delete.
    pub(crate) fn delete_thread_record_with_projections(
        &self,
        drained_delete: garyx_router::DrainedDeleteReservation,
    ) -> GaryxDbResult<(bool, garyx_router::DeleteSettlement)> {
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_block_test_db_mutation(TestDbMutationPoint::DeleteThreadRecord);
        #[cfg(any(test, feature = "test-seams"))]
        self.maybe_fail_test_db_call(TestDbFaultPoint::DeleteThreadRecord)?;
        drained_delete.storage_delete(|target| {
            let key = normalize_required("key", target)?;
            let mut conn = self.conn()?;
            let tx = conn.transaction()?;
            let record_exists = tx
                .query_row(
                    "SELECT 1 FROM thread_records WHERE key = ?1",
                    params![key],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            let terminal = if is_thread_key(&key) {
                read_thread_terminal_state(&tx, &key)?
            } else {
                None
            };
            if is_thread_key(&key)
                && (terminal == Some(ThreadTerminalState::Deleted)
                    || (!record_exists && terminal.is_none()))
            {
                return Ok(false);
            }
            if is_thread_key(&key) {
                tx.execute(
                    "INSERT INTO archived_threads (thread_id, archived_at, kind)
                 VALUES (?1, ?2, 'deleted')
                 ON CONFLICT(thread_id) DO UPDATE SET
                    archived_at = excluded.archived_at,
                    kind = 'deleted'",
                    params![key, now_string()],
                )?;
            }
            let removed =
                tx.execute("DELETE FROM thread_records WHERE key = ?1", params![key])? > 0;
            remove_thread_meta_projection_tx(&tx, &key)?;
            remove_task_projection_tx(&tx, &key)?;
            remove_recent_thread_tx(&tx, &key)?;
            let removed_pin =
                tx.execute("DELETE FROM thread_pins WHERE thread_id = ?1", params![key])? > 0;
            bump_thread_pins_revision_if_changed_tx(&tx, removed_pin)?;
            let removed_favorite = tx.execute(
                "DELETE FROM thread_favorites WHERE thread_id = ?1",
                params![key],
            )? > 0;
            // The terminal tombstone is now the resurrection fence, so the
            // collection revision changes only when the favorite collection did.
            bump_thread_favorites_revision_if_changed_tx(&tx, removed_favorite)?;
            tx.commit()?;
            Ok(removed)
        })
    }

    /// Point read of a record body from the reader connection (WAL snapshot
    /// read — never queued behind the writer).
    pub fn get_thread_record_body(&self, key: &str) -> GaryxDbResult<Option<String>> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT body FROM thread_records WHERE key = ?1",
                params![key.trim()],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn thread_record_exists(&self, key: &str) -> GaryxDbResult<bool> {
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM thread_records WHERE key = ?1",
                params![key.trim()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Count record keys by prefix with the same exact case-sensitive
    /// prefix semantics as `list_thread_record_keys`.
    pub fn count_thread_record_keys(&self, prefix: Option<&str>) -> GaryxDbResult<usize> {
        match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) => {
                // LIKE is ASCII case-insensitive in SQLite; count exact
                // matches in Rust over the narrowed set (same reasoning as
                // list_thread_record_keys, review #TASK-1896).
                Ok(self.list_thread_record_keys(Some(prefix))?.len())
            }
            None => {
                let conn = self.read_conn()?;
                let count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM thread_records", [], |row| row.get(0))?;
                Ok(usize::try_from(count).unwrap_or(usize::MAX))
            }
        }
    }

    pub fn list_thread_record_keys(&self, prefix: Option<&str>) -> GaryxDbResult<Vec<String>> {
        let conn = self.read_conn()?;
        let mut keys = Vec::new();
        match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) => {
                // LIKE narrows the scan but is ASCII case-insensitive in
                // SQLite; the starts_with post-filter restores the exact
                // case-sensitive prefix semantics of the File/InMemory
                // stores (review #TASK-1896).
                let pattern = format!("{}%", escape_like_pattern(prefix));
                let mut stmt = conn.prepare(
                    "SELECT key FROM thread_records WHERE key LIKE ?1 ESCAPE '\\' ORDER BY key",
                )?;
                let rows = stmt.query_map(params![pattern], |row| row.get::<_, String>(0))?;
                for row in rows {
                    let key: String = row?;
                    if key.starts_with(prefix) {
                        keys.push(key);
                    }
                }
            }
            None => {
                let mut stmt = conn.prepare("SELECT key FROM thread_records ORDER BY key")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                for row in rows {
                    keys.push(row?);
                }
            }
        }
        Ok(keys)
    }
}
