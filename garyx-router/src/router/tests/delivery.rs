use super::*;
use crate::memory_store::InMemoryThreadStore;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn test_rebuild_last_delivery_cache() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "bot1::main::u1",
            json!({
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "bot1",
                    "chat_id": "42",
                    "user_id": "u1",
                    "thread_id": null,
                    "metadata": {},
                }
            }),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let rebuilt = router.rebuild_last_delivery_cache().await;
    assert_eq!(rebuilt, 1);

    let latest = router.latest_delivery().unwrap();
    assert_eq!(latest.0, "bot1::main::u1");
    assert_eq!(latest.1.channel, "telegram");
    assert_eq!(latest.1.account_id, "bot1");
    assert_eq!(latest.1.chat_id, "42");
}

#[tokio::test]
async fn test_rebuild_last_delivery_cache_prefers_latest_timestamp() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "bot1::main::u1",
            json!({
                "lastChannel": "telegram",
                "lastTo": "101",
                "lastAccountId": "bot1",
                "lastUpdatedAt": "2026-03-01T10:00:00Z",
            }),
        )
        .await;
    store
        .set(
            "bot2::main::u2",
            json!({
                "lastChannel": "telegram",
                "lastTo": "202",
                "lastAccountId": "bot2",
                "lastUpdatedAt": "2026-03-01T11:00:00Z",
            }),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let rebuilt = router.rebuild_last_delivery_cache().await;
    assert_eq!(rebuilt, 2);

    let latest = router.latest_delivery().unwrap();
    assert_eq!(latest.0, "bot2::main::u2");
    assert_eq!(latest.1.account_id, "bot2");
    assert_eq!(latest.1.chat_id, "202");
}

#[tokio::test]
async fn test_resolve_delivery_target_with_rebuild() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "bot1::main::u1",
            json!({
                "lastChannel": "telegram",
                "lastTo": "42",
                "lastAccountId": "bot1",
                "lastUpdatedAt": "2026-03-01T12:00:00Z",
            }),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    assert!(
        router
            .resolve_delivery_target("thread:bot1::main::u1")
            .is_none()
    );

    let resolved = router
        .resolve_delivery_target_with_rebuild("thread:bot1::main::u1")
        .await
        .expect("expected delivery target to be rebuilt from store");
    assert_eq!(resolved.0, "bot1::main::u1");
    assert_eq!(resolved.1.channel, "telegram");
    assert_eq!(resolved.1.account_id, "bot1");
    assert_eq!(resolved.1.chat_id, "42");
}

#[tokio::test]
async fn test_resolve_delivery_target_with_rebuild_sanitizes_legacy_telegram_dm_thread_id() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::telegram-dm",
            json!({
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "bot1",
                    "chat_id": "42",
                    "user_id": "42",
                    "thread_id": "42",
                    "metadata": {}
                }
            }),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let resolved = router
        .resolve_delivery_target_with_rebuild("thread::telegram-dm")
        .await
        .expect("expected delivery target to be rebuilt from store");
    assert_eq!(resolved.0, "thread::telegram-dm");
    assert!(resolved.1.thread_id.is_none());
}

#[tokio::test]
async fn test_resolve_delivery_target_with_rebuild_drops_non_numeric_telegram_thread_id() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::telegram-dm",
            json!({
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "bot1",
                    "chat_id": "42",
                    "user_id": "42",
                    "thread_id": "thread::internal",
                    "metadata": {}
                }
            }),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let resolved = router
        .resolve_delivery_target_with_rebuild("thread::telegram-dm")
        .await
        .expect("expected delivery target to be rebuilt from store");
    assert_eq!(resolved.0, "thread::telegram-dm");
    assert!(resolved.1.thread_id.is_none());
}

#[tokio::test]
async fn test_resolve_delivery_target_with_rebuild_last_target() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "bot1::main::u1",
            json!({
                "lastChannel": "telegram",
                "lastTo": "42",
                "lastAccountId": "bot1",
                "lastUpdatedAt": "2026-03-01T10:00:00Z",
            }),
        )
        .await;
    store
        .set(
            "bot2::main::u2",
            json!({
                "lastChannel": "feishu",
                "lastTo": "ou_2",
                "lastAccountId": "app1",
                "lastUpdatedAt": "2026-03-01T11:00:00Z",
            }),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let resolved = router
        .resolve_delivery_target_with_rebuild("last")
        .await
        .expect("expected last target to resolve after rebuild");
    assert_eq!(resolved.0, "bot2::main::u2");
    assert_eq!(resolved.1.channel, "feishu");
    assert_eq!(resolved.1.account_id, "app1");
    assert_eq!(resolved.1.chat_id, "ou_2");
}

#[tokio::test]
async fn test_resolve_delivery_target_accepts_canonical_thread_id() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store, GaryxConfig::default());
    router.set_last_delivery(
        "thread::bound",
        DeliveryContext {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            user_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            thread_id: None,
            metadata: HashMap::new(),
        },
    );

    let direct = router
        .resolve_delivery_target("thread::bound")
        .expect("canonical thread target should resolve");
    assert_eq!(direct.0, "thread::bound");

    let prefixed = router
        .resolve_delivery_target("thread:thread::bound")
        .expect("prefixed canonical thread target should resolve");
    assert_eq!(prefixed.0, "thread::bound");
}

#[tokio::test]
async fn test_set_last_delivery_with_persistence_does_not_recreate_deleted_session() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());

    store
        .set(
            "bot1::main::u1",
            json!({
                "messages": []
            }),
        )
        .await;
    assert!(store.delete("bot1::main::u1").await);

    router
        .set_last_delivery_with_persistence(
            "bot1::main::u1",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "u1".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        )
        .await;

    assert!(store.get("bot1::main::u1").await.is_none());
    assert!(router.get_last_delivery("bot1::main::u1").is_some());
}

// ------------------------------------------------------------------
// Tests for new semantic parity features
// ------------------------------------------------------------------

#[test]
fn test_enrich_metadata() {
    let meta = MessageRouter::enrich_metadata(
        "telegram",
        "bot1",
        "user42",
        true,
        Some("thread_123"),
        "bot1::group::thread_123",
    );

    assert_eq!(meta.channel.as_deref(), Some("telegram"));
    assert_eq!(meta.account_id.as_deref(), Some("bot1"));
    assert_eq!(meta.from_id.as_deref(), Some("user42"));
    assert!(meta.is_group);
    assert_eq!(meta.thread_id.as_deref(), Some("thread_123"));
    assert_eq!(
        meta.resolved_thread_id.as_deref(),
        Some("bot1::group::thread_123")
    );
}

#[test]
fn test_enrich_metadata_dm() {
    let meta = MessageRouter::enrich_metadata(
        "feishu",
        "app1",
        "ou_user",
        false,
        None,
        "app1::main::ou_user",
    );

    assert_eq!(meta.channel.as_deref(), Some("feishu"));
    assert!(!meta.is_group);
    assert!(meta.thread_id.is_none());
}

#[test]
fn test_last_delivery_context() {
    let mut router = make_router();

    assert!(router.get_last_delivery("session_1").is_none());

    let ctx = DeliveryContext {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        chat_id: "user42".to_owned(),
        user_id: "user42".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "user42".to_owned(),
        thread_id: Some("thread_1".to_owned()),
        metadata: HashMap::new(),
    };
    router.set_last_delivery("session_1", ctx);

    let got = router.get_last_delivery("session_1").unwrap();
    assert_eq!(got.channel, "telegram");
    assert_eq!(got.account_id, "bot1");
    assert_eq!(got.chat_id, "user42");
    assert_eq!(got.thread_id.as_deref(), Some("thread_1"));
}

#[test]
fn test_last_delivery_overwrite() {
    let mut router = make_router();

    let ctx1 = DeliveryContext {
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        chat_id: "user1".to_owned(),
        ..Default::default()
    };
    router.set_last_delivery("s1", ctx1);

    let ctx2 = DeliveryContext {
        channel: "feishu".to_owned(),
        account_id: "app1".to_owned(),
        chat_id: "user2".to_owned(),
        ..Default::default()
    };
    router.set_last_delivery("s1", ctx2);

    let got = router.get_last_delivery("s1").unwrap();
    assert_eq!(got.channel, "feishu");
    assert_eq!(got.chat_id, "user2");
}

#[tokio::test]
async fn test_set_last_delivery_with_persistence_updates_binding_timestamp() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::delivery",
            json!({
                "thread_id": "thread::delivery",
                "thread_id": "thread::delivery",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "bot1",
                    "peer_id": "u1",
                    "chat_id": "42",
                    "display_label": "u1"
                }]
            }),
        )
        .await;

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router
        .set_last_delivery_with_persistence(
            "thread::delivery",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "u1".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        )
        .await;

    let stored = store.get("thread::delivery").await.unwrap();
    assert!(
        stored["channel_bindings"][0]["last_delivery_at"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
}

#[tokio::test]
async fn test_clear_last_delivery_with_persistence_clears_binding_timestamp() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::delivery",
            json!({
                "thread_id": "thread::delivery",
                "thread_id": "thread::delivery",
                "delivery_context": {
                    "channel": "telegram",
                    "account_id": "bot1",
                    "chat_id": "42",
                    "user_id": "u1",
                    "thread_id": null,
                    "metadata": {}
                },
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "bot1",
                    "peer_id": "u1",
                    "chat_id": "42",
                    "display_label": "u1",
                    "last_delivery_at": "2026-03-07T10:00:00Z"
                }]
            }),
        )
        .await;

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router.rebuild_last_delivery_cache().await;
    router
        .clear_last_delivery_with_persistence("thread::delivery")
        .await;

    let stored = store.get("thread::delivery").await.unwrap();
    assert!(stored.get("delivery_context").is_none());
    assert!(stored["channel_bindings"][0]["last_delivery_at"].is_null());
}
