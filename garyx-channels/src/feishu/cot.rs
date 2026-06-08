use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use garyx_models::provider::ProviderMessage;
use serde_json::{Map, Value, json};

const MAX_EVENT_CONTENT_BYTES: usize = 4096;
const MAX_TOOL_RESULT_PREVIEW_BYTES: usize = 900;
const MAX_TOOL_ARG_DISPLAY_CHARS: usize = 120;
const COT_TOOL_DISPLAY_COLS: usize = 70;
const COT_TOOL_ERROR_MAX_LINES: usize = 3;
const TRUNCATED_SUFFIX: &str = "\n...[truncated]";

pub(super) const EVENT_RUN_STARTED: &str = "RUN_STARTED";
pub(super) const EVENT_RUN_FINISHED: &str = "RUN_FINISHED";
pub(super) const EVENT_TOOL_CALL_START: &str = "TOOL_CALL_START";
pub(super) const EVENT_TOOL_CALL_ARGS: &str = "TOOL_CALL_ARGS";
pub(super) const EVENT_TOOL_CALL_END: &str = "TOOL_CALL_END";
pub(super) const EVENT_TOOL_CALL_RESULT: &str = "TOOL_CALL_RESULT";

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
    pub(super) tool_names_by_call_id: HashMap<String, String>,
    pub(super) tool_inputs_by_call_id: HashMap<String, String>,
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
        let call_id = tool_call_id(message, self.sequence + 1);
        if !self.started_tool_call_ids.insert(call_id.clone()) {
            return Vec::new();
        }

        let tool_name = tool_name(message);
        let tool_input = readable_tool_input(message);
        self.tool_names_by_call_id
            .insert(call_id.clone(), tool_name.clone());
        self.tool_inputs_by_call_id
            .insert(call_id.clone(), tool_input.clone());
        let timestamp = message_timestamp_millis(message);
        let mut events = vec![FeishuCotEventRecord::new_at(
            EVENT_TOOL_CALL_START,
            self.next_event_id(&format!("tool-start-{call_id}")),
            json!({
                "toolCallId": call_id,
                "toolCallName": truncate_chars(tool_input.trim(), MAX_TOOL_ARG_DISPLAY_CHARS)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| tool_name.clone()),
                "title": tool_title(&tool_name),
                "status": "running",
                "icon": tool_icon(&tool_name),
            }),
            timestamp,
        )];

        let args = summarize_tool_args(message);
        if !args.is_empty() {
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
        let call_id = tool_call_id(message, self.sequence + 1);
        let timestamp = message_timestamp_millis(message);
        let mut events = Vec::new();
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

        let result = summarize_tool_result(message);
        let is_error = message.is_error.unwrap_or(false);
        let tool_name = self
            .tool_names_by_call_id
            .get(&call_id)
            .cloned()
            .unwrap_or_else(|| tool_name(message));
        let tool_input = self
            .tool_inputs_by_call_id
            .get(&call_id)
            .cloned()
            .unwrap_or_else(|| readable_tool_input(message));
        events.push(FeishuCotEventRecord::new_at(
            EVENT_TOOL_CALL_RESULT,
            self.next_event_id(&format!("tool-result-{call_id}")),
            json!({
                "messageId": format!("tool-result-{call_id}"),
                "toolCallId": call_id,
                "role": "tool",
                "status": if is_error { "failed" } else { "completed" },
                "isError": is_error,
                "content": tool_result_content(&tool_name, &tool_input, &result, is_error),
            }),
            timestamp,
        ));
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

fn summarize_tool_args(message: &ProviderMessage) -> String {
    let candidate = message
        .content
        .get("command")
        .or_else(|| message.content.get("input"))
        .or_else(|| message.content.get("input_json"))
        .or_else(|| message.content.get("inputJson"))
        .or_else(|| message.content.get("args"))
        .or_else(|| message.content.get("arguments"))
        .unwrap_or(&message.content);
    preview_json(candidate, MAX_TOOL_ARG_DISPLAY_CHARS)
}

fn readable_tool_input(message: &ProviderMessage) -> String {
    let candidate = message
        .content
        .get("command")
        .or_else(|| message.content.get("input"))
        .or_else(|| message.content.get("args"))
        .or_else(|| message.content.get("arguments"))
        .or_else(|| message.content.get("input_json"))
        .or_else(|| message.content.get("inputJson"))
        .unwrap_or(&Value::Null);
    preview_json(candidate, MAX_TOOL_RESULT_PREVIEW_BYTES)
}

fn summarize_tool_result(message: &ProviderMessage) -> String {
    if is_image_generation_message(message) {
        return if message.is_error.unwrap_or(false) {
            "Image generation failed.".to_owned()
        } else {
            "Image generated.".to_owned()
        };
    }
    if let Some(text) = message.text.as_deref() {
        return truncate_utf8_bytes(text.trim(), MAX_TOOL_RESULT_PREVIEW_BYTES);
    }
    let candidate = message
        .content
        .get("output")
        .or_else(|| message.content.get("result"))
        .or_else(|| message.content.get("message"))
        .or_else(|| message.content.get("content"))
        .unwrap_or(&message.content);
    preview_json(candidate, MAX_TOOL_RESULT_PREVIEW_BYTES)
}

fn is_image_generation_message(message: &ProviderMessage) -> bool {
    let metadata_type = message
        .metadata
        .get("item_type")
        .or_else(|| message.metadata.get("itemType"))
        .and_then(Value::as_str);
    if matches!(metadata_type, Some("imageGeneration")) {
        return true;
    }
    let content_type = message
        .content
        .get("type")
        .or_else(|| message.content.get("item_type"))
        .or_else(|| message.content.get("itemType"))
        .and_then(Value::as_str);
    matches!(content_type, Some("imageGeneration"))
        || message.tool_name.as_deref() == Some("imageGeneration")
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

fn tool_result_content(tool_name: &str, tool_input: &str, output: &str, is_error: bool) -> Value {
    let shown = clamp_tool_text(
        output,
        if is_error {
            COT_TOOL_ERROR_MAX_LINES
        } else {
            1
        },
    );
    if !is_error && is_code_output_tool(tool_name) {
        let header = invocation_header(tool_name, tool_input);
        return json!({
            "type": "code",
            "code": format!("{header}{shown}"),
        });
    }
    json!({
        "type": "text",
        "text": shown,
    })
}

fn invocation_header(tool_name: &str, tool_input: &str) -> String {
    let trimmed = tool_input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let (line, was_cut) = clamp_cols(trimmed, COT_TOOL_DISPLAY_COLS);
    let prefix = if contains_any(
        &tool_name.to_ascii_lowercase(),
        &["bash", "shell", "exec", "run"],
    ) {
        "$"
    } else {
        "#"
    };
    format!("{prefix} {line}{}\n", if was_cut { "..." } else { "" })
}

fn clamp_tool_text(text: &str, max_lines: usize) -> String {
    let lines = text
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let mut kept = Vec::new();
    for line in lines.iter().take(max_lines) {
        let (clamped, was_cut) = clamp_cols(line.trim_end(), COT_TOOL_DISPLAY_COLS);
        kept.push(format!("{clamped}{}", if was_cut { "..." } else { "" }));
    }
    let omitted = lines.len().saturating_sub(kept.len());
    if omitted > 0
        && let Some(last) = kept.last_mut()
    {
        last.push_str(&format!(" ...(+{omitted} lines)"));
    }
    kept.join("\n")
}

fn clamp_cols(text: &str, max_cols: usize) -> (String, bool) {
    let mut used = 0;
    let mut out = String::new();
    for ch in text.chars() {
        used += char_cols(ch);
        if used > max_cols {
            return (out, true);
        }
        out.push(ch);
    }
    (out, false)
}

fn char_cols(ch: char) -> usize {
    if matches!(
        ch,
        '\u{1100}'..='\u{115f}'
            | '\u{2e80}'..='\u{a4cf}'
            | '\u{ac00}'..='\u{d7a3}'
            | '\u{f900}'..='\u{faff}'
            | '\u{fe10}'..='\u{fe19}'
            | '\u{fe30}'..='\u{fe6f}'
            | '\u{ff00}'..='\u{ff60}'
            | '\u{ffe0}'..='\u{ffe6}'
    ) {
        2
    } else {
        1
    }
}

fn is_code_output_tool(tool_name: &str) -> bool {
    contains_any(
        &tool_name.to_ascii_lowercase(),
        &[
            "bash", "shell", "exec", "run", "read", "grep", "glob", "edit", "write",
        ],
    )
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn truncate_chars(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    if trimmed.chars().count() > max_chars {
        out.push('…');
    }
    Some(out)
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
    fn tool_result_summary_prefers_output_field() {
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
        assert_eq!(events[0].event_type, EVENT_TOOL_CALL_END);
        assert_eq!(events[1].event_type, EVENT_TOOL_CALL_RESULT);
        let content = content_json(&events[1]);
        assert_eq!(content["role"], "tool");
        assert_eq!(content["isError"], false);
        assert_eq!(content["content"]["type"], "code");
        assert_eq!(content["content"]["code"], "$ pwd\n/tmp/workspace");
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
        assert_eq!(start["toolCallName"], "pwd");
        assert_eq!(start["title"], "运行命令");
        assert_eq!(start["icon"], "bash");
        assert_eq!(start["status"], "running");
        let args = content_json(&events[1]);
        assert_eq!(args["toolCallId"], "tool-1");
        assert_eq!(args["delta"], "pwd");
    }

    #[test]
    fn image_generation_tool_result_does_not_embed_base64() {
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
        let result_event = events
            .iter()
            .find(|event| event.event_type == EVENT_TOOL_CALL_RESULT)
            .expect("tool result event");
        let content = content_json(result_event);
        assert_eq!(content["content"]["type"], "text");
        assert_eq!(content["content"]["text"], "Image generated.");
        assert!(!result_event.content.contains("iVBORw0KGgo="));
    }
}
