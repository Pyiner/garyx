use super::default_restart_continuation_message;

#[test]
fn default_restart_continuation_message_marks_system_notice_and_language_rule() {
    let message = default_restart_continuation_message();
    assert!(message.starts_with("<system-restart>"));
    assert!(message.ends_with("</system-restart>"));
    assert!(message.contains("system continuation notice"));
    assert!(message.contains("user's language"));
}
