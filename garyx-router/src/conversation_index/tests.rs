use super::*;

use crate::InMemoryThreadStore;
use serde_json::json;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_conversation_index_background_indexing_and_metadata() {
    let thread_store = Arc::new(InMemoryThreadStore::new());
    thread_store
        .set(
            "thread::vector-demo",
            json!({
                "workspace_dir": "/tmp/workspace-a"
            }),
        )
        .await;

    let temp = tempdir().unwrap();
    let transcript_store = Arc::new(
        ThreadTranscriptStore::file(temp.path().join("transcripts"))
            .await
            .unwrap(),
    );
    transcript_store
        .append_committed_messages(
            "thread::vector-demo",
            None,
            &[
                json!({
                    "role": "user",
                    "content": "Please remember the once schedule protocol.",
                    "timestamp": "2026-03-20T10:00:00Z"
                }),
                json!({
                    "role": "assistant",
                    "content": "Use ONCE:1992-10-03 11:11.",
                    "timestamp": "2026-03-20T10:00:05Z"
                }),
            ],
        )
        .await
        .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {
                    "embedding": [1.0, 0.0, 0.0]
                }
            ]
        })))
        .mount(&server)
        .await;

    let manager = ConversationIndexManager::new(
        thread_store,
        transcript_store.clone(),
        temp.path().join("conversation-index").join("index.sqlite3"),
        ConversationIndexConfig {
            enabled: true,
            api_key: "test-key".to_owned(),
            model: "text-embedding-3-small".to_owned(),
            base_url: format!("{}/v1", server.uri()),
        },
    )
    .await
    .unwrap();

    manager.enqueue_thread("thread::vector-demo");

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(result) = manager
                .search(ConversationIndexSearchRequest {
                    query: "once schedule protocol".to_owned(),
                    thread_id: None,
                    workspace_dir: None,
                    from: None,
                    to: None,
                    limit: 5,
                })
                .await
                .unwrap()
                && !result.results.is_empty()
            {
                return result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].thread_id, "thread::vector-demo");
    assert_eq!(
        result.results[0].workspace_dir.as_deref(),
        Some("/tmp/workspace-a")
    );
    assert!(
        result.results[0]
            .transcript_file
            .as_deref()
            .unwrap_or_default()
            .ends_with(".jsonl")
    );
    assert!(
        result.results[0]
            .snippet
            .contains("assistant: Use ONCE:1992-10-03 11:11.")
    );

    let requests = server.received_requests().await.unwrap();
    assert!(requests.len() >= 2);
    let index_request_body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let input = index_request_body["input"][0].as_str().unwrap_or_default();
    assert!(!input.contains("/tmp/workspace-a"));
    assert!(!input.contains("thread::vector-demo"));
}

#[tokio::test]
async fn test_conversation_index_empty_search_skips_embedding_call() {
    let thread_store = Arc::new(InMemoryThreadStore::new());
    let temp = tempdir().unwrap();
    let transcript_store = Arc::new(
        ThreadTranscriptStore::file(temp.path().join("transcripts"))
            .await
            .unwrap(),
    );

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {
                    "embedding": [1.0, 0.0, 0.0]
                }
            ]
        })))
        .mount(&server)
        .await;

    let manager = ConversationIndexManager::new(
        thread_store,
        transcript_store,
        temp.path().join("conversation-index").join("index.sqlite3"),
        ConversationIndexConfig {
            enabled: true,
            api_key: "test-key".to_owned(),
            model: "text-embedding-3-small".to_owned(),
            base_url: format!("{}/v1", server.uri()),
        },
    )
    .await
    .unwrap();

    let result = manager
        .search(ConversationIndexSearchRequest {
            query: "anything".to_owned(),
            thread_id: None,
            workspace_dir: None,
            from: None,
            to: None,
            limit: 5,
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(result.candidate_chunks, 0);
    assert!(result.results.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert!(requests.is_empty());
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_conversation_index_real_openai_roundtrip() {
    let api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .expect("OPENAI_API_KEY must be set for this test");

    let thread_store = Arc::new(InMemoryThreadStore::new());
    thread_store
        .set(
            "thread::real-vector",
            json!({ "workspace_dir": "/tmp/workspace-real" }),
        )
        .await;
    thread_store
        .set(
            "thread::real-noise",
            json!({ "workspace_dir": "/tmp/workspace-real" }),
        )
        .await;

    let temp = tempdir().unwrap();
    let transcript_store = Arc::new(
        ThreadTranscriptStore::file(temp.path().join("transcripts"))
            .await
            .unwrap(),
    );
    transcript_store
        .append_committed_messages(
            "thread::real-vector",
            None,
            &[
                json!({
                    "role": "user",
                    "content": "Can we support once schedule protocol in Garyx?",
                    "timestamp": "2026-03-20T10:00:00Z"
                }),
                json!({
                    "role": "assistant",
                    "content": "Yes. The format is ONCE:1992-10-03 11:11.",
                    "timestamp": "2026-03-20T10:00:05Z"
                }),
            ],
        )
        .await
        .unwrap();
    transcript_store
        .append_committed_messages(
            "thread::real-noise",
            None,
            &[json!({
                "role": "assistant",
                "content": "We also changed the dashboard card spacing.",
                "timestamp": "2026-03-20T10:05:00Z"
            })],
        )
        .await
        .unwrap();

    let manager = ConversationIndexManager::new(
        thread_store,
        transcript_store,
        temp.path().join("conversation-index").join("index.sqlite3"),
        ConversationIndexConfig {
            enabled: true,
            api_key,
            model: "text-embedding-3-small".to_owned(),
            base_url: "https://api.openai.com/v1".to_owned(),
        },
    )
    .await
    .unwrap();

    manager.enqueue_thread("thread::real-vector");
    manager.enqueue_thread("thread::real-noise");

    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let result = manager
                .search(ConversationIndexSearchRequest {
                    query: "once schedule protocol".to_owned(),
                    thread_id: None,
                    workspace_dir: Some("/tmp/workspace-real".to_owned()),
                    from: None,
                    to: None,
                    limit: 3,
                })
                .await
                .unwrap()
                .unwrap();
            if !result.results.is_empty() {
                return result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();

    assert_eq!(result.results[0].thread_id, "thread::real-vector");
    assert!(result.results[0].snippet.contains("ONCE:1992-10-03 11:11."));
}
