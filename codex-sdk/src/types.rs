//! JSON-RPC types and domain-specific request/response structures for the Codex
//! app-server protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// JSON-RPC wire types
// ---------------------------------------------------------------------------

/// An outgoing JSON-RPC request (client -> server).
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// An outgoing JSON-RPC notification (client -> server, no response expected).
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcNotificationOut {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A server-to-client JSON-RPC notification (no `id`).
#[derive(Debug, Clone)]
pub struct JsonRpcNotification {
    pub method: String,
    pub params: Value,
}

/// A JSON-RPC response (server -> client).
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A response to a server-initiated request (client -> server).
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcServerResponse {
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

// ---------------------------------------------------------------------------
// Initialize
// ---------------------------------------------------------------------------

/// Parameters for the `initialize` request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub client_info: ClientInfo,
    pub capabilities: Capabilities,
}

/// Client identification sent during initialize.
#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub title: String,
    pub version: String,
}

/// Client capabilities sent during initialize.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub experimental_api: bool,
}

// ---------------------------------------------------------------------------
// Thread lifecycle
// ---------------------------------------------------------------------------

/// Parameters for `thread/start`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
}

/// Parameters for `thread/resume`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeParams {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
}

// ---------------------------------------------------------------------------
// Turn lifecycle
// ---------------------------------------------------------------------------

/// An input item for `turn/start` or `turn/steer`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputItem {
    Text { text: String },
    Image { url: String },
}

/// Parameters for `turn/start`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams {
    pub thread_id: String,
    pub input: Vec<InputItem>,
}

/// Parameters for `turn/steer`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSteerParams {
    pub thread_id: String,
    /// Backward-compatible active turn id field used by older app-server builds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Required by the current app-server protocol as an active-turn precondition.
    pub expected_turn_id: String,
    pub input: Vec<InputItem>,
}

/// Parameters for `turn/interrupt`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptParams {
    pub thread_id: String,
    pub turn_id: String,
}

// ---------------------------------------------------------------------------
// Server notifications
// ---------------------------------------------------------------------------

/// Parameters for `item/agentMessage/delta`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageDelta {
    pub thread_id: String,
    pub turn_id: String,
    pub delta: String,
}

/// Parameters for `item/started` and `item/completed`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemEventParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item: Value,
}

/// Parameters for `turn/completed`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCompletedParams {
    pub thread_id: String,
    pub turn: TurnInfo,
}

/// Information about a completed turn.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInfo {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub usage: Option<UsageInfo>,
}

/// Token usage and cost information.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    #[serde(
        alias = "input_tokens",
        alias = "input",
        alias = "prompt_tokens",
        default
    )]
    pub input_tokens: Option<Value>,
    #[serde(
        alias = "output_tokens",
        alias = "output",
        alias = "completion_tokens",
        default
    )]
    pub output_tokens: Option<Value>,
    #[serde(alias = "total_cost_usd", alias = "costUsd", alias = "cost", default)]
    pub total_cost_usd: Option<Value>,
}

// ---------------------------------------------------------------------------
// Server-initiated requests
// ---------------------------------------------------------------------------

/// Parameters for `item/commandExecution/requestApproval`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandApprovalRequest {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub command: String,
}

/// Parameters for `item/fileChange/requestApproval`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeApprovalRequest {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub file_path: String,
    pub change_type: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a thread ID from a JSON-RPC response payload.
///
/// Tries `payload.thread.id`, then `payload.threadId`, then `payload.thread_id`.
pub fn extract_thread_id(payload: &Value) -> Option<String> {
    if let Some(id) = payload
        .get("thread")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
    {
        if !id.is_empty() {
            return Some(id.to_owned());
        }
    }
    for key in &["threadId", "thread_id"] {
        if let Some(id) = payload.get(*key).and_then(|v| v.as_str()) {
            if !id.is_empty() {
                return Some(id.to_owned());
            }
        }
    }
    None
}

/// Extract a turn ID from a JSON-RPC response payload.
///
/// Tries `payload.turn.id`, then `payload.turnId`, then `payload.turn_id`.
pub fn extract_turn_id(payload: &Value) -> Option<String> {
    if let Some(id) = payload
        .get("turn")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
    {
        if !id.is_empty() {
            return Some(id.to_owned());
        }
    }
    for key in &["turnId", "turn_id"] {
        if let Some(id) = payload.get(*key).and_then(|v| v.as_str()) {
            if !id.is_empty() {
                return Some(id.to_owned());
            }
        }
    }
    None
}

/// Coerce a JSON value to i64.
pub fn coerce_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// Coerce a JSON value to f64.
pub fn coerce_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
