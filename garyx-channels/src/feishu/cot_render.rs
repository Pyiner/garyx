//! Pure rendering/formatting helpers for the Feishu chain-of-thought
//! card: tool titles/icons, readable argument extraction, JSON
//! preview, and UTF-8-safe truncation. Moved verbatim from cot.rs
//! (Phase-6 B3 pure code motion). Decision logic (event sequencing,
//! dedup, visibility) stays in cot.rs.

use garyx_models::provider::ProviderMessage;
use serde_json::{Map, Value, json};

pub(super) const MAX_EVENT_CONTENT_BYTES: usize = 4096;
pub(super) const MAX_TOOL_ARG_DISPLAY_CHARS: usize = 120;
const TRUNCATED_SUFFIX: &str = "\n...[truncated]";

pub(super) fn stringify_event_content(content: Value) -> String {
    let serialized = serde_json::to_string(&content).unwrap_or_else(|_| "{}".to_owned());
    if fits_event_content(&serialized) {
        return serialized;
    }

    let Value::Object(map) = content else {
        return truncate_utf8_bytes_with_suffix(&serialized, MAX_EVENT_CONTENT_BYTES);
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
        "preview": truncate_utf8_bytes_with_suffix(&serialized, MAX_EVENT_CONTENT_BYTES / 2),
    })
    .to_string()
}

fn shrink_longest_string_fields(map: &mut Map<String, Value>) {
    let mut keys = map
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|text| (key.clone(), text.len())))
        .collect::<Vec<_>>();
    keys.sort_by_key(|entry| std::cmp::Reverse(entry.1));

    for (key, len) in keys {
        let Some(Value::String(text)) = map.get_mut(&key) else {
            continue;
        };
        let next_len = (len / 2).max(64);
        *text = truncate_utf8_bytes_with_suffix(text, next_len);
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

pub(super) fn summarize_tool_args(message: &ProviderMessage) -> String {
    if let Some(args) = readable_tool_argument(message) {
        return args;
    }
    let candidate = default_tool_args_candidate(message);
    if is_uninformative_tool_args(candidate) {
        return String::new();
    }
    preview_json(candidate, MAX_TOOL_ARG_DISPLAY_CHARS)
}

pub(super) fn tool_parameter_result_content(message: &ProviderMessage, args: &str) -> Value {
    let args = args.trim();
    if is_command_like_tool(message) {
        return json!({
            "type": "code",
            "code": format!("$ {args}"),
        });
    }
    let tool = tool_name(message).to_ascii_lowercase();
    if contains_any(&tool, &["read", "write", "edit", "grep", "glob"]) {
        return json!({
            "type": "code",
            "code": format!("# {args}"),
        });
    }
    json!({
        "type": "text",
        "text": args,
    })
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

pub(super) fn readable_tool_argument(message: &ProviderMessage) -> Option<String> {
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
    if contains_any(&tool, &["read", "write", "edit", "grep", "glob"])
        && let Some(path) = readable_value_from_keys(
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
        )
    {
        return Some(path);
    }

    if contains_any(&tool, &["search", "webfetch", "fetch"])
        && let Some(query) =
            readable_value_from_keys(&message.content, &["query", "queries", "url", "urls"])
    {
        return Some(query);
    }

    if contains_any(&tool, &["task", "skill"])
        && let Some(prompt) = readable_value_from_keys(
            &message.content,
            &["prompt", "task", "title", "description", "name", "skill"],
        )
    {
        return Some(prompt);
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

pub(super) fn tool_title(tool_name: &str) -> String {
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

pub(super) fn tool_icon(tool_name: &str) -> &'static str {
    let value = tool_name.to_ascii_lowercase();
    if contains_any(&value, &["search", "web"]) {
        "search"
    } else if contains_any(&value, &["read", "grep", "glob"]) || value == "imageview" {
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

pub(super) fn sanitize_event_id_part(value: &str) -> String {
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
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub(super) fn split_utf8_bytes(text: &str, max_bytes: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if max_bytes == 0 {
        return vec![text.to_owned()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map(|(idx, _)| start + idx)
                .unwrap_or(text.len());
        }
        chunks.push(text[start..end].to_owned());
        start = end;
    }
    chunks
}

fn truncate_utf8_bytes_with_suffix(text: &str, max_bytes: usize) -> String {
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
