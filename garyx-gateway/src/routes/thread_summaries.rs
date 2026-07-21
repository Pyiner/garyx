//! Thread summary keyset pages: params, cursors, handler.

use super::*;

pub(super) const DEFAULT_THREAD_SUMMARY_LIMIT: usize = 30;

pub(super) const MAX_THREAD_SUMMARY_LIMIT: usize = 100;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListThreadSummariesParams {
    #[serde(default)]
    pub root_workspace_path: Option<String>,
    #[serde(default)]
    pub tasks: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
}

pub(super) fn parse_thread_summary_task_filter(
    value: Option<&str>,
) -> Result<ThreadSummaryTaskFilter, String> {
    match value {
        None | Some("include") => Ok(ThreadSummaryTaskFilter::Include),
        Some("exclude") => Ok(ThreadSummaryTaskFilter::Exclude),
        Some("only") => Ok(ThreadSummaryTaskFilter::Only),
        Some(_) => Err("tasks must be one of: include, exclude, only".to_owned()),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadSummariesCursor {
    pub(super) v: u8,
    pub(super) scope: String,
    pub(super) tasks: String,
    pub(super) q: Option<String>,
    pub(super) incarnation: String,
    pub(super) sort_key: i64,
    pub(super) thread_id: String,
}

pub(super) struct ParsedThreadSummariesParams {
    root_workspace_path: Option<String>,
    filter: ThreadSummaryTaskFilter,
    query: Option<String>,
    scope: String,
    limit: usize,
    cursor: Option<ThreadSummariesCursor>,
}

pub(super) fn thread_summary_scope(root_workspace_path: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root_workspace_path.unwrap_or_default().as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(super) fn decode_thread_summaries_cursor(raw: &str) -> Result<ThreadSummariesCursor, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "cursor must be an opaque thread-summaries cursor".to_owned())?;
    let cursor: ThreadSummariesCursor = serde_json::from_slice(&bytes)
        .map_err(|_| "cursor must be an opaque thread-summaries cursor".to_owned())?;
    if cursor.v != 1 {
        return Err("cursor version is not supported".to_owned());
    }
    if cursor.thread_id.trim().is_empty() {
        return Err("cursor must be an opaque thread-summaries cursor".to_owned());
    }
    Ok(cursor)
}

pub(super) fn encode_thread_summaries_cursor(
    scope: &str,
    filter: ThreadSummaryTaskFilter,
    query: Option<&str>,
    incarnation: &str,
    sort_key: i64,
    thread_id: &str,
) -> String {
    let encoded = serde_json::to_vec(&ThreadSummariesCursor {
        v: 1,
        scope: scope.to_owned(),
        tasks: filter.cursor_value().to_owned(),
        q: query.map(ToOwned::to_owned),
        incarnation: incarnation.to_owned(),
        sort_key,
        thread_id: thread_id.to_owned(),
    })
    .expect("thread-summaries cursor serialization is infallible");
    URL_SAFE_NO_PAD.encode(encoded)
}

pub(super) fn parse_thread_summaries_params(
    query: Result<Query<ListThreadSummariesParams>, axum::extract::rejection::QueryRejection>,
) -> Result<ParsedThreadSummariesParams, String> {
    let Query(params) = query.map_err(|error| error.to_string())?;
    let limit = match params.limit.as_deref() {
        None => DEFAULT_THREAD_SUMMARY_LIMIT,
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=MAX_THREAD_SUMMARY_LIMIT).contains(value))
            .ok_or_else(|| {
                format!("limit must be an integer from 1 through {MAX_THREAD_SUMMARY_LIMIT}")
            })?,
    };
    let root_workspace_path = params.root_workspace_path;
    if root_workspace_path
        .as_deref()
        .is_some_and(|root_workspace_path| !FsPath::new(root_workspace_path).is_absolute())
    {
        return Err("root_workspace_path must be an absolute path".to_owned());
    }
    let filter = parse_thread_summary_task_filter(params.tasks.as_deref())?;
    let query = params
        .q
        .as_deref()
        .map(str::trim)
        .map(normalize_for_search)
        .filter(|value| !value.is_empty());
    if query
        .as_deref()
        .is_some_and(|query| query.chars().count() > 100)
    {
        return Err(
            "q must contain at most 100 Unicode scalar values after normalization".to_owned(),
        );
    }
    let scope = thread_summary_scope(root_workspace_path.as_deref());
    let cursor = params
        .cursor
        .as_deref()
        .map(decode_thread_summaries_cursor)
        .transpose()?;
    if let Some(cursor) = &cursor {
        if cursor.scope != scope {
            return Err("cursor does not belong to the requested workspace scope".to_owned());
        }
        if cursor.tasks != filter.cursor_value() {
            return Err("cursor does not belong to the requested tasks filter".to_owned());
        }
        if cursor.q != query {
            return Err("cursor does not belong to the requested search query".to_owned());
        }
    }
    Ok(ParsedThreadSummariesParams {
        root_workspace_path,
        filter,
        query,
        scope,
        limit,
        cursor,
    })
}

pub(super) fn thread_summaries_invalid_request(
    message: impl Into<String>,
) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "kind": "garyx_api_error",
            "operation": "thread_summaries_list",
            "code": "invalid_request",
            "message": message.into(),
        })),
    )
        .into_response()
}

/// GET /api/thread-summaries - keyset-paged canonical thread summaries.
pub async fn list_thread_summaries(
    State(state): State<Arc<AppState>>,
    query: Result<Query<ListThreadSummariesParams>, axum::extract::rejection::QueryRejection>,
) -> impl IntoResponse {
    let params = match parse_thread_summaries_params(query) {
        Ok(params) => params,
        Err(message) => return thread_summaries_invalid_request(message),
    };
    let root_workspace_path = params.root_workspace_path.clone();
    let normalized_query = params.query.clone();
    let cursor_key = params
        .cursor
        .as_ref()
        .map(|cursor| (cursor.sort_key, cursor.thread_id.clone()));
    let cursor_incarnation = params
        .cursor
        .as_ref()
        .map(|cursor| cursor.incarnation.clone());
    let filter = params.filter;
    let limit = params.limit;
    let paged = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            db.list_thread_summaries_keyset_page(
                filter,
                root_workspace_path.as_deref(),
                normalized_query.as_deref(),
                limit,
                cursor_key
                    .as_ref()
                    .map(|(sort_key, thread_id)| (*sort_key, thread_id.as_str())),
                cursor_incarnation.as_deref(),
            )
        })
        .await;
    match paged {
        Ok(page) => {
            let next_cursor = page.has_more.then(|| {
                let last = page
                    .records
                    .last()
                    .expect("a positive summary page limit with has_more must return a row");
                encode_thread_summaries_cursor(
                    &params.scope,
                    params.filter,
                    params.query.as_deref(),
                    &page.store_incarnation_id,
                    last.sort_updated_at_us,
                    &last.thread_id,
                )
            });
            (
                StatusCode::OK,
                Json(json!({
                    "threads": page.records,
                    "next_cursor": next_cursor,
                    "has_more": page.has_more,
                    "store_incarnation_id": page.store_incarnation_id,
                    "server_boot_id": state.server_boot_id(),
                })),
            )
                .into_response()
        }
        Err(GaryxDbError::BadRequest(message)) => thread_summaries_invalid_request(message),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}
