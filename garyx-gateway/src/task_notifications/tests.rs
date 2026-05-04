use super::*;

use async_trait::async_trait;
use chrono::Utc;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_channels::{ChannelDispatcher, ChannelInfo};
use garyx_models::config::{GaryxConfig, OwnerTargetConfig, TelegramAccount};
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use garyx_models::{Principal, TaskEvent, TaskEventKind};

type ProviderCall = (String, String, HashMap<String, Value>);

#[derive(Default)]
struct RecordingDispatcher {
    calls: std::sync::Mutex<Vec<OutboundMessage>>,
}

impl RecordingDispatcher {
    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls.lock().expect("dispatcher lock poisoned").clone()
    }
}

#[derive(Default)]
struct RecordingProvider {
    calls: std::sync::Mutex<Vec<ProviderCall>>,
}

impl RecordingProvider {
    fn calls(&self) -> Vec<ProviderCall> {
        self.calls.lock().expect("provider lock poisoned").clone()
    }
}

#[async_trait]
impl AgentLoopProvider for RecordingProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        self.calls.lock().expect("provider lock poisoned").push((
            options.thread_id.clone(),
            options.message.clone(),
            options.metadata.clone(),
        ));
        on_chunk(StreamEvent::Delta {
            text: "reviewing task notification".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "task-notification-provider-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "reviewing task notification".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: 0,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
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
        source: None,
        body: None,
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
        "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-42\" status=\"in_review\">"
    ));
    assert!(text.contains("Task #TASK-42 is ready for review: Ship task notifications"));
    assert!(text.contains("Done."));
    assert!(text.contains("Review next:"));
    assert!(text.contains(
        "garyx task update #TASK-42 --status in_progress --note \"needs changes: summary\""
    ));
    assert!(
        text.contains("garyx task update #TASK-42 --status done --note \"approved by reviewer\"")
    );
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
    let provider = Arc::new(RecordingProvider::default());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("recording-provider", provider.clone())
        .await;
    bridge
        .set_route("telegram", "main", "recording-provider")
        .await;
    bridge.set_default_provider_key("recording-provider").await;
    let state = crate::app_bootstrap::AppStateBuilder::new(telegram_owner_config())
        .with_bridge(bridge.clone())
        .with_channel_dispatcher(dispatcher.clone())
        .build();
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
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
            task_id: "#TASK-42".to_owned(),
            run_id: Some("run-42".to_owned()),
            final_message: Some("The implementation is complete.".to_owned()),
        },
    )
    .await
    .unwrap();

    let calls = dispatcher.calls();
    let notification_call = calls
        .iter()
        .find(|call| {
            call.content
                .as_text()
                .is_some_and(|text| text.contains("Task #TASK-42 is ready for review"))
        })
        .expect("direct bot notification should be sent");
    assert_eq!(notification_call.channel, "telegram");
    assert_eq!(notification_call.account_id, "main");
    assert_eq!(notification_call.delivery_target_id, "chat-42");
    let text = notification_call.content.as_text().unwrap();
    assert!(text.contains("Task #TASK-42 is ready for review"));
    assert!(text.contains("The implementation is complete."));

    let provider_calls = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let calls = provider.calls();
            if !calls.is_empty() {
                break calls;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("notification should trigger target thread agent");
    assert_eq!(provider_calls.len(), 1);
    assert!(provider_calls[0].0.starts_with("thread::"));
    assert!(
        provider_calls[0]
            .1
            .contains("Task #TASK-42 is ready for review")
    );
    assert!(
        provider_calls[0]
            .1
            .contains("The implementation is complete.")
    );
    assert_eq!(
        provider_calls[0].2.get("task_notification"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        provider_calls[0].2.get("task_id"),
        Some(&Value::String("#TASK-42".to_owned()))
    );

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
