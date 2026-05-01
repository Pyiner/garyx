use serde::{Deserialize, Serialize};

use crate::provider::{ProviderMessage, StreamEvent};

/// Channel-facing outbound content.
///
/// This is the structured boundary between Garyx core and channel adapters:
/// core decides what happened, while each channel decides how to render it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelOutboundContent {
    Text { text: String },
    ToolUse { message: ProviderMessage },
    ToolResult { message: ProviderMessage },
}

impl ChannelOutboundContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn from_stream_event(event: StreamEvent) -> Option<Self> {
        match event {
            StreamEvent::ToolUse { message } => Some(Self::ToolUse { message }),
            StreamEvent::ToolResult { message } => Some(Self::ToolResult { message }),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::ToolUse { .. } => "tool_use",
            Self::ToolResult { .. } => "tool_result",
        }
    }
}
