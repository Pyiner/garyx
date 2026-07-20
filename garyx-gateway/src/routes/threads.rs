//! Thread list/get/create/update handlers and their parameter plumbing.

use super::*;

pub(super) const DEFAULT_THREAD_LIMIT: usize = 100;

pub(super) const MAX_THREAD_LIMIT: usize = 1000;

pub(super) const DEFAULT_RECENT_THREAD_LIMIT: usize = 30;

pub(super) const MAX_RECENT_THREAD_LIMIT: usize = 200;

#[derive(Deserialize)]
pub struct ListThreadsParams {
    /// Maximum number of threads to return.
    #[serde(default = "default_thread_limit")]
    pub limit: usize,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: usize,
    /// Optional prefix filter for thread ids.
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub include_hidden: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRecentThreadsParams {
    /// Maximum number of recent threads to return.
    #[serde(default)]
    pub limit: Option<String>,
    /// Task membership filter. Omitting it preserves the existing unfiltered
    /// recent-thread response.
    #[serde(default)]
    pub tasks: Option<String>,
    /// Opaque filter-bound keyset cursor returned by the preceding page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Deserialize)]
pub struct ThreadLogParams {
    #[serde(default)]
    pub cursor: Option<u64>,
}

pub(super) fn default_thread_limit() -> usize {
    DEFAULT_THREAD_LIMIT
}

pub(super) fn parse_recent_thread_task_filter(
    value: Option<&str>,
) -> Result<RecentThreadTaskFilter, String> {
    match value {
        None | Some("include") => Ok(RecentThreadTaskFilter::Include),
        Some("exclude") => Ok(RecentThreadTaskFilter::Exclude),
        Some("only") => Ok(RecentThreadTaskFilter::Only),
        Some(_) => Err("tasks must be one of: include, exclude, only".to_owned()),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecentThreadsCursor {
    v: u8,
    filter: String,
    activity_seq: i64,
}

pub(super) fn decode_recent_threads_cursor(
    raw: &str,
    filter: RecentThreadTaskFilter,
) -> Result<i64, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "cursor must be an opaque recent-threads cursor".to_owned())?;
    let cursor: RecentThreadsCursor = serde_json::from_slice(&bytes)
        .map_err(|_| "cursor must be an opaque recent-threads cursor".to_owned())?;
    if cursor.v != 1 {
        return Err("cursor version is not supported".to_owned());
    }
    if cursor.filter != filter.cursor_value() {
        return Err("cursor does not belong to the requested tasks filter".to_owned());
    }
    if !(0..MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE).contains(&cursor.activity_seq) {
        return Err("cursor activity_seq is outside the supported range".to_owned());
    }
    Ok(cursor.activity_seq)
}

pub(super) fn encode_recent_threads_cursor(
    filter: RecentThreadTaskFilter,
    activity_seq: i64,
) -> String {
    let encoded = serde_json::to_vec(&RecentThreadsCursor {
        v: 1,
        filter: filter.cursor_value().to_owned(),
        activity_seq,
    })
    .expect("recent cursor serialization is infallible");
    URL_SAFE_NO_PAD.encode(encoded)
}

pub(super) fn parse_recent_threads_params(
    query: Result<Query<ListRecentThreadsParams>, axum::extract::rejection::QueryRejection>,
) -> Result<(RecentThreadTaskFilter, usize, Option<i64>), String> {
    let Query(params) = query.map_err(|error| error.to_string())?;
    let limit = match params.limit.as_deref() {
        None => DEFAULT_RECENT_THREAD_LIMIT,
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=MAX_RECENT_THREAD_LIMIT).contains(value))
            .ok_or_else(|| {
                format!("limit must be an integer from 1 through {MAX_RECENT_THREAD_LIMIT}")
            })?,
    };
    let filter = parse_recent_thread_task_filter(params.tasks.as_deref())?;
    let before_activity_seq = params
        .cursor
        .as_deref()
        .map(|cursor| decode_recent_threads_cursor(cursor, filter))
        .transpose()?;
    Ok((filter, limit, before_activity_seq))
}

pub(super) fn parse_sdk_session_provider_hint(
    value: Option<&str>,
) -> Result<Option<ProviderType>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    ProviderType::from_slug(&value.to_ascii_lowercase())
        .map(Some)
        .ok_or_else(|| {
            format!(
                "Unsupported sdkSessionProviderHint '{value}'. Use claude, codex, traex, or antigravity."
            )
        })
}

pub(super) fn provider_hint_label(value: &ProviderType) -> &'static str {
    match value {
        ProviderType::ClaudeCode => "Claude",
        ProviderType::CodexAppServer => "Codex",
        ProviderType::Traex => "Traex",
        ProviderType::AntigravityCli => "Antigravity",
    }
}

pub(super) fn is_resume_provider(value: &ProviderType) -> bool {
    // Traex is intentionally excluded: garyx does not support disk-based session
    // recovery / fork-from-session for TRAE CLI (its sessions live under
    // ~/.trae and are not wired into the provider session locator).
    matches!(
        value,
        ProviderType::ClaudeCode | ProviderType::CodexAppServer
    )
}

pub(super) fn provider_type_from_thread_value(thread_data: &Value) -> Option<ProviderType> {
    thread_data
        .get("provider_type")
        .cloned()
        .and_then(|value| serde_json::from_value::<ProviderType>(value).ok())
}

pub(super) fn non_empty_json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn fork_source_sdk_session_id(
    thread_data: &Value,
    provider_type: &ProviderType,
) -> Option<String> {
    if provider_type_from_thread_value(thread_data)
        .as_ref()
        .is_some_and(|persisted_provider_type| persisted_provider_type == provider_type)
        && let Some(session_id) = non_empty_json_string(thread_data.get("sdk_session_id"))
    {
        return Some(session_id);
    }

    let provider_scoped_session_ids = thread_data
        .get("provider_sdk_session_ids")
        .and_then(Value::as_object)?;
    if provider_scoped_session_ids.len() == 1 {
        return provider_scoped_session_ids
            .values()
            .next()
            .and_then(|value| non_empty_json_string(Some(value)));
    }
    None
}

pub(super) async fn seed_imported_thread_history(
    state: &Arc<AppState>,
    thread_id: &str,
    thread_data: &mut Value,
    messages: &[Value],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }
    let observed = thread_data.clone();

    let append_result = state
        .threads
        .history
        .transcript_store()
        .rewrite_from_messages(thread_id, messages)
        .await
        .map_err(|error| format!("failed to import local provider session history: {error}"))?;

    let Some(object) = thread_data.as_object_mut() else {
        return Err(format!("thread payload is not an object: {thread_id}"));
    };

    // The transcript is the only imported-content copy (#TASK-1864
    // batch 1c): no record `messages` snapshot is seeded. The write-time
    // preview fields are derived from the imported content directly.
    for role in ["user", "assistant"] {
        if let Some(field) = garyx_models::message_preview::preview_field_for_role(role)
            && let Some(preview) =
                garyx_models::message_preview::last_message_preview_for_role(messages.iter(), role)
        {
            object.insert(field.to_owned(), Value::String(preview));
        }
    }
    object.insert(
        "message_count".to_owned(),
        Value::Number(serde_json::Number::from(
            append_result.total_messages as u64,
        )),
    );

    let history = object
        .entry("history".to_owned())
        .or_insert_with(|| json!({}));
    if !history.is_object() {
        *history = json!({});
    }
    let history_object = history.as_object_mut().expect("history must be object");

    history_object.insert(
        "source".to_owned(),
        Value::String("transcript_v1".to_owned()),
    );
    if let Some(path) = state
        .threads
        .history
        .transcript_store()
        .transcript_path(thread_id)
    {
        history_object.insert(
            "transcript_file".to_owned(),
            Value::String(path.display().to_string()),
        );
    }
    history_object.insert(
        "message_count".to_owned(),
        Value::Number(serde_json::Number::from(
            append_result.total_messages as u64,
        )),
    );
    history_object.insert(
        "snapshot_limit".to_owned(),
        Value::Number(serde_json::Number::from(
            garyx_router::DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT as u64,
        )),
    );
    history_object.insert(
        "snapshot_truncated".to_owned(),
        Value::Bool(
            append_result.total_messages > garyx_router::DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT,
        ),
    );
    match append_result.last_message_at {
        Some(last_message_at) if !last_message_at.trim().is_empty() => {
            history_object.insert("last_message_at".to_owned(), Value::String(last_message_at));
        }
        _ => {
            history_object.remove("last_message_at");
        }
    }
    history_object.insert(
        "recent_committed_run_ids".to_owned(),
        Value::Array(Vec::new()),
    );

    object.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
    );
    let patch = ThreadRecordPatch::from_diff(
        &observed,
        thread_data,
        &[
            "last_user_preview",
            "last_assistant_preview",
            "message_count",
            "history",
            "updated_at",
        ],
    )
    .map_err(|error| error.to_string())?;
    state
        .threads
        .thread_store
        .patch(thread_id, patch)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThreadBody {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default, alias = "workspace_mode")]
    pub workspace_mode: WorkspaceMode,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
    /// Agent ID.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Optional per-thread model override; wins over the agent's configured model.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional per-thread reasoning/thinking level override.
    #[serde(default)]
    pub model_reasoning_effort: Option<String>,
    /// Optional per-thread service tier override.
    #[serde(default)]
    pub model_service_tier: Option<String>,
    /// Optional provider-native session id to resume from on the first run.
    #[serde(default, alias = "sessionId")]
    pub sdk_session_id: Option<String>,
    /// Optional provider hint for sdkSessionId. Supported values: claude, codex.
    #[serde(default)]
    pub sdk_session_provider_hint: Option<String>,
    /// Optional Garyx thread id to fork from using the provider-native session fork.
    #[serde(default)]
    pub fork_from_thread_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProviderSessionsParams {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// GET /api/provider-sessions/recent - list recent local provider-native sessions
pub async fn list_recent_provider_sessions(
    Query(params): Query<RecentProviderSessionsParams>,
) -> impl IntoResponse {
    let provider_hint = match parse_sdk_session_provider_hint(params.provider.as_deref()) {
        Ok(value) => value,
        Err(error) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
    };
    if let Some(provider_hint) = provider_hint.as_ref()
        && !is_resume_provider(provider_hint)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "provider must be claude or codex"
            })),
        );
    }
    let limit = params.limit.unwrap_or(10).clamp(1, 50);
    let sessions = list_recent_local_provider_sessions(provider_hint, limit);
    (StatusCode::OK, Json(json!({ "sessions": sessions })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadBody {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    /// Optional per-thread model override. An empty string clears the override.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional per-thread reasoning/thinking level override. An empty string clears it.
    #[serde(default)]
    pub model_reasoning_effort: Option<String>,
    /// Optional per-thread service tier override. An empty string clears it.
    #[serde(default)]
    pub model_service_tier: Option<String>,
}

/// Write one thread runtime cell (single-cell semantics): `body` values
/// rewrite the cell key that the run path and runtime summary read, an empty
/// string empties the cell so provider/agent defaults apply again, and any
/// legacy dual-track override key is migrated away (deleted) whenever the
/// cell is touched.
pub(super) fn apply_thread_metadata_cell(
    data: &mut Value,
    cell_key: &str,
    legacy_override_key: &str,
    input: &Option<String>,
) -> bool {
    let Some(input) = input.as_deref() else {
        return false;
    };
    let Some(obj) = data.as_object_mut() else {
        return false;
    };
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return obj
            .get_mut("metadata")
            .and_then(Value::as_object_mut)
            .map(|metadata| {
                let removed_cell = metadata.remove(cell_key).is_some();
                let removed_legacy = metadata.remove(legacy_override_key).is_some();
                removed_cell || removed_legacy
            })
            .unwrap_or(false);
    }

    if !obj.get("metadata").is_some_and(Value::is_object) {
        obj.insert("metadata".to_owned(), Value::Object(Map::new()));
    }
    let Some(metadata) = obj.get_mut("metadata").and_then(Value::as_object_mut) else {
        return false;
    };
    let removed_legacy = metadata.remove(legacy_override_key).is_some();
    let next = Value::String(trimmed.to_owned());
    if !removed_legacy && metadata.get(cell_key) == Some(&next) {
        return false;
    }
    metadata.insert(cell_key.to_owned(), next);
    true
}

pub(super) fn apply_thread_runtime_cells(data: &mut Value, body: &UpdateThreadBody) -> bool {
    let mut changed = false;
    changed |= apply_thread_metadata_cell(
        data,
        MODEL_METADATA_KEY,
        MODEL_OVERRIDE_METADATA_KEY,
        &body.model,
    );
    changed |= apply_thread_metadata_cell(
        data,
        MODEL_REASONING_EFFORT_METADATA_KEY,
        MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
        &body.model_reasoning_effort,
    );
    changed |= apply_thread_metadata_cell(
        data,
        MODEL_SERVICE_TIER_METADATA_KEY,
        MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY,
        &body.model_service_tier,
    );
    changed
}

pub(super) fn last_message_preview(data: &Value, role: &str) -> Option<String> {
    // Write-time preview fields are the source (#TASK-1864 batch 1).
    if let Some(preview) = garyx_models::message_preview::preview_field_for_role(role)
        .and_then(|field| data.get(field))
        .and_then(Value::as_str)
    {
        return Some(preview.to_owned());
    }
    None
}

pub(crate) fn thread_summary(thread_id: &str, data: &Value) -> Value {
    let message_count = history_message_count(data);
    let label = data.get("label").cloned().unwrap_or(Value::Null);
    let updated_at = data.get("updated_at").cloned().unwrap_or(Value::Null);
    let created_at = data.get("created_at").cloned().unwrap_or(Value::Null);
    let workspace_dir = workspace_dir_from_value(data)
        .map(Value::String)
        .unwrap_or(Value::Null);
    let channel_bindings = serde_json::to_value(bindings_from_value(data))
        .unwrap_or_else(|_| Value::Array(Vec::new()));
    let agent_id = data.get("agent_id").cloned().unwrap_or(Value::Null);
    let provider_type = data.get("provider_type").cloned().unwrap_or(Value::Null);
    let worktree = data.get("worktree").cloned().unwrap_or(Value::Null);
    let recent_run_id = data
        .get("history")
        .and_then(|history| history.get("recent_committed_run_ids"))
        .and_then(Value::as_array)
        .and_then(|entries| entries.last())
        .cloned()
        .unwrap_or(Value::Null);
    let active_run_id = Value::Null;

    json!({
        "thread_id": thread_id,
        "thread_key": thread_id,
        "thread_type": thread_summary_type_from_record(data),
        "label": label,
        "workspace_dir": workspace_dir,
        "channel_bindings": channel_bindings,
        "updated_at": updated_at,
        "created_at": created_at,
        "message_count": message_count,
        "last_user_message": last_message_preview(data, "user"),
        "last_assistant_message": last_message_preview(data, "assistant"),
        "agent_id": agent_id,
        "provider_type": provider_type,
        "worktree": worktree,
        "recent_run_id": recent_run_id,
        "active_run_id": active_run_id,
    })
}

pub(super) fn thread_summary_from_meta(record: &ThreadMetaRecord) -> Value {
    let worktree = record
        .worktree_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .unwrap_or(Value::Null);
    json!({
        "thread_id": record.thread_id.as_str(),
        "thread_key": record.thread_id.as_str(),
        "thread_type": record.thread_type.as_str(),
        "label": record.thread_label.as_deref(),
        "workspace_dir": record.workspace_dir.as_deref(),
        "channel_bindings": [],
        "updated_at": record.updated_at.as_deref(),
        "created_at": record.created_at.as_deref(),
        "message_count": record.message_count,
        "last_user_message": record.last_user_message.as_deref(),
        "last_assistant_message": record.last_assistant_message.as_deref(),
        "last_message_preview": record.last_message_preview.as_deref(),
        "agent_id": record.agent_id.as_deref(),
        "provider_type": record.provider_type.as_deref(),
        "worktree": worktree,
        "recent_run_id": record.recent_run_id.as_deref(),
        "active_run_id": record.active_run_id.as_deref(),
    })
}

pub(super) const RECENT_THREADS_LIST_OPERATION: &str = "recent_threads_list";

pub(super) async fn recent_threads_payload(
    state: &Arc<AppState>,
    records: &[RecentThreadRecord],
    filter: RecentThreadTaskFilter,
    limit: usize,
    total: usize,
    has_more: bool,
    store_incarnation_id: &str,
) -> Value {
    let threads = recent_thread_values(state, records).await;
    let next_cursor = has_more.then(|| {
        let last = records
            .last()
            .expect("a positive page limit with has_more must return a row");
        encode_recent_threads_cursor(filter, last.activity_seq)
    });
    json!({
        "store_incarnation_id": store_incarnation_id,
        "server_boot_id": state.server_boot_id(),
        "threads": threads,
        "count": records.len(),
        "limit": limit,
        "total": total,
        "has_more": has_more,
        "next_cursor": next_cursor,
    })
}

pub(super) fn recent_threads_invalid_request(
    message: impl Into<String>,
) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "kind": "garyx_api_error",
            "operation": RECENT_THREADS_LIST_OPERATION,
            "code": "invalid_request",
            "message": message.into(),
        })),
    )
        .into_response()
}

pub(super) async fn recent_thread_values(
    state: &Arc<AppState>,
    records: &[RecentThreadRecord],
) -> Vec<Value> {
    let mut threads = Vec::with_capacity(records.len());
    let catalog = AgentCatalogSnapshot::load(state).await;
    for record in records {
        let mut thread = serde_json::to_value(record).unwrap_or(Value::Null);
        attach_thread_runtime_summary_with_catalog(state, &record.thread_id, &mut thread, &catalog)
            .await;
        threads.push(thread);
    }
    threads
}

pub(super) async fn thread_metadata_response(
    state: &Arc<AppState>,
    thread_id: &str,
    data: &Value,
) -> Value {
    let mut value = data.clone();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("thread_mode");
        obj.entry("thread_id".to_owned())
            .or_insert_with(|| Value::String(thread_id.to_owned()));
        obj.entry("thread_key".to_owned())
            .or_insert_with(|| Value::String(thread_id.to_owned()));
        obj.insert(
            "thread_type".to_owned(),
            Value::String(thread_summary_type_from_record(data)),
        );
        obj.insert(
            "thread_runtime".to_owned(),
            build_thread_runtime_summary(state, Some(data)).await,
        );
    }
    value
}

pub(super) async fn attach_thread_runtime_summary_with_catalog(
    state: &Arc<AppState>,
    thread_id: &str,
    summary: &mut Value,
    catalog: &AgentCatalogSnapshot,
) {
    let thread_value = state.threads.thread_store.get_logged(thread_id).await;
    if let Some(obj) = summary.as_object_mut() {
        obj.insert(
            "thread_runtime".to_owned(),
            build_thread_runtime_summary_with_catalog(state, thread_value.as_ref(), catalog),
        );
    }
}

/// GET /api/threads - list threads with filtering and pagination.
pub async fn list_threads(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListThreadsParams>,
) -> impl IntoResponse {
    let limit = params.limit.min(MAX_THREAD_LIMIT);
    let include_hidden = params.include_hidden;
    let prefix = params.prefix.clone();
    let requested_offset = params.offset;
    // Count + page in one blocking hop: SQLite work must not hold a runtime
    // worker (#TASK-1829 batch 3).
    let paged = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            let total = db.count_thread_meta_list(include_hidden, prefix.as_deref())?;
            let offset = requested_offset.min(total);
            let records =
                db.list_thread_meta_page(limit, offset, include_hidden, prefix.as_deref())?;
            Ok((total, offset, records))
        })
        .await;
    let (total, offset, records) = match paged {
        Ok(paged) => paged,
        Err(error) => return garyx_db_error_response(error).into_response(),
    };
    let catalog = AgentCatalogSnapshot::load(&state).await;
    let mut page = Vec::with_capacity(records.len());
    for record in &records {
        let mut summary = thread_summary_from_meta(record);
        if let Some(obj) = summary.as_object_mut() {
            obj.insert(
                "thread_runtime".to_owned(),
                build_thread_runtime_summary_from_meta(&state, record, &catalog),
            );
        }
        page.push(summary);
    }
    let count = page.len();

    (
        StatusCode::OK,
        Json(json!({
        "threads": page,
        "count": count,
        "total": total,
        "limit": limit,
        "offset": offset,
        })),
    )
        .into_response()
}

/// GET /api/recent-threads - list recently active threads for compact clients.
pub async fn list_recent_threads(
    State(state): State<Arc<AppState>>,
    query: Result<Query<ListRecentThreadsParams>, axum::extract::rejection::QueryRejection>,
) -> impl IntoResponse {
    let (filter, limit, before_activity_seq) = match parse_recent_threads_params(query) {
        Ok(params) => params,
        Err(message) => return recent_threads_invalid_request(message),
    };
    let paged = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            let page = db.list_recent_threads_keyset_page(filter, limit, before_activity_seq)?;
            let store_incarnation_id = db.store_incarnation_id()?;
            Ok((page, store_incarnation_id))
        })
        .await;
    match paged {
        Ok((page, store_incarnation_id)) => (
            StatusCode::OK,
            Json(
                recent_threads_payload(
                    &state,
                    &page.records,
                    filter,
                    limit,
                    page.total,
                    page.has_more,
                    &store_incarnation_id,
                )
                .await,
            ),
        )
            .into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/threads/:key - get thread metadata
pub async fn get_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            );
        }
        Err(response) => return response,
    };
    match state.threads.thread_store.get(&thread_id).await {
        Ok(Some(data)) => (
            StatusCode::OK,
            Json(thread_metadata_response(&state, &thread_id, &data).await),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "thread not found"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        ),
    }
}

/// GET /api/threads/:key/logs - get full or incremental thread log content
pub async fn get_thread_logs(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Query(params): Query<ThreadLogParams>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            );
        }
        Err(response) => return response,
    };

    match state
        .ops
        .thread_logs
        .read_chunk(&thread_id, params.cursor)
        .await
    {
        Ok(chunk) => (StatusCode::OK, Json(json!(chunk))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        ),
    }
}

/// POST /api/threads - create a canonical thread
pub async fn create_thread(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateThreadBody>,
) -> impl IntoResponse {
    let requested_session_id = body
        .sdk_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let requested_fork_thread_key = body
        .fork_from_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested_session_id.is_some() && requested_fork_thread_key.is_some() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "sdkSessionId resume cannot be combined with forkFromThreadId"
            })),
        );
    }
    if requested_session_id.is_some() && body.workspace_mode.is_worktree() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "workspaceMode=worktree cannot be combined with sdkSessionId resume"
            })),
        );
    }
    if requested_fork_thread_key.is_some() && body.workspace_mode.is_worktree() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "workspaceMode=worktree cannot be combined with forkFromThreadId"
            })),
        );
    }
    let requested_session_provider_hint =
        match parse_sdk_session_provider_hint(body.sdk_session_provider_hint.as_deref()) {
            Ok(value) => value,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": error })));
            }
        };
    let recovered_session = match requested_session_id {
        Some(session_id) => match recover_local_provider_session(
            session_id,
            requested_session_provider_hint.clone(),
        ) {
            Ok(Some(recovered)) => Some(recovered),
            Ok(None) => {
                let provider_label = requested_session_provider_hint
                    .as_ref()
                    .map(provider_hint_label)
                    .unwrap_or("Claude or Codex");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": format!(
                            "No local {provider_label} session was found for session id '{session_id}'. Resume must start from an existing local {provider_label} session on this Mac."
                        )
                    })),
                );
            }
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": error })));
            }
        },
        None => None,
    };

    let fork_source = match requested_fork_thread_key {
        Some(source_key) => {
            let source_thread_id = match ensure_existing_thread_id(&state, source_key).await {
                Ok(Some(source_thread_id)) => source_thread_id,
                Ok(None) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "fork source thread not found"})),
                    );
                }
                Err(response) => return response,
            };
            // NotFound stays a 400 claim; a backend failure must not
            // masquerade as "fork source thread not found" (#TASK-2130).
            let source_thread_data = match state.threads.thread_store.get(&source_thread_id).await {
                Ok(Some(source_thread_data)) => source_thread_data,
                Ok(None) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "fork source thread not found"})),
                    );
                }
                Err(error) => return thread_store_error_response(&error),
            };
            let Some(provider_type) = provider_type_from_thread_value(&source_thread_data) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "fork source thread has no provider type"})),
                );
            };
            if !is_resume_provider(&provider_type) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "forkFromThreadId is only supported for Claude or Codex provider sessions"
                    })),
                );
            }
            let Some(sdk_session_id) =
                fork_source_sdk_session_id(&source_thread_data, &provider_type)
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "fork source thread has no provider session id yet"
                    })),
                );
            };
            Some((
                source_thread_id,
                source_thread_data,
                provider_type,
                sdk_session_id,
            ))
        }
        None => None,
    };

    let mut metadata = body.metadata.clone();
    garyx_models::strip_server_owned_agent_metadata(&mut metadata);
    // Seed the thread's single runtime cells (metadata.model & co.), the keys
    // the run path and runtime summary read. The legacy dual-track
    // `*_override` keys are read-compat only and are never written anymore.
    for (cell_key, requested) in [
        (MODEL_METADATA_KEY, body.model.as_deref()),
        (
            MODEL_REASONING_EFFORT_METADATA_KEY,
            body.model_reasoning_effort.as_deref(),
        ),
        (
            MODEL_SERVICE_TIER_METADATA_KEY,
            body.model_service_tier.as_deref(),
        ),
    ] {
        if let Some(value) = requested.map(str::trim).filter(|value| !value.is_empty()) {
            metadata.insert(cell_key.to_owned(), Value::String(value.to_owned()));
        }
    }
    if let Some((source_thread_id, _source_thread_data, provider_type, sdk_session_id)) =
        fork_source.as_ref()
    {
        metadata.insert(
            FORK_FROM_THREAD_ID_METADATA_KEY.to_owned(),
            Value::String(source_thread_id.clone()),
        );
        metadata.insert(
            FORK_FROM_SDK_SESSION_ID_METADATA_KEY.to_owned(),
            Value::String(sdk_session_id.clone()),
        );
        metadata.insert(
            FORK_FROM_PROVIDER_TYPE_METADATA_KEY.to_owned(),
            serde_json::to_value(provider_type).unwrap_or(Value::Null),
        );
        metadata.insert(SDK_SESSION_FORK_METADATA_KEY.to_owned(), Value::Bool(true));
    }

    let options = ThreadEnsureOptions {
        label: body.label.clone(),
        workspace_dir: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.workspace_dir.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .and_then(|(_, source_thread_data, _, _)| {
                        workspace_dir_from_value(source_thread_data)
                    })
            })
            .or_else(|| body.workspace_dir.clone()),
        workspace_mode: body.workspace_mode,
        worktree_base_dir: Some(worktree_base_dir_for_config(&state.config_snapshot())),
        agent_id: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.agent_id.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .and_then(|(_, source_thread_data, _, _)| {
                        source_thread_data
                            .get("agent_id")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                    })
            })
            .or_else(|| body.agent_id.clone()),
        metadata,
        provider_type: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.provider_type.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .map(|(_, _, provider_type, _)| provider_type.clone())
            }),
        sdk_session_id: body.sdk_session_id.clone(),
        thread_kind: None,
        origin_channel: None,
        origin_account_id: None,
        origin_from_id: None,
        is_group: None,
    };

    let binding_intent = if recovered_session.is_some() {
        crate::agent_identity::AgentBindingIntent::RecoverExistingSession
    } else if fork_source.is_some() {
        crate::agent_identity::AgentBindingIntent::Fork
    } else {
        crate::agent_identity::AgentBindingIntent::Fresh
    };
    match create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        options,
        binding_intent,
    )
    .await
    {
        Ok((thread_id, mut data, _resolved)) => {
            if workspace_dir_from_value(&data).is_none() {
                let implicit_update = match ensure_implicit_thread_workspace_for_config(
                    &state.config_snapshot(),
                    &thread_id,
                )
                .await
                {
                    Ok(workspace_dir) => {
                        update_thread_record(
                            &state.threads.thread_store,
                            &thread_id,
                            None,
                            Some(workspace_dir),
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
                match implicit_update {
                    Ok(updated) => {
                        data = updated;
                        state
                            .integration
                            .bridge
                            .set_thread_workspace_binding(
                                &thread_id,
                                workspace_dir_from_value(&data),
                            )
                            .await;
                    }
                    Err(error) => {
                        state.threads.thread_store.delete_logged(&thread_id).await;
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": error })),
                        );
                    }
                }
            }
            if let Some(recovered) = recovered_session.as_ref()
                && let Err(error) =
                    seed_imported_thread_history(&state, &thread_id, &mut data, &recovered.messages)
                        .await
            {
                state.threads.thread_store.delete_logged(&thread_id).await;
                let _ = state
                    .threads
                    .history
                    .delete_thread_history(&thread_id)
                    .await;
                state
                    .integration
                    .bridge
                    .set_thread_workspace_binding(&thread_id, None)
                    .await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                );
            }
            // A freshly created thread has no channel-endpoint bindings yet,
            // so it cannot invalidate the router's endpoint/binding indexes;
            // no index maintenance is needed on this path.
            state.invalidate_gateway_sync_caches().await;
            (StatusCode::CREATED, Json(thread_summary(&thread_id, &data)))
        }
        Err(ThreadCreationError::AgentBinding(error)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string() })),
        ),
        Err(ThreadCreationError::Other(error))
            if error.starts_with("unknown agent_id:")
                || error.starts_with("agent_id is not standalone:")
                || error.starts_with("workspace_mode=worktree") =>
        {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        Err(ThreadCreationError::Storage(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ),
    }
}

/// PATCH /api/threads/:key - update canonical thread metadata
pub async fn update_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<UpdateThreadBody>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            );
        }
        Err(response) => return response,
    };

    match update_thread_record(
        &state.threads.thread_store,
        &thread_id,
        body.label.clone(),
        body.workspace_dir.clone(),
    )
    .await
    {
        Ok(mut data) => {
            let observed = data.clone();
            let runtime_cells_changed = apply_thread_runtime_cells(&mut data, &body);
            if runtime_cells_changed {
                if let Some(obj) = data.as_object_mut() {
                    obj.insert(
                        "updated_at".to_owned(),
                        Value::String(Utc::now().to_rfc3339()),
                    );
                }
                let patch = match ThreadRecordPatch::from_diff(
                    &observed,
                    &data,
                    &["metadata", "updated_at"],
                ) {
                    Ok(patch) => patch,
                    Err(error) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": error.to_string() })),
                        );
                    }
                };
                if let Err(error) = state.threads.thread_store.patch(&thread_id, patch).await {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": error.to_string() })),
                    );
                }
            }
            state
                .integration
                .bridge
                .set_thread_workspace_binding(&thread_id, workspace_dir_from_value(&data))
                .await;
            state.invalidate_gateway_sync_caches().await;
            (StatusCode::OK, Json(thread_summary(&thread_id, &data)))
        }
        Err(error) if error.contains("thread not found") => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": error })))
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
    }
}
