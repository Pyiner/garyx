use super::MessageLedgerStore;
use garyx_models::{MessageLedgerEvent, MessageLifecycleStatus, MessageTerminalReason};
use serde_json::json;
use tempfile::tempdir;

fn sample_event(
    ledger_id: &str,
    created_at: &str,
    status: MessageLifecycleStatus,
) -> MessageLedgerEvent {
    MessageLedgerEvent {
        ledger_id: ledger_id.to_owned(),
        bot_id: "telegram:main".to_owned(),
        status,
        created_at: created_at.to_owned(),
        thread_id: Some("thread::alpha".to_owned()),
        run_id: None,
        channel: Some("telegram".to_owned()),
        account_id: Some("main".to_owned()),
        chat_id: Some("-100".to_owned()),
        from_id: Some("42".to_owned()),
        native_message_id: Some(format!("native-{ledger_id}")),
        text_excerpt: Some(format!("message {ledger_id}")),
        terminal_reason: None,
        reply_message_id: None,
        metadata: json!({}),
    }
}

#[tokio::test]
async fn memory_store_folds_records_by_thread_and_bot() {
    let store = MessageLedgerStore::memory();
    store
        .append_event(sample_event(
            "ledger-1",
            "2026-03-22T10:00:00Z",
            MessageLifecycleStatus::Received,
        ))
        .await
        .unwrap();
    store
        .append_event(MessageLedgerEvent {
            terminal_reason: Some(MessageTerminalReason::SelfRestart),
            status: MessageLifecycleStatus::RunInterrupted,
            run_id: Some("run-1".to_owned()),
            created_at: "2026-03-22T10:00:01Z".to_owned(),
            ..sample_event(
                "ledger-1",
                "2026-03-22T10:00:00Z",
                MessageLifecycleStatus::Received,
            )
        })
        .await
        .unwrap();
    store
        .append_event(sample_event(
            "ledger-2",
            "2026-03-22T10:00:02Z",
            MessageLifecycleStatus::ReplySent,
        ))
        .await
        .unwrap();

    let thread_records = store.records_for_thread("thread::alpha", 10).await.unwrap();
    assert_eq!(thread_records.len(), 2);
    assert_eq!(thread_records[0].ledger_id, "ledger-1");
    assert_eq!(
        thread_records[0].terminal_reason,
        Some(MessageTerminalReason::SelfRestart)
    );

    let bot_records = store.records_for_bot("telegram:main", 10).await.unwrap();
    assert_eq!(bot_records.len(), 2);

    let problems = store
        .problem_threads_for_bot("telegram:main", 10)
        .await
        .unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].thread_id, "thread::alpha");
    assert_eq!(problems[0].message_count, 1);
}

#[tokio::test]
async fn file_store_persists_and_reads_events() {
    let temp = tempdir().unwrap();
    let store = MessageLedgerStore::file(temp.path()).await.unwrap();
    store
        .append_event(sample_event(
            "ledger-1",
            "2026-03-22T10:00:00Z",
            MessageLifecycleStatus::Received,
        ))
        .await
        .unwrap();

    let events = store
        .list_events_for_bot("telegram:main", 10)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert!(store.events_path().unwrap().exists());
}
