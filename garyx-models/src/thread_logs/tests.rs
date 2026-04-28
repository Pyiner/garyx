use super::*;

#[test]
fn resolve_thread_log_prefers_metadata_thread_id() {
    let metadata = HashMap::from([(
        "thread_id".to_owned(),
        Value::String("thread::one".to_owned()),
    )]);

    assert_eq!(
        resolve_thread_log_thread_id("thread::fallback", &metadata).as_deref(),
        Some("thread::one")
    );
}

#[test]
fn resolve_thread_log_uses_canonical_thread_id() {
    assert_eq!(
        resolve_thread_log_thread_id("thread::abc", &HashMap::new()).as_deref(),
        Some("thread::abc")
    );
    assert!(resolve_thread_log_thread_id("cron::abc", &HashMap::new()).is_none());
}
