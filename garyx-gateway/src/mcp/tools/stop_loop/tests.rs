use super::*;
use serde_json::json;

#[tokio::test]
async fn stop_loop_clears_internal_pending_continuations() {
    let thread_id = "thread::stop-loop";
    let mut stored = json!({
        "thread_id": thread_id,
        "channel": "telegram",
        "account_id": "bot1",
        "from_id": "user42",
        "loop_enabled": true,
        "loop_iteration_count": 7,
        "pending_user_inputs": [
            {
                "id": "queued_input:1",
                "bridge_run_id": "run-1",
                "text": LOOP_CONTINUATION_MESSAGE,
                "content": LOOP_CONTINUATION_MESSAGE,
                "queued_at": "2026-01-01T00:00:00Z",
                "status": "queued"
            },
            {
                "id": "queued_input:2",
                "bridge_run_id": "run-1",
                "text": "user follow-up",
                "content": "user follow-up",
                "queued_at": "2026-01-01T00:00:01Z",
                "status": "queued"
            }
        ]
    });

    apply_stop_loop_to_thread_value(&mut stored);

    assert_eq!(stored["loop_enabled"], Value::Bool(false));
    assert_eq!(stored["loop_iteration_count"], json!(0));
    let pending = stored["pending_user_inputs"]
        .as_array()
        .expect("pending inputs should remain array");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["text"], "user follow-up");
}
