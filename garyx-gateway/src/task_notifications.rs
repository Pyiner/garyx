use garyx_router::ThreadStoreExt;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use garyx_channels::{OutboundMessage, SendMessageResult};
use garyx_models::thread_logs::{ThreadLogEvent, is_canonical_thread_id};
use garyx_models::{
    ChannelOutboundContent, MessageLifecycleStatus, MessageTerminalReason, TaskNotificationTarget,
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
const BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT: usize = 4000;

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
                        if let Err(error) = deliver_task_review_handoff(&state, event).await {
                            warn!(
                                thread_id = %error.thread_id,
                                task_id = %error.task_id,
                                error = %error.message,
                                "failed to deliver task review handoff"
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
    pub(crate) task_id: String,
    pub(crate) run_id: Option<String>,
    pub(crate) handoff: Option<String>,
}

#[derive(Debug)]
pub(crate) struct TaskNotificationError {
    thread_id: String,
    task_id: String,
    message: String,
}

impl TaskNotificationError {
    fn new(event: &TaskReadyForReviewEvent, message: impl Into<String>) -> Self {
        Self {
            thread_id: event.thread_id.clone(),
            task_id: event.task_id.clone(),
            message: message.into(),
        }
    }
}

fn parse_task_ready_for_review_event(payload: &Value) -> Option<TaskReadyForReviewEvent> {
    if payload.get("type").and_then(Value::as_str)? != TASK_NOTIFICATION_EVENT {
        return None;
    }
    let thread_id = trimmed_string(payload.get("thread_id")?)?;
    let task_id = trimmed_string(payload.get("task_id")?)?;
    let run_id = payload.get("run_id").and_then(trimmed_string);
    let handoff = payload.get("handoff").and_then(trimmed_string);
    Some(TaskReadyForReviewEvent {
        thread_id,
        task_id,
        run_id,
        handoff,
    })
}

pub(crate) async fn deliver_task_review_handoff(
    state: &Arc<AppState>,
    event: TaskReadyForReviewEvent,
) -> Result<(), TaskNotificationError> {
    let Some(handoff) = event
        .handoff
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let Some(record) = state
        .threads
        .thread_store
        .get_logged(&event.thread_id)
        .await
    else {
        return Err(TaskNotificationError::new(&event, "task thread not found"));
    };
    let task = task_from_record(&record)
        .map_err(|error| TaskNotificationError::new(&event, error.to_string()))?
        .ok_or_else(|| TaskNotificationError::new(&event, "thread has no task"))?;
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
            .with_field("task_id", json!(event.task_id)),
        )
        .await;
        return Ok(());
    }

    let notification_handoff = match &target {
        TaskNotificationTarget::Bot { .. } => {
            Cow::Owned(cap_bot_task_notification_handoff(handoff))
        }
        TaskNotificationTarget::None | TaskNotificationTarget::Thread { .. } => {
            Cow::Borrowed(handoff)
        }
    };
    let notification =
        format_task_ready_notification(&event.task_id, &task.title, &notification_handoff);
    match target {
        TaskNotificationTarget::None => Ok(()),
        TaskNotificationTarget::Thread { thread_id } => {
            deliver_notification_to_thread(
                state,
                &event,
                &thread_id,
                task.title.trim(),
                &notification,
            )
            .await
        }
        TaskNotificationTarget::Bot {
            channel,
            account_id,
        } => {
            deliver_notification_to_bot(
                state,
                &event,
                &channel,
                &account_id,
                task.title.trim(),
                &notification,
            )
            .await
        }
    }
}

pub(crate) fn format_task_ready_notification(
    task_id: &str,
    title: &str,
    final_message: &str,
) -> String {
    let safe_task_id = xml_attr(task_id.trim());
    let safe_title = xml_attr(title.trim());
    let body_task_id = neutralize_task_notification_tag(task_id.trim());
    let final_message = neutralize_task_notification_tag(final_message.trim());
    let final_message = if final_message.is_empty() {
        "The task is ready for review.".to_owned()
    } else {
        final_message
    };
    format!(
        "<{TASK_NOTIFICATION_TAG} event=\"ready_for_review\" task_id=\"{safe_task_id}\" status=\"in_review\" title=\"{safe_title}\">\n\
{final_message}\n\
</{TASK_NOTIFICATION_TAG}>\n\n\
View details: garyx task get {body_task_id}\n\n\
Review next:\n\
If changes are needed, move the task back to in progress and send feedback to the task thread:\n\
garyx task update {body_task_id} --status in_progress --note \"needs changes: summary\"\n\n\
If approved, mark it done:\n\
garyx task update {body_task_id} --status done --note \"approved by reviewer\""
    )
}

fn cap_bot_task_notification_handoff(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT {
        return trimmed.to_owned();
    }
    let mut clipped = trimmed
        .chars()
        .take(BOT_TASK_NOTIFICATION_HANDOFF_CHAR_LIMIT)
        .collect::<String>();
    clipped.push_str("\n\n[truncated]");
    clipped
}

async fn deliver_notification_to_thread(
    state: &Arc<AppState>,
    event: &TaskReadyForReviewEvent,
    target_thread_id: &str,
    title: &str,
    text: &str,
) -> Result<(), TaskNotificationError> {
    if state
        .threads
        .thread_store
        .get_logged(target_thread_id)
        .await
        .is_none()
    {
        return Err(TaskNotificationError::new(
            event,
            format!("notification thread target not found: {target_thread_id}"),
        ));
    }
    dispatch_notification_to_thread_agent(state, event, target_thread_id, title, text).await
}

async fn deliver_notification_to_bot(
    state: &Arc<AppState>,
    event: &TaskReadyForReviewEvent,
    channel: &str,
    account_id: &str,
    title: &str,
    text: &str,
) -> Result<(), TaskNotificationError> {
    let endpoint = crate::routes::resolve_main_endpoint_by_bot(state, channel, account_id)
        .await
        .map_err(|error| {
            TaskNotificationError::new(
                event,
                format!(
                    "thread store error resolving notification bot target {channel}:{account_id}: {error}"
                ),
            )
        })?
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
                .map_err(|error| {
                    TaskNotificationError::new(
                        event,
                        format!("failed to create notification target thread: {error}"),
                    )
                })?
        }
    };

    let dispatch_result =
        dispatch_notification_to_thread_agent(state, event, &target_thread_id, title, text).await;
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
    title: &str,
    text: &str,
) -> Result<(), TaskNotificationError> {
    let extra_metadata = HashMap::from([(
        "task_notification".to_owned(),
        json!({
            "event": "ready_for_review",
            "status": "in_review",
            "task_id": event.task_id.trim(),
            "title": title.trim(),
        }),
    )]);
    let run_id = format!(
        "task-notify-{}-{}",
        event.task_id.trim_start_matches('#'),
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
    .map(|_outcome| ())
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
                        "task_id": event.task_id,
                        "message_id_count": message_ids.len(),
                    })),
                    ..Default::default()
                },
            )
            .await;
            if is_canonical_thread_id(log_thread_id) {
                for message_id in message_ids {
                    record_api_thread_log(
                        state,
                        ThreadLogEvent::info(
                            log_thread_id,
                            "delivery",
                            "outbound message delivered",
                        )
                        .with_field("channel", json!(channel))
                        .with_field("account_id", json!(account_id))
                        .with_field("chat_id", json!(chat_id))
                        .with_field("message_id", json!(message_id))
                        .with_field("thread_id", json!(log_thread_id)),
                    )
                    .await;
                }
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
                        "task_id": event.task_id,
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
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\r' => escaped.push_str("&#xD;"),
            '\n' => escaped.push_str("&#xA;"),
            '\t' => escaped.push_str("&#x9;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn neutralize_task_notification_tag(value: &str) -> String {
    value.replace(
        &format!("</{TASK_NOTIFICATION_TAG}>"),
        &format!("</ {TASK_NOTIFICATION_TAG}>"),
    )
}

#[cfg(test)]
mod tests;
