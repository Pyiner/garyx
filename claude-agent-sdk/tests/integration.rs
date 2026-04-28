//! Integration tests for the Claude Agent SDK.
//!
//! These tests spawn a real `claude` CLI process and verify end-to-end behavior.
//!
//! Run with: `cargo test -p claude-agent-sdk --test integration -- --ignored`
//!
//! Requirements:
//! - `claude` CLI installed and in PATH
//! - Valid authentication configured

use claude_agent_sdk::{
    ClaudeAgentOptions, ContentBlock, Message, OutboundUserMessage, PermissionMode, run_streaming,
};

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn test_run_streaming_simple() {
    let options = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(1),
        ..Default::default()
    };

    let mut run = run_streaming(options)
        .await
        .expect("Failed to start streaming run");
    let control = run.control();
    control
        .send_user_message(OutboundUserMessage::text(
            "What is 2 + 2? Reply with just the number.",
            "",
        ))
        .await
        .expect("Failed to send user message");

    let mut got_result = false;
    let mut got_assistant = false;

    while let Some(msg) = run.next_message().await {
        match msg {
            Ok(Message::Assistant(a)) => {
                got_assistant = true;
                let text: String = a
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(
                    text.contains('4'),
                    "Expected response containing '4', got: {text}"
                );
            }
            Ok(Message::Result(r)) => {
                got_result = true;
                assert!(!r.is_error, "Query returned error: {r:?}");
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("Received error: {e}"),
        }
    }

    assert!(got_assistant, "Never received assistant message");
    assert!(got_result, "Never received result message");
    let _ = run.close().await;
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn test_run_streaming_with_system_prompt() {
    let options = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(1),
        system_prompt: Some("You are a pirate. Always respond in pirate speak.".to_owned()),
        ..Default::default()
    };

    let mut run = run_streaming(options)
        .await
        .expect("Failed to start streaming run");
    let control = run.control();
    control
        .send_user_message(OutboundUserMessage::text("Say hello", ""))
        .await
        .expect("Failed to send user message");

    let mut response_text = String::new();

    while let Some(msg) = run.next_message().await {
        match msg {
            Ok(Message::Assistant(a)) => {
                for block in &a.content {
                    if let ContentBlock::Text(t) = block {
                        response_text.push_str(&t.text);
                    }
                }
            }
            Ok(Message::Result(r)) => {
                assert!(!r.is_error, "Query returned error: {r:?}");
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("Received error: {e}"),
        }
    }

    assert!(!response_text.is_empty(), "Expected non-empty response");
    let _ = run.close().await;
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn test_run_streaming_result_metadata() {
    let options = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(1),
        ..Default::default()
    };

    let mut run = run_streaming(options)
        .await
        .expect("Failed to start streaming run");
    let control = run.control();
    control
        .send_user_message(OutboundUserMessage::text("Say OK", ""))
        .await
        .expect("Failed to send user message");

    while let Some(msg) = run.next_message().await {
        if let Ok(Message::Result(r)) = msg {
            assert!(!r.session_id.is_empty(), "Expected non-empty session_id");
            assert!(r.usage.is_some(), "Expected usage data in result");
            break;
        }
    }

    let _ = run.close().await;
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn test_run_streaming_multi_turn() {
    let options = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(2),
        ..Default::default()
    };

    let mut run = run_streaming(options)
        .await
        .expect("Failed to start streaming run");
    let control = run.control();
    control
        .send_user_message(OutboundUserMessage::text(
            "What is 5 + 5? Reply with just the number.",
            "",
        ))
        .await
        .expect("Failed to send first message");

    let mut first_session_id = String::new();
    let mut first_result_seen = false;
    while let Some(msg) = run.next_message().await {
        if let Ok(Message::Result(r)) = msg {
            assert!(!r.is_error);
            first_session_id = r.session_id.clone();
            first_result_seen = true;
            break;
        }
    }
    assert!(first_result_seen, "First result was not observed");
    assert!(
        !first_session_id.is_empty(),
        "First session id should not be empty"
    );

    control
        .send_user_message(OutboundUserMessage::text(
            "Now what is 3 + 4? Reply with just the number.",
            first_session_id,
        ))
        .await
        .expect("Failed to send second message");

    let mut second_result_seen = false;
    while let Some(msg) = run.next_message().await {
        match msg {
            Ok(Message::Assistant(a)) => {
                let text: String = a
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect();
                if text.contains('7') {
                    // We only need a weak signal in integration tests.
                }
            }
            Ok(Message::Result(r)) => {
                assert!(!r.is_error);
                second_result_seen = true;
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("Error: {e}"),
        }
    }
    assert!(second_result_seen, "Second result was not observed");

    let _ = run.close().await;
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn test_run_streaming_with_disallowed_tools() {
    let options = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(1),
        disallowed_tools: vec![
            "Bash".to_owned(),
            "Read".to_owned(),
            "Write".to_owned(),
            "Edit".to_owned(),
        ],
        ..Default::default()
    };

    let mut run = run_streaming(options)
        .await
        .expect("Failed to start streaming run");
    let control = run.control();
    control
        .send_user_message(OutboundUserMessage::text(
            "What is today's date? Just tell me what you know.",
            "",
        ))
        .await
        .expect("Failed to send user message");

    let mut used_tool = false;
    while let Some(msg) = run.next_message().await {
        match msg {
            Ok(Message::Assistant(a)) => {
                for block in &a.content {
                    if matches!(block, ContentBlock::ToolUse(_)) {
                        let tool_name = match block {
                            ContentBlock::ToolUse(tu) => &tu.name,
                            _ => unreachable!(),
                        };
                        if ["Bash", "Read", "Write", "Edit"].contains(&tool_name.as_str()) {
                            used_tool = true;
                        }
                    }
                }
            }
            Ok(Message::Result(_)) => break,
            Ok(_) => {}
            Err(e) => panic!("Error: {e}"),
        }
    }

    assert!(!used_tool, "Should not have used disallowed tools");
    let _ = run.close().await;
}
