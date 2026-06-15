use super::*;
use crate::memory_store::InMemoryThreadStore;
use serde_json::json;

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
async fn repository_overlays_active_run_snapshot() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let transcript_store = Arc::new(ThreadTranscriptStore::memory());
    transcript_store
        .append_committed_messages(
            "thread::overlay",
            Some("run-past"),
            &[json!({"role": "user", "content": "past"})],
        )
        .await
        .unwrap();
    thread_store
        .set(
            "thread::overlay",
            json!({
                "history": {
                    "message_count": 1,
                    "active_run_snapshot": {
                        "run_id": "run-live",
                        "messages": [{"role": "assistant", "content": "live"}]
                    }
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(thread_store, transcript_store);

    let snapshot = repo.thread_snapshot("thread::overlay", 10).await.unwrap();
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0]["content"], "past");
    assert_eq!(combined[1]["content"], "live");
}

#[tokio::test]
async fn transcript_backend_allows_empty_thread_with_live_overlay() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    thread_store
        .set(
            "thread::live-only",
            json!({
                "messages": [],
                "history": {
                    "message_count": 0,
                    "active_run_snapshot": {
                        "run_id": "run-live",
                        "messages": [{"role": "assistant", "content": "live"}]
                    }
                }
            }),
        )
        .await;
    let repo =
        ThreadHistoryRepository::new(thread_store, Arc::new(ThreadTranscriptStore::memory()));

    let snapshot = repo.thread_snapshot("thread::live-only", 10).await.unwrap();
    assert_eq!(snapshot.total_committed_messages, 0);
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0]["content"], "live");
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
    assert_eq!(msgs.iter().map(|m| m["content"].as_str().unwrap()).collect::<Vec<_>>(), ["c", "d"]);

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
async fn thread_snapshot_after_index_includes_overlay_when_committed_drained() {
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
                    "message_count": 2,
                    "active_run_snapshot": {
                        "run_id": "run-live",
                        "messages": [{"role": "assistant", "content": "x"}]
                    }
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(thread_store, transcript_store);

    // after index 0 → committed tail [b] reaches end → overlay [x] included
    let snapshot = repo
        .thread_snapshot_after_index("thread::fa2", 0, 10)
        .await
        .unwrap();
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0]["content"], "b");
    assert_eq!(combined[1]["content"], "x");
}

#[tokio::test]
async fn thread_snapshot_after_index_withholds_overlay_when_more_committed() {
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
                    "message_count": 4,
                    "active_run_snapshot": {
                        "run_id": "run-live",
                        "messages": [{"role": "assistant", "content": "x"}]
                    }
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(thread_store, transcript_store);

    // after 0, limit 1 → committed tail [b] does NOT reach end → overlay withheld (no gap)
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
    let authoritative = [json!({"role":"user","content":"u","metadata":{"sdk_session_id":"sess-9"}})];
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
    assert_eq!(records.len(), 2, "must not re-append the whole run without a run_id");
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
    let delta = store.records_after_seq("thread::seq", 7, 100).await.unwrap();
    assert_eq!(delta.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![8, 9, 10]);
    assert_eq!(delta[0].message["content"], "m7");
    // caught up → empty
    assert!(store.records_after_seq("thread::seq", 10, 100).await.unwrap().is_empty());
    // after_seq=0 → all
    assert_eq!(store.records_after_seq("thread::seq", 0, 100).await.unwrap().len(), 10);
    // limit smaller than delta → NEWEST `limit`, ascending (keeps the stream's
    // live handoff gapless; older history pages in via before_index)
    let capped = store.records_after_seq("thread::seq", 0, 3).await.unwrap();
    assert_eq!(capped.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![8, 9, 10]);
    // unknown thread → empty
    assert!(store.records_after_seq("thread::nope", 0, 100).await.unwrap().is_empty());
}
