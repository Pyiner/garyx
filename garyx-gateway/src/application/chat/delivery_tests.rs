use garyx_channels::StreamingDispatchTarget;
use serde_json::json;

use super::plan::{bound_thread_delivery_targets, targets_except_streaming_target};

#[test]
fn bound_delivery_targets_skip_internal_api_bindings() {
    let thread = json!({
        "channel_bindings": [
            {
                "channel": "api",
                "account_id": "main",
                "binding_key": "loop",
                "chat_id": "loop",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "loop"
            },
            {
                "channel": "telegram",
                "account_id": "codex_bot",
                "binding_key": "chat-1",
                "chat_id": "chat-1",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "chat-1"
            }
        ]
    });

    let targets = bound_thread_delivery_targets(&thread);

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].channel, "telegram");
}

#[test]
fn bound_delivery_targets_exclude_only_direct_streaming_target() {
    let thread = json!({
        "channel_bindings": [
            {
                "channel": "telegram",
                "account_id": "bot1",
                "binding_key": "chat-a",
                "chat_id": "chat-a",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "chat-a"
            },
            {
                "channel": "telegram",
                "account_id": "bot2",
                "binding_key": "chat-b",
                "chat_id": "chat-b",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "chat-b"
            }
        ]
    });
    let targets = bound_thread_delivery_targets(&thread);
    let streaming_target = StreamingDispatchTarget {
        target_thread_id: "thread::target".to_owned(),
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        chat_id: "chat-a".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "chat-a".to_owned(),
        thread_id: None,
    };

    let filtered = targets_except_streaming_target(&targets, &streaming_target);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].account_id, "bot2");
    assert_eq!(filtered[0].chat_id, "chat-b");
}
