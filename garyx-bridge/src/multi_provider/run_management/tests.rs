use super::*;
use garyx_router::InMemoryThreadStore;

#[test]
fn queued_dispatch_metadata_persists_full_semantics_but_no_runtime_wiring() {
    let mut metadata = HashMap::from([
        ("source".to_owned(), json!("task_notification")),
        ("internal_dispatch".to_owned(), json!(true)),
        (
            "task_notification".to_owned(),
            json!({
                "event": "ready_for_review",
                "status": "in_review",
                "task_id": "#TASK-42",
                "title": "Synthetic review"
            }),
        ),
        ("custom_semantic_key".to_owned(), json!({"nested": true})),
    ]);
    for key in super::super::persistence::RUNTIME_ONLY_METADATA_KEYS {
        metadata.insert((*key).to_owned(), json!(format!("sentinel-{key}")));
    }

    let projected = queued_dispatch_metadata(&metadata, "run-requested");
    let persisted = pending_input_metadata_for_persistence(projected);

    assert_eq!(persisted.get("source"), Some(&json!("task_notification")));
    assert_eq!(persisted.get("internal_dispatch"), Some(&json!(true)));
    assert_eq!(
        persisted
            .get("task_notification")
            .and_then(|value| value.get("task_id")),
        Some(&json!("#TASK-42"))
    );
    assert_eq!(
        persisted.get("custom_semantic_key"),
        Some(&json!({"nested": true}))
    );
    assert_eq!(
        persisted.get("origin_run_id"),
        Some(&json!("run-requested"))
    );
    for key in super::super::persistence::RUNTIME_ONLY_METADATA_KEYS {
        assert!(!persisted.contains_key(*key), "runtime key {key} persisted");
    }
}

#[test]
fn queued_dispatch_metadata_keeps_every_internal_source_family() {
    let metadata = HashMap::from([
        ("source".to_owned(), json!("automation")),
        ("automation_id".to_owned(), json!("automation-1")),
        ("cron_job_id".to_owned(), json!("cron-1")),
        ("cron_action".to_owned(), json!("run")),
        ("task_auto_start".to_owned(), json!(true)),
        ("task_dispatch_reason".to_owned(), json!("created")),
        ("restart_wake".to_owned(), json!(true)),
        ("restart_wake_id".to_owned(), json!("wake-1")),
    ]);

    let persisted = pending_input_metadata_for_persistence(queued_dispatch_metadata(
        &metadata,
        "run-requested",
    ));

    for (key, value) in metadata {
        assert_eq!(persisted.get(&key), Some(&value), "lost source key {key}");
    }
}

#[test]
fn task_work_run_wake_excludes_notification_internal_and_system_runs() {
    assert!(is_task_work_run_wake("run-1", &HashMap::new()));
    assert!(!is_task_work_run_wake("task-notify-42", &HashMap::new()));
    assert!(!is_task_work_run_wake(
        "run-1",
        &HashMap::from([(
            "task_notification".to_owned(),
            json!({
                "event": "ready_for_review",
                "status": "in_review",
                "task_id": "#TASK-42",
                "title": "Synthetic review"
            })
        )])
    ));
    assert!(!is_task_work_run_wake(
        "run-1",
        &HashMap::from([("internal_dispatch".to_owned(), json!(true))])
    ));
    assert!(!is_task_work_run_wake(
        "run-1",
        &HashMap::from([("system".to_owned(), json!(true))])
    ));
}

#[tokio::test]
async fn provider_thread_title_replaces_prompt_fallback_label() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::title",
            json!({
                "thread_id": "thread::title",
                "label": "Please investigate why Codex events do not show titles",
                "thread_title_source": "garyx_prompt"
            }),
        )
        .await
        .unwrap();

    let applied = persist_provider_thread_title_if_missing(
        &store,
        "thread::title",
        Some("Provider Generated Title"),
    )
    .await;

    assert_eq!(applied.as_deref(), Some("Provider Generated Title"));
    let updated = store
        .get("thread::title")
        .await
        .unwrap()
        .expect("thread exists");
    assert_eq!(updated["label"], "Provider Generated Title");
    assert_eq!(updated["thread_title_source"], "provider");
    assert_eq!(updated["provider_thread_title"], "Provider Generated Title");
}

#[tokio::test]
async fn provider_thread_title_does_not_replace_explicit_label() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::explicit",
            json!({
                "thread_id": "thread::explicit",
                "label": "Human Label"
            }),
        )
        .await
        .unwrap();

    let applied = persist_provider_thread_title_if_missing(
        &store,
        "thread::explicit",
        Some("Provider Generated Title"),
    )
    .await;

    assert!(applied.is_none());
    let updated = store
        .get("thread::explicit")
        .await
        .unwrap()
        .expect("thread exists");
    assert_eq!(updated["label"], "Human Label");
    assert!(updated.get("provider_thread_title").is_none());
}

#[tokio::test]
async fn provider_thread_title_does_not_replace_task_label() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::task",
            json!({
                "thread_id": "thread::task",
                "label": "Polish roguelike copy in English after FOV",
                "metadata": {
                    "task_id": "#TASK-33"
                }
            }),
        )
        .await
        .unwrap();

    let applied = persist_provider_thread_title_if_missing(
        &store,
        "thread::task",
        Some("Provider Generated Title"),
    )
    .await;

    assert!(applied.is_none());
    let updated = store
        .get("thread::task")
        .await
        .unwrap()
        .expect("thread exists");
    assert_eq!(
        updated["label"],
        "Polish roguelike copy in English after FOV"
    );
    assert!(updated.get("provider_thread_title").is_none());
}

#[tokio::test]
async fn provider_thread_title_does_not_replace_task_managed_label() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    store
        .set(
            "thread::task-managed",
            json!({
                "thread_id": "thread::task-managed",
                "label": "#TASK-33 Ship thread title",
                "thread_title_source": "task"
            }),
        )
        .await
        .unwrap();

    let applied = persist_provider_thread_title_if_missing(
        &store,
        "thread::task-managed",
        Some("Provider Generated Title"),
    )
    .await;

    assert!(applied.is_none());
    let updated = store
        .get("thread::task-managed")
        .await
        .unwrap()
        .expect("thread exists");
    assert_eq!(updated["label"], "#TASK-33 Ship thread title");
    assert_eq!(updated["thread_title_source"], "task");
    assert!(updated.get("provider_thread_title").is_none());
}

#[test]
fn test_resolve_sdk_session_id_for_persistence_prefers_non_empty_result() {
    let mut metadata = HashMap::new();
    metadata.insert("sdk_session_id".to_owned(), json!("persisted-session"));

    let resolved = resolve_sdk_session_id_for_persistence(&metadata, Some("new-session"));

    assert_eq!(resolved.as_deref(), Some("new-session"));
}

#[test]
fn test_resolve_sdk_session_id_for_persistence_falls_back_to_metadata() {
    let mut metadata = HashMap::new();
    metadata.insert("sdk_session_id".to_owned(), json!("persisted-session"));

    let resolved = resolve_sdk_session_id_for_persistence(&metadata, Some("   "));

    assert_eq!(resolved.as_deref(), Some("persisted-session"));
}

#[test]
fn test_resolve_sdk_session_id_for_persistence_ignores_empty_values() {
    let mut metadata = HashMap::new();
    metadata.insert("sdk_session_id".to_owned(), json!("   "));

    let resolved = resolve_sdk_session_id_for_persistence(&metadata, None);

    assert!(resolved.is_none());
}

#[test]
fn test_resolve_persisted_sdk_session_id_for_provider_prefers_provider_scoped_value() {
    let session_data = json!({
        "provider_key": "claude",
        "sdk_session_id": "legacy-session",
        "provider_sdk_session_ids": {
            "claude": "claude-session",
            "codex": "codex-thread"
        }
    });

    let resolved = resolve_persisted_sdk_session_id_for_provider(&session_data, "claude", None);

    assert_eq!(resolved.as_deref(), Some("claude-session"));
}

#[test]
fn test_resolve_persisted_sdk_session_id_for_provider_falls_back_to_matching_legacy_value() {
    let session_data = json!({
        "provider_key": "claude",
        "sdk_session_id": "legacy-session",
    });

    let resolved = resolve_persisted_sdk_session_id_for_provider(&session_data, "claude", None);

    assert_eq!(resolved.as_deref(), Some("legacy-session"));
}

#[test]
fn test_resolve_persisted_sdk_session_id_for_provider_ignores_other_provider_legacy_value() {
    let session_data = json!({
        "provider_key": "codex",
        "sdk_session_id": "codex-thread",
    });

    let resolved = resolve_persisted_sdk_session_id_for_provider(&session_data, "claude", None);

    assert!(resolved.is_none());
}
