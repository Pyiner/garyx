//! Shared classification for transcript/history message projection.
//!
//! These helpers decide whether a stored transcript message is a tool-related
//! trace (versus human-visible user/assistant text) and what coarse `kind` the
//! gateway history projection reports to clients. The exact same logic is
//! consumed by gateway HTTP history projection and transcript-stream readers,
//! so it lives here as the single source of truth instead of being copy-pasted
//! into each.

use serde_json::{Map, Value};

/// Returns true when a transcript message is a persisted control record.
pub fn is_control_message(message: &Map<String, Value>) -> bool {
    message
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("control"))
        || message
            .get("internal_kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("control"))
}

/// Returns true when a transcript message represents tool activity (a tool call
/// or tool result) rather than human-visible user/assistant text.
///
/// Tool-relatedness is determined by **structural** signals only: the message
/// role, an explicit `tool_use_result` flag, a non-empty `tool_name`, or a
/// structured tool payload (arrays/objects carrying tool keys such as
/// `tool_use_id`). The free-text message body is deliberately *not* scanned for
/// tool keywords: an assistant reply that merely discusses tools (mentioning
/// `tool_use`, `tool_result`, `mcp__`, …) must stay an assistant reply, not be
/// reclassified as a tool trace and rendered as a phantom "tool call" on the
/// clients.
pub fn is_tool_related_message(role: &str, message: &Map<String, Value>) -> bool {
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

/// Detects tool signals inside a *structured* payload (arrays/objects).
///
/// A top-level plain string is never treated as a tool signal: it is the
/// message body, and substring-matching prose for `tool_use`/`mcp__`/… produces
/// false positives on ordinary replies that merely talk about tools. Strings
/// nested *inside* structured content are still inspected, so genuine
/// `{"type":"tool_use", …}` blocks are detected.
pub fn contains_tool_hint(value: Option<&Value>) -> bool {
    fn inner(value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }

        match value {
            // Only treat a string as a tool signal when it is nested inside a
            // structured payload (depth > 0), never when it is the message body
            // itself (depth 0). This is what keeps an assistant reply that
            // mentions "tool_use"/"mcp__" in prose from being misclassified.
            Value::String(text) if depth > 0 => {
                let lower = text.to_ascii_lowercase();
                lower.contains("tool_use")
                    || lower.contains("tool_result")
                    || lower.contains("tool_call")
                    || lower.contains("mcp__")
            }
            Value::String(_) => false,
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

/// Maps a message `role` (plus its computed tool-relatedness) to the coarse
/// `kind` the history projection reports to clients.
pub fn resolve_message_kind(role: &str, tool_related: bool) -> &'static str {
    match role {
        "user" => "user_input",
        "assistant" => {
            if tool_related {
                "tool_trace"
            } else {
                "assistant_reply"
            }
        }
        "tool" | "tool_use" | "tool_result" => "tool_trace",
        "system" => "system",
        _ if tool_related => "tool_trace",
        _ => "internal",
    }
}

/// Maps a full transcript message object to the coarse `kind` reported to
/// clients, with persisted control records taking precedence over role fallback.
pub fn resolve_message_kind_for_object(
    role: &str,
    message: &Map<String, Value>,
    tool_related: bool,
) -> &'static str {
    if is_control_message(message) {
        "control"
    } else {
        resolve_message_kind(role, tool_related)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("object literal")
    }

    #[test]
    fn assistant_prose_about_tools_is_not_tool_related() {
        // Regression: a reply that *discusses* tools must remain an assistant
        // reply, not be reclassified as a tool trace. This is the bug that
        // rendered such replies as "Using Tool Trace" on iOS.
        let message = obj(json!({
            "role": "assistant",
            "content": "is_tool_related_message scans for tool_use / tool_result / tool_call / mcp__ substrings.",
        }));
        assert!(!is_tool_related_message("assistant", &message));
        assert_eq!(resolve_message_kind("assistant", false), "assistant_reply");
    }

    #[test]
    fn user_prose_about_tools_is_not_tool_related() {
        let message = obj(json!({
            "role": "user",
            "content": "why does mcp__garyx__status render as a tool_call?",
        }));
        assert!(!is_tool_related_message("user", &message));
    }

    #[test]
    fn tool_roles_are_tool_related() {
        assert!(is_tool_related_message("tool", &Map::new()));
        assert!(is_tool_related_message("tool_use", &Map::new()));
        assert!(is_tool_related_message("tool_result", &Map::new()));
    }

    #[test]
    fn control_kind_takes_precedence_over_system_role() {
        let message = obj(json!({
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": { "kind": "run_start" },
        }));
        assert!(is_control_message(&message));
        assert_eq!(
            resolve_message_kind_for_object("system", &message, false),
            "control"
        );
    }

    #[test]
    fn internal_kind_control_is_control_even_without_top_level_kind() {
        let message = obj(json!({
            "role": "system",
            "internal": true,
            "internal_kind": "control",
            "control": { "kind": "done" },
        }));
        assert!(is_control_message(&message));
        assert_eq!(
            resolve_message_kind_for_object("system", &message, true),
            "control"
        );
    }

    #[test]
    fn tool_name_marks_tool_related() {
        let message = obj(json!({ "role": "assistant", "tool_name": "Bash" }));
        assert!(is_tool_related_message("assistant", &message));
        // Empty/whitespace tool_name does not count.
        let blank = obj(json!({ "role": "assistant", "tool_name": "  " }));
        assert!(!is_tool_related_message("assistant", &blank));
    }

    #[test]
    fn tool_use_result_flag_marks_tool_related() {
        let message = obj(json!({ "role": "assistant", "tool_use_result": true }));
        assert!(is_tool_related_message("assistant", &message));
    }

    #[test]
    fn structured_tool_payload_is_detected_by_key() {
        let message = obj(json!({
            "role": "assistant",
            "content": { "tool_use_id": "abc", "input": { "x": 1 } },
        }));
        assert!(is_tool_related_message("assistant", &message));
    }

    #[test]
    fn nested_tool_use_block_is_detected() {
        // A structured content array carrying a tool_use block is still a tool
        // trace; the nested "tool_use" string (depth > 0) is inspected.
        let message = obj(json!({
            "role": "assistant",
            "content": [ { "type": "tool_use", "name": "Bash" } ],
        }));
        assert!(is_tool_related_message("assistant", &message));
    }

    #[test]
    fn plain_string_payload_is_never_a_hint() {
        assert!(!contains_tool_hint(Some(&json!(
            "calling tool_use then mcp__garyx__status"
        ))));
    }

    #[test]
    fn resolve_kind_table() {
        assert_eq!(resolve_message_kind("user", false), "user_input");
        assert_eq!(resolve_message_kind("assistant", false), "assistant_reply");
        assert_eq!(resolve_message_kind("assistant", true), "tool_trace");
        assert_eq!(resolve_message_kind("tool_use", false), "tool_trace");
        assert_eq!(resolve_message_kind("system", false), "system");
        assert_eq!(resolve_message_kind("other", true), "tool_trace");
        assert_eq!(resolve_message_kind("other", false), "internal");
    }

    #[test]
    fn structured_object_keys_are_detected() {
        // OpenAI-style and MCP-style structured payloads stay tool-related via
        // their object keys (these branches are preserved verbatim).
        let tool_calls = obj(json!({
            "role": "assistant",
            "content": { "tool_calls": [ { "id": "c1" } ] },
        }));
        assert!(is_tool_related_message("assistant", &tool_calls));

        let tool_call_id = obj(json!({
            "role": "assistant",
            "content": { "tool_call_id": "c1", "output": "ok" },
        }));
        assert!(is_tool_related_message("assistant", &tool_call_id));

        let mcp_key = obj(json!({
            "role": "assistant",
            "content": { "mcp__garyx__status": { "ok": true } },
        }));
        assert!(is_tool_related_message("assistant", &mcp_key));
    }

    #[test]
    fn auxiliary_fields_are_scanned_for_structured_hints() {
        // Fields beyond `content` (here `input`) are also inspected, so a
        // structured tool signal is detected even when the body is plain text.
        let message = obj(json!({
            "role": "assistant",
            "content": "plain reply",
            "input": { "tool_calls": [ { "id": "c1" } ] },
        }));
        assert!(is_tool_related_message("assistant", &message));
    }
}
