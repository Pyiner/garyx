use super::*;

#[test]
fn test_default_config() {
    let cfg = CodexClientConfig::default();
    assert_eq!(cfg.codex_bin, "codex");
    assert!(cfg.workspace_dir.is_none());
    assert!(cfg.model.is_none());
    assert_eq!(cfg.approval_policy, "never");
    assert_eq!(cfg.sandbox_mode, "off");
    assert!(!cfg.experimental_api);
    assert_eq!(cfg.request_timeout, Duration::from_secs(300));
    assert_eq!(cfg.startup_timeout, Duration::from_secs(300));
    assert_eq!(cfg.max_overload_retries, 4);
}

#[test]
fn test_client_not_ready_before_init() {
    let client = CodexClient::new(CodexClientConfig::default());
    assert!(!client.is_ready());
}

#[tokio::test]
async fn test_start_thread_before_init() {
    let client = CodexClient::new(CodexClientConfig::default());
    let err = client
        .start_thread(ThreadStartParams::default())
        .await
        .unwrap_err();
    assert!(matches!(err, CodexError::NotInitialized));
}

#[tokio::test]
async fn test_start_turn_before_init() {
    let client = CodexClient::new(CodexClientConfig::default());
    let err = client
        .start_turn(
            "th_1",
            vec![InputItem::Text {
                text: "hi".to_owned(),
            }],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CodexError::NotInitialized));
}

#[tokio::test]
async fn test_steer_turn_before_init() {
    let client = CodexClient::new(CodexClientConfig::default());
    let err = client
        .steer_turn(
            "th_1",
            "turn_1",
            vec![InputItem::Text {
                text: "more".to_owned(),
            }],
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CodexError::NotInitialized));
}

#[tokio::test]
async fn test_interrupt_turn_before_init() {
    let client = CodexClient::new(CodexClientConfig::default());
    let err = client.interrupt_turn("th_1", "turn_1").await.unwrap_err();
    assert!(matches!(err, CodexError::NotInitialized));
}

#[tokio::test]
async fn test_shutdown_before_init() {
    let mut client = CodexClient::new(CodexClientConfig::default());
    client.shutdown().await;
    assert!(!client.is_ready());
}

#[tokio::test]
async fn test_double_initialize_errors() {
    let mut client = CodexClient::new(CodexClientConfig::default());
    // Manually mark as initialized
    client.initialized = true;
    let err = client.initialize().await.unwrap_err();
    assert!(matches!(err, CodexError::Fatal(_)));
}
