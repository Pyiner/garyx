use super::*;
use crate::memory_store::InMemoryThreadStore;
use crate::thread_history::{ThreadHistoryRepository, ThreadTranscriptStore};
use crate::threads::{
    ChannelBinding, bindings_from_value, remove_binding, upsert_known_channel_endpoint,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn test_build_binding_context_key_dm() {
    let key = MessageRouter::build_binding_context_key("telegram", "bot1", "user1");
    assert_eq!(key, "telegram::bot1::user1");
}

#[test]
fn test_build_binding_context_key_group_topic() {
    let key = MessageRouter::build_binding_context_key("telegram", "bot1", "group42:topic:9");
    assert_eq!(key, "telegram::bot1::group42:topic:9");
}

#[test]
fn test_resolve_inbound_thread_dm() {
    let mut router = make_router();
    let thread_id = router.resolve_inbound_thread("telegram", "bot1", "user1", false, None);
    assert!(thread_id.starts_with("thread::"));
}

#[test]
fn test_resolve_inbound_thread_group() {
    let mut router = make_router();
    let thread_id =
        router.resolve_inbound_thread("telegram", "bot1", "user1", true, Some("group42"));
    assert!(thread_id.starts_with("thread::"));
}

#[test]
fn test_switch_and_resolve() {
    let mut router = make_router();
    let initial = router.resolve_inbound_thread("telegram", "bot1", "u1", false, None);
    assert!(initial.starts_with("thread::"));

    let binding_context_key = MessageRouter::build_binding_context_key("telegram", "bot1", "u1");
    router.switch_to_thread(&binding_context_key, "my_custom_session");

    assert_eq!(
        router.resolve_inbound_thread("telegram", "bot1", "u1", false, None),
        "my_custom_session"
    );
}

#[test]
fn test_get_current_and_reset() {
    let mut router = make_router();
    let binding_context_key = MessageRouter::build_binding_context_key("telegram", "bot1", "u1");

    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "u1"),
        None
    );
    router.switch_to_thread(&binding_context_key, "session_a");
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "u1"),
        Some("session_a")
    );
    assert!(router.reset_thread_for_binding("telegram", "bot1", "u1"));
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "u1"),
        None
    );
    assert!(!router.reset_thread_for_binding("telegram", "bot1", "u1"));
}

#[tokio::test]
async fn test_latest_assistant_message_text_for_thread_ignores_user_messages() {
    let store = Arc::new(InMemoryThreadStore::new());
    let transcript_store = Arc::new(ThreadTranscriptStore::memory());
    transcript_store
        .append_committed_messages(
            "thread::wx-assistant-final",
            None,
            &[
                json!({"role": "assistant", "content": "assistant-first"}),
                json!({"role": "user", "content": "user-latest"}),
                json!({"role": "assistant", "content": "assistant-final"}),
            ],
        )
        .await
        .unwrap();
    store
        .set(
            "thread::wx-assistant-final",
            json!({"history": {"message_count": 3}}),
        )
        .await
        .unwrap();

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router.set_thread_history_repository(Arc::new(ThreadHistoryRepository::new(
        store,
        transcript_store,
    )));
    assert_eq!(
        router
            .latest_assistant_message_text_for_thread("thread::wx-assistant-final")
            .await
            .as_deref(),
        Some("assistant-final")
    );
}

#[tokio::test]
async fn test_lazy_endpoint_resolution_prefers_latest_updated_binding_for_duplicate_endpoint() {
    let store = Arc::new(InMemoryThreadStore::new());
    for (thread_id, label, updated_at) in [
        ("thread::older", "Older", "2026-03-01T10:00:00Z"),
        ("thread::newer", "Newer", "2026-03-01T12:00:00Z"),
    ] {
        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "label": label,
                    "updated_at": updated_at,
                    "channel_bindings": [{
                        "channel": "telegram",
                        "account_id": "main",
                        "binding_key": "user42",
                        "chat_id": "user42",
                        "display_label": "User 42"
                    }]
                }),
            )
            .await;
    }

    let mut router = MessageRouter::new(store, GaryxConfig::default());

    // The lazy point lookup applies the preferred-holder tie-break when
    // two records hold the same endpoint (no startup rebuild).
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "main", "user42")
            .await
            .as_deref(),
        Some("thread::newer")
    );
}

#[tokio::test]
async fn test_resolve_endpoint_thread_id_uses_projected_owner_point_lookup() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
                "label": "Bound",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "user42",
                    "chat_id": "user42",
                    "display_label": "Test User"
                }]
            }),
        )
        .await
        .unwrap();

    let binding = ChannelBinding {
        channel: "telegram".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "user42".to_owned(),
        chat_id: "user42".to_owned(),
        display_label: "Test User".to_owned(),
        ..Default::default()
    };
    let (mut router, mutator) = test_router(store, GaryxConfig::default());
    mutator.seed_owner("thread::bound", binding).await;

    assert!(router.thread_nav.endpoint_thread_map.is_empty());
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "main", "user42")
            .await
            .as_deref(),
        Some("thread::bound")
    );
    assert_eq!(
        router
            .thread_nav
            .endpoint_thread_map
            .get("telegram::main::user42")
            .map(String::as_str),
        Some("thread::bound")
    );
}

#[tokio::test]
async fn test_rebuild_thread_indexes_clears_detached_endpoint_thread_context() {
    let store: Arc<dyn crate::ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await
        .unwrap();

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    let binding_context_key = MessageRouter::build_binding_context_key("telegram", "main", "alice");
    router.switch_to_thread(&binding_context_key, "thread::bound");
    let mut record = store.get("thread::bound").await.unwrap();
    let binding = bindings_from_value(&record).into_iter().next().unwrap();
    upsert_known_channel_endpoint(&store, &binding)
        .await
        .unwrap();
    assert!(remove_binding(&mut record, "telegram::main::alice"));
    store.set("thread::bound", record).await;

    let stats = router.rebuild_thread_indexes().await;
    assert_eq!(stats.endpoint_bindings, 0);
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "main", "alice"),
        None
    );
}

#[tokio::test]
async fn test_lazy_endpoint_resolution_preserves_explicit_thread_overrides_for_bound_endpoint() {
    let store: Arc<dyn crate::ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await
        .unwrap();

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let binding_context_key = MessageRouter::build_binding_context_key("telegram", "main", "alice");
    router.switch_to_thread(&binding_context_key, "named-session");

    let stats = router.rebuild_thread_indexes().await;
    assert_eq!(stats.endpoint_bindings, 1);
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "main", "alice"),
        Some("named-session")
    );
}

#[tokio::test]
async fn test_canonical_resolution_skips_missing_explicit_thread_override_for_bound_endpoint() {
    let store: Arc<dyn crate::ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await
        .unwrap();

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let binding_context_key = MessageRouter::build_binding_context_key("telegram", "main", "alice");
    router.switch_to_thread(&binding_context_key, "thread::missing");

    let stats = router.rebuild_thread_indexes().await;
    assert_eq!(stats.endpoint_bindings, 1);
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "main", "alice"),
        None
    );
    assert_eq!(
        router
            .current_canonical_thread_for_binding("telegram", "main", "alice")
            .await
            .as_deref(),
        Some("thread::bound")
    );
}

#[tokio::test]
async fn test_ensure_thread_entry_creates_with_label() {
    let store = Arc::new(InMemoryThreadStore::new());
    let router = MessageRouter::new(store.clone(), GaryxConfig::default());

    router
        .ensure_thread_entry(
            "bot1::main::u1:session-1",
            "telegram",
            "bot1",
            "u1",
            Some("session-1"),
        )
        .await;

    let saved = store
        .get("bot1::main::u1:session-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved["thread_id"], "bot1::main::u1:session-1");
    assert_eq!(saved["channel"], "telegram");
    assert_eq!(saved["account_id"], "bot1");
    assert_eq!(saved["thread_binding_key"], "u1");
    assert_eq!(saved["label"], "session-1");
    assert!(saved["created_at"].is_string());
    assert!(saved["updated_at"].is_string());
    assert!(saved.get("messages").is_none());
    assert!(saved["context"].is_object());
}

#[test]
fn test_clear_thread_references_removes_only_matching_current_entries() {
    let mut router = make_router();
    router.switch_to_thread("telegram::main::one", "thread::one");
    router.switch_to_thread("telegram::main::two", "thread::two");

    router.clear_thread_references("thread::one");

    assert!(
        !router
            .thread_nav
            .binding_thread_map
            .contains_key("telegram::main::one")
    );
    assert_eq!(
        router
            .thread_nav
            .binding_thread_map
            .get("telegram::main::two")
            .map(String::as_str),
        Some("thread::two")
    );
}

#[test]
fn test_purge_thread_from_indexes_clears_binding_and_endpoint_entries() {
    let mut router = make_router();
    router.switch_to_thread("telegram::main::chat-1", "thread::one");
    router.thread_nav.endpoint_thread_map.insert(
        "telegram::main::chat-1".to_owned(),
        "thread::one".to_owned(),
    );
    router.thread_nav.endpoint_thread_map.insert(
        "telegram::main::chat-2".to_owned(),
        "thread::two".to_owned(),
    );

    router.purge_thread_from_indexes("thread::one");

    assert!(
        !router
            .thread_nav
            .binding_thread_map
            .values()
            .any(|thread| thread == "thread::one")
    );
    assert!(
        !router
            .thread_nav
            .endpoint_thread_map
            .contains_key("telegram::main::chat-1")
    );
    assert_eq!(
        router
            .thread_nav
            .endpoint_thread_map
            .get("telegram::main::chat-2")
            .map(String::as_str),
        Some("thread::two")
    );
}

#[test]
fn test_purge_endpoint_binding_drops_current_and_endpoint_entries() {
    let mut router = make_router();
    let endpoint_key = "telegram::main::chat-1";
    router.switch_to_thread(endpoint_key, "thread::one");
    router
        .thread_nav
        .endpoint_thread_map
        .insert(endpoint_key.to_owned(), "thread::one".to_owned());

    router.purge_endpoint_binding(endpoint_key);

    assert!(
        !router
            .thread_nav
            .binding_thread_map
            .contains_key(endpoint_key)
    );
    assert!(
        !router
            .thread_nav
            .endpoint_thread_map
            .contains_key(endpoint_key)
    );
}
