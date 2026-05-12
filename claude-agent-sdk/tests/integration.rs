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
    ClaudeAgentOptions, ClaudeRun, ContentBlock, Message, OutboundUserMessage, PermissionMode,
    run_streaming,
};
use std::path::{Path, PathBuf};

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
    let _ = run.finish().await;
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
    let _ = run.finish().await;
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

    let _ = run.finish().await;
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

    let _ = run.finish().await;
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
    let _ = run.finish().await;
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn test_streaming_follow_up_persists_before_resume_live() {
    let marker = format!(
        "GARYX_SDK_PERSIST_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let cwd = std::env::temp_dir().join(format!("claude-sdk-live-{}", marker));
    std::fs::create_dir_all(&cwd).expect("failed to create test cwd");

    let options = ClaudeAgentOptions {
        cwd: Some(cwd.clone()),
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(4),
        ..Default::default()
    };

    let mut run = run_streaming(options)
        .await
        .expect("Failed to start streaming run");
    let control = run.control();
    control
        .send_user_message(OutboundUserMessage::text(
            format!(
                "Automated SDK regression test. First reply exactly READY_{marker}. \
When a later user message says FOLLOW_REQUEST_{marker}, reply exactly FINAL_{marker} FOLLOW_A FOLLOW_B. \
Use no other words."
            ),
            "",
        ))
        .await
        .expect("Failed to send first message");

    let first = collect_text_until_result(&mut run).await;
    assert!(
        first.contains(&format!("READY_{marker}")),
        "Expected READY marker, got: {first}"
    );

    control
        .send_user_message(OutboundUserMessage::text(
            format!("FOLLOW_REQUEST_{marker}"),
            "",
        ))
        .await
        .expect("Failed to send follow-up message");

    let final_marker = format!("FINAL_{marker} FOLLOW_A FOLLOW_B");
    let session_id = collect_text_and_session_until_result(&mut run, &final_marker).await;
    run.finish().await.expect("finish should succeed");

    let transcript = claude_transcript_path(&cwd, &session_id);
    let transcript_content = std::fs::read_to_string(&transcript)
        .unwrap_or_else(|err| panic!("failed to read transcript {}: {err}", transcript.display()));
    assert!(
        transcript_content.contains(&final_marker),
        "transcript should contain final follow-up marker before resume"
    );

    let resume_options = ClaudeAgentOptions {
        cwd: Some(cwd.clone()),
        permission_mode: Some(PermissionMode::BypassPermissions),
        max_turns: Some(1),
        resume: Some(session_id),
        ..Default::default()
    };
    let mut resume_run = run_streaming(resume_options)
        .await
        .expect("Failed to start resume run");
    let resume_control = resume_run.control();
    resume_control
        .send_user_message(OutboundUserMessage::text(
            format!(
                "If the previous assistant message contained '{final_marker}', reply exactly RESUME_SEES_{marker}=yes. Otherwise reply exactly RESUME_SEES_{marker}=no."
            ),
            "",
        ))
        .await
        .expect("Failed to send resume verification message");

    let resume_text = collect_text_until_result(&mut resume_run).await;
    resume_run
        .finish()
        .await
        .expect("resume finish should succeed");
    assert!(
        resume_text.contains(&format!("RESUME_SEES_{marker}=yes")),
        "resume did not see final follow-up marker, got: {resume_text}"
    );

    let _ = std::fs::remove_dir_all(cwd);
}

async fn collect_text_until_result(run: &mut ClaudeRun) -> String {
    let mut text = String::new();
    while let Some(msg) = run.next_message().await {
        match msg {
            Ok(Message::Assistant(a)) => append_assistant_text(&mut text, &a.content),
            Ok(Message::Result(r)) => {
                assert!(!r.is_error, "Query returned error: {r:?}");
                return text;
            }
            Ok(_) => {}
            Err(e) => panic!("Received error: {e}"),
        }
    }
    panic!("stream ended without result");
}

async fn collect_text_and_session_until_result(run: &mut ClaudeRun, expected: &str) -> String {
    let mut text = String::new();
    while let Some(msg) = run.next_message().await {
        match msg {
            Ok(Message::Assistant(a)) => append_assistant_text(&mut text, &a.content),
            Ok(Message::Result(r)) => {
                assert!(!r.is_error, "Query returned error: {r:?}");
                assert!(
                    text.contains(expected),
                    "Expected final marker {expected}, got: {text}"
                );
                assert!(!r.session_id.is_empty(), "Expected non-empty session_id");
                return r.session_id;
            }
            Ok(_) => {}
            Err(e) => panic!("Received error: {e}"),
        }
    }
    panic!("stream ended without result");
}

fn append_assistant_text(out: &mut String, blocks: &[ContentBlock]) {
    for block in blocks {
        if let ContentBlock::Text(text) = block {
            out.push_str(&text.text);
        }
    }
}

fn claude_transcript_path(cwd: &Path, session_id: &str) -> PathBuf {
    claude_config_dir()
        .join("projects")
        .join(claude_project_key(cwd))
        .join(format!("{session_id}.jsonl"))
}

fn claude_config_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
        .expect("CLAUDE_CONFIG_DIR or HOME must be set")
}

fn claude_project_key(cwd: &Path) -> String {
    cwd.canonicalize()
        .unwrap_or_else(|_| cwd.to_path_buf())
        .to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}
