//! Helpers shared by built-in and subprocess channel plugins.

use std::time::{Duration, Instant};

use garyx_models::provider::{ProviderMessage, StreamBoundaryKind};

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

/// When text-only stream deltas should become visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStreamTextFlushPolicy {
    /// Text deltas may flush as they arrive, subject to `min_flush_interval`.
    OnEveryDelta,
    /// Text deltas wait until a tool call needs context or the run completes.
    OnToolUseOrDone,
}

/// Policy for channel/plugin stream presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginStreamSendPolicy {
    pub text_flush_policy: PluginStreamTextFlushPolicy,
    pub min_flush_interval: Option<Duration>,
}

impl PluginStreamSendPolicy {
    pub fn telegram_like() -> Self {
        Self {
            text_flush_policy: PluginStreamTextFlushPolicy::OnEveryDelta,
            min_flush_interval: Some(Duration::from_millis(300)),
        }
    }

    pub fn buffered_until_tool_or_done() -> Self {
        Self {
            text_flush_policy: PluginStreamTextFlushPolicy::OnToolUseOrDone,
            min_flush_interval: None,
        }
    }
}

/// Synchronous decision returned by the shared stream presentation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStreamSendDecision {
    Wait,
    FlushNow { content_text: String },
    ScheduleFlush { after: Duration },
}

/// Reusable text and tool-call presentation state for plugin-style stream senders.
///
/// Text deltas and tool calls are independent inputs, but the policy decides
/// when they should be rendered together. Tool calls are high-priority: a
/// visible top-level tool call flushes immediately, even when text deltas are
/// otherwise rate-limited or buffered until final output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginStreamSendState {
    accumulated_text: String,
    tool_display: ToolCallDisplayState,
    policy: PluginStreamSendPolicy,
    last_flush_time: Option<Instant>,
    flush_scheduled: bool,
}

impl Default for PluginStreamSendPolicy {
    fn default() -> Self {
        Self::buffered_until_tool_or_done()
    }
}

impl PluginStreamSendState {
    pub fn new(policy: PluginStreamSendPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn accumulated_text(&self) -> &str {
        &self.accumulated_text
    }

    pub fn is_tool_placeholder_active(&self) -> bool {
        self.tool_display.is_active()
    }

    pub fn flush_scheduled(&self) -> bool {
        self.flush_scheduled
    }

    pub fn set_accumulated_text(&mut self, text: impl Into<String>) {
        self.accumulated_text = text.into();
    }

    pub fn on_delta(&mut self, text: &str, now: Instant) -> PluginStreamSendDecision {
        if text.is_empty() {
            return PluginStreamSendDecision::Wait;
        }
        self.tool_display.reset();
        self.accumulated_text =
            crate::streaming_core::merge_stream_text(&self.accumulated_text, text);
        self.text_delta_flush_decision(now)
    }

    pub fn apply_boundary(&mut self, kind: StreamBoundaryKind) {
        crate::streaming_core::apply_stream_boundary_text(&mut self.accumulated_text, kind);
    }

    pub fn on_tool_call(
        &mut self,
        message: &ProviderMessage,
        _now: Instant,
    ) -> PluginStreamSendDecision {
        self.tool_display.start_tool_call(message);
        self.flush_scheduled = false;
        PluginStreamSendDecision::FlushNow {
            content_text: self.render_content_text(),
        }
    }

    pub fn on_done(&mut self, _now: Instant) -> PluginStreamSendDecision {
        self.flush_scheduled = false;
        let content_text = self.render_content_text();
        if content_text.trim().is_empty() {
            PluginStreamSendDecision::Wait
        } else {
            PluginStreamSendDecision::FlushNow { content_text }
        }
    }

    pub fn clear_tool_placeholder(&mut self) {
        self.tool_display.clear_active();
    }

    pub fn mark_flushed(&mut self, now: Instant) {
        self.last_flush_time = Some(now);
        self.flush_scheduled = false;
    }

    pub fn scheduled_flush(&mut self) -> PluginStreamSendDecision {
        if !self.flush_scheduled || self.tool_display.is_active() {
            return PluginStreamSendDecision::Wait;
        }
        let content_text = self.render_content_text();
        if content_text.trim().is_empty() {
            PluginStreamSendDecision::Wait
        } else {
            self.flush_scheduled = false;
            PluginStreamSendDecision::FlushNow { content_text }
        }
    }

    pub fn render_content_text(&self) -> String {
        self.tool_display
            .render_content_text(&self.accumulated_text)
    }

    fn text_delta_flush_decision(&mut self, now: Instant) -> PluginStreamSendDecision {
        if self.policy.text_flush_policy == PluginStreamTextFlushPolicy::OnToolUseOrDone {
            return PluginStreamSendDecision::Wait;
        }

        let content_text = self.render_content_text();
        if content_text.trim().is_empty() {
            return PluginStreamSendDecision::Wait;
        }

        let Some(min_interval) = self.policy.min_flush_interval else {
            return PluginStreamSendDecision::FlushNow { content_text };
        };
        let Some(last_flush_time) = self.last_flush_time else {
            return PluginStreamSendDecision::FlushNow { content_text };
        };
        let elapsed = now.saturating_duration_since(last_flush_time);
        if elapsed >= min_interval {
            PluginStreamSendDecision::FlushNow { content_text }
        } else if !self.flush_scheduled {
            self.flush_scheduled = true;
            PluginStreamSendDecision::ScheduleFlush {
                after: min_interval - elapsed,
            }
        } else {
            PluginStreamSendDecision::Wait
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

    #[test]
    fn buffered_stream_text_merges_delta_until_tool_call() {
        let now = Instant::now();
        let mut buffer =
            PluginStreamSendState::new(PluginStreamSendPolicy::buffered_until_tool_or_done());
        assert_eq!(
            buffer.on_delta("before ", now),
            PluginStreamSendDecision::Wait
        );
        assert_eq!(buffer.on_delta("tool", now), PluginStreamSendDecision::Wait);

        assert_eq!(buffer.accumulated_text(), "before tool");
        let decision = buffer.on_tool_call(
            &ProviderMessage::tool_use(json!({"name": "Bash"}), Some("tool-1".to_owned()), None),
            now,
        );

        assert_eq!(
            decision,
            PluginStreamSendDecision::FlushNow {
                content_text: "before tool\n\n🔧 #1 Bash".to_owned()
            }
        );
        assert!(buffer.is_tool_placeholder_active());
    }

    #[test]
    fn buffered_stream_text_delta_clears_tool_placeholder_and_resets_numbering() {
        let now = Instant::now();
        let mut buffer = PluginStreamSendState::default();
        let _ = buffer.on_tool_call(
            &ProviderMessage::tool_use(json!({"name": "Read"}), Some("tool-1".to_owned()), None),
            now,
        );
        buffer.on_delta("done", now);

        assert_eq!(buffer.render_content_text(), "done");
        assert!(!buffer.is_tool_placeholder_active());
        assert_eq!(
            buffer.on_tool_call(
                &ProviderMessage::tool_use(
                    json!({"name": "Bash"}),
                    Some("tool-2".to_owned()),
                    None,
                ),
                now,
            ),
            PluginStreamSendDecision::FlushNow {
                content_text: "done\n\n🔧 #1 Bash".to_owned()
            }
        );
    }

    #[test]
    fn buffered_stream_text_applies_assistant_segment_boundary() {
        let now = Instant::now();
        let mut buffer = PluginStreamSendState::default();
        buffer.on_delta("first", now);
        buffer.apply_boundary(StreamBoundaryKind::AssistantSegment);
        buffer.on_delta("second", now);

        assert_eq!(buffer.accumulated_text(), "first\n\nsecond");
    }

    #[test]
    fn telegram_like_policy_flushes_text_with_throttle_but_tools_immediately() {
        let now = Instant::now();
        let mut buffer = PluginStreamSendState::new(PluginStreamSendPolicy::telegram_like());

        assert_eq!(
            buffer.on_delta("first", now),
            PluginStreamSendDecision::FlushNow {
                content_text: "first".to_owned()
            }
        );
        buffer.mark_flushed(now);
        assert!(matches!(
            buffer.on_delta(" second", now + Duration::from_millis(50)),
            PluginStreamSendDecision::ScheduleFlush { .. }
        ));
        assert!(buffer.flush_scheduled());

        assert_eq!(
            buffer.on_tool_call(
                &ProviderMessage::tool_use(
                    json!({"name": "Read"}),
                    Some("tool-1".to_owned()),
                    None,
                ),
                now + Duration::from_millis(60),
            ),
            PluginStreamSendDecision::FlushNow {
                content_text: "first second\n\n🔧 #1 Read".to_owned()
            }
        );
        assert!(!buffer.flush_scheduled());
    }
}
