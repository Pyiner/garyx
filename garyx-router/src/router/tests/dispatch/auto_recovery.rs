use super::*;

#[tokio::test]
async fn test_auto_recovery_no_redirect() {
    let store = Arc::new(InMemoryThreadStore::new());
    store.set("s1", json!({"messages": []})).await;

    let router = MessageRouter::new(store, GaryxConfig::default());
    assert!(router.check_auto_recovery("s1").await.is_none());
}

#[tokio::test]
async fn test_auto_recovery_with_redirect() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "s_broken",
            json!({
                "auto_recover_next_thread": "s_new"
            }),
        )
        .await;
    store.set("s_new", json!({"messages": []})).await;

    let router = MessageRouter::new(store, GaryxConfig::default());
    assert_eq!(
        router.check_auto_recovery("s_broken").await,
        Some("s_new".to_owned())
    );
}

#[tokio::test]
async fn test_auto_recovery_empty_redirect() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "s1",
            json!({
                "auto_recover_next_thread": ""
            }),
        )
        .await;

    let router = MessageRouter::new(store, GaryxConfig::default());
    assert!(router.check_auto_recovery("s1").await.is_none());
}

#[tokio::test]
async fn test_auto_recovery_nonexistent_session() {
    let router = make_router();
    assert!(router.check_auto_recovery("nonexistent").await.is_none());
}

#[tokio::test]
async fn test_auto_recovery_reads_legacy_session_key() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "s_legacy",
            json!({
                "auto_recover_next_session": "s_new"
            }),
        )
        .await;
    store.set("s_new", json!({"messages": []})).await;

    let router = MessageRouter::new(store, GaryxConfig::default());
    assert_eq!(
        router.check_auto_recovery("s_legacy").await,
        Some("s_new".to_owned())
    );
}
