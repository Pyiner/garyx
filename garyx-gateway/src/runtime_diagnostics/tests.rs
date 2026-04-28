use super::{RuntimeDiagnosticContext, record_message_ledger_event};
use garyx_models::{GaryxConfig, MessageLifecycleStatus, MessageTerminalReason};

#[tokio::test]
async fn records_event_with_derived_bot_and_run_ledger_id() {
    let state = crate::server::create_app_state(GaryxConfig::default());
    record_message_ledger_event(
        &state,
        MessageLifecycleStatus::RunInterrupted,
        RuntimeDiagnosticContext {
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            thread_id: Some("thread::alpha".to_owned()),
            run_id: Some("run-1".to_owned()),
            terminal_reason: Some(MessageTerminalReason::SelfRestart),
            ..Default::default()
        },
    )
    .await;

    let records = state
        .threads
        .message_ledger
        .records_for_thread("thread::alpha", 10)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].bot_id, "telegram:main");
    assert_eq!(records[0].ledger_id, "run:run-1");
    assert_eq!(
        records[0].terminal_reason,
        Some(MessageTerminalReason::SelfRestart)
    );
}
