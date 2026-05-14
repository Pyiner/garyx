use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use garyx_bridge::claude_tty_provider::ClaudeTtyProvider;
use garyx_bridge::provider_trait::AgentLoopProvider;
use garyx_models::provider::{ClaudeCodeConfig, ProviderRunOptions, QueuedUserInput, StreamEvent};
use serde_json::json;

#[tokio::test]
#[ignore = "requires a logged-in local Claude CLI and makes a real Claude request"]
async fn live_claude_tty_smoke() {
    let temp = tempfile::tempdir().expect("temp workspace");
    let mut provider = ClaudeTtyProvider::new(ClaudeCodeConfig {
        mcp_base_url: String::new(),
        timeout_seconds: 120.0,
        ..Default::default()
    });
    provider.initialize().await.expect("claude tty initialize");

    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: format!("thread::claude-tty-live-{}", uuid::Uuid::new_v4()),
                message: "Reply with exactly this single line: GARYX_CLAUDE_TTY_SMOKE_OK"
                    .to_owned(),
                workspace_dir: Some(temp.path().to_string_lossy().into_owned()),
                images: None,
                metadata: HashMap::from([("client_run_id".to_owned(), json!("live-tty-smoke"))]),
            },
            Box::new(move |event| {
                captured.lock().expect("events lock").push(event);
            }),
        )
        .await
        .expect("claude tty run");

    assert!(
        result.success,
        "provider result should be successful: {result:?}"
    );
    assert!(
        result.response.contains("GARYX_CLAUDE_TTY_SMOKE_OK"),
        "unexpected response: {:?}",
        result.response
    );
    let events = events.lock().expect("events lock");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::SessionBound { .. })),
        "missing session bound event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::Delta { .. })),
        "missing delta event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::Done)),
        "missing done event: {events:?}"
    );
}

#[tokio::test]
#[ignore = "requires a logged-in local Claude CLI and makes a real Claude request"]
async fn live_claude_tty_streaming_follow_up() {
    let temp = tempfile::tempdir().expect("temp workspace");
    let thread_id = format!("thread::claude-tty-follow-up-{}", uuid::Uuid::new_v4());
    let provider = new_live_tty_provider().await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let options = ProviderRunOptions {
        thread_id: thread_id.clone(),
        message: "Start by replying READY on its own line. Then write a numbered list from 1 to 400, one item per line. If a follow-up message arrives, stop the list and reply with exactly FOLLOW_UP_OK.".to_owned(),
        workspace_dir: Some(temp.path().to_string_lossy().into_owned()),
        images: None,
        metadata: HashMap::from([("client_run_id".to_owned(), json!("live-tty-follow-up"))]),
    };

    let run_future = provider.run_streaming(
        &options,
        Box::new(move |event| {
            captured.lock().expect("events lock").push(event);
        }),
    );
    tokio::pin!(run_future);

    let result = tokio::time::timeout(Duration::from_secs(180), async {
        let mut accepted = false;
        let mut attempts = 0usize;
        while !accepted {
            tokio::select! {
                result = &mut run_future => {
                    panic!("claude tty run finished before follow-up was accepted: {result:?}");
                }
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    attempts += 1;
                    accepted = provider
                        .add_streaming_input(
                            &thread_id,
                            QueuedUserInput::text(
                                "Stop the numbered list now and reply with exactly FOLLOW_UP_OK",
                            )
                            .with_pending_input_id("tty-follow-up-1"),
                        )
                        .await;
                    if attempts >= 120 {
                        panic!("add_streaming_input was not accepted within 30s");
                    }
                }
            }
        }

        run_future.await
    })
    .await
    .expect("claude tty follow-up run timed out")
    .expect("claude tty follow-up run failed");

    assert!(
        result.success,
        "provider result should be successful: {result:?}"
    );
    assert!(
        result.response.contains("FOLLOW_UP_OK"),
        "unexpected response: {:?}",
        result.response
    );
    let events = events.lock().expect("events lock");
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                StreamEvent::Boundary {
                    pending_input_id: Some(id),
                    ..
                } if id == "tty-follow-up-1"
            )
        }),
        "missing follow-up ack event: {events:?}"
    );
}

#[tokio::test]
#[ignore = "requires a logged-in local Claude CLI and makes real Claude requests"]
async fn live_claude_tty_resume_after_provider_restart() {
    let temp = tempfile::tempdir().expect("temp workspace");
    let thread_id = format!("thread::claude-tty-resume-{}", uuid::Uuid::new_v4());

    let mut first_provider = new_live_tty_provider().await;
    let first = first_provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: thread_id.clone(),
                message: "Remember this synthetic codeword for the next turn: RESUME_CODE_43. Reply with exactly FIRST_TURN_OK.".to_owned(),
                workspace_dir: Some(temp.path().to_string_lossy().into_owned()),
                images: None,
                metadata: HashMap::from([("client_run_id".to_owned(), json!("live-tty-resume-1"))]),
            },
            Box::new(|_| {}),
        )
        .await
        .expect("first claude tty run");
    assert!(
        first.success && first.response.contains("FIRST_TURN_OK"),
        "unexpected first result: {first:?}"
    );
    let sdk_session_id = first.sdk_session_id.expect("first sdk_session_id");
    first_provider.shutdown().await.expect("first shutdown");

    let second_provider = new_live_tty_provider().await;
    let second = second_provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id,
                message: "Using the existing conversation only, reply with exactly RESUME_OK:RESUME_CODE_43.".to_owned(),
                workspace_dir: Some(temp.path().to_string_lossy().into_owned()),
                images: None,
                metadata: HashMap::from([
                    ("client_run_id".to_owned(), json!("live-tty-resume-2")),
                    ("sdk_session_id".to_owned(), json!(sdk_session_id)),
                ]),
            },
            Box::new(|_| {}),
        )
        .await
        .expect("second claude tty run");

    assert!(
        second.success,
        "second result should be successful: {second:?}"
    );
    assert!(
        second.response.contains("RESUME_OK:RESUME_CODE_43"),
        "resume did not preserve conversation context: {:?}",
        second.response
    );
}

#[tokio::test]
#[ignore = "requires a logged-in local Claude CLI and makes real Claude requests"]
async fn live_claude_tty_interrupt_then_resume() {
    let temp = tempfile::tempdir().expect("temp workspace");
    let thread_id = format!("thread::claude-tty-interrupt-{}", uuid::Uuid::new_v4());
    let mut provider = new_live_tty_provider().await;
    let sdk_session_id = provider
        .get_or_create_session(&thread_id)
        .await
        .expect("precreate session id");
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let options = ProviderRunOptions {
        thread_id: thread_id.clone(),
        message: "Remember this synthetic codeword: INTERRUPT_CODE_99. Then write a numbered list from 1 to 2000, one item per line.".to_owned(),
        workspace_dir: Some(temp.path().to_string_lossy().into_owned()),
        images: None,
        metadata: HashMap::from([("client_run_id".to_owned(), json!("live-tty-abort"))]),
    };

    let aborted_result = {
        let run_future = provider.run_streaming(
            &options,
            Box::new(move |event| {
                captured.lock().expect("events lock").push(event);
            }),
        );
        tokio::pin!(run_future);

        tokio::time::timeout(Duration::from_secs(90), async {
            let mut aborted = false;
            let mut attempts = 0usize;
            while !aborted {
                tokio::select! {
                    result = &mut run_future => {
                        panic!("claude tty run finished before abort was accepted: {result:?}");
                    }
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {
                        attempts += 1;
                        aborted = provider.abort("live-tty-abort").await;
                        if attempts >= 120 {
                            panic!("abort was not accepted within 30s");
                        }
                    }
                }
            }

            tokio::time::timeout(Duration::from_secs(30), run_future)
                .await
                .expect("run did not finish within 30s after abort")
        })
        .await
        .expect("interrupt scenario timed out")
    };

    match aborted_result {
        Ok(result) => assert!(
            !result.success || result.response.len() < 10_000,
            "abort should not complete the full long run: {result:?}"
        ),
        Err(error) => {
            let text = error.to_string();
            assert!(
                text.contains("interrupt")
                    || text.contains("aborted")
                    || text.contains("no result")
                    || text.contains("timeout")
                    || text.contains("claude"),
                "unexpected abort error: {text}"
            );
        }
    }
    provider.shutdown().await.expect("shutdown after abort");

    let resume_provider = new_live_tty_provider().await;
    let resumed = resume_provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id,
                message: "Continue this existing conversation and reply with exactly INTERRUPT_RESUME_OK.".to_owned(),
                workspace_dir: Some(temp.path().to_string_lossy().into_owned()),
                images: None,
                metadata: HashMap::from([
                    ("client_run_id".to_owned(), json!("live-tty-abort-resume")),
                    ("sdk_session_id".to_owned(), json!(sdk_session_id)),
                ]),
            },
            Box::new(|_| {}),
        )
        .await
        .expect("resume after interrupt");
    assert!(
        resumed.success && resumed.response.contains("INTERRUPT_RESUME_OK"),
        "resume after interrupt failed: {resumed:?}"
    );
}

async fn new_live_tty_provider() -> ClaudeTtyProvider {
    let mut provider = ClaudeTtyProvider::new(ClaudeCodeConfig {
        mcp_base_url: String::new(),
        timeout_seconds: 120.0,
        ..Default::default()
    });
    provider.initialize().await.expect("claude tty initialize");
    provider
}
