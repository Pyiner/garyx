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
