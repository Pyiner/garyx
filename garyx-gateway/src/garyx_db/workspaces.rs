//! Workspace rows.

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub name: Option<String>,
    pub path: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDraft {
    pub name: Option<String>,
    pub path: String,
}

/// One `/api/workspaces` list entry: the workspace row joined with its
/// thread-membership aggregates (`thread_meta.root_workspace_path`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceListEntry {
    pub name: Option<String>,
    pub path: String,
    pub created_at: String,
    pub updated_at: String,
    pub pinned_at: Option<String>,
    pub thread_count: u64,
    pub last_activity_us: Option<i64>,
}

impl WorkspaceListEntry {
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| workspace_path_display_name(&self.path))
    }
}

pub fn workspace_path_display_name(path: &str) -> String {
    path.trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_owned()
}

pub(super) fn normalize_workspace_path(path: &str) -> GaryxDbResult<String> {
    let normalized = normalize_required("workspace path", path)?.replace('\\', "/");
    if !is_absolute_workspace_path(&normalized) {
        return Err(GaryxDbError::BadRequest(
            "workspace path must be absolute".to_owned(),
        ));
    }
    Ok(normalized)
}

pub(super) fn is_absolute_workspace_path(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with("//") {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

pub(super) fn workspace_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    Ok(WorkspaceRecord {
        name: row.get(0)?,
        path: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

pub(super) fn workspace_by_path(
    conn: &Connection,
    path: &str,
) -> GaryxDbResult<Option<WorkspaceRecord>> {
    Ok(conn
        .query_row(
            "SELECT name, path, created_at, updated_at FROM workspaces WHERE path = ?1",
            params![path],
            workspace_from_row,
        )
        .optional()?)
}

/// The one total order for workspace lists everywhere (sidebar and picker):
/// pinned first (latest pin on top), then latest thread activity, then
/// display name, then normalized path as the final tie-breaker.
pub(super) fn sort_workspace_list_entries(entries: &mut [WorkspaceListEntry]) {
    entries.sort_by(|left, right| {
        right
            .pinned_at
            .is_some()
            .cmp(&left.pinned_at.is_some())
            .then_with(|| right.pinned_at.cmp(&left.pinned_at))
            .then_with(|| right.last_activity_us.cmp(&left.last_activity_us))
            .then_with(|| {
                left.display_name()
                    .to_lowercase()
                    .cmp(&right.display_name().to_lowercase())
            })
            .then_with(|| left.path.cmp(&right.path))
    });
}

impl GaryxDbService {
    pub fn list_workspaces(&self) -> GaryxDbResult<Vec<WorkspaceRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT name, path, created_at, updated_at
             FROM workspaces
             WHERE deleted_at IS NULL
             ORDER BY lower(COALESCE(NULLIF(name, ''), path)) ASC, lower(path) ASC",
        )?;
        let rows = stmt.query_map([], workspace_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    /// Workspace list with thread-membership aggregates, pre-sorted in the
    /// shared total order. Counts and activity are live SQL aggregates over
    /// the `thread_meta` projection (`root_workspace_path` generated column);
    /// hidden side chats are excluded, task threads are counted.
    pub fn list_workspaces_with_stats(&self) -> GaryxDbResult<Vec<WorkspaceListEntry>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT w.name, w.path, w.created_at, w.updated_at, w.pinned_at,
                    COUNT(tm.thread_id) AS thread_count,
                    MAX(tm.sort_updated_at_us) AS last_activity_us
             FROM workspaces w
             LEFT JOIN thread_meta tm
                 ON tm.root_workspace_path = w.path
                AND tm.default_list_hidden = 0
             WHERE w.deleted_at IS NULL
             GROUP BY w.path",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WorkspaceListEntry {
                name: row.get(0)?,
                path: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                pinned_at: row.get(4)?,
                thread_count: row.get::<_, i64>(5)?.max(0) as u64,
                last_activity_us: row.get(6)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        sort_workspace_list_entries(&mut entries);
        Ok(entries)
    }

    /// Active-row-only point mutation: pin or unpin one workspace. Never
    /// inserts and never revives a tombstoned row; explicit Add is the only
    /// revival path.
    pub fn set_workspace_pinned(&self, path: &str, pinned: bool) -> GaryxDbResult<()> {
        let path = normalize_workspace_path(path)?;
        let now = now_string();
        let pinned_at = pinned.then(|| now.clone());
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE workspaces
             SET pinned_at = ?2, updated_at = ?3
             WHERE path = ?1 AND deleted_at IS NULL",
            params![path, pinned_at, now],
        )?;
        if updated == 0 {
            return Err(GaryxDbError::NotFound(format!(
                "workspace not found: {path}"
            )));
        }
        Ok(())
    }

    /// Active-row-only point mutation: rename one workspace. Never inserts
    /// and never revives a tombstoned row.
    pub fn rename_workspace(&self, path: &str, name: &str) -> GaryxDbResult<()> {
        let path = normalize_workspace_path(path)?;
        let name = normalize_required("workspace name", name)?;
        let now = now_string();
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE workspaces
             SET name = ?2, updated_at = ?3
             WHERE path = ?1 AND deleted_at IS NULL",
            params![path, name, now],
        )?;
        if updated == 0 {
            return Err(GaryxDbError::NotFound(format!(
                "workspace not found: {path}"
            )));
        }
        Ok(())
    }

    pub fn count_workspace_rows(&self) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn upsert_workspace(&self, draft: WorkspaceDraft) -> GaryxDbResult<WorkspaceRecord> {
        let path = normalize_workspace_path(&draft.path)?;
        let name = normalize_optional(draft.name.as_deref());
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO workspaces (path, name, created_at, updated_at, deleted_at, pinned_at)
             VALUES (?1, ?2, ?3, ?3, NULL, NULL)
             ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                updated_at = excluded.updated_at,
                -- Reviving a tombstoned row starts a fresh workspace
                -- lifecycle: the previous pin does not survive removal.
                pinned_at = CASE
                    WHEN workspaces.deleted_at IS NOT NULL THEN NULL
                    ELSE workspaces.pinned_at
                END,
                deleted_at = NULL",
            params![path, name, now],
        )?;
        workspace_by_path(&conn, &path)?
            .ok_or_else(|| GaryxDbError::BadRequest("workspace was not saved".to_owned()))
    }

    pub fn delete_workspace(&self, path: &str) -> GaryxDbResult<bool> {
        let path = normalize_workspace_path(path)?;
        let now = now_string();
        let conn = self.conn()?;
        let removed = conn.execute(
            "UPDATE workspaces
             SET updated_at = ?2, deleted_at = ?2
             WHERE path = ?1 AND deleted_at IS NULL",
            params![path, now],
        )?;
        if removed == 0 {
            conn.execute(
                "INSERT INTO workspaces (path, name, created_at, updated_at, deleted_at)
                 VALUES (?1, NULL, ?2, ?2, ?2)
                 ON CONFLICT(path) DO NOTHING",
                params![path, now],
            )?;
        }
        Ok(removed > 0)
    }

    pub fn seed_workspaces_if_empty(&self, drafts: Vec<WorkspaceDraft>) -> GaryxDbResult<bool> {
        let mut normalized = Vec::new();
        let mut seen = BTreeSet::new();
        for draft in drafts {
            let path = normalize_workspace_path(&draft.path)?;
            if !seen.insert(path.clone()) {
                continue;
            }
            normalized.push(WorkspaceDraft {
                name: normalize_optional(draft.name.as_deref()),
                path,
            });
        }
        if normalized.is_empty() {
            return Ok(false);
        }

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?;
        if count > 0 {
            tx.commit()?;
            return Ok(false);
        }

        let now = now_string();
        for draft in normalized {
            tx.execute(
                "INSERT INTO workspaces (path, name, created_at, updated_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?3, NULL)",
                params![draft.path, draft.name, now],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }
}
