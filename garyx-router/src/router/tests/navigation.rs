use super::*;
use crate::memory_store::InMemoryThreadStore;
use crate::thread_history::{ThreadHistoryRepository, ThreadTranscriptStore};
use crate::threads::{ChannelBinding, detach_endpoint_from_thread};
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

    // Before switching, resolves to a fresh thread id.
    let s1 = router.resolve_inbound_thread("telegram", "bot1", "u1", false, None);
    assert!(s1.starts_with("thread::"));

    // Switch to a custom thread.
    let binding_context_key = MessageRouter::build_binding_context_key("telegram", "bot1", "u1");
    router.switch_to_thread(&binding_context_key, "my_custom_session");

    let s2 = router.resolve_inbound_thread("telegram", "bot1", "u1", false, None);
    assert_eq!(s2, "my_custom_session");
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
    // Second reset returns false.
    assert!(!router.reset_thread_for_binding("telegram", "bot1", "u1"));
}

#[test]
fn test_navigate_thread() {
    let mut router = make_router();
    let user_key = "telegram:u1".to_owned();

    router.switch_to_thread(&user_key, "s1");
    router.switch_to_thread(&user_key, "s2");
    router.switch_to_thread(&user_key, "s3");

    // Navigate back.
    let prev = router.navigate_thread(&user_key, -1);
    assert_eq!(prev.as_deref(), Some("s2"));

    let prev2 = router.navigate_thread(&user_key, -1);
    assert_eq!(prev2.as_deref(), Some("s1"));

    // Already at oldest.
    assert_eq!(router.navigate_thread(&user_key, -1), None);

    // Navigate forward.
    let next = router.navigate_thread(&user_key, 1);
    assert_eq!(next.as_deref(), Some("s2"));

    let next2 = router.navigate_thread(&user_key, 1);
    assert_eq!(next2.as_deref(), Some("s3"));

    // Already at newest.
    assert_eq!(router.navigate_thread(&user_key, 1), None);
}

#[test]
fn test_navigate_empty_history() {
    let mut router = make_router();
    assert_eq!(router.navigate_thread("nonexistent", -1), None);
    assert_eq!(router.navigate_thread("nonexistent", 1), None);
}

#[tokio::test]
async fn test_navigate_thread_with_rebuild_after_restart() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "bot1::main::u1_a",
            json!({"from_id": "u1", "updated_at": "2026-03-01T10:00:00Z"}),
        )
        .await;
    store
        .set(
            "bot1::main::u1_b",
            json!({"from_id": "u1", "updated_at": "2026-03-01T11:00:00Z"}),
        )
        .await;
    store
        .set(
            "bot1::main::u1_c",
            json!({"from_id": "u1", "updated_at": "2026-03-01T12:00:00Z"}),
        )
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let binding_context_key = MessageRouter::build_binding_context_key("telegram", "bot1", "u1");

    let prev = router
        .navigate_thread_with_rebuild(
            &binding_context_key,
            NavigationContext {
                channel: "telegram",
                account_id: "bot1",
                thread_binding_key: "u1",
            },
            -1,
        )
        .await;
    assert_eq!(prev.as_deref(), Some("bot1::main::u1_b"));
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "bot1", "u1"),
        Some("bot1::main::u1_b")
    );

    let next = router
        .navigate_thread_with_rebuild(
            &binding_context_key,
            NavigationContext {
                channel: "telegram",
                account_id: "bot1",
                thread_binding_key: "u1",
            },
            1,
        )
        .await;
    assert_eq!(next.as_deref(), Some("bot1::main::u1_c"));
}

#[tokio::test]
async fn test_latest_message_text_for_thread_from_transcript() {
    let store = Arc::new(InMemoryThreadStore::new());
    let transcript_store = Arc::new(ThreadTranscriptStore::memory());
    transcript_store
        .append_committed_messages(
            "thread::wx-final",
            None,
            &[
                json!({"role": "assistant", "content": "first"}),
                json!({"role": "assistant", "content": "  "}),
                json!({"role": "assistant", "content": "final answer"}),
            ],
        )
        .await
        .unwrap();
    store
        .set(
            "thread::wx-final",
            json!({
                "history": {
                    "message_count": 3
                }
            }),
        )
        .await;

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router.set_thread_history_repository(Arc::new(ThreadHistoryRepository::new(
        store,
        transcript_store,
    )));
    let latest = router
        .latest_message_text_for_thread("thread::wx-final")
        .await;
    assert_eq!(latest.as_deref(), Some("final answer"));
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
            json!({
                "history": {
                    "message_count": 3
                }
            }),
        )
        .await;

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router.set_thread_history_repository(Arc::new(ThreadHistoryRepository::new(
        store,
        transcript_store,
    )));
    let latest = router
        .latest_assistant_message_text_for_thread("thread::wx-assistant-final")
        .await;
    assert_eq!(latest.as_deref(), Some("assistant-final"));
}

#[tokio::test]
async fn test_navigate_thread_with_rebuild_respects_bound_current_thread_for_prev_and_next() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::a",
            json!({
                "thread_id": "thread::a",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "u1",
                "label": "oldest",
                "updated_at": "2026-03-01T10:00:00Z"
            }),
        )
        .await;
    store
        .set(
            "thread::b",
            json!({
                "thread_id": "thread::b",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "u1",
                "label": "current",
                "updated_at": "2026-03-01T11:00:00Z"
            }),
        )
        .await;
    store
        .set(
            "thread::c",
            json!({
                "thread_id": "thread::c",
                "channel": "telegram",
                "account_id": "bot1",
                "from_id": "u1",
                "label": "newest",
                "updated_at": "9999-12-31T23:59:59Z"
            }),
        )
        .await;

    let mut seeded_router = MessageRouter::new(store.clone(), GaryxConfig::default());
    seeded_router
        .bind_endpoint_runtime(
            "thread::b",
            ChannelBinding {
                channel: "telegram".to_owned(),
                account_id: "bot1".to_owned(),
                binding_key: "u1".to_owned(),
                chat_id: "u1".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "u1".to_owned(),
                display_label: "User 1".to_owned(),
                last_inbound_at: Some("2026-03-01T11:00:00Z".to_owned()),
                last_delivery_at: None,
            },
        )
        .await
        .expect("binding current thread should succeed");

    let user_key = MessageRouter::build_binding_context_key("telegram", "bot1", "u1");

    let mut prev_router = MessageRouter::new(store.clone(), GaryxConfig::default());
    prev_router.rebuild_thread_indexes().await;
    let prev = prev_router
        .navigate_thread_with_rebuild(
            &user_key,
            NavigationContext {
                channel: "telegram",
                account_id: "bot1",
                thread_binding_key: "u1",
            },
            -1,
        )
        .await;
    assert_eq!(prev.as_deref(), Some("thread::a"));

    let mut next_router = MessageRouter::new(store, GaryxConfig::default());
    next_router.rebuild_thread_indexes().await;
    let next = next_router
        .navigate_thread_with_rebuild(
            &user_key,
            NavigationContext {
                channel: "telegram",
                account_id: "bot1",
                thread_binding_key: "u1",
            },
            1,
        )
        .await;
    assert_eq!(next.as_deref(), Some("thread::c"));
}

#[tokio::test]
async fn test_list_user_threads_for_account_sorted_desc() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "bot1::main::u1_old",
            json!({
                "from_id": "u1",
                "label": "old",
                "updated_at": "2026-03-01T10:00:00Z"
            }),
        )
        .await;
    store
        .set(
            "bot1::main::u1_new",
            json!({
                "from_id": "u1",
                "label": "new",
                "updated_at": "2026-03-01T11:00:00Z"
            }),
        )
        .await;
    store
        .set(
            "bot2::main::u1_other_account",
            json!({
                "from_id": "u1",
                "label": "other-account",
                "updated_at": "2026-03-01T12:00:00Z"
            }),
        )
        .await;

    let router = MessageRouter::new(store, GaryxConfig::default());
    let listed = router
        .list_user_threads_for_account("telegram", "bot1", "u1")
        .await;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].thread_id, "bot1::main::u1_new");
    assert_eq!(listed[0].label.as_deref(), Some("new"));
    assert_eq!(listed[1].thread_id, "bot1::main::u1_old");
}

#[tokio::test]
async fn test_rebuild_thread_indexes_prefers_latest_updated_binding_for_duplicate_endpoint() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::older",
            json!({
                "thread_id": "thread::older",
                "thread_id": "thread::older",
                "label": "Older",
                "updated_at": "2026-03-01T10:00:00Z",
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
    store
        .set(
            "thread::newer",
            json!({
                "thread_id": "thread::newer",
                "thread_id": "thread::newer",
                "label": "Newer",
                "updated_at": "2026-03-01T12:00:00Z",
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

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let stats = router.rebuild_thread_indexes().await;

    assert_eq!(stats.endpoint_bindings, 1);
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "main", "user42")
            .await
            .as_deref(),
        Some("thread::newer")
    );
}

#[tokio::test]
async fn test_rebuild_thread_indexes_preserves_switched_thread_history() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
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
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let user_key = MessageRouter::build_binding_context_key("telegram", "main", "u1");
    router.switch_to_thread(&user_key, "s1");
    router.switch_to_thread(&user_key, "s2");

    let stats = router.rebuild_thread_indexes().await;

    assert_eq!(stats.endpoint_bindings, 1);
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "main", "u1"),
        Some("s2")
    );
    assert_eq!(
        router
            .thread_nav
            .binding_thread_history
            .get(&user_key)
            .cloned(),
        Some(vec!["s1".to_owned(), "s2".to_owned()])
    );
}

#[tokio::test]
async fn test_list_user_threads_uses_message_summary_when_label_missing() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::summary-demo",
            json!({
                "thread_id": "thread::summary-demo",
                "channel": "telegram",
                "account_id": "main",
                "from_id": "alice",
                "updated_at": "2026-03-16T12:00:00Z",
                "messages": [
                    {
                        "role": "user",
                        "content": "帮我检查一下 gateway 重连时为什么会把正在 streaming 的消息直接打成失败"
                    }
                ],
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;

    let router = MessageRouter::new(store, GaryxConfig::default());
    let listed = router
        .list_user_threads_for_account("telegram", "main", "alice")
        .await;

    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].label.as_deref(),
        Some("帮我检查一下 gateway 重连时为什么会把正在 streaming 的消息直接打成失败")
    );
}

#[tokio::test]
async fn test_list_user_threads_shortens_canonical_thread_id_when_label_missing() {
    let store = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::c9dade26-eca3-4f9b-b910-e93e1c2941c2",
            json!({
                "thread_id": "thread::c9dade26-eca3-4f9b-b910-e93e1c2941c2",
                "channel": "telegram",
                "account_id": "main",
                "from_id": "alice",
                "updated_at": "2026-03-16T12:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;

    let router = MessageRouter::new(store, GaryxConfig::default());
    let listed = router
        .list_user_threads_for_account("telegram", "main", "alice")
        .await;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].label.as_deref(), Some("Thread c9dade26"));
}

#[tokio::test]
async fn test_rebuild_thread_indexes_clears_detached_endpoint_thread_context() {
    let store: Arc<dyn crate::ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
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
        .await;

    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    let user_key = MessageRouter::build_binding_context_key("telegram", "main", "alice");
    router.switch_to_thread(&user_key, "thread::bound");

    detach_endpoint_from_thread(&store, "telegram::main::alice")
        .await
        .expect("detach should succeed");
    let stats = router.rebuild_thread_indexes().await;

    assert_eq!(stats.endpoint_bindings, 0);
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "main", "alice"),
        None
    );
    assert_eq!(
        router.thread_nav.binding_thread_history.get(&user_key),
        None
    );
}

#[tokio::test]
async fn test_rebuild_thread_indexes_preserves_explicit_thread_overrides_for_bound_endpoint() {
    let store: Arc<dyn crate::ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
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
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let user_key = MessageRouter::build_binding_context_key("telegram", "main", "alice");
    router.switch_to_thread(&user_key, "named-session");

    let stats = router.rebuild_thread_indexes().await;

    assert_eq!(stats.endpoint_bindings, 1);
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "main", "alice"),
        Some("named-session")
    );
}

#[tokio::test]
async fn test_rebuild_thread_indexes_clears_missing_explicit_thread_override_for_bound_endpoint() {
    let store: Arc<dyn crate::ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::bound",
            json!({
                "thread_id": "thread::bound",
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
        .await;

    let mut router = MessageRouter::new(store, GaryxConfig::default());
    let user_key = MessageRouter::build_binding_context_key("telegram", "main", "alice");
    router.switch_to_thread(&user_key, "thread::missing");

    let stats = router.rebuild_thread_indexes().await;

    assert_eq!(stats.endpoint_bindings, 1);
    assert_eq!(
        router.get_current_thread_id_for_binding("telegram", "main", "alice"),
        None
    );
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "main", "alice")
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

    let saved = store.get("bot1::main::u1:session-1").await.unwrap();
    assert_eq!(saved["thread_id"], "bot1::main::u1:session-1");
    assert_eq!(saved["channel"], "telegram");
    assert_eq!(saved["account_id"], "bot1");
    assert_eq!(saved["thread_binding_key"], "u1");
    assert_eq!(saved["label"], "session-1");
    assert!(saved["created_at"].is_string());
    assert!(saved["updated_at"].is_string());
    assert!(saved["messages"].is_array());
    assert!(saved["context"].is_object());
}

#[test]
fn test_switch_truncates_forward_history() {
    let mut router = make_router();
    let uk = "telegram:u1".to_owned();

    router.switch_to_thread(&uk, "s1");
    router.switch_to_thread(&uk, "s2");
    router.switch_to_thread(&uk, "s3");

    // Go back to s2.
    router.navigate_thread(&uk, -1);
    // Now switch to s4 -- this should truncate s3 from history.
    router.switch_to_thread(&uk, "s4");

    // Forward should be impossible (s3 was truncated).
    assert_eq!(router.navigate_thread(&uk, 1), None);

    // Current should be s4.
    assert_eq!(
        router
            .thread_nav
            .binding_thread_map
            .get(&uk)
            .map(|s| s.as_str()),
        Some("s4")
    );
}

#[test]
fn test_history_limit() {
    let mut router = make_router();
    let uk = "telegram:u1".to_owned();

    for i in 0..30 {
        router.switch_to_thread(&uk, &format!("s{i}"));
    }

    let history = &router.thread_nav.binding_thread_history[&uk];
    assert!(history.len() <= MAX_HISTORY);
}

#[test]
fn test_clear_thread_references_removes_deleted_current_thread() {
    let mut router = make_router();
    let uk = "telegram:u1".to_owned();

    router.switch_to_thread(&uk, "s1");
    router.switch_to_thread(&uk, "s2");
    router.switch_to_thread(&uk, "s3");

    router.clear_thread_references("s3");

    assert_eq!(
        router
            .thread_nav
            .binding_thread_map
            .get(&uk)
            .map(String::as_str),
        Some("s2")
    );
    assert_eq!(
        router.thread_nav.binding_thread_index.get(&uk).copied(),
        Some(1)
    );
    assert_eq!(
        router.thread_nav.binding_thread_history.get(&uk).cloned(),
        Some(vec!["s1".to_owned(), "s2".to_owned()])
    );
}

#[test]
fn test_clear_thread_references_prunes_deleted_history_entries() {
    let mut router = make_router();
    let uk = "telegram:u1".to_owned();

    router.switch_to_thread(&uk, "s1");
    router.switch_to_thread(&uk, "s2");
    router.switch_to_thread(&uk, "s3");
    router.navigate_thread(&uk, -1);

    router.clear_thread_references("s1");

    assert_eq!(
        router
            .thread_nav
            .binding_thread_map
            .get(&uk)
            .map(String::as_str),
        Some("s2")
    );
    assert_eq!(
        router.thread_nav.binding_thread_index.get(&uk).copied(),
        Some(0)
    );
    assert_eq!(
        router.thread_nav.binding_thread_history.get(&uk).cloned(),
        Some(vec!["s2".to_owned(), "s3".to_owned()])
    );
}
