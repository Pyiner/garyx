use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use garyx_bridge::claude_tty_provider::ClaudeTtyProvider;
use garyx_bridge::provider_trait::AgentLoopProvider;
use garyx_models::provider::{ClaudeCodeConfig, ProviderRunOptions, StreamEvent};
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
