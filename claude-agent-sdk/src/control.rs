use std::collections::HashMap;

use crate::types::McpServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Client -> Server  (SDK sends to CLI)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SDKControlRequest {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: String,
    pub request: ControlRequestKind,
}

impl SDKControlRequest {
    pub fn new(request_id: impl Into<String>, request: ControlRequestKind) -> Self {
        Self {
            msg_type: "control_request".into(),
            request_id: request_id.into(),
            request,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum ControlRequestKind {
    Interrupt,
    Initialize {
        hooks: Option<Value>,
    },
    #[serde(rename = "set_permission_mode")]
    SetPermissionMode {
        mode: String,
    },
    #[serde(rename = "set_model")]
    SetModel {
        model: Option<String>,
    },
    #[serde(rename = "set_max_thinking_tokens")]
    SetMaxThinkingTokens {
        max_thinking_tokens: Option<i64>,
    },
    #[serde(rename = "mcp_set_servers")]
    McpSetServers {
        servers: HashMap<String, McpServerConfig>,
    },
    #[serde(rename = "mcp_reconnect")]
    McpReconnect {
        #[serde(rename = "serverName")]
        server_name: String,
    },
    #[serde(rename = "mcp_toggle")]
    McpToggle {
        #[serde(rename = "serverName")]
        server_name: String,
        enabled: bool,
    },
    #[serde(rename = "rewind_files")]
    RewindFiles {
        user_message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dry_run: Option<bool>,
    },
    #[serde(rename = "mcp_status")]
    McpStatus,
    #[serde(rename = "stop_task")]
    StopTask {
        task_id: String,
    },
    #[serde(rename = "apply_flag_settings")]
    ApplyFlagSettings {
        settings: Value,
    },
}

// ---------------------------------------------------------------------------
// Server -> Client  (CLI sends to SDK)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SDKControlResponse {
    #[serde(rename = "type")]
    pub _msg_type: String,
    pub response: ControlResponsePayload,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum ControlResponsePayload {
    Success {
        request_id: String,
        response: Option<Value>,
    },
    Error {
        request_id: String,
        error: String,
    },
}

impl ControlResponsePayload {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Success { request_id, .. } => request_id,
            Self::Error { request_id, .. } => request_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Incoming control requests (CLI asks SDK for something)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct IncomingControlRequest {
    #[serde(rename = "type")]
    pub _msg_type: String,
    pub request_id: String,
    pub request: IncomingRequestPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanUseToolRequest {
    pub tool_name: String,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_suggestions: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookCallbackRequest {
    pub callback_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpMessageRequest {
    pub server_name: String,
    pub message: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationRequest {
    pub mcp_server_name: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elicitation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum IncomingRequestPayload {
    #[serde(rename = "can_use_tool")]
    CanUseTool(CanUseToolRequest),
    #[serde(rename = "hook_callback")]
    HookCallback(HookCallbackRequest),
    #[serde(rename = "mcp_message")]
    McpMessage(McpMessageRequest),
    #[serde(rename = "elicitation")]
    Elicitation(ElicitationRequest),
}

// ---------------------------------------------------------------------------
// Control response we send back (to answer an incoming request)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ControlResponseMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub response: ControlResponseBody,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum ControlResponseBody {
    Error { request_id: String, error: String },
}

impl ControlResponseMessage {
    pub fn error(request_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            msg_type: "control_response".into(),
            response: ControlResponseBody::Error {
                request_id: request_id.into(),
                error: error.into(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
