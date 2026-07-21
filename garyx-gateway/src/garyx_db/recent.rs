//! `recent_threads` projection: pages, activity seq, active-run recovery.

use super::*;

pub(crate) const MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveRecentThreadPage {
    pub thread_ids: Vec<String>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RecentThreadRecord {
    pub thread_id: String,
    pub title: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub provider_type: Option<String>,
    pub agent_id: Option<String>,
    pub message_count: u32,
    pub last_message_preview: String,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub run_state: String,
    pub updated_at: Option<String>,
    pub last_active_at: String,
    pub activity_seq: i64,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RecentThreadTaskFilter {
    #[default]
    Include,
    Exclude,
    Only,
}

impl RecentThreadTaskFilter {
    pub(crate) fn cursor_value(self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::Exclude => "exclude",
            Self::Only => "only",
        }
    }

    fn count_sql(self) -> &'static str {
        match self {
            Self::Include => "SELECT COUNT(*) FROM recent_threads",
            Self::Exclude => "SELECT COUNT(*) FROM recent_threads WHERE thread_type <> 'task'",
            Self::Only => "SELECT COUNT(*) FROM recent_threads WHERE thread_type = 'task'",
        }
    }

    fn page_sql(self) -> &'static str {
        match self {
            Self::Include => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  ORDER BY activity_seq DESC
                  LIMIT ?1 OFFSET ?2"
            }
            Self::Exclude => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type <> 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1 OFFSET ?2"
            }
            Self::Only => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type = 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1 OFFSET ?2"
            }
        }
    }

    fn keyset_page_sql(self, has_cursor: bool) -> &'static str {
        match (self, has_cursor) {
            (Self::Include, false) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  ORDER BY activity_seq DESC
                  LIMIT ?1"
            }
            (Self::Include, true) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE activity_seq < ?1
                  ORDER BY activity_seq DESC
                  LIMIT ?2"
            }
            (Self::Exclude, false) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type <> 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1"
            }
            (Self::Exclude, true) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type <> 'task' AND activity_seq < ?1
                  ORDER BY activity_seq DESC
                  LIMIT ?2"
            }
            (Self::Only, false) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type = 'task'
                  ORDER BY activity_seq DESC
                  LIMIT ?1"
            }
            (Self::Only, true) => {
                "SELECT thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
                        message_count, last_message_preview, recent_run_id, active_run_id,
                        run_state, updated_at, last_active_at, activity_seq, recorded_at
                   FROM recent_threads
                  WHERE thread_type = 'task' AND activity_seq < ?1
                  ORDER BY activity_seq DESC
                  LIMIT ?2"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecentThreadDbPage {
    pub records: Vec<RecentThreadRecord>,
    pub total: usize,
    pub offset: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecentThreadKeysetDbPage {
    pub records: Vec<RecentThreadRecord>,
    pub total: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentThreadDraft {
    pub thread_id: String,
    pub title: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub provider_type: Option<String>,
    pub agent_id: Option<String>,
    pub message_count: u32,
    pub last_message_preview: String,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub run_state: String,
    pub updated_at: Option<String>,
    pub last_active_at: String,
}

pub(super) fn list_active_recent_thread_ids(
    conn: &mut Connection,
    limit: usize,
) -> GaryxDbResult<ActiveRecentThreadPage> {
    const ACTIVE_RECENT_THREAD_PREDICATE: &str = "thread_id GLOB 'thread::*' AND (run_state = 'running' OR COALESCE(TRIM(active_run_id), '') <> '')";

    // Count and page share one WAL snapshot, matching the recent-thread page
    // contract. The predicate stays in SQL: restart wake-all is a conditional
    // thread query and must not enumerate record bodies or filter a full table
    // in application code.
    let tx = conn.transaction()?;
    let total_sql =
        format!("SELECT COUNT(*) FROM recent_threads WHERE {ACTIVE_RECENT_THREAD_PREDICATE}");
    let total: i64 = tx.query_row(&total_sql, [], |row| row.get(0))?;
    let total = usize::try_from(total).unwrap_or(usize::MAX);

    let page_sql = format!(
        "SELECT thread_id
           FROM recent_threads
          WHERE {ACTIVE_RECENT_THREAD_PREDICATE}
          ORDER BY activity_seq DESC
          LIMIT ?1"
    );
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut stmt = tx.prepare(&page_sql)?;
    let rows = stmt.query_map([limit], |row| row.get(0))?;
    let mut thread_ids = Vec::new();
    for row in rows {
        thread_ids.push(row?);
    }
    drop(stmt);
    tx.commit()?;

    Ok(ActiveRecentThreadPage { thread_ids, total })
}

pub(super) fn upsert_recent_thread_tx(
    tx: &Transaction<'_>,
    draft: RecentThreadDraft,
    recorded_at: &str,
) -> GaryxDbResult<RecentThreadRecord> {
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
    let recorded_at = recorded_at.to_owned();
    let activity_seq = allocate_recent_thread_activity_seq_tx(tx)?;

    tx.execute(
        "INSERT INTO recent_threads (
            thread_id, title, workspace_dir, thread_type, provider_type, agent_id,
            message_count, last_message_preview, recent_run_id, active_run_id, run_state,
            updated_at, last_active_at, activity_seq, recorded_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(thread_id) DO UPDATE SET
            title = excluded.title,
            workspace_dir = excluded.workspace_dir,
            thread_type = excluded.thread_type,
            provider_type = excluded.provider_type,
            agent_id = excluded.agent_id,
            message_count = excluded.message_count,
            last_message_preview = excluded.last_message_preview,
            recent_run_id = excluded.recent_run_id,
            active_run_id = excluded.active_run_id,
            run_state = excluded.run_state,
            updated_at = excluded.updated_at,
            last_active_at = excluded.last_active_at,
            activity_seq = excluded.activity_seq,
            recorded_at = excluded.recorded_at",
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
            activity_seq,
            recorded_at,
        ],
    )?;

    Ok(RecentThreadRecord {
        thread_id,
        title,
        workspace_dir,
        thread_type,
        provider_type,
        agent_id,
        message_count: draft.message_count,
        last_message_preview,
        recent_run_id,
        active_run_id,
        run_state,
        updated_at,
        last_active_at,
        activity_seq,
        recorded_at,
    })
}

pub(super) fn allocate_recent_thread_activity_seq_tx(tx: &Transaction<'_>) -> GaryxDbResult<i64> {
    let current: i64 = tx.query_row(
        "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let next = current
        .checked_add(1)
        .filter(|value| *value < MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE)
        .ok_or_else(|| {
            GaryxDbError::Configuration(
                "recent thread activity sequence space is exhausted".to_owned(),
            )
        })?;
    let updated = tx.execute(
        "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
        params![next],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "recent_threads_meta singleton is missing".to_owned(),
        ));
    }
    Ok(next)
}

pub(super) fn remove_recent_thread_tx(conn: &Connection, thread_id: &str) -> GaryxDbResult<bool> {
    let removed = conn.execute(
        "DELETE FROM recent_threads WHERE thread_id = ?1",
        params![thread_id],
    )?;
    Ok(removed > 0)
}

pub(super) fn recent_thread_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RecentThreadRecord> {
    Ok(RecentThreadRecord {
        thread_id: row.get(0)?,
        title: row.get(1)?,
        workspace_dir: row.get(2)?,
        thread_type: row.get(3)?,
        provider_type: row.get(4)?,
        agent_id: row.get(5)?,
        message_count: row.get(6)?,
        last_message_preview: row.get(7)?,
        recent_run_id: row.get(8)?,
        active_run_id: row.get(9)?,
        run_state: row.get(10)?,
        updated_at: row.get(11)?,
        last_active_at: row.get(12)?,
        activity_seq: row.get(13)?,
        recorded_at: row.get(14)?,
    })
}

impl GaryxDbService {
    pub(crate) fn list_active_recent_thread_ids(
        &self,
        limit: usize,
    ) -> GaryxDbResult<ActiveRecentThreadPage> {
        let mut conn = self.read_conn()?;
        list_active_recent_thread_ids(&mut conn, limit)
    }

    pub fn list_recent_threads(
        &self,
        limit: usize,
        offset: usize,
    ) -> GaryxDbResult<Vec<RecentThreadRecord>> {
        Ok(self
            .list_recent_threads_page(RecentThreadTaskFilter::Include, limit, offset)?
            .records)
    }

    pub(crate) fn list_recent_threads_page(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        requested_offset: usize,
    ) -> GaryxDbResult<RecentThreadDbPage> {
        self.list_recent_threads_page_inner(filter, limit, requested_offset, || Ok(()))
    }

    pub(crate) fn list_recent_threads_keyset_page(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        before_activity_seq: Option<i64>,
    ) -> GaryxDbResult<RecentThreadKeysetDbPage> {
        self.list_recent_threads_keyset_page_inner(filter, limit, before_activity_seq, || Ok(()))
    }

    pub(crate) fn contains_selectable_recent_thread(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT 1
                   FROM recent_threads
                  WHERE thread_id = ?1 AND thread_type <> 'task'",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub(super) fn list_recent_threads_page_inner<F>(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        requested_offset: usize,
        after_count: F,
    ) -> GaryxDbResult<RecentThreadDbPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(filter.count_sql(), [], |row| row.get(0))?;
        let total = usize::try_from(total).unwrap_or(usize::MAX);
        let offset = requested_offset.min(total);

        // Test seam for proving that the count and page stay on one WAL read
        // snapshot when a writer commits between the two statements.
        after_count()?;

        let limit_param = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset_param = i64::try_from(offset).unwrap_or(i64::MAX);
        let mut stmt = tx.prepare(filter.page_sql())?;
        let rows = stmt.query_map(
            params![limit_param, offset_param],
            recent_thread_record_from_row,
        )?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        drop(stmt);
        tx.commit()?;

        let has_more = offset.saturating_add(records.len()) < total;
        Ok(RecentThreadDbPage {
            records,
            total,
            offset,
            has_more,
        })
    }

    pub(super) fn list_recent_threads_keyset_page_inner<F>(
        &self,
        filter: RecentThreadTaskFilter,
        limit: usize,
        before_activity_seq: Option<i64>,
        after_count: F,
    ) -> GaryxDbResult<RecentThreadKeysetDbPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(filter.count_sql(), [], |row| row.get(0))?;
        let total = usize::try_from(total).unwrap_or(usize::MAX);

        // Count and page are display metadata from one WAL snapshot. A
        // concurrent writer may commit here, but this page must not mix it
        // with the earlier total.
        after_count()?;

        let fetch_limit = limit.saturating_add(1);
        let fetch_limit = i64::try_from(fetch_limit).unwrap_or(i64::MAX);
        let mut stmt = tx.prepare(filter.keyset_page_sql(before_activity_seq.is_some()))?;
        let mut rows = match before_activity_seq {
            Some(activity_seq) => stmt.query(params![activity_seq, fetch_limit])?,
            None => stmt.query(params![fetch_limit])?,
        };
        let mut records = Vec::with_capacity(limit.saturating_add(1));
        while let Some(row) = rows.next()? {
            records.push(recent_thread_record_from_row(row)?);
        }
        drop(rows);
        drop(stmt);
        tx.commit()?;

        let has_more = records.len() > limit;
        if has_more {
            records.truncate(limit);
        }
        Ok(RecentThreadKeysetDbPage {
            records,
            total,
            has_more,
        })
    }

    /// Startup crash recovery: the bridge run index is rebuilt empty on
    /// boot, so any projected `active_run_id`/`running` row is a dangling
    /// orphan from the previous process. One SQL pass settles both
    /// projection tables — no store scan, no file reads (#TASK-1864
    /// closing batch; replaces the retired reconcile walk).
    pub fn clear_stale_active_runs(&self) -> GaryxDbResult<usize> {
        let conn = self.conn()?;
        // Deliberately does not allocate activity_seq: merely settling a run
        // orphan from the previous boot must not move an old thread to the
        // head. RuntimeAssembler invokes this under the data-dir lock before
        // listener bind; this is a reviewed pre-bind-only direct
        // recent_threads UPDATE (contract recorded in
        // docs/agents/repository-contracts.md).
        let recent = conn.execute(
            "UPDATE recent_threads
                SET active_run_id = NULL,
                    run_state = CASE
                        WHEN recent_run_id IS NULL OR recent_run_id = '' THEN 'idle'
                        ELSE 'completed'
                    END
              WHERE active_run_id IS NOT NULL OR run_state = 'running'",
            [],
        )?;
        let meta = conn.execute(
            "UPDATE thread_meta SET active_run_id = NULL WHERE active_run_id IS NOT NULL",
            [],
        )?;
        Ok(recent + meta)
    }

    pub fn count_recent_threads(&self) -> GaryxDbResult<usize> {
        Ok(self
            .list_recent_threads_page(RecentThreadTaskFilter::Include, 0, 0)?
            .total)
    }

    pub fn upsert_recent_thread(
        &self,
        draft: RecentThreadDraft,
    ) -> GaryxDbResult<RecentThreadRecord> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let record = upsert_recent_thread_tx(&tx, draft, &recorded_at)?;
        tx.commit()?;
        Ok(record)
    }

    pub fn remove_recent_thread(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let removed = conn.execute(
            "DELETE FROM recent_threads WHERE thread_id = ?1",
            params![thread_id],
        )?;
        Ok(removed > 0)
    }
}
