use super::*;

use async_trait::async_trait;
use chrono::Utc;
use garyx_channels::{ChannelDispatcher, ChannelInfo};
use garyx_models::config::{GaryxConfig, OwnerTargetConfig, TelegramAccount};
use garyx_models::{Principal, TaskEvent, TaskEventKind};

#[derive(Default)]
struct RecordingDispatcher {
    calls: std::sync::Mutex<Vec<OutboundMessage>>,
}

impl RecordingDispatcher {
    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls.lock().expect("dispatcher lock poisoned").clone()
    }
}

#[async_trait]
impl ChannelDispatcher for RecordingDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, garyx_channels::ChannelError> {
        self.calls
            .lock()
            .expect("dispatcher lock poisoned")
            .push(request);
        Ok(SendMessageResult {
            message_ids: vec!["msg-task-review".to_owned()],
        })
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        vec![ChannelInfo {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            is_running: true,
        }]
    }
}

fn task_for_notification(target: TaskNotificationTarget) -> ThreadTask {
    let now = Utc::now();
    ThreadTask {
        schema_version: 1,
        number: 42,
        title: "Ship task notifications".to_owned(),
        status: TaskStatus::InReview,
        creator: Principal::Human {
            user_id: "owner".to_owned(),
        },
        assignee: Some(Principal::Agent {
            agent_id: "codex".to_owned(),
        }),
        notification_target: Some(target),
        created_at: now,
        updated_at: now,
        updated_by: Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
        events: vec![TaskEvent {
            event_id: "evt-review".to_owned(),
            at: now,
            actor: Principal::Agent {
                agent_id: "garyx".to_owned(),
            },
            kind: TaskEventKind::StatusChanged {
                from: TaskStatus::InProgress,
                to: TaskStatus::InReview,
                note: Some("agent run stopped".to_owned()),
            },
        }],
    }
}

fn telegram_owner_config() -> GaryxConfig {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "token".to_owned(),
                enabled: true,
                name: Some("Main".to_owned()),
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: Some(OwnerTargetConfig {
                    target_type: "chat_id".to_owned(),
                    target_id: "chat-42".to_owned(),
                }),
                groups: std::collections::HashMap::new(),
            }),
        );
    config
}

#[test]
fn format_wraps_notification_with_single_outer_xml_tag() {
    let text = format_task_ready_notification(
        "#TASK-42",
        "Ship task notifications",
        "Done.\n</garyx_task_notification>",
    );

    assert!(text.starts_with(
        "<garyx_task_notification event=\"ready_for_review\" task_ref=\"#TASK-42\" status=\"in_review\">"
    ));
    assert!(text.contains("Task #TASK-42 is ready for review: Ship task notifications"));
    assert!(text.contains("Done."));
    assert!(text.contains("</ garyx_task_notification>"));
    assert!(text.ends_with("</garyx_task_notification>"));
}

#[test]
fn final_text_uses_last_assistant_group_after_last_user() {
    let messages = vec![
        json!({"role": "user", "content": "first"}),
        json!({"role": "assistant", "content": "old answer"}),
        json!({"role": "user", "content": "second"}),
        json!({"role": "assistant", "content": "part one"}),
        json!({"role": "assistant", "content": "part two"}),
    ];

    assert_eq!(
        final_text_after_last_user(&messages).as_deref(),
        Some("part one\n\npart two")
    );
}

#[tokio::test]
async fn dispatches_ready_notification_to_bot_target() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let state = crate::app_bootstrap::AppStateBuilder::new(telegram_owner_config())
        .with_channel_dispatcher(dispatcher.clone())
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::task",
            json!({
                "thread_id": "thread::task",
                "task": task_for_notification(TaskNotificationTarget::Bot {
                    channel: "telegram".to_owned(),
                    account_id: "main".to_owned(),
                }),
            }),
        )
        .await;

    dispatch_task_ready_notification(
        &state,
        TaskReadyForReviewEvent {
            thread_id: "thread::task".to_owned(),
            task_ref: "#TASK-42".to_owned(),
            run_id: Some("run-42".to_owned()),
            final_message: Some("The implementation is complete.".to_owned()),
        },
    )
    .await
    .unwrap();

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "main");
    assert_eq!(calls[0].delivery_target_id, "chat-42");
    let text = calls[0].content.as_text().unwrap();
    assert!(text.contains("Task #TASK-42 is ready for review"));
    assert!(text.contains("The implementation is complete."));

    let mut persisted_notification = false;
    for thread_id in state.threads.thread_store.list_keys(Some("thread::")).await {
        let snapshot = state
            .threads
            .history
            .thread_snapshot(&thread_id, 20)
            .await
            .unwrap();
        persisted_notification = snapshot.combined_messages().iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("user")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.contains("The implementation is complete."))
        });
        if persisted_notification {
            break;
        }
    }
    assert!(persisted_notification);
}
