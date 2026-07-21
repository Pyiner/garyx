//! Thread summary keyset pages over the `thread_meta` projection.

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadSummaryRow {
    pub thread_id: String,
    pub title: Option<String>,
    pub workspace_dir: Option<String>,
    pub thread_type: String,
    pub provider_type: Option<String>,
    pub agent_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: u32,
    pub last_user_message: Option<String>,
    pub last_assistant_message: Option<String>,
    pub last_message_preview: Option<String>,
    pub recent_run_id: Option<String>,
    pub active_run_id: Option<String>,
    pub worktree: Option<Value>,
    pub root_workspace_path: Option<String>,
    pub workspace_origin: Option<String>,
    #[serde(skip)]
    pub(crate) sort_updated_at_us: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadSummaryDbPage {
    pub records: Vec<ThreadSummaryRow>,
    pub has_more: bool,
    pub store_incarnation_id: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ThreadSummaryTaskFilter {
    #[default]
    Include,
    Exclude,
    Only,
}

impl ThreadSummaryTaskFilter {
    pub(crate) fn cursor_value(self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::Exclude => "exclude",
            Self::Only => "only",
        }
    }

    pub(super) fn page_sql(self, scoped: bool, has_query: bool, has_cursor: bool) -> &'static str {
        match self {
            Self::Include => thread_summary_include_sql(scoped, has_query, has_cursor),
            Self::Exclude => thread_summary_exclude_sql(scoped, has_query, has_cursor),
            Self::Only => thread_summary_only_sql(scoped, has_query, has_cursor),
        }
    }
}

macro_rules! thread_summary_sql {
    ("", "", $query:literal, $cursor:literal) => {
        thread_summary_sql!("idx_thread_meta_summary_visible", "", "", $query, $cursor)
    };
    ("\n   AND root_workspace_path = ?", "", $query:literal, $cursor:literal) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_root_workspace_visible",
            "\n   AND root_workspace_path = ?",
            "",
            $query,
            $cursor
        )
    };
    ("", "\n   AND thread_type <> 'task'", $query:literal, $cursor:literal) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_non_task",
            "",
            "\n   AND thread_type <> 'task'",
            $query,
            $cursor
        )
    };
    (
        "\n   AND root_workspace_path = ?",
        "\n   AND thread_type <> 'task'",
        $query:literal,
        $cursor:literal
    ) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_root_workspace_non_task",
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type <> 'task'",
            $query,
            $cursor
        )
    };
    ("", "\n   AND thread_type = 'task'", $query:literal, $cursor:literal) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_task",
            "",
            "\n   AND thread_type = 'task'",
            $query,
            $cursor
        )
    };
    (
        "\n   AND root_workspace_path = ?",
        "\n   AND thread_type = 'task'",
        $query:literal,
        $cursor:literal
    ) => {
        thread_summary_sql!(
            "idx_thread_meta_summary_root_workspace_task",
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type = 'task'",
            $query,
            $cursor
        )
    };
    ($index:literal, $scope:literal, $task:literal, $query:literal, $cursor:literal) => {
        concat!(
            "SELECT thread_id, thread_label, workspace_dir, thread_type, provider_type,\n",
            "       agent_id, created_at, updated_at, message_count, last_user_message,\n",
            "       last_assistant_message, last_message_preview, recent_run_id,\n",
            "       active_run_id, worktree_json, root_workspace_path,\n",
            "       workspace_origin, sort_updated_at_us\n",
            "  FROM thread_meta INDEXED BY ",
            $index,
            "\n",
            " WHERE default_list_hidden = 0",
            $scope,
            $task,
            $query,
            $cursor,
            "\n ORDER BY sort_updated_at_us DESC, thread_id DESC\n LIMIT ?"
        )
    };
}

pub(super) fn thread_summary_include_sql(
    scoped: bool,
    has_query: bool,
    has_cursor: bool,
) -> &'static str {
    match (scoped, has_query, has_cursor) {
        (false, false, false) => thread_summary_sql!("", "", "", ""),
        (false, false, true) => thread_summary_sql!(
            "",
            "",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (false, true, false) => {
            thread_summary_sql!("", "", "\n   AND instr(search_text, ?) > 0", "")
        }
        (false, true, true) => thread_summary_sql!(
            "",
            "",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, false, false) => {
            thread_summary_sql!("\n   AND root_workspace_path = ?", "", "", "")
        }
        (true, false, true) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, true, false) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (true, true, true) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
    }
}

pub(super) fn thread_summary_exclude_sql(
    scoped: bool,
    has_query: bool,
    has_cursor: bool,
) -> &'static str {
    match (scoped, has_query, has_cursor) {
        (false, false, false) => {
            thread_summary_sql!("", "\n   AND thread_type <> 'task'", "", "")
        }
        (false, false, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type <> 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (false, true, false) => thread_summary_sql!(
            "",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (false, true, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, false, false) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type <> 'task'",
            "",
            ""
        ),
        (true, false, true) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type <> 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, true, false) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (true, true, true) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type <> 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
    }
}

pub(super) fn thread_summary_only_sql(
    scoped: bool,
    has_query: bool,
    has_cursor: bool,
) -> &'static str {
    match (scoped, has_query, has_cursor) {
        (false, false, false) => {
            thread_summary_sql!("", "\n   AND thread_type = 'task'", "", "")
        }
        (false, false, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type = 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (false, true, false) => thread_summary_sql!(
            "",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (false, true, true) => thread_summary_sql!(
            "",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, false, false) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type = 'task'",
            "",
            ""
        ),
        (true, false, true) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type = 'task'",
            "",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
        (true, true, false) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            ""
        ),
        (true, true, true) => thread_summary_sql!(
            "\n   AND root_workspace_path = ?",
            "\n   AND thread_type = 'task'",
            "\n   AND instr(search_text, ?) > 0",
            "\n   AND (sort_updated_at_us, thread_id) < (?, ?)"
        ),
    }
}

pub(super) fn thread_summary_row_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ThreadSummaryRow> {
    let worktree_json: Option<String> = row.get(14)?;
    Ok(ThreadSummaryRow {
        thread_id: row.get(0)?,
        title: row.get(1)?,
        workspace_dir: row.get(2)?,
        thread_type: row.get(3)?,
        provider_type: row.get(4)?,
        agent_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        message_count: row.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
        last_user_message: row.get(9)?,
        last_assistant_message: row.get(10)?,
        last_message_preview: row.get(11)?,
        recent_run_id: row.get(12)?,
        active_run_id: row.get(13)?,
        worktree: worktree_json.and_then(|value| serde_json::from_str(&value).ok()),
        root_workspace_path: row.get(15)?,
        workspace_origin: row.get(16)?,
        sort_updated_at_us: row.get(17)?,
    })
}

impl GaryxDbService {
    pub(crate) fn list_thread_summaries_keyset_page(
        &self,
        filter: ThreadSummaryTaskFilter,
        root_workspace_path: Option<&str>,
        query: Option<&str>,
        limit: usize,
        before: Option<(i64, &str)>,
        expected_store_incarnation: Option<&str>,
    ) -> GaryxDbResult<ThreadSummaryDbPage> {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let store_incarnation_id = read_store_incarnation_id(&tx)?;
        if expected_store_incarnation.is_some_and(|expected| expected != store_incarnation_id) {
            return Err(GaryxDbError::BadRequest(
                "cursor does not belong to the current store incarnation".to_owned(),
            ));
        }

        let mut bind = Vec::with_capacity(6);
        if let Some(root_workspace_path) = root_workspace_path {
            bind.push(SqlValue::Text(root_workspace_path.to_owned()));
        }
        if let Some(query) = query {
            bind.push(SqlValue::Text(query.to_owned()));
        }
        if let Some((sort_updated_at_us, thread_id)) = before {
            bind.push(SqlValue::Integer(sort_updated_at_us));
            bind.push(SqlValue::Text(thread_id.to_owned()));
        }
        let fetch_limit = limit.saturating_add(1);
        bind.push(SqlValue::Integer(
            i64::try_from(fetch_limit).unwrap_or(i64::MAX),
        ));

        let sql = filter.page_sql(
            root_workspace_path.is_some(),
            query.is_some(),
            before.is_some(),
        );
        let mut stmt = tx.prepare(sql)?;
        let rows = stmt.query_map(params_from_iter(bind.iter()), thread_summary_row_from_row)?;
        let mut records = Vec::with_capacity(fetch_limit);
        for row in rows {
            records.push(row?);
        }
        drop(stmt);
        tx.commit()?;

        let has_more = records.len() > limit;
        if has_more {
            records.truncate(limit);
        }
        Ok(ThreadSummaryDbPage {
            records,
            has_more,
            store_incarnation_id,
        })
    }
}
