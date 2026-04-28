use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use garyx_channels::StreamingDispatchTarget;
use garyx_channels::{ChannelError, OutboundMessage, SendMessageResult};
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use garyx_models::thread_logs::ThreadLogEvent;
use garyx_models::{MessageLifecycleStatus, MessageTerminalReason};
use garyx_router::{bindings_from_value, detach_endpoint_from_thread};
use serde_json::{Value, json};

use crate::chat_shared::record_api_thread_log;
use crate::server::AppState;

#[cfg(test)]
pub(crate) const LOOP_BOUND_DELIVERY_FLUSH_DELAY: Duration = Duration::from_millis(20);
#[cfg(not(test))]
pub(crate) const LOOP_BOUND_DELIVERY_FLUSH_DELAY: Duration = Duration::from_secs(10);

fn is_message_tool_name(tool_name: &str) -> bool {
    let trimmed = tool_name.trim();
    !trimmed.is_empty()
        && trimmed
            .rsplit(':')
            .next()
            .is_some_and(|value| value.eq_ignore_ascii_case("message"))
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .filter(|text| !text.trim().is_empty())
}

fn value_marks_message_tool(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            non_empty_string(map.get("tool"))
                .or_else(|| non_empty_string(map.get("tool_name")))
                .or_else(|| non_empty_string(map.get("toolName")))
                .or_else(|| non_empty_string(map.get("name")))
                .is_some_and(|name| is_message_tool_name(&name))
                || map.values().any(value_marks_message_tool)
        }
        Value::Array(items) => items.iter().any(value_marks_message_tool),
        _ => false,
    }
}

fn extract_message_tool_text(content: &Value) -> Option<String> {
    const POINTERS: &[&str] = &[
        "/text",
        "/input/text",
        "/input/params/text",
        "/arguments/text",
        "/args/text",
        "/params/text",
        "/result/text",
        "/result/input/text",
        "/result/input/params/text",
        "/result/arguments/text",
        "/result/args/text",
        "/result/params/text",
    ];

    for pointer in POINTERS {
        if let Some(text) = non_empty_string(content.pointer(pointer)) {
            return Some(text);
        }
    }
    None
}

pub(crate) fn message_tool_mirror_text(message: &ProviderMessage) -> Option<String> {
    if message.is_error.unwrap_or(false) {
        return None;
    }
    let marked = message
        .tool_name
        .as_deref()
        .is_some_and(is_message_tool_name)
        || value_marks_message_tool(&message.content);
    if !marked {
        return None;
    }
    extract_message_tool_text(&message.content)
}

#[derive(Clone)]
struct BoundThreadDeliveryTarget {
    endpoint_key: String,
    channel: String,
    account_id: String,
    chat_id: String,
    delivery_target_type: String,
    delivery_target_id: String,
    thread_id: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct BoundThreadDeliveryBuffer {
    pending: Arc<std::sync::Mutex<String>>,
    suppressed: Arc<AtomicBool>,
    delivery_gate: Arc<tokio::sync::Mutex<()>>,
    inflight: Arc<std::sync::atomic::AtomicUsize>,
    idle_notify: Arc<tokio::sync::Notify>,
}

impl BoundThreadDeliveryBuffer {
    pub(crate) fn push_delta(&self, text: &str, warn_context: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        self.suppressed.store(false, Ordering::Relaxed);
        if let Ok(mut pending) = self.pending.lock() {
            let was_empty = pending.is_empty();
            pending.push_str(text);
            was_empty
        } else {
            tracing::warn!("{warn_context}: buffer lock poisoned while collecting assistant delta");
            false
        }
    }

    pub(crate) fn suppress(&self) {
        let should_suppress = match self.pending.lock() {
            Ok(pending) => pending.trim().is_empty(),
            Err(_) => {
                tracing::warn!(
                    "bound delivery buffer lock poisoned while deciding message-tool suppression"
                );
                true
            }
        };
        if should_suppress {
            self.suppressed.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn push_separator(&self, warn_context: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            if pending.trim().is_empty() {
                return;
            }
            if pending.ends_with("\n\n") {
                return;
            }
            if pending.ends_with('\n') {
                pending.push('\n');
            } else {
                pending.push_str("\n\n");
            }
        } else {
            tracing::warn!(
                "{warn_context}: buffer lock poisoned while collecting assistant boundary"
            );
        }
    }

    pub(crate) fn flush(
        &self,
        state: Arc<AppState>,
        thread_id: String,
        run_id: String,
        warn_context: &'static str,
    ) {
        if self.suppressed.load(Ordering::Relaxed) {
            return;
        }

        let merged = match self.pending.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => {
                tracing::warn!(
                    "{warn_context}: buffer lock poisoned while finalizing assistant delivery"
                );
                return;
            }
        };
        if merged.trim().is_empty() {
            return;
        }

        let delivery_gate = self.delivery_gate.clone();
        let inflight = self.inflight.clone();
        let idle_notify = self.idle_notify.clone();
        inflight.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let _guard = delivery_gate.lock().await;
            deliver_assistant_reply_to_bound_channels(state, thread_id, run_id, merged).await;
            if inflight.fetch_sub(1, Ordering::Relaxed) == 1 {
                idle_notify.notify_waiters();
            }
        });
    }

    pub(crate) fn finish(
        &self,
        state: Arc<AppState>,
        thread_id: String,
        run_id: String,
        warn_context: &'static str,
    ) {
        self.flush(state, thread_id, run_id, warn_context);
    }
}

fn bound_thread_delivery_targets(value: &Value) -> Vec<BoundThreadDeliveryTarget> {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();

    for binding in bindings_from_value(value) {
        let channel = binding.channel.trim();
        let account_id = binding.account_id.trim();
        if channel.is_empty() || account_id.is_empty() {
            continue;
        }

        let chat_id = binding.chat_id.trim().to_owned();
        let binding_key = binding.binding_key.trim().to_owned();
        let resolved_chat_id = if chat_id.is_empty() {
            binding_key.clone()
        } else {
            chat_id
        };
        if resolved_chat_id.is_empty() {
            continue;
        }

        let endpoint_key = binding.endpoint_key();
        if !seen.insert(endpoint_key.clone()) {
            continue;
        }

        let thread_id =
            crate::routes::binding_delivery_thread_id(&binding.binding_key, &binding.chat_id);

        targets.push(BoundThreadDeliveryTarget {
            endpoint_key,
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            chat_id: resolved_chat_id,
            delivery_target_type: binding.resolved_delivery_target_type(),
            delivery_target_id: binding.resolved_delivery_target_id(),
            thread_id,
        });
    }

    targets
}

fn should_prune_bound_delivery_target(channel: &str, error: &ChannelError) -> bool {
    if channel != "telegram" {
        return false;
    }

    let error_text = error.to_string().to_ascii_lowercase();
    error_text.contains("bot was blocked by the user")
}

async fn prune_failed_bound_delivery_target(
    state: &Arc<AppState>,
    thread_id: &str,
    run_id: &str,
    target: &BoundThreadDeliveryTarget,
    error: &ChannelError,
) {
    if !should_prune_bound_delivery_target(&target.channel, error) {
        return;
    }

    match detach_endpoint_from_thread(&state.threads.thread_store, &target.endpoint_key).await {
        Ok(_) => {
            {
                let mut router = state.threads.router.lock().await;
                router.rebuild_thread_indexes().await;
                router.rebuild_last_delivery_cache().await;
            }

            record_api_thread_log(
                state,
                ThreadLogEvent::warn(
                    thread_id,
                    "delivery",
                    "detached bound endpoint after terminal send failure",
                )
                .with_run_id(run_id.to_owned())
                .with_field("endpoint_key", json!(target.endpoint_key))
                .with_field("channel", json!(target.channel))
                .with_field("account_id", json!(target.account_id))
                .with_field("chat_id", json!(target.chat_id))
                .with_field("error", json!(error.to_string())),
            )
            .await;
        }
        Err(detach_error) => {
            record_api_thread_log(
                state,
                ThreadLogEvent::warn(
                    thread_id,
                    "delivery",
                    "failed to detach bound endpoint after terminal send failure",
                )
                .with_run_id(run_id.to_owned())
                .with_field("endpoint_key", json!(target.endpoint_key))
                .with_field("channel", json!(target.channel))
                .with_field("account_id", json!(target.account_id))
                .with_field("chat_id", json!(target.chat_id))
                .with_field("error", json!(error.to_string()))
                .with_field("detach_error", json!(detach_error)),
            )
            .await;
        }
    }
}

async fn deliver_assistant_reply_to_bound_channels(
    state: Arc<AppState>,
    thread_id: String,
    run_id: String,
    text: String,
) {
    if text.trim().is_empty() {
        return;
    }

    let Some(session_data) = state.threads.thread_store.get(&thread_id).await else {
        return;
    };
    let targets = bound_thread_delivery_targets(&session_data);
    if targets.is_empty() {
        return;
    }

    record_api_thread_log(
        &state,
        ThreadLogEvent::info(
            &thread_id,
            "delivery",
            "assistant reply forwarding to bound endpoints",
        )
        .with_run_id(run_id.clone())
        .with_field("target_count", json!(targets.len())),
    )
    .await;

    let dispatcher = state.channel_dispatcher();
    for target in targets {
        let request = OutboundMessage {
            channel: target.channel.clone(),
            account_id: target.account_id.clone(),
            chat_id: target.chat_id.clone(),
            delivery_target_type: target.delivery_target_type.clone(),
            delivery_target_id: target.delivery_target_id.clone(),
            text: text.clone(),
            reply_to: None,
            thread_id: target.thread_id.clone(),
        };

        match dispatcher.send_message(request).await {
            Ok(SendMessageResult { message_ids }) => {
                let first_message_id = message_ids.first().cloned();
                crate::runtime_diagnostics::record_message_ledger_event(
                    &state,
                    MessageLifecycleStatus::ReplySent,
                    crate::runtime_diagnostics::RuntimeDiagnosticContext {
                        thread_id: Some(thread_id.clone()),
                        run_id: Some(run_id.clone()),
                        channel: Some(target.channel.clone()),
                        account_id: Some(target.account_id.clone()),
                        chat_id: Some(target.chat_id.clone()),
                        reply_message_id: first_message_id,
                        text_excerpt: Some(text.chars().take(200).collect()),
                        metadata: Some(json!({
                            "source": "bound_delivery",
                            "message_id_count": message_ids.len(),
                        })),
                        ..Default::default()
                    },
                )
                .await;
                if message_ids.is_empty() {
                    continue;
                }
                let mut router = state.threads.router.lock().await;
                for message_id in message_ids {
                    router
                        .record_outbound_message_with_thread_log(
                            &thread_id,
                            &target.channel,
                            &target.account_id,
                            &target.chat_id,
                            target.thread_id.as_deref(),
                            &message_id,
                            None,
                        )
                        .await;
                }
            }
            Err(error) => {
                crate::runtime_diagnostics::record_message_ledger_event(
                    &state,
                    MessageLifecycleStatus::ReplyFailed,
                    crate::runtime_diagnostics::RuntimeDiagnosticContext {
                        thread_id: Some(thread_id.clone()),
                        run_id: Some(run_id.clone()),
                        channel: Some(target.channel.clone()),
                        account_id: Some(target.account_id.clone()),
                        chat_id: Some(target.chat_id.clone()),
                        text_excerpt: Some(text.chars().take(200).collect()),
                        terminal_reason: Some(MessageTerminalReason::ReplyDispatchFailed),
                        metadata: Some(json!({
                            "source": "bound_delivery",
                            "error": error.to_string(),
                        })),
                        ..Default::default()
                    },
                )
                .await;
                record_api_thread_log(
                    &state,
                    ThreadLogEvent::warn(
                        &thread_id,
                        "delivery",
                        "assistant reply forwarding failed",
                    )
                    .with_run_id(run_id.clone())
                    .with_field("endpoint_key", json!(target.endpoint_key))
                    .with_field("channel", json!(target.channel))
                    .with_field("account_id", json!(target.account_id))
                    .with_field("chat_id", json!(target.chat_id))
                    .with_field("error", json!(error.to_string())),
                )
                .await;

                prune_failed_bound_delivery_target(&state, &thread_id, &run_id, &target, &error)
                    .await;
            }
        }
    }
}

pub(crate) fn schedule_loop_bound_delivery_flush(
    buffer: BoundThreadDeliveryBuffer,
    scheduled: Arc<AtomicBool>,
    state: Arc<AppState>,
    thread_id: String,
    run_id: String,
) {
    if scheduled.swap(true, Ordering::Relaxed) {
        return;
    }

    tokio::spawn(async move {
        tokio::time::sleep(LOOP_BOUND_DELIVERY_FLUSH_DELAY).await;
        scheduled.store(false, Ordering::Relaxed);
        buffer.flush(state, thread_id, run_id, "loop bound delivery");
    });
}

pub async fn build_bound_response_callback(
    state: &Arc<AppState>,
    thread_id: &str,
    run_id: &str,
    streaming_target: Option<StreamingDispatchTarget>,
) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
    if let Some(target) = streaming_target {
        if let Some(callback) = state
            .channel_dispatcher()
            .build_streaming_callback(target, state.threads.router.clone())
        {
            return Some(callback);
        }
    }

    let bound_delivery = BoundThreadDeliveryBuffer::default();
    let callback_state = state.clone();
    let callback_thread_id = thread_id.to_owned();
    let callback_run_id = run_id.to_owned();
    let callback_delivery = bound_delivery.clone();
    let delayed_flush_scheduled = Arc::new(AtomicBool::new(false));
    let callback_flush_scheduled = delayed_flush_scheduled.clone();

    Some(Arc::new(move |event| match event {
        StreamEvent::Delta { text } => {
            if callback_delivery.push_delta(&text, "bound delivery") {
                schedule_loop_bound_delivery_flush(
                    callback_delivery.clone(),
                    callback_flush_scheduled.clone(),
                    callback_state.clone(),
                    callback_thread_id.clone(),
                    callback_run_id.clone(),
                );
            }
        }
        StreamEvent::ToolResult { message } => {
            if message_tool_mirror_text(&message).is_some() {
                callback_delivery.suppress();
            }
        }
        StreamEvent::Boundary { kind, .. } => match kind {
            StreamBoundaryKind::AssistantSegment => {
                callback_delivery.push_separator("bound delivery");
            }
            StreamBoundaryKind::UserAck => {
                callback_delivery.finish(
                    callback_state.clone(),
                    callback_thread_id.clone(),
                    callback_run_id.clone(),
                    "bound delivery",
                );
            }
        },
        StreamEvent::Done => {
            callback_delivery.finish(
                callback_state.clone(),
                callback_thread_id.clone(),
                callback_run_id.clone(),
                "bound delivery",
            );
        }
        StreamEvent::ToolUse { .. } => {
            // Flush any accumulated assistant text before a tool call so that
            // channels without native streaming (e.g. WeChat) deliver messages
            // incrementally between tool invocations instead of batching
            // everything until the run completes.
            callback_delivery.flush(
                callback_state.clone(),
                callback_thread_id.clone(),
                callback_run_id.clone(),
                "bound delivery",
            );
        }
    }))
}
