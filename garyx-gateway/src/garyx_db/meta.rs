//! `thread_meta` projection records and mutations.

use super::*;

pub(super) const CURRENT_THREAD_META_PROJECTION_VERSION: i64 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMetaRecord {
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub thread_label: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub provider_key: Option<String>,
    pub selected_model: Option<String>,
    pub selected_model_reasoning_effort: Option<String>,
    pub selected_model_service_tier: Option<String>,
    pub sdk_session_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: u32,
    pub last_user_message: Option<String>,
    pub last_assistant_message: Option<String>,
    pub last_message_preview: Option<String>,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub worktree_json: Option<String>,
    pub last_delivery_context_json: Option<String>,
    pub last_delivery_updated_at: Option<String>,
    pub default_list_hidden: bool,
    pub sort_updated_at_us: i64,
    pub search_text: String,
    /// Server-owned workspace membership: worktree threads map to their
    /// source workspace, implicit Garyx-managed thread workspaces to None.
    pub root_workspace_path: Option<String>,
    /// Server-owned provenance: "explicit" or "implicit".
    pub workspace_origin: Option<String>,
    pub projection_version: i64,
    pub projected_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadMetaDraft {
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub thread_label: Option<String>,
    pub agent_id: Option<String>,
    pub provider_type: Option<String>,
    pub provider_key: Option<String>,
    pub selected_model: Option<String>,
    pub selected_model_reasoning_effort: Option<String>,
    pub selected_model_service_tier: Option<String>,
    pub sdk_session_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: u32,
    pub last_user_message: Option<String>,
    pub last_assistant_message: Option<String>,
    pub last_message_preview: Option<String>,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub worktree_json: Option<String>,
    pub last_delivery_context_json: Option<String>,
    pub last_delivery_updated_at: Option<String>,
    pub default_list_hidden: bool,
    pub sort_updated_at_us: i64,
    pub search_text: String,
    pub root_workspace_path: Option<String>,
    pub workspace_origin: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadMetaProjectionDraft {
    pub thread_id: String,
    pub thread_meta: ThreadMetaDraft,
    pub channel_endpoints: Vec<KnownChannelEndpoint>,
}

pub(super) fn thread_meta_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ThreadMetaRecord> {
    Ok(ThreadMetaRecord {
        thread_id: row.get(0)?,
        workspace_dir: row.get(1)?,
        thread_type: row.get(2)?,
        thread_label: row.get(3)?,
        agent_id: row.get(4)?,
        provider_type: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        message_count: row.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
        last_user_message: row.get(9)?,
        last_assistant_message: row.get(10)?,
        last_message_preview: row.get(11)?,
        recent_run_id: row.get(12)?,
        active_run_id: row.get(13)?,
        worktree_json: row.get(14)?,
        last_delivery_context_json: row.get(15)?,
        last_delivery_updated_at: row.get(16)?,
        default_list_hidden: row.get::<_, i64>(17)? != 0,
        sort_updated_at_us: row.get(18)?,
        search_text: row.get(19)?,
        provider_key: row.get(20)?,
        selected_model: row.get(21)?,
        selected_model_reasoning_effort: row.get(22)?,
        selected_model_service_tier: row.get(23)?,
        sdk_session_id: row.get(24)?,
        projection_version: row.get(25)?,
        projected_at: row.get(26)?,
        root_workspace_path: row.get(27)?,
        workspace_origin: row.get(28)?,
    })
}

pub(super) fn replace_thread_meta_projection_tx(
    tx: &Transaction<'_>,
    draft: ThreadMetaProjectionDraft,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let thread_id = normalize_thread_id(&draft.thread_id)?;
    remove_thread_meta_projection_tx(tx, &thread_id)?;
    let mut thread_meta = draft.thread_meta;
    thread_meta.thread_id = thread_id.clone();
    upsert_thread_meta(tx, &thread_meta, recorded_at)?;
    for mut endpoint in draft.channel_endpoints {
        endpoint.thread_id = Some(thread_id.clone());
        upsert_thread_channel_endpoint(tx, &endpoint, recorded_at)?;
    }
    Ok(())
}

pub(super) fn remove_thread_meta_projection_tx(
    conn: &Connection,
    thread_id: &str,
) -> GaryxDbResult<usize> {
    let mut removed = 0usize;
    removed += conn.execute(
        "DELETE FROM thread_meta WHERE thread_id = ?1",
        params![thread_id],
    )?;
    removed += conn.execute(
        "DELETE FROM thread_channel_endpoints WHERE thread_id = ?1",
        params![thread_id],
    )?;
    Ok(removed)
}

pub(super) fn upsert_thread_meta(
    tx: &Transaction<'_>,
    meta: &ThreadMetaDraft,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let thread_id = normalize_thread_id(&meta.thread_id)?;
    let workspace_dir = normalize_optional(meta.workspace_dir.as_deref());
    let thread_type =
        normalize_optional(Some(&meta.thread_type)).unwrap_or_else(|| "chat".to_owned());
    let thread_label = normalize_optional(meta.thread_label.as_deref());
    let agent_id = normalize_optional(meta.agent_id.as_deref());
    let provider_type = normalize_optional(meta.provider_type.as_deref());
    let created_at = normalize_optional(meta.created_at.as_deref());
    let updated_at = normalize_optional(meta.updated_at.as_deref());
    let message_count = i64::from(meta.message_count);
    let last_user_message = normalize_optional(meta.last_user_message.as_deref());
    let last_assistant_message = normalize_optional(meta.last_assistant_message.as_deref());
    let last_message_preview = normalize_optional(meta.last_message_preview.as_deref());
    let recent_run_id = normalize_optional(meta.recent_run_id.as_deref());
    let active_run_id = normalize_optional(meta.active_run_id.as_deref());
    let worktree_json = normalize_optional(meta.worktree_json.as_deref());
    let last_delivery_context_json = normalize_optional(meta.last_delivery_context_json.as_deref());
    let last_delivery_updated_at = normalize_optional(meta.last_delivery_updated_at.as_deref());
    let default_list_hidden = if meta.default_list_hidden { 1 } else { 0 };
    let sort_updated_at_us = meta.sort_updated_at_us;
    let search_text = meta.search_text.clone();
    let provider_key = normalize_optional(meta.provider_key.as_deref());
    let selected_model = normalize_optional(meta.selected_model.as_deref());
    let selected_model_reasoning_effort =
        normalize_optional(meta.selected_model_reasoning_effort.as_deref());
    let selected_model_service_tier =
        normalize_optional(meta.selected_model_service_tier.as_deref());
    let sdk_session_id = normalize_optional(meta.sdk_session_id.as_deref());
    let root_workspace_path = normalize_optional(meta.root_workspace_path.as_deref());
    let workspace_origin = normalize_optional(meta.workspace_origin.as_deref());

    tx.execute(
        "INSERT INTO thread_meta (
            thread_id, workspace_dir, thread_type, thread_label, agent_id, provider_type,
            created_at, updated_at, message_count, last_user_message, last_assistant_message,
            last_message_preview, recent_run_id, active_run_id, worktree_json,
            last_delivery_context_json, last_delivery_updated_at, default_list_hidden,
            sort_updated_at_us, search_text,
            provider_key, selected_model, selected_model_reasoning_effort,
            selected_model_service_tier, sdk_session_id,
            projection_version, projected_at,
            root_workspace_path, workspace_origin
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)
         ON CONFLICT(thread_id) DO UPDATE SET
            workspace_dir = excluded.workspace_dir,
            thread_type = excluded.thread_type,
            thread_label = excluded.thread_label,
            agent_id = excluded.agent_id,
            provider_type = excluded.provider_type,
            provider_key = excluded.provider_key,
            selected_model = excluded.selected_model,
            selected_model_reasoning_effort = excluded.selected_model_reasoning_effort,
            selected_model_service_tier = excluded.selected_model_service_tier,
            sdk_session_id = excluded.sdk_session_id,
            created_at = excluded.created_at,
            updated_at = excluded.updated_at,
            message_count = excluded.message_count,
            last_user_message = excluded.last_user_message,
            last_assistant_message = excluded.last_assistant_message,
            last_message_preview = excluded.last_message_preview,
            recent_run_id = excluded.recent_run_id,
            active_run_id = excluded.active_run_id,
            worktree_json = excluded.worktree_json,
            last_delivery_context_json = excluded.last_delivery_context_json,
            last_delivery_updated_at = excluded.last_delivery_updated_at,
            default_list_hidden = excluded.default_list_hidden,
            sort_updated_at_us = excluded.sort_updated_at_us,
            search_text = excluded.search_text,
            root_workspace_path = excluded.root_workspace_path,
            workspace_origin = excluded.workspace_origin,
            projection_version = excluded.projection_version,
            projected_at = excluded.projected_at",
        params![
            thread_id,
            workspace_dir,
            thread_type,
            thread_label,
            agent_id,
            provider_type,
            created_at,
            updated_at,
            message_count,
            last_user_message,
            last_assistant_message,
            last_message_preview,
            recent_run_id,
            active_run_id,
            worktree_json,
            last_delivery_context_json,
            last_delivery_updated_at,
            default_list_hidden,
            sort_updated_at_us,
            search_text,
            provider_key,
            selected_model,
            selected_model_reasoning_effort,
            selected_model_service_tier,
            sdk_session_id,
            CURRENT_THREAD_META_PROJECTION_VERSION,
            recorded_at,
            root_workspace_path,
            workspace_origin,
        ],
    )?;
    Ok(())
}

impl GaryxDbService {
    pub fn count_thread_meta_projection_rows(&self) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM thread_meta) +
                (SELECT COUNT(*) FROM thread_channel_endpoints)",
            [],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn count_thread_meta_rows(&self) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM thread_meta", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn count_thread_meta_list(
        &self,
        include_hidden: bool,
        prefix: Option<&str>,
    ) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 = match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) if include_hidden => conn.query_row(
                "SELECT COUNT(*)
                 FROM thread_meta
                 WHERE substr(thread_id, 1, length(?1)) = ?1",
                params![prefix],
                |row| row.get(0),
            )?,
            Some(prefix) => conn.query_row(
                "SELECT COUNT(*)
                 FROM thread_meta
                 WHERE default_list_hidden = 0
                   AND substr(thread_id, 1, length(?1)) = ?1",
                params![prefix],
                |row| row.get(0),
            )?,
            None if include_hidden => {
                conn.query_row("SELECT COUNT(*) FROM thread_meta", [], |row| row.get(0))?
            }
            None => conn.query_row(
                "SELECT COUNT(*) FROM thread_meta WHERE default_list_hidden = 0",
                [],
                |row| row.get(0),
            )?,
        };
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn list_thread_meta_page(
        &self,
        limit: usize,
        offset: usize,
        include_hidden: bool,
        prefix: Option<&str>,
    ) -> GaryxDbResult<Vec<ThreadMetaRecord>> {
        let conn = self.read_conn()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset = i64::try_from(offset).unwrap_or(i64::MAX);
        let sql = "SELECT thread_id, workspace_dir, thread_type, thread_label, agent_id,
                          provider_type, created_at, updated_at, message_count,
                          last_user_message, last_assistant_message, last_message_preview,
                          recent_run_id, active_run_id, worktree_json,
                          last_delivery_context_json, last_delivery_updated_at,
                          default_list_hidden, sort_updated_at_us,
                          search_text, provider_key, selected_model,
                          selected_model_reasoning_effort, selected_model_service_tier,
                          sdk_session_id, projection_version, projected_at,
                          root_workspace_path, workspace_origin
                   FROM thread_meta";
        let order = " ORDER BY COALESCE(updated_at, projected_at) DESC, thread_id ASC
                      LIMIT ?1 OFFSET ?2";
        let mut records = Vec::new();
        match prefix.map(str::trim).filter(|value| !value.is_empty()) {
            Some(prefix) if include_hidden => {
                let mut stmt = conn.prepare(&format!(
                    "{sql} WHERE substr(thread_id, 1, length(?3)) = ?3{order}"
                ))?;
                let rows =
                    stmt.query_map(params![limit, offset, prefix], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
            Some(prefix) => {
                let mut stmt = conn.prepare(&format!(
                    "{sql} WHERE default_list_hidden = 0
                            AND substr(thread_id, 1, length(?3)) = ?3{order}"
                ))?;
                let rows =
                    stmt.query_map(params![limit, offset, prefix], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
            None if include_hidden => {
                let mut stmt = conn.prepare(&format!("{sql}{order}"))?;
                let rows = stmt.query_map(params![limit, offset], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
            None => {
                let mut stmt =
                    conn.prepare(&format!("{sql} WHERE default_list_hidden = 0{order}"))?;
                let rows = stmt.query_map(params![limit, offset], thread_meta_record_from_row)?;
                for row in rows {
                    records.push(row?);
                }
            }
        }
        Ok(records)
    }

    pub fn list_thread_meta(&self) -> GaryxDbResult<Vec<ThreadMetaRecord>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, workspace_dir, thread_type, thread_label, agent_id,
                    provider_type, created_at, updated_at, message_count,
                    last_user_message, last_assistant_message, last_message_preview,
                    recent_run_id, active_run_id, worktree_json,
                    last_delivery_context_json, last_delivery_updated_at,
                    default_list_hidden, sort_updated_at_us,
                    search_text, provider_key, selected_model,
                    selected_model_reasoning_effort, selected_model_service_tier,
                    sdk_session_id, projection_version, projected_at,
                    root_workspace_path, workspace_origin
             FROM thread_meta
             ORDER BY thread_id ASC",
        )?;
        let rows = stmt.query_map([], thread_meta_record_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    /// Test-fixture seeding only: production thread_meta rows derive in
    /// the same transaction as the record write
    /// (`write_thread_record_with_projections`).
    #[cfg(test)]
    pub fn replace_thread_meta_projection(
        &self,
        draft: ThreadMetaProjectionDraft,
    ) -> GaryxDbResult<()> {
        let recorded_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        replace_thread_meta_projection_tx(&tx, draft, &recorded_at)?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_thread_meta_projection(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let removed = remove_thread_meta_projection_tx(&conn, &thread_id)?;
        Ok(removed > 0)
    }
}
