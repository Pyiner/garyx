use super::cot_render::{
    readable_tool_argument, sanitize_event_id_part, split_utf8_bytes, stringify_event_content,
    summarize_tool_args, tool_icon, tool_parameter_result_content, tool_title,
};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use garyx_models::provider::ProviderMessage;
use serde_json::{Value, json};

pub(super) const MAX_EVENT_CONTENT_BYTES: usize = 4096;
const MAX_TEXT_DELTA_BYTES: usize = 3500;
pub(super) const MAX_TOOL_ARG_DISPLAY_CHARS: usize = 120;
pub(super) const TRUNCATED_SUFFIX: &str = "\n...[truncated]";

pub(super) const EVENT_RUN_STARTED: &str = "RUN_STARTED";
pub(super) const EVENT_RUN_FINISHED: &str = "RUN_FINISHED";
pub(super) const EVENT_TEXT_MESSAGE_START: &str = "TEXT_MESSAGE_START";
pub(super) const EVENT_TEXT_MESSAGE_CONTENT: &str = "TEXT_MESSAGE_CONTENT";
pub(super) const EVENT_TEXT_MESSAGE_END: &str = "TEXT_MESSAGE_END";
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
    pub(super) result_sent_tool_call_ids: HashSet<String>,
    pub(super) arg_sent_tool_call_ids: HashSet<String>,
    pub(super) tool_call_args_by_id: HashMap<String, String>,
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

    pub(super) fn text_message_events(&mut self, text: &str) -> Vec<FeishuCotEventRecord> {
        let text = text.trim();
        if text.is_empty() {
            return Vec::new();
        }

        let message_id = self.next_event_id("text");
        let mut events = Vec::new();
        events.push(FeishuCotEventRecord::new(
            EVENT_TEXT_MESSAGE_START,
            self.next_event_id(&format!("{message_id}-start")),
            json!({
                "messageId": message_id.clone(),
                "role": "assistant",
            }),
        ));
        for (idx, delta) in split_utf8_bytes(text, MAX_TEXT_DELTA_BYTES)
            .into_iter()
            .enumerate()
        {
            events.push(FeishuCotEventRecord::new(
                EVENT_TEXT_MESSAGE_CONTENT,
                self.next_event_id(&format!("{message_id}-content-{idx}")),
                json!({
                    "messageId": message_id.clone(),
                    "delta": delta,
                }),
            ));
        }
        events.push(FeishuCotEventRecord::new(
            EVENT_TEXT_MESSAGE_END,
            self.next_event_id(&format!("{message_id}-end")),
            json!({
                "messageId": message_id,
            }),
        ));
        events
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
        let args = summarize_tool_args(message);
        let tool_call_name = readable_tool_argument(message)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| tool_display_name.clone());
        let mut events = vec![FeishuCotEventRecord::new_at(
            EVENT_TOOL_CALL_START,
            self.next_event_id(&format!("tool-start-{call_id}")),
            json!({
                "toolCallId": call_id,
                "toolCallName": tool_call_name,
                "title": tool_display_name,
                "icon": tool_icon(&tool_name),
            }),
            timestamp,
        )];

        if !args.is_empty() {
            self.tool_call_args_by_id
                .insert(call_id.clone(), args.clone());
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
                self.tool_call_args_by_id
                    .insert(call_id.clone(), args.clone());
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
        if self.result_sent_tool_call_ids.insert(call_id.clone())
            && let Some(args) = self
                .tool_call_args_by_id
                .get(&call_id)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        {
            events.push(FeishuCotEventRecord::new_at(
                EVENT_TOOL_CALL_RESULT,
                self.next_event_id(&format!("tool-result-{call_id}")),
                json!({
                    "messageId": format!("tool-result-{call_id}"),
                    "toolCallId": call_id,
                    "role": "tool",
                    "content": tool_parameter_result_content(message, &args),
                    "isError": false,
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

fn tool_call_id(message: &ProviderMessage, fallback: u64) -> String {
    message
        .tool_use_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(sanitize_event_id_part)
        .unwrap_or_else(|| format!("tool-call-{fallback}"))
}

pub(super) fn tool_name(message: &ProviderMessage) -> String {
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

/// Tool-row visibility is the shared engine decision
/// ([`crate::plugin_tools::should_hide_tool_call_display`]) as of
/// Phase-6 B3 — a DECLARED unification: Feishu previously hid only
/// `reasoning` items; it now also hides the engine's full internal
/// set (subagent-tagged rows, hookPrompt/plan/review/compaction),
/// matching Telegram and Discord.
fn is_hidden_cot_tool(message: &ProviderMessage) -> bool {
    crate::plugin_tools::should_hide_tool_call_display(message)
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
    fn text_message_events_emit_visible_text_without_step_ids_or_truncation_marker() {
        let mut state = FeishuCotState::default();
        let title = "准备做文档并下载截图".repeat(20);
        let events = state.text_message_events(&title);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, EVENT_TEXT_MESSAGE_START);
        assert_eq!(events[1].event_type, EVENT_TEXT_MESSAGE_CONTENT);
        assert_eq!(events[2].event_type, EVENT_TEXT_MESSAGE_END);

        let start = content_json(&events[0]);
        let content = content_json(&events[1]);
        let end = content_json(&events[2]);
        assert_eq!(start["messageId"], content["messageId"]);
        assert_eq!(start["messageId"], end["messageId"]);
        assert_eq!(start["role"], "assistant");
        let visible_text = content["delta"].as_str().unwrap_or_default();
        assert!(visible_text.contains("准备做文档并下载截图"));
        assert!(
            !visible_text.contains("truncated") && !visible_text.contains("step-"),
            "text={visible_text}"
        );
    }

    #[test]
    fn tool_result_events_backfill_args_and_emit_parameter_result_without_output() {
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
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, EVENT_TOOL_CALL_ARGS);
        let args = content_json(&events[0]);
        assert_eq!(args["delta"], "pwd");
        assert_eq!(events[1].event_type, EVENT_TOOL_CALL_END);
        assert_eq!(events[2].event_type, EVENT_TOOL_CALL_RESULT);
        let result = content_json(&events[2]);
        assert_eq!(result["toolCallId"], "tool-1");
        assert_eq!(result["role"], "tool");
        assert_eq!(result["content"], json!({"type": "code", "code": "$ pwd"}));
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
        assert_eq!(start["toolCallName"], "pwd");
        assert_eq!(start["title"], "运行命令");
        assert_eq!(start["icon"], "bash");
        assert!(start.get("status").is_none());
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
        assert_eq!(start["title"], "运行命令");
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
        assert_eq!(result_events.len(), 2);
        assert_eq!(result_events[0].event_type, EVENT_TOOL_CALL_END);
        assert_eq!(result_events[1].event_type, EVENT_TOOL_CALL_RESULT);
        let result = content_json(&result_events[1]);
        assert_eq!(
            result["content"],
            json!({"type": "code", "code": "$ echo \"=== knowledge-base/prds/ ===\" && ls /Users/test/prds"})
        );
        assert!(!result_events[0].content.contains("example.md"));
        assert!(!result_events[1].content.contains("example.md"));
    }

    #[test]
    fn command_tool_args_truncate_without_visible_marker() {
        let command = format!(
            "cd /Users/test/knowledge-base/prds && echo \"{}\"",
            "关键模块分布".repeat(20)
        );
        let message = ProviderMessage::tool_use(
            json!({
                "type": "commandExecution",
                "command": command,
            }),
            Some("tool-command-long".to_owned()),
            Some("shell".to_owned()),
        );
        let mut state = FeishuCotState::default();
        let events = state.tool_use_events(&message);
        let args = content_json(&events[1]);
        let delta = args["delta"].as_str().unwrap_or_default();
        assert!(delta.len() <= MAX_TOOL_ARG_DISPLAY_CHARS);
        assert!(!delta.contains("truncated"), "delta={delta}");

        let result = ProviderMessage::tool_result(
            json!({
                "type": "commandExecution",
                "command": command,
                "output": "hidden output",
            }),
            Some("tool-command-long".to_owned()),
            Some("shell".to_owned()),
            Some(false),
        );
        let result_events = state.tool_result_events(&result);
        let result = content_json(&result_events[1]);
        let code = result["content"]["code"].as_str().unwrap_or_default();
        assert!(code.starts_with("$ cd /Users/test/knowledge-base/prds"));
        assert!(!code.contains("truncated"), "code={code}");
        assert!(
            !result_events
                .iter()
                .any(|event| event.content.contains("hidden output"))
        );
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
        assert_eq!(start["toolCallName"], "/Users/test/workspace/src/main.rs");
        assert_eq!(start["title"], "读取文件");
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
        assert_eq!(
            start["toolCallName"],
            "/Users/test/workspace/references/schema.md"
        );
        assert_eq!(start["title"], "读取文件");
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
    fn tool_result_backfills_missing_args_and_emits_parameter_result_without_file_content() {
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
        assert_eq!(result_events.len(), 3);
        assert_eq!(result_events[0].event_type, EVENT_TOOL_CALL_ARGS);
        let args = content_json(&result_events[0]);
        assert_eq!(args["delta"], "/Users/test/workspace/src/lib.rs");
        assert_eq!(result_events[1].event_type, EVENT_TOOL_CALL_END);
        assert_eq!(result_events[2].event_type, EVENT_TOOL_CALL_RESULT);
        let result = content_json(&result_events[2]);
        assert_eq!(
            result["content"],
            json!({"type": "code", "code": "# /Users/test/workspace/src/lib.rs"})
        );
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
        assert_eq!(
            start["toolCallName"],
            "EPM LAPS password, macOS Secure Token"
        );
        assert_eq!(start["title"], "搜索");
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
        assert_eq!(start["toolCallName"], "file_131.jpg");
        assert_eq!(start["title"], "ImageView");
        assert_eq!(start["icon"], "read");
        let args = content_json(&use_events[1]);
        assert_eq!(args["delta"], "file_131.jpg");

        let result_events = state.tool_result_events(&image_result);
        assert_eq!(result_events.len(), 2);
        assert_eq!(result_events[0].event_type, EVENT_TOOL_CALL_END);
        assert_eq!(result_events[1].event_type, EVENT_TOOL_CALL_RESULT);
        let result = content_json(&result_events[1]);
        assert_eq!(
            result["content"],
            json!({"type": "text", "text": "file_131.jpg"})
        );
        assert!(!result_events[0].content.contains("file_131.jpg"));
        assert!(result_events[1].content.contains("file_131.jpg"));
    }
}
