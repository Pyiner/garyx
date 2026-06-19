use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

mod buffer;
mod images;
mod plan;
mod sender;

use self::buffer::{BoundThreadDeliveryBuffer, schedule_loop_bound_delivery_flush};
use self::plan::{
    BoundThreadDeliveryTarget, snapshot_bound_thread_delivery_targets,
    targets_except_streaming_target,
};
use garyx_channels::StreamingDispatchTarget;
use garyx_models::ChannelOutboundContent;
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use serde_json::Value;

use crate::server::AppState;

#[cfg(test)]
const STREAMING_MARKDOWN_IMAGE_FORWARD_DELAY: Duration = Duration::from_millis(1);
#[cfg(not(test))]
const STREAMING_MARKDOWN_IMAGE_FORWARD_DELAY: Duration = Duration::from_millis(500);

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

pub async fn build_bound_response_callback(
    state: &Arc<AppState>,
    thread_id: &str,
    run_id: &str,
    streaming_target: Option<StreamingDispatchTarget>,
) -> Result<
    Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    garyx_channels::committed_replay::CommittedReplayError,
> {
    let targets = snapshot_bound_thread_delivery_targets(state, thread_id).await;
    if let Some(target) = streaming_target.as_ref()
        && let Some(callback) = state
            .channel_dispatcher()
            .build_streaming_callback(target.clone(), state.threads.router.clone())
    {
        let image_scan = BoundThreadDeliveryBuffer::with_targets(targets.clone());
        let image_scan_state = state.clone();
        let image_scan_thread_id = thread_id.to_owned();
        let image_scan_run_id = run_id.to_owned();
        let bound_consumer = build_bound_delivery_consumer(
            state.clone(),
            thread_id.to_owned(),
            run_id.to_owned(),
            targets_except_streaming_target(&targets, target),
        );

        let streaming_consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
            match &event {
                StreamEvent::Delta { text } => {
                    image_scan.push_image_scan_delta(text, "streaming markdown image delivery");
                }
                StreamEvent::Boundary { kind, .. } => match kind {
                    StreamBoundaryKind::AssistantSegment => {
                        image_scan.push_image_scan_separator("streaming markdown image delivery");
                    }
                    StreamBoundaryKind::UserAck => {
                        callback(event.clone());
                        image_scan.finish_markdown_images_after(
                            image_scan_state.clone(),
                            image_scan_thread_id.clone(),
                            image_scan_run_id.clone(),
                            "streaming markdown image delivery",
                            STREAMING_MARKDOWN_IMAGE_FORWARD_DELAY,
                        );
                        return;
                    }
                },
                StreamEvent::Done => {
                    callback(event.clone());
                    image_scan.finish_markdown_images_after(
                        image_scan_state.clone(),
                        image_scan_thread_id.clone(),
                        image_scan_run_id.clone(),
                        "streaming markdown image delivery",
                        STREAMING_MARKDOWN_IMAGE_FORWARD_DELAY,
                    );
                    return;
                }
                StreamEvent::SessionBound { .. }
                | StreamEvent::ToolUse { .. }
                | StreamEvent::ToolResult { .. }
                | StreamEvent::ThreadTitleUpdated { .. } => {}
            }
            callback(event);
        });
        let consumer = if let Some(bound_consumer) = bound_consumer {
            Arc::new(move |event: StreamEvent| {
                streaming_consumer(event.clone());
                bound_consumer(event);
            }) as Arc<dyn Fn(StreamEvent) + Send + Sync>
        } else {
            streaming_consumer
        };
        // Read this run's stream from the durable committed transcript. The
        // streaming sender is unchanged; only the source changes.
        return garyx_channels::committed_replay::committed_callback(
            &state.integration.bridge,
            run_id,
            consumer,
        )
        .await;
    }

    let Some(bound_consumer) = build_bound_delivery_consumer(
        state.clone(),
        thread_id.to_owned(),
        run_id.to_owned(),
        targets,
    ) else {
        return Ok(None);
    };

    // Read this run's stream from the durable committed transcript. The bound
    // delivery buffer is unchanged; only the source changes.
    garyx_channels::committed_replay::committed_callback(
        &state.integration.bridge,
        run_id,
        bound_consumer,
    )
    .await
}

fn build_bound_delivery_consumer(
    state: Arc<AppState>,
    thread_id: String,
    run_id: String,
    targets: Vec<BoundThreadDeliveryTarget>,
) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
    if targets.is_empty() {
        return None;
    }
    let bound_delivery = BoundThreadDeliveryBuffer::with_targets(targets);
    let callback_state = state;
    let callback_thread_id = thread_id;
    let callback_run_id = run_id;
    let callback_delivery = bound_delivery.clone();
    let delayed_flush_scheduled = Arc::new(AtomicBool::new(false));
    let callback_flush_scheduled = delayed_flush_scheduled.clone();

    let bound_consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> =
        Arc::new(move |event| match event {
            StreamEvent::SessionBound { .. } => {}
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
                callback_delivery.dispatch_content_after_flush(
                    callback_state.clone(),
                    callback_thread_id.clone(),
                    callback_run_id.clone(),
                    ChannelOutboundContent::ToolResult {
                        message: message.clone(),
                    },
                    "bound delivery",
                );
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
            StreamEvent::ThreadTitleUpdated { .. } => {}
            StreamEvent::ToolUse { message } => {
                // Flush any accumulated assistant text before a tool call so that
                // channels without native streaming (e.g. WeChat) deliver messages
                // incrementally between tool invocations instead of batching
                // everything until the run completes.
                callback_delivery.dispatch_content_after_flush(
                    callback_state.clone(),
                    callback_thread_id.clone(),
                    callback_run_id.clone(),
                    ChannelOutboundContent::ToolUse { message },
                    "bound delivery",
                );
            }
        });
    Some(bound_consumer)
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
