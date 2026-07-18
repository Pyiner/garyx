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
            "INSERT INTO workspaces (path, name, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, ?3, ?3, NULL)
             ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                updated_at = excluded.updated_at,
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
