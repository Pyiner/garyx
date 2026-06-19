use super::*;
use garyx_models::provider::ProviderMessage;
use garyx_router::InMemoryThreadStore;

#[test]
fn task_work_run_wake_excludes_notification_internal_system_and_workflow_runs() {
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
    assert!(!is_task_work_run_wake(
        "run-1",
        &HashMap::from([("workflow_child_run_id".to_owned(), json!("child-1"))])
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
        .await;

    let applied = persist_provider_thread_title_if_missing(
        &store,
        "thread::title",
        Some("Provider Generated Title"),
    )
    .await;

    assert_eq!(applied.as_deref(), Some("Provider Generated Title"));
    let updated = store.get("thread::title").await.expect("thread exists");
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
        .await;

    let applied = persist_provider_thread_title_if_missing(
        &store,
        "thread::explicit",
        Some("Provider Generated Title"),
    )
    .await;

    assert!(applied.is_none());
    let updated = store.get("thread::explicit").await.expect("thread exists");
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
        .await;

    let applied = persist_provider_thread_title_if_missing(
        &store,
        "thread::task",
        Some("Provider Generated Title"),
    )
    .await;

    assert!(applied.is_none());
    let updated = store.get("thread::task").await.expect("thread exists");
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
        .await;

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
        .expect("thread exists");
    assert_eq!(updated["label"], "#TASK-33 Ship thread title");
    assert_eq!(updated["thread_title_source"], "task");
    assert!(updated.get("provider_thread_title").is_none());
}

#[test]
fn native_session_messages_are_attached_from_committed_thread_messages() {
    let session_data = json!({
        "messages": [
            ProviderMessage::user_text("previous question").to_json_value(),
            ProviderMessage::assistant_text("previous answer").to_json_value()
        ]
    });
    let mut options = ProviderRunOptions {
        thread_id: "thread::native".to_owned(),
        message: "next".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    attach_native_session_messages(&mut options, &session_data, &ProviderType::ClaudeLlm);

    let messages: Vec<ProviderMessage> = serde_json::from_value(
        options
            .metadata
            .get(SESSION_MESSAGES_METADATA_KEY)
            .cloned()
            .expect("session messages metadata"),
    )
    .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].text.as_deref(), Some("previous question"));
    assert_eq!(messages[1].text.as_deref(), Some("previous answer"));
}

#[test]
fn native_session_messages_are_attached_for_all_native_model_backends() {
    let session_data = json!({
        "messages": [ProviderMessage::assistant_text("previous answer").to_json_value()]
    });

    for provider_type in [
        ProviderType::Gpt,
        ProviderType::ClaudeLlm,
        ProviderType::GeminiLlm,
    ] {
        let mut options = ProviderRunOptions {
            thread_id: "thread::native".to_owned(),
            message: "next".to_owned(),
            workspace_dir: None,
            images: None,
            metadata: HashMap::new(),
        };

        attach_native_session_messages(&mut options, &session_data, &provider_type);

        assert!(
            options.metadata.contains_key(SESSION_MESSAGES_METADATA_KEY),
            "missing native session replay for {provider_type:?}"
        );
    }
}

#[test]
fn native_session_messages_are_not_attached_for_other_providers() {
    let session_data = json!({
        "messages": [ProviderMessage::assistant_text("previous answer").to_json_value()]
    });
    let mut options = ProviderRunOptions {
        thread_id: "thread::claude".to_owned(),
        message: "next".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    attach_native_session_messages(&mut options, &session_data, &ProviderType::ClaudeCode);

    assert!(!options.metadata.contains_key(SESSION_MESSAGES_METADATA_KEY));
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

#[test]
fn persisted_provider_messages_reads_committed_cache_only() {
    // Native provider resume state comes from the bounded committed cache; live
    // run recovery is driven by committed transcript controls elsewhere.
    let session_data = serde_json::json!({
        "messages": [
            {"role": "user", "content": "q1"},
            {"role": "assistant", "content": "a1"}
        ]
    });
    let messages = persisted_provider_messages_from_thread(&session_data);
    assert_eq!(
        messages.len(),
        2,
        "resume should use the committed cache without side-channel tails"
    );

    let empty = serde_json::json!({});
    assert!(persisted_provider_messages_from_thread(&empty).is_empty());
}
