//! Thread history pages and content enrichment for the HTTP surface.

use super::*;
use crate::server::AppState;
use crate::thread_runtime::build_thread_runtime_summary;
use crate::thread_type::thread_summary_type_from_record;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::engine::general_purpose::STANDARD as BASE64;
use garyx_models::provider::ProviderMessage;
use garyx_models::transcript_kind::is_tool_related_message;
use garyx_router::{
    ThreadHistoryError, bindings_from_value, count_user_query_messages, history_message_count,
    is_thread_key, workspace_dir_from_value,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path as FsPath;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ThreadHistoryParams {
    /// Maximum number of history items to return.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Optional prefix filter for thread ids.
    #[serde(default)]
    pub prefix: Option<String>,
    /// Whether to include message content (tool messages, etc.).
    #[serde(default)]
    pub include_messages: bool,
    /// Optional single thread lookup for detailed history.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Return messages before this zero-based global message index.
    #[serde(default, alias = "before")]
    pub before_index: Option<usize>,
    /// Return a page containing this many human user query turns.
    #[serde(default, alias = "limit_user_queries", alias = "user_turn_limit")]
    pub user_query_limit: Option<usize>,
    /// Return committed messages strictly after this zero-based global index
    /// (forward / delta cursor). Takes precedence over before_index and
    /// user_query_limit.
    #[serde(default, alias = "after")]
    pub after_index: Option<usize>,
    /// Whether detailed history should keep tool-related messages.
    #[serde(default = "default_include_tool_messages")]
    pub include_tool_messages: bool,
}

pub(super) fn default_limit() -> usize {
    50
}

pub(super) fn default_include_tool_messages() -> bool {
    true
}

pub(super) const MAX_THREAD_HISTORY_LIMIT: usize = 500;

pub(super) const MAX_THREAD_HISTORY_USER_QUERY_LIMIT: usize = 50;

/// GET /api/threads/history - thread history with optional filtering.
pub async fn thread_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ThreadHistoryParams>,
) -> axum::response::Response {
    if let Some(thread_id) = params.thread_id.as_deref() {
        let payload = thread_history_for_key(
            &state,
            thread_id,
            params.limit,
            params.include_tool_messages,
            params.before_index,
            params.user_query_limit,
            params.after_index,
        )
        .await;
        return Json(payload).into_response();
    }

    // List mode is a request boundary: a store failure must surface as a
    // 500, never as a successful empty listing (#TASK-2099).
    match thread_history_listing(&state, &params).await {
        Ok(payload) => Json(payload).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "reason": "thread-store-error",
                "error": error.to_string(),
            })),
        )
            .into_response(),
    }
}

pub(super) async fn thread_history_listing(
    state: &Arc<AppState>,
    params: &ThreadHistoryParams,
) -> Result<Value, garyx_router::ThreadStoreError> {
    let keys = state
        .threads
        .thread_store
        .list_keys(params.prefix.as_deref())
        .await?;
    let keys: Vec<String> = keys.into_iter().filter(|key| is_thread_key(key)).collect();

    let limited_keys: Vec<&String> = keys.iter().take(params.limit).collect();
    let mut threads = Vec::new();

    for key in &limited_keys {
        if params.include_messages {
            // A missing record (deleted between list and get) is Ok(None)
            // and keeps its `data: null` row; only real store failures
            // abort the listing.
            let data = state.threads.thread_store.get(key).await?;
            threads.push(json!({
                "key": key,
                "data": data,
            }));
        } else {
            // Summary only: key + existence
            let exists = state.threads.thread_store.exists(key).await?;
            threads.push(json!({
                "key": key,
                "active": exists,
            }));
        }
    }

    Ok(json!({
        "threads": threads,
        "total": keys.len(),
        "limit": params.limit,
        "include_messages": params.include_messages,
    }))
}

pub(crate) async fn thread_history_for_key(
    state: &Arc<AppState>,
    thread_id: &str,
    limit: usize,
    include_tool_messages: bool,
    before_index: Option<usize>,
    user_query_limit: Option<usize>,
    after_index: Option<usize>,
) -> Value {
    let key = thread_id.trim();
    if key.is_empty() {
        let thread = Value::Null;
        return json!({
            "ok": false,
            "reason": "missing-thread-id",
            "thread": thread,
            "session": thread,
            "thread_runtime": Value::Null,
            "messages": [],
            "pending_user_inputs": [],
            "message_stats": { "returned_messages": 0 },
        });
    }

    let bounded_limit = limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    let bounded_user_query_limit =
        user_query_limit.map(|value| value.clamp(1, MAX_THREAD_HISTORY_USER_QUERY_LIMIT));
    // When the client sends both an `after_index` cursor and a `user_query_limit`,
    // bound the catch-up to the newest N user turns: fetch that window once, then if
    // the cursor is older than it, return the window and flag `reset` so the client
    // overwrites its cache instead of replaying the whole delta; otherwise trim to the
    // forward delta within the window.
    let mut reset_to_newest = false;
    let snapshot_result = if let (Some(after_index), Some(user_query_limit)) =
        (after_index, bounded_user_query_limit)
    {
        match state
            .threads
            .history
            .thread_snapshot_user_query_page(key, bounded_limit, None, user_query_limit)
            .await
        {
            Ok(mut snapshot) => {
                let floor = snapshot.committed_start_index;
                if after_index + 1 < floor {
                    reset_to_newest = true;
                } else {
                    let drop = (after_index + 1 - floor).min(snapshot.committed_messages.len());
                    snapshot.committed_messages.drain(0..drop);
                    snapshot.committed_start_index = after_index + 1;
                }
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    } else if let Some(after_index) = after_index {
        state
            .threads
            .history
            .thread_snapshot_after_index(key, after_index, bounded_limit)
            .await
    } else if let Some(user_query_limit) = bounded_user_query_limit {
        state
            .threads
            .history
            .thread_snapshot_user_query_page(key, bounded_limit, before_index, user_query_limit)
            .await
    } else {
        state
            .threads
            .history
            .thread_snapshot_page(key, bounded_limit, before_index)
            .await
    };
    let snapshot = match snapshot_result {
        Ok(snapshot) => snapshot,
        Err(ThreadHistoryError::ThreadNotFound(_)) => {
            let thread = json!({ "thread_id": key, "thread_key": key });
            return json!({
                "ok": false,
                "reason": "thread-not-found",
                "thread": thread,
                "session": thread,
                "thread_runtime": Value::Null,
                "messages": [],
                "pending_user_inputs": [],
                "message_stats": { "returned_messages": 0 },
            });
        }
        Err(ThreadHistoryError::MissingTranscript(_)) => {
            let thread = json!({ "thread_id": key, "thread_key": key });
            return json!({
                "ok": false,
                "reason": "thread-transcript-missing",
                "thread": thread,
                "session": thread,
                "thread_runtime": Value::Null,
                "messages": [],
                "pending_user_inputs": [],
                "message_stats": { "returned_messages": 0 },
            });
        }
        Err(error) => {
            let thread = json!({ "thread_id": key, "thread_key": key });
            return json!({
                "ok": false,
                "reason": format!("thread-history-error:{error}"),
                "thread": thread,
                "session": thread,
                "thread_runtime": Value::Null,
                "messages": [],
                "pending_user_inputs": [],
                "message_stats": { "returned_messages": 0 },
            });
        }
    };
    let messages = snapshot.combined_messages();
    let total_messages = snapshot.total_messages();

    let mut history_messages = Vec::new();
    let mut kind_counts = serde_json::Map::new();
    let mut role_counts = serde_json::Map::new();
    let mut tool_related_count = 0_u64;
    let mut likely_user_visible_count = 0_u64;

    for (idx, message) in messages.iter().enumerate() {
        let Some(obj) = message.as_object() else {
            continue;
        };
        let global_index = snapshot.message_index_at(idx);

        let role = obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .trim()
            .to_ascii_lowercase();

        let is_control = garyx_models::is_control_message(obj);
        let tool_related = if is_control {
            false
        } else {
            is_tool_related_message(&role, obj)
        };
        if !include_tool_messages && tool_related {
            continue;
        }

        let kind = garyx_models::resolve_message_kind_for_object(&role, obj, tool_related);
        let likely_user_visible = matches!(kind, "user_input" | "assistant_reply");
        if tool_related {
            tool_related_count += 1;
        }
        if likely_user_visible {
            likely_user_visible_count += 1;
        }
        increment_counter(&mut kind_counts, kind);
        increment_counter(&mut role_counts, &role);

        let normalized_message = (!is_control)
            .then(|| ProviderMessage::from_value(message))
            .flatten();
        let raw_content = normalized_message
            .as_ref()
            .map(|entry| entry.content.clone())
            .or_else(|| obj.get("content").cloned())
            .unwrap_or(Value::Null);
        let content = if is_control {
            Value::String(String::new())
        } else {
            enrich_message_content_for_history(&raw_content)
        };
        let message_value = if let Some(mut entry) = normalized_message.clone() {
            entry.content = content.clone();
            entry.to_json_value()
        } else if is_control {
            message.clone()
        } else {
            let mut value = message.clone();
            if let Some(map) = value.as_object_mut() {
                map.insert("content".to_owned(), content.clone());
            }
            value
        };
        let text = if is_control {
            String::new()
        } else {
            normalized_message
                .as_ref()
                .and_then(|entry| entry.text.clone())
                .unwrap_or_else(|| stringify_message_content(&content))
        };
        history_messages.push(json!({
            "index": global_index,
            "role": if role.is_empty() { "unknown" } else { role.as_str() },
            "kind": kind,
            "tool_related": tool_related,
            "likely_user_visible": likely_user_visible,
            "internal": obj.get("internal").cloned().unwrap_or(Value::Bool(false)),
            "internal_kind": obj.get("internal_kind").cloned().unwrap_or(Value::Null),
            "loop_origin": obj.get("loop_origin").cloned().unwrap_or(Value::Null),
            "timestamp": obj.get("timestamp").cloned().unwrap_or(Value::Null),
            "text": text,
            "content": stringify_message_content(&content),
            "raw_content_type": raw_content_type_name(&content),
            "message": message_value,
        }));
    }

    let returned_messages = history_messages.len();
    let returned_user_queries = count_user_query_messages(&messages);
    let page_start_index = snapshot.first_message_index().unwrap_or(0);
    let page_end_index = messages
        .len()
        .checked_sub(1)
        .map(|offset| snapshot.message_index_at(offset) + 1)
        .unwrap_or(page_start_index);
    let has_more_before = page_start_index > 0;
    let next_before_index = if has_more_before {
        Value::Number(serde_json::Number::from(page_start_index as u64))
    } else {
        Value::Null
    };
    // An empty page means caught up: page_end_index falls back to page_start_index
    // (0 when nothing is returned), so guard on a non-empty page or a caught-up
    // client would be told "more after, resume at 0" → full re-fetch loop.
    let has_more_after = !messages.is_empty() && page_end_index < total_messages;
    let next_after_index = if has_more_after {
        Value::Number(serde_json::Number::from(
            page_end_index.saturating_sub(1) as u64
        ))
    } else {
        Value::Null
    };
    let data_raw = snapshot.thread_data;

    let pending_raw = data_raw
        .get("pending_user_inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut pending_user_inputs = Vec::new();
    let mut active_pending_user_input_count = 0_u64;
    for record in pending_raw {
        let Some(obj) = record.as_object() else {
            continue;
        };
        let run_id = obj
            .get("bridge_run_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let run_active = if let Some(run_id) = run_id.as_deref() {
            state.integration.bridge.is_run_active(run_id).await
        } else {
            false
        };
        let stored_status = obj
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("queued");
        if stored_status.eq_ignore_ascii_case("abandoned") || !run_active {
            continue;
        }

        active_pending_user_input_count += 1;
        let id = obj.get("id").cloned().unwrap_or(Value::Null);
        let timestamp = obj.get("queued_at").cloned().unwrap_or(Value::Null);
        let content = obj.get("content").cloned().unwrap_or(Value::Null);
        let text = obj
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| stringify_message_content(&content));
        let raw_content_type = raw_content_type_name(&content);
        pending_user_inputs.push(json!({
            "id": id,
            "run_id": run_id,
            "timestamp": timestamp,
            "status": "awaiting_ack",
            "active": true,
            "text": text,
            "content": content,
            "raw_content_type": raw_content_type,
        }));
    }
    let thread = summarize_thread(key, &data_raw, &messages);
    json!({
        "ok": true,
        "thread": thread,
        "session": thread,
        "thread_runtime": build_thread_runtime_summary(state, Some(&data_raw)).await,
        "messages": history_messages,
        "pending_user_inputs": pending_user_inputs,
        "message_stats": {
            "total_messages_in_thread": total_messages,
            "total_messages_in_session": total_messages,
            "committed_message_count": snapshot.total_committed_messages,
            "returned_messages": returned_messages,
            "returned_user_queries": returned_user_queries,
            "returned_start_index": page_start_index,
            "returned_end_index": page_end_index,
            "has_more_before": has_more_before,
            "next_before_index": next_before_index,
            "has_more_after": has_more_after,
            "next_after_index": next_after_index,
            "reset": reset_to_newest,
            "user_query_limit": bounded_user_query_limit,
            "tool_related_count": tool_related_count,
            "likely_user_visible_count": likely_user_visible_count,
            "pending_user_input_count": pending_user_inputs.len(),
            "active_pending_user_input_count": active_pending_user_input_count,
            "kind_counts": Value::Object(kind_counts),
            "role_counts": Value::Object(role_counts),
        },
        "include_tool_messages": include_tool_messages,
    })
}

pub(super) fn summarize_thread(thread_id: &str, data: &Value, messages: &[Value]) -> Value {
    let message_count = history_message_count(data);
    let thread_type = thread_summary_type_from_record(data);

    let last_user_message = last_message_content(messages, "user");
    let last_assistant_message = last_message_content(messages, "assistant");

    let get_value = |primary: &str, fallback: &str| {
        data.get(primary)
            .cloned()
            .or_else(|| data.get(fallback).cloned())
            .unwrap_or(Value::Null)
    };

    let workspace_dir = workspace_dir_from_value(data);
    let recorded_origin = data
        .get("workspace_origin")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let workspace_origin = crate::workspace_mode::effective_workspace_origin(
        thread_id,
        workspace_dir.as_deref(),
        recorded_origin,
    );
    let root_workspace_path = crate::workspace_mode::thread_root_workspace_path(
        workspace_origin,
        workspace_dir.as_deref(),
        data.get("worktree").unwrap_or(&Value::Null),
    );
    json!({
        "thread_id": thread_id,
        "thread_key": thread_id,
        "label": data.get("label").cloned().unwrap_or(Value::Null),
        "channel": get_value("channel", "last_channel"),
        "account_id": get_value("account_id", "last_account_id"),
        "from_id": get_value("from_id", "last_to"),
        "workspace_dir": workspace_dir.map(Value::String).unwrap_or(Value::Null),
        "workspace_origin": workspace_origin,
        "root_workspace_path": root_workspace_path,
        "channel_bindings": serde_json::to_value(bindings_from_value(data)).unwrap_or_else(|_| Value::Array(Vec::new())),
        "message_count": message_count,
        "updated_at": get_value("updated_at", "_updated_at"),
        "created_at": get_value("created_at", "_created_at"),
        "last_user_message": last_user_message,
        "last_assistant_message": last_assistant_message,
        "thread_type": thread_type,
    })
}

pub(super) fn last_message_content(messages: &[Value], role: &str) -> Option<String> {
    for message in messages.iter().rev() {
        let Some(obj) = message.as_object() else {
            continue;
        };
        let role_match = obj
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|value| value == role);
        if !role_match {
            continue;
        }
        if let Some(content) = obj.get("content") {
            let text = stringify_message_content(content);
            if !text.trim().is_empty() {
                return Some(truncate_preview_text(text, 260));
            }
        }
    }
    None
}

pub(super) fn raw_content_type_name(content: &Value) -> &'static str {
    match content {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

pub(super) fn summarize_content_block(
    content: &Value,
    parts: &mut Vec<String>,
    image_count: &mut usize,
) {
    let Some(obj) = content.as_object() else {
        return;
    };

    match obj.get("type").and_then(Value::as_str).unwrap_or_default() {
        "text" => {
            if let Some(text) = obj.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_owned());
                }
            }
        }
        "image" => {
            *image_count += 1;
        }
        "file" => {
            let label = obj
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| obj.get("path").and_then(Value::as_str))
                .unwrap_or("file");
            parts.push(format!("[File] {label}"));
        }
        _ => {}
    }
}

pub(super) const HISTORY_IMAGE_INLINE_MAX_BYTES: u64 = 12 * 1024 * 1024;

pub(super) fn trimmed_string_field<'a>(
    block: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| block.get(*key).and_then(Value::as_str).map(str::trim))
        .find(|value| !value.is_empty())
}

pub(super) fn has_inline_image_source(block: &serde_json::Map<String, Value>) -> bool {
    block.get("url").and_then(Value::as_str).is_some()
        || block
            .get("source")
            .and_then(Value::as_object)
            .and_then(|source| source.get("data"))
            .and_then(Value::as_str)
            .is_some()
}

pub(super) fn history_image_media_type(
    block: &serde_json::Map<String, Value>,
    path: &str,
) -> String {
    trimmed_string_field(
        block,
        &[
            "media_type",
            "mediaType",
            "mime_type",
            "mimeType",
            "contentType",
        ],
    )
    .map(str::to_owned)
    .or_else(|| {
        FsPath::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .map(|ext| match ext.to_ascii_lowercase().as_str() {
                "png" => "image/png".to_owned(),
                "gif" => "image/gif".to_owned(),
                "webp" => "image/webp".to_owned(),
                _ => "image/jpeg".to_owned(),
            })
    })
    .unwrap_or_else(|| "image/jpeg".to_owned())
}

pub(super) fn image_source_for_history_path(
    block: &serde_json::Map<String, Value>,
    path: &str,
) -> Option<Value> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return None;
    };
    if !metadata.is_file() || metadata.len() > HISTORY_IMAGE_INLINE_MAX_BYTES {
        return None;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return None;
    };

    let media_type = history_image_media_type(block, path);
    Some(json!({
        "type": "base64",
        "media_type": media_type,
        "data": BASE64.encode(bytes),
    }))
}

pub(super) fn enrich_image_block_for_history(block: &serde_json::Map<String, Value>) -> Value {
    if has_inline_image_source(block) {
        return Value::Object(block.clone());
    }

    let Some(path) = trimmed_string_field(block, &["path"]) else {
        return Value::Object(block.clone());
    };
    let Some(source) = image_source_for_history_path(block, path) else {
        return Value::Object(block.clone());
    };

    let mut hydrated = block.clone();
    hydrated.insert("source".to_owned(), source);
    Value::Object(hydrated)
}

/// Prepare structured content for thread history. Message text is preserved
/// verbatim; image path blocks are additionally hydrated for the client.
pub(super) fn enrich_message_content_for_history(content: &Value) -> Value {
    match content {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(enrich_message_content_for_history)
                .collect(),
        ),
        Value::Object(block) => match block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            // Images are hydrated to inline base64 and must not be length-capped.
            "image" => enrich_image_block_for_history(block),
            // Recurse into other objects so nested image blocks are hydrated.
            _ => Value::Object(
                block
                    .iter()
                    .map(|(key, value)| (key.clone(), enrich_message_content_for_history(value)))
                    .collect(),
            ),
        },
        Value::String(text) => Value::String(text.clone()),
        _ => content.clone(),
    }
}

pub(super) fn humanize_structured_content(content: &Value) -> Option<String> {
    let mut parts = Vec::new();
    let mut image_count = 0_usize;

    match content {
        Value::Array(items) => {
            for item in items {
                summarize_content_block(item, &mut parts, &mut image_count);
            }
        }
        Value::Object(_) => {
            summarize_content_block(content, &mut parts, &mut image_count);
        }
        _ => {}
    }

    if image_count > 0 {
        parts.push(if image_count == 1 {
            "[1 image]".to_owned()
        } else {
            format!("[{image_count} images]")
        });
    }

    let summary = parts.join("\n\n");
    if summary.trim().is_empty() {
        None
    } else {
        Some(summary)
    }
}

pub(super) fn stringify_message_content(content: &Value) -> String {
    match content {
        Value::Null => String::new(),
        Value::String(text) => text.to_owned(),
        Value::Array(_) | Value::Object(_) => {
            if let Some(summary) = humanize_structured_content(content) {
                summary
            } else {
                serde_json::to_string(content).unwrap_or_else(|_| format!("{content}"))
            }
        }
        _ => content.to_string(),
    }
}

pub(super) fn truncate_preview_text(text: String, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text;
    }
    text.chars().take(max_len).collect()
}

pub(super) fn increment_counter(map: &mut serde_json::Map<String, Value>, key: &str) {
    let entry = map
        .entry(key.to_owned())
        .or_insert_with(|| Value::from(0_u64));
    let next = entry.as_u64().unwrap_or(0) + 1;
    *entry = Value::from(next);
}

// ---------------------------------------------------------------------------
// GET /api/cron/jobs
// ---------------------------------------------------------------------------
