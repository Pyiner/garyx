use super::*;
use serde_json::json;

#[test]
fn test_serialize_interrupt_request() {
    let req = SDKControlRequest::new("req-1", ControlRequestKind::Interrupt);
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["type"], "control_request");
    assert_eq!(json["request_id"], "req-1");
    assert_eq!(json["request"]["subtype"], "interrupt");
}

#[test]
fn test_serialize_initialize_request() {
    let req = SDKControlRequest::new("req-2", ControlRequestKind::Initialize { hooks: None });
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["request"]["subtype"], "initialize");
    assert!(json["request"]["hooks"].is_null());
}

#[test]
fn test_serialize_set_permission_mode() {
    let req = SDKControlRequest::new(
        "req-3",
        ControlRequestKind::SetPermissionMode {
            mode: "bypassPermissions".into(),
        },
    );
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["request"]["subtype"], "set_permission_mode");
    assert_eq!(json["request"]["mode"], "bypassPermissions");
}

#[test]
fn test_serialize_set_model() {
    let req = SDKControlRequest::new(
        "req-4",
        ControlRequestKind::SetModel {
            model: Some("claude-sonnet-4-5".into()),
        },
    );
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["request"]["subtype"], "set_model");
    assert_eq!(json["request"]["model"], "claude-sonnet-4-5");
}

#[test]
fn test_serialize_rewind_files() {
    let req = SDKControlRequest::new(
        "req-5",
        ControlRequestKind::RewindFiles {
            user_message_id: "msg-abc".into(),
            dry_run: None,
        },
    );
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["request"]["subtype"], "rewind_files");
    assert_eq!(json["request"]["user_message_id"], "msg-abc");
}

#[test]
fn test_deserialize_success_response() {
    let data = json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": "req-1",
            "response": { "ok": true }
        }
    });

    let resp: SDKControlResponse = serde_json::from_value(data).unwrap();
    assert_eq!(resp._msg_type, "control_response");
    assert_eq!(resp.response.request_id(), "req-1");
    match &resp.response {
        ControlResponsePayload::Success { response, .. } => {
            assert_eq!(response.as_ref().unwrap()["ok"], true);
        }
        _ => panic!("Expected success"),
    }
}

#[test]
fn test_deserialize_error_response() {
    let data = json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": "req-2",
            "error": "something went wrong"
        }
    });

    let resp: SDKControlResponse = serde_json::from_value(data).unwrap();
    match &resp.response {
        ControlResponsePayload::Error { error, .. } => {
            assert_eq!(error, "something went wrong");
        }
        _ => panic!("Expected error"),
    }
}

#[test]
fn test_control_response_message_error() {
    let msg = ControlResponseMessage::error("req-1", "failed");
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["response"]["subtype"], "error");
    assert_eq!(json["response"]["error"], "failed");
}

#[test]
fn test_deserialize_incoming_can_use_tool() {
    let data = json!({
        "type": "control_request",
        "request_id": "req-10",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "Bash",
            "input": { "command": "rm -rf /" },
            "permission_suggestions": null,
            "blocked_path": null,
            "tool_use_id": "toolu_123"
        }
    });

    let req: IncomingControlRequest = serde_json::from_value(data).unwrap();
    assert_eq!(req.request_id, "req-10");
    match &req.request {
        IncomingRequestPayload::CanUseTool(can_use_tool) => {
            assert_eq!(can_use_tool.tool_name, "Bash");
            assert_eq!(can_use_tool.tool_use_id.as_deref(), Some("toolu_123"));
        }
        other => panic!("Expected CanUseTool, got {other:?}"),
    }
}

#[test]
fn test_serialize_set_max_thinking_tokens() {
    let req = SDKControlRequest::new(
        "req-6",
        ControlRequestKind::SetMaxThinkingTokens {
            max_thinking_tokens: Some(1024),
        },
    );
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["request"]["subtype"], "set_max_thinking_tokens");
    assert_eq!(json["request"]["max_thinking_tokens"], 1024);
}

#[test]
fn test_serialize_mcp_toggle() {
    let req = SDKControlRequest::new(
        "req-7",
        ControlRequestKind::McpToggle {
            server_name: "garyx".to_owned(),
            enabled: false,
        },
    );
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["request"]["subtype"], "mcp_toggle");
    assert_eq!(json["request"]["serverName"], "garyx");
    assert_eq!(json["request"]["enabled"], false);
}
