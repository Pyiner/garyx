//! `thread_channel_endpoints` projection reads and writes.

use super::*;

pub(super) fn upsert_thread_channel_endpoint(
    tx: &Transaction<'_>,
    endpoint: &KnownChannelEndpoint,
    recorded_at: &str,
) -> GaryxDbResult<()> {
    let endpoint_key = normalize_required("endpoint_key", &endpoint.endpoint_key)?;
    let channel = normalize_required("channel", &endpoint.channel)?;
    let account_id = normalize_required("account_id", &endpoint.account_id)?;
    let binding_key = endpoint.binding_key.trim().to_owned();
    let chat_id = endpoint.chat_id.trim().to_owned();
    let delivery_target_type = normalize_optional(Some(&endpoint.delivery_target_type))
        .unwrap_or_else(|| "chat_id".to_owned());
    let delivery_target_id = endpoint.delivery_target_id.trim().to_owned();
    let display_label = endpoint.display_label.trim().to_owned();
    let thread_id = normalize_optional(endpoint.thread_id.as_deref());
    let thread_label = normalize_optional(endpoint.thread_label.as_deref());
    let workspace_dir = normalize_optional(endpoint.workspace_dir.as_deref());
    let thread_updated_at = normalize_optional(endpoint.thread_updated_at.as_deref());
    let last_inbound_at = normalize_optional(endpoint.last_inbound_at.as_deref());
    let last_delivery_at = normalize_optional(endpoint.last_delivery_at.as_deref());

    tx.execute(
        "INSERT INTO thread_channel_endpoints (
            endpoint_key, channel, account_id, binding_key, chat_id,
            delivery_target_type, delivery_target_id, display_label,
            thread_id, thread_label, workspace_dir, thread_updated_at,
            last_inbound_at, last_delivery_at, projected_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(endpoint_key) DO UPDATE SET
            channel = excluded.channel,
            account_id = excluded.account_id,
            binding_key = excluded.binding_key,
            chat_id = excluded.chat_id,
            delivery_target_type = excluded.delivery_target_type,
            delivery_target_id = excluded.delivery_target_id,
            display_label = excluded.display_label,
            thread_id = excluded.thread_id,
            thread_label = excluded.thread_label,
            workspace_dir = excluded.workspace_dir,
            thread_updated_at = excluded.thread_updated_at,
            last_inbound_at = excluded.last_inbound_at,
            last_delivery_at = excluded.last_delivery_at,
            projected_at = excluded.projected_at",
        params![
            endpoint_key,
            channel,
            account_id,
            binding_key,
            chat_id,
            delivery_target_type,
            delivery_target_id,
            display_label,
            thread_id,
            thread_label,
            workspace_dir,
            thread_updated_at,
            last_inbound_at,
            last_delivery_at,
            recorded_at,
        ],
    )?;
    Ok(())
}

pub(super) fn known_channel_endpoint_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<KnownChannelEndpoint> {
    Ok(KnownChannelEndpoint {
        endpoint_key: row.get(0)?,
        channel: row.get(1)?,
        account_id: row.get(2)?,
        binding_key: row.get(3)?,
        chat_id: row.get(4)?,
        delivery_target_type: row.get(5)?,
        delivery_target_id: row.get(6)?,
        display_label: row.get(7)?,
        thread_id: row.get(8)?,
        thread_label: row.get(9)?,
        workspace_dir: row.get(10)?,
        thread_updated_at: row.get(11)?,
        last_inbound_at: row.get(12)?,
        last_delivery_at: row.get(13)?,
    })
}

impl GaryxDbService {
    pub fn count_thread_channel_endpoints(&self) -> GaryxDbResult<usize> {
        let conn = self.read_conn()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM thread_channel_endpoints", [], |row| {
                row.get(0)
            })?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub fn list_thread_channel_endpoints(&self) -> GaryxDbResult<Vec<KnownChannelEndpoint>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT endpoint_key, channel, account_id, binding_key, chat_id,
                    delivery_target_type, delivery_target_id, display_label,
                    thread_id, thread_label, workspace_dir, thread_updated_at,
                    last_inbound_at, last_delivery_at
             FROM thread_channel_endpoints
             ORDER BY endpoint_key ASC",
        )?;
        let rows = stmt.query_map([], known_channel_endpoint_from_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub(crate) fn get_thread_channel_endpoint(
        &self,
        endpoint_key: &str,
    ) -> GaryxDbResult<Option<KnownChannelEndpoint>> {
        let endpoint_key = normalize_required("endpoint_key", endpoint_key)?;
        let conn = self.read_conn()?;
        Ok(conn
            .query_row(
                "SELECT endpoint_key, channel, account_id, binding_key, chat_id,
                        delivery_target_type, delivery_target_id, display_label,
                        thread_id, thread_label, workspace_dir, thread_updated_at,
                        last_inbound_at, last_delivery_at
                   FROM thread_channel_endpoints
                  WHERE endpoint_key = ?1",
                params![endpoint_key],
                known_channel_endpoint_from_row,
            )
            .optional()?)
    }

    /// Per-thread persisted delivery contexts from the thread_meta projection.
    pub fn list_thread_delivery_contexts(
        &self,
    ) -> GaryxDbResult<Vec<(String, String, Option<String>)>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, last_delivery_context_json, last_delivery_updated_at
             FROM thread_meta
             WHERE last_delivery_context_json IS NOT NULL
             ORDER BY thread_id ASC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }
}
