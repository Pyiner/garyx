use super::*;
use crate::memory_store::InMemoryThreadStore;
use garyx_models::RenderRow;
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn transcript_store_appends_and_reads_tail() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_committed_messages(
            "thread::tail",
            Some("run-1"),
            &[
                json!({"role": "user", "content": "a"}),
                json!({"role": "assistant", "content": "b"}),
            ],
        )
        .await
        .unwrap();

    let tail = store.tail("thread::tail", 1).await.unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0]["content"], "b");
    assert_eq!(store.message_count("thread::tail").await.unwrap(), 2);
}

#[tokio::test]
async fn repository_reads_only_committed_transcript() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let transcript_store = Arc::new(ThreadTranscriptStore::memory());
    transcript_store
        .append_committed_messages(
            "thread::committed-only",
            Some("run-past"),
            &[json!({"role": "user", "content": "past"})],
        )
        .await
        .unwrap();
    thread_store
        .set(
            "thread::committed-only",
            json!({
                "history": {
                    "message_count": 1
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(thread_store, transcript_store);

    let snapshot = repo
        .thread_snapshot("thread::committed-only", 10)
        .await
        .unwrap();
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0]["content"], "past");
}

#[tokio::test]
async fn transcript_run_state_reports_dangling_run_as_busy() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let transcript_store = Arc::new(ThreadTranscriptStore::memory());
    transcript_store
        .append_run_records(
            "thread::live-only",
            Some("run-live"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "system",
                        "kind": "control",
                        "internal": true,
                        "internal_kind": "control",
                        "control": {
                            "kind": "run_start",
                            "thread_id": "thread::live-only",
                            "run_id": "run-live",
                            "at": "2026-06-18T12:00:00Z"
                        }
                    }),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::from_message(json!({
                    "role": "user",
                    "content": "live"
                })),
            ],
        )
        .await
        .unwrap();
    thread_store
        .set(
            "thread::live-only",
            json!({
                "history": {
                    "message_count": 2
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(thread_store, transcript_store.clone());

    let snapshot = repo.thread_snapshot("thread::live-only", 10).await.unwrap();
    assert_eq!(snapshot.total_committed_messages, 2);
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0]["control"]["kind"], "run_start");
    assert_eq!(combined[1]["content"], "live");

    let state = transcript_store
        .run_state("thread::live-only")
        .await
        .unwrap();
    assert!(state.busy);
    assert_eq!(state.active_run_id.as_deref(), Some("run-live"));
}

#[tokio::test]
async fn render_snapshot_at_seq_uses_committed_records_up_to_bound() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::render-bound",
            Some("run-render"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "answer"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "future"}),
                    "2026-06-18T12:00:02Z",
                ),
            ],
        )
        .await
        .unwrap();

    let snapshot = store
        .render_snapshot_at_seq("thread::render-bound", 2)
        .await
        .unwrap();

    assert_eq!(snapshot.based_on_seq, 2);
    assert_eq!(snapshot.visible_message_ids, vec!["seq:1", "seq:2"]);
    assert!(
        !snapshot.visible_message_ids.iter().any(|id| id == "seq:3"),
        "render snapshot must not include future records beyond the frame seq"
    );
}

#[tokio::test]
async fn render_snapshot_at_seq_reports_dangling_run_activity() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::render-live",
            Some("run-render-live"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "system",
                        "kind": "control",
                        "internal": true,
                        "internal_kind": "control",
                        "control": {
                            "kind": "run_start",
                            "thread_id": "thread::render-live",
                            "run_id": "run-render-live",
                            "at": "2026-06-18T12:00:00Z"
                        }
                    }),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "live"}),
                    "2026-06-18T12:00:01Z",
                ),
            ],
        )
        .await
        .unwrap();

    let snapshot = store
        .render_snapshot_at_seq("thread::render-live", 2)
        .await
        .unwrap();

    assert_eq!(snapshot.based_on_seq, 2);
    assert_eq!(
        snapshot.tail_activity,
        garyx_models::RenderTailActivity::Thinking
    );
}

#[tokio::test]
async fn render_snapshot_in_window_limits_rows_and_reports_window() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::render-window",
            Some("run-render-window"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "older question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "older answer"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "new question"}),
                    "2026-06-18T12:00:02Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "new answer"}),
                    "2026-06-18T12:00:03Z",
                ),
            ],
        )
        .await
        .unwrap();

    let snapshot = store
        .render_snapshot_in_window("thread::render-window", 3, 4)
        .await
        .unwrap();

    assert_eq!(snapshot.based_on_seq, 4);
    assert_eq!(snapshot.visible_message_ids, vec!["seq:3", "seq:4"]);
    assert_eq!(
        snapshot.window,
        Some(garyx_models::RenderWindow {
            floor_seq: 3,
            has_more_above: true,
        })
    );
}

#[tokio::test]
async fn render_snapshot_in_window_uses_full_prefix_run_state() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::render-window-run-state",
            Some("run-render-window-run-state"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "system",
                        "kind": "control",
                        "internal": true,
                        "internal_kind": "control",
                        "control": {
                            "kind": "run_start",
                            "thread_id": "thread::render-window-run-state",
                            "run_id": "run-render-window-run-state",
                            "at": "2026-06-18T12:00:00Z"
                        }
                    }),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "new question"}),
                    "2026-06-18T12:00:01Z",
                ),
            ],
        )
        .await
        .unwrap();

    let snapshot = store
        .render_snapshot_in_window("thread::render-window-run-state", 2, 2)
        .await
        .unwrap();

    assert_eq!(snapshot.visible_message_ids, vec!["seq:2"]);
    assert_eq!(
        snapshot.tail_activity,
        garyx_models::RenderTailActivity::Thinking,
        "run_state must come from the full prefix, not only window records"
    );
}

#[tokio::test]
async fn repository_rejects_stale_history_count_without_transcript() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    thread_store
        .set(
            "thread::legacy-inline",
            json!({
                "messages": [{"role": "user", "content": "legacy"}],
                "history": {
                    "message_count": 1
                }
            }),
        )
        .await;
    let repo =
        ThreadHistoryRepository::new(thread_store, Arc::new(ThreadTranscriptStore::memory()));

    let error = repo
        .thread_snapshot("thread::legacy-inline", 10)
        .await
        .expect_err("missing transcript should fail when history count is non-zero");
    assert!(matches!(
        error,
        ThreadHistoryError::MissingTranscript(thread_id) if thread_id == "thread::legacy-inline"
    ));
}

#[tokio::test]
async fn page_after_index_returns_messages_after_cursor() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_committed_messages(
            "thread::fa",
            Some("run-1"),
            &[
                json!({"role": "user", "content": "a"}),
                json!({"role": "assistant", "content": "b"}),
                json!({"role": "user", "content": "c"}),
                json!({"role": "assistant", "content": "d"}),
            ],
        )
        .await
        .unwrap();

    let (msgs, total, start) = store.page_after_index("thread::fa", 1, 10).await.unwrap();
    assert_eq!(total, 4);
    assert_eq!(start, 2);
    assert_eq!(
        msgs.iter()
            .map(|m| m["content"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["c", "d"]
    );

    // bounded by limit
    let (msgs2, _, start2) = store.page_after_index("thread::fa", 1, 1).await.unwrap();
    assert_eq!(start2, 2);
    assert_eq!(msgs2.len(), 1);
    assert_eq!(msgs2[0]["content"], "c");

    // caught up (cursor at last) → empty
    let (msgs3, _, _) = store.page_after_index("thread::fa", 3, 10).await.unwrap();
    assert!(msgs3.is_empty());
}

#[tokio::test]
async fn thread_snapshot_after_index_returns_committed_tail_only() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let transcript_store = Arc::new(ThreadTranscriptStore::memory());
    transcript_store
        .append_committed_messages(
            "thread::fa2",
            Some("run-1"),
            &[
                json!({"role": "user", "content": "a"}),
                json!({"role": "assistant", "content": "b"}),
            ],
        )
        .await
        .unwrap();
    thread_store
        .set(
            "thread::fa2",
            json!({
                "history": {
                    "message_count": 2
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(thread_store, transcript_store);

    // after index 0 -> committed tail [b] only.
    let snapshot = repo
        .thread_snapshot_after_index("thread::fa2", 0, 10)
        .await
        .unwrap();
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0]["content"], "b");
}

#[tokio::test]
async fn thread_snapshot_after_index_respects_limit_without_overlay() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let transcript_store = Arc::new(ThreadTranscriptStore::memory());
    transcript_store
        .append_committed_messages(
            "thread::fa3",
            Some("run-1"),
            &[
                json!({"role": "user", "content": "a"}),
                json!({"role": "assistant", "content": "b"}),
                json!({"role": "user", "content": "c"}),
                json!({"role": "assistant", "content": "d"}),
            ],
        )
        .await
        .unwrap();
    thread_store
        .set(
            "thread::fa3",
            json!({
                "history": {
                    "message_count": 4
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(thread_store, transcript_store);

    // after 0, limit 1 -> committed tail [b] only.
    let snapshot = repo
        .thread_snapshot_after_index("thread::fa3", 0, 1)
        .await
        .unwrap();
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0]["content"], "b");
}

#[tokio::test]
async fn reconcile_run_tail_is_noop_when_tail_already_matches() {
    let store = ThreadTranscriptStore::memory();
    let run = [
        json!({"role": "user", "content": "u"}),
        json!({"role": "assistant", "content": "a"}),
    ];
    store
        .append_committed_messages("thread::rec-noop", Some("run-1"), &run)
        .await
        .unwrap();

    let result = store
        .reconcile_run_tail("thread::rec-noop", "run-1", &run)
        .await
        .unwrap();
    assert_eq!(result.total_messages, 2);
    assert_eq!(store.message_count("thread::rec-noop").await.unwrap(), 2);
}

#[tokio::test]
async fn reconcile_run_tail_appends_grown_suffix_without_rewriting_prefix() {
    let store = ThreadTranscriptStore::memory();
    // The streaming worker committed the user row + first assistant segment.
    store
        .append_committed_messages(
            "thread::rec-grow",
            Some("run-1"),
            &[
                json!({"role": "user", "content": "u"}),
                json!({"role": "assistant", "content": "first"}),
            ],
        )
        .await
        .unwrap();

    // Terminal authoritative set adds the final assistant segment.
    let authoritative = [
        json!({"role": "user", "content": "u"}),
        json!({"role": "assistant", "content": "first"}),
        json!({"role": "assistant", "content": "final"}),
    ];
    let result = store
        .reconcile_run_tail("thread::rec-grow", "run-1", &authoritative)
        .await
        .unwrap();
    assert_eq!(result.total_messages, 3);

    let records = store.records("thread::rec-grow").await.unwrap();
    let contents: Vec<&str> = records
        .iter()
        .filter_map(|record| record.message["content"].as_str())
        .collect();
    assert_eq!(contents, vec!["u", "first", "final"]);
    // Prefix seqs are preserved (suffix appended, not a full rewrite).
    assert_eq!(
        records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[tokio::test]
async fn reconcile_run_tail_rewrites_divergent_retry_without_duplication() {
    let store = ThreadTranscriptStore::memory();
    // A prior committed turn from a different run must be preserved untouched.
    store
        .append_committed_messages(
            "thread::rec-diverge",
            Some("run-old"),
            &[json!({"role": "user", "content": "earlier turn"})],
        )
        .await
        .unwrap();
    // First attempt of run-1 streamed a wrong/aborted answer.
    store
        .append_committed_messages(
            "thread::rec-diverge",
            Some("run-1"),
            &[
                json!({"role": "user", "content": "u"}),
                json!({"role": "assistant", "content": "aborted attempt"}),
            ],
        )
        .await
        .unwrap();

    // The retry produced a different authoritative answer for the same run.
    let authoritative = [
        json!({"role": "user", "content": "u"}),
        json!({"role": "assistant", "content": "correct answer"}),
    ];
    let result = store
        .reconcile_run_tail("thread::rec-diverge", "run-1", &authoritative)
        .await
        .unwrap();
    assert_eq!(result.total_messages, 3, "old turn + reconciled run tail");

    let records = store.records("thread::rec-diverge").await.unwrap();
    let contents: Vec<&str> = records
        .iter()
        .filter_map(|record| record.message["content"].as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["earlier turn", "u", "correct answer"],
        "the aborted attempt is replaced, the prior turn preserved, no duplicate run tail"
    );
}

#[tokio::test]
async fn reconcile_run_tail_suffix_appends_despite_sdk_divergence_no_rewrite() {
    let store = ThreadTranscriptStore::memory();
    // The initial streaming flush committed the user row before the SDK session
    // bound, so its metadata.sdk_session_id is null.
    store
        .append_committed_messages(
            "thread::rec-sdk",
            Some("run-1"),
            &[json!({"role":"user","content":"u","metadata":{"sdk_session_id":null}})],
        )
        .await
        .unwrap();

    // Terminal authoritative set rebuilds the user row WITH the now-bound session
    // id and adds the assistant reply that was in flight.
    let authoritative = [
        json!({"role":"user","content":"u","metadata":{"sdk_session_id":"sess-9"}}),
        json!({"role":"assistant","content":"hi","metadata":{}}),
    ];
    let result = store
        .reconcile_run_tail("thread::rec-sdk", "run-1", &authoritative)
        .await
        .unwrap();
    assert_eq!(result.total_messages, 2);

    let records = store.records("thread::rec-sdk").await.unwrap();
    // The user row was NOT rewritten: seq preserved, original (null) sdk kept —
    // only the assistant suffix was appended (the cheap path, not a full rewrite).
    assert_eq!(records[0].seq, 1);
    assert!(records[0].message["metadata"]["sdk_session_id"].is_null());
    assert_eq!(records[1].seq, 2);
    assert_eq!(records[1].message["content"], "hi");
}

#[tokio::test]
async fn reconcile_run_tail_noop_when_only_sdk_differs() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_committed_messages(
            "thread::rec-sdk-noop",
            Some("run-1"),
            &[json!({"role":"user","content":"u","metadata":{"sdk_session_id":null}})],
        )
        .await
        .unwrap();
    let authoritative =
        [json!({"role":"user","content":"u","metadata":{"sdk_session_id":"sess-9"}})];
    let result = store
        .reconcile_run_tail("thread::rec-sdk-noop", "run-1", &authoritative)
        .await
        .unwrap();
    assert_eq!(result.total_messages, 1);
    let records = store.records("thread::rec-sdk-noop").await.unwrap();
    assert_eq!(records.len(), 1, "no duplicate row");
    assert!(records[0].message["metadata"]["sdk_session_id"].is_null());
}

#[tokio::test]
async fn reconcile_run_tail_empty_run_id_is_noop_not_double_append() {
    let store = ThreadTranscriptStore::memory();
    // Worker already appended this run's rows (run_id-less is unreachable via the
    // bridge, but the public primitive must not double-write).
    store
        .append_committed_messages(
            "thread::rec-empty",
            None,
            &[
                json!({"role":"user","content":"u"}),
                json!({"role":"assistant","content":"a"}),
            ],
        )
        .await
        .unwrap();
    let authoritative = [
        json!({"role":"user","content":"u"}),
        json!({"role":"assistant","content":"a"}),
    ];
    let result = store
        .reconcile_run_tail("thread::rec-empty", "", &authoritative)
        .await
        .unwrap();
    assert_eq!(result.total_messages, 2);
    let records = store.records("thread::rec-empty").await.unwrap();
    assert_eq!(
        records.len(),
        2,
        "must not re-append the whole run without a run_id"
    );
}

fn draft(message: serde_json::Value) -> RunTranscriptRecordDraft {
    RunTranscriptRecordDraft::from_message(message)
}

fn control_draft(kind: &str, at: &str) -> RunTranscriptRecordDraft {
    RunTranscriptRecordDraft::with_timestamp(
        json!({
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": {
                "kind": kind,
                "thread_id": "thread::control-aware",
                "run_id": "run-1",
                "at": at,
            }
        }),
        at,
    )
}

#[tokio::test]
async fn append_run_records_persists_control_and_content_gaplessly() {
    let store = ThreadTranscriptStore::memory();
    let result = store
        .append_run_records(
            "thread::control-aware",
            Some("run-1"),
            &[
                control_draft("run_start", "2026-06-18T12:00:00Z"),
                draft(json!({"role": "user", "content": "hello"})),
                control_draft("done", "2026-06-18T12:00:01Z"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(result.appended_records.len(), 3);

    let records = store.records("thread::control-aware").await.unwrap();
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(records[0].message["kind"], "control");
    assert_eq!(records[2].message["control"]["kind"], "done");
}

#[tokio::test]
async fn reconcile_run_records_tail_preserves_control_records_and_appends_terminal() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::control-aware",
            Some("run-1"),
            &[
                control_draft("run_start", "2026-06-18T12:00:00Z"),
                draft(json!({"role": "user", "content": "hello"})),
                control_draft("done", "2026-06-18T12:00:01Z"),
            ],
        )
        .await
        .unwrap();

    let result = store
        .reconcile_run_records_tail(
            "thread::control-aware",
            "run-1",
            &[
                control_draft("run_start", "2026-06-18T12:00:09Z"),
                draft(json!({"role": "user", "content": "hello"})),
                control_draft("done", "2026-06-18T12:00:10Z"),
                control_draft("run_complete", "2026-06-18T12:00:11Z"),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        result
            .appended_records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![1, 3, 4, 5],
        "control payload changes are same-seq overwrites, terminal is a suffix append, and a marker makes overwrites reconnect-visible"
    );

    let records = store.records("thread::control-aware").await.unwrap();
    assert_eq!(records.len(), 5);
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(records[0].message["control"]["at"], "2026-06-18T12:00:09Z");
    assert_eq!(records[2].message["control"]["at"], "2026-06-18T12:00:10Z");
    assert_eq!(records[3].message["control"]["kind"], "run_complete");
    assert_eq!(records[4].message["control"]["kind"], "range_rewrite");
    assert_eq!(records[4].message["control"]["start_seq"], 1);
    assert_eq!(records[4].message["control"]["end_seq"], 3);
    assert_eq!(
        records[4].message["control"]["reason"],
        "same_seq_overwrite"
    );
}

#[tokio::test]
async fn reconcile_run_records_tail_preserves_user_origin_id() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::origin-reconcile",
            Some("run-1"),
            &[
                control_draft("run_start", "2026-06-18T12:00:00Z"),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "user",
                        "content": "hello",
                        "metadata": {
                            "origin_id": "00000000-0000-0000-0000-000000000001"
                        }
                    }),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "answer"}),
                    "2026-06-18T12:00:02Z",
                ),
            ],
        )
        .await
        .unwrap();

    let result = store
        .reconcile_run_records_tail(
            "thread::origin-reconcile",
            "run-1",
            &[
                control_draft("run_start", "2026-06-18T12:00:00Z"),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "user",
                        "content": "hello",
                        "timestamp": "2026-06-18T12:00:11Z",
                        "metadata": {
                            "origin_id": "00000000-0000-0000-0000-000000000001"
                        }
                    }),
                    "2026-06-18T12:00:11Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "answer"}),
                    "2026-06-18T12:00:02Z",
                ),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        result
            .appended_records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![2, 4]
    );

    let records = store.records("thread::origin-reconcile").await.unwrap();
    assert_eq!(
        records[1].message["metadata"]["origin_id"],
        "00000000-0000-0000-0000-000000000001"
    );
    let render = store
        .render_snapshot_at_seq("thread::origin-reconcile", 2)
        .await
        .unwrap();
    let RenderRow::UserTurn(row) = &render.rows[0];
    assert_eq!(
        row.id,
        "user_turn:origin:00000000-0000-0000-0000-000000000001"
    );
}

#[tokio::test]
async fn reconcile_run_records_tail_marks_shrink_with_range_rewrite_not_renumber() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::control-shrink",
            Some("run-1"),
            &[
                control_draft("run_start", "2026-06-18T12:00:00Z"),
                draft(json!({"role": "user", "content": "hello"})),
                draft(json!({"role": "assistant", "content": "extra"})),
            ],
        )
        .await
        .unwrap();

    let result = store
        .reconcile_run_records_tail(
            "thread::control-shrink",
            "run-1",
            &[
                control_draft("run_start", "2026-06-18T12:00:00Z"),
                draft(json!({"role": "user", "content": "hello"})),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        result
            .appended_records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![3, 4],
        "removed content is same-seq overwritten, then a higher-seq marker notifies caught-up readers"
    );

    let records = store.records("thread::control-shrink").await.unwrap();
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "shrink keeps already-issued seqs and appends an explicit control marker"
    );
    assert_eq!(records[2].message["kind"], "control");
    assert_eq!(records[2].message["control"]["kind"], "range_rewrite");
    assert_eq!(records[2].message["control"]["tombstone"], true);
    assert_eq!(records[2].message["control"]["start_seq"], 3);
    assert_eq!(records[2].message["control"]["end_seq"], 3);
    assert_eq!(records[3].message["control"]["kind"], "range_rewrite");
    assert_eq!(records[3].message["control"]["tombstone"], false);

    let second = store
        .reconcile_run_records_tail(
            "thread::control-shrink",
            "run-1",
            &[
                control_draft("run_start", "2026-06-18T12:00:00Z"),
                draft(json!({"role": "user", "content": "hello"})),
            ],
        )
        .await
        .unwrap();
    assert!(
        second.appended_records.is_empty(),
        "an already-materialized rewrite must not append another marker"
    );
    let after_second = store.records("thread::control-shrink").await.unwrap();
    assert_eq!(
        after_second
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
}

#[tokio::test]
async fn records_after_seq_returns_delta_ascending_and_handles_overflow() {
    let store = ThreadTranscriptStore::memory();
    let msgs: Vec<_> = (0..10)
        .map(|i| json!({"role":"assistant","content":format!("m{i}")}))
        .collect();
    store
        .append_committed_messages("thread::seq", Some("run-1"), &msgs)
        .await
        .unwrap();
    // seqs are 1..=10. after_seq=7 → seq 8,9,10 ascending.
    let delta = store
        .records_after_seq("thread::seq", 7, 100)
        .await
        .unwrap();
    assert_eq!(
        delta.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![8, 9, 10]
    );
    assert_eq!(delta[0].message["content"], "m7");
    // caught up → empty
    assert!(
        store
            .records_after_seq("thread::seq", 10, 100)
            .await
            .unwrap()
            .is_empty()
    );
    // after_seq=0 → all
    assert_eq!(
        store
            .records_after_seq("thread::seq", 0, 100)
            .await
            .unwrap()
            .len(),
        10
    );
    // limit smaller than delta → NEWEST `limit`, ascending (keeps the stream's
    // live handoff gapless; older history pages in via before_index)
    let capped = store.records_after_seq("thread::seq", 0, 3).await.unwrap();
    assert_eq!(
        capped.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![8, 9, 10]
    );
    // unknown thread → empty
    assert!(
        store
            .records_after_seq("thread::nope", 0, 100)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn file_records_after_seq_tail_scan_does_not_parse_old_head() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::file-tail-scan";
    let path = store.transcript_path(thread_id).unwrap();
    let mut lines = vec!["not-json-old-head".to_owned()];
    for seq in 1..=5 {
        lines.push(
            serde_json::to_string(&TranscriptLine::Message {
                seq,
                thread_id: thread_id.to_owned(),
                run_id: Some("run-tail".to_owned()),
                timestamp: format!("2026-06-18T12:00:0{seq}Z"),
                message: json!({"role": "assistant", "content": format!("m{seq}")}),
            })
            .unwrap(),
        );
    }
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    let tail = store.records_after_seq(thread_id, 3, 10).await.unwrap();
    assert_eq!(
        tail.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![4, 5]
    );
    assert!(
        store.records_after_seq(thread_id, 0, 10).await.is_err(),
        "a full-range tail scan still validates malformed lines it reaches"
    );
}

#[tokio::test]
async fn records_after_seq_page_returns_oldest_records_after_cursor() {
    let store = ThreadTranscriptStore::memory();
    for seq in 1..=5 {
        store
            .append_committed_messages(
                "thread::page",
                Some("run-page"),
                &[json!({"role": "assistant", "content": format!("m{seq}")})],
            )
            .await
            .unwrap();
    }

    let page = store
        .records_after_seq_page("thread::page", 1, 2)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![2, 3]
    );

    let tail = store.records_after_seq("thread::page", 1, 2).await.unwrap();
    assert_eq!(
        tail.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![4, 5]
    );
}

#[tokio::test]
async fn records_for_run_after_seq_pages_oldest_matching_run_records() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_committed_messages(
            "thread::run-page",
            Some("run-other"),
            &[json!({"role": "assistant", "content": "other"})],
        )
        .await
        .unwrap();
    store
        .append_committed_messages(
            "thread::run-page",
            Some("run-target"),
            &[
                json!({"role": "assistant", "content": "a"}),
                json!({"role": "assistant", "content": "b"}),
                json!({"role": "assistant", "content": "c"}),
            ],
        )
        .await
        .unwrap();

    let page = store
        .records_for_run_after_seq("thread::run-page", "run-target", 0, 2)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[tokio::test]
async fn rewrite_from_messages_preserves_seq_and_marks_shrink() {
    let store = ThreadTranscriptStore::memory();
    store
        .rewrite_from_messages(
            "thread::rewrite-stable",
            &[
                json!({"role": "user", "content": "one"}),
                json!({"role": "assistant", "content": "two"}),
                json!({"role": "assistant", "content": "remove me"}),
            ],
        )
        .await
        .unwrap();

    store
        .rewrite_from_messages(
            "thread::rewrite-stable",
            &[
                json!({"role": "user", "content": "one"}),
                json!({"role": "assistant", "content": "two"}),
            ],
        )
        .await
        .unwrap();

    let records = store.records("thread::rewrite-stable").await.unwrap();
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(records[2].message["control"]["kind"], "range_rewrite");
    assert_eq!(records[2].message["control"]["tombstone"], true);
    assert!(records[2].run_id.is_none());
    assert_eq!(records[3].message["control"]["kind"], "range_rewrite");
    assert_eq!(
        records[3].message["control"]["reason"],
        "rewrite_from_messages_shrink"
    );

    store
        .rewrite_from_messages(
            "thread::rewrite-stable",
            &[
                json!({"role": "user", "content": "one"}),
                json!({"role": "assistant", "content": "two"}),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .records("thread::rewrite-stable")
            .await
            .unwrap()
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "reapplying the same import must not append another marker"
    );
}

#[tokio::test]
async fn rewrite_from_messages_uses_same_seq_overwrite_marker_for_changed_prefix() {
    let store = ThreadTranscriptStore::memory();
    store
        .rewrite_from_messages(
            "thread::rewrite-overwrite",
            &[
                json!({"role": "user", "content": "one"}),
                json!({"role": "assistant", "content": "two"}),
            ],
        )
        .await
        .unwrap();

    store
        .rewrite_from_messages(
            "thread::rewrite-overwrite",
            &[
                json!({"role": "user", "content": "ONE"}),
                json!({"role": "assistant", "content": "two"}),
                json!({"role": "assistant", "content": "three"}),
            ],
        )
        .await
        .unwrap();

    let records = store.records("thread::rewrite-overwrite").await.unwrap();
    assert_eq!(
        records.iter().map(|record| record.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(records[0].message["content"], "ONE");
    assert_eq!(records[2].message["content"], "three");
    assert_eq!(records[3].message["control"]["kind"], "range_rewrite");
    assert_eq!(
        records[3].message["control"]["reason"],
        "same_seq_overwrite"
    );
    assert!(records[3].run_id.is_none());
}
