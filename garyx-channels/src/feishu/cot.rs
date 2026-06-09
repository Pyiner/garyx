use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use garyx_models::provider::ProviderMessage;
use serde_json::{Map, Value, json};

const MAX_EVENT_CONTENT_BYTES: usize = 4096;
const MAX_TOOL_ARG_DISPLAY_CHARS: usize = 120;
const TRUNCATED_SUFFIX: &str = "\n...[truncated]";

pub(super) const EVENT_RUN_STARTED: &str = "RUN_STARTED";
pub(super) const EVENT_RUN_FINISHED: &str = "RUN_FINISHED";
pub(super) const EVENT_TOOL_CALL_START: &str = "TOOL_CALL_START";
pub(super) const EVENT_TOOL_CALL_ARGS: &str = "TOOL_CALL_ARGS";
pub(super) const EVENT_TOOL_CALL_END: &str = "TOOL_CALL_END";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FeishuCotSession {
    pub(super) cot_id: String,
    pub(super) message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FeishuCotEventRecord {
    pub(super) event_id: String,
    pub(super) event_type: String,
    pub(super) timestamp: u64,
    pub(super) content: String,
}

impl FeishuCotEventRecord {
    pub(super) fn new(
        event_type: impl Into<String>,
        event_id: impl Into<String>,
        content: Value,
    ) -> Self {
        Self::new_at(event_type, event_id, content, current_timestamp_millis())
    }

    pub(super) fn new_at(
        event_type: impl Into<String>,
        event_id: impl Into<String>,
        content: Value,
        timestamp: u64,
    ) -> Self {
        Self {
            event_id: sanitize_event_id_part(&event_id.into()),
            event_type: event_type.into(),
            timestamp,
            content: stringify_event_content(content),
        }
    }

    pub(super) fn to_openapi(&self) -> Value {
        json!({
            "event_id": self.event_id,
            "event_type": self.event_type,
            "timestamp": self.timestamp,
            "content": self.content,
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct FeishuCotState {
    pub(super) session: Option<FeishuCotSession>,
    pub(super) failed: bool,
    pub(super) completed: bool,
    pub(super) sequence: u64,
    pub(super) started_tool_call_ids: HashSet<String>,
    pub(super) ended_tool_call_ids: HashSet<String>,
    pub(super) arg_sent_tool_call_ids: HashSet<String>,
}

impl FeishuCotState {
    fn next_event_id(&mut self, prefix: &str) -> String {
        self.sequence += 1;
        format!("{}-{}", sanitize_event_id_part(prefix), self.sequence)
    }

    pub(super) fn run_started_event(
        &mut self,
        thread_id: &str,
        run_id: &str,
    ) -> FeishuCotEventRecord {
        let event_id = self.next_event_id("run-started");
        FeishuCotEventRecord::new(
            EVENT_RUN_STARTED,
            event_id,
            json!({
                "threadId": thread_id,
                "runId": run_id,
            }),
        )
    }

    pub(super) fn run_finished_event(
        &mut self,
        thread_id: &str,
        run_id: &str,
    ) -> FeishuCotEventRecord {
        let event_id = self.next_event_id("run-finished");
        FeishuCotEventRecord::new(
            EVENT_RUN_FINISHED,
            event_id,
            json!({
                "threadId": thread_id,
                "runId": run_id,
                "status": "completed",
            }),
        )
    }

    pub(super) fn tool_use_events(
        &mut self,
        message: &ProviderMessage,
    ) -> Vec<FeishuCotEventRecord> {
        if is_hidden_cot_tool(message) {
            return Vec::new();
        }

        let call_id = tool_call_id(message, self.sequence + 1);
        if !self.started_tool_call_ids.insert(call_id.clone()) {
            return Vec::new();
        }

        let tool_name = tool_name(message);
        let timestamp = message_timestamp_millis(message);
        let tool_display_name = tool_title(&tool_name);
        let mut events = vec![FeishuCotEventRecord::new_at(
            EVENT_TOOL_CALL_START,
            self.next_event_id(&format!("tool-start-{call_id}")),
            json!({
                "toolCallId": call_id,
                "toolCallName": tool_display_name,
                "title": tool_display_name,
                "status": "running",
                "icon": tool_icon(&tool_name),
            }),
            timestamp,
        )];

        let args = summarize_tool_args(message);
        if !args.is_empty() {
            self.arg_sent_tool_call_ids.insert(call_id.clone());
            events.push(FeishuCotEventRecord::new_at(
                EVENT_TOOL_CALL_ARGS,
                self.next_event_id(&format!("tool-args-{call_id}")),
                json!({
                    "toolCallId": call_id,
                    "delta": args,
                }),
                timestamp,
            ));
        }
        events
    }

    pub(super) fn tool_result_events(
        &mut self,
        message: &ProviderMessage,
    ) -> Vec<FeishuCotEventRecord> {
        if is_hidden_cot_tool(message) {
            return Vec::new();
        }

        let call_id = tool_call_id(message, self.sequence + 1);
        let timestamp = message_timestamp_millis(message);
        let mut events = Vec::new();
        if !self.arg_sent_tool_call_ids.contains(&call_id) {
            let args = readable_tool_argument(message).unwrap_or_default();
            if !args.is_empty() {
                self.arg_sent_tool_call_ids.insert(call_id.clone());
                events.push(FeishuCotEventRecord::new_at(
                    EVENT_TOOL_CALL_ARGS,
                    self.next_event_id(&format!("tool-args-{call_id}")),
                    json!({
                        "toolCallId": call_id,
                        "delta": args,
                    }),
                    timestamp,
                ));
            }
        }
        if self.ended_tool_call_ids.insert(call_id.clone()) {
            events.push(FeishuCotEventRecord::new_at(
                EVENT_TOOL_CALL_END,
                self.next_event_id(&format!("tool-end-{call_id}")),
                json!({
                    "toolCallId": call_id,
                }),
                timestamp,
            ));
        }
        events
    }
}

fn current_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn message_timestamp_millis(message: &ProviderMessage) -> u64 {
    message
        .timestamp
        .as_deref()
        .and_then(parse_timestamp_millis)
        .unwrap_or_else(current_timestamp_millis)
}

fn parse_timestamp_millis(value: &str) -> Option<u64> {
    let timestamp = value.trim().parse::<u64>().ok()?;
    if timestamp < 1_000_000_000_000 {
        timestamp.checked_mul(1000)
    } else {
        Some(timestamp)
    }
}

fn stringify_event_content(content: Value) -> String {
    let serialized = serde_json::to_string(&content).unwrap_or_else(|_| "{}".to_owned());
    if fits_event_content(&serialized) {
        return serialized;
    }

    let Value::Object(map) = content else {
        return truncate_utf8_bytes(&serialized, MAX_EVENT_CONTENT_BYTES);
    };

    let mut fitted = map;
    shrink_longest_string_fields(&mut fitted);
    let serialized =
        serde_json::to_string(&Value::Object(fitted.clone())).unwrap_or_else(|_| "{}".to_owned());
    if fits_event_content(&serialized) {
        return serialized;
    }

    json!({
        "truncated": true,
        "preview": truncate_utf8_bytes(&serialized, MAX_EVENT_CONTENT_BYTES / 2),
    })
    .to_string()
}

fn shrink_longest_string_fields(map: &mut Map<String, Value>) {
    let mut keys = map
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|text| (key.clone(), text.len())))
        .collect::<Vec<_>>();
    keys.sort_by(|left, right| right.1.cmp(&left.1));

    for (key, len) in keys {
        let Some(Value::String(text)) = map.get_mut(&key) else {
            continue;
        };
        let next_len = (len / 2).max(64);
        *text = truncate_utf8_bytes(text, next_len);
        let serialized =
            serde_json::to_string(&Value::Object(map.clone())).unwrap_or_else(|_| "{}".to_owned());
        if fits_event_content(&serialized) {
            return;
        }
    }
}

fn fits_event_content(text: &str) -> bool {
    text.len() <= MAX_EVENT_CONTENT_BYTES
}

fn tool_call_id(message: &ProviderMessage, fallback: u64) -> String {
    message
        .tool_use_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(sanitize_event_id_part)
        .unwrap_or_else(|| format!("tool-call-{fallback}"))
}

fn tool_name(message: &ProviderMessage) -> String {
    message
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            message
                .content
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "tool".to_owned())
}

fn is_hidden_cot_tool(message: &ProviderMessage) -> bool {
    let tool_name = message.tool_name.as_deref().unwrap_or_default().trim();
    let metadata_item_type = message
        .metadata
        .get("item_type")
        .or_else(|| message.metadata.get("itemType"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let content_type = message
        .content
        .get("type")
        .or_else(|| message.content.get("item_type"))
        .or_else(|| message.content.get("itemType"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();

    [tool_name, metadata_item_type, content_type]
        .iter()
        .any(|value| value.eq_ignore_ascii_case("reasoning"))
}

fn summarize_tool_args(message: &ProviderMessage) -> String {
    if let Some(args) = readable_tool_argument(message) {
        return args;
    }
    let candidate = default_tool_args_candidate(message);
    if is_uninformative_tool_args(candidate) {
        return String::new();
    }
    preview_json(candidate, MAX_TOOL_ARG_DISPLAY_CHARS)
}

fn is_image_view_message(message: &ProviderMessage) -> bool {
    let metadata_type = message
        .metadata
        .get("item_type")
        .or_else(|| message.metadata.get("itemType"))
        .and_then(Value::as_str);
    if metadata_type.is_some_and(|value| value.eq_ignore_ascii_case("imageView")) {
        return true;
    }
    let content_type = message
        .content
        .get("type")
        .or_else(|| message.content.get("item_type"))
        .or_else(|| message.content.get("itemType"))
        .and_then(Value::as_str);
    content_type.is_some_and(|value| value.eq_ignore_ascii_case("imageView"))
        || message
            .tool_name
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("imageView"))
}

fn image_view_summary(message: &ProviderMessage) -> String {
    let Some(path) = message.content.get("path").and_then(Value::as_str) else {
        return "查看图片".to_owned();
    };
    let file_name = path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path);
    truncate_utf8_bytes(file_name.trim(), MAX_TOOL_ARG_DISPLAY_CHARS)
}

fn readable_tool_argument(message: &ProviderMessage) -> Option<String> {
    if is_image_view_message(message) {
        return Some(image_view_summary(message));
    }

    if is_command_like_tool(message)
        && let Some(command) = command_from_value(&message.content)
    {
        return Some(truncate_utf8_bytes(
            command.trim(),
            MAX_TOOL_ARG_DISPLAY_CHARS,
        ));
    }

    let tool = tool_name(message).to_ascii_lowercase();
    if contains_any(&tool, &["read", "write", "edit", "grep", "glob"]) {
        if let Some(path) = readable_value_from_keys(
            &message.content,
            &[
                "file_path",
                "filePath",
                "filepath",
                "path",
                "paths",
                "filename",
                "file",
                "pattern",
                "glob",
            ],
        ) {
            return Some(path);
        }
    }

    if contains_any(&tool, &["search", "webfetch", "fetch"]) {
        if let Some(query) =
            readable_value_from_keys(&message.content, &["query", "queries", "url", "urls"])
        {
            return Some(query);
        }
    }

    if contains_any(&tool, &["task", "skill"]) {
        if let Some(prompt) = readable_value_from_keys(
            &message.content,
            &["prompt", "task", "title", "description", "name", "skill"],
        ) {
            return Some(prompt);
        }
    }

    readable_value_from_keys(
        &message.content,
        &[
            "file_path",
            "filePath",
            "path",
            "url",
            "query",
            "queries",
            "pattern",
            "prompt",
            "title",
            "name",
        ],
    )
}

fn readable_value_from_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
                && let Some(found) = readable_value_from_keys(&parsed, keys)
            {
                return Some(found);
            }
            None
        }
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key) {
                    return value_to_readable_text_for_key(value, key);
                }
            }
            for key in [
                "input_json",
                "inputJson",
                "input",
                "args",
                "arguments",
                "parameters",
                "params",
                "action",
            ] {
                if let Some(value) = map.get(key)
                    && let Some(found) = readable_value_from_keys(value, keys)
                {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn value_to_readable_text_for_key(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Array(values) => {
            let parts = values
                .iter()
                .filter_map(|item| value_to_readable_text_for_key(item, key))
                .filter(|part| !part.trim().is_empty())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        }
        _ => value_to_readable_text(value).map(|text| format_readable_value_for_key(key, &text)),
    }
}

fn format_readable_value_for_key(key: &str, text: &str) -> String {
    let trimmed = text.trim();
    if is_path_like_key(key) {
        return truncate_path_for_display(trimmed, MAX_TOOL_ARG_DISPLAY_CHARS);
    }
    truncate_utf8_bytes(trimmed, MAX_TOOL_ARG_DISPLAY_CHARS)
}

fn is_path_like_key(key: &str) -> bool {
    matches!(
        key,
        "file_path" | "filePath" | "filepath" | "path" | "paths" | "filename" | "file"
    )
}

fn truncate_path_for_display(path: &str, max_bytes: usize) -> String {
    if path.len() <= max_bytes {
        return path.to_owned();
    }

    let normalized = path.trim_end_matches('/');
    let mut parts = normalized.rsplit('/').filter(|part| !part.is_empty());
    let Some(file_name) = parts.next() else {
        return truncate_utf8_bytes(path, max_bytes);
    };

    if let Some(parent) = parts.next() {
        let parent_tail = format!(".../{parent}/{file_name}");
        if parent_tail.len() <= max_bytes {
            return parent_tail;
        }
    }

    let file_tail = format!(".../{file_name}");
    if file_tail.len() <= max_bytes {
        return file_tail;
    }

    let filename_budget = max_bytes.saturating_sub(".../".len());
    format!(
        ".../{}",
        middle_elide_utf8_bytes(file_name, filename_budget)
    )
}

fn command_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
                && let Some(command) = command_from_value(&parsed)
            {
                return Some(command);
            }
            Some(trimmed.to_owned())
        }
        Value::Object(map) => [
            "command",
            "cmd",
            "script",
            "shell",
            "bash",
            "input",
            "input_json",
            "inputJson",
            "args",
            "arguments",
            "parameters",
            "params",
        ]
        .iter()
        .filter_map(|key| map.get(*key))
        .find_map(command_from_value),
        _ => None,
    }
}

fn value_to_readable_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Value::Array(values) => {
            let parts = values
                .iter()
                .filter_map(value_to_readable_text)
                .filter(|part| !part.trim().is_empty())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        }
        _ => Some(preview_json(value, MAX_TOOL_ARG_DISPLAY_CHARS)),
    }
}

fn default_tool_args_candidate(message: &ProviderMessage) -> &Value {
    message
        .content
        .get("command")
        .or_else(|| message.content.get("input"))
        .or_else(|| message.content.get("input_json"))
        .or_else(|| message.content.get("inputJson"))
        .or_else(|| message.content.get("args"))
        .or_else(|| message.content.get("arguments"))
        .unwrap_or(&message.content)
}

fn is_uninformative_tool_args(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Object(map) => {
            map.is_empty()
                || map.keys().all(|key| {
                    matches!(
                        key.as_str(),
                        "type" | "item_type" | "itemType" | "id" | "status"
                    )
                })
        }
        Value::Array(values) => values.is_empty(),
        _ => false,
    }
}

fn is_command_like_tool(message: &ProviderMessage) -> bool {
    let tool_name = tool_name(message).to_ascii_lowercase();
    if contains_any(&tool_name, &["bash", "shell", "exec", "run", "command"]) {
        return true;
    }

    let content_type = message
        .content
        .get("type")
        .or_else(|| message.content.get("item_type"))
        .or_else(|| message.content.get("itemType"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    contains_any(&content_type, &["command", "bash", "shell", "exec", "run"])
}

fn preview_json(value: &Value, max_bytes: usize) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => truncate_utf8_bytes(text.trim(), max_bytes),
        _ => serde_json::to_string(value)
            .map(|text| truncate_utf8_bytes(text.trim(), max_bytes))
            .unwrap_or_default(),
    }
}

fn tool_title(tool_name: &str) -> String {
    let value = tool_name.to_ascii_lowercase();
    if contains_any(&value, &["bash", "shell", "exec", "run"]) {
        "运行命令".to_owned()
    } else if contains_any(&value, &["read"]) {
        "读取文件".to_owned()
    } else if contains_any(&value, &["write"]) {
        "写入文件".to_owned()
    } else if contains_any(&value, &["edit"]) {
        "编辑文件".to_owned()
    } else if contains_any(&value, &["grep", "glob"]) {
        "检索代码".to_owned()
    } else if contains_any(&value, &["skill"]) {
        "加载技能".to_owned()
    } else if contains_any(&value, &["task"]) {
        "派发子任务".to_owned()
    } else if contains_any(&value, &["search"]) {
        "搜索".to_owned()
    } else if contains_any(&value, &["webfetch", "fetch"]) {
        "抓取网页".to_owned()
    } else if value.eq_ignore_ascii_case("imageview") {
        "ImageView".to_owned()
    } else {
        tool_name.trim().to_owned()
    }
}

fn tool_icon(tool_name: &str) -> &'static str {
    let value = tool_name.to_ascii_lowercase();
    if contains_any(&value, &["search", "web"]) {
        "search"
    } else if contains_any(&value, &["read", "grep", "glob"]) {
        "read"
    } else if value.eq_ignore_ascii_case("imageview") {
        "read"
    } else if contains_any(&value, &["write", "edit"]) {
        "write"
    } else if contains_any(
        &value,
        &["run", "exec", "bash", "shell", "skill", "mcp", "task"],
    ) {
        "bash"
    } else {
        "default"
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn sanitize_event_id_part(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "event".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn truncate_utf8_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let suffix_len = TRUNCATED_SUFFIX.len().min(max_bytes);
    let mut end = max_bytes.saturating_sub(suffix_len);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &text[..end], TRUNCATED_SUFFIX)
}

fn middle_elide_utf8_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    const MARKER: &str = "...";
    if max_bytes <= MARKER.len() {
        return truncate_utf8_bytes(text, max_bytes);
    }

    let keep = max_bytes - MARKER.len();
    let mut head = keep / 2;
    let mut tail = keep - head;
    while head > 0 && !text.is_char_boundary(head) {
        head -= 1;
        tail += 1;
    }
    while tail > 0 && !text.is_char_boundary(text.len() - tail) {
        tail -= 1;
    }
    format!("{}{}{}", &text[..head], MARKER, &text[text.len() - tail..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_json(event: &FeishuCotEventRecord) -> Value {
        serde_json::from_str(&event.content).expect("event content json")
    }

    #[test]
    fn event_content_is_capped_by_bytes() {
        let content = stringify_event_content(json!({
            "messageId": "tool-result-1",
            "toolCallId": "tool-1",
            "role": "tool",
            "content": "中".repeat(5000),
        }));
        assert!(content.len() <= MAX_EVENT_CONTENT_BYTES);
        assert!(content.contains("truncated") || content.contains("[truncated]"));
    }

    #[test]
    fn tool_result_events_backfill_args_without_result_payload() {
        let message = ProviderMessage::tool_result(
            json!({
                "command": "pwd",
                "output": "/tmp/workspace",
            }),
            Some("tool-1".to_owned()),
            Some("shell".to_owned()),
            Some(false),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_result_events(&message);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, EVENT_TOOL_CALL_ARGS);
        let args = content_json(&events[0]);
        assert_eq!(args["delta"], "pwd");
        assert_eq!(events[1].event_type, EVENT_TOOL_CALL_END);
        assert!(
            events
                .iter()
                .all(|event| !event.content.contains("/tmp/workspace"))
        );
    }

    #[test]
    fn tool_use_uses_readable_name_title_icon_and_millisecond_timestamp() {
        let message = ProviderMessage::tool_use(
            json!({
                "type": "commandExecution",
                "command": "pwd",
            }),
            Some("tool-1".to_owned()),
            Some("shell".to_owned()),
        )
        .with_timestamp("1713200000");
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        assert_eq!(events[0].timestamp, 1_713_200_000_000);
        let start = content_json(&events[0]);
        assert_eq!(start["toolCallId"], "tool-1");
        assert_eq!(start["toolCallName"], "运行命令");
        assert_eq!(start["title"], "运行命令");
        assert_eq!(start["icon"], "bash");
        assert_eq!(start["status"], "running");
        let args = content_json(&events[1]);
        assert_eq!(args["toolCallId"], "tool-1");
        assert_eq!(args["delta"], "pwd");
    }

    #[test]
    fn tool_use_does_not_surface_internal_tool_call_id_as_display_name() {
        let message = ProviderMessage::tool_use(
            json!({
                "output": "garyx-channels/src/feishu/client.rs",
            }),
            Some("tool-call-exec-command-1".to_owned()),
            Some("tool-call-exec-command-1".to_owned()),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        let start = content_json(&events[0]);
        assert_eq!(start["toolCallId"], "tool-call-exec-command-1");
        assert_eq!(start["toolCallName"], "运行命令");
        assert_ne!(start["toolCallName"], "tool-call-exec-command-1");
    }

    #[test]
    fn command_tool_args_extract_nested_command_without_json_wrapper() {
        let message = ProviderMessage::tool_use(
            json!({
                "type": "commandExecution",
                "input_json": {
                    "command": "echo \"=== knowledge-base/prds/ ===\" && ls /Users/test/prds",
                },
            }),
            Some("tool-command-1".to_owned()),
            Some("tool-call-exec-command-1".to_owned()),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        let args = content_json(&events[1]);
        assert_eq!(
            args["delta"],
            "echo \"=== knowledge-base/prds/ ===\" && ls /Users/test/prds"
        );
        assert!(!args["delta"].as_str().unwrap_or_default().contains("{"));

        let result = ProviderMessage::tool_result(
            json!({
                "type": "commandExecution",
                "input_json": {
                    "command": "echo \"=== knowledge-base/prds/ ===\" && ls /Users/test/prds",
                },
                "output": "=== knowledge-base/prds/ ===\nexample.md",
            }),
            Some("tool-command-1".to_owned()),
            Some("tool-call-exec-command-1".to_owned()),
            Some(false),
        );
        let result_events = state.tool_result_events(&result);
        assert_eq!(result_events.len(), 1);
        assert_eq!(result_events[0].event_type, EVENT_TOOL_CALL_END);
        assert!(!result_events[0].content.contains("example.md"));
    }

    #[test]
    fn read_file_tool_args_extract_file_path_without_json_wrapper() {
        let message = ProviderMessage::tool_use(
            json!({
                "type": "readFile",
                "input_json": {
                    "file_path": "/Users/test/workspace/src/main.rs",
                },
            }),
            Some("tool-read-1".to_owned()),
            Some("read_file".to_owned()),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        let start = content_json(&events[0]);
        assert_eq!(start["toolCallName"], "读取文件");
        let args = content_json(&events[1]);
        assert_eq!(args["delta"], "/Users/test/workspace/src/main.rs");
        assert!(!args["delta"].as_str().unwrap_or_default().contains("{"));
    }

    #[test]
    fn read_file_tool_args_extract_claude_sdk_input_file_path() {
        let message = ProviderMessage::tool_use(
            json!({
                "tool": "Read",
                "input": {
                    "file_path": "/Users/test/workspace/references/schema.md",
                },
            }),
            Some("tool-read-claude".to_owned()),
            Some("Read".to_owned()),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        let start = content_json(&events[0]);
        assert_eq!(start["toolCallName"], "读取文件");
        let args = content_json(&events[1]);
        assert_eq!(args["delta"], "/Users/test/workspace/references/schema.md");
        assert!(!args["delta"].as_str().unwrap_or_default().contains("{"));
    }

    #[test]
    fn read_file_tool_args_preserve_filename_for_long_temp_paths() {
        let message = ProviderMessage::tool_use(
            json!({
                "tool": "Read",
                "input": {
                    "file_path": "/var/folders/test/session/T/garyx-feishu/inbound/12345678-1234-1234-1234-123456789abc-feishu-img_v3_0212g_98bd011e-4897-4f72-80ed-5348e3f2612g.jpeg",
                },
            }),
            Some("tool-read-long-path".to_owned()),
            Some("Read".to_owned()),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        let args = content_json(&events[1]);
        let delta = args["delta"].as_str().unwrap_or_default();
        assert!(delta.starts_with(".../inbound/"), "delta={delta}");
        assert!(delta.contains("feishu-img_v3_0212g"), "delta={delta}");
        assert!(delta.ends_with(".jpeg"), "delta={delta}");
        assert!(!delta.contains("[truncated]"), "delta={delta}");
    }

    #[test]
    fn tool_result_backfills_missing_args_without_result_payload() {
        let use_message = ProviderMessage::tool_use(
            json!({
                "type": "readFile",
            }),
            Some("tool-read-2".to_owned()),
            Some("read_file".to_owned()),
        );
        let result_message = ProviderMessage::tool_result(
            json!({
                "file_path": "/Users/test/workspace/src/lib.rs",
                "data": "secret file contents",
            }),
            Some("tool-read-2".to_owned()),
            Some("read_file".to_owned()),
            Some(false),
        );
        let mut state = FeishuCotState::default();
        let use_events = state.tool_use_events(&use_message);
        assert_eq!(use_events.len(), 1, "ToolUse has no args to display");

        let result_events = state.tool_result_events(&result_message);
        assert_eq!(result_events.len(), 2);
        assert_eq!(result_events[0].event_type, EVENT_TOOL_CALL_ARGS);
        let args = content_json(&result_events[0]);
        assert_eq!(args["delta"], "/Users/test/workspace/src/lib.rs");
        assert_eq!(result_events[1].event_type, EVENT_TOOL_CALL_END);
        assert!(
            result_events
                .iter()
                .all(|event| !event.content.contains("secret file contents"))
        );
    }

    #[test]
    fn search_tool_args_extract_queries_without_json_wrapper() {
        let message = ProviderMessage::tool_use(
            json!({
                "action": {
                    "queries": ["EPM LAPS password", "macOS Secure Token"],
                },
            }),
            Some("tool-search-1".to_owned()),
            Some("webSearch".to_owned()),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        let start = content_json(&events[0]);
        assert_eq!(start["toolCallName"], "搜索");
        let args = content_json(&events[1]);
        assert_eq!(args["delta"], "EPM LAPS password, macOS Secure Token");
        assert!(!args["delta"].as_str().unwrap_or_default().contains("{"));
    }

    #[test]
    fn image_generation_tool_result_only_closes_without_base64() {
        let message = ProviderMessage::tool_result(
            json!({
                "type": "imageGeneration",
                "result": "iVBORw0KGgo=".repeat(200),
            }),
            Some("image-tool-1".to_owned()),
            Some("imageGeneration".to_owned()),
            Some(false),
        )
        .with_metadata_value("item_type", json!("imageGeneration"));
        let mut state = FeishuCotState::default();
        let events = state.tool_result_events(&message);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EVENT_TOOL_CALL_END);
        assert!(!events[0].content.contains("iVBORw0KGgo="));
    }

    #[test]
    fn reasoning_items_do_not_emit_cot_tool_events() {
        let reasoning = ProviderMessage::tool_use(
            json!({
                "type": "reasoning",
                "id": "rs_home_log",
                "summary": [],
                "content": [],
            }),
            Some("rs_home_log".to_owned()),
            Some("reasoning".to_owned()),
        )
        .with_metadata_value("item_type", json!("reasoning"));
        let reasoning_result = ProviderMessage::tool_result(
            reasoning.content.clone(),
            reasoning.tool_use_id.clone(),
            reasoning.tool_name.clone(),
            Some(false),
        )
        .with_metadata_value("item_type", json!("reasoning"));

        let mut state = FeishuCotState::default();
        assert!(state.tool_use_events(&reasoning).is_empty());
        assert!(state.tool_result_events(&reasoning_result).is_empty());
        assert!(state.started_tool_call_ids.is_empty());
        assert!(state.ended_tool_call_ids.is_empty());
    }

    #[test]
    fn image_view_items_emit_readable_cot_tool_events() {
        let image_view = ProviderMessage::tool_use(
            json!({
                "type": "imageView",
                "id": "call_home_image",
                "path": "/var/folders/example/file_131.jpg",
            }),
            Some("call_home_image".to_owned()),
            Some("imageView".to_owned()),
        )
        .with_metadata_value("item_type", json!("imageView"));
        let image_result = ProviderMessage::tool_result(
            image_view.content.clone(),
            image_view.tool_use_id.clone(),
            image_view.tool_name.clone(),
            Some(false),
        )
        .with_metadata_value("item_type", json!("imageView"));

        let mut state = FeishuCotState::default();
        let use_events = state.tool_use_events(&image_view);
        assert_eq!(use_events.len(), 2);
        let start = content_json(&use_events[0]);
        assert_eq!(start["toolCallId"], "call_home_image");
        assert_eq!(start["toolCallName"], "ImageView");
        assert_eq!(start["title"], "ImageView");
        assert_eq!(start["icon"], "read");
        let args = content_json(&use_events[1]);
        assert_eq!(args["delta"], "file_131.jpg");

        let result_events = state.tool_result_events(&image_result);
        assert_eq!(result_events.len(), 1);
        assert_eq!(result_events[0].event_type, EVENT_TOOL_CALL_END);
        assert!(!result_events[0].content.contains("file_131.jpg"));
    }
}
