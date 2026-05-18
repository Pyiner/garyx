//! Helpers shared by built-in and subprocess channel plugins.

use garyx_models::provider::ProviderMessage;

/// Per-response state for rendering lightweight tool-call progress in channels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCallDisplayState {
    active_tool_name: Option<String>,
    tool_call_index: usize,
}

impl ToolCallDisplayState {
    pub fn is_active(&self) -> bool {
        self.active_tool_name.is_some()
    }

    pub fn start_tool_call(&mut self, message: &ProviderMessage) -> String {
        let name = tool_call_display_name(message);
        self.tool_call_index = self.tool_call_index.saturating_add(1);
        self.active_tool_name = Some(name.clone());
        render_tool_call_placeholder(self.tool_call_index, &name)
    }

    pub fn clear_active(&mut self) {
        self.active_tool_name = None;
    }

    pub fn reset(&mut self) {
        self.active_tool_name = None;
        self.tool_call_index = 0;
    }

    pub fn render_content_text(&self, accumulated_text: &str) -> String {
        let Some(name) = self.active_tool_name.as_deref() else {
            return accumulated_text.to_owned();
        };

        let placeholder = render_tool_call_placeholder(self.tool_call_index, name);
        if placeholder.trim().is_empty() {
            return accumulated_text.to_owned();
        }
        if accumulated_text.trim().is_empty() {
            return placeholder;
        }
        if accumulated_text.ends_with("\n\n") {
            format!("{accumulated_text}{placeholder}")
        } else if accumulated_text.ends_with('\n') {
            format!("{accumulated_text}\n{placeholder}")
        } else {
            format!("{accumulated_text}\n\n{placeholder}")
        }
    }
}

pub fn tool_call_display_name(message: &ProviderMessage) -> String {
    message
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            message
                .content
                .pointer("/name")
                .or_else(|| message.content.pointer("/tool_name"))
                .or_else(|| message.content.pointer("/tool"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "tool".to_owned())
}

pub fn should_hide_tool_call_display(message: &ProviderMessage) -> bool {
    if ["agent_id", "parent_tool_use_id"].iter().any(|key| {
        message
            .metadata
            .get(*key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    }) {
        return true;
    }

    matches!(
        provider_message_item_type(message),
        Some(
            "hookPrompt"
                | "reasoning"
                | "plan"
                | "enteredReviewMode"
                | "exitedReviewMode"
                | "contextCompaction"
        )
    )
}

pub fn render_tool_call_placeholder(index: usize, name: &str) -> String {
    format!("🔧 #{index} {name}")
}

fn provider_message_item_type(message: &ProviderMessage) -> Option<&str> {
    message
        .metadata
        .get("item_type")
        .and_then(|value| value.as_str())
        .or_else(|| message.content.get("type").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tool_call_display_name_prefers_structured_tool_name() {
        let message = ProviderMessage::tool_use(
            json!({"name": "ignored"}),
            Some("tool-1".to_owned()),
            Some("Bash".to_owned()),
        );

        assert_eq!(tool_call_display_name(&message), "Bash");
    }

    #[test]
    fn tool_call_display_name_falls_back_to_content() {
        let message = ProviderMessage::tool_use(
            json!({"tool_name": "Read"}),
            Some("tool-1".to_owned()),
            None,
        );

        assert_eq!(tool_call_display_name(&message), "Read");
    }

    #[test]
    fn child_agent_tool_call_display_is_hidden() {
        let message =
            ProviderMessage::tool_use(json!({"name": "Bash"}), Some("tool-1".to_owned()), None)
                .with_metadata_value("parent_tool_use_id", json!("tool-parent"));

        assert!(should_hide_tool_call_display(&message));
    }

    #[test]
    fn internal_item_type_tool_call_display_is_hidden() {
        let message = ProviderMessage::tool_use(json!({"type": "reasoning"}), None, None);

        assert!(should_hide_tool_call_display(&message));
    }

    #[test]
    fn tool_call_display_state_renders_numbered_placeholder() {
        let mut state = ToolCallDisplayState::default();
        let message =
            ProviderMessage::tool_use(json!({"name": "Read"}), Some("tool-1".to_owned()), None);

        assert_eq!(state.start_tool_call(&message), "🔧 #1 Read");
        assert_eq!(state.render_content_text(""), "🔧 #1 Read");
        assert_eq!(
            state.render_content_text("working"),
            "working\n\n🔧 #1 Read"
        );
    }
}
