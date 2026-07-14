use super::*;
use crate::memory_store::InMemoryThreadStore;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn test_resolve_delivery_target_from_store_last_prefers_latest_timestamp() {
    let delivery_record = |account_id: &str, chat_id: &str, updated_at: Option<&str>| {
        let mut record = json!({
            "delivery_context": {
                "channel": "telegram",
                "account_id": account_id,
                "chat_id": chat_id,
                "user_id": chat_id,
                "delivery_target_type": "chat_id",
                "delivery_target_id": chat_id,
                "thread_id": null,
                "metadata": {},
            }
        });
        if let Some(updated_at) = updated_at {
            record["updated_at"] = json!(updated_at);
        }
        record
    };

    let store = Arc::new(InMemoryThreadStore::new());
    for (thread_id, account_id, chat_id, updated_at) in [
        (
            "thread::older",
            "older",
            "100",
            Some("2026-07-01T10:00:00Z"),
        ),
        (
            "thread::latest-a",
            "latest-a",
            "200",
            Some("2026-07-01T11:00:00Z"),
        ),
        (
            "thread::latest-z",
            "latest-z",
            "300",
            Some("2026-07-01T11:00:00Z"),
        ),
        ("thread::untimed-zzzz", "untimed", "400", None),
    ] {
        store
            .set(thread_id, delivery_record(account_id, chat_id, updated_at))
            .await
            .unwrap();
    }

    let resolved = MessageRouter::resolve_delivery_target_from_store(store, "last")
        .await
        .expect("latest persisted delivery target");
    assert_eq!(resolved.0, "thread::latest-z");
    assert_eq!(resolved.1.account_id, "latest-z");
    assert_eq!(resolved.1.chat_id, "300");

    let untimed_store = Arc::new(InMemoryThreadStore::new());
    for (thread_id, account_id) in [
        ("thread::untimed-a", "untimed-a"),
        ("thread::untimed-z", "untimed-z"),
    ] {
        untimed_store
            .set(thread_id, delivery_record(account_id, "500", None))
            .await
            .unwrap();
    }

    let resolved = MessageRouter::resolve_delivery_target_from_store(untimed_store, "last")
        .await
        .expect("untimed persisted delivery target");
    assert_eq!(resolved.0, "thread::untimed-z");
    assert_eq!(resolved.1.account_id, "untimed-z");
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
        .await
        .unwrap();
    assert!(store.delete("bot1::main::u1").await.unwrap());

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

    assert!(store.get("bot1::main::u1").await.unwrap().is_none());
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
        .await
        .unwrap();

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

    let stored = store.get("thread::delivery").await.unwrap().unwrap();
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
        .await
        .unwrap();

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router
        .clear_last_delivery_with_persistence("thread::delivery")
        .await;

    let stored = store.get("thread::delivery").await.unwrap().unwrap();
    assert!(stored.get("delivery_context").is_none());
    assert!(stored["channel_bindings"][0]["last_delivery_at"].is_null());
}

#[tokio::test]
async fn test_delivery_persistence_sync_never_lists_store_keys() {
    // The endpoint delivery-timestamp sync runs on every run delivery; it
    // must stay point reads (registry + holder), never a store walk.
    let store = Arc::new(super::NoScanThreadStore::new());
    store
        .set(
            "thread::delivery-no-scan",
            json!({
                "thread_id": "thread::delivery-no-scan",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "bot1",
                    "binding_key": "42",
                    "chat_id": "42",
                    "display_label": "u1"
                }]
            }),
        )
        .await
        .unwrap();

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router
        .set_last_delivery_with_persistence(
            "thread::delivery-no-scan",
            garyx_models::routing::DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: Default::default(),
            },
        )
        .await;
    let after_set = store
        .get("thread::delivery-no-scan")
        .await
        .unwrap()
        .unwrap();
    assert!(
        after_set["channel_bindings"][0]["last_delivery_at"].is_string(),
        "point sync must stamp the holder binding"
    );

    router
        .clear_last_delivery_with_persistence("thread::delivery-no-scan")
        .await;

    assert_eq!(
        store.list_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "delivery persistence must not scan the thread store"
    );
    let stored = store
        .get("thread::delivery-no-scan")
        .await
        .unwrap()
        .unwrap();
    assert!(stored["channel_bindings"][0]["last_delivery_at"].is_null());
}
