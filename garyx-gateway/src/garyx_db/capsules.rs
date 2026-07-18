//! Capsule rows.

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapsuleRecord {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub html_sha256: String,
    pub byte_size: i64,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub favorited_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsuleCreateDraft {
    pub id: String,
    pub title: String,
    pub description: String,
    pub thread_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub html_sha256: String,
    pub byte_size: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapsuleUpdateDraft {
    pub title: Option<String>,
    pub description: Option<String>,
    pub html_sha256: Option<String>,
    pub byte_size: Option<i64>,
}

pub(super) fn normalize_capsule_id(id: &str) -> GaryxDbResult<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(GaryxDbError::BadRequest(
            "capsule id must not be empty".to_owned(),
        ));
    }
    Uuid::parse_str(trimmed)
        .map(|uuid| uuid.to_string())
        .map_err(|_| GaryxDbError::BadRequest("capsule id must be a UUID".to_owned()))
}

pub(super) fn normalize_capsule_text(value: &str) -> String {
    value.trim().to_owned()
}

pub(super) fn normalize_capsule_sha256(value: &str) -> GaryxDbResult<String> {
    let trimmed = normalize_required("html_sha256", value)?.to_ascii_lowercase();
    if trimmed.len() != 64 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GaryxDbError::BadRequest(
            "html_sha256 must be 64 hex characters".to_owned(),
        ));
    }
    Ok(trimmed)
}

pub(super) fn normalize_capsule_byte_size(value: i64) -> GaryxDbResult<i64> {
    if value < 0 {
        return Err(GaryxDbError::BadRequest(
            "byte_size must be non-negative".to_owned(),
        ));
    }
    Ok(value)
}

pub(super) fn capsule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CapsuleRecord> {
    Ok(CapsuleRecord {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        thread_id: row.get(3)?,
        run_id: row.get(4)?,
        agent_id: row.get(5)?,
        provider_type: row.get(6)?,
        html_sha256: row.get(7)?,
        byte_size: row.get(8)?,
        revision: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        favorited_at: row.get(12)?,
    })
}

pub(super) fn capsule_by_id(conn: &Connection, id: &str) -> GaryxDbResult<Option<CapsuleRecord>> {
    Ok(conn
        .query_row(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at, favorited_at
             FROM capsules
             WHERE id = ?1",
            params![id],
            capsule_from_row,
        )
        .optional()?)
}

impl GaryxDbService {
    pub fn create_capsule(&self, draft: CapsuleCreateDraft) -> GaryxDbResult<CapsuleRecord> {
        let id = normalize_capsule_id(&draft.id)?;
        let title = normalize_capsule_text(&draft.title);
        let description = normalize_capsule_text(&draft.description);
        let thread_id = normalize_optional(draft.thread_id.as_deref());
        let run_id = normalize_optional(draft.run_id.as_deref());
        let agent_id = normalize_optional(draft.agent_id.as_deref());
        let provider_type = normalize_optional(draft.provider_type.as_deref());
        let html_sha256 = normalize_capsule_sha256(&draft.html_sha256)?;
        let byte_size = normalize_capsule_byte_size(draft.byte_size)?;
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO capsules (
                id, title, description, thread_id, run_id, agent_id, provider_type,
                html_sha256, byte_size, revision, created_at, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10)",
            params![
                id,
                title,
                description,
                thread_id,
                run_id,
                agent_id,
                provider_type,
                html_sha256,
                byte_size,
                now,
            ],
        )?;
        capsule_by_id(&conn, &id)?
            .ok_or_else(|| GaryxDbError::BadRequest("capsule was not saved".to_owned()))
    }

    pub fn update_capsule(
        &self,
        id: &str,
        draft: CapsuleUpdateDraft,
    ) -> GaryxDbResult<Option<CapsuleRecord>> {
        let id = normalize_capsule_id(id)?;
        let title = draft.title.as_deref().map(normalize_capsule_text);
        let description = draft.description.as_deref().map(normalize_capsule_text);
        let html_sha256 = draft
            .html_sha256
            .as_deref()
            .map(normalize_capsule_sha256)
            .transpose()?;
        let byte_size = draft
            .byte_size
            .map(normalize_capsule_byte_size)
            .transpose()?;
        let now = now_string();
        let conn = self.conn()?;
        let updated = conn.execute(
            "UPDATE capsules
             SET title = COALESCE(?2, title),
                 description = COALESCE(?3, description),
                 html_sha256 = COALESCE(?4, html_sha256),
                 byte_size = COALESCE(?5, byte_size),
                 revision = revision + 1,
                 updated_at = ?6
             WHERE id = ?1",
            params![id, title, description, html_sha256, byte_size, now],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        capsule_by_id(&conn, &id)
    }

    pub fn set_capsule_favorite(
        &self,
        id: &str,
        favorited: bool,
    ) -> GaryxDbResult<Option<CapsuleRecord>> {
        let id = normalize_capsule_id(id)?;
        let conn = self.conn()?;
        let updated = if favorited {
            conn.execute(
                "UPDATE capsules
                 SET favorited_at = COALESCE(favorited_at, ?2)
                 WHERE id = ?1",
                params![id, now_string()],
            )?
        } else {
            conn.execute(
                "UPDATE capsules SET favorited_at = NULL WHERE id = ?1",
                params![id],
            )?
        };
        if updated == 0 {
            return Ok(None);
        }
        capsule_by_id(&conn, &id)
    }

    pub fn get_capsule(&self, id: &str) -> GaryxDbResult<Option<CapsuleRecord>> {
        let id = normalize_capsule_id(id)?;
        let conn = self.read_conn()?;
        capsule_by_id(&conn, &id)
    }

    pub fn list_capsules(&self) -> GaryxDbResult<Vec<CapsuleRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at, favorited_at
             FROM capsules
             ORDER BY updated_at DESC, id ASC",
        )?;
        let rows = stmt.query_map([], capsule_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn list_capsules_for_thread(&self, thread_id: &str) -> GaryxDbResult<Vec<CapsuleRecord>> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, thread_id, run_id, agent_id, provider_type,
                    html_sha256, byte_size, revision, created_at, updated_at, favorited_at
             FROM capsules
             WHERE thread_id = ?1
             ORDER BY updated_at DESC, id ASC",
        )?;
        let rows = stmt.query_map(params![thread_id], capsule_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn delete_capsule(&self, id: &str) -> GaryxDbResult<bool> {
        let id = normalize_capsule_id(id)?;
        let conn = self.conn()?;
        let removed = conn.execute("DELETE FROM capsules WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }
}
