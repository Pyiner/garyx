use super::*;
use crate::memory_store::InMemoryThreadStore;
use serde_json::json;

#[tokio::test]
async fn list_known_channel_endpoints_includes_thread_metadata() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::123",
            json!({
                "thread_id": "thread::123",
                "label": "Alice Support",
                "workspace_dir": "/tmp/test-workspace",
                "updated_at": "2026-03-07T10:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "12345678",
                    "chat_id": "12345678",
                    "display_label": "Alice",
                    "last_inbound_at": "2026-03-07T09:59:00Z",
                    "last_delivery_at": "2026-03-07T10:00:00Z"
                }]
            }),
        )
        .await;

    let endpoints = list_known_channel_endpoints(&store).await;
    assert_eq!(endpoints.len(), 1);

    let endpoint = &endpoints[0];
    assert_eq!(endpoint.thread_id.as_deref(), Some("thread::123"));
    assert_eq!(endpoint.thread_label.as_deref(), Some("Alice Support"));
    assert_eq!(
        endpoint.workspace_dir.as_deref(),
        Some("/tmp/test-workspace")
    );
    assert_eq!(
        endpoint.thread_updated_at.as_deref(),
        Some("2026-03-07T10:00:00Z")
    );
}

#[tokio::test]
async fn detached_endpoint_remains_known_without_thread_binding() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::123",
            json!({
                "thread_id": "thread::123",
                "label": "Alice Support",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "12345678",
                    "chat_id": "12345678",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;

    let detached = detach_endpoint_from_thread(&store, "telegram::main::12345678")
        .await
        .unwrap();
    assert_eq!(detached.as_deref(), Some("thread::123"));

    let endpoints = list_known_channel_endpoints(&store).await;
    assert_eq!(endpoints.len(), 1);
    let endpoint = &endpoints[0];
    assert_eq!(endpoint.endpoint_key, "telegram::main::12345678");
    assert!(endpoint.thread_id.is_none());
    assert_eq!(endpoint.display_label, "Alice");
}

#[tokio::test]
async fn list_known_channel_endpoints_prefers_latest_thread_binding_when_duplicates_exist() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::old",
            json!({
                "thread_id": "thread::old",
                "label": "Old",
                "updated_at": "2026-03-07T10:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "12345678",
                    "chat_id": "12345678",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;
    store
        .set(
            "thread::new",
            json!({
                "thread_id": "thread::new",
                "label": "New",
                "updated_at": "2026-03-07T12:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "12345678",
                    "chat_id": "12345678",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;

    let endpoints = list_known_channel_endpoints(&store).await;
    assert_eq!(endpoints.len(), 1);
    let endpoint = &endpoints[0];
    assert_eq!(endpoint.endpoint_key, "telegram::main::12345678");
    assert_eq!(endpoint.thread_id.as_deref(), Some("thread::new"));
    assert_eq!(endpoint.thread_label.as_deref(), Some("New"));
    assert_eq!(
        endpoint.thread_updated_at.as_deref(),
        Some("2026-03-07T12:00:00Z")
    );
}

#[tokio::test]
async fn detach_endpoint_from_thread_clears_all_duplicate_bindings() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    for (thread_id, updated_at) in [
        ("thread::one", "2026-03-07T10:00:00Z"),
        ("thread::two", "2026-03-07T12:00:00Z"),
    ] {
        store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "updated_at": updated_at,
                    "channel_bindings": [{
                        "channel": "telegram",
                        "account_id": "main",
                        "peer_id": "12345678",
                        "chat_id": "12345678",
                        "display_label": "Alice"
                    }]
                }),
            )
            .await;
    }

    let detached = detach_endpoint_from_thread(&store, "telegram::main::12345678")
        .await
        .unwrap();
    assert_eq!(detached.as_deref(), Some("thread::two"));

    for thread_id in ["thread::one", "thread::two"] {
        let value = store.get(thread_id).await.unwrap();
        assert_eq!(bindings_from_value(&value).len(), 0);
    }
}

#[tokio::test]
async fn update_thread_record_preserves_workspace_when_not_provided() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::keep",
            json!({
                "thread_id": "thread::keep",
                "label": "Before",
                "workspace_dir": "/tmp/workspace-a"
            }),
        )
        .await;

    let updated = update_thread_record(&store, "thread::keep", Some("After".to_owned()), None)
        .await
        .unwrap();

    assert_eq!(label_from_value(&updated).as_deref(), Some("After"));
    assert_eq!(
        workspace_dir_from_value(&updated).as_deref(),
        Some("/tmp/workspace-a")
    );
}

#[tokio::test]
async fn update_thread_record_rejects_clearing_workspace() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::clear",
            json!({
                "thread_id": "thread::clear",
                "workspace_dir": "/tmp/workspace-b"
            }),
        )
        .await;

    let error = update_thread_record(&store, "thread::clear", None, Some("   ".to_owned()))
        .await
        .unwrap_err();

    assert!(error.contains("workspace_dir is immutable"));
    let stored = store.get("thread::clear").await.unwrap();
    assert_eq!(
        workspace_dir_from_value(&stored).as_deref(),
        Some("/tmp/workspace-b")
    );
}

#[tokio::test]
async fn update_thread_record_rejects_workspace_change() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::move",
            json!({
                "thread_id": "thread::move",
                "workspace_dir": "/tmp/workspace-a"
            }),
        )
        .await;

    let error = update_thread_record(
        &store,
        "thread::move",
        None,
        Some("/tmp/workspace-b".to_owned()),
    )
    .await
    .unwrap_err();

    assert!(error.contains("workspace_dir is immutable"));
    let stored = store.get("thread::move").await.unwrap();
    assert_eq!(
        workspace_dir_from_value(&stored).as_deref(),
        Some("/tmp/workspace-a")
    );
}

#[tokio::test]
async fn update_thread_record_allows_initial_workspace_set() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::initial",
            json!({
                "thread_id": "thread::initial"
            }),
        )
        .await;

    let updated = update_thread_record(
        &store,
        "thread::initial",
        None,
        Some("/tmp/workspace-c".to_owned()),
    )
    .await
    .unwrap();

    assert_eq!(
        workspace_dir_from_value(&updated).as_deref(),
        Some("/tmp/workspace-c")
    );
}

#[tokio::test]
async fn update_thread_record_allows_same_workspace_value() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::same",
            json!({
                "thread_id": "thread::same",
                "workspace_dir": "/tmp/workspace-d"
            }),
        )
        .await;

    let updated = update_thread_record(
        &store,
        "thread::same",
        Some("Same".to_owned()),
        Some("/tmp/workspace-d".to_owned()),
    )
    .await
    .unwrap();

    assert_eq!(label_from_value(&updated).as_deref(), Some("Same"));
    assert_eq!(
        workspace_dir_from_value(&updated).as_deref(),
        Some("/tmp/workspace-d")
    );
}

#[tokio::test]
async fn create_thread_record_persists_workspace_dir_as_plain_path() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let (_thread_id, created) = create_thread_record(
        &store,
        ThreadEnsureOptions {
            label: Some("Path backed thread".to_owned()),
            workspace_dir: Some("  /tmp/path-only-workspace  ".to_owned()),
            ..ThreadEnsureOptions::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        workspace_dir_from_value(&created).as_deref(),
        Some("/tmp/path-only-workspace")
    );
}

#[tokio::test]
async fn create_thread_record_persists_metadata_object() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let (thread_id, created) = create_thread_record(
        &store,
        ThreadEnsureOptions {
            metadata: HashMap::from([(
                "agent_team_child".to_owned(),
                json!({
                    "team_id": "product-ship-camel",
                    "child_agent_id": "planner"
                }),
            )]),
            ..ThreadEnsureOptions::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        thread_metadata_from_value(&created)
            .get("agent_team_child")
            .cloned(),
        Some(json!({
            "team_id": "product-ship-camel",
            "child_agent_id": "planner"
        }))
    );

    let stored = store.get(&thread_id).await.unwrap();
    assert_eq!(
        thread_metadata_from_value(&stored)
            .get("agent_team_child")
            .cloned(),
        Some(json!({
            "team_id": "product-ship-camel",
            "child_agent_id": "planner"
        }))
    );
}

#[test]
fn default_workspace_for_channel_account_returns_bot_workspace() {
    let mut config = garyx_models::config::GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/bot-workspace".to_owned()),
                    owner_target: None,
                    groups: HashMap::new(),
                },
            ),
        );

    assert_eq!(
        default_workspace_for_channel_account(&config, "telegram", "main").as_deref(),
        Some("/tmp/bot-workspace")
    );
}

#[test]
fn default_workspace_for_channel_account_returns_none_without_bot_workspace() {
    let mut config = garyx_models::config::GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: None,
                    owner_target: None,
                    groups: HashMap::new(),
                },
            ),
        );

    assert!(default_workspace_for_channel_account(&config, "telegram", "main").is_none());
}

#[test]
fn automation_threads_are_not_hidden_by_default() {
    let value = json!({
        "thread_id": "thread::automation"
    });

    assert!(!is_hidden_thread_value(&value));
}

#[test]
fn explicit_hidden_flag_marks_thread_hidden() {
    let value = json!({
        "thread_id": "thread::hidden",
        "hidden": true
    });

    assert!(is_hidden_thread_value(&value));
}
