use super::{
    ClaudeSDKClient, Prompt, build_user_message_payload, incoming_control_response,
    unsupported_incoming_control_request_response,
};
use crate::control::IncomingControlRequest;
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

#[test]
fn test_incoming_elicitation_declines_by_default() {
    let req: IncomingControlRequest = serde_json::from_value(serde_json::json!({
        "type": "control_request",
        "request_id": "req-elicit",
        "request": {
            "subtype": "elicitation",
            "mcp_server_name": "test-server",
            "message": "Approve?"
        }
    }))
    .unwrap();

    let resp = serde_json::to_value(incoming_control_response(req)).unwrap();
    assert_eq!(resp["type"], "control_response");
    assert_eq!(resp["response"]["subtype"], "success");
    assert_eq!(resp["response"]["request_id"], "req-elicit");
    assert_eq!(resp["response"]["response"]["action"], "decline");
}

#[test]
fn test_unknown_incoming_control_request_returns_error_response() {
    let raw = serde_json::json!({
        "type": "control_request",
        "request_id": "req-unknown",
        "request": {
            "subtype": "oauth_token_refresh"
        }
    });
    let err = serde_json::from_value::<IncomingControlRequest>(raw.clone()).unwrap_err();

    let resp = unsupported_incoming_control_request_response(&raw, &err).unwrap();
    let resp = serde_json::to_value(resp).unwrap();

    assert_eq!(resp["type"], "control_response");
    assert_eq!(resp["response"]["subtype"], "error");
    assert_eq!(resp["response"]["request_id"], "req-unknown");
    assert!(
        resp["response"]["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported control request: oauth_token_refresh")
    );
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
async fn test_reader_replies_to_unknown_control_request() {
    let marker = std::env::temp_dir().join(format!(
        "claude-sdk-client-unknown-control-marker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let script = write_mock_claude_script(
        "unknown-control",
        &format!(
            "#!/bin/sh\n\
IFS= read -r line || exit 1\n\
request_id=$(printf '%s\\n' \"$line\" | sed -E 's/.*\"request_id\":\"([^\"]+)\".*/\\1/')\n\
printf '%s\\n' \"{{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{{\\\"subtype\\\":\\\"success\\\",\\\"request_id\\\":\\\"$request_id\\\",\\\"response\\\":{{}}}}}}\"\n\
printf '%s\\n' '{{\"type\":\"control_request\",\"request_id\":\"req_unknown\",\"request\":{{\"subtype\":\"oauth_token_refresh\"}}}}'\n\
IFS= read -r response || exit 2\n\
printf '%s\\n' \"$response\" > '{}'\n",
            marker.to_string_lossy()
        ),
    );
    let options = ClaudeAgentOptions {
        cli_path: Some(script.clone()),
        ..ClaudeAgentOptions::default()
    };

    let mut client = ClaudeSDKClient::new(options);
    client
        .connect(None)
        .await
        .expect("streaming connect should succeed");

    for _ in 0..50 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert!(
        marker.exists(),
        "reader should answer unknown control_request instead of dropping it"
    );
    let response: Value =
        serde_json::from_str(fs::read_to_string(&marker).unwrap().trim()).unwrap();
    assert_eq!(response["response"]["subtype"], "error");
    assert_eq!(response["response"]["request_id"], "req_unknown");
    assert!(
        response["response"]["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported control request: oauth_token_refresh")
    );

    client.finish().await.expect("finish should succeed");
    let _ = fs::remove_file(script);
    let _ = fs::remove_file(marker);
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

#[tokio::test]
async fn test_finish_closes_stdin_and_waits_for_process_exit() {
    let marker = std::env::temp_dir().join(format!(
        "claude-sdk-client-finish-marker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let script = write_mock_claude_script(
        "finish",
        &format!(
            "#!/bin/sh\n\
IFS= read -r line || exit 1\n\
request_id=$(printf '%s\\n' \"$line\" | sed -E 's/.*\"request_id\":\"([^\"]+)\".*/\\1/')\n\
printf '%s\\n' \"{{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{{\\\"subtype\\\":\\\"success\\\",\\\"request_id\\\":\\\"$request_id\\\",\\\"response\\\":{{}}}}}}\"\n\
while IFS= read -r line; do\n\
  :\n\
done\n\
printf done > '{}'\n",
            marker.to_string_lossy()
        ),
    );
    let options = ClaudeAgentOptions {
        cli_path: Some(script.clone()),
        ..ClaudeAgentOptions::default()
    };

    let mut client = ClaudeSDKClient::new(options);
    client
        .connect(None)
        .await
        .expect("streaming connect should succeed");

    client.finish().await.expect("finish should succeed");

    assert!(
        marker.exists(),
        "finish should close stdin and let the process exit naturally"
    );
    assert!(client.transport.is_none());
    assert!(client.reader_handle.is_none());
    assert!(client.stream_handle.is_none());
    assert!(client.pending.lock().await.is_empty());
    assert!(client.msg_tx.is_some());
    assert!(client.msg_rx.is_some());
    assert!(!client.closed.load(Ordering::SeqCst));

    let _ = fs::remove_file(script);
    let _ = fs::remove_file(marker);
}
