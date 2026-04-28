use super::{ClaudeSDKClient, Prompt, build_user_message_payload};
use crate::types::ClaudeAgentOptions;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

fn write_mock_claude_script(name: &str, body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "claude-sdk-client-{name}-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    fs::write(&path, body).expect("failed to write mock claude script");
    let mut perms = fs::metadata(&path)
        .expect("failed to stat mock claude script")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("failed to chmod mock claude script");
    path
}

#[test]
fn test_build_user_message_payload_omits_empty_optional_fields() {
    let payload = build_user_message_payload(
        Value::String("hello".to_owned()),
        Some(""),
        Option::<&str>::None,
    );

    assert_eq!(payload["type"], "user");
    assert!(payload.get("session_id").is_none());
    assert!(payload.get("parent_tool_use_id").is_none());
}

#[test]
fn test_build_user_message_payload_includes_non_empty_optional_fields() {
    let payload = build_user_message_payload(
        Value::String("hello".to_owned()),
        Some("session-1"),
        Some("tool-1"),
    );

    assert_eq!(payload["type"], "user");
    assert_eq!(payload["session_id"], "session-1");
    assert_eq!(payload["parent_tool_use_id"], "tool-1");
}

#[tokio::test]
async fn test_connect_failure_resets_client_state_for_retry() {
    let script = write_mock_claude_script("fail-connect", "#!/bin/sh\nexit 0\n");
    let options = ClaudeAgentOptions {
        cli_path: Some(script.clone()),
        ..ClaudeAgentOptions::default()
    };

    let mut client = ClaudeSDKClient::new(options);
    let err = client.connect(None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("CLI process exited before responding")
    );

    assert!(client.transport.is_none());
    assert!(client.reader_handle.is_none());
    assert!(client.stream_handle.is_none());
    assert!(client.pending.lock().await.is_empty());
    assert!(client.msg_tx.is_some());
    assert!(client.msg_rx.is_some());
    assert!(!client.closed.load(Ordering::SeqCst));

    client
        .connect(Some(Prompt::Text("hello".to_owned())))
        .await
        .expect("retry connect should succeed");
    assert!(client.transport.is_some());
    assert!(client.reader_handle.is_some());

    client
        .disconnect()
        .await
        .expect("disconnect should succeed");
    let _ = fs::remove_file(script);
}

#[tokio::test]
async fn test_disconnect_resets_client_state_for_reconnect() {
    let script = write_mock_claude_script("reconnect", "#!/bin/sh\nexit 0\n");
    let options = ClaudeAgentOptions {
        cli_path: Some(script.clone()),
        ..ClaudeAgentOptions::default()
    };

    let mut client = ClaudeSDKClient::new(options);
    client
        .connect(Some(Prompt::Text("first".to_owned())))
        .await
        .expect("initial connect should succeed");

    client
        .disconnect()
        .await
        .expect("disconnect should succeed");

    assert!(client.transport.is_none());
    assert!(client.reader_handle.is_none());
    assert!(client.stream_handle.is_none());
    assert!(client.pending.lock().await.is_empty());
    assert!(client.msg_tx.is_some());
    assert!(client.msg_rx.is_some());
    assert!(!client.closed.load(Ordering::SeqCst));

    client
        .connect(Some(Prompt::Text("second".to_owned())))
        .await
        .expect("reconnect should succeed");
    assert!(client.transport.is_some());
    assert!(client.reader_handle.is_some());

    client
        .disconnect()
        .await
        .expect("final disconnect should succeed");
    let _ = fs::remove_file(script);
}
