//! Write-time thread message previews.
//!
//! `last_user_preview` / `last_assistant_preview` are small thread-record
//! fields maintained at the run's terminal persistence point (#TASK-1864
//! batch 1). They replace read-time scans of the legacy `messages`
//! snapshot in the thread_meta/recent projections, automation views, and
//! thread summaries. Readers fall back to scanning a record's `messages`
//! only for records not yet touched by a post-batch-1 run; Batch 2's
//! import backfills the fields and deletes those fallbacks.

use serde_json::Value;

/// Thread-record field holding the newest user message preview.
pub const LAST_USER_PREVIEW_FIELD: &str = "last_user_preview";
/// Thread-record field holding the newest assistant message preview.
pub const LAST_ASSISTANT_PREVIEW_FIELD: &str = "last_assistant_preview";
/// Preview truncation limit in characters.
pub const MESSAGE_PREVIEW_CHAR_LIMIT: usize = 160;

/// The preview field maintained for `role`, if the role has one.
pub fn preview_field_for_role(role: &str) -> Option<&'static str> {
    match role {
        "user" => Some(LAST_USER_PREVIEW_FIELD),
        "assistant" => Some(LAST_ASSISTANT_PREVIEW_FIELD),
        _ => None,
    }
}

/// Trimmed, char-limited preview of `value`; `None` when blank.
pub fn summarize_preview_text(value: &str, limit: usize) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    let mut chars = text.chars();
    let mut summary = String::new();
    for _ in 0..limit {
        let Some(ch) = chars.next() else {
            return Some(summary);
        };
        summary.push(ch);
    }
    Some(summary + "…")
}

/// Newest preview-worthy message for `role` in an ordered message walk:
/// string `content` first, string `text` fallback, truncated to
/// [`MESSAGE_PREVIEW_CHAR_LIMIT`].
pub fn last_message_preview_for_role<'a>(
    messages: impl DoubleEndedIterator<Item = &'a Value>,
    role: &str,
) -> Option<String> {
    for message in messages.rev() {
        let Some(object) = message.as_object() else {
            continue;
        };
        if object.get("role").and_then(Value::as_str) != Some(role) {
            continue;
        }
        let text = match object.get("content") {
            Some(Value::String(value)) => Some(value.as_str()),
            _ => object.get("text").and_then(Value::as_str),
        };
        if let Some(summary) =
            text.and_then(|value| summarize_preview_text(value, MESSAGE_PREVIEW_CHAR_LIMIT))
        {
            return Some(summary);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn preview_prefers_newest_matching_role_and_truncates() {
        let messages = vec![
            json!({"role": "user", "content": "first question"}),
            json!({"role": "assistant", "content": "an answer"}),
            json!({"role": "user", "content": "x".repeat(200)}),
        ];
        let preview = last_message_preview_for_role(messages.iter(), "user").unwrap();
        assert_eq!(preview.chars().count(), MESSAGE_PREVIEW_CHAR_LIMIT + 1);
        assert!(preview.ends_with('…'));
        assert_eq!(
            last_message_preview_for_role(messages.iter(), "assistant").as_deref(),
            Some("an answer")
        );
    }

    #[test]
    fn preview_falls_back_to_text_field_and_skips_structured_content() {
        let messages = vec![
            json!({"role": "assistant", "content": [{"type": "tool_use"}], "text": "tool reply"}),
            json!({"role": "assistant", "content": {"nested": true}}),
        ];
        assert_eq!(
            last_message_preview_for_role(messages.iter(), "assistant").as_deref(),
            Some("tool reply")
        );
    }

    #[test]
    fn preview_ignores_blank_text_and_unknown_roles() {
        let messages = vec![
            json!({"role": "user", "content": "   "}),
            json!({"role": "system", "content": "internal"}),
        ];
        assert_eq!(last_message_preview_for_role(messages.iter(), "user"), None);
        assert_eq!(preview_field_for_role("system"), None);
        assert_eq!(
            preview_field_for_role("user"),
            Some(LAST_USER_PREVIEW_FIELD)
        );
    }
}
