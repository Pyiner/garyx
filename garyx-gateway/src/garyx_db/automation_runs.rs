//! Automation thread-run rows.

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AutomationThreadRunRecord {
    pub automation_id: String,
    pub run_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub agent_id: Option<String>,
    pub automation_label_snapshot: Option<String>,
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationThreadRunDraft {
    pub automation_id: String,
    pub run_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub agent_id: Option<String>,
    pub automation_label_snapshot: Option<String>,
    pub mode: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
}

pub(super) fn normalize_automation_thread_run_mode(value: &str) -> GaryxDbResult<String> {
    let mode = normalize_required("mode", value)?;
    match mode.as_str() {
        "generated_thread" | "target_thread" => Ok(mode),
        _ => Err(GaryxDbError::BadRequest(
            "mode must be generated_thread or target_thread".to_owned(),
        )),
    }
}

pub(super) fn automation_thread_run_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AutomationThreadRunRecord> {
    Ok(AutomationThreadRunRecord {
        automation_id: row.get(0)?,
        run_id: row.get(1)?,
        thread_id: row.get(2)?,
        workspace_dir: row.get(3)?,
        agent_id: row.get(4)?,
        automation_label_snapshot: row.get(5)?,
        mode: row.get(6)?,
        status: row.get(7)?,
        started_at: row.get(8)?,
        finished_at: row.get(9)?,
        recorded_at: row.get(10)?,
    })
}

pub(super) fn automation_thread_run_by_key(
    conn: &Connection,
    automation_id: &str,
    run_id: &str,
) -> GaryxDbResult<Option<AutomationThreadRunRecord>> {
    Ok(conn
        .query_row(
            "SELECT automation_id, run_id, thread_id, workspace_dir, agent_id,
                    automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             FROM automation_thread_runs
             WHERE automation_id = ?1 AND run_id = ?2",
            params![automation_id, run_id],
            automation_thread_run_from_row,
        )
        .optional()?)
}

impl GaryxDbService {
    pub fn upsert_automation_thread_run(
        &self,
        draft: AutomationThreadRunDraft,
    ) -> GaryxDbResult<AutomationThreadRunRecord> {
        let automation_id = normalize_required("automation_id", &draft.automation_id)?;
        let run_id = normalize_required("run_id", &draft.run_id)?;
        let thread_id = normalize_thread_id(&draft.thread_id)?;
        let mode = normalize_automation_thread_run_mode(&draft.mode)?;
        let status = normalize_required("status", &draft.status)?;
        let started_at = normalize_required("started_at", &draft.started_at)?;
        let workspace_dir = normalize_optional(draft.workspace_dir.as_deref());
        let agent_id = normalize_optional(draft.agent_id.as_deref());
        let automation_label_snapshot =
            normalize_optional(draft.automation_label_snapshot.as_deref());
        let finished_at = normalize_optional(draft.finished_at.as_deref());
        let recorded_at = now_string();

        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO automation_thread_runs (
                automation_id, run_id, thread_id, workspace_dir, agent_id,
                automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(automation_id, run_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                workspace_dir = excluded.workspace_dir,
                agent_id = excluded.agent_id,
                automation_label_snapshot = excluded.automation_label_snapshot,
                mode = excluded.mode,
                status = excluded.status,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                recorded_at = excluded.recorded_at",
            params![
                automation_id,
                run_id,
                thread_id,
                workspace_dir,
                agent_id,
                automation_label_snapshot,
                mode,
                status,
                started_at,
                finished_at,
                recorded_at,
            ],
        )?;

        automation_thread_run_by_key(&conn, &automation_id, &run_id)?.ok_or_else(|| {
            GaryxDbError::BadRequest("automation thread run was not saved".to_owned())
        })
    }

    pub fn finish_automation_thread_run(
        &self,
        automation_id: &str,
        run_id: &str,
        status: &str,
        finished_at: &str,
    ) -> GaryxDbResult<bool> {
        let automation_id = normalize_required("automation_id", automation_id)?;
        let run_id = normalize_required("run_id", run_id)?;
        let status = normalize_required("status", status)?;
        let finished_at = normalize_required("finished_at", finished_at)?;
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE automation_thread_runs
             SET status = ?3, finished_at = ?4, recorded_at = ?5
             WHERE automation_id = ?1 AND run_id = ?2",
            params![automation_id, run_id, status, finished_at, now_string()],
        )?;
        Ok(updated > 0)
    }

    pub fn list_automation_thread_runs(
        &self,
        automation_id: &str,
        mode: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> GaryxDbResult<Vec<AutomationThreadRunRecord>> {
        let automation_id = normalize_required("automation_id", automation_id)?;
        let mode = mode.map(normalize_automation_thread_run_mode).transpose()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let conn = self.read_conn()?;
        let sql = if mode.is_some() {
            "SELECT automation_id, run_id, thread_id, workspace_dir, agent_id,
                    automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             FROM automation_thread_runs
             WHERE automation_id = ?1 AND mode = ?2
             ORDER BY started_at DESC, recorded_at DESC, run_id ASC
             LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT automation_id, run_id, thread_id, workspace_dir, agent_id,
                    automation_label_snapshot, mode, status, started_at, finished_at, recorded_at
             FROM automation_thread_runs
             WHERE automation_id = ?1
             ORDER BY started_at DESC, recorded_at DESC, run_id ASC
             LIMIT ?2 OFFSET ?3"
        };
        let mut stmt = conn.prepare(sql)?;
        let mut records = Vec::new();
        if let Some(mode) = mode {
            let rows = stmt.query_map(
                params![automation_id, mode, limit, offset],
                automation_thread_run_from_row,
            )?;
            for row in rows {
                records.push(row?);
            }
        } else {
            let rows = stmt.query_map(
                params![automation_id, limit, offset],
                automation_thread_run_from_row,
            )?;
            for row in rows {
                records.push(row?);
            }
        }
        Ok(records)
    }

    pub fn count_automation_thread_runs(
        &self,
        automation_id: &str,
        mode: Option<&str>,
    ) -> GaryxDbResult<usize> {
        let automation_id = normalize_required("automation_id", automation_id)?;
        let mode = mode.map(normalize_automation_thread_run_mode).transpose()?;
        let conn = self.read_conn()?;
        let count: i64 = if let Some(mode) = mode {
            conn.query_row(
                "SELECT COUNT(*) FROM automation_thread_runs WHERE automation_id = ?1 AND mode = ?2",
                params![automation_id, mode],
                |row| row.get(0),
            )?
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM automation_thread_runs WHERE automation_id = ?1",
                params![automation_id],
                |row| row.get(0),
            )?
        };
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }
}
