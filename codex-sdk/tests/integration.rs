//! Integration tests for the Codex SDK.
//!
//! These tests spawn a real `codex app-server` process and verify end-to-end
//! JSON-RPC communication.
//!
//! Run with: `cargo test -p codex-sdk --test integration -- --ignored`
//!
//! Requirements:
//! - `codex` CLI installed and in PATH
//! - Valid authentication configured (run `codex login` first)

use std::time::Duration;

use codex_sdk::{CodexClient, CodexClientConfig, CodexError, InputItem};

/// Helper: check if `codex` binary is available.
async fn codex_available() -> bool {
    tokio::process::Command::new("which")
        .arg("codex")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Client lifecycle tests
// ---------------------------------------------------------------------------

/// Initialize the client, verify handshake, then shutdown.
#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn test_initialize_and_shutdown() {
    if !codex_available().await {
        eprintln!("codex not found, skipping");
        return;
    }

    let config = CodexClientConfig {
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        request_timeout: Duration::from_secs(60),
        startup_timeout: Duration::from_secs(60),
        ..CodexClientConfig::default()
    };

    let mut client = CodexClient::new(config);

    let init_result = client.initialize().await;
    assert!(
        init_result.is_ok(),
        "Initialize failed: {:?}",
        init_result.err()
    );
    assert!(client.is_ready());

    client.shutdown().await;
    assert!(!client.is_ready());
}

/// Start a thread, send a simple turn, and verify we get a response.
#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn test_simple_turn() {
    if !codex_available().await {
        eprintln!("codex not found, skipping");
        return;
    }

    let config = CodexClientConfig {
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        request_timeout: Duration::from_secs(120),
        startup_timeout: Duration::from_secs(60),
        ..CodexClientConfig::default()
    };

    let mut client = CodexClient::new(config);
    client.initialize().await.expect("initialize failed");

    // Subscribe to events before starting the thread
    let mut event_rx = client.subscribe_events();

    // Start a thread
    let thread_id = client
        .start_thread(codex_sdk::ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            config: None,
            model: Some("gpt-5.4".to_owned()),
            model_reasoning_effort: Some("xhigh".to_owned()),
            ..Default::default()
        })
        .await
        .expect("thread/start failed");

    assert!(!thread_id.is_empty(), "thread_id should not be empty");

    // Start a turn
    let turn_id = client
        .start_turn(
            &thread_id,
            vec![InputItem::Text {
                text: "What is 2 + 2? Reply with just the number.".to_owned(),
            }],
        )
        .await
        .expect("turn/start failed");

    assert!(!turn_id.is_empty(), "turn_id should not be empty");

    // Collect response from notifications
    let mut response = String::new();
    let mut turn_completed = false;

    let timeout = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match event_rx.recv().await {
                Ok(notif) => {
                    match notif.method.as_str() {
                        "item/agentMessage/delta" => {
                            if let Some(delta) = notif.params.get("delta").and_then(|v| v.as_str())
                            {
                                response.push_str(delta);
                            }
                        }
                        "turn/completed" => {
                            turn_completed = true;
                            break;
                        }
                        "transport/fatal" => {
                            panic!("Fatal transport error: {:?}", notif.params.get("error"));
                        }
                        _ => {} // item/started, item/completed, etc.
                    }
                }
                Err(e) => {
                    panic!("Event channel error: {e}");
                }
            }
        }
    })
    .await;

    assert!(timeout.is_ok(), "Timed out waiting for turn/completed");
    assert!(turn_completed, "Never received turn/completed");
    assert!(
        response.contains('4'),
        "Expected '4' in response, got: {response}"
    );

    client.shutdown().await;
}

/// Start a thread, send a turn, then interrupt it.
#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn test_interrupt_turn() {
    if !codex_available().await {
        eprintln!("codex not found, skipping");
        return;
    }

    let config = CodexClientConfig {
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        request_timeout: Duration::from_secs(120),
        startup_timeout: Duration::from_secs(60),
        ..CodexClientConfig::default()
    };

    let mut client = CodexClient::new(config);
    client.initialize().await.expect("initialize failed");

    let thread_id = client
        .start_thread(codex_sdk::ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            config: None,
            model: Some("gpt-5.4".to_owned()),
            model_reasoning_effort: Some("xhigh".to_owned()),
            ..Default::default()
        })
        .await
        .expect("thread/start failed");

    let turn_id = client
        .start_turn(
            &thread_id,
            vec![InputItem::Text {
                text: "Write a very long story about a dragon. Make it at least 5000 words."
                    .to_owned(),
            }],
        )
        .await
        .expect("turn/start failed");

    // Wait briefly then interrupt
    tokio::time::sleep(Duration::from_secs(3)).await;

    let result = client.interrupt_turn(&thread_id, &turn_id).await;
    assert!(result.is_ok(), "interrupt_turn failed: {:?}", result.err());

    client.shutdown().await;
}

/// Resume a thread and verify continuity.
#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn test_resume_thread() {
    if !codex_available().await {
        eprintln!("codex not found, skipping");
        return;
    }

    let config = CodexClientConfig {
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        request_timeout: Duration::from_secs(120),
        startup_timeout: Duration::from_secs(60),
        ..CodexClientConfig::default()
    };

    let mut client = CodexClient::new(config);
    client.initialize().await.expect("initialize failed");

    let mut event_rx = client.subscribe_events();

    // Start first thread + turn
    let thread_id = client
        .start_thread(codex_sdk::ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            config: None,
            model: Some("gpt-5.4".to_owned()),
            model_reasoning_effort: Some("xhigh".to_owned()),
            ..Default::default()
        })
        .await
        .expect("thread/start failed");

    let _turn_id = client
        .start_turn(
            &thread_id,
            vec![InputItem::Text {
                text: "Remember: the secret word is 'pineapple'. Say OK.".to_owned(),
            }],
        )
        .await
        .expect("turn/start failed");

    // Wait for turn to complete
    let timeout = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Ok(notif) = event_rx.recv().await {
                if notif.method == "turn/completed" {
                    break;
                }
            }
        }
    })
    .await;
    assert!(timeout.is_ok(), "First turn timed out");

    // Resume the thread with a follow-up
    let resumed_thread_id = client
        .resume_thread(codex_sdk::ThreadResumeParams {
            thread_id: thread_id.clone(),
            cwd: Some("/tmp".to_owned()),
            config: None,
            model: None,
            model_reasoning_effort: None,
            approval_policy: None,
            sandbox: None,
        })
        .await
        .expect("thread/resume failed");

    assert!(!resumed_thread_id.is_empty());

    let _turn_id2 = client
        .start_turn(
            &resumed_thread_id,
            vec![InputItem::Text {
                text: "What was the secret word I told you? Reply with just the word.".to_owned(),
            }],
        )
        .await
        .expect("second turn/start failed");

    // Collect response
    let mut response = String::new();
    let timeout = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Ok(notif) = event_rx.recv().await {
                match notif.method.as_str() {
                    "item/agentMessage/delta" => {
                        if let Some(delta) = notif.params.get("delta").and_then(|v| v.as_str()) {
                            response.push_str(delta);
                        }
                    }
                    "turn/completed" => break,
                    _ => {}
                }
            }
        }
    })
    .await;

    assert!(timeout.is_ok(), "Second turn timed out");
    assert!(
        response.to_lowercase().contains("pineapple"),
        "Expected 'pineapple' in resumed response, got: {response}"
    );

    client.shutdown().await;
}

/// Start a long-running turn, steer it mid-flight, and verify the follow-up
/// instruction lands on the same active turn.
#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn test_steer_turn() {
    if !codex_available().await {
        eprintln!("codex not found, skipping");
        return;
    }

    let config = CodexClientConfig {
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        request_timeout: Duration::from_secs(120),
        startup_timeout: Duration::from_secs(60),
        ..CodexClientConfig::default()
    };

    let mut client = CodexClient::new(config);
    client.initialize().await.expect("initialize failed");

    let mut event_rx = client.subscribe_events();

    let thread_id = client
        .start_thread(codex_sdk::ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            config: None,
            model: Some("gpt-5.4".to_owned()),
            model_reasoning_effort: Some("xhigh".to_owned()),
            ..Default::default()
        })
        .await
        .expect("thread/start failed");

    let turn_id = client
        .start_turn(
            &thread_id,
            vec![InputItem::Text {
                text: "Run `sleep 5` in the shell before replying. Do not answer until the command completes. After it finishes, reply with exactly READY.".to_owned(),
            }],
        )
        .await
        .expect("turn/start failed");

    let mut response = String::new();
    let steer_input = vec![InputItem::Text {
        text: "Higher priority update: when you finally reply, the final answer must contain the exact token STEER_ACK_123.".to_owned(),
    }];
    let steer_deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut saw_turn_activity = false;
    loop {
        assert!(
            std::time::Instant::now() < steer_deadline,
            "Timed out waiting for an active steerable turn"
        );

        match tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await {
            Ok(Ok(notif)) => match notif.method.as_str() {
                "item/started" => {
                    if notif.params.get("turnId").and_then(|v| v.as_str()) == Some(turn_id.as_str())
                    {
                        saw_turn_activity = true;
                    }
                }
                "item/agentMessage/delta" => {
                    if notif.params.get("turnId").and_then(|v| v.as_str()) == Some(turn_id.as_str())
                    {
                        saw_turn_activity = true;
                        if let Some(delta) = notif.params.get("delta").and_then(|v| v.as_str()) {
                            response.push_str(delta);
                        }
                    }
                }
                "turn/completed" => {
                    let completed_turn_id = notif
                        .params
                        .get("turn")
                        .and_then(|turn| turn.get("id"))
                        .and_then(|v| v.as_str());
                    if completed_turn_id == Some(turn_id.as_str()) {
                        panic!("turn completed before it became steerable");
                    }
                }
                "transport/fatal" => {
                    panic!("Fatal transport error: {:?}", notif.params.get("error"));
                }
                _ => {}
            },
            Ok(Err(e)) => panic!("Event channel error: {e}"),
            Err(_) => {}
        }

        if !saw_turn_activity {
            continue;
        }

        match client
            .steer_turn(&thread_id, &turn_id, steer_input.clone())
            .await
        {
            Ok(()) => break,
            Err(CodexError::RpcError {
                code: -32600,
                message,
                ..
            }) if message.contains("no active turn to steer") => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(error) => panic!("turn/steer failed: {error:?}"),
        }
    }

    let timeout = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match event_rx.recv().await {
                Ok(notif) => match notif.method.as_str() {
                    "item/agentMessage/delta" => {
                        if let Some(delta) = notif.params.get("delta").and_then(|v| v.as_str()) {
                            response.push_str(delta);
                        }
                    }
                    "turn/completed" => break,
                    "transport/fatal" => {
                        panic!("Fatal transport error: {:?}", notif.params.get("error"));
                    }
                    _ => {}
                },
                Err(e) => panic!("Event channel error: {e}"),
            }
        }
    })
    .await;

    assert!(timeout.is_ok(), "Timed out waiting for steered turn");
    assert!(
        response.contains("STEER_ACK_123"),
        "Expected steer token in response, got: {response}"
    );

    client.shutdown().await;
}
