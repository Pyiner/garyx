use super::*;
use crate::memory_store::InMemoryThreadStore;
use serde_json::json;
use std::sync::Arc;

#[test]
fn test_reply_routing() {
    let mut router = make_router();

    router.record_outbound_message("session_x", "telegram", "bot1", "msg100");

    assert_eq!(
        router.resolve_reply_thread("telegram", "bot1", "msg100"),
        Some("session_x")
    );
    assert_eq!(
        router.resolve_reply_thread("telegram", "bot1", "msg999"),
        None
    );
}

#[test]
fn test_reply_routing_is_scoped_by_chat_id() {
    let mut router = make_router();

    router.record_outbound_message_for_chat(
        "session_chat_1",
        "telegram",
        "bot1",
        "chat-1",
        None,
        "42",
    );
    router.record_outbound_message_for_chat(
        "session_chat_2",
        "telegram",
        "bot1",
        "chat-2",
        None,
        "42",
    );

    assert_eq!(
        router.resolve_reply_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "42"),
        Some("session_chat_1")
    );
    assert_eq!(
        router.resolve_reply_thread_for_chat("telegram", "bot1", Some("chat-2"), None, "42"),
        Some("session_chat_2")
    );
    assert_eq!(
        router.resolve_reply_thread_for_chat("telegram", "bot1", Some("chat-3"), None, "42"),
        None
    );
}

#[test]
fn test_reply_routing_is_scoped_by_thread_binding_key() {
    let mut router = make_router();

    router.record_outbound_message_for_chat(
        "session_topic_1",
        "telegram",
        "bot1",
        "chat-1",
        Some("chat-1_t100"),
        "42",
    );
    router.record_outbound_message_for_chat(
        "session_topic_2",
        "telegram",
        "bot1",
        "chat-1",
        Some("chat-1_t200"),
        "42",
    );

    assert_eq!(
        router.resolve_reply_thread_for_chat(
            "telegram",
            "bot1",
            Some("chat-1"),
            Some("chat-1_t100"),
            "42",
        ),
        Some("session_topic_1")
    );
    assert_eq!(
        router.resolve_reply_thread_for_chat(
            "telegram",
            "bot1",
            Some("chat-1"),
            Some("chat-1_t200"),
            "42",
        ),
        Some("session_topic_2")
    );
}

#[test]
fn test_switched_thread_is_account_scoped() {
    let mut router = make_router();

    let bot1_key = MessageRouter::build_binding_context_key("telegram", "bot1", "u1");
    router.switch_to_thread(&bot1_key, "custom_bot1");

    let bot1_thread = router.resolve_inbound_thread("telegram", "bot1", "u1", false, Some("u1"));
    let bot2_thread = router.resolve_inbound_thread("telegram", "bot2", "u1", false, Some("u1"));

    assert_eq!(bot1_thread, "custom_bot1");
    assert!(bot2_thread.starts_with("thread::"));
}

#[test]
fn test_is_scheduled_thread() {
    assert!(MessageRouter::is_scheduled_thread("cron::daily"));
    assert!(!MessageRouter::is_scheduled_thread("bot1::main::user1"));
}

#[test]
fn test_resolve_agent_default() {
    let router = make_router();
    assert_eq!(
        router.resolve_agent_for_channel("telegram", "bot1", Some("u1"), false),
        "main"
    );
}

#[test]
fn test_update_config() {
    let mut router = make_router();
    assert_eq!(router.default_agent, "main");

    let mut new_config = GaryxConfig::default();
    new_config
        .agents
        .insert("default".to_owned(), json!("assistant1"));
    router.update_config(new_config);
    assert_eq!(router.default_agent, "assistant1");
}

#[tokio::test]
async fn test_rebuild_routing_index() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "s1",
            json!({
                "outbound_message_ids": [
                    {"channel": "telegram", "account_id": "bot1", "message_id": "42"}
                ]
            }),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let count = router.rebuild_routing_index("telegram").await;
    assert_eq!(count, 1);
    assert_eq!(
        router.resolve_reply_thread("telegram", "bot1", "42"),
        Some("s1")
    );
}

#[tokio::test]
async fn test_record_outbound_message_with_persistence_rebuilds() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    store
        .set(
            "s_outbound",
            json!({
                "messages": []
            }),
        )
        .await;

    router
        .record_outbound_message_with_persistence(
            "s_outbound",
            "telegram",
            "bot1",
            "42",
            Some("42_t9"),
            "m-001",
        )
        .await;

    let saved = store
        .get("s_outbound")
        .await
        .expect("existing thread should persist outbound routing");
    let records = saved["outbound_message_ids"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["message_id"], "m-001");
    assert_eq!(records[0]["chat_id"], "42");
    assert_eq!(records[0]["thread_binding_key"], "42_t9");

    let mut router_after_restart = MessageRouter::new(store, GaryxConfig::default());
    assert_eq!(
        router_after_restart.rebuild_routing_index("telegram").await,
        1
    );
    assert_eq!(
        router_after_restart.resolve_reply_thread_for_chat(
            "telegram",
            "bot1",
            Some("42"),
            Some("42_t9"),
            "m-001",
        ),
        Some("s_outbound")
    );
}

#[tokio::test]
async fn test_record_outbound_message_with_persistence_does_not_recreate_deleted_session() {
    let store = Arc::new(InMemoryThreadStore::new());
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());

    store
        .set(
            "s_deleted",
            json!({
                "messages": []
            }),
        )
        .await;
    assert!(store.delete("s_deleted").await);

    router
        .record_outbound_message_with_persistence(
            "s_deleted",
            "telegram",
            "bot1",
            "42",
            None,
            "m-001",
        )
        .await;

    assert!(store.get("s_deleted").await.is_none());
    assert_eq!(
        router.resolve_reply_thread_for_chat("telegram", "bot1", Some("42"), None, "m-001"),
        Some("s_deleted")
    );
}
