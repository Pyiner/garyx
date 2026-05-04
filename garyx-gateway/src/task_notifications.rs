use std::collections::HashMap;
use std::sync::Arc;

use garyx_channels::{OutboundMessage, SendMessageResult};
use garyx_models::thread_logs::ThreadLogEvent;
use garyx_models::{
    ChannelOutboundContent, MessageLifecycleStatus, MessageTerminalReason, TaskEventKind,
    TaskNotificationTarget, TaskStatus, ThreadTask,
};
use garyx_router::tasks::task_from_record;
use serde_json::{Value, json};
use tracing::warn;
use uuid::Uuid;

use crate::chat_shared::record_api_thread_log;
use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::server::AppState;

const TASK_NOTIFICATION_EVENT: &str = "task_ready_for_review";
const TASK_NOTIFICATION_TAG: &str = "garyx_task_notification";

pub(crate) fn spawn_listener(state: Arc<AppState>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    let mut rx = state.ops.events.subscribe();
    handle.spawn(async move {
        loop {
            match rx.recv().await {
                Ok(raw_event) => {
                    let Ok(payload) = serde_json::from_str::<Value>(&raw_event) else {
                        continue;
                    };
                    let Some(event) = parse_task_ready_for_review_event(&payload) else {
                        continue;
                    };
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(error) = dispatch_task_ready_notification(&state, event).await {
                            warn!(
                                thread_id = %error.thread_id,
                                task_ref = %error.task_ref,
                                error = %error.message,
                                "failed to dispatch task ready notification"
                            );
                        }
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[derive(Debug, Clone)]
pub(crate) struct TaskReadyForReviewEvent {
    pub(crate) thread_id: String,
    pub(crate) task_ref: String,
    pub(crate) run_id: Option<String>,
    pub(crate) final_message: Option<String>,
}

#[derive(Debug)]
pub(crate) struct TaskNotificationError {
    thread_id: String,
    task_ref: String,
    message: String,
}

impl TaskNotificationError {
    fn new(event: &TaskReadyForReviewEvent, message: impl Into<String>) -> Self {
        Self {
            thread_id: event.thread_id.clone(),
            task_ref: event.task_ref.clone(),
            message: message.into(),
        }
    }
}

fn parse_task_ready_for_review_event(payload: &Value) -> Option<TaskReadyForReviewEvent> {
    if payload.get("type").and_then(Value::as_str)? != TASK_NOTIFICATION_EVENT {
        return None;
    }
    let thread_id = trimmed_string(payload.get("thread_id")?)?;
    let task_ref = trimmed_string(payload.get("task_ref")?)?;
    let run_id = payload.get("run_id").and_then(trimmed_string);
    let final_message = payload.get("final_message").and_then(trimmed_string);
    Some(TaskReadyForReviewEvent {
        thread_id,
        task_ref,
        run_id,
        final_message,
    })
}

pub(crate) async fn dispatch_task_ready_notification(
    state: &Arc<AppState>,
    event: TaskReadyForReviewEvent,
) -> Result<(), TaskNotificationError> {
    let Some(record) = state.threads.thread_store.get(&event.thread_id).await else {
        return Err(TaskNotificationError::new(&event, "task thread not found"));
    };
    let task = task_from_record(&record)
        .map_err(|error| TaskNotificationError::new(&event, error.to_string()))?
        .ok_or_else(|| TaskNotificationError::new(&event, "thread has no task"))?;
    if task.status != TaskStatus::InReview || !latest_event_is_ready_for_review(&task) {
        return Ok(());
    }
    let Some(target) = task.notification_target.clone() else {
        return Err(TaskNotificationError::new(
            &event,
            "task has no notification target",
        ));
    };
    if matches!(target, TaskNotificationTarget::None) {
        record_api_thread_log(
            state,
            ThreadLogEvent::info(
                &event.thread_id,
                "task",
                "task ready notification skipped by target",
            )
            .with_run_id(event.run_id.clone().unwrap_or_default())
            .with_field("task_ref", json!(event.task_ref)),
        )
        .await;
        return Ok(());
    }

    let final_message = match event.final_message.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => final_message_from_task_thread(state, &event.thread_id)
            .await
            .unwrap_or_default(),
    };
    let notification = format_task_ready_notification(&event.task_ref, &task.title, &final_message);
    match target {
        TaskNotificationTarget::None => Ok(()),
        TaskNotificationTarget::Thread { thread_id } => {
            deliver_notification_to_thread(state, &event, &thread_id, &notification).await
        }
        TaskNotificationTarget::Bot {
            channel,
            account_id,
        } => deliver_notification_to_bot(state, &event, &channel, &account_id, &notification).await,
    }
}

fn latest_event_is_ready_for_review(task: &ThreadTask) -> bool {
    matches!(
        task.events.last().map(|event| &event.kind),
        Some(TaskEventKind::StatusChanged {
            from: TaskStatus::InProgress,
            to: TaskStatus::InReview,
            ..
        })
    )
}

pub(crate) fn format_task_ready_notification(
    task_ref: &str,
    title: &str,
    final_message: &str,
) -> String {
    let safe_task_ref = xml_attr(task_ref);
    let body_task_ref = neutralize_task_notification_tag(task_ref.trim());
    let title = neutralize_task_notification_tag(title.trim());
    let final_message = neutralize_task_notification_tag(final_message.trim());
    let final_message = if final_message.is_empty() {
        "The task is ready for review.".to_owned()
    } else {
        final_message
    };
    format!(
        "<{TASK_NOTIFICATION_TAG} event=\"ready_for_review\" task_ref=\"{safe_task_ref}\" status=\"in_review\">\n\
Task {body_task_ref} is ready for review: {title}\n\n\
{final_message}\n\n\
View details:\n\
garyx task get {body_task_ref}\n\
</{TASK_NOTIFICATION_TAG}>"
    )
}

async fn final_message_from_task_thread(state: &Arc<AppState>, thread_id: &str) -> Option<String> {
    let snapshot = state
        .threads
        .history
        .thread_snapshot(thread_id, 500)
        .await
        .ok()?;
    final_text_after_last_user(&snapshot.combined_messages())
}

fn final_text_after_last_user(messages: &[Value]) -> Option<String> {
    let mut after_user = false;
    let mut current_group: Vec<String> = Vec::new();
    let mut last_group: Vec<String> = Vec::new();
    let mut previous_was_assistant = false;

    for message in messages {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                after_user = true;
                current_group.clear();
                last_group.clear();
                previous_was_assistant = false;
            }
            Some("assistant") if after_user => {
                let text = message_text(message);
                if !previous_was_assistant {
                    current_group.clear();
                }
                if let Some(text) = text {
                    current_group.push(text);
                    last_group = current_group.clone();
                }
                previous_was_assistant = true;
            }
            _ if after_user => {
                previous_was_assistant = false;
            }
            _ => {}
        }
    }

    (!last_group.is_empty()).then(|| last_group.join("\n\n"))
}

fn message_text(message: &Value) -> Option<String> {
    match message.get("content") {
        Some(Value::String(value)) => trimmed_owned(value),
        Some(Value::Array(parts)) => {
            let text = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            trimmed_owned(&text)
        }
        _ => None,
    }
}

async fn deliver_notification_to_thread(
    state: &Arc<AppState>,
    event: &TaskReadyForReviewEvent,
    target_thread_id: &str,
    text: &str,
) -> Result<(), TaskNotificationError> {
    if state
        .threads
        .thread_store
        .get(target_thread_id)
        .await
        .is_none()
    {
        return Err(TaskNotificationError::new(
            event,
            format!("notification thread target not found: {target_thread_id}"),
        ));
    }
    dispatch_notification_to_thread_agent(state, event, target_thread_id, text).await
}

async fn deliver_notification_to_bot(
    state: &Arc<AppState>,
    event: &TaskReadyForReviewEvent,
    channel: &str,
    account_id: &str,
    text: &str,
) -> Result<(), TaskNotificationError> {
    let endpoint = crate::routes::resolve_main_endpoint_by_bot(state, channel, account_id)
        .await
        .ok_or_else(|| {
            TaskNotificationError::new(
                event,
                format!(
                    "notification bot target has no resolved main endpoint: {channel}:{account_id}"
                ),
            )
        })?;
    let target_thread_id = match endpoint
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(thread_id) => thread_id.to_owned(),
        None => {
            let mut metadata = HashMap::new();
            metadata.insert(
                "chat_id".to_owned(),
                Value::String(endpoint.chat_id.clone()),
            );
            metadata.insert(
                "display_label".to_owned(),
                Value::String(endpoint.display_label.clone()),
            );
            metadata.insert(
                "thread_binding_key".to_owned(),
                Value::String(endpoint.binding_key.clone()),
            );
            metadata.insert(
                "delivery_target_type".to_owned(),
                Value::String(endpoint.delivery_target_type.clone()),
            );
            metadata.insert(
                "delivery_target_id".to_owned(),
                Value::String(endpoint.delivery_target_id.clone()),
            );
            metadata.insert(
                "delivery_thread_id".to_owned(),
                endpoint
                    .delivery_thread_id
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null),
            );
            let mut router = state.threads.router.lock().await;
            router
                .resolve_or_create_inbound_thread(
                    &endpoint.channel,
                    &endpoint.account_id,
                    &endpoint.binding_key,
                    &metadata,
                )
                .await
        }
    };

    let dispatch_result =
        dispatch_notification_to_thread_agent(state, event, &target_thread_id, text).await;
    let send_result = send_notification_message(
        state,
        event,
        &target_thread_id,
        &endpoint.channel,
        &endpoint.account_id,
        &endpoint.chat_id,
        &endpoint.delivery_target_type,
        &endpoint.delivery_target_id,
        endpoint.delivery_thread_id.as_deref(),
        text,
    )
    .await;

    dispatch_result?;
    send_result
}

async fn dispatch_notification_to_thread_agent(
    state: &Arc<AppState>,
    event: &TaskReadyForReviewEvent,
    target_thread_id: &str,
    text: &str,
) -> Result<(), TaskNotificationError> {
    let mut extra_metadata = HashMap::from([
        ("task_notification".to_owned(), Value::Bool(true)),
        (
            "task_notification_event".to_owned(),
            Value::String("ready_for_review".to_owned()),
        ),
        ("task_ref".to_owned(), Value::String(event.task_ref.clone())),
        (
            "task_thread_id".to_owned(),
            Value::String(event.thread_id.clone()),
        ),
    ]);
    if let Some(source_run_id) = event.run_id.as_deref() {
        extra_metadata.insert(
            "task_notification_source_run_id".to_owned(),
            Value::String(source_run_id.to_owned()),
        );
    }
    let run_id = format!(
        "task-notify-{}-{}",
        event.task_ref.trim_start_matches('#'),
        Uuid::now_v7()
    );
    dispatch_internal_message_to_thread(
        state,
        target_thread_id,
        &run_id,
        text,
        InternalDispatchOptions {
            extra_metadata,
            ..Default::default()
        },
    )
    .await
    .map_err(|error| {
        TaskNotificationError::new(
            event,
            format!("failed to dispatch notification to thread agent: {error}"),
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn send_notification_message(
    state: &Arc<AppState>,
    event: &TaskReadyForReviewEvent,
    log_thread_id: &str,
    channel: &str,
    account_id: &str,
    chat_id: &str,
    delivery_target_type: &str,
    delivery_target_id: &str,
    delivery_thread_id: Option<&str>,
    text: &str,
) -> Result<(), TaskNotificationError> {
    let request = OutboundMessage {
        channel: channel.to_owned(),
        account_id: account_id.to_owned(),
        chat_id: chat_id.to_owned(),
        delivery_target_type: delivery_target_type.to_owned(),
        delivery_target_id: delivery_target_id.to_owned(),
        content: ChannelOutboundContent::text(text.to_owned()),
        reply_to: None,
        thread_id: delivery_thread_id.map(ToOwned::to_owned),
    };
    match state.channel_dispatcher().send_message(request).await {
        Ok(SendMessageResult { message_ids }) => {
            crate::runtime_diagnostics::record_message_ledger_event(
                state,
                MessageLifecycleStatus::ReplySent,
                crate::runtime_diagnostics::RuntimeDiagnosticContext {
                    thread_id: Some(log_thread_id.to_owned()),
                    run_id: event.run_id.clone(),
                    channel: Some(channel.to_owned()),
                    account_id: Some(account_id.to_owned()),
                    chat_id: Some(chat_id.to_owned()),
                    reply_message_id: message_ids.first().cloned(),
                    text_excerpt: Some(text.chars().take(200).collect()),
                    metadata: Some(json!({
                        "source": "task_notification",
                        "task_ref": event.task_ref,
                        "message_id_count": message_ids.len(),
                    })),
                    ..Default::default()
                },
            )
            .await;
            let mut router = state.threads.router.lock().await;
            for message_id in message_ids {
                router
                    .record_outbound_message_with_thread_log(
                        log_thread_id,
                        channel,
                        account_id,
                        chat_id,
                        delivery_thread_id,
                        &message_id,
                        None,
                    )
                    .await;
            }
            Ok(())
        }
        Err(error) => {
            crate::runtime_diagnostics::record_message_ledger_event(
                state,
                MessageLifecycleStatus::ReplyFailed,
                crate::runtime_diagnostics::RuntimeDiagnosticContext {
                    thread_id: Some(log_thread_id.to_owned()),
                    run_id: event.run_id.clone(),
                    channel: Some(channel.to_owned()),
                    account_id: Some(account_id.to_owned()),
                    chat_id: Some(chat_id.to_owned()),
                    text_excerpt: Some(text.chars().take(200).collect()),
                    terminal_reason: Some(MessageTerminalReason::ReplyDispatchFailed),
                    metadata: Some(json!({
                        "source": "task_notification",
                        "task_ref": event.task_ref,
                        "error": error.to_string(),
                    })),
                    ..Default::default()
                },
            )
            .await;
            Err(TaskNotificationError::new(
                event,
                format!("message delivery failed: {error}"),
            ))
        }
    }
}

fn trimmed_string(value: &Value) -> Option<String> {
    value.as_str().and_then(trimmed_owned)
}

fn trimmed_owned(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn neutralize_task_notification_tag(value: &str) -> String {
    value.replace(
        &format!("</{TASK_NOTIFICATION_TAG}>"),
        &format!("</ {TASK_NOTIFICATION_TAG}>"),
    )
}

#[cfg(test)]
mod tests;
