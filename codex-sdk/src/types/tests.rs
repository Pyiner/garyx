use super::*;
use serde_json::json;

// -- Serialization tests --

#[test]
fn test_json_rpc_request_serialization() {
    let req = JsonRpcRequest {
        id: 1,
        method: "initialize".to_owned(),
        params: Some(json!({"key": "value"})),
    };
    let val = serde_json::to_value(&req).unwrap();
    assert_eq!(val["id"], 1);
    assert_eq!(val["method"], "initialize");
    assert_eq!(val["params"]["key"], "value");
}

#[test]
fn test_json_rpc_request_no_params() {
    let req = JsonRpcRequest {
        id: 2,
        method: "initialized".to_owned(),
        params: None,
    };
    let val = serde_json::to_value(&req).unwrap();
    assert!(val.get("params").is_none());
}

#[test]
fn test_json_rpc_notification_out_serialization() {
    let notif = JsonRpcNotificationOut {
        method: "initialized".to_owned(),
        params: Some(json!({})),
    };
    let val = serde_json::to_value(&notif).unwrap();
    assert_eq!(val["method"], "initialized");
    assert!(val.get("id").is_none());
}

#[test]
fn test_json_rpc_error_serialization() {
    let err = JsonRpcError {
        code: -32001,
        message: "overloaded".to_owned(),
        data: None,
    };
    let val = serde_json::to_value(&err).unwrap();
    assert_eq!(val["code"], -32001);
    assert_eq!(val["message"], "overloaded");
    assert!(val.get("data").is_none());
}

#[test]
fn test_json_rpc_server_response_result() {
    let resp = JsonRpcServerResponse {
        id: json!(10),
        result: Some(json!({"decision": "accept"})),
        error: None,
    };
    let val = serde_json::to_value(&resp).unwrap();
    assert_eq!(val["id"], 10);
    assert_eq!(val["result"]["decision"], "accept");
    assert!(val.get("error").is_none());
}

#[test]
fn test_json_rpc_server_response_error() {
    let resp = JsonRpcServerResponse {
        id: json!(11),
        result: None,
        error: Some(JsonRpcError {
            code: -32601,
            message: "not supported".to_owned(),
            data: None,
        }),
    };
    let val = serde_json::to_value(&resp).unwrap();
    assert_eq!(val["error"]["code"], -32601);
    assert!(val.get("result").is_none());
}

// -- Initialize types --

#[test]
fn test_initialize_params_serialization() {
    let params = InitializeParams {
        client_info: ClientInfo {
            name: "test".to_owned(),
            title: "Test".to_owned(),
            version: "0.1.0".to_owned(),
        },
        capabilities: Capabilities {
            experimental_api: true,
        },
    };
    let val = serde_json::to_value(&params).unwrap();
    assert_eq!(val["clientInfo"]["name"], "test");
    assert_eq!(val["capabilities"]["experimentalApi"], true);
}

// -- Thread types --

#[test]
fn test_thread_start_params_serialization() {
    let params = ThreadStartParams {
        cwd: Some("/tmp".to_owned()),
        config: None,
        model: Some("o3-mini".to_owned()),
        model_reasoning_effort: Some("xhigh".to_owned()),
        approval_policy: None,
        sandbox: None,
    };
    let val = serde_json::to_value(&params).unwrap();
    assert_eq!(val["cwd"], "/tmp");
    assert_eq!(val["model"], "o3-mini");
    assert_eq!(val["modelReasoningEffort"], "xhigh");
    assert!(val.get("approvalPolicy").is_none());
    assert!(val.get("sandbox").is_none());
}

#[test]
fn test_thread_resume_params_serialization() {
    let params = ThreadResumeParams {
        thread_id: "th_abc".to_owned(),
        cwd: None,
        config: None,
        model: None,
        model_reasoning_effort: Some("high".to_owned()),
        approval_policy: Some("never".to_owned()),
        sandbox: Some("off".to_owned()),
    };
    let val = serde_json::to_value(&params).unwrap();
    assert_eq!(val["threadId"], "th_abc");
    assert_eq!(val["modelReasoningEffort"], "high");
    assert_eq!(val["approvalPolicy"], "never");
    assert_eq!(val["sandbox"], "off");
}

// -- Input items --

#[test]
fn test_input_item_text_serialization() {
    let item = InputItem::Text {
        text: "hello".to_owned(),
    };
    let val = serde_json::to_value(&item).unwrap();
    assert_eq!(val["type"], "text");
    assert_eq!(val["text"], "hello");
}

#[test]
fn test_input_item_image_serialization() {
    let item = InputItem::Image {
        url: "data:image/png;base64,abc".to_owned(),
    };
    let val = serde_json::to_value(&item).unwrap();
    assert_eq!(val["type"], "image");
    assert_eq!(val["url"], "data:image/png;base64,abc");
}

// -- Turn types --

#[test]
fn test_turn_start_params_serialization() {
    let params = TurnStartParams {
        thread_id: "th_1".to_owned(),
        input: vec![InputItem::Text {
            text: "hi".to_owned(),
        }],
    };
    let val = serde_json::to_value(&params).unwrap();
    assert_eq!(val["threadId"], "th_1");
    assert_eq!(val["input"][0]["type"], "text");
    assert_eq!(val["input"][0]["text"], "hi");
}

#[test]
fn test_turn_steer_params_serialization() {
    let params = TurnSteerParams {
        thread_id: "th_1".to_owned(),
        turn_id: Some("turn_1".to_owned()),
        expected_turn_id: "turn_1".to_owned(),
        input: vec![InputItem::Text {
            text: "more".to_owned(),
        }],
    };
    let val = serde_json::to_value(&params).unwrap();
    assert_eq!(val["threadId"], "th_1");
    assert_eq!(val["turnId"], "turn_1");
    assert_eq!(val["expectedTurnId"], "turn_1");
}

#[test]
fn test_turn_steer_params_omits_legacy_turn_id_when_absent() {
    let params = TurnSteerParams {
        thread_id: "th_1".to_owned(),
        turn_id: None,
        expected_turn_id: "turn_1".to_owned(),
        input: vec![InputItem::Text {
            text: "more".to_owned(),
        }],
    };
    let val = serde_json::to_value(&params).unwrap();
    assert_eq!(val["threadId"], "th_1");
    assert_eq!(val["expectedTurnId"], "turn_1");
    assert!(val.get("turnId").is_none());
}

#[test]
fn test_turn_interrupt_params_serialization() {
    let params = TurnInterruptParams {
        thread_id: "th_1".to_owned(),
        turn_id: "turn_1".to_owned(),
    };
    let val = serde_json::to_value(&params).unwrap();
    assert_eq!(val["threadId"], "th_1");
    assert_eq!(val["turnId"], "turn_1");
}

// -- Notification deserialization --

#[test]
fn test_agent_message_delta_deserialization() {
    let val = json!({"threadId": "t1", "turnId": "u1", "delta": "hello "});
    let delta: AgentMessageDelta = serde_json::from_value(val).unwrap();
    assert_eq!(delta.thread_id, "t1");
    assert_eq!(delta.turn_id, "u1");
    assert_eq!(delta.delta, "hello ");
}

#[test]
fn test_turn_completed_deserialization() {
    let val = json!({
        "threadId": "t1",
        "turn": {
            "id": "u1",
            "status": "completed",
            "usage": {
                "inputTokens": 100,
                "outputTokens": 50,
                "totalCostUsd": 0.005
            }
        }
    });
    let params: TurnCompletedParams = serde_json::from_value(val).unwrap();
    assert_eq!(params.thread_id, "t1");
    assert_eq!(params.turn.id, "u1");
    assert_eq!(params.turn.status, "completed");
    assert!(params.turn.usage.is_some());
}

#[test]
fn test_turn_info_failed_with_error() {
    let val = json!({
        "id": "u2",
        "status": "failed",
        "error": {"message": "something broke"}
    });
    let info: TurnInfo = serde_json::from_value(val).unwrap();
    assert_eq!(info.status, "failed");
    assert!(info.error.is_some());
}

// -- Helper function tests --

#[test]
fn test_extract_thread_id_from_thread_object() {
    assert_eq!(
        extract_thread_id(&json!({"thread": {"id": "th_abc"}})),
        Some("th_abc".to_owned())
    );
}

#[test]
fn test_extract_thread_id_from_thread_id_key() {
    assert_eq!(
        extract_thread_id(&json!({"threadId": "th_def"})),
        Some("th_def".to_owned())
    );
}

#[test]
fn test_extract_thread_id_from_snake_case() {
    assert_eq!(
        extract_thread_id(&json!({"thread_id": "th_ghi"})),
        Some("th_ghi".to_owned())
    );
}

#[test]
fn test_extract_thread_id_none() {
    assert_eq!(extract_thread_id(&json!({"foo": "bar"})), None);
}

#[test]
fn test_extract_thread_id_empty() {
    assert_eq!(extract_thread_id(&json!({"threadId": ""})), None);
}

#[test]
fn test_extract_turn_id_from_turn_object() {
    assert_eq!(
        extract_turn_id(&json!({"turn": {"id": "turn_abc"}})),
        Some("turn_abc".to_owned())
    );
}

#[test]
fn test_extract_turn_id_from_turn_id_key() {
    assert_eq!(
        extract_turn_id(&json!({"turnId": "turn_def"})),
        Some("turn_def".to_owned())
    );
}

#[test]
fn test_extract_turn_id_none() {
    assert_eq!(extract_turn_id(&json!({})), None);
}

#[test]
fn test_coerce_i64_number() {
    assert_eq!(coerce_i64(&json!(42)), 42);
    assert_eq!(coerce_i64(&json!(-5)), -5);
}

#[test]
fn test_coerce_i64_string() {
    assert_eq!(coerce_i64(&json!("100")), 100);
    assert_eq!(coerce_i64(&json!("bad")), 0);
}

#[test]
fn test_coerce_i64_other() {
    assert_eq!(coerce_i64(&json!(null)), 0);
    assert_eq!(coerce_i64(&json!(true)), 0);
}

#[test]
fn test_coerce_f64_number() {
    assert!((coerce_f64(&json!(2.5)) - 2.5).abs() < f64::EPSILON);
}

#[test]
fn test_coerce_f64_string() {
    assert!((coerce_f64(&json!("2.5")) - 2.5).abs() < f64::EPSILON);
    assert!((coerce_f64(&json!("bad"))).abs() < f64::EPSILON);
}
