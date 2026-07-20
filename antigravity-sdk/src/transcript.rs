use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::types::AntigravityEvent;

#[derive(Debug, Clone)]
struct TranscriptRow {
    row_type: String,
    step_index: Option<i64>,
    created_at: Option<String>,
    content: Option<Value>,
    thinking: Option<Value>,
    tool_calls: Option<Value>,
    error: Option<Value>,
    is_truncated: bool,
    truncated_fields: HashSet<String>,
    raw: Value,
}

impl TranscriptRow {
    fn from_value(raw: Value) -> Option<Self> {
        let row_type = raw
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_owned();
        Some(Self {
            row_type,
            step_index: raw.get("step_index").and_then(value_i64),
            created_at: raw
                .get("created_at")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            content: raw.get("content").filter(|value| !value.is_null()).cloned(),
            thinking: raw
                .get("thinking")
                .filter(|value| !value.is_null())
                .cloned(),
            tool_calls: raw
                .get("tool_calls")
                .filter(|value| !value.is_null())
                .cloned(),
            error: raw.get("error").filter(|value| !value.is_null()).cloned(),
            is_truncated: raw
                .get("is_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            truncated_fields: truncated_fields_from_value(&raw),
            raw,
        })
    }
}

fn truncated_fields_from_value(raw: &Value) -> HashSet<String> {
    match raw.get("truncated_fields") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Some(Value::String(field)) => {
            let field = field.trim();
            if field.is_empty() {
                HashSet::new()
            } else {
                HashSet::from([field.to_owned()])
            }
        }
        _ => HashSet::new(),
    }
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

fn parse_jsonl_rows(contents: &str) -> Vec<TranscriptRow> {
    contents
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<Value>(trimmed)
                .ok()
                .and_then(TranscriptRow::from_value)
        })
        .collect()
}

fn read_jsonl_rows(path: &Path) -> Vec<TranscriptRow> {
    std::fs::read_to_string(path)
        .map(|contents| parse_jsonl_rows(&contents))
        .unwrap_or_default()
}

fn value_payload_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        _ => value.to_string().len(),
    }
}

fn field_payload_richer(
    compact_field: Option<&Value>,
    full_field: Option<&Value>,
    compact_field_truncated: bool,
) -> bool {
    let Some(full_field) = full_field else {
        return false;
    };
    if compact_field_truncated {
        return true;
    }
    compact_field.is_none_or(|compact_field| {
        full_field != compact_field
            && value_payload_len(full_field) > value_payload_len(compact_field)
    })
}

fn row_payload_richer(compact: &TranscriptRow, full: &TranscriptRow) -> bool {
    compact.is_truncated
        || field_payload_richer(
            compact.content.as_ref(),
            full.content.as_ref(),
            compact.truncated_fields.contains("content"),
        )
        || field_payload_richer(
            compact.thinking.as_ref(),
            full.thinking.as_ref(),
            compact.truncated_fields.contains("thinking"),
        )
        || field_payload_richer(
            compact.tool_calls.as_ref(),
            full.tool_calls.as_ref(),
            compact.truncated_fields.contains("tool_calls"),
        )
        || field_payload_richer(
            compact.error.as_ref(),
            full.error.as_ref(),
            compact.truncated_fields.contains("error"),
        )
}

pub(crate) fn transcript_path(brain_root: &Path, conversation_id: &str) -> PathBuf {
    brain_root
        .join(conversation_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl")
}

fn transcript_full_path(compact_path: &Path) -> PathBuf {
    compact_path.with_file_name("transcript_full.jsonl")
}

fn read_transcript_rows(compact_path: &Path) -> Vec<TranscriptRow> {
    let compact_rows = read_jsonl_rows(compact_path);
    if compact_rows.is_empty() {
        return compact_rows;
    }
    let full_rows = read_jsonl_rows(&transcript_full_path(compact_path));
    if full_rows.is_empty() {
        return compact_rows;
    }
    let mut full_by_step = full_rows
        .into_iter()
        .filter_map(|row| row.step_index.map(|step| (step, row)))
        .collect::<HashMap<_, _>>();
    compact_rows
        .into_iter()
        .map(|row| {
            if let Some(step) = row.step_index
                && let Some(full_row) = full_by_step.remove(&step)
                && row_payload_richer(&row, &full_row)
            {
                return full_row;
            }
            row
        })
        .collect()
}

pub(crate) fn max_step_index(compact_path: &Path) -> i64 {
    read_transcript_rows(compact_path)
        .into_iter()
        .filter_map(|row| row.step_index)
        .max()
        .unwrap_or(-1)
}

pub(crate) fn first_user_input_text(compact_path: &Path) -> Option<String> {
    read_transcript_rows(compact_path)
        .into_iter()
        .find(|row| row.row_type == "USER_INPUT")
        .and_then(|row| row_value_text(row.content.as_ref()))
}

fn row_value_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| text.to_owned())
        }
        Value::Array(_) | Value::Object(_) => Some(value?.to_string()),
        other => {
            let text = other.to_string();
            (!text.trim().is_empty()).then_some(text)
        }
    }
}

fn normalize_tool_name(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn canonical_tool_name(value: &str) -> String {
    let normalized = normalize_tool_name(value);
    match normalized.as_str() {
        "list_dir" | "list_directory" => "list_directory".to_owned(),
        _ => normalized,
    }
}

#[derive(Debug, Clone)]
struct ToolCall {
    name: String,
    input: Value,
}

fn parse_tool_input(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => {
            serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

fn transcript_tool_calls(row: &TranscriptRow) -> Vec<ToolCall> {
    let Some(Value::Array(items)) = row.tool_calls.as_ref() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| item.get("functionName").and_then(Value::as_str))
                .or_else(|| item.get("tool_name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_owned();
            let input = parse_tool_input(item.get("args").or_else(|| item.get("input")));
            Some(ToolCall { name, input })
        })
        .collect()
}

fn skip_visible_row(row_type: &str) -> bool {
    matches!(
        row_type,
        "USER_INPUT" | "CONVERSATION_HISTORY" | "SYSTEM_MESSAGE" | "CHECKPOINT"
    )
}

fn is_tool_result_row(row: &TranscriptRow) -> bool {
    if skip_visible_row(&row.row_type) || row.row_type == "PLANNER_RESPONSE" {
        return false;
    }
    row.content.is_some()
}

/// Stateful decoder for a single run's transcript tail.
pub(crate) struct TranscriptDecoder {
    processed_steps: HashSet<i64>,
    pending_tools: HashMap<String, VecDeque<String>>,
    anonymous_pending: VecDeque<String>,
    pending_reasoning: String,
    last_error: Option<String>,
    visible_output: bool,
}

impl TranscriptDecoder {
    pub(crate) fn new() -> Self {
        Self {
            processed_steps: HashSet::new(),
            pending_tools: HashMap::new(),
            anonymous_pending: VecDeque::new(),
            pending_reasoning: String::new(),
            last_error: None,
            visible_output: false,
        }
    }

    pub(crate) fn apply_path(
        &mut self,
        path: &Path,
        baseline_step_index: i64,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) {
        self.apply_rows(read_transcript_rows(path), baseline_step_index, on_event);
    }

    fn apply_rows(
        &mut self,
        rows: Vec<TranscriptRow>,
        baseline_step_index: i64,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) {
        for row in rows {
            let Some(step_index) = row.step_index else {
                continue;
            };
            if step_index <= baseline_step_index || !self.processed_steps.insert(step_index) {
                continue;
            }
            self.apply_row(row, on_event);
        }
    }

    fn apply_row(
        &mut self,
        row: TranscriptRow,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) {
        match row.row_type.as_str() {
            "PLANNER_RESPONSE" => self.apply_planner_response(row, on_event),
            "ERROR_MESSAGE" => self.apply_error_message(row, on_event),
            row_type if skip_visible_row(row_type) => {}
            _ if is_tool_result_row(&row) => self.apply_tool_result(row, on_event),
            _ => {}
        }
    }

    fn apply_planner_response(
        &mut self,
        row: TranscriptRow,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) {
        if let Some(thinking) = row_value_text(row.thinking.as_ref()) {
            if !self.pending_reasoning.is_empty() {
                self.pending_reasoning.push_str("\n\n");
            }
            self.pending_reasoning.push_str(&thinking);
        }

        for (index, call) in transcript_tool_calls(&row).into_iter().enumerate() {
            let step_index = row.step_index.unwrap_or_default();
            let tool_use_id = format!("antigravity-tool-{step_index}-{index}");
            self.pending_tools
                .entry(canonical_tool_name(&call.name))
                .or_default()
                .push_back(tool_use_id.clone());
            self.anonymous_pending.push_back(tool_use_id.clone());
            self.visible_output = true;
            on_event(AntigravityEvent::ToolUse {
                step_index,
                tool_use_id,
                name: call.name,
                input: call.input,
                created_at: row.created_at.clone(),
            });
        }

        let Some(text) = row_value_text(row.content.as_ref()) else {
            return;
        };
        let reasoning = (!self.pending_reasoning.is_empty()).then(|| {
            let value = self.pending_reasoning.clone();
            self.pending_reasoning.clear();
            value
        });
        self.visible_output = true;
        on_event(AntigravityEvent::AssistantDelta {
            step_index: row.step_index.unwrap_or_default(),
            text,
            reasoning,
            created_at: row.created_at,
        });
    }

    fn apply_tool_result(
        &mut self,
        row: TranscriptRow,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) {
        let name = row.row_type.clone();
        let tool_use_id = self.pop_tool_id_for_name(&name);
        self.visible_output = true;
        on_event(AntigravityEvent::ToolResult {
            step_index: row.step_index.unwrap_or_default(),
            tool_use_id,
            name,
            content: row.raw,
            is_error: false,
            created_at: row.created_at,
        });
    }

    fn apply_error_message(
        &mut self,
        row: TranscriptRow,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) {
        let error = row_value_text(row.error.as_ref())
            .or_else(|| row_value_text(row.content.as_ref()))
            .unwrap_or_else(|| "antigravity CLI reported an error".to_owned());
        self.last_error = Some(error.clone());
        let step_index = row.step_index.unwrap_or_default();
        on_event(AntigravityEvent::Error {
            step_index,
            message: error.clone(),
            created_at: row.created_at.clone(),
        });

        if let Some(tool_use_id) = self.pop_any_tool_id() {
            let mut content = Map::new();
            content.insert("type".to_owned(), Value::String(row.row_type.clone()));
            content.insert("error".to_owned(), Value::String(error));
            self.visible_output = true;
            on_event(AntigravityEvent::ToolResult {
                step_index,
                tool_use_id: Some(tool_use_id),
                name: row.row_type,
                content: Value::Object(content),
                is_error: true,
                created_at: row.created_at,
            });
        }
    }

    fn pop_tool_id_for_name(&mut self, tool_name: &str) -> Option<String> {
        let normalized = canonical_tool_name(tool_name);
        let mut keys = vec![normalized.clone()];
        if let Some(stripped) = normalized.strip_suffix("_result") {
            keys.push(canonical_tool_name(stripped));
        }
        for key in keys {
            if let Some(queue) = self.pending_tools.get_mut(&key)
                && let Some(tool_use_id) = queue.pop_front()
            {
                self.remove_anonymous_pending(&tool_use_id);
                return Some(tool_use_id);
            }
        }
        self.pop_any_tool_id()
    }

    fn pop_any_tool_id(&mut self) -> Option<String> {
        let tool_use_id = self.anonymous_pending.pop_front()?;
        for queue in self.pending_tools.values_mut() {
            if let Some(position) = queue.iter().position(|value| value == &tool_use_id) {
                queue.remove(position);
                break;
            }
        }
        Some(tool_use_id)
    }

    fn remove_anonymous_pending(&mut self, tool_use_id: &str) {
        if let Some(position) = self
            .anonymous_pending
            .iter()
            .position(|value| value == tool_use_id)
        {
            self.anonymous_pending.remove(position);
        }
    }

    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(crate) fn has_visible_output(&self) -> bool {
        self.visible_output
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    fn decode(contents: &str, baseline: i64) -> (TranscriptDecoder, Vec<AntigravityEvent>) {
        let rows = parse_jsonl_rows(contents);
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);
        let callback = move |event| events_for_callback.lock().unwrap().push(event);
        let mut decoder = TranscriptDecoder::new();
        decoder.apply_rows(rows, baseline, &callback);
        let values = events.lock().unwrap().clone();
        (decoder, values)
    }

    #[test]
    fn transcript_decoding_emits_structured_events_in_order() {
        let (decoder, events) = decode(
            r#"{"type":"USER_INPUT","step_index":1,"content":"hello"}
{"type":"PLANNER_RESPONSE","step_index":2,"created_at":"2026-01-01T00:00:00Z","thinking":"checking","tool_calls":[{"name":"RUN_COMMAND","args":"{\"command\":\"pwd\"}"}]}
{"type":"RUN_COMMAND","step_index":3,"created_at":"2026-01-01T00:00:01Z","content":"stdout"}
{"type":"PLANNER_RESPONSE","step_index":4,"created_at":"2026-01-01T00:00:02Z","content":"done"}
"#,
            1,
        );

        assert!(decoder.has_visible_output());
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            AntigravityEvent::ToolUse { name, input, .. }
                if name == "RUN_COMMAND" && input == &json!({"command": "pwd"})
        ));
        assert!(matches!(
            &events[1],
            AntigravityEvent::ToolResult {
                is_error: false,
                ..
            }
        ));
        assert!(matches!(
            &events[2],
            AntigravityEvent::AssistantDelta { text, reasoning, .. }
                if text == "done" && reasoning.as_deref() == Some("checking")
        ));
    }

    #[test]
    fn transcript_full_row_replaces_real_truncated_fields_schema() {
        let temp = tempfile::tempdir().expect("tempdir");
        let logs = temp.path().join("logs");
        fs::create_dir_all(&logs).expect("logs");
        let compact = logs.join("transcript.jsonl");
        let full = logs.join("transcript_full.jsonl");
        let compact_content = "x".repeat(4_096);
        let full_content = format!("{compact_content}{}", "y".repeat(512));
        fs::write(
            &compact,
            serde_json::to_string(&json!({
                "type": "PLANNER_RESPONSE",
                "step_index": 1,
                "truncated_fields": ["content"],
                "content": compact_content,
            }))
            .expect("compact json")
                + "\n",
        )
        .expect("write compact");
        fs::write(
            &full,
            serde_json::to_string(&json!({
                "type": "PLANNER_RESPONSE",
                "step_index": 1,
                "content": full_content,
            }))
            .expect("full json")
                + "\n",
        )
        .expect("write full");

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);
        let callback = move |event| events_for_callback.lock().unwrap().push(event);
        let mut decoder = TranscriptDecoder::new();
        decoder.apply_path(&compact, -1, &callback);

        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AntigravityEvent::AssistantDelta { text, .. }] if text == &full_content
        ));
    }

    #[test]
    fn list_directory_result_pairs_with_list_dir_tool_call() {
        let (_, events) = decode(
            r#"{"type":"PLANNER_RESPONSE","step_index":1,"tool_calls":[{"name":"list_dir","args":{"path":"."}}]}
{"type":"LIST_DIRECTORY","step_index":2,"content":"listing"}
"#,
            0,
        );
        let tool_use_id = match &events[0] {
            AntigravityEvent::ToolUse { tool_use_id, .. } => tool_use_id,
            other => panic!("unexpected event: {other:?}"),
        };
        let result_id = match &events[1] {
            AntigravityEvent::ToolResult { tool_use_id, .. } => tool_use_id.as_ref().unwrap(),
            other => panic!("unexpected event: {other:?}"),
        };
        assert_eq!(result_id, tool_use_id);
    }

    #[test]
    fn bare_error_is_not_visible_output_and_last_error_wins() {
        let (decoder, events) = decode(
            r#"not-json
{"type":"ERROR_MESSAGE","step_index":1,"error":"first"}
{"type":"ERROR_MESSAGE","step_index":2,"error":"second"}
"#,
            -1,
        );

        assert!(!decoder.has_visible_output());
        assert_eq!(decoder.last_error(), Some("second"));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AntigravityEvent::Error { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn duplicate_and_baseline_steps_are_not_reemitted() {
        let rows = parse_jsonl_rows(
            r#"{"type":"PLANNER_RESPONSE","step_index":1,"content":"old"}
{"type":"PLANNER_RESPONSE","step_index":2,"content":"new"}
"#,
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_callback = Arc::clone(&events);
        let callback = move |event| events_for_callback.lock().unwrap().push(event);
        let mut decoder = TranscriptDecoder::new();
        decoder.apply_rows(rows.clone(), 1, &callback);
        decoder.apply_rows(rows, 1, &callback);

        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AntigravityEvent::AssistantDelta { text, .. }] if text == "new"
        ));
    }
}
