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
    thread_store
        .set(
            "thread::overlay",
            json!({
                "messages": [{"role": "user", "content": "past"}],
                "history": {
                    "active_run_snapshot": {
                        "run_id": "run-live",
                        "messages": [{"role": "assistant", "content": "live"}]
                    }
                }
            }),
        )
        .await;
    let repo = ThreadHistoryRepository::new(
        thread_store,
        Arc::new(ThreadTranscriptStore::memory()),
        ThreadHistoryBackend::InlineMessages,
    );

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
    let repo = ThreadHistoryRepository::new(
        thread_store,
        Arc::new(ThreadTranscriptStore::memory()),
        ThreadHistoryBackend::TranscriptV1,
    );

    let snapshot = repo.thread_snapshot("thread::live-only", 10).await.unwrap();
    assert_eq!(snapshot.total_committed_messages, 0);
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0]["content"], "live");
}

#[tokio::test]
async fn transcript_backend_rejects_legacy_inline_history_without_transcript() {
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
    let repo = ThreadHistoryRepository::new(
        thread_store,
        Arc::new(ThreadTranscriptStore::memory()),
        ThreadHistoryBackend::TranscriptV1,
    );

    let error = repo
        .thread_snapshot("thread::legacy-inline", 10)
        .await
        .expect_err("missing transcript should fail for legacy inline history");
    assert!(matches!(
        error,
        ThreadHistoryError::MissingTranscript(thread_id) if thread_id == "thread::legacy-inline"
    ));
}
