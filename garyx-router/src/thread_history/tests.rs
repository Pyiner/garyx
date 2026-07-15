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
        .await
        .unwrap();
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
        .await
        .unwrap();
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
    assert_eq!(row_ref_ids(&snapshot), vec!["seq:1", "seq:2"]);
    assert!(
        !row_ref_ids(&snapshot).iter().any(|id| id == "seq:3"),
        "render snapshot must not include future records beyond the frame seq"
    );
}

#[tokio::test]
async fn render_snapshot_at_seq_does_not_backfill_capsule_before_marker() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::render-capsule-bound",
            Some("run-render-capsule"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "create capsule"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "before marker"}),
                    "2026-06-18T12:00:01Z",
                ),
                control_draft_with_payload(
                    "capsule_attached",
                    "2026-06-18T12:00:02Z",
                    json!({
                        "capsule_id": "01900000-0000-7000-8000-000000000701",
                        "revision": 1,
                        "action": "created",
                        "title": "Snapshot Capsule"
                    }),
                ),
            ],
        )
        .await
        .unwrap();

    let before_marker = store
        .render_snapshot_at_seq("thread::render-capsule-bound", 2)
        .await
        .unwrap();
    let after_marker = store
        .render_snapshot_at_seq("thread::render-capsule-bound", 3)
        .await
        .unwrap();

    assert_eq!(before_marker.based_on_seq, 2);
    assert!(first_capsule_cards(&before_marker).is_empty());
    assert_eq!(after_marker.based_on_seq, 3);
    assert_eq!(first_capsule_cards(&after_marker).len(), 1);
}

#[tokio::test]
async fn render_snapshot_in_window_omits_capsule_marker_below_floor() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::render-capsule-window",
            Some("run-render-capsule-window"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "create capsule"}),
                    "2026-06-18T12:00:00Z",
                ),
                control_draft_with_payload(
                    "capsule_attached",
                    "2026-06-18T12:00:01Z",
                    json!({
                        "capsule_id": "01900000-0000-7000-8000-000000000702",
                        "revision": 1,
                        "action": "created",
                        "title": "Window Capsule"
                    }),
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "created"}),
                    "2026-06-18T12:00:02Z",
                ),
            ],
        )
        .await
        .unwrap();

    let snapshot = store
        .render_snapshot_in_window("thread::render-capsule-window", 3, 3)
        .await
        .unwrap();

    assert_eq!(snapshot.based_on_seq, 3);
    assert_eq!(row_ref_ids(&snapshot), vec!["seq:3"]);
    assert!(first_capsule_cards(&snapshot).is_empty());
    assert_eq!(
        snapshot.window,
        Some(garyx_models::RenderWindow {
            floor_seq: 3,
            has_more_above: true,
        })
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
    assert_eq!(row_ref_ids(&snapshot), vec!["seq:3", "seq:4"]);
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

    assert_eq!(row_ref_ids(&snapshot), vec!["seq:2"]);
    assert_eq!(
        snapshot.tail_activity,
        garyx_models::RenderTailActivity::Thinking,
        "run_state must come from the full prefix, not only window records"
    );
}

/// Regression reproduction for the captured mobile transcript shape: the
/// cold-open floor landed on a task notification between a pending tool use
/// and its late result. Re-reducing only records at/after the floor must not
/// move that result from the pre-floor turn into the notification turn. A
/// row changing ownership when the render floor changes gives SwiftUI a
/// structurally different row under the same id and can leave the old card
/// visually detached while later rows continue to update.
#[tokio::test]
async fn render_window_does_not_reparent_cross_floor_tool_result_to_task_notification() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::render-window-task-notification",
            Some("run-render-window-task-notification"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "Earlier request"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "tool_use",
                        "kind": "tool_trace",
                        "tool_use_id": "tool-window-boundary",
                        "content": {
                            "tool": "Bash",
                            "input": {"command": "true"}
                        }
                    }),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "user",
                        "content": "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-42\" status=\"in_review\">\nTask #TASK-42 is ready for review: Test Review\n\nReview complete.\n</garyx_task_notification>"
                    }),
                    "2026-06-18T12:00:02Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "tool_result",
                        "kind": "tool_trace",
                        "tool_use_id": "tool-window-boundary",
                        "content": {"result": {"stdout": "done"}}
                    }),
                    "2026-06-18T12:00:03Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "Notification handled"}),
                    "2026-06-18T12:00:04Z",
                ),
            ],
        )
        .await
        .unwrap();

    let full = store
        .render_snapshot_at_seq("thread::render-window-task-notification", 5)
        .await
        .unwrap();
    assert_eq!(
        row_ref_ids(&full),
        vec!["seq:1", "seq:2", "seq:4", "seq:3", "seq:5"],
        "the full reducer repairs the late result into its original pre-floor tool group"
    );

    let window = store
        .render_snapshot_in_window("thread::render-window-task-notification", 3, 5)
        .await
        .unwrap();
    assert_eq!(
        row_ref_ids(&window),
        vec!["seq:3", "seq:5"],
        "narrowing at the notification must not reparent seq:4 into that turn"
    );
}

#[tokio::test]
async fn rolled_render_prefix_checkpoint_preserves_hidden_tool_owner() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file_for_tests(dir.path(), 1 << 20, 3, 1 << 20)
        .await
        .unwrap();
    let thread_id = "thread::rolled-render-prefix";
    store
        .append_run_records(
            thread_id,
            Some("run-rolled-render-prefix"),
            &[
                RunTranscriptRecordDraft::from_message(
                    json!({"role": "user", "content": "Earlier request"}),
                ),
                RunTranscriptRecordDraft::from_message(json!({
                    "role": "tool_use",
                    "kind": "tool_trace",
                    "tool_use_id": "tool-rolled-boundary",
                    "content": {"tool": "Bash", "input": {"command": "true"}}
                })),
                RunTranscriptRecordDraft::from_message(
                    json!({"role": "assistant", "content": "Waiting for the result"}),
                ),
                RunTranscriptRecordDraft::from_message(
                    json!({"role": "assistant", "content": "Still working"}),
                ),
                RunTranscriptRecordDraft::from_message(
                    json!({"role": "assistant", "content": "One more update"}),
                ),
                RunTranscriptRecordDraft::from_message(
                    json!({"role": "user", "content": "Task notification"}),
                ),
                RunTranscriptRecordDraft::from_message(json!({
                    "role": "tool_result",
                    "kind": "tool_trace",
                    "tool_use_id": "tool-rolled-boundary",
                    "content": {"result": {"stdout": "done"}}
                })),
                RunTranscriptRecordDraft::from_message(
                    json!({"role": "assistant", "content": "Notification handled"}),
                ),
            ],
        )
        .await
        .unwrap();

    let full = store.render_snapshot_at_seq(thread_id, 8).await.unwrap();
    assert_eq!(
        row_ref_ids(&full),
        vec![
            "seq:1", "seq:2", "seq:7", "seq:3", "seq:4", "seq:5", "seq:6", "seq:8"
        ],
        "full reduction is the ownership ground truth"
    );

    let (base_seq, tail_start_seq, render_prefix_bytes) = store
        .render_cache_checkpoint_debug(thread_id)
        .await
        .expect("append path keeps a cache entry");
    assert!(base_seq > 0, "test must exercise a rolled checkpoint");
    assert_eq!(tail_start_seq, 6, "floor must start at cached tail");
    assert!(
        render_prefix_bytes > 0,
        "the rolled checkpoint must retain the hidden pending owner"
    );

    let baseline = store.full_file_reads.load(CacheTestOrdering::Relaxed);
    let window = store
        .render_snapshot_in_window(thread_id, 6, 8)
        .await
        .unwrap();
    assert_eq!(row_ref_ids(&window), vec!["seq:6", "seq:8"]);
    assert_eq!(
        store.full_file_reads.load(CacheTestOrdering::Relaxed),
        baseline,
        "cache-eligible render must use the rolled checkpoint, not a full-file fallback"
    );
}

#[tokio::test]
async fn cold_open_user_turn_window_selects_newest_user_turn() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::cold-window",
            Some("run-cold-window"),
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

    let window = store
        .cold_open_user_turn_window("thread::cold-window", 1, THREAD_TRANSCRIPT_REPLAY_CAP)
        .await
        .unwrap();

    assert_eq!(window.floor_seq, 3);
    assert!(window.has_more_above);
    assert_eq!(
        window
            .records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
}

#[tokio::test]
async fn cold_open_user_turn_window_excludes_loop_continuation() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::cold-window-loop-continuation",
            Some("run-cold-window-loop-continuation"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "real question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({
                        "role": "user",
                        "content": "loop continuation",
                        "internal_kind": "loop_continuation"
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

    let window = store
        .cold_open_user_turn_window(
            "thread::cold-window-loop-continuation",
            1,
            THREAD_TRANSCRIPT_REPLAY_CAP,
        )
        .await
        .unwrap();

    assert_eq!(window.floor_seq, 1);
    assert_eq!(
        window
            .records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[tokio::test]
async fn cold_open_user_turn_window_falls_back_without_user_turns() {
    let store = ThreadTranscriptStore::memory();
    store
        .append_run_records(
            "thread::cold-window-no-user",
            Some("run-cold-window-no-user"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "one"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "two"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "three"}),
                    "2026-06-18T12:00:02Z",
                ),
            ],
        )
        .await
        .unwrap();

    let window = store
        .cold_open_user_turn_window("thread::cold-window-no-user", 1, 2)
        .await
        .unwrap();

    assert_eq!(window.floor_seq, 2);
    assert!(window.has_more_above);
    assert_eq!(
        window
            .records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![2, 3]
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
        .await
        .unwrap();
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
        .await
        .unwrap();
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
        .await
        .unwrap();
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

fn draft(message: serde_json::Value) -> RunTranscriptRecordDraft {
    RunTranscriptRecordDraft::from_message(message)
}

/// Every message-ref id the snapshot's row tree references — the
/// "which messages does this snapshot render" oracle.
fn row_ref_ids(snapshot: &garyx_models::RenderSnapshot) -> Vec<String> {
    use garyx_models::{RenderActivityRow, RenderRow, RenderStepItem};
    let mut ids = Vec::new();
    for row in &snapshot.rows {
        let RenderRow::UserTurn(row) = row;
        if let Some(user) = &row.user {
            ids.push(user.id.clone());
        }
        for activity in &row.activity {
            match activity {
                RenderActivityRow::AssistantReply(reply) => ids.push(reply.message.id.clone()),
                RenderActivityRow::Step(step) => {
                    for item in &step.steps {
                        match item {
                            RenderStepItem::AssistantMessage(message) => {
                                ids.push(message.message.id.clone());
                            }
                            RenderStepItem::ToolGroup(group) => {
                                for entry in &group.entries {
                                    if let Some(tool_use) = &entry.tool_use {
                                        ids.push(tool_use.id.clone());
                                    }
                                    if let Some(tool_result) = &entry.tool_result {
                                        ids.push(tool_result.id.clone());
                                    }
                                }
                            }
                        }
                    }
                    if let Some(final_message) = &step.final_message {
                        ids.push(final_message.id.clone());
                    }
                }
            }
        }
    }
    ids
}

fn first_capsule_cards(
    snapshot: &garyx_models::RenderSnapshot,
) -> Vec<garyx_models::RenderCapsuleCard> {
    snapshot
        .rows
        .first()
        .map(|row| match row {
            garyx_models::RenderRow::UserTurn(row) => row.capsule_cards.clone(),
        })
        .unwrap_or_default()
}

fn control_draft_with_payload(
    kind: &str,
    at: &str,
    payload: serde_json::Value,
) -> RunTranscriptRecordDraft {
    let mut control = serde_json::Map::new();
    control.insert("kind".to_owned(), json!(kind));
    control.insert("thread_id".to_owned(), json!("thread::control-aware"));
    control.insert("run_id".to_owned(), json!("run-control"));
    control.insert("at".to_owned(), json!(at));
    if let Some(payload) = payload.as_object() {
        for (key, value) in payload {
            control.insert(key.clone(), value.clone());
        }
    }
    RunTranscriptRecordDraft::with_timestamp(
        json!({
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": control,
        }),
        at,
    )
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

fn transcript_session_line(thread_id: &str, version: u32) -> String {
    serde_json::to_string(&json!({
        "type": "session",
        "version": version,
        "thread_id": thread_id,
        "created_at": "2026-07-15T00:00:00Z"
    }))
    .unwrap()
}

fn transcript_message_line(
    thread_id: &str,
    seq: u64,
    run_id: Option<&str>,
    timestamp: &str,
    message: Value,
) -> String {
    serde_json::to_string(&json!({
        "type": "message",
        "seq": seq,
        "thread_id": thread_id,
        "run_id": run_id,
        "timestamp": timestamp,
        "message": message
    }))
    .unwrap()
}

async fn write_raw_transcript(store: &ThreadTranscriptStore, thread_id: &str, raw: &[u8]) {
    tokio::fs::write(store.transcript_path(thread_id).unwrap(), raw)
        .await
        .unwrap();
}

#[tokio::test]
async fn legacy_backfill_absent_and_empty_is_atomic_and_idempotent() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let legacy = vec![
        json!({"role": "user", "content": "one"}),
        json!({"role": "assistant", "content": "two"}),
    ];

    for thread_id in ["thread::absent-backfill", "thread::empty-backfill"] {
        if thread_id.contains("empty") {
            write_raw_transcript(&store, thread_id, b"").await;
        }
        assert_eq!(
            store
                .ensure_transcript_backfilled(thread_id, &legacy)
                .await
                .unwrap(),
            BackfillOutcome::Backfilled
        );
        let first = tokio::fs::read(store.transcript_path(thread_id).unwrap())
            .await
            .unwrap();
        assert!(first.ends_with(b"\n"));
        assert_eq!(store.records(thread_id).await.unwrap().len(), 2);
        assert_eq!(
            store
                .ensure_transcript_backfilled(thread_id, &legacy)
                .await
                .unwrap(),
            BackfillOutcome::AlreadyComplete
        );
        assert_eq!(
            tokio::fs::read(store.transcript_path(thread_id).unwrap())
                .await
                .unwrap(),
            first
        );
    }
}

#[tokio::test]
async fn legacy_backfill_header_only_completes_once() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::header-only";
    write_raw_transcript(
        &store,
        thread_id,
        format!("{}\n", transcript_session_line(thread_id, 1)).as_bytes(),
    )
    .await;
    let legacy = vec![json!({"role": "user", "content": "no timestamp"})];

    assert_eq!(
        store
            .ensure_transcript_backfilled(thread_id, &legacy)
            .await
            .unwrap(),
        BackfillOutcome::Backfilled
    );
    let completed = tokio::fs::read(store.transcript_path(thread_id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        store
            .ensure_transcript_backfilled(thread_id, &legacy)
            .await
            .unwrap(),
        BackfillOutcome::AlreadyComplete
    );
    assert_eq!(
        tokio::fs::read(store.transcript_path(thread_id).unwrap())
            .await
            .unwrap(),
        completed
    );
}

#[tokio::test]
async fn legacy_backfill_repairs_identity_prefix_torn_tail_and_retries_cleanly() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::torn-prefix";
    let first = json!({"role": "user", "content": "one"});
    let raw = format!(
        "{}\n{}\n{{\"type\":\"message\",\"seq\":2",
        transcript_session_line(thread_id, 1),
        transcript_message_line(
            thread_id,
            7,
            Some("legacy-run"),
            "2026-07-15T01:02:03Z",
            first.clone(),
        )
    );
    write_raw_transcript(&store, thread_id, raw.as_bytes()).await;
    let legacy = vec![first, json!({"role": "assistant", "content": "two"})];

    assert_eq!(
        store
            .ensure_transcript_backfilled(thread_id, &legacy)
            .await
            .unwrap(),
        BackfillOutcome::Backfilled
    );
    let repaired = store.records(thread_id).await.unwrap();
    assert_eq!(repaired.len(), 2);
    assert_eq!(repaired[0].seq, 7);
    assert_eq!(repaired[0].run_id.as_deref(), Some("legacy-run"));
    assert_eq!(repaired[0].timestamp, "2026-07-15T01:02:03Z");
    let bytes = tokio::fs::read(store.transcript_path(thread_id).unwrap())
        .await
        .unwrap();
    assert!(bytes.ends_with(b"\n"));
    assert_eq!(
        store
            .ensure_transcript_backfilled(thread_id, &legacy)
            .await
            .unwrap(),
        BackfillOutcome::AlreadyComplete
    );

    let missing_newline_id = "thread::missing-newline";
    let only_message = json!({"role": "user", "content": "complete json"});
    let missing_newline = format!(
        "{}\n{}",
        transcript_session_line(missing_newline_id, 1),
        transcript_message_line(
            missing_newline_id,
            1,
            None,
            "2026-07-15T02:03:04Z",
            only_message.clone(),
        )
    );
    write_raw_transcript(&store, missing_newline_id, missing_newline.as_bytes()).await;
    assert_eq!(
        store
            .ensure_transcript_backfilled(missing_newline_id, &[only_message])
            .await
            .unwrap(),
        BackfillOutcome::Backfilled
    );
    assert!(
        tokio::fs::read(store.transcript_path(missing_newline_id).unwrap())
            .await
            .unwrap()
            .ends_with(b"\n")
    );
}

#[tokio::test]
async fn legacy_backfill_rejects_middle_garbage_wrong_record_thread_nonmonotonic_and_duplicate_header()
 {
    let legacy = vec![json!({"role": "user", "content": "one"})];
    let cases = [
        (
            "thread::garbage-middle",
            format!(
                "{}\nnot-json\n{}\n",
                transcript_session_line("thread::garbage-middle", 1),
                transcript_message_line(
                    "thread::garbage-middle",
                    1,
                    None,
                    "2026-07-15T00:00:01Z",
                    legacy[0].clone(),
                )
            ),
        ),
        (
            "thread::wrong-record",
            format!(
                "{}\n{}\n",
                transcript_session_line("thread::wrong-record", 1),
                transcript_message_line(
                    "thread::other",
                    1,
                    None,
                    "2026-07-15T00:00:01Z",
                    legacy[0].clone(),
                )
            ),
        ),
        (
            "thread::nonmonotonic",
            format!(
                "{}\n{}\n{}\n",
                transcript_session_line("thread::nonmonotonic", 1),
                transcript_message_line(
                    "thread::nonmonotonic",
                    2,
                    None,
                    "2026-07-15T00:00:01Z",
                    legacy[0].clone(),
                ),
                transcript_message_line(
                    "thread::nonmonotonic",
                    2,
                    None,
                    "2026-07-15T00:00:02Z",
                    json!({"role": "assistant", "content": "two"}),
                )
            ),
        ),
        (
            "thread::duplicate-header",
            format!(
                "{}\n{}\n",
                transcript_session_line("thread::duplicate-header", 1),
                transcript_session_line("thread::duplicate-header", 1),
            ),
        ),
    ];

    for (thread_id, raw) in cases {
        let dir = tempdir().unwrap();
        let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        write_raw_transcript(&store, thread_id, raw.as_bytes()).await;
        let before = tokio::fs::read(store.transcript_path(thread_id).unwrap())
            .await
            .unwrap();
        assert!(matches!(
            store.ensure_transcript_backfilled(thread_id, &legacy).await,
            Err(ThreadHistoryError::InvalidTranscript { .. })
        ));
        assert_eq!(
            tokio::fs::read(store.transcript_path(thread_id).unwrap())
                .await
                .unwrap(),
            before,
            "{thread_id} must not be overwritten"
        );
    }
}

#[tokio::test]
async fn legacy_backfill_rejects_wrong_header_thread_and_unsupported_version() {
    let legacy = vec![json!({"role": "user", "content": "one"})];
    for (thread_id, header) in [
        (
            "thread::wrong-header",
            transcript_session_line("thread::other", 1),
        ),
        (
            "thread::unsupported-header",
            transcript_session_line("thread::unsupported-header", 99),
        ),
    ] {
        let dir = tempdir().unwrap();
        let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        let raw = format!("{header}\n");
        write_raw_transcript(&store, thread_id, raw.as_bytes()).await;
        assert!(matches!(
            store.ensure_transcript_backfilled(thread_id, &legacy).await,
            Err(ThreadHistoryError::InvalidTranscript { .. })
        ));
        assert_eq!(
            tokio::fs::read(store.transcript_path(thread_id).unwrap())
                .await
                .unwrap(),
            raw.as_bytes()
        );
    }
}

#[tokio::test]
async fn legacy_backfill_identity_prefix_preserves_existing_record_fields_and_is_idempotent() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::identity-prefix";
    let first_message = json!({"role": "user", "content": "one"});
    let raw = format!(
        "{}\n{}\n",
        transcript_session_line(thread_id, 1),
        transcript_message_line(
            thread_id,
            41,
            Some("kept-run"),
            "2026-07-15T04:05:06Z",
            first_message.clone(),
        )
    );
    write_raw_transcript(&store, thread_id, raw.as_bytes()).await;
    let legacy = vec![
        first_message,
        json!({"role": "assistant", "content": "two"}),
    ];

    assert_eq!(
        store
            .ensure_transcript_backfilled(thread_id, &legacy)
            .await
            .unwrap(),
        BackfillOutcome::Backfilled
    );
    let records = store.records(thread_id).await.unwrap();
    assert_eq!(records[0].seq, 41);
    assert_eq!(records[0].run_id.as_deref(), Some("kept-run"));
    assert_eq!(records[0].timestamp, "2026-07-15T04:05:06Z");
    let completed = tokio::fs::read(store.transcript_path(thread_id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        store
            .ensure_transcript_backfilled(thread_id, &legacy)
            .await
            .unwrap(),
        BackfillOutcome::AlreadyComplete
    );
    assert_eq!(
        tokio::fs::read(store.transcript_path(thread_id).unwrap())
            .await
            .unwrap(),
        completed
    );
}

#[tokio::test]
async fn legacy_backfill_diverged_identity_preserves_existing_bytes() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::diverged";
    let raw = format!(
        "{}\n{}\n",
        transcript_session_line(thread_id, 1),
        transcript_message_line(
            thread_id,
            1,
            Some("runtime-run"),
            "2026-07-15T00:00:01Z",
            json!({"role": "user", "content": "runtime evolved"}),
        )
    );
    write_raw_transcript(&store, thread_id, raw.as_bytes()).await;

    assert_eq!(
        store
            .ensure_transcript_backfilled(
                thread_id,
                &[json!({"role": "user", "content": "stale archive"})],
            )
            .await
            .unwrap(),
        BackfillOutcome::PreservedDiverged
    );
    assert_eq!(
        tokio::fs::read(store.transcript_path(thread_id).unwrap())
            .await
            .unwrap(),
        raw.as_bytes()
    );
}

#[tokio::test]
async fn atomic_replace_stage_failures_have_exact_disk_and_cache_postconditions() {
    for stage in [
        TranscriptReplaceStage::TempWrite,
        TranscriptReplaceStage::FileFsync,
        TranscriptReplaceStage::Rename,
        TranscriptReplaceStage::ParentFsync,
    ] {
        let dir = tempdir().unwrap();
        let store = ThreadTranscriptStore::file_with_atomic_failure_for_tests(dir.path(), stage)
            .await
            .unwrap();
        let thread_id = format!("thread::atomic-{stage}");
        let first = json!({"role": "user", "content": "one"});
        let raw = format!(
            "{}\n{}\n",
            transcript_session_line(&thread_id, 1),
            transcript_message_line(
                &thread_id,
                1,
                Some("old-run"),
                "2026-07-15T00:00:01Z",
                first.clone(),
            )
        );
        write_raw_transcript(&store, &thread_id, raw.as_bytes()).await;
        let legacy = vec![first, json!({"role": "assistant", "content": "two"})];

        let error = store
            .ensure_transcript_backfilled(&thread_id, &legacy)
            .await
            .expect_err("the configured stage fails once");
        assert!(matches!(
            error,
            ThreadHistoryError::AtomicReplace {
                stage: failed_stage,
                ..
            } if failed_stage == stage
        ));
        assert!(
            store
                .render_cache_checkpoint_debug(&thread_id)
                .await
                .is_none(),
            "every failed stage invalidates the cache"
        );
        let after_failure = tokio::fs::read(store.transcript_path(&thread_id).unwrap())
            .await
            .unwrap();
        if stage == TranscriptReplaceStage::ParentFsync {
            assert_ne!(after_failure, raw.as_bytes());
            assert_eq!(store.records(&thread_id).await.unwrap().len(), 2);
        } else {
            assert_eq!(after_failure, raw.as_bytes());
        }

        let retry = store
            .ensure_transcript_backfilled(&thread_id, &legacy)
            .await
            .unwrap();
        assert_eq!(
            retry,
            if stage == TranscriptReplaceStage::ParentFsync {
                BackfillOutcome::AlreadyComplete
            } else {
                BackfillOutcome::Backfilled
            }
        );
        assert_eq!(
            store.provider_session_tail(&thread_id, 10).await.unwrap(),
            legacy
        );
        assert!(
            store
                .render_cache_checkpoint_debug(&thread_id)
                .await
                .is_some(),
            "success is immediately served by the same store cache"
        );
        let final_bytes = tokio::fs::read(store.transcript_path(&thread_id).unwrap())
            .await
            .unwrap();
        assert!(final_bytes.ends_with(b"\n"));
        assert_eq!(store.records(&thread_id).await.unwrap().len(), 2);
    }
}

// ---------------------------------------------------------------------------
// #TASK-1715 knife 2: transcript cache guard tests. The oracle for every
// cached read is the ORIGINAL uncached derivation, recomputed here from a raw
// `records()` full read; `full_file_reads` proves the hot paths actually hit
// the cache instead of silently falling back.
// ---------------------------------------------------------------------------

use std::sync::atomic::Ordering as CacheTestOrdering;

fn oracle_render_in_window(
    records: &[ThreadTranscriptRecord],
    floor_seq: u64,
    based_on_seq: u64,
) -> garyx_models::RenderSnapshot {
    let prefix = records
        .iter()
        .filter(|record| record.seq <= based_on_seq)
        .collect::<Vec<_>>();
    let actual_based_on_seq = prefix.iter().map(|record| record.seq).max().unwrap_or(0);
    let full_values = prefix
        .iter()
        .filter_map(|record| serde_json::to_value(record).ok())
        .collect::<Vec<_>>();
    let run_state = garyx_models::reduce_transcript_run_state(&full_values);
    let render_prefix_values = prefix
        .iter()
        .filter(|record| record.seq < floor_seq)
        .filter_map(|record| serde_json::to_value(record).ok())
        .collect::<Vec<_>>();
    let render_prefix_state =
        garyx_models::reduce_transcript_render_prefix_state(&render_prefix_values);
    let window_values = prefix
        .iter()
        .filter(|record| record.seq >= floor_seq)
        .filter_map(|record| serde_json::to_value(record).ok())
        .collect::<Vec<_>>();
    let mut snapshot = garyx_models::reduce_transcript_render_state_with_prefix_state(
        &window_values,
        &run_state,
        &render_prefix_state,
    );
    if snapshot.based_on_seq == 0 {
        snapshot.based_on_seq = actual_based_on_seq;
    }
    snapshot.window = Some(garyx_models::RenderWindow {
        floor_seq,
        has_more_above: prefix.iter().any(|record| record.seq < floor_seq),
    });
    snapshot
}

fn oracle_run_state(records: &[ThreadTranscriptRecord]) -> garyx_models::TranscriptRunState {
    let values = records
        .iter()
        .filter_map(|record| serde_json::to_value(record).ok())
        .collect::<Vec<_>>();
    garyx_models::reduce_transcript_run_state(&values)
}

fn oracle_cold_open(
    records: &[ThreadTranscriptRecord],
    user_turns: usize,
    cap: usize,
) -> ThreadTranscriptWindow {
    let total = records.len();
    if total == 0 {
        return ThreadTranscriptWindow {
            records: Vec::new(),
            floor_seq: 0,
            has_more_above: false,
        };
    }
    let target = user_turns.max(1);
    let mut start = total;
    let mut user_queries = 0usize;
    while start > 0 && user_queries < target {
        start -= 1;
        if is_user_query_message(&records[start].message) {
            user_queries += 1;
        }
    }
    if user_queries == 0 {
        start = total.saturating_sub(cap.max(1));
    }
    if total.saturating_sub(start) > cap {
        start = total.saturating_sub(cap);
    }
    let window = records[start..].to_vec();
    let floor_seq = window.first().map(|record| record.seq).unwrap_or(0);
    ThreadTranscriptWindow {
        records: window,
        floor_seq,
        has_more_above: start > 0,
    }
}

/// Compare every cache-eligible read of `store` against the uncached oracle
/// derivation for `thread_id`.
async fn assert_reads_match_oracle(store: &ThreadTranscriptStore, thread_id: &str, label: &str) {
    let records = store.records(thread_id).await.unwrap();
    let last_seq = records.last().map(|record| record.seq).unwrap_or(0);
    let mid_seq = records
        .get(records.len() / 2)
        .map(|record| record.seq)
        .unwrap_or(0);

    assert_eq!(
        store.message_count(thread_id).await.unwrap(),
        records.len(),
        "{label}: message_count"
    );
    for limit in [1usize, 3, records.len().max(1), records.len() + 5] {
        let expected: Vec<Value> = records
            .iter()
            .skip(records.len().saturating_sub(limit))
            .map(|record| record.message.clone())
            .collect();
        assert_eq!(
            store.tail(thread_id, limit).await.unwrap(),
            expected,
            "{label}: tail({limit})"
        );
    }
    assert_eq!(
        store.run_state(thread_id).await.unwrap(),
        oracle_run_state(&records),
        "{label}: run_state"
    );
    for (user_turns, cap) in [(1usize, 100usize), (2, 100), (3, 4), (1, 1)] {
        assert_eq!(
            store
                .cold_open_user_turn_window(thread_id, user_turns, cap)
                .await
                .unwrap(),
            oracle_cold_open(&records, user_turns, cap),
            "{label}: cold_open({user_turns},{cap})"
        );
    }
    let mut floors = vec![1u64, mid_seq.max(1), last_seq.max(1), last_seq + 5];
    if let Some(window_floor) = records
        .iter()
        .rev()
        .find(|record| is_user_query_message(&record.message))
        .map(|record| record.seq)
    {
        floors.push(window_floor);
    }
    for floor in floors {
        for based_on in [last_seq, mid_seq.max(1), last_seq + 100] {
            assert_eq!(
                store
                    .render_snapshot_in_window(thread_id, floor, based_on)
                    .await
                    .unwrap(),
                oracle_render_in_window(&records, floor, based_on),
                "{label}: render_snapshot_in_window({floor},{based_on})"
            );
        }
    }
}

fn oracle_test_message(index: usize, kind: usize) -> Value {
    match kind % 4 {
        0 => json!({"role": "user", "content": format!("user message {index}")}),
        1 => json!({"role": "assistant", "content": format!("assistant reply {index}")}),
        2 => json!({
            "role": "system",
            "kind": "control",
            "internal": true,
            "control": {"kind": if index % 2 == 0 { "run_start" } else { "run_complete" },
                         "run_id": format!("run-{}", index / 3)}
        }),
        _ => json!({
            "role": "tool_use",
            "kind": "tool_trace",
            "content": {"tool": "Bash", "input": {"command": format!("echo {index}")}},
            "tool_use_id": format!("tu-{index}")
        }),
    }
}

#[tokio::test]
async fn transcript_cache_matches_uncached_oracle_across_write_paths() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::oracle";

    // Deterministic mixed op sequence: appends of both kinds, reconciles
    // hitting no-op / suffix-grow / same-seq-overwrite / divergent-rewrite /
    // shrink, and a full rewrite_from_messages.
    let mut state = 0x9e3779b97f4a7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut index = 0usize;
    for round in 0..12usize {
        let run_id = format!("run-{round}");
        let batch = 1 + (next() as usize % 4);
        let mut drafts = Vec::new();
        for _ in 0..batch {
            drafts.push(RunTranscriptRecordDraft::with_timestamp(
                oracle_test_message(index, next() as usize),
                format!("2026-03-01T00:{:02}:{:02}Z", round, index % 60),
            ));
            index += 1;
        }
        store
            .append_run_records(thread_id, Some(&run_id), &drafts)
            .await
            .unwrap();
        assert_reads_match_oracle(&store, thread_id, &format!("after append round {round}")).await;

        match next() % 4 {
            // Terminal reconcile no-op: authoritative equals what streamed.
            0 => {
                store
                    .reconcile_run_records_tail(thread_id, &run_id, &drafts)
                    .await
                    .unwrap();
            }
            // Suffix growth: terminal saw one more trailing record.
            1 => {
                let mut grown = drafts.clone();
                grown.push(RunTranscriptRecordDraft::with_timestamp(
                    oracle_test_message(index, 1),
                    format!("2026-03-01T00:{:02}:59Z", round),
                ));
                index += 1;
                store
                    .reconcile_run_records_tail(thread_id, &run_id, &grown)
                    .await
                    .unwrap();
            }
            // Same-seq overwrite: identical identity, changed timestamps.
            2 => {
                let changed: Vec<RunTranscriptRecordDraft> = drafts
                    .iter()
                    .map(|draft| {
                        RunTranscriptRecordDraft::with_timestamp(
                            draft.message.clone(),
                            format!("2026-03-02T11:{:02}:00Z", round),
                        )
                    })
                    .collect();
                store
                    .reconcile_run_records_tail(thread_id, &run_id, &changed)
                    .await
                    .unwrap();
            }
            // Divergent rewrite: retry re-streamed different content.
            _ => {
                let divergent = vec![RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": format!("retry answer {round}")}),
                    format!("2026-03-03T00:{:02}:00Z", round),
                )];
                store
                    .reconcile_run_records_tail(thread_id, &run_id, &divergent)
                    .await
                    .unwrap();
            }
        }
        assert_reads_match_oracle(&store, thread_id, &format!("after reconcile round {round}"))
            .await;
    }

    // Full rewrite path keeps the cache coherent too.
    let messages: Vec<Value> = (0..6).map(|i| oracle_test_message(i, i)).collect();
    store
        .rewrite_from_messages(thread_id, &messages)
        .await
        .unwrap();
    assert_reads_match_oracle(&store, thread_id, "after rewrite_from_messages").await;
}

#[tokio::test]
async fn transcript_cache_hot_paths_do_not_reread_file() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::hot";
    let run_id = "run-hot";

    let mut drafts = Vec::new();
    for index in 0..8usize {
        drafts.push(RunTranscriptRecordDraft::from_message(oracle_test_message(
            index, index,
        )));
    }
    store
        .append_run_records(thread_id, Some(run_id), &drafts)
        .await
        .unwrap();

    // Warm the cache (this may cost one build read), then the steady-state
    // loop below must never re-read the whole file.
    let last_seq = store.records(thread_id).await.unwrap().last().unwrap().seq;
    store
        .render_snapshot_in_window(thread_id, last_seq, last_seq)
        .await
        .unwrap();

    let baseline = store.full_file_reads.load(CacheTestOrdering::Relaxed);
    for step in 0..10u64 {
        let appended = store
            .append_run_records(
                thread_id,
                Some(run_id),
                &[RunTranscriptRecordDraft::from_message(json!({
                    "role": "assistant",
                    "content": format!("hot step {step}")
                }))],
            )
            .await
            .unwrap();
        let seq = appended.appended_records[0].seq;
        store
            .render_snapshot_in_window(thread_id, last_seq, seq)
            .await
            .unwrap();
        store.run_state(thread_id).await.unwrap();
        store.message_count(thread_id).await.unwrap();
        store.tail(thread_id, 3).await.unwrap();
        // Steady-state terminal reconcile: no-op decided from the cached tail.
        let authoritative: Vec<RunTranscriptRecordDraft> = store
            .records(thread_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|record| record.run_id.as_deref() == Some(run_id))
            .map(|record| {
                RunTranscriptRecordDraft::with_timestamp(record.message, record.timestamp)
            })
            .collect();
        store
            .reconcile_run_records_tail(thread_id, run_id, &authoritative)
            .await
            .unwrap();
    }
    // records() full-reads are the oracle's cost (one per loop step above);
    // subtract them: everything else must have been served from the cache.
    let reads = store.full_file_reads.load(CacheTestOrdering::Relaxed) - baseline;
    assert_eq!(
        reads, 10,
        "hot appends/renders/reconciles must not re-read the transcript (only the 10 records() oracle reads may)"
    );

    // Seq continuity across cached appends.
    let records = store.records(thread_id).await.unwrap();
    let seqs: Vec<u64> = records.iter().map(|record| record.seq).collect();
    let expected: Vec<u64> = (1..=records.len() as u64).collect();
    assert_eq!(seqs, expected);
}

#[tokio::test]
async fn render_window_expansion_has_bounded_reads_rows_and_serialized_bytes() {
    const RECORDS: u64 = 12_000;
    const TAIL_CACHE_RECORDS: u64 = 4_096;
    const MAX_PREPAY_RECORDS: u64 = 2_048;
    const EXPANSIONS: usize = 4;

    let dir = tempdir().unwrap();
    let thread_id = "thread::render-window-perf-contract";
    let store = ThreadTranscriptStore::file_for_tests(
        dir.path(),
        8 * 1024 * 1024,
        TAIL_CACHE_RECORDS as usize,
        64 * 1024 * 1024,
    )
    .await
    .unwrap();
    let fixed_body = "x".repeat(128);
    let drafts = (0..RECORDS)
        .map(|index| {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            RunTranscriptRecordDraft::from_message(json!({
                "role": role,
                "content": fixed_body.clone(),
            }))
        })
        .collect::<Vec<_>>();
    store
        .append_run_records(thread_id, Some("run-render-window-perf"), &drafts)
        .await
        .unwrap();

    // The cache retains seq 7905..=12000. Every target below starts one
    // record beneath that tail and then lowers by exactly one prepay-cap span.
    let first_target = RECORDS - TAIL_CACHE_RECORDS;
    let targets = (0..EXPANSIONS)
        .map(|step| first_target - step as u64 * MAX_PREPAY_RECORDS)
        .collect::<Vec<_>>();
    let baseline_reads = store.full_file_reads.load(CacheTestOrdering::Relaxed);
    let mut first_per_row_bound = None;
    let mut previous_record_count = None;
    let mut diagnostics = Vec::new();

    for (step, target_floor) in targets.into_iter().enumerate() {
        let started = std::time::Instant::now();
        let snapshot = store
            .render_snapshot_in_window(thread_id, target_floor, RECORDS)
            .await
            .unwrap();
        let elapsed = started.elapsed();
        let record_count = row_ref_ids(&snapshot).len();
        let serialized_bytes = serde_json::to_vec(&snapshot).unwrap().len();
        diagnostics.push((target_floor, record_count, serialized_bytes, elapsed));

        assert_eq!(
            store.full_file_reads.load(CacheTestOrdering::Relaxed) - baseline_reads,
            step + 1,
            "each below-tail derivation performs exactly one full-file read"
        );
        let inclusive_window_bound = (RECORDS - target_floor + 1) as usize;
        assert!(
            record_count <= inclusive_window_bound,
            "window refs {record_count} exceed inclusive record bound {inclusive_window_bound}"
        );
        if let Some(previous) = previous_record_count {
            assert!(
                record_count.saturating_sub(previous) <= MAX_PREPAY_RECORDS as usize,
                "one floor lowering grew by more than the capped prepay span"
            );
        }

        let per_row_bound = *first_per_row_bound.get_or_insert_with(|| {
            let divisor = record_count.max(1);
            (serialized_bytes + divisor - 1) / divisor
        });
        let self_calibrated_bound = record_count
            .max(1)
            .saturating_mul(per_row_bound)
            .saturating_mul(2);
        assert!(
            serialized_bytes <= self_calibrated_bound,
            "snapshot bytes {serialized_bytes} exceed self-calibrated bound {self_calibrated_bound}"
        );
        previous_record_count = Some(record_count);
    }

    eprintln!(
        "render-window expansion informational timings (floor, refs, bytes, duration): {diagnostics:?}"
    );
}

#[tokio::test]
async fn transcript_cache_survives_tail_roll_and_eviction() {
    let dir = tempdir().unwrap();
    // Tiny budgets: tail rolls constantly, global budget evicts other threads.
    let store = ThreadTranscriptStore::file_for_tests(dir.path(), 512, 3, 1024)
        .await
        .unwrap();

    for thread in 0..3usize {
        let thread_id = format!("thread::roll-{thread}");
        for index in 0..12usize {
            store
                .append_run_records(
                    &thread_id,
                    Some("run-roll"),
                    &[RunTranscriptRecordDraft::from_message(oracle_test_message(
                        index, index,
                    ))],
                )
                .await
                .unwrap();
        }
    }
    for thread in 0..3usize {
        let thread_id = format!("thread::roll-{thread}");
        assert_reads_match_oracle(&store, &thread_id, &format!("rolled thread {thread}")).await;
    }
}

#[tokio::test]
async fn transcript_cache_cold_restart_matches_previous_instance() {
    let dir = tempdir().unwrap();
    let thread_id = "thread::restart";
    {
        let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        for index in 0..7usize {
            store
                .append_run_records(
                    thread_id,
                    Some("run-a"),
                    &[RunTranscriptRecordDraft::from_message(oracle_test_message(
                        index, index,
                    ))],
                )
                .await
                .unwrap();
        }
    }
    let reopened = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    assert_reads_match_oracle(&reopened, thread_id, "reopened store").await;
    // Appends continue the seq chain after a cold rebuild.
    let appended = reopened
        .append_run_records(
            thread_id,
            Some("run-b"),
            &[RunTranscriptRecordDraft::from_message(
                json!({"role": "user", "content": "after restart"}),
            )],
        )
        .await
        .unwrap();
    assert_eq!(appended.appended_records[0].seq, 8);
    assert_reads_match_oracle(&reopened, thread_id, "reopened store after append").await;
}

#[tokio::test]
async fn cold_start_cache_build_streams_without_full_file_read() {
    let dir = tempdir().unwrap();
    let thread_id = "thread::cold-start-stream";
    {
        let writer = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        for index in 0..12usize {
            writer
                .append_run_records(
                    thread_id,
                    Some("run-cold"),
                    &[RunTranscriptRecordDraft::from_message(oracle_test_message(
                        index, index,
                    ))],
                )
                .await
                .unwrap();
        }
    }

    // A fresh store instance is the post-restart cold-cache case: the first
    // touch must stream the cache build instead of materializing the file.
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let baseline = store.full_file_reads.load(CacheTestOrdering::Relaxed);

    let window = store
        .cold_open_user_turn_window(thread_id, 3, 100)
        .await
        .unwrap();
    assert!(!window.records.is_empty());
    let last_seq = window.records.last().unwrap().seq;
    let snapshot = store
        .render_snapshot_in_window(thread_id, window.floor_seq.max(1), last_seq)
        .await
        .unwrap();
    assert_eq!(snapshot.based_on_seq, last_seq);
    assert_eq!(store.message_count(thread_id).await.unwrap(), 12);

    let reads = store.full_file_reads.load(CacheTestOrdering::Relaxed) - baseline;
    assert_eq!(
        reads, 0,
        "cold-start touches must stream the cache build, never full-read the transcript"
    );

    assert_reads_match_oracle(&store, thread_id, "cold start streaming build").await;
}

#[tokio::test]
async fn cold_start_small_tail_budget_serves_tail_window_without_full_read() {
    // Small tail budget: the streamed cold build keeps only the newest
    // records (older ones fold into the checkpoint), and a render window
    // inside that tail is still served with zero full reads.
    let dir = tempdir().unwrap();
    let thread_id = "thread::cold-start-small-tail";
    {
        let writer = ThreadTranscriptStore::file_for_tests(dir.path(), 512, 3, 1 << 20)
            .await
            .unwrap();
        for index in 0..12usize {
            writer
                .append_run_records(
                    thread_id,
                    Some("run-cold-small"),
                    &[RunTranscriptRecordDraft::from_message(oracle_test_message(
                        index, index,
                    ))],
                )
                .await
                .unwrap();
        }
    }

    let store = ThreadTranscriptStore::file_for_tests(dir.path(), 512, 3, 1 << 20)
        .await
        .unwrap();
    let baseline = store.full_file_reads.load(CacheTestOrdering::Relaxed);

    let total = store.message_count(thread_id).await.unwrap() as u64;
    assert_eq!(total, 12);
    let snapshot = store
        .render_snapshot_in_window(thread_id, total, total)
        .await
        .unwrap();
    assert_eq!(snapshot.based_on_seq, total);

    let reads = store.full_file_reads.load(CacheTestOrdering::Relaxed) - baseline;
    assert_eq!(
        reads, 0,
        "tail-window render on a cold rolled cache must not full-read the transcript"
    );
}

#[tokio::test]
async fn file_paging_matches_full_read_oracle_without_full_reads() {
    // Small tail budget: pages near the head fall below the cached tail and
    // must stream just their range; pages near the end serve from the cache.
    let dir = tempdir().unwrap();
    let thread_id = "thread::paging-stream";
    let store = ThreadTranscriptStore::file_for_tests(dir.path(), 2048, 6, 1 << 20)
        .await
        .unwrap();
    for index in 0..30usize {
        store
            .append_run_records(
                thread_id,
                Some("run-page"),
                &[RunTranscriptRecordDraft::from_message(oracle_test_message(
                    index, index,
                ))],
            )
            .await
            .unwrap();
    }

    // Oracle uses the full-read records() (counted separately below).
    let records = store.records(thread_id).await.unwrap();
    let total = records.len();
    let oracle_slice = |start: usize, end: usize| -> Vec<Value> {
        records[start..end]
            .iter()
            .map(|record| record.message.clone())
            .collect()
    };

    let baseline = store.full_file_reads.load(CacheTestOrdering::Relaxed);

    // Tail page (cache hit), deep page (streamed), forward page, tail-less
    // boundary and out-of-range cursors.
    let cases: Vec<(Option<usize>, usize)> =
        vec![(None, 10), (Some(8), 5), (Some(30), 30), (Some(99), 4)];
    for (before, limit) in cases {
        let (messages, reported_total, start) = store
            .page_before_index(thread_id, before, limit)
            .await
            .unwrap();
        let end = before.unwrap_or(total).min(total);
        let expected_start = end.saturating_sub(limit);
        assert_eq!(reported_total, total, "total for before={before:?}");
        assert_eq!(start, expected_start, "start for before={before:?}");
        assert_eq!(
            messages,
            oracle_slice(expected_start, end),
            "page_before_index({before:?}, {limit})"
        );
    }

    for (after, limit) in [(0usize, 7usize), (12, 100), (29, 5), (99, 5)] {
        let (messages, reported_total, start) = store
            .page_after_index(thread_id, after, limit)
            .await
            .unwrap();
        let expected_start = after.saturating_add(1).min(total);
        let expected_end = expected_start.saturating_add(limit).min(total);
        assert_eq!(reported_total, total);
        assert_eq!(start, expected_start);
        assert_eq!(
            messages,
            oracle_slice(expected_start, expected_end),
            "page_after_index({after}, {limit})"
        );
    }

    for (before, queries, fallback) in [
        (None, 3usize, 10usize),
        (Some(20), 2, 10),
        (Some(20), 50, 10), // more queries than exist -> window from head
        (Some(3), 1, 2),
    ] {
        let (messages, reported_total, start) = store
            .page_before_user_queries(thread_id, before, queries, fallback)
            .await
            .unwrap();
        let end = before.unwrap_or(total).min(total);
        let mut expected_start = end;
        let mut seen = 0usize;
        while expected_start > 0 && seen < queries.max(1) {
            expected_start -= 1;
            if is_user_query_message(&records[expected_start].message) {
                seen += 1;
            }
        }
        if seen == 0 {
            expected_start = end.saturating_sub(fallback.max(1));
        }
        assert_eq!(reported_total, total);
        assert_eq!(
            start, expected_start,
            "start for before={before:?} q={queries}"
        );
        assert_eq!(
            messages,
            oracle_slice(expected_start, end),
            "page_before_user_queries({before:?}, {queries}, {fallback})"
        );
    }

    let reads = store.full_file_reads.load(CacheTestOrdering::Relaxed) - baseline;
    assert_eq!(
        reads, 0,
        "paging must stream ranges or hit the cache, never full-read the transcript"
    );
}

#[tokio::test]
async fn cold_start_deep_page_streams_without_full_read() {
    // Post-restart shape: fresh store, client pages deep history (below the
    // cached tail) on a thread whose transcript is large. Both the cache
    // build and the page itself must avoid full-file reads.
    let dir = tempdir().unwrap();
    let thread_id = "thread::paging-cold";
    {
        let writer = ThreadTranscriptStore::file_for_tests(dir.path(), 512, 3, 1 << 20)
            .await
            .unwrap();
        for index in 0..20usize {
            writer
                .append_run_records(
                    thread_id,
                    Some("run-cold-page"),
                    &[RunTranscriptRecordDraft::from_message(oracle_test_message(
                        index, index,
                    ))],
                )
                .await
                .unwrap();
        }
    }

    let store = ThreadTranscriptStore::file_for_tests(dir.path(), 512, 3, 1 << 20)
        .await
        .unwrap();
    let baseline = store.full_file_reads.load(CacheTestOrdering::Relaxed);

    let (messages, total, start) = store
        .page_before_index(thread_id, Some(6), 4)
        .await
        .unwrap();
    assert_eq!(total, 20);
    assert_eq!(start, 2);
    assert_eq!(messages.len(), 4);

    let reads = store.full_file_reads.load(CacheTestOrdering::Relaxed) - baseline;
    assert_eq!(reads, 0, "cold deep page must stream, not full-read");
}

#[tokio::test]
async fn transcript_cache_detects_out_of_band_file_change() {
    let dir = tempdir().unwrap();
    let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
    let thread_id = "thread::oob";
    store
        .append_run_records(
            thread_id,
            Some("run-oob"),
            &[
                RunTranscriptRecordDraft::from_message(json!({"role": "user", "content": "hi"})),
                RunTranscriptRecordDraft::from_message(
                    json!({"role": "assistant", "content": "hello"}),
                ),
            ],
        )
        .await
        .unwrap();
    // Warm the cache.
    store.run_state(thread_id).await.unwrap();

    // Out-of-band writer appends a record directly to the jsonl.
    let path = store.transcript_path(thread_id).unwrap();
    let line = serde_json::to_string(&TranscriptLine::Message {
        seq: 3,
        thread_id: thread_id.to_owned(),
        run_id: Some("run-oob".to_owned()),
        timestamp: "2026-03-01T00:00:30Z".to_owned(),
        message: json!({"role": "assistant", "content": "sneaky"}),
    })
    .unwrap();
    let mut raw = std::fs::read_to_string(&path).unwrap();
    raw.push_str(&line);
    raw.push('\n');
    std::fs::write(&path, raw).unwrap();

    // The fstat guard must drop the stale entry: reads see the new record and
    // the next append continues after it instead of duplicating seq 3.
    assert_eq!(store.message_count(thread_id).await.unwrap(), 3);
    assert_reads_match_oracle(&store, thread_id, "after out-of-band append").await;
    let appended = store
        .append_run_records(
            thread_id,
            Some("run-oob"),
            &[RunTranscriptRecordDraft::from_message(
                json!({"role": "user", "content": "next"}),
            )],
        )
        .await
        .unwrap();
    assert_eq!(appended.appended_records[0].seq, 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transcript_cache_concurrent_threads_do_not_block_or_corrupt() {
    let dir = tempdir().unwrap();
    let store = std::sync::Arc::new(ThreadTranscriptStore::file(dir.path()).await.unwrap());

    let mut handles = Vec::new();
    for thread in 0..4usize {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            let thread_id = format!("thread::conc-{thread}");
            for index in 0..25usize {
                store
                    .append_run_records(
                        &thread_id,
                        Some("run-conc"),
                        &[RunTranscriptRecordDraft::from_message(oracle_test_message(
                            index, index,
                        ))],
                    )
                    .await
                    .unwrap();
                if index % 5 == 0 {
                    let _ = store
                        .render_snapshot_in_window(&thread_id, 1 + index as u64, 60)
                        .await;
                    let _ = store.run_state(&thread_id).await;
                }
            }
        }));
    }
    for handle in handles {
        tokio::time::timeout(std::time::Duration::from_secs(30), handle)
            .await
            .expect("concurrent transcript ops deadlocked")
            .unwrap();
    }

    for thread in 0..4usize {
        let thread_id = format!("thread::conc-{thread}");
        let records = store.records(&thread_id).await.unwrap();
        let seqs: Vec<u64> = records.iter().map(|record| record.seq).collect();
        let expected: Vec<u64> = (1..=25u64).collect();
        assert_eq!(seqs, expected, "thread {thread} seq chain broken");
        assert_reads_match_oracle(&store, &thread_id, &format!("concurrent thread {thread}")).await;
    }
}
