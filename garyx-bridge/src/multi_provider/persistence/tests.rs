use super::*;
use garyx_router::{InMemoryThreadStore, ThreadHistoryRepository, ThreadTranscriptStore};
use serde_json::json;

fn make_history(store: Arc<dyn ThreadStore>) -> Arc<ThreadHistoryRepository> {
    Arc::new(ThreadHistoryRepository::new(
        store,
        Arc::new(ThreadTranscriptStore::memory()),
    ))
}

fn fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("test-fixtures")
}

fn load_capsule_provider_fixture(name: &str) -> Value {
    let path = fixture_root().join("capsules/provider-results").join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn provider_message_from_fixture(fixture: &Value, key: &str) -> ProviderMessage {
    serde_json::from_value(
        fixture
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("fixture missing {key}")),
    )
    .unwrap_or_else(|error| panic!("decode {key}: {error}"))
}

#[test]
fn test_extract_capsule_attachment_from_claude_fixture_correlates_tool_use_id() {
    let fixture = load_capsule_provider_fixture("claude-capsule-create.json");
    let tool_use = provider_message_from_fixture(&fixture, "tool_use");
    let tool_result = provider_message_from_fixture(&fixture, "tool_result");
    let mut snapshot = StreamingRunSnapshot::default();
    snapshot.apply_stream_event(&StreamEvent::ToolUse { message: tool_use });

    let attachment = snapshot
        .capsule_attachment_for_tool_result(&tool_result)
        .expect("Claude anonymous tool_result should correlate by tool_use_id");

    assert_eq!(attachment.action, CapsuleAttachmentAction::Created);
    assert_eq!(
        attachment.capsule_id,
        "01900000-0000-7000-8000-000000000001"
    );
    assert_eq!(attachment.title, "Test Capsule");
    assert_eq!(attachment.revision, 1);
}

#[test]
fn test_extract_capsule_attachment_from_codex_fixture_uses_direct_tool_name() {
    let fixture = load_capsule_provider_fixture("codex-capsule-create.json");
    let tool_result = provider_message_from_fixture(&fixture, "tool_result");

    let attachment = extract_capsule_attachment_from_tool_result(&tool_result, &HashMap::new())
        .expect("Codex direct mcp tool_result should extract capsule attachment");

    assert_eq!(attachment.action, CapsuleAttachmentAction::Created);
    assert_eq!(
        attachment.capsule_id,
        "01900000-0000-7000-8000-000000000002"
    );
    assert_eq!(attachment.title, "Test Capsule");
    assert_eq!(attachment.revision, 1);
}

#[test]
fn test_extract_capsule_attachment_from_payload_self_identifying_update_fixture() {
    let fixture = load_capsule_provider_fixture("payload-self-identifying-update.json");
    let tool_result = provider_message_from_fixture(&fixture, "tool_result");

    let attachment = extract_capsule_attachment_from_tool_result(&tool_result, &HashMap::new())
        .expect("self-identifying payload should extract capsule update attachment");

    assert_eq!(attachment.action, CapsuleAttachmentAction::Updated);
    assert_eq!(
        attachment.capsule_id,
        "01900000-0000-7000-8000-000000000003"
    );
    assert_eq!(attachment.title, "Updated Test Capsule");
    assert_eq!(attachment.revision, 2);
}

#[test]
fn test_capsule_attachment_marker_key_dedupes_repeated_tool_use_id() {
    let attachment = CapsuleMutationAttachment {
        action: CapsuleAttachmentAction::Created,
        capsule_id: "01900000-0000-7000-8000-000000000007".to_owned(),
        title: "Repeated Result Capsule".to_owned(),
        revision: 1,
    };

    assert_eq!(
        attachment.marker_key(Some("toolu_fixture_repeat"), 3),
        attachment.marker_key(Some("toolu_fixture_repeat"), 4),
        "a repeated completed result with the same tool_use_id must not emit another marker just because the content count advanced"
    );
    assert_ne!(
        attachment.marker_key(None, 3),
        attachment.marker_key(None, 4),
        "anonymous results still fall back to their physical content position"
    );
}

#[test]
fn test_capsule_attached_run_control_has_control_envelope() {
    let attachment = CapsuleMutationAttachment {
        action: CapsuleAttachmentAction::Updated,
        capsule_id: "01900000-0000-7000-8000-000000000004".to_owned(),
        title: "Envelope Test Capsule".to_owned(),
        revision: 3,
    };

    let control = capsule_attached_control_record(
        "thread::fixture-capsule",
        "run::fixture-capsule",
        &attachment,
        4,
    );

    assert_eq!(control.after_content_count, 4);
    assert_eq!(control.message["role"], "system");
    assert_eq!(control.message["kind"], "control");
    assert_eq!(control.message["internal"], true);
    assert_eq!(control.message["internal_kind"], "control");
    assert_eq!(control.message["control"]["kind"], "capsule_attached");
    assert_eq!(
        control.message["control"]["thread_id"],
        "thread::fixture-capsule"
    );
    assert_eq!(control.message["control"]["run_id"], "run::fixture-capsule");
    assert_eq!(
        control.message["control"]["capsule_id"],
        attachment.capsule_id
    );
    assert_eq!(control.message["control"]["revision"], 3);
    assert_eq!(control.message["control"]["action"], "updated");
    assert_eq!(control.message["control"]["title"], "Envelope Test Capsule");
}

#[tokio::test]
async fn test_capsule_attached_survives_terminal_reconcile_without_range_rewrite() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::capsule-reconcile";
    let run_id = "run::capsule-reconcile";
    let metadata = HashMap::from([("bridge_run_id".to_owned(), json!(run_id))]);
    let session_messages = vec![
        ProviderMessage::tool_use(
            json!({"tool": "mcp__garyx__capsule_create", "input": {"title": "Test Capsule"}}),
            Some("toolu_fixture_capsule_create".to_owned()),
            Some("mcp__garyx__capsule_create".to_owned()),
        )
        .with_timestamp("2026-06-29T00:00:01Z"),
        ProviderMessage::tool_result(
            json!({
                "result": [{
                    "type": "text",
                    "text": "{\"tool\":\"capsule_create\",\"status\":\"ok\",\"capsule_id\":\"01900000-0000-7000-8000-000000000005\",\"id\":\"01900000-0000-7000-8000-000000000005\",\"title\":\"Survives Reconcile Capsule\",\"revision\":1,\"open_url\":\"garyx://capsules/01900000-0000-7000-8000-000000000005\"}"
                }],
                "text": ""
            }),
            Some("toolu_fixture_capsule_create".to_owned()),
            None,
            Some(false),
        )
        .with_timestamp("2026-06-29T00:00:02Z"),
        ProviderMessage::assistant_text("final answer").with_timestamp("2026-06-29T00:00:03Z"),
    ];
    let mut tool_names = HashMap::new();
    tool_names.insert(
        "toolu_fixture_capsule_create".to_owned(),
        "mcp__garyx__capsule_create".to_owned(),
    );
    let attachment = extract_capsule_attachment_from_tool_result(&session_messages[1], &tool_names)
        .expect("fixture result extracts attachment");
    let controls = vec![capsule_attached_control_record(
        thread_id,
        run_id,
        &attachment,
        3,
    )];

    let (appended, committed) = save_streaming_partial(
        &store,
        &history,
        PersistedRun {
            thread_id,
            user_message: "create a capsule",
            user_timestamp: Some("2026-06-29T00:00:00Z"),
            user_images: &[],
            assistant_response: "final answer",
            sdk_session_id: None,
            provider_key: "provider::capsule",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &session_messages,
            metadata: &metadata,
        },
        &[],
        &controls,
        2,
        0,
    )
    .await;
    assert_eq!(appended, 4);
    assert!(
        committed.iter().any(|(_, message)| message
            .pointer("/control/kind")
            .and_then(Value::as_str)
            == Some("capsule_attached")),
        "streaming partial must commit the capsule marker once the tool_result is finalized"
    );

    let terminal = save_thread_messages_with_terminal_control(
        &store,
        &history,
        PersistedRun {
            thread_id,
            user_message: "create a capsule",
            user_timestamp: Some("2026-06-29T00:00:00Z"),
            user_images: &[],
            assistant_response: "final answer",
            sdk_session_id: None,
            provider_key: "provider::capsule",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &session_messages,
            metadata: &metadata,
        },
        &controls,
        None,
    )
    .await;
    assert!(
        terminal.iter().all(|(_, message)| message
            .pointer("/control/kind")
            .and_then(Value::as_str)
            != Some("range_rewrite")),
        "terminal reconcile may append the trailing assistant, but not a range_rewrite"
    );

    let records = history
        .transcript_store()
        .records(thread_id)
        .await
        .expect("records load");
    let control_kinds = records
        .iter()
        .filter_map(|record| {
            record
                .message
                .pointer("/control/kind")
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    assert_eq!(control_kinds, vec!["capsule_attached"]);
    assert_eq!(
        records
            .iter()
            .filter(|record| record
                .message
                .pointer("/control/kind")
                .and_then(Value::as_str)
                == Some("range_rewrite"))
            .count(),
        0,
        "authoritative capsule marker path must not create a range_rewrite"
    );
    assert_eq!(records[3].message["control"]["kind"], "capsule_attached");
    assert_eq!(records[4].message["content"], "final answer");
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
            user_timestamp: Some("2026-03-01T00:00:00Z"),
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
    assert_eq!(messages[0]["timestamp"], "2026-03-01T00:00:00Z");
    assert_eq!(messages[1]["content"], "在。先执行 ls。");
    assert_eq!(messages[4]["content"], "\n结果如下。");
}

#[tokio::test]
async fn test_save_thread_messages_copies_client_intent_to_user_origin_id() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let metadata = HashMap::from([(
        "client_intent_id".to_owned(),
        json!("00000000-0000-0000-0000-000000000001"),
    )]);

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::origin",
            user_message: "hello",
            user_timestamp: Some("2026-03-01T00:00:00Z"),
            user_images: &[],
            assistant_response: "answer",
            sdk_session_id: None,
            provider_key: "provider::origin",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
    )
    .await;

    let stored = store
        .get("thread::origin")
        .await
        .expect("stored session should exist");
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    assert_eq!(
        messages[0]["metadata"]["origin_id"],
        "00000000-0000-0000-0000-000000000001"
    );
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
            user_timestamp: None,
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

#[tokio::test]
async fn test_save_thread_messages_overrides_stale_metadata_sdk_session_id() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let session_messages = vec![ProviderMessage::assistant_text("new session answer")];
    let mut metadata = HashMap::new();
    metadata.insert("sdk_session_id".to_owned(), json!("old-session"));
    metadata.insert("client_run_id".to_owned(), json!("run-1"));

    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::sdk-session-message",
            user_message: "hello",
            user_timestamp: Some("2026-03-01T00:00:00Z"),
            user_images: &[],
            assistant_response: "new session answer",
            sdk_session_id: Some("new-session"),
            provider_key: "provider::sdk-session",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &session_messages,
            metadata: &metadata,
        },
    )
    .await;

    let stored = store
        .get("thread::sdk-session-message")
        .await
        .expect("stored session should exist");
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    assert_eq!(messages[0]["metadata"]["sdk_session_id"], "new-session");
    assert_eq!(messages[1]["sdk_session_id"], "new-session");
    assert_eq!(stored["sdk_session_id"], "new-session");
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
fn test_streaming_run_snapshot_stamps_assistant_segments_at_creation() {
    let mut snapshot = StreamingRunSnapshot::default();
    assert!(snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "alpha".to_owned(),
    }));
    let created_timestamp = snapshot.session_messages[0]
        .timestamp
        .clone()
        .expect("assistant segment must be stamped at creation");

    // Appending more deltas to the same segment keeps the creation stamp:
    // every partial save backfills unstamped rows with the save moment, so a
    // missing or shifting timestamp re-stamps the whole run's assistant rows
    // at each flush and loses their real order against tool rows.
    assert!(snapshot.apply_stream_event(&StreamEvent::Delta {
        text: " beta".to_owned(),
    }));
    assert_eq!(snapshot.session_messages.len(), 1);
    assert_eq!(
        snapshot.session_messages[0].timestamp.as_deref(),
        Some(created_timestamp.as_str())
    );
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
async fn test_save_streaming_partial_commits_user_row_without_inflight_content_track() {
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
    let run_started_at = "2026-03-01T00:00:10Z";

    let mut appended = 0usize;
    let mut snapshot = StreamingRunSnapshot::default();
    snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "hel".to_owned(),
    });
    appended = save_streaming_partial(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial",
            user_message: "hello",
            user_timestamp: Some(run_started_at),
            user_images: &[],
            assistant_response: &snapshot.assistant_response,
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &snapshot.session_messages,
            metadata: &metadata,
        },
        &[],
        &[],
        snapshot.finalized_len(),
        appended,
    )
    .await
    .0;

    snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "lo".to_owned(),
    });
    appended = save_streaming_partial(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial",
            user_message: "hello",
            user_timestamp: Some(run_started_at),
            user_images: &[],
            assistant_response: &snapshot.assistant_response,
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &snapshot.session_messages,
            metadata: &metadata,
        },
        &[],
        &[],
        snapshot.finalized_len(),
        appended,
    )
    .await
    .0;

    // The in-flight assistant segment is not finalized, so only the synthesized
    // user row is committed to the transcript (appended once, not twice).
    assert_eq!(appended, 1);
    let committed = history
        .transcript_store()
        .records("thread::partial")
        .await
        .expect("records should load");
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].message["role"], "user");
    assert_eq!(committed[0].message["content"], "hello");
    assert_eq!(committed[0].seq, 1);

    let stored = store
        .get("thread::partial")
        .await
        .expect("stored session should exist");
    assert_eq!(stored["sdk_session_id"], "sdk-existing");
    assert_eq!(
        stored["provider_sdk_session_ids"]["provider::partial"],
        "sdk-existing"
    );
    // The legacy bounded `messages` cache is left untouched by streaming partials.
    let messages = stored["messages"]
        .as_array()
        .expect("messages should be an array");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "older run");
    assert!(
        stored["history"].get("message_count").is_some(),
        "streaming partials still update committed history metadata"
    );
    assert_eq!(stored["history"]["message_count"], 1);
}

#[tokio::test]
async fn test_save_streaming_partial_clears_abandoned_pending_inputs_for_new_user_turn() {
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

    save_streaming_partial(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial-clear-orphaned",
            user_message: "fresh turn",
            user_timestamp: None,
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
        &[],
        &[],
        0,
        0,
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
async fn test_save_streaming_partial_keeps_abandoned_pending_inputs_for_internal_dispatch() {
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

    save_streaming_partial(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::partial-keep-orphaned",
            user_message: "continue working",
            user_timestamp: None,
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::partial",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
        &[],
        &[],
        0,
        0,
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

/// F1 end-to-end: streaming flushes append finalized rows to the committed
/// transcript in real time, and the terminal commit reconciles the tail to the
/// authoritative set without duplicating any streamed row.
#[tokio::test]
async fn test_streaming_then_terminal_commit_does_not_duplicate_messages() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::stream-no-dup";
    let metadata = HashMap::from([("client_run_id".to_owned(), json!("run-stream"))]);

    let flush = |snapshot: &StreamingRunSnapshot, appended: usize| {
        let store = store.clone();
        let history = history.clone();
        let metadata = metadata.clone();
        let assistant_response = snapshot.assistant_response.clone();
        let session_messages = snapshot.session_messages.clone();
        let finalized_len = snapshot.finalized_len();
        async move {
            save_streaming_partial(
                &store,
                &history,
                PersistedRun {
                    thread_id,
                    user_message: "do the thing",
                    user_timestamp: Some("2026-03-01T00:00:00Z"),
                    user_images: &[],
                    assistant_response: &assistant_response,
                    sdk_session_id: None,
                    provider_key: "provider::stream",
                    provider_type: ProviderType::ClaudeCode,
                    session_messages: &session_messages,
                    metadata: &metadata,
                },
                &[],
                &[],
                finalized_len,
                appended,
            )
            .await
            .0
        }
    };

    let mut snapshot = StreamingRunSnapshot::default();
    let mut appended = 0usize;

    // Initial flush commits just the synthesized user row.
    appended = flush(&snapshot, appended).await;
    assert_eq!(appended, 1);

    snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "Working".to_owned(),
    });
    appended = flush(&snapshot, appended).await; // assistant still in flight
    assert_eq!(appended, 1);

    snapshot.apply_stream_event(&StreamEvent::ToolUse {
        message: ProviderMessage::tool_use(
            json!({"tool": "Bash", "input": {"command": "ls"}}),
            None,
            Some("Bash".to_owned()),
        ),
    });
    appended = flush(&snapshot, appended).await; // assistant + tool_use finalized
    assert_eq!(appended, 3);

    snapshot.apply_stream_event(&StreamEvent::ToolResult {
        message: ProviderMessage::tool_result(
            json!({"result": "a\nb\n", "text": "a\nb\n"}),
            None,
            Some("Bash".to_owned()),
            Some(false),
        ),
    });
    appended = flush(&snapshot, appended).await;
    assert_eq!(appended, 4);

    snapshot.apply_stream_event(&StreamEvent::Delta {
        text: "Done".to_owned(),
    });
    appended = flush(&snapshot, appended).await; // final assistant still in flight
    assert_eq!(appended, 4);

    // During the run, the transcript already contains every finalized row and
    // the trailing in-flight assistant is intentionally not mirrored elsewhere.
    let mid = store.get(thread_id).await.expect("session exists");
    assert_eq!(mid["history"]["message_count"], 4);

    // Terminal commit: reconcile the tail to the full authoritative set.
    save_thread_messages(
        &store,
        &history,
        PersistedRun {
            thread_id,
            user_message: "do the thing",
            user_timestamp: Some("2026-03-01T00:00:00Z"),
            user_images: &[],
            assistant_response: &snapshot.assistant_response,
            sdk_session_id: None,
            provider_key: "provider::stream",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &snapshot.session_messages,
            metadata: &metadata,
        },
    )
    .await;

    let committed = history
        .transcript_store()
        .records(thread_id)
        .await
        .expect("records load");
    let content_roles: Vec<&str> = committed
        .iter()
        .filter(|record| record.message.get("kind").and_then(Value::as_str) != Some("control"))
        .filter_map(|record| record.message.get("role").and_then(Value::as_str))
        .collect();
    assert_eq!(
        content_roles,
        vec!["user", "assistant", "tool_use", "tool_result", "assistant"],
        "committed transcript should hold each run message exactly once"
    );
    let seqs: Vec<u64> = committed.iter().map(|record| record.seq).collect();
    assert_eq!(
        seqs,
        vec![1, 2, 3, 4, 5, 6],
        "seqs are monotonic with no gaps"
    );
    assert_eq!(committed[1].message["content"], "Working");
    assert_eq!(committed[4].message["content"], "Done");
    assert_eq!(committed[5].message["control"]["kind"], "range_rewrite");
    assert_eq!(
        committed[5].message["control"]["reason"],
        "same_seq_overwrite"
    );

    let stored = store.get(thread_id).await.expect("session exists");
    assert_eq!(stored["history"]["message_count"], 6);
}

#[tokio::test]
async fn test_control_records_stream_and_terminal_commit_to_transcript() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let thread_id = "thread::control-records";
    let metadata = HashMap::from([("bridge_run_id".to_owned(), json!("run-control"))]);
    let controls = vec![
        RunControlRecord::new(
            "run_start",
            thread_id,
            "run-control",
            "2026-06-18T12:00:00Z".to_owned(),
            serde_json::Map::new(),
            0,
        ),
        RunControlRecord::new(
            "done",
            thread_id,
            "run-control",
            "2026-06-18T12:00:01Z".to_owned(),
            serde_json::Map::new(),
            1,
        ),
    ];

    let (appended, committed) = save_streaming_partial(
        &store,
        &history,
        PersistedRun {
            thread_id,
            user_message: "persist controls",
            user_timestamp: Some("2026-06-18T12:00:00Z"),
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::control",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
        &[],
        &controls,
        0,
        0,
    )
    .await;
    assert_eq!(appended, 3);
    assert_eq!(committed.len(), 3);

    let terminal = save_thread_messages_with_terminal_control(
        &store,
        &history,
        PersistedRun {
            thread_id,
            user_message: "persist controls",
            user_timestamp: Some("2026-06-18T12:00:00Z"),
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::control",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &[],
            metadata: &metadata,
        },
        &controls,
        Some(TerminalRunControl {
            duration_ms: Some(42),
            success: Some(true),
            error: None,
            thread_title: Some("Control Fixture".to_owned()),
            rate_limit: None,
        }),
    )
    .await;
    assert_eq!(
        terminal.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
        vec![4, 5]
    );

    let records = history
        .transcript_store()
        .records(thread_id)
        .await
        .expect("records load");
    let control_kinds: Vec<&str> = records
        .iter()
        .filter_map(|record| {
            record
                .message
                .pointer("/control/kind")
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(
        control_kinds,
        vec!["run_start", "done", "thread_title_updated", "run_complete"]
    );
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
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
            user_timestamp: None,
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
async fn test_save_streaming_partial_does_not_commit_delivery_mirror() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let history = make_history(store.clone());
    let metadata = HashMap::from([("client_run_id".to_owned(), json!("run-mirror"))]);
    // A message-tool turn with no explicit assistant text: the terminal build
    // would synthesize a delivery-mirror assistant, but the streaming commit must
    // not — that synthesized row is unstable across the run.
    let session_messages = vec![
        ProviderMessage::tool_use(
            json!({"tool": "message", "input": {"text": "sent"}}),
            Some("tool-1".to_owned()),
            Some("mcp:gary:message".to_owned()),
        ),
        ProviderMessage::tool_result(
            json!({"result": {"tool": "message", "status": "ok", "text": "sent"}}),
            Some("tool-1".to_owned()),
            Some("mcp:gary:message".to_owned()),
            Some(false),
        ),
    ];
    save_streaming_partial(
        &store,
        &history,
        PersistedRun {
            thread_id: "thread::stream-mirror",
            user_message: "sync it",
            user_timestamp: Some("2026-03-01T00:00:00Z"),
            user_images: &[],
            assistant_response: "",
            sdk_session_id: None,
            provider_key: "provider::mirror",
            provider_type: ProviderType::ClaudeCode,
            session_messages: &session_messages,
            metadata: &metadata,
        },
        &[],
        &[],
        session_messages.len(),
        0,
    )
    .await;

    let committed = history
        .transcript_store()
        .records("thread::stream-mirror")
        .await
        .expect("records load");
    let roles: Vec<&str> = committed
        .iter()
        .filter_map(|record| record.message["role"].as_str())
        .collect();
    assert_eq!(
        roles,
        vec!["user", "tool_use", "tool_result"],
        "streaming must not commit the synthesized delivery-mirror assistant"
    );
    assert!(
        committed
            .iter()
            .all(|record| record.message["metadata"]["delivery_mirror"] != json!(true)),
        "no delivery_mirror row in the streamed transcript"
    );
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
            user_timestamp: None,
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
            user_timestamp: None,
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
            user_timestamp: None,
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
