//! Model-neutral conversation primitives used by the loop core and adapters.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_json_null() -> Value {
    Value::Null
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationRole {
    User,
    Assistant,
    System,
    ToolUse,
    ToolResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: ConversationRole,

    #[serde(default = "default_json_null", skip_serializing_if = "Value::is_null")]
    pub content: Value,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl ConversationMessage {
    pub fn user_text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            role: ConversationRole::User,
            content: Value::String(text.clone()),
            text: Some(text),
            timestamp: None,
            metadata: HashMap::new(),
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            role: ConversationRole::Assistant,
            content: Value::String(text.clone()),
            text: Some(text),
            timestamp: None,
            metadata: HashMap::new(),
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        }
    }

    pub fn system_text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            role: ConversationRole::System,
            content: Value::String(text.clone()),
            text: Some(text),
            timestamp: None,
            metadata: HashMap::new(),
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        }
    }

    pub fn tool_use(
        content: Value,
        tool_call_id: Option<String>,
        tool_name: Option<String>,
    ) -> Self {
        Self {
            role: ConversationRole::ToolUse,
            content,
            text: None,
            timestamp: None,
            metadata: HashMap::new(),
            tool_call_id,
            tool_name,
            is_error: None,
        }
    }

    pub fn tool_result(
        content: Value,
        tool_call_id: Option<String>,
        tool_name: Option<String>,
        is_error: Option<bool>,
    ) -> Self {
        Self {
            role: ConversationRole::ToolResult,
            content,
            text: None,
            timestamp: None,
            metadata: HashMap::new(),
            tool_call_id,
            tool_name,
            is_error,
        }
    }

    pub fn role_str(&self) -> &'static str {
        match self.role {
            ConversationRole::User => "user",
            ConversationRole::Assistant => "assistant",
            ConversationRole::System => "system",
            ConversationRole::ToolUse => "tool_use",
            ConversationRole::ToolResult => "tool_result",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUserInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_input_id: Option<String>,
    pub message: String,
}

impl PendingUserInput {
    pub fn text(message: impl Into<String>) -> Self {
        Self {
            pending_input_id: None,
            message: message.into(),
        }
    }

    pub fn with_pending_input_id(mut self, pending_input_id: impl Into<String>) -> Self {
        self.pending_input_id = Some(pending_input_id.into());
        self
    }
}
