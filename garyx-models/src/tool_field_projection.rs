use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolKind {
    Command,
    FileRead,
    FileWrite,
    FileEdit,
    Search,
    Web,
    Agent,
    Task,
    Image,
    System,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolFieldRoot {
    Content,
    Input,
    Result,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolFieldFormat {
    Text,
    Code,
    Path,
    Json,
    Diff,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolFieldLabel {
    Call,
    Command,
    File,
    Query,
    Url,
    Prompt,
    Parameters,
    Content,
    Output,
    Result,
    Response,
    Diff,
    Image,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolVisibility {
    Normal,
    Nested,
    Quiet,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderToolFieldSelector {
    pub root: RenderToolFieldRoot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
    pub format: RenderToolFieldFormat,
    pub label: RenderToolFieldLabel,
}

/// Lightweight server-owned field mapping for one paired tool activity.
///
/// The projection deliberately carries selectors rather than selected values:
/// command output can be very large and is already available in the committed
/// message body referenced by `RenderToolEntry`. Mac and iOS therefore share
/// one provider/tool rule set without duplicating stdout in every render frame.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderToolFieldProjection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub kind: RenderToolKind,
    pub visibility: RenderToolVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<RenderToolFieldSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<RenderToolFieldSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl RenderToolFieldProjection {
    pub(crate) fn from_message(message: &Map<String, Value>, is_result: bool) -> Option<Self> {
        let envelope_tool_name = tool_name(message);
        let visibility = tool_visibility(envelope_tool_name.as_deref());
        let tool_name = display_tool_name(message, envelope_tool_name);
        let kind = classify_tool(tool_name.as_deref());
        let mut projection = Self {
            tool_name,
            kind,
            visibility,
            call: None,
            result: None,
            status: None,
            exit_code: None,
            duration_ms: None,
        };
        if is_result {
            projection.result = result_selector(message, kind);
        } else {
            projection.call = call_selector(message, kind);
        }
        projection.absorb_metadata(message);

        (projection.tool_name.is_some()
            || projection.call.is_some()
            || projection.result.is_some()
            || projection.status.is_some()
            || projection.exit_code.is_some()
            || projection.duration_ms.is_some())
        .then_some(projection)
    }

    pub(crate) fn absorb_result(&mut self, result: Self) {
        if self.tool_name.is_none() {
            self.tool_name = result.tool_name;
        }
        if self.kind == RenderToolKind::Generic && result.kind != RenderToolKind::Generic {
            self.kind = result.kind;
        }
        if self.visibility == RenderToolVisibility::Normal
            && result.visibility != RenderToolVisibility::Normal
        {
            self.visibility = result.visibility;
        }
        if let Some(result_selector) = result.result {
            let repeats_visual_call = self.call.as_ref().is_some_and(|call_selector| {
                matches!(
                    call_selector.format,
                    RenderToolFieldFormat::Diff | RenderToolFieldFormat::Image
                ) && call_selector.root == result_selector.root
                    && call_selector.path == result_selector.path
                    && call_selector.format == result_selector.format
            });
            if !repeats_visual_call {
                self.result = Some(result_selector);
            }
        }
        if result.status.is_some() {
            self.status = result.status;
        }
        if result.exit_code.is_some() {
            self.exit_code = result.exit_code;
        }
        if result.duration_ms.is_some() {
            self.duration_ms = result.duration_ms;
        }
    }

    fn absorb_metadata(&mut self, message: &Map<String, Value>) {
        let Some(payload) = message_payload_object(message) else {
            return;
        };
        self.status = string_field(payload, &["status"]);
        self.exit_code = integer_field(payload, &["exitCode", "exit_code"]);
        self.duration_ms = unsigned_field(payload, &["durationMs", "duration_ms"]);
    }
}

pub(crate) fn merge_tool_result_projection(
    existing: Option<RenderToolFieldProjection>,
    result: Option<RenderToolFieldProjection>,
) -> Option<RenderToolFieldProjection> {
    match (existing, result) {
        (Some(mut existing), Some(result)) => {
            existing.absorb_result(result);
            Some(existing)
        }
        (Some(existing), None) => Some(existing),
        (None, Some(result)) => Some(result),
        (None, None) => None,
    }
}

fn tool_name(message: &Map<String, Value>) -> Option<String> {
    string_field(message, &["tool_name", "toolName"])
        .or_else(|| {
            message
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| string_field(metadata, &["item_type", "itemType"]))
        })
        .or_else(|| {
            message_payload_object(message).and_then(|payload| {
                string_field(payload, &["tool", "name", "tool_name", "toolName", "type"])
            })
        })
}

fn display_tool_name(
    message: &Map<String, Value>,
    envelope_tool_name: Option<String>,
) -> Option<String> {
    let item_name = compact_name(envelope_tool_name.as_deref().unwrap_or_default());
    if matches!(item_name.as_str(), "mcptoolcall" | "dynamictoolcall") {
        return message_payload_object(message)
            .and_then(|payload| string_field(payload, &["tool", "name"]))
            .or(envelope_tool_name);
    }
    envelope_tool_name
}

fn classify_tool(tool_name: Option<&str>) -> RenderToolKind {
    let name = compact_name(tool_name.unwrap_or_default());
    if matches!(
        name.as_str(),
        "contextcompaction"
            | "hookprompt"
            | "reasoning"
            | "enteredreviewmode"
            | "exitedreviewmode"
            | "enterplanmode"
            | "exitplanmode"
            | "plan"
    ) {
        return RenderToolKind::System;
    }
    if name.contains("imagegeneration")
        || name.contains("imageview")
        || name.contains("imagegen")
        || name == "viewimage"
    {
        return RenderToolKind::Image;
    }
    if name == "commandstatus" {
        return RenderToolKind::Task;
    }
    if matches!(
        name.as_str(),
        "bash"
            | "shell"
            | "command"
            | "execcommand"
            | "commandexecution"
            | "runcommand"
            | "monitor"
    ) || name.ends_with("commandexecution")
        || name.contains("executecommand")
    {
        return RenderToolKind::Command;
    }
    if name == "filechange"
        || name.contains("applypatch")
        || name.contains("multiedit")
        || name.contains("replacefilecontent")
        || name.contains("deletefile")
        || name.contains("renamefile")
        || name.contains("movefile")
        || name == "edit"
    {
        return RenderToolKind::FileEdit;
    }
    if matches!(name.as_str(), "write" | "create") || name.contains("writefile") {
        return RenderToolKind::FileWrite;
    }
    if matches!(name.as_str(), "read" | "view" | "open" | "cat" | "viewfile")
        || name.contains("readfile")
        || name.contains("notebookread")
    {
        return RenderToolKind::FileRead;
    }
    if name == "webrun"
        || name.contains("websearch")
        || name.contains("searchweb")
        || name.contains("webfetch")
    {
        return RenderToolKind::Web;
    }
    if matches!(
        name.as_str(),
        "grep" | "glob" | "find" | "rg" | "toolsearch"
    ) || name.starts_with("findby")
        || name.starts_with("grep")
        || name.starts_with("glob")
        || name.ends_with("search")
    {
        return RenderToolKind::Search;
    }
    if name == "agent" || name.starts_with("collabagent") || name.starts_with("subagent") {
        return RenderToolKind::Agent;
    }
    if name.starts_with("task")
        || matches!(
            name.as_str(),
            "todowrite" | "managetask" | "schedule" | "sleep"
        )
    {
        return RenderToolKind::Task;
    }
    RenderToolKind::Generic
}

fn tool_visibility(tool_name: Option<&str>) -> RenderToolVisibility {
    let name = compact_name(tool_name.unwrap_or_default());
    if matches!(
        name.as_str(),
        "contextcompaction" | "hookprompt" | "reasoning"
    ) {
        RenderToolVisibility::Hidden
    } else if name.starts_with("subagent") {
        RenderToolVisibility::Nested
    } else if matches!(
        name.as_str(),
        "plan" | "enteredreviewmode" | "exitedreviewmode" | "enterplanmode" | "exitplanmode"
    ) {
        RenderToolVisibility::Quiet
    } else {
        RenderToolVisibility::Normal
    }
}

fn call_selector(
    message: &Map<String, Value>,
    kind: RenderToolKind,
) -> Option<RenderToolFieldSelector> {
    let (root, prefix, input) = call_input_object(message)?;
    let keys: &[&str] = match kind {
        RenderToolKind::Command => &[
            "label",
            "description",
            "toolSummary",
            "toolAction",
            "cmd",
            "command",
            "CommandLine",
        ],
        RenderToolKind::FileRead => &[
            "label",
            "description",
            "toolSummary",
            "toolAction",
            "file_path",
            "filePath",
            "AbsolutePath",
            "path",
            "file",
        ],
        RenderToolKind::FileWrite | RenderToolKind::FileEdit => &[
            "label",
            "description",
            "toolSummary",
            "toolAction",
            "file_path",
            "filePath",
            "AbsolutePath",
            "path",
            "file",
            "changes",
            "diff",
            "content",
        ],
        RenderToolKind::Search | RenderToolKind::Web => &[
            "label",
            "description",
            "toolSummary",
            "toolAction",
            "query",
            "pattern",
            "search",
            "url",
        ],
        RenderToolKind::Agent => &[
            "label",
            "description",
            "toolSummary",
            "task",
            "prompt",
            "message",
            "agentPath",
        ],
        RenderToolKind::Task => &[
            "label",
            "subject",
            "description",
            "toolSummary",
            "toolAction",
            "Prompt",
            "prompt",
            "TaskId",
            "task_id",
            "CommandId",
            "DurationSeconds",
            "duration",
            "status",
            "Action",
            "action",
        ],
        RenderToolKind::Image => &[
            "label",
            "description",
            "prompt",
            "path",
            "file_path",
            "filePath",
        ],
        RenderToolKind::System | RenderToolKind::Generic => &[
            "label",
            "description",
            "toolSummary",
            "toolAction",
            "title",
            "subject",
            "cmd",
            "command",
            "CommandLine",
            "query",
            "pattern",
            "url",
            "file_path",
            "filePath",
            "AbsolutePath",
            "path",
            "prompt",
            "message",
            "skill",
            "TaskId",
            "task_id",
            "Action",
            "action",
            "arguments",
            "params",
        ],
    };
    select_object_field(root, &prefix, input, keys, kind, false).or_else(|| {
        (!input.is_empty()).then(|| RenderToolFieldSelector {
            root,
            path: prefix,
            format: RenderToolFieldFormat::Json,
            label: if input.len() == 1 {
                RenderToolFieldLabel::Call
            } else {
                RenderToolFieldLabel::Parameters
            },
        })
    })
}

fn result_selector(
    message: &Map<String, Value>,
    kind: RenderToolKind,
) -> Option<RenderToolFieldSelector> {
    if let Some(content) = message.get("content") {
        if let Some(object) = content.as_object() {
            if let Some(selector) = select_object_field(
                RenderToolFieldRoot::Content,
                &[],
                object,
                &[
                    "aggregatedOutput",
                    "result",
                    "output",
                    "content",
                    "stdout",
                    "stderr",
                    "text",
                    "message",
                    "error",
                    "changes",
                    "diff",
                    "action",
                    "path",
                ],
                kind,
                true,
            ) {
                return Some(selector);
            }
            // A command execution with no meaningful output should have no
            // result body. Falling back to the entire execution envelope is
            // precisely the noisy JSON presentation this projection avoids.
            if kind == RenderToolKind::Command {
                return None;
            }
            if !object.is_empty() {
                return Some(RenderToolFieldSelector {
                    root: RenderToolFieldRoot::Content,
                    path: Vec::new(),
                    format: RenderToolFieldFormat::Json,
                    label: RenderToolFieldLabel::Result,
                });
            }
        } else if meaningful_value(content) {
            return Some(selector_for_value(
                RenderToolFieldRoot::Content,
                Vec::new(),
                content,
                kind,
                true,
                None,
            ));
        }
    }

    for (root, key) in [
        (RenderToolFieldRoot::Result, "result"),
        (RenderToolFieldRoot::Text, "text"),
    ] {
        if let Some(value) = message.get(key).filter(|value| meaningful_value(value)) {
            return Some(selector_for_value(
                root,
                Vec::new(),
                value,
                kind,
                true,
                Some(key),
            ));
        }
    }
    None
}

fn call_input_object(
    message: &Map<String, Value>,
) -> Option<(RenderToolFieldRoot, Vec<String>, &Map<String, Value>)> {
    if let Some(content) = message.get("content").and_then(Value::as_object) {
        for key in ["input", "arguments", "args", "params"] {
            if let Some(input) = content.get(key).and_then(Value::as_object) {
                return Some((RenderToolFieldRoot::Content, vec![key.to_owned()], input));
            }
        }
        return Some((RenderToolFieldRoot::Content, Vec::new(), content));
    }
    message
        .get("input")
        .and_then(Value::as_object)
        .map(|input| (RenderToolFieldRoot::Input, Vec::new(), input))
}

fn select_object_field(
    root: RenderToolFieldRoot,
    prefix: &[String],
    object: &Map<String, Value>,
    keys: &[&str],
    kind: RenderToolKind,
    result: bool,
) -> Option<RenderToolFieldSelector> {
    keys.iter().find_map(|key| {
        let value = object.get(*key).filter(|value| meaningful_value(value))?;
        let mut path = prefix.to_vec();
        path.push((*key).to_owned());
        Some(selector_for_value(
            root,
            path,
            value,
            kind,
            result,
            Some(key),
        ))
    })
}

fn selector_for_value(
    root: RenderToolFieldRoot,
    path: Vec<String>,
    value: &Value,
    kind: RenderToolKind,
    result: bool,
    key: Option<&str>,
) -> RenderToolFieldSelector {
    let key = key.unwrap_or_default();
    let (label, format) = if key == "aggregatedOutput" || matches!(key, "stdout" | "stderr") {
        (RenderToolFieldLabel::Output, RenderToolFieldFormat::Code)
    } else if key == "error" {
        (RenderToolFieldLabel::Error, RenderToolFieldFormat::Code)
    } else if matches!(key, "changes" | "diff") {
        (RenderToolFieldLabel::Diff, RenderToolFieldFormat::Diff)
    } else if matches!(key, "command" | "cmd" | "CommandLine") {
        (RenderToolFieldLabel::Command, RenderToolFieldFormat::Code)
    } else if matches!(key, "file_path" | "filePath" | "AbsolutePath" | "file") {
        (RenderToolFieldLabel::File, RenderToolFieldFormat::Path)
    } else if key == "path" && kind == RenderToolKind::Image {
        (RenderToolFieldLabel::Image, RenderToolFieldFormat::Image)
    } else if key == "path" {
        (RenderToolFieldLabel::File, RenderToolFieldFormat::Path)
    } else if matches!(key, "query" | "pattern" | "search") {
        (RenderToolFieldLabel::Query, RenderToolFieldFormat::Text)
    } else if key == "url" {
        (RenderToolFieldLabel::Url, RenderToolFieldFormat::Text)
    } else if matches!(key, "prompt" | "Prompt") {
        (RenderToolFieldLabel::Prompt, RenderToolFieldFormat::Text)
    } else if key == "content" && result && kind == RenderToolKind::Command {
        (RenderToolFieldLabel::Output, RenderToolFieldFormat::Code)
    } else if key == "content" {
        (RenderToolFieldLabel::Content, format_for_value(value))
    } else if result && key == "action" {
        (RenderToolFieldLabel::Response, RenderToolFieldFormat::Json)
    } else if result {
        (RenderToolFieldLabel::Result, format_for_value(value))
    } else if matches!(key, "arguments" | "params") {
        (
            RenderToolFieldLabel::Parameters,
            RenderToolFieldFormat::Json,
        )
    } else {
        (RenderToolFieldLabel::Call, format_for_value(value))
    };
    RenderToolFieldSelector {
        root,
        path,
        format,
        label,
    }
}

fn format_for_value(value: &Value) -> RenderToolFieldFormat {
    if value_contains_image(value, 0) {
        return RenderToolFieldFormat::Image;
    }
    match value {
        Value::Object(_) | Value::Array(_) => RenderToolFieldFormat::Json,
        _ => RenderToolFieldFormat::Text,
    }
}

fn value_contains_image(value: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match value {
        Value::Array(items) => items
            .iter()
            .any(|item| value_contains_image(item, depth + 1)),
        Value::Object(object) => {
            object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case("image"))
                || object
                    .get("source")
                    .and_then(Value::as_object)
                    .is_some_and(|source| source.get("data").is_some())
                || object
                    .values()
                    .any(|value| value_contains_image(value, depth + 1))
        }
        _ => false,
    }
}

fn message_payload_object(message: &Map<String, Value>) -> Option<&Map<String, Value>> {
    message.get("content").and_then(Value::as_object)
}

fn meaningful_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn string_field(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn integer_field(object: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_i64().or_else(|| value.as_u64()?.try_into().ok()))
    })
}

fn unsigned_field(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok()))
    })
}

fn compact_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    #[test]
    fn codex_command_projects_command_and_aggregated_output_without_copying_values() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "commandExecution",
            "content": {
                "type": "commandExecution",
                "command": "/bin/zsh -lc 'git status --short'",
                "status": "inProgress"
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "commandExecution",
            "content": {
                "type": "commandExecution",
                "aggregatedOutput": " M AGENTS.md\n M CLAUDE.md\n",
                "status": "completed",
                "exitCode": 0,
                "durationMs": 12
            }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(projection.kind, RenderToolKind::Command);
        assert_eq!(projection.call.as_ref().unwrap().path, ["command"]);
        assert_eq!(
            projection.result.as_ref().unwrap().path,
            ["aggregatedOutput"]
        );
        assert_eq!(projection.exit_code, Some(0));
        assert_eq!(projection.duration_ms, Some(12));
        let wire = serde_json::to_string(&projection).unwrap();
        assert!(!wire.contains("AGENTS.md"));
        assert!(!wire.contains("git status"));
    }

    #[test]
    fn claude_bash_prefers_label_then_result_value() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "Bash",
            "content": {
                "tool": "Bash",
                "input": {
                    "label": "Check repository state",
                    "command": "git status --short"
                }
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "content": { "result": "clean", "text": "clean" }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(projection.call.as_ref().unwrap().path, ["input", "label"]);
        assert_eq!(projection.result.as_ref().unwrap().path, ["result"]);
    }

    #[test]
    fn antigravity_projects_tool_summary_and_content_despite_result_type_change() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "run_command",
            "content": {
                "name": "run_command",
                "args": {
                    "toolSummary": "\"Check status\"",
                    "CommandLine": "\"git status --short\""
                }
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "RUN_COMMAND",
            "content": {
                "type": "RUN_COMMAND",
                "status": "DONE",
                "content": " M AGENTS.md\n"
            }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(projection.tool_name.as_deref(), Some("run_command"));
        assert_eq!(
            projection.call.as_ref().unwrap().path,
            ["args", "toolSummary"]
        );
        assert_eq!(projection.result.as_ref().unwrap().path, ["content"]);
        assert_eq!(
            projection.result.as_ref().unwrap().label,
            RenderToolFieldLabel::Output
        );
        assert_eq!(projection.status.as_deref(), Some("DONE"));
    }

    #[test]
    fn command_with_null_aggregated_output_does_not_fall_back_to_json_envelope() {
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "commandExecution",
            "content": {
                "type": "commandExecution",
                "aggregatedOutput": null,
                "command": "true",
                "status": "completed",
                "exitCode": 0
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&result, true).unwrap();

        assert_eq!(projection.result, None);
        assert_eq!(projection.status.as_deref(), Some("completed"));
        assert_eq!(projection.exit_code, Some(0));
    }

    #[test]
    fn mcp_projection_uses_inner_tool_name_and_primary_argument() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "mcpToolCall",
            "content": {
                "type": "mcpToolCall",
                "server": "garyx",
                "tool": "capsule_create",
                "arguments": { "title": "Synthetic capsule" }
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&call, false).unwrap();

        assert_eq!(projection.tool_name.as_deref(), Some("capsule_create"));
        assert_eq!(projection.kind, RenderToolKind::Generic);
        assert_eq!(
            projection.call.as_ref().unwrap().path,
            ["arguments", "title"]
        );
    }

    #[test]
    fn wrapped_dynamic_tool_is_classified_by_its_display_name() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "dynamicToolCall",
            "content": {
                "type": "dynamicToolCall",
                "tool": "exec_command",
                "arguments": { "cmd": "git status --short" }
            }
        }));

        let projection = RenderToolFieldProjection::from_message(&call, false).unwrap();

        assert_eq!(projection.tool_name.as_deref(), Some("exec_command"));
        assert_eq!(projection.kind, RenderToolKind::Command);
        assert_eq!(projection.call.as_ref().unwrap().path, ["arguments", "cmd"]);
    }

    #[test]
    fn file_change_does_not_repeat_the_same_diff_as_its_result() {
        let call = object(json!({
            "role": "tool_use",
            "tool_name": "fileChange",
            "content": {
                "type": "fileChange",
                "changes": [{ "path": "/Users/test/README.md", "diff": "+hello" }]
            }
        }));
        let result = object(json!({
            "role": "tool_result",
            "tool_name": "fileChange",
            "content": {
                "type": "fileChange",
                "changes": [{ "path": "/Users/test/README.md", "diff": "+hello" }],
                "status": "completed"
            }
        }));

        let mut projection = RenderToolFieldProjection::from_message(&call, false).unwrap();
        projection.absorb_result(RenderToolFieldProjection::from_message(&result, true).unwrap());

        assert_eq!(
            projection.call.as_ref().unwrap().format,
            RenderToolFieldFormat::Diff
        );
        assert_eq!(projection.result, None);
        assert_eq!(projection.status.as_deref(), Some("completed"));
    }
}
