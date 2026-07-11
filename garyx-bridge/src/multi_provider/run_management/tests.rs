use super::*;
use garyx_router::InMemoryThreadStore;

#[test]
fn task_work_run_wake_excludes_notification_internal_and_system_runs() {
    assert!(is_task_work_run_wake("run-1", &HashMap::new()));
    assert!(!is_task_work_run_wake("task-notify-42", &HashMap::new()));
    assert!(!is_task_work_run_wake(
        "run-1",
        &HashMap::from([("task_notification".to_owned(), json!(true))])
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
