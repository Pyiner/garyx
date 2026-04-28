use std::collections::HashMap;

use crate::provider::StreamEvent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderInfo {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub max_concurrent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NodeLoadInfo {
    pub active_tasks: u32,
    pub cpu_percent: f64,
    pub memory_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeToGateway {
    Register {
        node_id: String,
        hostname: String,
        providers: Vec<ProviderInfo>,
    },
    Heartbeat {
        node_id: String,
        load: NodeLoadInfo,
    },
    RunProgress {
        request_id: String,
        event: StreamEvent,
    },
    RunComplete {
        request_id: String,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    ClearSessionComplete {
        request_id: String,
        success: bool,
        cleared: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayToNode {
    RegisterAck {
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Ping,
    RunRequest {
        request_id: String,
        messages: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_hint: Option<String>,
        mcp_servers: HashMap<String, serde_json::Value>,
    },
    ClearSessionRequest {
        request_id: String,
        thread_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_hint: Option<String>,
    },
    AbortRequest {
        request_id: String,
        thread_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_hint: Option<String>,
    },
    ConfigUpdate {
        mcp_servers: HashMap<String, serde_json::Value>,
    },
}
