use super::super::*;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use garyx_models::provider::ProviderMessage;
use garyx_router::{
    active_run_snapshot_messages, is_thread_key, normalize_workspace_dir, workspace_dir_from_value,
};
use serde_json::{Map, Value, json};

const DEFAULT_HISTORY_LIMIT: usize = 200;
const MAX_HISTORY_LIMIT: usize = 2_000;

#[derive(Debug, Clone)]
pub(super) struct HistoryEntry {
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub role: String,
    pub text: String,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum TimeBound {
    Start,
    End,
}

#[derive(Debug)]
pub(super) struct CollectedHistory {
    pub entries: Vec<HistoryEntry>,
    pub threads_scanned: usize,
    pub matched_threads: usize,
}

pub(crate) async fn run(
    server: &GaryMcpServer,
    params: ConversationHistoryParams,
) -> Result<String, String> {
    let started = Instant::now();
    let result = async {
        let thread_filter = normalize_thread_id_filter(params.thread_id.as_deref());
        let workspace_filter = normalize_workspace_dir(params.workspace_dir.as_deref());
        let from = parse_time_bound(params.from.as_deref(), TimeBound::Start)?;
        let to = parse_time_bound(params.to.as_deref(), TimeBound::End)?;
        if let (Some(from), Some(to)) = (from, to) {
            if from > to {
                return Err(
                    "invalid time range: `from` must be earlier than or equal to `to`".to_owned(),
                );
            }
        }

        let limit = params
            .limit
            .unwrap_or(DEFAULT_HISTORY_LIMIT)
            .clamp(1, MAX_HISTORY_LIMIT);

        let mut collected = collect_history_entries(
            server,
            thread_filter.as_deref(),
            workspace_filter.as_deref(),
            from.clone(),
            to.clone(),
        )
        .await?;

        collected.entries.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.thread_id.cmp(&right.thread_id))
                .then_with(|| left.sequence.cmp(&right.sequence))
        });

        let truncated = collected.entries.len() > limit;
        if truncated {
            let drop_count = collected.entries.len() - limit;
            collected.entries.drain(0..drop_count);
        }

        let transcript = collected
            .entries
            .iter()
            .map(|entry| format!("{}: {}", entry.role, sanitize_transcript_text(&entry.text)))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(serde_json::to_string(&json!({
            "tool": "conversation_history",
            "status": "ok",
            "thread_id": thread_filter,
            "workspace_dir": workspace_filter,
            "from": from.map(|value| value.to_rfc3339()),
            "to": to.map(|value| value.to_rfc3339()),
            "limit": limit,
            "threads_scanned": collected.threads_scanned,
            "matched_threads": collected.matched_threads,
            "matched_messages": collected.entries.len(),
            "truncated": truncated,
            "transcript": transcript,
        }))
        .unwrap_or_default())
    }
    .await;

    server.record_tool_metric(
        "conversation_history",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result
}

pub(super) async fn collect_history_entries(
    server: &GaryMcpServer,
    thread_filter: Option<&str>,
    workspace_filter: Option<&str>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> Result<CollectedHistory, String> {
    let thread_keys = server.app_state.threads.thread_store.list_keys(None).await;
    let transcript_store = server.app_state.threads.history.transcript_store();
    let mut scanned_threads = 0usize;
    let mut matched_threads = 0usize;
    let mut entries = Vec::new();

    for thread_id in thread_keys {
        if !is_thread_key(&thread_id) {
            continue;
        }
        if thread_filter.is_some_and(|filter| filter != thread_id) {
            continue;
        }

        let Some(thread_data) = server.app_state.threads.thread_store.get(&thread_id).await else {
            continue;
        };
        scanned_threads += 1;

        let workspace_dir = workspace_dir_from_value(&thread_data);
        if workspace_filter.is_some_and(|filter| workspace_dir.as_deref() != Some(filter)) {
            continue;
        }

        let mut thread_entries = Vec::new();
        if transcript_store.exists(&thread_id).await {
            let records = transcript_store
                .records(&thread_id)
                .await
                .map_err(|error| format!("failed to load transcript for {thread_id}: {error}"))?;
            for record in records {
                if let Some(entry) = build_history_entry(
                    &thread_id,
                    workspace_dir.as_deref(),
                    &record.message,
                    Some(record.seq),
                    Some(record.timestamp.as_str()),
                    from.as_ref(),
                    to.as_ref(),
                ) {
                    thread_entries.push(entry);
                }
            }
        }

        let base_sequence = thread_entries
            .iter()
            .map(|entry| entry.sequence)
            .max()
            .unwrap_or(0);
        for (idx, message) in active_run_snapshot_messages(&thread_data)
            .iter()
            .enumerate()
        {
            if let Some(entry) = build_history_entry(
                &thread_id,
                workspace_dir.as_deref(),
                message,
                Some(base_sequence + idx as u64 + 1),
                None,
                from.as_ref(),
                to.as_ref(),
            ) {
                thread_entries.push(entry);
            }
        }

        if thread_entries.is_empty() {
            continue;
        }
        matched_threads += 1;
        entries.extend(thread_entries);
    }

    Ok(CollectedHistory {
        entries,
        threads_scanned: scanned_threads,
        matched_threads,
    })
}

pub(super) fn normalize_thread_id_filter(value: Option<&str>) -> Option<String> {
    let trimmed = value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())?;
    if trimmed.starts_with("thread::") {
        return Some(trimmed.to_owned());
    }
    trimmed
        .strip_prefix("thread:")
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| Some(trimmed.to_owned()))
}

pub(super) fn parse_time_bound(
    raw: Option<&str>,
    bound: TimeBound,
) -> Result<Option<DateTime<Utc>>, String> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if let Ok(timestamp) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(timestamp.with_timezone(&Utc)));
    }

    for format in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(timestamp) = NaiveDateTime::parse_from_str(raw, format) {
            return Ok(Some(DateTime::<Utc>::from_naive_utc_and_offset(
                timestamp, Utc,
            )));
        }
    }

    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let timestamp = match bound {
            TimeBound::Start => date
                .and_hms_opt(0, 0, 0)
                .expect("midnight should always be valid"),
            TimeBound::End => date
                .and_hms_milli_opt(23, 59, 59, 999)
                .expect("end-of-day should always be valid"),
        };
        return Ok(Some(DateTime::<Utc>::from_naive_utc_and_offset(
            timestamp, Utc,
        )));
    }

    Err(format!(
        "invalid timestamp `{raw}`. Use RFC3339, YYYY-MM-DD, YYYY-MM-DD HH:MM, or YYYY-MM-DDTHH:MM"
    ))
}

fn build_history_entry(
    thread_id: &str,
    workspace_dir: Option<&str>,
    message: &Value,
    sequence: Option<u64>,
    timestamp_hint: Option<&str>,
    from: Option<&DateTime<Utc>>,
    to: Option<&DateTime<Utc>>,
) -> Option<HistoryEntry> {
    let object = message.as_object()?;
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    if !matches!(role.as_str(), "user" | "assistant") {
        return None;
    }
    if is_tool_related_message(&role, object) {
        return None;
    }

    let text = extract_visible_text(message)?;
    let timestamp = timestamp_hint
        .or_else(|| object.get("timestamp").and_then(Value::as_str))
        .and_then(parse_stored_timestamp);
    if let Some(lower) = from {
        if timestamp
            .as_ref()
            .map(|candidate| candidate < lower)
            .unwrap_or(true)
        {
            return None;
        }
    }
    if let Some(upper) = to {
        if timestamp
            .as_ref()
            .map(|candidate| candidate > upper)
            .unwrap_or(true)
        {
            return None;
        }
    }

    Some(HistoryEntry {
        thread_id: thread_id.to_owned(),
        workspace_dir: workspace_dir.map(ToOwned::to_owned),
        timestamp,
        role,
        text,
        sequence: sequence.unwrap_or(0),
    })
}

pub(super) fn parse_stored_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

pub(super) fn extract_visible_text(message: &Value) -> Option<String> {
    if let Some(provider_message) = ProviderMessage::from_value(message) {
        if let Some(text) = provider_message
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_owned());
        }
        let extracted = extract_text_from_content(&provider_message.content);
        if !extracted.is_empty() {
            return Some(extracted);
        }
    }

    if let Some(text) = garyx_router::message_text(message)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_owned());
    }

    let extracted = extract_text_from_content(message.get("content").unwrap_or(&Value::Null));
    if extracted.is_empty() {
        None
    } else {
        Some(extracted)
    }
}

pub(super) fn extract_text_from_content(content: &Value) -> String {
    let mut parts = Vec::new();
    collect_content_text(content, &mut parts, 0);
    parts.join("\n")
}

fn collect_content_text(content: &Value, parts: &mut Vec<String>, depth: usize) {
    if depth > 32 {
        return;
    }

    match content {
        Value::String(text) => push_text_part(parts, text),
        Value::Array(items) => {
            for item in items {
                collect_content_text(item, parts, depth + 1);
            }
        }
        Value::Object(map) => collect_object_text(map, parts, depth + 1),
        _ => {}
    }
}

fn collect_object_text(map: &Map<String, Value>, parts: &mut Vec<String>, depth: usize) {
    if let Some(text) = map.get("text").and_then(Value::as_str) {
        push_text_part(parts, text);
    }
    if let Some(content) = map.get("content") {
        collect_content_text(content, parts, depth + 1);
    }
    if let Some(parts_value) = map.get("parts") {
        collect_content_text(parts_value, parts, depth + 1);
    }
    if let Some(items_value) = map.get("items") {
        collect_content_text(items_value, parts, depth + 1);
    }
}

fn push_text_part(parts: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_owned());
    }
}

pub(super) fn sanitize_transcript_text(value: &str) -> String {
    value
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn is_tool_related_message(role: &str, message: &Map<String, Value>) -> bool {
    if matches!(role, "tool" | "tool_use" | "tool_result") {
        return true;
    }

    if message
        .get("tool_use_result")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }

    if message
        .get("tool_name")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }

    contains_tool_hint(message.get("content"))
        || contains_tool_hint(message.get("metadata"))
        || contains_tool_hint(message.get("input"))
        || contains_tool_hint(message.get("result"))
}

pub(super) fn contains_tool_hint(value: Option<&Value>) -> bool {
    fn inner(value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }

        match value {
            Value::String(text) => {
                let lower = text.to_ascii_lowercase();
                lower.contains("tool_use")
                    || lower.contains("tool_result")
                    || lower.contains("tool_call")
                    || lower.contains("mcp__")
            }
            Value::Array(items) => items.iter().any(|item| inner(item, depth + 1)),
            Value::Object(map) => map.iter().any(|(key, item)| {
                let lower = key.to_ascii_lowercase();
                lower == "tool_use_id"
                    || lower == "tool_call_id"
                    || lower == "tool_calls"
                    || lower.contains("mcp__")
                    || lower.contains("tool_")
                    || inner(item, depth + 1)
            }),
            _ => false,
        }
    }

    value.is_some_and(|value| inner(value, 0))
}
