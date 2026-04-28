use super::*;
use crate::memory_store::InMemoryThreadStore;
use serde_json::json;

#[test]
fn test_record_and_lookup() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("session1", "telegram", "bot1", "chat-1", None, "msg42");

    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "msg42"),
        Some("session1")
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "msg99"),
        None
    );
}

#[test]
fn test_empty_message_id_skipped() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("s1", "telegram", "bot1", "chat-1", None, "  ");
    assert_eq!(idx.get_stats().total_entries, 0);
}

#[test]
fn test_clear_thread() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("s1", "telegram", "bot1", "chat-1", None, "m1");
    idx.record_outbound("s1", "telegram", "bot1", "chat-1", None, "m2");
    idx.record_outbound("s2", "telegram", "bot1", "chat-2", None, "m3");

    idx.clear_thread("s1");
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "m1"),
        None
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "m2"),
        None
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-2"), None, "m3"),
        Some("s2")
    );
}

#[test]
fn test_clear_all() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("s1", "telegram", "bot1", "chat-1", None, "m1");
    idx.record_outbound("s2", "telegram", "bot1", "chat-2", None, "m2");
    idx.clear_all();
    assert_eq!(idx.get_stats().total_entries, 0);
    assert_eq!(idx.get_stats().threads_tracked, 0);
}

#[test]
fn test_pruning() {
    let mut idx = MessageRoutingIndex::new();
    idx.max_messages_per_thread = 5;

    for i in 0..10 {
        idx.record_outbound("s1", "telegram", "bot1", "chat-1", None, &format!("m{i}"));
    }

    let stats = idx.get_stats();
    assert!(stats.total_entries <= 10);
}

#[test]
fn test_get_stats() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("s1", "ch", "a", "chat-1", None, "m1");
    idx.record_outbound("s2", "ch", "a", "chat-2", None, "m2");

    let stats = idx.get_stats();
    assert_eq!(stats.total_entries, 2);
    assert_eq!(stats.threads_tracked, 2);
}

#[test]
fn test_same_message_id_is_scoped_by_chat_id() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("session_chat_1", "telegram", "bot1", "chat-1", None, "42");
    idx.record_outbound("session_chat_2", "telegram", "bot1", "chat-2", None, "42");

    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "42"),
        Some("session_chat_1")
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-2"), None, "42"),
        Some("session_chat_2")
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-3"), None, "42"),
        None
    );
}

#[test]
fn test_clear_thread_chat_also_removes_legacy_unscoped_routes() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("session_1", "telegram", "bot1", "", None, "legacy-1");
    idx.record_outbound("session_1", "telegram", "bot1", "chat-1", None, "scoped-1");
    idx.record_outbound("session_1", "telegram", "bot1", "chat-2", None, "scoped-2");

    idx.clear_thread_chat("session_1", "telegram", "bot1", "chat-1", None);

    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "legacy-1"),
        None
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "scoped-1"),
        None
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-2"), None, "scoped-2"),
        Some("session_1")
    );
}

#[test]
fn test_clear_thread_chat_for_topic_preserves_primary_routes() {
    let mut idx = MessageRoutingIndex::new();
    idx.record_outbound("session_1", "telegram", "bot1", "chat-1", None, "primary-1");
    idx.record_outbound(
        "session_1",
        "telegram",
        "bot1",
        "chat-1",
        Some("chat-1_t100"),
        "topic-1",
    );

    idx.clear_thread_chat(
        "session_1",
        "telegram",
        "bot1",
        "chat-1",
        Some("chat-1_t100"),
    );

    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "primary-1"),
        Some("session_1")
    );
    assert_eq!(
        idx.lookup_thread_for_chat(
            "telegram",
            "bot1",
            Some("chat-1"),
            Some("chat-1_t100"),
            "topic-1",
        ),
        None
    );
}

#[tokio::test]
async fn test_rebuild_from_store() {
    let store = InMemoryThreadStore::new();
    store
        .set(
            "s1",
            json!({
                "outbound_message_ids": [
                    {"channel": "telegram", "account_id": "bot1", "message_id": "100"},
                    {"channel": "telegram", "account_id": "bot1", "message_id": "101"},
                ]
            }),
        )
        .await;

    let mut idx = MessageRoutingIndex::new();
    let count = idx.rebuild_from_store(&store, "telegram").await;
    assert_eq!(count, 2);
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some(""), None, "100"),
        Some("s1")
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some(""), None, "101"),
        Some("s1")
    );
}

#[tokio::test]
async fn test_rebuild_from_store_restores_chat_scoped_routing() {
    let store = InMemoryThreadStore::new();
    store
        .set(
            "s1",
            json!({
                "outbound_message_ids": [
                    {"channel": "telegram", "account_id": "bot1", "chat_id": "chat-1", "message_id": "42"}
                ]
            }),
        )
        .await;
    store
        .set(
            "s2",
            json!({
                "outbound_message_ids": [
                    {"channel": "telegram", "account_id": "bot1", "chat_id": "chat-2", "message_id": "42"}
                ]
            }),
        )
        .await;

    let mut idx = MessageRoutingIndex::new();
    let count = idx.rebuild_from_store(&store, "telegram").await;
    assert_eq!(count, 2);
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-1"), None, "42"),
        Some("s1")
    );
    assert_eq!(
        idx.lookup_thread_for_chat("telegram", "bot1", Some("chat-2"), None, "42"),
        Some("s2")
    );
}
