use serde_json::Value;

fn parse_jsonl(name: &str, raw: &str) -> Vec<Value> {
    raw.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(
                serde_json::from_str::<Value>(trimmed)
                    .unwrap_or_else(|error| panic!("{name}: line {}: {error}", index + 1)),
            )
        })
        .collect()
}

fn assert_public_fixture(raw: &str) {
    assert!(
        raw.match_indices("/Users/")
            .all(|(offset, _)| raw[offset..].starts_with("/Users/test")),
        "fixture must use synthetic local user paths"
    );
    assert!(
        !raw.contains("@"),
        "fixture must not contain email-like personal identifiers"
    );
}

#[test]
fn stream_sync_transcript_fixture_is_gapless_and_has_tool_pair() {
    let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-tool.jsonl");
    assert_public_fixture(raw);
    let records = parse_jsonl("transcript-with-tool.jsonl", raw);
    assert!(
        !records.is_empty(),
        "fixture should contain transcript records"
    );

    for (offset, record) in records.iter().enumerate() {
        assert_eq!(
            record.get("seq").and_then(Value::as_u64),
            Some((offset + 1) as u64),
            "transcript seq must be 1-based and gapless"
        );
        assert_eq!(
            record.get("thread_id").and_then(Value::as_str),
            Some("thread::fixture-stream-sync-tool")
        );
        assert!(record.get("timestamp").and_then(Value::as_str).is_some());
        assert!(record.get("message").and_then(Value::as_object).is_some());
    }

    let roles: Vec<&str> = records
        .iter()
        .filter_map(|record| record.pointer("/message/role").and_then(Value::as_str))
        .collect();
    assert!(roles.contains(&"user"));
    assert!(roles.contains(&"assistant"));
    assert!(roles.contains(&"tool_use"));
    assert!(roles.contains(&"tool_result"));
}

#[test]
fn stream_sync_event_fixtures_cover_user_ack_and_lifecycle_seq_split() {
    let ack_raw = include_str!("../../test-fixtures/stream-sync/stream-events-with-user-ack.jsonl");
    let lifecycle_raw = include_str!("../../test-fixtures/stream-sync/stream-lifecycle.jsonl");
    assert_public_fixture(ack_raw);
    assert_public_fixture(lifecycle_raw);

    let ack_events = parse_jsonl("stream-events-with-user-ack.jsonl", ack_raw);
    let ack_index = ack_events
        .iter()
        .position(|event| event.get("type").and_then(Value::as_str) == Some("user_ack"))
        .expect("fixture should include user_ack");
    let stream_input_index = ack_events
        .iter()
        .position(|event| event.get("type").and_then(Value::as_str) == Some("stream_input"))
        .expect("fixture should include stream_input");
    assert!(
        ack_index < stream_input_index,
        "fixture should capture user_ack arriving before stream_input"
    );
    assert_eq!(
        ack_events[ack_index]
            .get("pendingInputId")
            .and_then(Value::as_str),
        Some("pending-fixture-followup"),
        "user_ack fixture should retain the pending input id"
    );
    assert_eq!(
        ack_events[stream_input_index]
            .get("pendingInputId")
            .and_then(Value::as_str),
        Some("pending-fixture-followup"),
        "stream_input should describe the same queued input"
    );
    assert_eq!(
        ack_events[stream_input_index]
            .get("clientIntentId")
            .and_then(Value::as_str),
        Some("intent-fixture-followup"),
        "stream_input should retain the client intent id"
    );
    assert!(
        ack_events
            .iter()
            .all(|event| event.get("pending_input_id").is_none()
                && event.get("client_intent_id").is_none()
                && event.get("thread_id").is_none()
                && event.get("run_id").is_none()),
        "chat WS fixture should use camelCase frame fields"
    );
    assert!(
        ack_events.iter().all(|event| event.get("seq").is_none()),
        "chat WS stream/control events in this fixture are intentionally unseqed"
    );

    let lifecycle_events = parse_jsonl("stream-lifecycle.jsonl", lifecycle_raw);
    let event_types: Vec<&str> = lifecycle_events
        .iter()
        .filter_map(|event| event.get("type").and_then(Value::as_str))
        .collect();
    assert_eq!(
        event_types,
        vec![
            "run_start",
            "committed_message",
            "assistant_delta",
            "committed_message",
            "done",
            "run_complete"
        ]
    );

    let committed_seqs: Vec<u64> = lifecycle_events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("committed_message"))
        .filter_map(|event| event.get("seq").and_then(Value::as_u64))
        .collect();
    assert_eq!(committed_seqs, vec![1, 2]);
    assert!(
        lifecycle_events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) != Some("committed_message"))
            .all(|event| event.get("seq").is_none()),
        "only committed_message carries seq before v3 control-record landing"
    );
}
