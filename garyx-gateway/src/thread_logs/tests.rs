use super::*;
use std::sync::Arc;

use tempfile::tempdir;

#[test]
fn encode_thread_id_uses_hex() {
    assert_eq!(
        ThreadFileLogger::encode_thread_id("thread::abc"),
        "7468726561643a3a616263"
    );
}

#[test]
fn compact_text_keeps_tail_boundary() {
    let source = format!(
        "{}\nsecond line\nthird line",
        "x".repeat(TRIM_TARGET_BYTES + 32)
    );
    let compacted = compact_text(&source);
    assert!(compacted.starts_with("second line"));
    assert!(compacted.contains("third line"));
}

#[tokio::test]
async fn read_chunk_returns_full_then_delta() {
    let dir = tempdir().unwrap();
    let logger = ThreadFileLogger::new(dir.path());
    logger
        .record_event(ThreadLogEvent::info("thread::one", "run", "hello"))
        .await;

    let full = logger.read_chunk("thread::one", None).await.unwrap();
    assert!(full.reset);
    assert!(full.text.contains("hello"));

    logger
        .record_event(ThreadLogEvent::info("thread::one", "run", "world"))
        .await;
    let delta = logger
        .read_chunk("thread::one", Some(full.cursor))
        .await
        .unwrap();
    assert!(!delta.reset);
    assert!(delta.text.contains("world"));
}

#[tokio::test]
async fn read_chunk_resets_after_compaction() {
    let dir = tempdir().unwrap();
    let logger = ThreadFileLogger::new(dir.path());
    let path = logger.thread_log_path("thread::one");
    logger.ensure_root_dir().await.unwrap();
    fs::write(&path, "before\n").await.unwrap();
    let stale_cursor = 99;

    let chunk = logger
        .read_chunk("thread::one", Some(stale_cursor))
        .await
        .unwrap();
    assert!(chunk.reset);
    assert_eq!(chunk.text, "before\n");
}

#[tokio::test]
async fn read_chunk_for_missing_file_returns_empty_reset_chunk() {
    let dir = tempdir().unwrap();
    let logger = ThreadFileLogger::new(dir.path());

    let chunk = logger.read_chunk("thread::one", None).await.unwrap();
    assert!(chunk.reset);
    assert_eq!(chunk.text, "");
    assert_eq!(chunk.cursor, 0);
}

#[tokio::test]
async fn record_event_redacts_sensitive_fields() {
    let dir = tempdir().unwrap();
    let logger = ThreadFileLogger::new(dir.path());
    logger
        .record_event(
            ThreadLogEvent::info("thread::one", "run", "done")
                .with_field(
                    "metadata",
                    serde_json::json!({
                        "token": "secret",
                        "safe": "value",
                    }),
                )
                .with_field("desktop_claude_env", serde_json::json!({"A": "B"})),
        )
        .await;

    let chunk = logger.read_chunk("thread::one", None).await.unwrap();
    assert!(chunk.text.contains("safe"));
    assert!(!chunk.text.contains("secret"));
    assert!(!chunk.text.contains("desktop_claude_env"));
}

#[tokio::test]
async fn compact_if_needed_trims_oversized_file() {
    let dir = tempdir().unwrap();
    let logger = ThreadFileLogger::new(dir.path());
    let path = logger.thread_log_path("thread::one");
    logger.ensure_root_dir().await.unwrap();

    let oversized = format!(
        "{}\nkeep-tail-1\nkeep-tail-2\n",
        "a".repeat((MAX_LOG_BYTES as usize) + 4096)
    );
    fs::write(&path, oversized).await.unwrap();

    logger.compact_if_needed(&path).await.unwrap();

    let compacted = fs::read_to_string(&path).await.unwrap();
    let metadata = fs::metadata(&path).await.unwrap();
    assert!(metadata.len() < MAX_LOG_BYTES);
    assert!(compacted.contains("keep-tail-1"));
    assert!(compacted.contains("keep-tail-2"));
}

#[tokio::test]
async fn concurrent_record_event_keeps_complete_lines() {
    let dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(dir.path()));

    let mut tasks = Vec::new();
    for index in 0..24 {
        let logger = logger.clone();
        tasks.push(tokio::spawn(async move {
            logger
                .record_event(
                    ThreadLogEvent::info("thread::one", "run", format!("entry-{index}"))
                        .with_field("index", serde_json::json!(index)),
                )
                .await;
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    let chunk = logger.read_chunk("thread::one", None).await.unwrap();
    let lines: Vec<&str> = chunk.text.lines().collect();
    assert_eq!(lines.len(), 24);
    for index in 0..24 {
        assert!(chunk.text.contains(&format!("entry-{index}")));
    }
    assert!(lines.iter().all(|line| line.contains("[run]")));
}
