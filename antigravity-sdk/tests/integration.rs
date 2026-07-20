//! Live Antigravity CLI smoke tests.
//!
//! Run with explicit local protocol inputs:
//!
//! ```text
//! ANTIGRAVITY_BRAIN_ROOT=... ANTIGRAVITY_MODEL=... \
//!   cargo test -p antigravity-sdk --test integration -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use antigravity_sdk::{
    AntigravityClient, AntigravityClientConfig, AntigravityEvent, AntigravityRunRequest,
    ApprovalCallback, ApprovalDecision, ApprovalFuture,
};

fn bypass_for_smoke_test() -> ApprovalCallback {
    Arc::new(|_| Box::pin(async { Ok(ApprovalDecision::BypassPermissions) }) as ApprovalFuture)
}

#[tokio::test]
#[ignore = "requires live agy CLI, authentication, brain root, and model"]
async fn live_one_turn_tails_transcript() {
    let brain_root = PathBuf::from(
        std::env::var("ANTIGRAVITY_BRAIN_ROOT")
            .expect("set ANTIGRAVITY_BRAIN_ROOT for the live smoke"),
    );
    let model =
        std::env::var("ANTIGRAVITY_MODEL").expect("set ANTIGRAVITY_MODEL for the live smoke");
    let cli_bin = std::env::var("ANTIGRAVITY_CLI").unwrap_or_else(|_| "agy".to_owned());
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let client = AntigravityClient::new(AntigravityClientConfig::new(cli_bin, brain_root));
    client.probe().await.expect("probe live CLI");

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_callback = Arc::clone(&events);
    let callback = move |event| events_for_callback.lock().unwrap().push(event);
    let outcome = client
        .execute(
            AntigravityRunRequest {
                run_id: "synthetic-live-smoke".to_owned(),
                prompt: "Reply with exactly ANTIGRAVITY_SDK_SMOKE_OK and do not use tools."
                    .to_owned(),
                discovery_text: "ANTIGRAVITY_SDK_SMOKE_OK".to_owned(),
                model,
                conversation_id: None,
                workspace_dir: workspace,
                log_path: temp.path().join("antigravity-live-smoke.log"),
                env: HashMap::new(),
                print_timeout: Duration::from_secs(120),
                approval_callback: bypass_for_smoke_test(),
            },
            &callback,
        )
        .await
        .expect("live run");

    assert!(outcome.success, "live outcome failed: {outcome:?}");
    let text = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AntigravityEvent::AssistantDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        text.contains("ANTIGRAVITY_SDK_SMOKE_OK"),
        "unexpected live response: {text}"
    );
}
