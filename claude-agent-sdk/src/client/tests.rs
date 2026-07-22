use super::{
    ClaudeSDKClient, Prompt, build_user_message_payload, incoming_control_response,
    resume_store_error, unsupported_incoming_control_request_response,
};
use crate::control::IncomingControlRequest;
use crate::error::ClaudeSDKError;
use crate::types::{ClaudeAgentOptions, Message};
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
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
fn test_resume_store_io_error_keeps_storage_identity() {
    let error = resume_store_error(ClaudeSDKError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "probe",
    )));
    assert!(matches!(error, ClaudeSDKError::SessionStore(message) if message.contains("probe")));
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
async fn test_incoming_elicitation_declines_by_default() {
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

    let resp = serde_json::to_value(incoming_control_response(req, None).await).unwrap();
    assert_eq!(resp["type"], "control_response");
    assert_eq!(resp["response"]["subtype"], "success");
    assert_eq!(resp["response"]["request_id"], "req-elicit");
    assert_eq!(resp["response"]["response"]["action"], "decline");
}

#[tokio::test]
async fn test_incoming_can_use_tool_uses_callback_response() {
    let req: IncomingControlRequest = serde_json::from_value(serde_json::json!({
        "type": "control_request",
        "request_id": "req-permission",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "Bash",
            "input": { "command": "rm -rf synthetic-target" },
            "tool_use_id": "toolu_synthetic"
        }
    }))
    .unwrap();
    let callback = Arc::new(|request: crate::CanUseToolRequest| {
        Box::pin(async move {
            Ok(serde_json::json!({
                "behavior": "deny",
                "message": format!("blocked {}", request.tool_name)
            }))
        }) as crate::types::CanUseToolFuture
    });

    let resp = serde_json::to_value(incoming_control_response(req, Some(callback)).await).unwrap();

    assert_eq!(resp["response"]["subtype"], "success");
    assert_eq!(resp["response"]["request_id"], "req-permission");
    assert_eq!(resp["response"]["response"]["behavior"], "deny");
    assert_eq!(resp["response"]["response"]["message"], "blocked Bash");
    assert_eq!(resp["response"]["response"]["toolUseID"], "toolu_synthetic");
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
async fn test_stop_hook_observer_declares_hook_and_forwards_observation() {
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let init_marker = std::env::temp_dir().join(format!("claude-sdk-stop-hook-init-{nonce}"));
    let resp_marker = std::env::temp_dir().join(format!("claude-sdk-stop-hook-resp-{nonce}"));
    let script = write_mock_claude_script(
        "stop-hook-observer",
        &format!(
            "#!/bin/sh\n\
IFS= read -r line || exit 1\n\
printf '%s\\n' \"$line\" > '{init}'\n\
request_id=$(printf '%s\\n' \"$line\" | sed -E 's/.*\"request_id\":\"([^\"]+)\".*/\\1/')\n\
printf '%s\\n' \"{{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{{\\\"subtype\\\":\\\"success\\\",\\\"request_id\\\":\\\"$request_id\\\",\\\"response\\\":{{}}}}}}\"\n\
printf '%s\\n' '{{\"type\":\"control_request\",\"request_id\":\"req_hook_1\",\"request\":{{\"subtype\":\"hook_callback\",\"callback_id\":\"garyx_stop_hook_observer\",\"input\":{{\"hook_event_name\":\"Stop\",\"background_tasks\":[{{\"id\":\"bg-1\",\"type\":\"shell\",\"status\":\"running\"}}]}}}}}}'\n\
IFS= read -r response || exit 2\n\
printf '%s\\n' \"$response\" > '{resp}'\n",
            init = init_marker.to_string_lossy(),
            resp = resp_marker.to_string_lossy()
        ),
    );
    let options = ClaudeAgentOptions {
        cli_path: Some(script.clone()),
        stop_hook_observer: true,
        ..ClaudeAgentOptions::default()
    };

    let mut client = ClaudeSDKClient::new(options);
    client
        .connect(None)
        .await
        .expect("streaming connect should succeed");

    // The initialize request must declare the Stop hook observer callback.
    let init_line = fs::read_to_string(&init_marker).expect("initialize line should be captured");
    let init_json: Value = serde_json::from_str(init_line.trim()).unwrap();
    assert_eq!(
        init_json["request"]["hooks"]["Stop"][0]["hookCallbackIds"][0],
        "garyx_stop_hook_observer"
    );

    // The observation must be forwarded in-band as a synthetic system message.
    let mut rx = client
        .take_message_receiver()
        .expect("message receiver should be available");
    let observation = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Some(Ok(Message::System(system)))
                    if system.subtype == super::STOP_HOOK_OBSERVATION_SUBTYPE =>
                {
                    break system;
                }
                Some(_) => continue,
                None => panic!("message stream ended before stop-hook observation"),
            }
        }
    })
    .await
    .expect("stop-hook observation should arrive");
    assert_eq!(
        observation.data["input"]["background_tasks"][0]["id"],
        "bg-1"
    );

    // The reader must answer with an empty hook output (never blocking stop).
    for _ in 0..50 {
        if resp_marker.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let response: Value =
        serde_json::from_str(fs::read_to_string(&resp_marker).unwrap().trim()).unwrap();
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["request_id"], "req_hook_1");
    assert_eq!(response["response"]["response"], serde_json::json!({}));

    client.finish().await.expect("finish should succeed");
    let _ = fs::remove_file(script);
    let _ = fs::remove_file(init_marker);
    let _ = fs::remove_file(resp_marker);
}

/// The Stop-hook ACK must reach the CLI even when the SDK message channel is
/// saturated and the consumer is not draining: the CLI awaits the hook
/// response to finish its stop processing, so it must never queue behind the
/// observation forward. The observation still stays ordered ahead of the
/// turn's result once the consumer drains.
#[tokio::test]
async fn test_stop_hook_ack_is_not_blocked_by_consumer_backpressure() {
    let ack_marker = std::env::temp_dir().join(format!(
        "claude-sdk-stop-hook-backpressure-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    // 256 filler messages fill the channel completely (capacity 256) while
    // the test deliberately does not consume; the observation forward then
    // blocks, and only an ACK written before it can reach the script.
    let script = write_mock_claude_script(
        "stop-hook-backpressure",
        &format!(
            "#!/bin/sh\n\
IFS= read -r line || exit 1\n\
request_id=$(printf '%s\\n' \"$line\" | sed -E 's/.*\"request_id\":\"([^\"]+)\".*/\\1/')\n\
printf '%s\\n' \"{{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{{\\\"subtype\\\":\\\"success\\\",\\\"request_id\\\":\\\"$request_id\\\",\\\"response\\\":{{}}}}}}\"\n\
i=0\n\
while [ $i -lt 256 ]; do\n\
  printf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"filler\"}}'\n\
  i=$((i+1))\n\
done\n\
printf '%s\\n' '{{\"type\":\"control_request\",\"request_id\":\"req_hook_bp\",\"request\":{{\"subtype\":\"hook_callback\",\"callback_id\":\"garyx_stop_hook_observer\",\"input\":{{\"hook_event_name\":\"Stop\",\"background_tasks\":[{{\"id\":\"bg-bp\",\"status\":\"running\"}}]}}}}}}'\n\
IFS= read -r response || exit 2\n\
printf '%s\\n' \"$response\" > '{ack}'\n\
printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"session_id\":\"session-bp\"}}'\n",
            ack = ack_marker.to_string_lossy()
        ),
    );
    let options = ClaudeAgentOptions {
        cli_path: Some(script.clone()),
        stop_hook_observer: true,
        ..ClaudeAgentOptions::default()
    };

    let mut client = ClaudeSDKClient::new(options);
    client
        .connect(None)
        .await
        .expect("streaming connect should succeed");

    // The ACK must appear while the channel is full and nothing consumes.
    let mut ack_seen = false;
    for _ in 0..250 {
        if ack_marker.exists() {
            ack_seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        ack_seen,
        "stop-hook ACK must not wait for the consumer to drain the channel"
    );
    let response: Value =
        serde_json::from_str(fs::read_to_string(&ack_marker).unwrap().trim()).unwrap();
    assert_eq!(response["response"]["subtype"], "success");
    assert_eq!(response["response"]["request_id"], "req_hook_bp");

    // Draining now must yield the observation strictly before the result.
    let mut rx = client
        .take_message_receiver()
        .expect("message receiver should be available");
    let order = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut order = Vec::new();
        while let Some(message) = rx.recv().await {
            match message {
                Ok(Message::System(system))
                    if system.subtype == super::STOP_HOOK_OBSERVATION_SUBTYPE =>
                {
                    order.push("observation");
                }
                Ok(Message::Result(_)) => {
                    order.push("result");
                    break;
                }
                _ => {}
            }
        }
        order
    })
    .await
    .expect("stream should drain");
    assert_eq!(
        order,
        vec!["observation", "result"],
        "observation must stay ordered ahead of the result"
    );

    client.finish().await.expect("finish should succeed");
    let _ = fs::remove_file(script);
    let _ = fs::remove_file(ack_marker);
}

#[tokio::test]
async fn test_connect_without_stop_hook_observer_sends_null_hooks() {
    let init_marker = std::env::temp_dir().join(format!(
        "claude-sdk-no-hooks-init-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let script = write_mock_claude_script(
        "no-hooks-init",
        &format!(
            "#!/bin/sh\n\
IFS= read -r line || exit 1\n\
printf '%s\\n' \"$line\" > '{init}'\n\
request_id=$(printf '%s\\n' \"$line\" | sed -E 's/.*\"request_id\":\"([^\"]+)\".*/\\1/')\n\
printf '%s\\n' \"{{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{{\\\"subtype\\\":\\\"success\\\",\\\"request_id\\\":\\\"$request_id\\\",\\\"response\\\":{{}}}}}}\"\n",
            init = init_marker.to_string_lossy()
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

    let init_line = fs::read_to_string(&init_marker).expect("initialize line should be captured");
    let init_json: Value = serde_json::from_str(init_line.trim()).unwrap();
    assert!(
        init_json["request"]["hooks"].is_null(),
        "hooks must stay null when the observer is disabled: {init_json}"
    );

    client.finish().await.expect("finish should succeed");
    let _ = fs::remove_file(script);
    let _ = fs::remove_file(init_marker);
}

#[tokio::test]
async fn test_connect_rejects_can_use_tool_with_explicit_permission_prompt_tool() {
    let script = write_mock_claude_script("permission-conflict", "#!/bin/sh\nexit 0\n");
    let options = ClaudeAgentOptions {
        cli_path: Some(script.clone()),
        can_use_tool: Some(Arc::new(|_| {
            Box::pin(async { Ok(serde_json::json!({ "behavior": "deny" })) })
        })),
        permission_prompt_tool_name: Some("stdio".to_owned()),
        ..ClaudeAgentOptions::default()
    };

    let mut client = ClaudeSDKClient::new(options);
    let err = client.connect(None).await.unwrap_err();

    assert!(
        err.to_string()
            .contains("can_use_tool callback cannot be used")
    );
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

#[tokio::test]
async fn test_end_input_keeps_reading_internally_queued_followup_turn() {
    let script = write_mock_claude_script(
        "end-input-followup",
        "#!/bin/sh\n\
IFS= read -r line || exit 1\n\
request_id=$(printf '%s\\n' \"$line\" | sed -E 's/.*\"request_id\":\"([^\"]+)\".*/\\1/')\n\
printf '%s\\n' \"{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{\\\"subtype\\\":\\\"success\\\",\\\"request_id\\\":\\\"$request_id\\\",\\\"response\\\":{}}}\"\n\
IFS= read -r line || exit 2\n\
printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"session_id\":\"session-1\"}'\n\
while IFS= read -r line; do :; done\n\
printf '%s\\n' '{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"completed\"},\"origin\":{\"kind\":\"task-notification\"}}'\n\
printf '%s\\n' '{\"type\":\"assistant\",\"message\":{\"model\":\"claude-test\",\"content\":[{\"type\":\"text\",\"text\":\"follow-up\"}]}}'\n\
printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"session_id\":\"session-2\",\"origin\":{\"kind\":\"task-notification\"}}'\n",
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
    let mut messages = client
        .take_message_receiver()
        .expect("message receiver should exist");
    client
        .send_user_content(Value::String("start".to_owned()), None, None)
        .await
        .expect("initial user input should send");

    let first = messages.recv().await.expect("first result should arrive");
    assert!(matches!(first, Ok(Message::Result(_))));
    client
        .end_input()
        .await
        .expect("stdin should close cleanly");

    let mut saw_followup_user = false;
    let mut final_session_id = None;
    while let Some(message) = messages.recv().await {
        match message.expect("follow-up frame should parse") {
            Message::User(user) => {
                saw_followup_user = user
                    .origin
                    .as_ref()
                    .is_some_and(|origin| origin.is_task_notification());
            }
            Message::Result(result) => final_session_id = Some(result.session_id.clone()),
            _ => {}
        }
    }

    assert!(saw_followup_user);
    assert_eq!(final_session_id.as_deref(), Some("session-2"));
    client
        .finish()
        .await
        .expect("natural finish should succeed");
    let _ = fs::remove_file(script);
}

#[tokio::test]
async fn test_finish_does_not_terminate_normal_process_after_two_seconds() {
    let script = write_mock_claude_script(
        "finish-ignores-eof",
        "#!/bin/sh\n\
IFS= read -r line || exit 1\n\
request_id=$(printf '%s\\n' \"$line\" | sed -E 's/.*\"request_id\":\"([^\"]+)\".*/\\1/')\n\
printf '%s\\n' \"{\\\"type\\\":\\\"control_response\\\",\\\"response\\\":{\\\"subtype\\\":\\\"success\\\",\\\"request_id\\\":\\\"$request_id\\\",\\\"response\\\":{}}}\"\n\
exec sleep 600\n",
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

    tokio::time::pause();
    let finish_task = tokio::spawn(async move { client.finish().await });
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    tokio::task::yield_now().await;

    assert!(
        !finish_task.is_finished(),
        "normal finish must wait for natural exit instead of killing at two seconds"
    );
    finish_task.abort();
    let _ = finish_task.await;

    let _ = fs::remove_file(script);
}
