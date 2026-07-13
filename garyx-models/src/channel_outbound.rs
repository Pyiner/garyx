use serde::{Deserialize, Serialize};

use crate::provider::ProviderMessage;

/// Channel-facing outbound content.
///
/// This is the structured boundary between Garyx core and channel adapters:
/// core decides what happened, while each channel decides how to render it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelOutboundContent {
    Text {
        text: String,
    },
    Image {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt: Option<String>,
    },
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
    ToolUse {
        message: ProviderMessage,
    },
    ToolResult {
        message: ProviderMessage,
    },
}

impl ChannelOutboundContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(path: impl Into<String>, alt: Option<String>) -> Self {
        Self::Image {
            path: path.into(),
            alt,
        }
    }

    pub fn file(path: impl Into<String>, caption: Option<String>) -> Self {
        Self::File {
            path: path.into(),
            caption,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }

    pub fn as_image(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::Image { path, alt } => Some((path, alt.as_deref())),
            _ => None,
        }
    }

    pub fn as_file(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::File { path, caption } => Some((path, caption.as_deref())),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Image { .. } => "image",
            Self::File { .. } => "file",
            Self::ToolUse { .. } => "tool_use",
            Self::ToolResult { .. } => "tool_result",
        }
    }
}
