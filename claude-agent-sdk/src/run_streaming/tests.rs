use super::*;

#[test]
fn test_outbound_user_message_text() {
    let msg = OutboundUserMessage::text("hello", "session-1");
    assert_eq!(msg.session_id, "session-1");
    assert_eq!(msg.parent_tool_use_id, None);
    assert_eq!(msg.content, UserInput::Text("hello".to_string()));
}

#[test]
fn test_outbound_user_message_blocks() {
    let blocks = vec![serde_json::json!({"type":"text","text":"hello"})];
    let msg = OutboundUserMessage::blocks(blocks.clone(), "session-2");
    assert_eq!(msg.session_id, "session-2");
    assert_eq!(msg.content, UserInput::Blocks(blocks));
}

#[tokio::test]
async fn test_control_close_idempotent_on_unconnected_client() {
    let state = Arc::new(RunState {
        client: Mutex::new(ClaudeSDKClient::new(ClaudeAgentOptions::default())),
        closed: AtomicBool::new(false),
    });
    let control = ClaudeRunControl { state };
    assert!(control.close().await.is_ok());
    assert!(control.close().await.is_ok());
}
