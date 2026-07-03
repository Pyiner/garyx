use super::*;
use garyx_models::provider::ProviderMessage;

fn user_message(text: &str, timestamp: &str) -> Value {
    let mut message = ProviderMessage::user_text(text);
    message.timestamp = Some(timestamp.to_owned());
    serde_json::to_value(message).expect("message serializes")
}

fn existing_dream_topic(
    dream_id: &str,
    thread_id: &str,
    start_seq: u64,
    end_seq: u64,
) -> DreamTopicRecord {
    DreamTopicRecord {
        dream_id: dream_id.to_owned(),
        title: "Dreams".to_owned(),
        summary: String::new(),
        first_message_at: "2026-05-21T09:00:00.000Z".to_owned(),
        last_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
        updated_at: "2026-05-21T10:00:00.000Z".to_owned(),
        source: "claude".to_owned(),
        confidence: 0.8,
        message_count: 1,
        span_count: 1,
        spans: vec![DreamSpanRecord {
            span_id: format!("span::{dream_id}"),
            dream_id: dream_id.to_owned(),
            thread_id: thread_id.to_owned(),
            workspace_dir: Some("/workspace/test".to_owned()),
            start_seq,
            end_seq,
            start_at: "2026-05-21T10:00:00.000Z".to_owned(),
            end_at: "2026-05-21T10:00:00.000Z".to_owned(),
            excerpt: "Continue dreams implementation".to_owned(),
            message_count: 1,
        }],
    }
}

#[tokio::test]
async fn recent_user_message_probe_detects_transcript_user_messages() {
    let state = crate::server::AppStateBuilder::new(GaryxConfig::default()).build();
    let thread_id = "thread::dream-probe";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "updated_at": "2026-05-21T10:05:00Z",
                "workspace_dir": "/workspace/test"
            }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            thread_id,
            Some("run::dream-probe"),
            &[user_message(
                "Design auto dreams scan",
                "2026-05-21T10:00:00Z",
            )],
        )
        .await
        .expect("append transcript");

    let has_message = has_recent_dream_user_message(
        &state,
        parse_timestamp("2026-05-21T09:30:00Z").unwrap(),
        parse_timestamp("2026-05-21T10:30:00Z").unwrap(),
    )
    .await
    .expect("probe succeeds");

    assert!(has_message);
}

#[tokio::test]
async fn recent_user_message_probe_skips_threads_older_than_window() {
    let state = crate::server::AppStateBuilder::new(GaryxConfig::default()).build();
    let thread_id = "thread::dream-old";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "updated_at": "2026-05-21T08:00:00Z"
            }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            thread_id,
            Some("run::dream-old"),
            &[user_message("Old dream message", "2026-05-21T08:00:00Z")],
        )
        .await
        .expect("append transcript");

    let has_message = has_recent_dream_user_message(
        &state,
        parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
        parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
    )
    .await
    .expect("probe succeeds");

    assert!(!has_message);
}

#[tokio::test]
async fn auto_dream_scan_respects_disabled_switch() {
    let state = crate::server::AppStateBuilder::new(GaryxConfig::default()).build();

    let outcome =
        run_auto_dream_scan_once(&state, parse_timestamp("2026-05-21T10:00:00Z").unwrap())
            .await
            .expect("auto scan succeeds");

    assert_eq!(outcome, DreamAutoScanOutcome::Disabled);
}

#[tokio::test]
async fn auto_dream_scan_skips_without_recent_user_messages() {
    let mut config = GaryxConfig::default();
    config.dreams.enabled = true;
    let state = crate::server::AppStateBuilder::new(config).build();

    let outcome =
        run_auto_dream_scan_once(&state, parse_timestamp("2026-05-21T10:00:00Z").unwrap())
            .await
            .expect("auto scan succeeds");

    assert_eq!(
        outcome,
        DreamAutoScanOutcome::NoRecentMessages {
            from: "2026-05-21T09:00:00.000Z".to_owned(),
            to: "2026-05-21T10:00:00.000Z".to_owned(),
        }
    );
}

#[test]
fn heuristic_splits_one_thread_into_multiple_topics() {
    let messages = vec![
        DreamUserMessage {
            thread_id: "thread::one".to_owned(),
            workspace_dir: None,
            seq: 1,
            timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
            text: "Review the pin API and gateway storage".to_owned(),
        },
        DreamUserMessage {
            thread_id: "thread::one".to_owned(),
            workspace_dir: None,
            seq: 2,
            timestamp: parse_timestamp("2026-05-21T10:04:00Z").unwrap(),
            text: "Check how mobile reads thread pins".to_owned(),
        },
        DreamUserMessage {
            thread_id: "thread::one".to_owned(),
            workspace_dir: None,
            seq: 3,
            timestamp: parse_timestamp("2026-05-21T10:08:00Z").unwrap(),
            text: "另外设计梦境的一天主题列表".to_owned(),
        },
    ];

    let topics = heuristic_topics(&messages);
    assert_eq!(topics.len(), 2);
    assert!(
        topics
            .iter()
            .any(|topic| topic.spans[0].start_seq == 1 && topic.spans[0].end_seq == 2)
    );
    assert!(topics.iter().any(|topic| topic.spans[0].start_seq == 3));
}

#[test]
fn heuristic_reuses_existing_id_for_overlapping_span() {
    let messages = vec![DreamUserMessage {
        thread_id: "thread::one".to_owned(),
        workspace_dir: Some("/workspace/test".to_owned()),
        seq: 1,
        timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
        text: "Continue dreams implementation".to_owned(),
    }];
    let topics = heuristic_topics(&messages);
    let existing_topics = vec![existing_dream_topic("dream::existing", "thread::one", 1, 1)];

    let reused = reuse_existing_topic_ids_for_matching_spans(topics, &existing_topics);

    assert_eq!(reused.len(), 1);
    assert_eq!(reused[0].dream_id.as_deref(), Some("dream::existing"));
}

#[test]
fn reuse_existing_ids_claims_each_existing_topic_once() {
    let span = ExtractedDreamSpan {
        thread_id: "thread::one".to_owned(),
        workspace_dir: Some("/workspace/test".to_owned()),
        start_seq: 1,
        end_seq: 1,
        start_at: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
        end_at: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
        excerpt: "Continue dreams implementation".to_owned(),
        message_count: 1,
    };
    let topics = vec![
        ExtractedDreamTopic {
            dream_id: None,
            title: "First".to_owned(),
            summary: String::new(),
            source: "heuristic".to_owned(),
            confidence: 0.5,
            spans: vec![span.clone()],
        },
        ExtractedDreamTopic {
            dream_id: None,
            title: "Second".to_owned(),
            summary: String::new(),
            source: "heuristic".to_owned(),
            confidence: 0.5,
            spans: vec![span],
        },
    ];
    let existing_topics = vec![existing_dream_topic("dream::existing", "thread::one", 1, 1)];

    let reused = reuse_existing_topic_ids_for_matching_spans(topics, &existing_topics);

    assert_eq!(reused[0].dream_id.as_deref(), Some("dream::existing"));
    assert_eq!(reused[1].dream_id, None);
}

#[test]
fn claude_topic_without_id_reuses_existing_overlap_after_normalization() {
    let messages = vec![DreamUserMessage {
        thread_id: "thread::one".to_owned(),
        workspace_dir: Some("/workspace/test".to_owned()),
        seq: 1,
        timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
        text: "Continue dreams implementation".to_owned(),
    }];
    let raw = r#"{
          "topics": [
            {
              "title": "Dreams",
              "spans": [
                {"thread_id": "thread::one", "start_seq": 1, "end_seq": 1}
              ]
            }
          ]
        }"#;
    let existing_topics = vec![existing_dream_topic("dream::existing", "thread::one", 1, 1)];
    let normalized = normalize_claude_topics(
        parse_claude_topics(raw).unwrap(),
        &messages,
        &existing_topics,
    );

    let reused = reuse_existing_topic_ids_for_matching_spans(normalized, &existing_topics);

    assert_eq!(reused.len(), 1);
    assert_eq!(reused[0].dream_id.as_deref(), Some("dream::existing"));
}

#[test]
fn normalizes_claude_json_into_known_spans() {
    let messages = vec![
        DreamUserMessage {
            thread_id: "thread::one".to_owned(),
            workspace_dir: Some("/workspace/test".to_owned()),
            seq: 1,
            timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
            text: "Implement dreams".to_owned(),
        },
        DreamUserMessage {
            thread_id: "thread::one".to_owned(),
            workspace_dir: Some("/workspace/test".to_owned()),
            seq: 2,
            timestamp: parse_timestamp("2026-05-21T10:02:00Z").unwrap(),
            text: "Add CLI and mobile".to_owned(),
        },
    ];
    let raw = r#"{"topics":[{"title":"Dreams","summary":"Daily topic map","confidence":0.9,"spans":[{"thread_id":"thread::one","start_seq":1,"end_seq":2,"excerpt":"dreams work"}]}]}"#;
    let topics = normalize_claude_topics(parse_claude_topics(raw).unwrap(), &messages, &[]);
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].title, "Dreams");
    assert_eq!(
        topics[0].spans[0].workspace_dir.as_deref(),
        Some("/workspace/test")
    );
    assert_eq!(topics[0].spans[0].message_count, 2);
}

#[test]
fn normalizes_claude_json_dedupes_duplicate_spans() {
    let messages = vec![DreamUserMessage {
        thread_id: "thread::one".to_owned(),
        workspace_dir: Some("/workspace/test".to_owned()),
        seq: 1,
        timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
        text: "Implement dream dedupe".to_owned(),
    }];
    let raw = r#"{
          "topics": [
            {
              "title": "Dream Dedupe",
              "spans": [
                {"thread_id": "thread::one", "start_seq": 1, "end_seq": 1},
                {"thread_id": "thread::one", "start_seq": 1, "end_seq": 1, "excerpt": "duplicate"}
              ]
            }
          ]
        }"#;

    let topics = normalize_claude_topics(parse_claude_topics(raw).unwrap(), &messages, &[]);

    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].spans.len(), 1);
    assert_eq!(topics[0].spans[0].start_seq, 1);
}

#[test]
fn normalizes_incremental_claude_json_preserves_existing_dream_id_and_skips_unchanged() {
    let messages = vec![DreamUserMessage {
        thread_id: "thread::one".to_owned(),
        workspace_dir: Some("/workspace/test".to_owned()),
        seq: 2,
        timestamp: parse_timestamp("2026-05-21T10:30:00Z").unwrap(),
        text: "Continue dream scheduler work".to_owned(),
    }];
    let raw = r#"{
          "topics": [
            {
              "dream_id": "dream::existing",
              "action": "update",
              "title": "Dream Scheduler",
              "spans": [
                {"thread_id": "thread::one", "start_seq": 2, "end_seq": 2}
              ]
            },
            {
              "dream_id": "dream::unchanged",
              "action": "unchanged",
              "title": "No change",
              "spans": [
                {"thread_id": "thread::one", "start_seq": 2, "end_seq": 2}
              ]
            }
          ]
        }"#;

    let existing_topics = vec![DreamTopicRecord {
        dream_id: "dream::existing".to_owned(),
        title: "Dream Scheduler".to_owned(),
        summary: String::new(),
        first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
        last_message_at: "2026-05-21T10:20:00.000Z".to_owned(),
        updated_at: "2026-05-21T10:20:00.000Z".to_owned(),
        source: "claude".to_owned(),
        confidence: 0.8,
        message_count: 1,
        span_count: 1,
        spans: Vec::new(),
    }];

    let topics = normalize_claude_topics(
        parse_claude_topics(raw).unwrap(),
        &messages,
        &existing_topics,
    );

    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].dream_id.as_deref(), Some("dream::existing"));
    assert_eq!(topics[0].title, "Dream Scheduler");
}

#[test]
fn normalizes_manual_claude_json_ignores_unknown_dream_ids() {
    let messages = vec![DreamUserMessage {
        thread_id: "thread::one".to_owned(),
        workspace_dir: Some("/workspace/test".to_owned()),
        seq: 1,
        timestamp: parse_timestamp("2026-05-21T10:00:00Z").unwrap(),
        text: "Implement dreams".to_owned(),
    }];
    let raw = r#"{
          "topics": [
            {
              "dream_id": "dream::hallucinated",
              "title": "Dreams",
              "spans": [
                {"thread_id": "thread::one", "start_seq": 1, "end_seq": 1}
              ]
            }
          ]
        }"#;

    let topics = normalize_claude_topics(parse_claude_topics(raw).unwrap(), &messages, &[]);

    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].dream_id, None);
}

#[test]
fn temporary_claude_options_disable_workspace_settings_and_tools() {
    let options = temporary_claude_options(&GaryxConfig::default());

    assert_eq!(options.setting_sources, Some(Vec::new()));
    assert_eq!(options.permission_mode, Some(PermissionMode::Default));
    assert!(options.allowed_tools.is_empty());
    assert!(options.disallowed_tools.is_empty());
    assert!(!options.extra_args.contains_key("bare"));
    assert!(options.extra_args.contains_key("disable-slash-commands"));
    assert!(options.extra_args.contains_key("no-session-persistence"));
    assert!(options.extra_args.contains_key("strict-mcp-config"));
    assert_eq!(
        options.extra_args.get("tools").and_then(Option::as_deref),
        Some("")
    );
    let args = options.to_cli_args();
    let setting_sources = args
        .iter()
        .position(|arg| arg == "--setting-sources")
        .expect("temporary Claude must explicitly override setting sources");
    assert_eq!(args[setting_sources + 1], "");
    assert!(!args.contains(&"--bare".to_owned()));
    assert!(args.contains(&"--no-session-persistence".to_owned()));
    let tools = args
        .iter()
        .position(|arg| arg == "--tools")
        .expect("temporary Claude must explicitly override built-in tools");
    assert_eq!(args[tools + 1], "");
}

#[test]
fn dream_user_message_extracts_provider_user_text() {
    let message = user_message("A visible user request", "2026-05-21T10:00:00Z");
    let entry = dream_user_message(
        "thread::one",
        None,
        1,
        &message,
        None,
        parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
        parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
    )
    .expect("user message is visible");
    assert_eq!(entry.text, "A visible user request");
}

#[test]
fn dream_user_message_skips_internal_dispatch_messages() {
    let mut message = user_message("Task #TASK-1 has been assigned", "2026-05-21T10:00:00Z");
    message["internal"] = json!(true);
    message["metadata"] = json!({
        "internal_dispatch": true,
        "task_auto_start": true,
        "task_id": "#TASK-1",
        "runtime_context": {
            "task": { "task_id": "#TASK-1" }
        }
    });
    let entry = dream_user_message(
        "thread::one",
        None,
        1,
        &message,
        None,
        parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
        parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
    );
    assert!(entry.is_none());
}

#[test]
fn dream_user_message_keeps_null_task_metadata_markers() {
    let mut message = user_message("Implement a task dashboard summary", "2026-05-21T10:00:00Z");
    message["metadata"] = json!({
        "task_id": null,
        "task_dispatch_reason": null,
        "runtime_context": {
            "task": null,
            "automation": null
        }
    });
    let entry = dream_user_message(
        "thread::one",
        None,
        1,
        &message,
        None,
        parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
        parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
    )
    .expect("null task metadata should not hide a normal user message");
    assert_eq!(entry.text, "Implement a task dashboard summary");
}

#[test]
fn dream_user_message_skips_control_only_text() {
    for text in [
        "continue",
        "/continue",
        "停止",
        "滴滴",
        "加油",
        "<garyx_task_notification event=\"ready_for_review\">Task #TASK-1</garyx_task_notification>",
    ] {
        let message = user_message(text, "2026-05-21T10:00:00Z");
        let entry = dream_user_message(
            "thread::one",
            None,
            1,
            &message,
            None,
            parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
            parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
        );
        assert!(entry.is_none(), "{text:?} should not become a dream input");
    }
}

#[test]
fn dream_user_message_keeps_real_followup_requests() {
    let message = user_message(
        "继续实现梦境 topic 抽取和桌面首页展示",
        "2026-05-21T10:00:00Z",
    );
    let entry = dream_user_message(
        "thread::one",
        None,
        1,
        &message,
        None,
        parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
        parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
    )
    .expect("substantive follow-up should remain dream input");
    assert_eq!(entry.text, "继续实现梦境 topic 抽取和桌面首页展示");
}

#[test]
fn dream_user_message_keeps_user_task_references() {
    let message = user_message(
        "Task #TASK-71 should also cover duplicate topic cleanup",
        "2026-05-21T10:00:00Z",
    );
    let entry = dream_user_message(
        "thread::one",
        None,
        1,
        &message,
        None,
        parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
        parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
    )
    .expect("substantive user task references should remain dream input");
    assert_eq!(
        entry.text,
        "Task #TASK-71 should also cover duplicate topic cleanup"
    );
}

#[test]
fn dream_user_message_skips_internal_messages() {
    let mut message = user_message("Implement dreams", "2026-05-21T10:00:00Z");
    message["internal"] = Value::Bool(true);
    let entry = dream_user_message(
        "thread::one",
        None,
        1,
        &message,
        None,
        parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
        parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
    );
    assert!(entry.is_none());
}

#[test]
fn dream_user_message_skips_task_and_automation_prompts() {
    for text in [
        "<garyx_task_notification event=\"ready_for_review\">review</garyx_task_notification>",
        "Task #TASK-70 has been assigned to you and is already in progress.",
        "你是负责增量下载电视剧《测试剧》的助手，每 3 小时被调起一次。",
        "你是 PT 增量下载助手，每 3 小时被调起一次。",
    ] {
        let message = user_message(text, "2026-05-21T10:00:00Z");
        let entry = dream_user_message(
            "thread::one",
            None,
            1,
            &message,
            None,
            parse_timestamp("2026-05-21T09:00:00Z").unwrap(),
            parse_timestamp("2026-05-21T11:00:00Z").unwrap(),
        );
        assert!(entry.is_none(), "{text:?} should not become a dream input");
    }
}
