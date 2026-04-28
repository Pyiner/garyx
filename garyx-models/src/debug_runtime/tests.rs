use super::{
    MessageLedgerEvent, MessageLedgerRecord, MessageLifecycleStatus, MessageTerminalReason,
};
use serde_json::json;

#[test]
fn applies_events_into_a_folded_record() {
    let mut record = MessageLedgerRecord::from_event(&MessageLedgerEvent {
        ledger_id: "msg-1".to_owned(),
        bot_id: "telegram:main".to_owned(),
        status: MessageLifecycleStatus::Received,
        created_at: "2026-03-22T10:00:00Z".to_owned(),
        thread_id: None,
        run_id: None,
        channel: Some("telegram".to_owned()),
        account_id: Some("main".to_owned()),
        chat_id: Some("-100".to_owned()),
        from_id: Some("42".to_owned()),
        native_message_id: Some("tg-1".to_owned()),
        text_excerpt: Some("hello".to_owned()),
        terminal_reason: None,
        reply_message_id: None,
        metadata: json!({"source":"ingress"}),
    });

    record.apply_event(&MessageLedgerEvent {
        ledger_id: "msg-1".to_owned(),
        bot_id: "telegram:main".to_owned(),
        status: MessageLifecycleStatus::RunInterrupted,
        created_at: "2026-03-22T10:00:02Z".to_owned(),
        thread_id: Some("thread::123".to_owned()),
        run_id: Some("run-1".to_owned()),
        channel: None,
        account_id: None,
        chat_id: None,
        from_id: None,
        native_message_id: None,
        text_excerpt: None,
        terminal_reason: Some(MessageTerminalReason::SelfRestart),
        reply_message_id: None,
        metadata: json!({"reason":"restart"}),
    });

    assert_eq!(record.thread_id.as_deref(), Some("thread::123"));
    assert_eq!(record.run_id.as_deref(), Some("run-1"));
    assert_eq!(record.status, MessageLifecycleStatus::RunInterrupted);
    assert_eq!(
        record.terminal_reason,
        Some(MessageTerminalReason::SelfRestart)
    );
    assert_eq!(record.metadata["source"], "ingress");
    assert_eq!(record.metadata["reason"], "restart");
    assert!(record.is_problem());
}
