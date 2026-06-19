use std::sync::Arc;

use garyx_channels::{ChannelError, OutboundMessage, SendMessageResult};
use garyx_models::ChannelOutboundContent;
use garyx_models::thread_logs::ThreadLogEvent;
use garyx_models::{MessageLifecycleStatus, MessageTerminalReason};
use garyx_router::detach_endpoint_from_thread;
use serde_json::json;

use super::images::extract_markdown_image_refs;
use super::plan::BoundThreadDeliveryTarget;
use crate::chat_shared::record_api_thread_log;
use crate::server::AppState;

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

pub(super) async fn deliver_assistant_reply_to_bound_channels(
    state: Arc<AppState>,
    thread_id: String,
    run_id: String,
    text: String,
    targets: Vec<BoundThreadDeliveryTarget>,
) {
    if text.trim().is_empty() {
        return;
    }

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
            content: ChannelOutboundContent::text(text.clone()),
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

pub(super) async fn deliver_markdown_images_to_bound_channels(
    state: Arc<AppState>,
    thread_id: String,
    run_id: String,
    text: &str,
    targets: Vec<BoundThreadDeliveryTarget>,
) {
    let images = extract_markdown_image_refs(text);
    if images.is_empty() {
        return;
    }

    if targets.is_empty() {
        return;
    }

    record_api_thread_log(
        &state,
        ThreadLogEvent::info(
            &thread_id,
            "delivery",
            "markdown image forwarding to bound endpoints",
        )
        .with_run_id(run_id.clone())
        .with_field("target_count", json!(targets.len()))
        .with_field("image_count", json!(images.len())),
    )
    .await;

    let dispatcher = state.channel_dispatcher();
    for target in targets {
        for image in &images {
            let image_path = image.path.to_string_lossy().to_string();
            let request = OutboundMessage {
                channel: target.channel.clone(),
                account_id: target.account_id.clone(),
                chat_id: target.chat_id.clone(),
                delivery_target_type: target.delivery_target_type.clone(),
                delivery_target_id: target.delivery_target_id.clone(),
                content: ChannelOutboundContent::image(image_path.clone(), image.alt.clone()),
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
                            text_excerpt: Some(image_path.chars().take(200).collect()),
                            metadata: Some(json!({
                                "source": "bound_delivery_markdown_image",
                                "message_id_count": message_ids.len(),
                                "content_kind": "image",
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
                            text_excerpt: Some(image_path.chars().take(200).collect()),
                            terminal_reason: Some(MessageTerminalReason::ReplyDispatchFailed),
                            metadata: Some(json!({
                                "source": "bound_delivery_markdown_image",
                                "content_kind": "image",
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
                            "markdown image forwarding failed",
                        )
                        .with_run_id(run_id.clone())
                        .with_field("endpoint_key", json!(target.endpoint_key))
                        .with_field("channel", json!(target.channel))
                        .with_field("account_id", json!(target.account_id))
                        .with_field("chat_id", json!(target.chat_id))
                        .with_field("image_path", json!(image_path))
                        .with_field("error", json!(error.to_string())),
                    )
                    .await;

                    prune_failed_bound_delivery_target(
                        &state, &thread_id, &run_id, &target, &error,
                    )
                    .await;
                }
            }
        }
    }
}

pub(super) async fn deliver_structured_content_to_bound_channels(
    state: Arc<AppState>,
    thread_id: String,
    run_id: String,
    content: ChannelOutboundContent,
    targets: Vec<BoundThreadDeliveryTarget>,
) {
    if targets.is_empty() {
        return;
    }

    record_api_thread_log(
        &state,
        ThreadLogEvent::info(
            &thread_id,
            "delivery",
            "structured assistant event forwarding to bound endpoints",
        )
        .with_run_id(run_id.clone())
        .with_field("target_count", json!(targets.len()))
        .with_field("content_kind", json!(content.kind())),
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
            content: content.clone(),
            reply_to: None,
            thread_id: target.thread_id.clone(),
        };

        if let Err(error) = dispatcher.send_message(request).await {
            record_api_thread_log(
                &state,
                ThreadLogEvent::warn(
                    &thread_id,
                    "delivery",
                    "structured assistant event forwarding failed",
                )
                .with_run_id(run_id.clone())
                .with_field("endpoint_key", json!(target.endpoint_key))
                .with_field("channel", json!(target.channel))
                .with_field("account_id", json!(target.account_id))
                .with_field("chat_id", json!(target.chat_id))
                .with_field("content_kind", json!(content.kind()))
                .with_field("error", json!(error.to_string())),
            )
            .await;
        }
    }
}
