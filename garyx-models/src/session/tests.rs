use super::*;

#[test]
fn test_chat_type_serde() {
    let ct = ChatType::Group;
    let json = serde_json::to_string(&ct).unwrap();
    assert_eq!(json, "\"group\"");
    let back: ChatType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ChatType::Group);
}

#[test]
fn test_exec_ask_serde() {
    let ea = ExecAsk::OnMiss;
    let json = serde_json::to_string(&ea).unwrap();
    assert_eq!(json, "\"on-miss\"");
    let back: ExecAsk = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ExecAsk::OnMiss);
}

#[test]
fn test_queue_mode_steer_backlog() {
    let qm = QueueMode::SteerBacklog;
    let json = serde_json::to_string(&qm).unwrap();
    assert_eq!(json, "\"steer-backlog\"");
    let back: QueueMode = serde_json::from_str(&json).unwrap();
    assert_eq!(back, QueueMode::SteerBacklog);
}

#[test]
fn test_session_entry_default_roundtrip() {
    let entry = SessionEntry::default();
    let json = serde_json::to_value(&entry).unwrap();
    let _back: SessionEntry = serde_json::from_value(json).unwrap();
}

#[test]
fn test_session_entry_new_thread_sets_thread_identity() {
    let entry = SessionEntry::new_thread("thread::new", "main");
    assert_eq!(entry.thread_id(), "thread::new");
    assert_eq!(entry.agent_id, "main");
}

#[test]
fn test_session_entry_update_usage() {
    let mut entry = SessionEntry::default();
    entry.update_usage(100, 50, 0.01);
    assert_eq!(entry.token_usage.input_tokens, 100);
    assert_eq!(entry.token_usage.output_tokens, 50);
    assert_eq!(entry.token_usage.total_tokens, 150);
}

#[test]
fn test_session_entry_can_send() {
    let mut entry = SessionEntry::default();
    assert!(entry.can_send());
    entry.send_policy = Some(SendPolicy::Deny);
    assert!(!entry.can_send());
    entry.send_policy = Some(SendPolicy::Allow);
    assert!(entry.can_send());
}

#[test]
fn test_session_entry_group_detection() {
    let mut entry = SessionEntry::default();
    assert!(entry.is_direct_session());
    assert!(!entry.is_group_session());

    entry.chat_type = Some(ChatType::Group);
    assert!(entry.is_group_session());
    assert!(!entry.is_direct_session());
}

#[test]
fn test_session_entry_exposes_decomposition_views() {
    let mut entry = SessionEntry::default();
    entry.session_file = Some("runtime.json".to_owned());
    entry.queue_mode = Some(QueueMode::Queue);
    entry.group_channel = Some("telegram".to_owned());
    entry.system_sent = true;
    entry.compaction_count = 3;

    let runtime = entry.provider_runtime_state();
    assert_eq!(runtime.session_file.as_deref(), Some("runtime.json"));

    let routing = entry.thread_routing_state();
    assert_eq!(routing.group_channel.as_deref(), Some("telegram"));

    let queue = entry.thread_queue_state();
    assert_eq!(queue.queue_mode, Some(QueueMode::Queue));
    assert!(queue.system_sent);

    let usage = entry.thread_usage_state();
    assert_eq!(usage.compaction_count, 3);
}

#[test]
fn test_session_entry_exposes_thread_record_view() {
    let mut entry = SessionEntry::default();
    entry.thread_id = "thread::abc".to_owned();
    entry.agent_id = "main".to_owned();
    entry.label = Some("Inbox".to_owned());
    entry.messages.push(HashMap::from([(
        "role".to_owned(),
        Value::String("user".to_owned()),
    )]));

    let view = entry.thread_record_view();
    assert_eq!(view.thread_id, "thread::abc");
    assert_eq!(view.agent_id, "main");
    assert_eq!(view.label, Some("Inbox"));
    assert_eq!(view.messages.len(), 1);
}

#[test]
fn test_session_entry_converts_to_owned_thread_record() {
    let mut entry = SessionEntry::default();
    entry.thread_id = "thread::owned".to_owned();
    entry.agent_id = "main".to_owned();
    entry.label = Some("Owned".to_owned());
    entry.messages.push(HashMap::from([(
        "role".to_owned(),
        Value::String("assistant".to_owned()),
    )]));

    let record = entry.to_thread_record();
    assert_eq!(record.thread_id, "thread::owned");
    assert_eq!(record.agent_id, "main");
    assert_eq!(record.label.as_deref(), Some("Owned"));
    assert_eq!(record.messages.len(), 1);
}

#[test]
fn test_session_entry_builds_from_owned_thread_record() {
    let record = crate::thread_record::ThreadRecord {
        thread_id: "thread::writeback".to_owned(),
        agent_id: "main".to_owned(),
        label: Some("Writeback".to_owned()),
        messages: vec![HashMap::from([(
            "role".to_owned(),
            Value::String("assistant".to_owned()),
        )])],
        queue: crate::thread_record::ThreadQueueState {
            system_sent: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let entry = SessionEntry::from(record);
    assert_eq!(entry.thread_id(), "thread::writeback");
    assert_eq!(entry.agent_id, "main");
    assert_eq!(entry.label.as_deref(), Some("Writeback"));
    assert!(entry.system_sent);
    assert_eq!(entry.messages.len(), 1);
}

#[test]
fn test_session_entry_deserializes_thread_id_alias() {
    let json = serde_json::json!({
        "thread_id": "thread::alias",
        "agent_id": "main"
    });
    let entry: SessionEntry = serde_json::from_value(json).unwrap();
    assert_eq!(entry.thread_id(), "thread::alias");
}

#[test]
fn test_session_entry_serializes_canonical_thread_id_field() {
    let entry = SessionEntry::new_thread("thread::canonical", "main");
    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(
        json.get("thread_id"),
        Some(&Value::String("thread::canonical".to_owned()))
    );
    assert!(json.get("session_id").is_none());
}
