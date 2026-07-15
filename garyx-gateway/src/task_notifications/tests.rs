use super::*;

use async_trait::async_trait;
use chrono::Utc;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{BridgeError, ProviderRuntime, StreamCallback};
use garyx_channels::{ChannelDispatcher, ChannelInfo};
use garyx_models::config::{GaryxConfig, OwnerTargetConfig, TelegramAccount};
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};
use garyx_models::thread_logs::{ThreadLogChunk, ThreadLogEvent, ThreadLogSink};
use garyx_models::{Principal, TaskEvent, TaskEventKind, TaskStatus, ThreadTask};

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

#[derive(Default)]
struct RecordingThreadLogSink {
    events: std::sync::Mutex<Vec<ThreadLogEvent>>,
}

impl RecordingThreadLogSink {
    fn events(&self) -> Vec<ThreadLogEvent> {
        self.events
            .lock()
            .expect("thread log lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ThreadLogSink for RecordingThreadLogSink {
    async fn record_event(&self, event: ThreadLogEvent) {
        self.events
            .lock()
            .expect("thread log lock poisoned")
            .push(event);
    }

    async fn read_chunk(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
    ) -> Result<ThreadLogChunk, String> {
        Ok(ThreadLogChunk {
            thread_id: thread_id.to_owned(),
            path: String::new(),
            text: String::new(),
            cursor: cursor.unwrap_or_default(),
            reset: cursor.is_none(),
        })
    }

    async fn delete_thread(&self, _thread_id: &str) -> Result<(), String> {
        Ok(())
    }
}

impl RecordingProvider {
    fn calls(&self) -> Vec<ProviderCall> {
        self.calls.lock().expect("provider lock poisoned").clone()
    }
}

#[async_trait]
impl ProviderRuntime for RecordingProvider {
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
        executor: None,
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
fn cap_bot_task_notification_handoff_limits_long_bodies() {
    let long_body = "a".repeat(BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT + 100);
    let capped = cap_bot_task_notification_handoff(&long_body);

    assert!(capped.ends_with("\n\n[truncated]"));
    assert!(
        capped.chars().count()
            <= BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT + "\n\n[truncated]".chars().count()
    );
    assert!(!capped.contains(&"a".repeat(BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT + 1)));
}

#[tokio::test]
async fn thread_target_receives_complete_handoff_beyond_bot_cap() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let provider = Arc::new(RecordingProvider::default());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("recording-provider", provider.clone())
        .await;
    bridge.set_default_provider_key("recording-provider").await;
    let state = crate::app_bootstrap::AppStateBuilder::new(telegram_owner_config())
        .with_bridge(bridge.clone())
        .with_channel_dispatcher(dispatcher)
        .build();
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
    state
        .threads
        .thread_store
        .set(
            "thread::task-long-handoff",
            json!({
                "thread_id": "thread::task-long-handoff",
                "task": task_for_notification(TaskNotificationTarget::Thread {
                    thread_id: "thread::review-parent".to_owned(),
                }),
            }),
        )
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set(
            "thread::review-parent",
            json!({
                "thread_id": "thread::review-parent",
                "provider_key": "recording-provider",
            }),
        )
        .await
        .unwrap();

    let handoff = format!(
        "{}\ncomplete-handoff-tail",
        "a".repeat(BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT)
    );
    deliver_task_review_handoff(
        &state,
        TaskReadyForReviewEvent {
            thread_id: "thread::task-long-handoff".to_owned(),
            task_id: "#TASK-42".to_owned(),
            run_id: Some("run-long-handoff".to_owned()),
            handoff: Some(handoff.clone()),
        },
    )
    .await
    .unwrap();

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
    assert!(provider_calls[0].1.contains(&handoff));
    assert!(!provider_calls[0].1.contains("[truncated]"));
}

#[tokio::test]
async fn deliver_without_handoff_does_not_fallback_to_committed_thread_final_message() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let provider = Arc::new(RecordingProvider::default());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("recording-provider", provider.clone())
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
            "thread::task-final",
            json!({
                "thread_id": "thread::task-final",
                "task": task_for_notification(TaskNotificationTarget::Bot {
                    channel: "telegram".to_owned(),
                    account_id: "main".to_owned(),
                }),
                "history": {
                    "message_count": 2
                }
            }),
        )
        .await
        .unwrap();
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::task-final",
            Some("run-final-task"),
            &[
                json!({"role": "user", "content": "Please finish the implementation."}),
                json!({"role": "assistant", "content": "Committed final handoff text."}),
            ],
        )
        .await
        .expect("append transcript");

    deliver_task_review_handoff(
        &state,
        TaskReadyForReviewEvent {
            thread_id: "thread::task-final".to_owned(),
            task_id: "#TASK-42".to_owned(),
            run_id: None,
            handoff: None,
        },
    )
    .await
    .unwrap();

    assert!(dispatcher.calls().is_empty());
    assert!(provider.calls().is_empty());
}

#[tokio::test]
async fn dispatch_does_not_replay_ready_notification_without_handoff() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let provider = Arc::new(RecordingProvider::default());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("recording-provider", provider)
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
            "thread::task-replay",
            json!({
                "thread_id": "thread::task-replay",
                "task": task_for_notification(TaskNotificationTarget::Bot {
                    channel: "telegram".to_owned(),
                    account_id: "main".to_owned(),
                }),
            }),
        )
        .await
        .unwrap();

    deliver_task_review_handoff(
        &state,
        TaskReadyForReviewEvent {
            thread_id: "thread::task-replay".to_owned(),
            task_id: "#TASK-42".to_owned(),
            run_id: Some("run-handoff".to_owned()),
            handoff: Some("First handoff.".to_owned()),
        },
    )
    .await
    .unwrap();

    deliver_task_review_handoff(
        &state,
        TaskReadyForReviewEvent {
            thread_id: "thread::task-replay".to_owned(),
            task_id: "#TASK-42".to_owned(),
            run_id: None,
            handoff: None,
        },
    )
    .await
    .unwrap();

    let ready_notifications = dispatcher
        .calls()
        .into_iter()
        .filter(|call| {
            call.content.as_text().is_some_and(|text| {
                text.contains("<garyx_task_notification event=\"ready_for_review\"")
            })
        })
        .count();
    assert_eq!(
        ready_notifications, 1,
        "a state replay without handoff must not send another ready notification"
    );
}

#[tokio::test]
async fn dispatches_ready_notification_to_bot_target() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let thread_logs = Arc::new(RecordingThreadLogSink::default());
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
        .with_thread_log_sink(thread_logs.clone())
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
        .await
        .unwrap();

    let handoff = format!(
        "The implementation is complete.\n{}\nbot-handoff-tail",
        "a".repeat(BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT)
    );
    deliver_task_review_handoff(
        &state,
        TaskReadyForReviewEvent {
            thread_id: "thread::task".to_owned(),
            task_id: "#TASK-42".to_owned(),
            run_id: Some("run-42".to_owned()),
            handoff: Some(handoff),
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
    assert!(text.contains("[truncated]"));
    assert!(!text.contains("bot-handoff-tail"));

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
    assert!(provider_calls[0].1.contains("[truncated]"));
    assert!(!provider_calls[0].1.contains("bot-handoff-tail"));
    assert_eq!(
        provider_calls[0].2.get("task_notification"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        provider_calls[0].2.get("task_id"),
        Some(&Value::String("#TASK-42".to_owned()))
    );

    let mut persisted_notification = false;
    for thread_id in state
        .threads
        .thread_store
        .list_keys(Some("thread::"))
        .await
        .unwrap()
    {
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

    let delivery_events = thread_logs
        .events()
        .into_iter()
        .filter(|event| event.stage == "delivery" && event.message == "outbound message delivered")
        .collect::<Vec<_>>();
    assert_eq!(delivery_events.len(), 1);
    let event = &delivery_events[0];
    assert!(event.thread_id.starts_with("thread::"));
    assert_eq!(event.fields.get("channel"), Some(&json!("telegram")));
    assert_eq!(event.fields.get("account_id"), Some(&json!("main")));
    assert_eq!(event.fields.get("chat_id"), Some(&json!("chat-42")));
    assert_eq!(
        event.fields.get("message_id"),
        Some(&json!("msg-task-review"))
    );
    assert_eq!(event.fields.get("thread_id"), Some(&json!(event.thread_id)));
}

#[test]
fn parse_event_treats_null_handoff_as_absent() {
    let payload = json!({
        "type": "task_ready_for_review",
        "thread_id": "thread::x",
        "task_id": "#TASK-1",
        "run_id": "run-1",
        "handoff": Value::Null,
    });
    let event = parse_task_ready_for_review_event(&payload).expect("event parses");
    assert_eq!(event.handoff, None);
}

#[test]
fn parse_event_treats_missing_handoff_as_absent() {
    let payload = json!({
        "type": "task_ready_for_review",
        "thread_id": "thread::x",
        "task_id": "#TASK-1",
    });
    let event = parse_task_ready_for_review_event(&payload).expect("event parses");
    assert_eq!(event.handoff, None);
}

#[test]
fn parse_event_keeps_non_empty_handoff() {
    let payload = json!({
        "type": "task_ready_for_review",
        "thread_id": "thread::x",
        "task_id": "#TASK-1",
        "handoff": "Done.",
    });
    let event = parse_task_ready_for_review_event(&payload).expect("event parses");
    assert_eq!(event.handoff.as_deref(), Some("Done."));
}
