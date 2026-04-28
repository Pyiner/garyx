use super::*;
use garyx_models::ThreadHistoryBackend;
use garyx_router::{InMemoryThreadStore, ThreadHistoryRepository, ThreadTranscriptStore};
use serde_json::json;

fn make_history(store: Arc<dyn ThreadStore>) -> Arc<ThreadHistoryRepository> {
    Arc::new(ThreadHistoryRepository::new(
        store,
        Arc::new(ThreadTranscriptStore::memory()),
        ThreadHistoryBackend::TranscriptV1,
    ))
}

#[tokio::test]
async fn test_save_thread_messages_preserves_provider_message_order() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let session_messages = vec![
        ProviderMessage::assistant_text("在。先执行 ls。"),
        ProviderMessage::tool_use(
            json!({"tool": "Bash", "input": {"command": "ls"}}),
            None,
            Some("Bash".to_owned()),
        ),
        ProviderMessage::tool_result(
            json!({"result": "a\nb\n", "text": "a\nb\n"}),
            None,
            Some("Bash".to_owned()),
            Some(false),
        ),
        ProviderMessage::assistant_text("\n结果如下。"),
    ];

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::ordered",
            user_message: "和我说话 然后执行 ls",
            user_images: &[],
            assistant_response: "在。先执行 ls。\n结果如下。",
            sdk_session_id: Some("sdk-1"),
            provider_key: "provider::ordered",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &session_messages,
            metadata: &HashMap::new(),
        },
    )
    .await;

    let stored = store
        .get("thread::ordered")
        .await
        .expect("stored session should exist");
    assert_eq!(stored["provider_key"], "provider::ordered");
    assert_eq!(
        stored["provider_sdk_session_ids"]["provider::ordered"],
        "sdk-1"
    );
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    let roles: Vec<&str> = messages
        .iter()
        .filter_map(|entry| entry.get("role").and_then(Value::as_str))
        .collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "tool_use", "tool_result", "assistant"]
    );
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[1]["content"], "在。先执行 ls。");
    assert_eq!(messages[4]["content"], "\n结果如下。");
}

#[tokio::test]
async fn test_save_thread_messages_persists_user_images_as_blocks() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let user_images = vec![ImagePayload {
        name: "diagram.png".to_owned(),
        data: "abc123==".to_owned(),
        media_type: "image/png".to_owned(),
    }];

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::image",
            user_message: "describe this",
            user_images: &user_images,
            assistant_response: "Looks like a diagram.",
            sdk_session_id: None,
            provider_key: "provider::image",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &HashMap::new(),
        },
    )
    .await;

    let stored = store
        .get("thread::image")
        .await
        .expect("stored session should exist");
    assert_eq!(stored["provider_key"], "provider::image");
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    let user = messages[0].as_object().expect("user message object");
    let content = user
        .get("content")
        .and_then(Value::as_array)
        .expect("user content blocks");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "describe this");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["media_type"], "image/png");
    assert_eq!(content[1]["source"]["data"], "abc123==");
}

#[test]
fn test_streaming_run_snapshot_splits_assistant_segments() {
    let mut snapshot = StreamingRunSnapshot::default();
    assert!(snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "alpha".to_owned(),
    }));
    assert!(!snapshot.apply_stream_event(&StreamEvent::Boundary {
        kind: garyx_models::provider::StreamBoundaryKind::AssistantSegment,
        pending_input_id: None,
    }));
    assert!(snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "beta".to_owned(),
    }));

    assert_eq!(snapshot.assistant_response, "alpha\n\nbeta");
    assert_eq!(snapshot.session_messages.len(), 2);
    assert_eq!(
        snapshot.session_messages[0].role,
        ProviderMessageRole::Assistant
    );
    assert_eq!(snapshot.session_messages[0].text.as_deref(), Some("alpha"));
    assert_eq!(snapshot.session_messages[1].text.as_deref(), Some("beta"));
}

#[test]
fn test_streaming_run_snapshot_strips_agent_prefix_into_metadata() {
    let mut snapshot = StreamingRunSnapshot::default();
    assert!(snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "[claude] hello team".to_owned(),
    }));

    assert_eq!(snapshot.assistant_response, "hello team");
    assert_eq!(snapshot.session_messages.len(), 1);
    assert_eq!(
        snapshot.session_messages[0].text.as_deref(),
        Some("hello team")
    );
    assert_eq!(
        snapshot.session_messages[0].metadata.get("agent_id"),
        Some(&json!("claude"))
    );
    assert_eq!(
        snapshot.session_messages[0]
            .metadata
            .get("agent_display_name"),
        Some(&json!("claude"))
    );
}

#[test]
fn test_streaming_run_snapshot_splits_on_agent_prefix_change_without_boundary() {
    let mut snapshot = StreamingRunSnapshot::default();
    assert!(snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "[claude] hello".to_owned(),
    }));
    assert!(snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "[codex] hi back".to_owned(),
    }));

    assert_eq!(snapshot.assistant_response, "hello\n\nhi back");
    assert_eq!(snapshot.session_messages.len(), 2);
    assert_eq!(
        snapshot.session_messages[0].metadata.get("agent_id"),
        Some(&json!("claude"))
    );
    assert_eq!(
        snapshot.session_messages[1].metadata.get("agent_id"),
        Some(&json!("codex"))
    );
    assert_eq!(snapshot.session_messages[0].text.as_deref(), Some("hello"));
    assert_eq!(
        snapshot.session_messages[1].text.as_deref(),
        Some("hi back")
    );
}

#[tokio::test]
async fn test_save_partial_thread_messages_replaces_existing_snapshot_for_same_run() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    store
        .set(
            "thread::partial",
            json!({
                "sdk_session_id": "sdk-existing",
                "provider_key": "provider::partial",
                "provider_sdk_session_ids": {
                    "provider::partial": "sdk-existing"
                },
                "messages": [{
                    "role": "assistant",
                    "content": "older run",
                    "metadata": {
                        "client_run_id": "run-older"
                    }
                }]
            }),
        )
        .await;

    let mut metadata = HashMap::new();
    metadata.insert("client_run_id".to_owned(), json!("run-partial"));
    metadata.insert("bridge_run_id".to_owned(), json!("bridge-partial"));

    let mut snapshot = StreamingRunSnapshot::default();
    snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "hel".to_owned(),
    });
    save_partial_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial",
            user_message: "hello",
            user_images: &[],
            assistant_response: &snapshot.assistant_response,
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &snapshot.session_messages,
            metadata: &metadata,
        },
        &[],
    )
    .await;

    snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "lo".to_owned(),
    });
    save_partial_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial",
            user_message: "hello",
            user_images: &[],
            assistant_response: &snapshot.assistant_response,
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &snapshot.session_messages,
            metadata: &metadata,
        },
        &[],
    )
    .await;

    let stored = store
        .get("thread::partial")
        .await
        .expect("stored session should exist");
    assert_eq!(stored["sdk_session_id"], "sdk-existing");
    assert_eq!(
        stored["provider_sdk_session_ids"]["provider::partial"],
        "sdk-existing"
    );
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "older run");
    let active_messages = stored["history"]["active_run_snapshot"]["messages"]
        .as_array()
        .expect("active snapshot messages should be an array");
    assert_eq!(active_messages.len(), 2);
    assert_eq!(active_messages[0]["role"], "user");
    assert_eq!(active_messages[1]["role"], "assistant");
    assert_eq!(active_messages[1]["content"], "hello");
}

#[tokio::test]
async fn test_save_partial_thread_messages_clears_abandoned_pending_inputs_for_new_user_turn() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    store
        .set(
            "thread::partial-clear-orphaned",
            json!({
                "pending_user_inputs": [
                    {
                        "id": "stale-abandoned",
                        "bridge_run_id": "run-old",
                        "text": "old follow-up",
                        "content": [{"type": "text", "text": "old follow-up"}],
                        "queued_at": "2026-03-01T00:00:00Z",
                        "status": "abandoned"
                    },
                    {
                        "id": "still-queued",
                        "bridge_run_id": "run-other",
                        "text": "still active elsewhere",
                        "content": [{"type": "text", "text": "still active elsewhere"}],
                        "queued_at": "2026-03-01T00:00:01Z",
                        "status": "queued"
                    }
                ]
            }),
        )
        .await;

    let metadata = HashMap::from([
        ("client_run_id".to_owned(), json!("run-new")),
        ("bridge_run_id".to_owned(), json!("bridge-new")),
    ]);

    save_partial_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial-clear-orphaned",
            user_message: "fresh turn",
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
        &[],
    )
    .await;

    let stored = store
        .get("thread::partial-clear-orphaned")
        .await
        .expect("stored session should exist");
    let pending_inputs = stored["pending_user_inputs"]
        .as_array()
        .expect("pending inputs should be an array");
    assert_eq!(pending_inputs.len(), 1);
    assert_eq!(pending_inputs[0]["id"], "still-queued");
}

#[tokio::test]
async fn test_save_partial_thread_messages_keeps_abandoned_pending_inputs_for_internal_dispatch() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    store
        .set(
            "thread::partial-keep-orphaned",
            json!({
                "pending_user_inputs": [
                    {
                        "id": "stale-abandoned",
                        "bridge_run_id": "run-old",
                        "text": "old follow-up",
                        "content": [{"type": "text", "text": "old follow-up"}],
                        "queued_at": "2026-03-01T00:00:00Z",
                        "status": "abandoned"
                    }
                ]
            }),
        )
        .await;

    let metadata = HashMap::from([
        ("client_run_id".to_owned(), json!("run-loop")),
        ("bridge_run_id".to_owned(), json!("bridge-loop")),
        ("internal_dispatch".to_owned(), Value::Bool(true)),
        (
            "internal_kind".to_owned(),
            Value::String("loop_continuation".to_owned()),
        ),
    ]);

    save_partial_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial-keep-orphaned",
            user_message: "continue working",
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
        &[],
    )
    .await;

    let stored = store
        .get("thread::partial-keep-orphaned")
        .await
        .expect("stored session should exist");
    let pending_inputs = stored["pending_user_inputs"]
        .as_array()
        .expect("pending inputs should be an array");
    assert_eq!(pending_inputs.len(), 1);
    assert_eq!(pending_inputs[0]["id"], "stale-abandoned");
}

#[tokio::test]
async fn test_save_thread_messages_clears_only_current_provider_sdk_session_id() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    store
        .set(
            "thread::provider-sessions",
            json!({
                "sdk_session_id": "sdk-legacy",
                "provider_key": "provider::ordered",
                "provider_sdk_session_ids": {
                    "provider::ordered": "sdk-ordered",
                    "provider::other": "sdk-other"
                },
            }),
        )
        .await;

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::provider-sessions",
            user_message: "clear ordered session",
            user_images: &[],
            assistant_response: "done",
            sdk_session_id: None,
            provider_key: "provider::ordered",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &HashMap::new(),
        },
    )
    .await;

    let stored = store
        .get("thread::provider-sessions")
        .await
        .expect("stored session should exist");
    assert_eq!(
        stored["provider_sdk_session_ids"]["provider::other"],
        "sdk-other"
    );
    assert!(
        stored["provider_sdk_session_ids"]
            .get("provider::ordered")
            .is_none()
    );
    assert!(stored.get("sdk_session_id").is_none());
}

#[tokio::test]
async fn test_save_thread_messages_synthesizes_message_tool_delivery_as_assistant_reply() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let session_messages = vec![
        ProviderMessage::tool_use(
            json!({
                "tool": "message",
                "input": {
                    "text": "已经发到 Telegram 了"
                }
            }),
            Some("tool-message-1".to_owned()),
            Some("mcp:gary:message".to_owned()),
        ),
        ProviderMessage::tool_result(
            json!({
                "result": {
                    "tool": "message",
                    "action": "send",
                    "status": "ok",
                    "text": "已经发到 Telegram 了"
                }
            }),
            Some("tool-message-1".to_owned()),
            Some("mcp:gary:message".to_owned()),
            Some(false),
        ),
    ];

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::delivery-mirror",
            user_message: "同步到 bot",
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::delivery",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &session_messages,
            metadata: &HashMap::new(),
        },
    )
    .await;

    let stored = store
        .get("thread::delivery-mirror")
        .await
        .expect("stored session should exist");
    assert_eq!(stored["provider_key"], "provider::delivery");
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    let roles: Vec<&str> = messages
        .iter()
        .filter_map(|entry| entry.get("role").and_then(Value::as_str))
        .collect();
    assert_eq!(roles, vec!["user", "tool_use", "tool_result", "assistant"]);
    assert_eq!(messages[3]["content"], "已经发到 Telegram 了");
    assert_eq!(messages[3]["metadata"]["delivery_mirror"], true);
    assert_eq!(messages[3]["metadata"]["delivery_source"], "message_tool");
}

#[tokio::test]
async fn test_save_thread_messages_does_not_synthesize_delivery_when_assistant_exists() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let session_messages = vec![
        ProviderMessage::tool_use(
            json!({
                "tool": "message",
                "input": {
                    "text": "已经发到 Telegram 了"
                }
            }),
            Some("tool-message-1".to_owned()),
            Some("message".to_owned()),
        ),
        ProviderMessage::tool_result(
            json!({
                "result": {
                    "tool": "message",
                    "action": "send",
                    "status": "ok",
                    "text": "已经发到 Telegram 了"
                }
            }),
            Some("tool-message-1".to_owned()),
            Some("message".to_owned()),
            Some(false),
        ),
        ProviderMessage::assistant_text("app 里也要看到这句"),
    ];

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::explicit-assistant",
            user_message: "同步到 bot",
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::explicit-assistant",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &session_messages,
            metadata: &HashMap::new(),
        },
    )
    .await;

    let stored = store
        .get("thread::explicit-assistant")
        .await
        .expect("stored session should exist");
    assert_eq!(stored["provider_key"], "provider::explicit-assistant");
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    let assistant_messages: Vec<&Value> = messages
        .iter()
        .filter(|entry| entry.get("role").and_then(Value::as_str) == Some("assistant"))
        .collect();
    assert_eq!(assistant_messages.len(), 1);
    assert_eq!(assistant_messages[0]["content"], "app 里也要看到这句");
}

#[tokio::test]
async fn test_save_thread_messages_marks_loop_continuation_as_internal() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let metadata = HashMap::from([
        ("internal_dispatch".to_owned(), Value::Bool(true)),
        ("loop_continuation".to_owned(), Value::Bool(true)),
        (
            "internal_kind".to_owned(),
            Value::String("loop_continuation".to_owned()),
        ),
        (
            "loop_origin".to_owned(),
            Value::String("auto_continue".to_owned()),
        ),
    ]);

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::loop-internal",
            user_message: "The user wants you to continue working.",
            user_images: &[],
            assistant_response: "当前没有剩余代码任务。",
            sdk_session_id: None,
            provider_key: "provider::loop",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
    )
    .await;

    let stored = store
        .get("thread::loop-internal")
        .await
        .expect("stored thread should exist");
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["internal"], true);
    assert_eq!(messages[0]["internal_kind"], "loop_continuation");
    assert_eq!(messages[0]["loop_origin"], "auto_continue");
    assert_eq!(messages[1]["internal"], true);
    assert_eq!(messages[1]["internal_kind"], "loop_continuation");
    assert_eq!(messages[1]["loop_origin"], "auto_continue");
}
