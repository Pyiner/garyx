use super::MessageRouter;

#[test]
fn sanitize_persisted_delivery_thread_id_preserves_telegram_topic_scope() {
    assert_eq!(
        MessageRouter::sanitize_persisted_delivery_thread_id(
            "telegram",
            "42",
            Some("42_t200".to_owned())
        ),
        Some("42_t200".to_owned())
    );
}

#[test]
fn sanitize_persisted_delivery_thread_id_drops_private_or_internal_telegram_scope() {
    assert_eq!(
        MessageRouter::sanitize_persisted_delivery_thread_id(
            "telegram",
            "42",
            Some("42".to_owned())
        ),
        None
    );
    assert_eq!(
        MessageRouter::sanitize_persisted_delivery_thread_id(
            "telegram",
            "42",
            Some("thread::abc".to_owned())
        ),
        None
    );
}
