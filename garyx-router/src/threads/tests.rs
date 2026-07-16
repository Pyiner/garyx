use super::*;

#[test]
fn internal_api_binding_remains_compatible_with_external_thread() {
    let thread = json!({
        "thread_id": "thread::external",
        "channel": "telegram",
        "account_id": "main",
        "channel_bindings": [{
            "channel": "telegram",
            "account_id": "main",
            "binding_key": "1000000001",
            "chat_id": "1000000001"
        }]
    });

    assert!(
        validate_thread_accepts_bot_binding("thread::external", &thread, "api", "main").is_ok()
    );
}
use crate::memory_store::InMemoryThreadStore;
use serde_json::json;

#[tokio::test]
async fn sync_endpoint_delivery_timestamp_point_updates_registry_and_holder() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let binding = json!({
        "channel": "telegram",
        "account_id": "main",
        "binding_key": "42",
        "chat_id": "42",
        "display_label": "User 42",
        "last_delivery_at": "2026-03-07T09:00:00Z"
    });
    store
        .set(
            KNOWN_CHANNEL_ENDPOINTS_KEY,
            json!({"channel_bindings": [binding]}),
        )
        .await
        .unwrap();
    store
        .set(
            "thread::holder",
            json!({"thread_id": "thread::holder", "channel_bindings": [binding]}),
        )
        .await
        .unwrap();
    // A drifted stale copy on another thread: point sync intentionally does
    // not chase it — the steady-state invariant is one holder per endpoint,
    // and drift cleanup is not a delivery-path job.
    store
        .set(
            "thread::stale",
            json!({"thread_id": "thread::stale", "channel_bindings": [binding]}),
        )
        .await
        .unwrap();

    sync_endpoint_delivery_timestamp(
        &store,
        "telegram",
        "main",
        "42",
        Some("2026-03-07T10:30:00Z"),
        "thread::holder",
    )
    .await
    .expect("sync succeeds");

    let delivery_at = |value: &Value| {
        bindings_from_value(value)
            .into_iter()
            .next()
            .and_then(|binding| binding.last_delivery_at)
    };
    let registry = store
        .get(KNOWN_CHANNEL_ENDPOINTS_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        delivery_at(&registry).as_deref(),
        Some("2026-03-07T10:30:00Z")
    );
    let holder = store.get("thread::holder").await.unwrap().unwrap();
    assert_eq!(
        delivery_at(&holder).as_deref(),
        Some("2026-03-07T10:30:00Z")
    );
    let stale = store.get("thread::stale").await.unwrap().unwrap();
    assert_eq!(
        delivery_at(&stale).as_deref(),
        Some("2026-03-07T09:00:00Z"),
        "point sync must not walk unrelated records"
    );
}

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
        .await
        .unwrap();

    let endpoints = list_known_channel_endpoints(&store)
        .await
        .expect("list endpoints");
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
async fn list_known_channel_endpoints_backfills_delivery_target_from_binding_key() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::legacy",
            json!({
                "thread_id": "thread::legacy",
                "label": "Legacy",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "1000000001",
                    "chat_id": "",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "",
                    "display_label": "Test User"
                }]
            }),
        )
        .await
        .unwrap();

    let endpoints = list_known_channel_endpoints(&store)
        .await
        .expect("list endpoints");
    assert_eq!(endpoints.len(), 1);
    let endpoint = &endpoints[0];
    assert_eq!(endpoint.binding_key, "1000000001");
    assert_eq!(endpoint.chat_id, "");
    assert_eq!(endpoint.delivery_target_type, "chat_id");
    assert_eq!(endpoint.delivery_target_id, "1000000001");
}

#[tokio::test]
async fn list_known_channel_endpoints_orders_by_endpoint_key_not_activity() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    for (thread_id, binding_key, label, updated_at) in [
        ("thread::z-room", "z-room", "Z Room", "2026-03-07T12:00:00Z"),
        ("thread::a-room", "a-room", "A Room", "2026-03-07T10:00:00Z"),
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
                        "binding_key": binding_key,
                        "chat_id": binding_key,
                        "display_label": label,
                        "last_inbound_at": updated_at
                    }]
                }),
            )
            .await
            .unwrap();
    }

    let endpoints = list_known_channel_endpoints(&store)
        .await
        .expect("list endpoints");
    let endpoint_keys: Vec<_> = endpoints
        .iter()
        .map(|endpoint| endpoint.endpoint_key.as_str())
        .collect();

    assert_eq!(
        endpoint_keys,
        vec!["telegram::main::a-room", "telegram::main::z-room"]
    );
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
        .await
        .unwrap();
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
        .await
        .unwrap();

    let endpoints = list_known_channel_endpoints(&store)
        .await
        .expect("list endpoints");
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
        .await
        .unwrap();

    let updated = update_thread_record(&store, "thread::keep", Some("After".to_owned()), None)
        .await
        .unwrap();

    assert_eq!(label_from_value(&updated).as_deref(), Some("After"));
    assert_eq!(updated["thread_title_source"], "explicit");
    assert_eq!(
        workspace_dir_from_value(&updated).as_deref(),
        Some("/tmp/workspace-a")
    );
}

#[tokio::test]
async fn update_thread_record_explicit_label_clears_provider_title() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::title",
            json!({
                "thread_id": "thread::title",
                "label": "Provider Title",
                "thread_title_source": "provider",
                "provider_thread_title": "Provider Title"
            }),
        )
        .await
        .unwrap();

    let updated = update_thread_record(
        &store,
        "thread::title",
        Some("Human Title".to_owned()),
        None,
    )
    .await
    .unwrap();

    assert_eq!(label_from_value(&updated).as_deref(), Some("Human Title"));
    assert_eq!(updated["thread_title_source"], "explicit");
    assert!(updated.get("provider_thread_title").is_none());
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
        .await
        .unwrap();

    let error = update_thread_record(&store, "thread::clear", None, Some("   ".to_owned()))
        .await
        .unwrap_err();

    assert!(error.contains("workspace_dir is immutable"));
    let stored = store.get("thread::clear").await.unwrap().unwrap();
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
        .await
        .unwrap();

    let error = update_thread_record(
        &store,
        "thread::move",
        None,
        Some("/tmp/workspace-b".to_owned()),
    )
    .await
    .unwrap_err();

    assert!(error.contains("workspace_dir is immutable"));
    let stored = store.get("thread::move").await.unwrap().unwrap();
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
        .await
        .unwrap();

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
        .await
        .unwrap();

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
                "custom_context".to_owned(),
                json!({
                    "run_id": "run-camel",
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
            .get("custom_context")
            .cloned(),
        Some(json!({
            "run_id": "run-camel",
            "child_agent_id": "planner"
        }))
    );

    let stored = store.get(&thread_id).await.unwrap().unwrap();
    assert_eq!(
        thread_metadata_from_value(&stored)
            .get("custom_context")
            .cloned(),
        Some(json!({
            "run_id": "run-camel",
            "child_agent_id": "planner"
        }))
    );
}

#[tokio::test]
async fn create_thread_record_mirrors_hidden_metadata_to_top_level() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let (_thread_id, created) = create_thread_record(
        &store,
        ThreadEnsureOptions {
            metadata: HashMap::from([("hidden".to_owned(), json!(true))]),
            ..ThreadEnsureOptions::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(created.get("hidden"), Some(&json!(true)));
    assert!(is_hidden_thread_value(&created));
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
                    agent_id: Some("claude".to_owned()),
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
                    agent_id: Some("claude".to_owned()),
                    workspace_dir: None,
                    owner_target: None,
                    groups: HashMap::new(),
                },
            ),
        );

    assert!(default_workspace_for_channel_account(&config, "telegram", "main").is_none());
}

#[test]
fn default_workspace_mode_for_channel_account_returns_bot_mode() {
    let mut config = garyx_models::config::GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::PluginAccountEntry {
                workspace_mode: Some("worktree".to_owned()),
                ..Default::default()
            },
        );
    config.channels.api.accounts.insert(
        "scripted".to_owned(),
        garyx_models::config::ApiAccount {
            workspace_mode: Some("local".to_owned()),
            ..Default::default()
        },
    );

    assert_eq!(
        default_workspace_mode_for_channel_account(&config, "telegram", "main"),
        WorkspaceMode::Worktree
    );
    assert_eq!(
        default_workspace_mode_for_channel_account(&config, "api", "scripted"),
        WorkspaceMode::Local
    );
    assert_eq!(
        default_workspace_mode_for_channel_account(&config, "telegram", "missing"),
        WorkspaceMode::Local
    );
}

#[test]
fn workspace_mode_serializes_public_local_name() {
    assert_eq!(
        serde_json::to_value(WorkspaceMode::Local).unwrap(),
        json!("local")
    );
    assert_eq!(
        serde_json::to_value(WorkspaceMode::Worktree).unwrap(),
        json!("worktree")
    );
    assert_eq!(
        serde_json::from_value::<WorkspaceMode>(json!("local")).unwrap(),
        WorkspaceMode::Local
    );
    // Backward-compat: `"direct"` was the pre-rename serde value for Local.
    assert_eq!(
        serde_json::from_value::<WorkspaceMode>(json!("direct")).unwrap(),
        WorkspaceMode::Local
    );
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
