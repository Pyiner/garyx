use std::collections::HashMap;

use garyx_models::agent_node::*;
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use serde_json::{Value, json};

#[test]
fn register_message_serialization_roundtrip() {
    let msg = NodeToGateway::Register {
        node_id: "node-1".to_owned(),
        hostname: "worker-01.local".to_owned(),
        providers: vec![
            ProviderInfo {
                provider_type: "claude".to_owned(),
                model: Some("claude-sonnet-4-20250514".to_owned()),
                reasoning_effort: None,
                max_concurrent: 4,
            },
            ProviderInfo {
                provider_type: "codex".to_owned(),
                model: None,
                reasoning_effort: Some("xhigh".to_owned()),
                max_concurrent: 2,
            },
        ],
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "register");
    assert_eq!(parsed["node_id"], "node-1");
    assert_eq!(parsed["hostname"], "worker-01.local");
    assert_eq!(parsed["providers"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["providers"][1]["reasoning_effort"], "xhigh");

    let deserialized: NodeToGateway = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn heartbeat_message_serialization() {
    let msg = NodeToGateway::Heartbeat {
        node_id: "node-1".to_owned(),
        load: NodeLoadInfo {
            active_tasks: 2,
            cpu_percent: 45.5,
            memory_percent: 60.0,
        },
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "heartbeat");
    assert_eq!(parsed["load"]["active_tasks"], 2);
    assert_eq!(parsed["load"]["cpu_percent"], 45.5);
}

#[test]
fn run_complete_message_serialization() {
    let msg = NodeToGateway::RunComplete {
        request_id: "req-123".to_owned(),
        success: true,
        result: Some("Task completed successfully".to_owned()),
        error: None,
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "run_complete");
    assert_eq!(parsed["success"], true);
    assert_eq!(parsed["result"], "Task completed successfully");
    assert!(parsed.get("error").is_none());
}

#[test]
fn run_complete_error_message() {
    let msg = NodeToGateway::RunComplete {
        request_id: "req-456".to_owned(),
        success: false,
        result: None,
        error: Some("Provider timeout".to_owned()),
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let deserialized: NodeToGateway = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn clear_session_complete_message_serialization() {
    let msg = NodeToGateway::ClearSessionComplete {
        request_id: "req-clear-1".to_owned(),
        success: true,
        cleared: true,
        error: None,
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "clear_session_complete");
    assert_eq!(parsed["request_id"], "req-clear-1");
    assert_eq!(parsed["success"], true);
    assert_eq!(parsed["cleared"], true);
    assert!(parsed.get("error").is_none());
}

#[test]
fn run_progress_message_serialization() {
    let msg = NodeToGateway::RunProgress {
        request_id: "req-789".to_owned(),
        event: StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        },
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "run_progress");
    assert_eq!(parsed["event"]["type"], "boundary");
    assert_eq!(parsed["event"]["kind"], "assistant_segment");
}

#[test]
fn run_progress_tool_result_roundtrip() {
    let msg = NodeToGateway::RunProgress {
        request_id: "req-tool-1".to_owned(),
        event: StreamEvent::ToolResult {
            message: ProviderMessage::tool_result(
                json!({"text": "done"}),
                Some("call-1".to_owned()),
                None,
                None,
            ),
        },
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: NodeToGateway = serde_json::from_str(&json_str).unwrap();
    match parsed {
        NodeToGateway::RunProgress { request_id, event } => {
            assert_eq!(request_id, "req-tool-1");
            match event {
                StreamEvent::ToolResult { message } => {
                    assert_eq!(message.tool_use_id.as_deref(), Some("call-1"));
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        other => panic!("unexpected message: {other:?}"),
    }
}

#[test]
fn register_ack_message_serialization() {
    let msg = GatewayToNode::RegisterAck {
        accepted: true,
        reason: None,
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "register_ack");
    assert_eq!(parsed["accepted"], true);
    assert!(parsed.get("reason").is_none());
}

#[test]
fn register_ack_rejected() {
    let msg = GatewayToNode::RegisterAck {
        accepted: false,
        reason: Some("Node ID already registered".to_owned()),
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let deserialized: GatewayToNode = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn ping_message_serialization() {
    let msg = GatewayToNode::Ping;
    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "ping");
}

#[test]
fn run_request_message_serialization() {
    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "chrome-devtools".to_owned(),
        json!({
            "command": "npx",
            "args": ["-y", "@anthropic/mcp-chrome-devtools"],
        }),
    );

    let msg = GatewayToNode::RunRequest {
        request_id: "req-001".to_owned(),
        messages: vec![
            json!({ "role": "user", "content": "Hello" }),
            json!({ "role": "assistant", "content": "Hi there!" }),
        ],
        provider_hint: Some("claude".to_owned()),
        mcp_servers,
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "run_request");
    assert_eq!(parsed["messages"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["provider_hint"], "claude");
    assert!(parsed["mcp_servers"]["chrome-devtools"].is_object());
}

#[test]
fn clear_session_request_message_serialization() {
    let msg = GatewayToNode::ClearSessionRequest {
        request_id: "req-clear-1".to_owned(),
        thread_id: "sess::abc".to_owned(),
        provider_hint: Some("claude".to_owned()),
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "clear_session_request");
    assert_eq!(parsed["request_id"], "req-clear-1");
    assert_eq!(parsed["thread_id"], "sess::abc");
    assert_eq!(parsed["provider_hint"], "claude");
}

#[test]
fn abort_request_message_serialization() {
    let msg = GatewayToNode::AbortRequest {
        request_id: "req-abort-1".to_owned(),
        thread_id: "sess::abc".to_owned(),
        provider_hint: Some("claude".to_owned()),
    };

    let json_str = serde_json::to_string(&msg).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "abort_request");
    assert_eq!(parsed["request_id"], "req-abort-1");
    assert_eq!(parsed["thread_id"], "sess::abc");
    assert_eq!(parsed["provider_hint"], "claude");
}

#[test]
fn config_update_message_serialization() {
    let mut mcp_servers = HashMap::new();
    mcp_servers.insert(
        "filesystem".to_owned(),
        json!({
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
        }),
    );

    let msg = GatewayToNode::ConfigUpdate { mcp_servers };
    let json_str = serde_json::to_string(&msg).unwrap();
    let deserialized: GatewayToNode = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized, msg);
}

#[test]
fn provider_info_defaults() {
    let info = ProviderInfo {
        provider_type: "claude".to_owned(),
        model: None,
        reasoning_effort: None,
        max_concurrent: 1,
    };
    let json_str = serde_json::to_string(&info).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["provider_type"], "claude");
    assert!(parsed["model"].is_null());
}

#[test]
fn node_load_info_default() {
    let load = NodeLoadInfo::default();
    assert_eq!(load.active_tasks, 0);
    assert_eq!(load.cpu_percent, 0.0);
    assert_eq!(load.memory_percent, 0.0);
}

#[test]
fn cross_deserialize_node_to_gateway_variants() {
    // Ensure all variants can be deserialized from JSON
    let variants = vec![
        json!({ "type": "register", "node_id": "n1", "hostname": "h1", "providers": [] }),
        json!({ "type": "heartbeat", "node_id": "n1", "load": { "active_tasks": 0, "cpu_percent": 0.0, "memory_percent": 0.0 } }),
        json!({
            "type": "run_progress",
            "request_id": "r1",
            "event": { "type": "delta", "text": "delta" }
        }),
        json!({ "type": "run_complete", "request_id": "r1", "success": true }),
        json!({ "type": "clear_session_complete", "request_id": "r2", "success": true, "cleared": true }),
    ];

    for variant in variants {
        let result: Result<NodeToGateway, _> = serde_json::from_value(variant.clone());
        assert!(
            result.is_ok(),
            "Failed to deserialize: {}",
            serde_json::to_string_pretty(&variant).unwrap()
        );
    }
}

#[test]
fn cross_deserialize_gateway_to_node_variants() {
    let variants = vec![
        json!({ "type": "register_ack", "accepted": true }),
        json!({ "type": "ping" }),
        json!({ "type": "run_request", "request_id": "r1", "messages": [], "mcp_servers": {} }),
        json!({ "type": "clear_session_request", "request_id": "r2", "thread_id": "sess::1" }),
        json!({ "type": "config_update", "mcp_servers": {} }),
    ];

    for variant in variants {
        let result: Result<GatewayToNode, _> = serde_json::from_value(variant.clone());
        assert!(
            result.is_ok(),
            "Failed to deserialize: {}",
            serde_json::to_string_pretty(&variant).unwrap()
        );
    }
}
