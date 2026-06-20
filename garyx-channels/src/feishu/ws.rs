use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use garyx_bridge::MultiProviderBridge;
use garyx_models::config::FeishuAccount;
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, PromptAttachment, PromptAttachmentKind, ProviderMessage,
    StreamBoundaryKind, StreamEvent, attachments_to_metadata_value,
};
use garyx_models::{MessageLedgerEvent, MessageLifecycleStatus, MessageTerminalReason};
use garyx_router::{InboundRequest, MessageRouter, endpoint_key};
use prost::Message as ProstMessage;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use super::client::WsConnectInfo;
use super::cot::{FeishuCotEventRecord, FeishuCotSession};
use super::mentions::{build_mention_prefix, extract_mention_targets, is_mention_forward_request};
use super::message::{extract_image_keys, merge_stream_text};
use super::pbbp2::{self, Frame};
use super::{
    FeishuClient, FeishuError, FeishuEventEnvelope, FeishuResponseStreamState,
    ImMessageReceiveEvent, MENTION_CONTEXT_LIMIT, PROCESSING_REACTION_EMOJI, SENDER_NAME_CACHE_TTL,
    TopicSessionMode, WS_MAX_RECONNECT_DELAY, WS_RECONNECT_DELAY, append_pending_history,
    build_card_content, build_text_content, clear_pending_history, extract_message_text,
    get_pending_history, is_duplicate_event, is_mentioned, notify_permission_error_if_needed,
    prune_sender_name_cache_locked, record_policy_block, requires_group_mention,
    resolve_topic_session_mode, send_native_command_reply, sender_name_cache, strip_mention_tokens,
};
use crate::dispatcher::ChannelDispatcher;
use crate::generated_images::{extract_image_generation_result, write_generated_image_temp};

#[derive(Clone)]
pub(super) struct FeishuRuntimeContext<'a> {
    pub(super) account_id: &'a str,
    pub(super) router: &'a Arc<Mutex<MessageRouter>>,
    pub(super) bridge: &'a Arc<MultiProviderBridge>,
    pub(super) dispatcher: &'a Arc<dyn ChannelDispatcher>,
    pub(super) client: &'a FeishuClient,
    pub(super) account: &'a FeishuAccount,
    pub(super) bot_open_id: &'a str,
}

impl<'a> FeishuRuntimeContext<'a> {
    pub(super) fn new(
        account_id: &'a str,
        router: &'a Arc<Mutex<MessageRouter>>,
        bridge: &'a Arc<MultiProviderBridge>,
        dispatcher: &'a Arc<dyn ChannelDispatcher>,
        client: &'a FeishuClient,
        account: &'a FeishuAccount,
        bot_open_id: &'a str,
    ) -> Self {
        Self {
            account_id,
            router,
            bridge,
            dispatcher,
            client,
            account,
            bot_open_id,
        }
    }
}

pub(crate) struct FeishuStreamingCallbackConfig {
    pub(crate) client: FeishuClient,
    pub(crate) router: Arc<Mutex<MessageRouter>>,
    pub(crate) account_id: String,
    pub(crate) receive_id_type: String,
    pub(crate) chat_id: String,
    pub(crate) reply_message_id: Option<String>,
    pub(crate) reply_in_thread: bool,
    pub(crate) is_group_reply: bool,
    pub(crate) mention_prefix: String,
    pub(crate) native_thread_scope: Option<String>,
    pub(crate) processing_reaction_id: Option<String>,
}

async fn notify_feishu_stream_error(
    client: &FeishuClient,
    account_id: &str,
    chat_id: &str,
    reply_message_id: Option<&str>,
    error: &FeishuError,
) {
    let Some(reply_message_id) = reply_message_id else {
        return;
    };
    notify_permission_error_if_needed(
        client,
        account_id,
        chat_id,
        reply_message_id,
        &error.to_string(),
    )
    .await;
}

async fn record_feishu_stream_outbound(
    router: &Arc<Mutex<MessageRouter>>,
    canonical_thread_id: &str,
    account_id: &str,
    chat_id: &str,
    native_thread_scope: Option<&str>,
    outbound_msg_id: &str,
) {
    if outbound_msg_id.is_empty() || canonical_thread_id.is_empty() {
        return;
    }
    let mut router = router.lock().await;
    router
        .record_outbound_message_with_persistence(
            canonical_thread_id,
            "feishu",
            account_id,
            chat_id,
            native_thread_scope,
            outbound_msg_id,
        )
        .await;
}

async fn send_feishu_stream_text(
    cfg: &FeishuStreamingCallbackConfig,
    canonical_thread_id: &str,
    text: &str,
) -> Result<String, FeishuError> {
    let content = build_card_content(text);
    let result = if let Some(reply_message_id) = cfg.reply_message_id.as_deref() {
        if cfg.is_group_reply {
            cfg.client
                .reply_message_ext(
                    reply_message_id,
                    &content,
                    "interactive",
                    cfg.reply_in_thread,
                )
                .await
        } else {
            cfg.client
                .send_message_to_target(&cfg.receive_id_type, &cfg.chat_id, &content, "interactive")
                .await
        }
    } else {
        cfg.client
            .send_message_to_target(&cfg.receive_id_type, &cfg.chat_id, &content, "interactive")
            .await
    };

    if let Ok(outbound_msg_id) = result.as_ref() {
        record_feishu_stream_outbound(
            &cfg.router,
            canonical_thread_id,
            &cfg.account_id,
            &cfg.chat_id,
            cfg.native_thread_scope.as_deref(),
            outbound_msg_id,
        )
        .await;
    }
    result
}

async fn send_feishu_stream_image(
    cfg: &FeishuStreamingCallbackConfig,
    canonical_thread_id: &str,
    image_path: &std::path::Path,
) -> Result<String, FeishuError> {
    let result = if let Some(reply_message_id) = cfg.reply_message_id.as_deref() {
        if cfg.is_group_reply {
            cfg.client
                .reply_image_ext(reply_message_id, image_path, cfg.reply_in_thread)
                .await
        } else {
            cfg.client
                .send_image_to_target(&cfg.receive_id_type, &cfg.chat_id, image_path)
                .await
        }
    } else {
        cfg.client
            .send_image_to_target(&cfg.receive_id_type, &cfg.chat_id, image_path)
            .await
    };

    if let Ok(outbound_msg_id) = result.as_ref() {
        record_feishu_stream_outbound(
            &cfg.router,
            canonical_thread_id,
            &cfg.account_id,
            &cfg.chat_id,
            cfg.native_thread_scope.as_deref(),
            outbound_msg_id,
        )
        .await;
    }
    result
}

pub(crate) fn build_feishu_response_callback(
    cfg: FeishuStreamingCallbackConfig,
) -> (
    Arc<dyn Fn(StreamEvent) + Send + Sync>,
    watch::Sender<String>,
) {
    let state = Arc::new(tokio::sync::Mutex::new(FeishuResponseStreamState {
        processing_reaction_id: cfg.processing_reaction_id.clone(),
        ..FeishuResponseStreamState::default()
    }));
    let (thread_id_tx, thread_id_rx) = watch::channel(String::new());
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<StreamEvent>();
    let cfg = Arc::new(cfg);
    let worker_state = state.clone();

    tokio::spawn(async move {
        let mut thread_id_rx = thread_id_rx;
        while let Some(event) = event_rx.recv().await {
            let mut canonical_thread_id = thread_id_rx.borrow().clone();
            while canonical_thread_id.is_empty() {
                if thread_id_rx.changed().await.is_err() {
                    break;
                }
                canonical_thread_id = thread_id_rx.borrow().clone();
            }
            if canonical_thread_id.is_empty() {
                continue;
            }

            let mut state = worker_state.lock().await;
            let mut text_to_send = String::new();
            let mut send_result: Option<Result<String, FeishuError>> = None;
            match event {
                StreamEvent::SessionBound { .. } => {
                    continue;
                }
                StreamEvent::Boundary { kind, .. } => match kind {
                    StreamBoundaryKind::UserAck => {
                        let mut boundary_text = state.stream_text.clone();
                        if !cfg.mention_prefix.is_empty() {
                            boundary_text = format!("{} {boundary_text}", cfg.mention_prefix);
                        }
                        boundary_text = boundary_text.trim().to_owned();
                        crate::streaming_core::apply_stream_boundary_text(
                            &mut state.stream_text,
                            StreamBoundaryKind::UserAck,
                        );
                        drop(state);

                        if !boundary_text.is_empty()
                            && let Err(error) =
                                send_feishu_stream_text(&cfg, &canonical_thread_id, &boundary_text)
                                    .await
                        {
                            error!(
                                account_id = %cfg.account_id,
                                error = %error,
                                "Failed to send Feishu boundary reply"
                            );
                            notify_feishu_stream_error(
                                &cfg.client,
                                &cfg.account_id,
                                &cfg.chat_id,
                                cfg.reply_message_id.as_deref(),
                                &error,
                            )
                            .await;
                        }
                        continue;
                    }
                    StreamBoundaryKind::AssistantSegment => {
                        crate::streaming_core::apply_stream_boundary_text(
                            &mut state.stream_text,
                            StreamBoundaryKind::AssistantSegment,
                        );
                        continue;
                    }
                },
                StreamEvent::Delta { text } => {
                    if text.is_empty() {
                        continue;
                    }
                    state.stream_text = merge_stream_text(&state.stream_text, &text);
                    continue;
                }
                StreamEvent::ToolUse { message } => {
                    if let Some(reply_message_id) = cfg.reply_message_id.as_deref() {
                        send_pending_stream_text_cot_events(
                            &cfg.client,
                            &mut state,
                            &cfg.account_id,
                            &cfg.chat_id,
                            &canonical_thread_id,
                            reply_message_id,
                        )
                        .await;
                        send_tool_use_cot_events(
                            &cfg.client,
                            &mut state,
                            &cfg.account_id,
                            &cfg.chat_id,
                            &canonical_thread_id,
                            reply_message_id,
                            &message,
                        )
                        .await;
                    }
                    continue;
                }
                StreamEvent::ToolResult { message } => {
                    if let Some(reply_message_id) = cfg.reply_message_id.as_deref() {
                        send_pending_stream_text_cot_events(
                            &cfg.client,
                            &mut state,
                            &cfg.account_id,
                            &cfg.chat_id,
                            &canonical_thread_id,
                            reply_message_id,
                        )
                        .await;
                        send_tool_result_cot_events(
                            &cfg.client,
                            &mut state,
                            &cfg.account_id,
                            &cfg.chat_id,
                            &canonical_thread_id,
                            reply_message_id,
                            &message,
                        )
                        .await;
                    }
                    let Some(image) = extract_image_generation_result(&message) else {
                        continue;
                    };
                    drop(state);

                    let image_path = match write_generated_image_temp("feishu", &image).await {
                        Ok(path) => path,
                        Err(error) => {
                            warn!(
                                account_id = %cfg.account_id,
                                error = %error,
                                "failed to write Feishu generated image temp file"
                            );
                            continue;
                        }
                    };
                    let image_send_result =
                        send_feishu_stream_image(&cfg, &canonical_thread_id, &image_path).await;
                    let _ = tokio::fs::remove_file(&image_path).await;

                    match image_send_result {
                        Ok(outbound_msg_id) => {
                            info!(
                                account_id = %cfg.account_id,
                                outbound_message_id = %outbound_msg_id,
                                "Feishu generated image sent"
                            );
                        }
                        Err(error) => {
                            error!(
                                account_id = %cfg.account_id,
                                error = %error,
                                "Failed to send Feishu generated image"
                            );
                            notify_feishu_stream_error(
                                &cfg.client,
                                &cfg.account_id,
                                &cfg.chat_id,
                                cfg.reply_message_id.as_deref(),
                                &error,
                            )
                            .await;
                        }
                    }
                    continue;
                }
                StreamEvent::ThreadTitleUpdated { .. } => {}
                StreamEvent::Done => {
                    finish_cot_run(
                        &cfg.client,
                        &mut state,
                        &cfg.account_id,
                        &canonical_thread_id,
                    )
                    .await;
                }
            }

            let mut final_text = state.stream_text.clone();
            state.stream_text.clear();

            if !cfg.mention_prefix.is_empty() {
                final_text = format!("{} {final_text}", cfg.mention_prefix);
            }
            final_text = final_text.trim().to_owned();
            if !final_text.is_empty() {
                text_to_send = final_text;
            }

            if !text_to_send.is_empty() {
                send_result =
                    Some(send_feishu_stream_text(&cfg, &canonical_thread_id, &text_to_send).await);
            }

            if !state.processing_reaction_removed {
                if let (Some(reply_message_id), Some(reaction_id)) = (
                    cfg.reply_message_id.as_deref(),
                    state.processing_reaction_id.take(),
                ) && let Err(error) = cfg
                    .client
                    .remove_reaction(reply_message_id, &reaction_id)
                    .await
                {
                    debug!(
                        account_id = %cfg.account_id,
                        message_id = %reply_message_id,
                        reaction_id = %reaction_id,
                        error = %error,
                        "Feishu remove processing reaction skipped"
                    );
                }
                state.processing_reaction_removed = true;
            }

            drop(state);

            if let Some(send_result) = send_result {
                match send_result {
                    Ok(outbound_msg_id) => {
                        info!(
                            account_id = %cfg.account_id,
                            outbound_message_id = %outbound_msg_id,
                            "Feishu reply sent"
                        );
                    }
                    Err(error) => {
                        error!(
                            account_id = %cfg.account_id,
                            error = %error,
                            "Failed to send Feishu reply"
                        );
                        notify_feishu_stream_error(
                            &cfg.client,
                            &cfg.account_id,
                            &cfg.chat_id,
                            cfg.reply_message_id.as_deref(),
                            &error,
                        )
                        .await;
                    }
                }
            }
        }
    });

    let response_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> =
        Arc::new(move |event: StreamEvent| {
            let _ = event_tx.send(event);
        });

    (response_callback, thread_id_tx)
}

fn resolve_native_thread_scope(
    message: &super::ImMessage,
    topic_session_mode: &TopicSessionMode,
) -> Option<String> {
    if message.chat_type != "group" {
        return None;
    }

    match topic_session_mode {
        TopicSessionMode::Disabled => Some(message.chat_id.clone()),
        TopicSessionMode::Enabled => {
            let topic_id = message.root_id.trim();
            if !topic_id.is_empty() {
                return Some(format!("{}:topic:{}", message.chat_id, topic_id));
            }
            let parent_id = message.parent_id.trim();
            if !parent_id.is_empty() {
                return Some(format!("{}:topic:{}", message.chat_id, parent_id));
            }
            Some(message.chat_id.clone())
        }
    }
}

async fn ensure_cot_session(
    client: &FeishuClient,
    state: &mut FeishuResponseStreamState,
    account_id: &str,
    chat_id: &str,
    thread_id: &str,
    origin_message_id: &str,
) -> Option<super::cot::FeishuCotSession> {
    if state.cot.failed {
        return None;
    }
    if let Some(session) = state.cot.session.clone() {
        return Some(session);
    }

    let run_started = state.cot.run_started_event(thread_id, thread_id);
    match client
        .create_cot_run_start_message(chat_id, thread_id, Some(origin_message_id), run_started)
        .await
    {
        Ok(session) => {
            state.cot.session = Some(session.clone());
            Some(session)
        }
        Err(err) => {
            state.cot.failed = true;
            warn!(
                account_id = %account_id,
                thread_id = %thread_id,
                error = %err,
                "Feishu COT run creation failed; continuing with Card Kit reply"
            );
            None
        }
    }
}

async fn send_cot_events(
    client: &FeishuClient,
    state: &mut FeishuResponseStreamState,
    account_id: &str,
    chat_id: &str,
    thread_id: &str,
    origin_message_id: &str,
    events: Vec<FeishuCotEventRecord>,
    label: &str,
) {
    if events.is_empty() || state.cot.failed {
        return;
    }
    let Some(session) = ensure_cot_session(
        client,
        state,
        account_id,
        chat_id,
        thread_id,
        origin_message_id,
    )
    .await
    else {
        return;
    };
    log_cot_events(account_id, thread_id, label, &session, &events);
    if let Err(err) = client.update_cot_events(&session, &events).await {
        state.cot.failed = true;
        warn!(
            account_id = %account_id,
            thread_id = %thread_id,
            label = %label,
            error = %err,
            "Feishu COT event update failed; continuing with Card Kit reply"
        );
    }
}

fn log_cot_events(
    account_id: &str,
    thread_id: &str,
    label: &str,
    session: &FeishuCotSession,
    events: &[FeishuCotEventRecord],
) {
    for event in events {
        let content = serde_json::from_str::<Value>(&event.content).ok();
        let tool_call_id = content
            .as_ref()
            .and_then(|value| value.get("toolCallId"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = content
            .as_ref()
            .and_then(|value| value.get("title"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool_call_name = content
            .as_ref()
            .and_then(|value| value.get("toolCallName"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let delta = content
            .as_ref()
            .and_then(|value| value.get("delta"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        info!(
            account_id = %account_id,
            thread_id = %thread_id,
            label = %label,
            cot_id = %session.cot_id,
            cot_message_id = %session.message_id,
            event_type = %event.event_type,
            event_id = %event.event_id,
            tool_call_id = %tool_call_id,
            title = %truncate_log_field(title),
            tool_call_name = %truncate_log_field(tool_call_name),
            delta = %truncate_log_field(delta),
            content = %truncate_log_field(&event.content),
            "Feishu COT event outgoing"
        );
    }
}

fn truncate_log_field(value: &str) -> String {
    const MAX: usize = 240;
    if value.len() <= MAX {
        return value.to_owned();
    }
    let mut end = MAX.saturating_sub(15);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...[truncated]", &value[..end])
}

async fn send_tool_use_cot_events(
    client: &FeishuClient,
    state: &mut FeishuResponseStreamState,
    account_id: &str,
    chat_id: &str,
    thread_id: &str,
    origin_message_id: &str,
    message: &ProviderMessage,
) {
    let events = state.cot.tool_use_events(message);
    send_cot_events(
        client,
        state,
        account_id,
        chat_id,
        thread_id,
        origin_message_id,
        events,
        "tool_use",
    )
    .await;
}

async fn send_pending_stream_text_cot_events(
    client: &FeishuClient,
    state: &mut FeishuResponseStreamState,
    account_id: &str,
    chat_id: &str,
    thread_id: &str,
    origin_message_id: &str,
) {
    let text = state.stream_text.trim().to_owned();
    if text.is_empty() {
        return;
    }
    let events = state.cot.text_message_events(&text);
    send_cot_events(
        client,
        state,
        account_id,
        chat_id,
        thread_id,
        origin_message_id,
        events,
        "stream_text",
    )
    .await;
    if !state.cot.failed {
        state.stream_text.clear();
    }
}

async fn send_tool_result_cot_events(
    client: &FeishuClient,
    state: &mut FeishuResponseStreamState,
    account_id: &str,
    chat_id: &str,
    thread_id: &str,
    origin_message_id: &str,
    message: &ProviderMessage,
) {
    let events = state.cot.tool_result_events(message);
    send_cot_events(
        client,
        state,
        account_id,
        chat_id,
        thread_id,
        origin_message_id,
        events,
        "tool_result",
    )
    .await;
}

async fn finish_cot_run(
    client: &FeishuClient,
    state: &mut FeishuResponseStreamState,
    account_id: &str,
    thread_id: &str,
) {
    if state.cot.failed || state.cot.completed {
        return;
    }
    let Some(session) = state.cot.session.clone() else {
        return;
    };
    let run_finished = state.cot.run_finished_event(thread_id, &session.cot_id);
    if let Err(err) = client.update_cot_events(&session, &[run_finished]).await {
        state.cot.failed = true;
        warn!(
            account_id = %account_id,
            thread_id = %thread_id,
            error = %err,
            "Feishu COT run finish update failed"
        );
        return;
    }
    if let Err(err) = client.complete_cot_run(&session).await {
        state.cot.failed = true;
        warn!(
            account_id = %account_id,
            thread_id = %thread_id,
            error = %err,
            "Feishu COT run complete failed"
        );
        return;
    }
    state.cot.completed = true;
}

async fn record_terminal_inbound_event(
    runtime: FeishuRuntimeContext<'_>,
    message: &super::ImMessage,
    from_id: &str,
    text_excerpt: &str,
    status: MessageLifecycleStatus,
    terminal_reason: MessageTerminalReason,
    metadata: Value,
) {
    let topic_session_mode = resolve_topic_session_mode(runtime.account);
    let native_thread_scope = resolve_native_thread_scope(message, &topic_session_mode);
    let thread_binding_key = native_thread_scope.unwrap_or_else(|| from_id.to_owned());

    let router_guard = runtime.router.lock().await;
    router_guard
        .record_message_ledger_event(MessageLedgerEvent {
            ledger_id: format!(
                "feishu:{}:{}:{}",
                runtime.account_id, message.chat_id, message.message_id
            ),
            bot_id: format!("feishu:{}", runtime.account_id),
            status,
            created_at: chrono::Utc::now().to_rfc3339(),
            thread_id: Some(thread_binding_key),
            run_id: None,
            channel: Some("feishu".to_owned()),
            account_id: Some(runtime.account_id.to_owned()),
            chat_id: Some(message.chat_id.clone()),
            from_id: Some(from_id.to_owned()),
            native_message_id: Some(message.message_id.clone()),
            text_excerpt: Some(text_excerpt.chars().take(200).collect()),
            terminal_reason: Some(terminal_reason),
            reply_message_id: None,
            metadata,
        })
        .await;
}

pub(super) async fn ws_listen_loop(
    account_id: &str,
    client: &FeishuClient,
    account: &FeishuAccount,
    running: Arc<AtomicBool>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    dispatcher: Arc<dyn ChannelDispatcher>,
    _public_url: &str,
) {
    let mut reconnect_delay = WS_RECONNECT_DELAY;
    let mut bot_open_id = String::new();

    match client.fetch_bot_open_id().await {
        Ok(Some(open_id)) => {
            bot_open_id = open_id;
            info!(
                account_id = %account_id,
                bot_open_id = %bot_open_id,
                "Resolved Feishu bot open_id"
            );
        }
        Ok(None) => {
            warn!(
                account_id = %account_id,
                "Feishu bot open_id is empty; mention detection will use permissive fallback"
            );
        }
        Err(err) => {
            warn!(
                account_id = %account_id,
                error = %err,
                "Failed to fetch Feishu bot open_id; mention detection will use permissive fallback"
            );
        }
    }

    while running.load(Ordering::SeqCst) {
        // Ensure token is fresh before connecting
        if let Err(e) = client.refresh_token_if_needed().await {
            error!(
                account_id = %account_id,
                error = %e,
                "Failed to refresh token before WS connect"
            );
            tokio::time::sleep(reconnect_delay).await;
            reconnect_delay = (reconnect_delay * 3 / 2).min(WS_MAX_RECONNECT_DELAY);
            continue;
        }

        // Get WS endpoint URL and config
        let connect_info = match client.get_ws_endpoint().await {
            Ok(info) => info,
            Err(e) => {
                error!(
                    account_id = %account_id,
                    error = %e,
                    "Failed to get WebSocket endpoint"
                );
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 3 / 2).min(WS_MAX_RECONNECT_DELAY);
                continue;
            }
        };

        info!(
            account_id = %account_id,
            service_id = connect_info.service_id,
            ping_interval = connect_info.ping_interval_secs,
            reconnect_interval = connect_info.reconnect_interval_secs,
            "Connecting to Feishu WebSocket"
        );
        let reconnect_base_delay = Duration::from_secs(connect_info.reconnect_interval_secs.max(1));
        reconnect_delay = reconnect_delay.max(reconnect_base_delay);
        let runtime = FeishuRuntimeContext::new(
            account_id,
            &router,
            &bridge,
            &dispatcher,
            client,
            account,
            &bot_open_id,
        );

        match ws_connect_and_listen(&connect_info, &running, runtime).await {
            Ok(()) => {
                info!(account_id = %account_id, "WebSocket connection ended normally");
                reconnect_delay = reconnect_base_delay; // Reset on clean close
            }
            Err(e) => {
                if !running.load(Ordering::SeqCst) {
                    debug!(account_id = %account_id, "WebSocket closed during shutdown");
                    break;
                }
                warn!(
                    account_id = %account_id,
                    error = %e,
                    delay_secs = reconnect_delay.as_secs(),
                    "WebSocket disconnected, reconnecting"
                );
            }
        }

        if !running.load(Ordering::SeqCst) {
            break;
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay =
            ((reconnect_delay * 3 / 2).min(WS_MAX_RECONNECT_DELAY)).max(reconnect_base_delay);
    }

    info!(account_id = %account_id, "WebSocket listener loop exited");
}

async fn ws_connect_and_listen(
    connect_info: &WsConnectInfo,
    running: &Arc<AtomicBool>,
    runtime: FeishuRuntimeContext<'_>,
) -> Result<(), FeishuError> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(&connect_info.ws_url)
        .await
        .map_err(|e| FeishuError::WebSocket(e.to_string()))?;

    let (mut ws_write, mut ws_read) = ws_stream.split();

    info!(account_id = %runtime.account_id, "Feishu WebSocket connected");

    let ping_interval = Duration::from_secs(connect_info.ping_interval_secs.max(10));
    let service_id = connect_info.service_id;

    while running.load(Ordering::SeqCst) {
        let msg = tokio::select! {
            msg = ws_read.next() => msg,
            _ = tokio::time::sleep(ping_interval) => {
                // Send protobuf-encoded ping frame
                let ping_frame = Frame::ping(service_id);
                let encoded = ping_frame.encode_to_vec();
                if let Err(e) = ws_write.send(tokio_tungstenite::tungstenite::Message::Binary(encoded.into())).await {
                    warn!(account_id = %runtime.account_id, error = %e, "Failed to send pbbp2 ping");
                    break;
                }
                debug!(account_id = %runtime.account_id, "Sent pbbp2 ping");
                continue;
            }
        };

        let msg = match msg {
            Some(Ok(m)) => m,
            Some(Err(e)) => {
                return Err(FeishuError::WebSocket(e.to_string()));
            }
            None => {
                return Ok(()); // Stream ended
            }
        };

        match msg {
            tokio_tungstenite::tungstenite::Message::Binary(data) => {
                let frame = match Frame::decode(data.as_ref()) {
                    Ok(f) => f,
                    Err(e) => {
                        warn!(account_id = %runtime.account_id, error = %e, "Failed to decode pbbp2 frame");
                        continue;
                    }
                };

                if frame.method == pbbp2::METHOD_CONTROL {
                    // Control frame (ping/pong) — just log pong
                    let msg_type = frame.header_value(pbbp2::HEADER_TYPE).unwrap_or("");
                    if msg_type == pbbp2::MSG_TYPE_PONG {
                        debug!(account_id = %runtime.account_id, "Received pbbp2 pong");
                    }
                } else if frame.method == pbbp2::METHOD_DATA {
                    let msg_type = frame.header_value(pbbp2::HEADER_TYPE).unwrap_or("");
                    if msg_type == pbbp2::MSG_TYPE_EVENT {
                        let start = std::time::Instant::now();

                        // ACK before processing so Feishu won't re-deliver on reconnect.
                        // This gives at-most-once semantics: events may be lost on crash
                        // but will never be processed twice (no duplicate replies).
                        let ack = Frame::event_ack(&frame, 0);
                        let encoded = ack.encode_to_vec();
                        if let Err(e) = ws_write
                            .send(tokio_tungstenite::tungstenite::Message::Binary(
                                encoded.into(),
                            ))
                            .await
                        {
                            warn!(account_id = %runtime.account_id, error = %e, "Failed to send event ack");
                        }

                        // The payload is the event JSON
                        if let Some(payload) = frame.payload_str() {
                            handle_ws_event_payload(payload, runtime.clone()).await;
                        }

                        debug!(account_id = %runtime.account_id, elapsed_ms = start.elapsed().as_millis(), "Event processed");
                    } else {
                        debug!(
                            account_id = %runtime.account_id,
                            msg_type = %msg_type,
                            "Ignoring non-event data frame"
                        );
                    }
                }
            }
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                // Fallback: some versions may send JSON text
                handle_ws_event_payload(&text, runtime.clone()).await;
            }
            tokio_tungstenite::tungstenite::Message::Ping(data) => {
                if let Err(e) = ws_write
                    .send(tokio_tungstenite::tungstenite::Message::Pong(data))
                    .await
                {
                    warn!(account_id = %runtime.account_id, error = %e, "Failed to send pong");
                }
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => {
                info!(account_id = %runtime.account_id, "Feishu WebSocket received close frame");
                return Ok(());
            }
            _ => {}
        }
    }

    Ok(())
}

/// Parse an event JSON payload (either from protobuf frame payload or raw text).
async fn handle_ws_event_payload(text: &str, runtime: FeishuRuntimeContext<'_>) {
    let envelope: FeishuEventEnvelope = match serde_json::from_str(text) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                account_id = %runtime.account_id,
                error = %e,
                payload_preview = %&text[..text.len().min(200)],
                "Failed to parse Feishu event payload"
            );
            return;
        }
    };

    let header = match &envelope.header {
        Some(h) => h,
        None => return,
    };

    if !header.event_id.is_empty() && is_duplicate_event(&header.event_id) {
        debug!(
            account_id = %runtime.account_id,
            event_id = %header.event_id,
            "Feishu duplicate event dropped"
        );
        return;
    }

    let event_type = &header.event_type;

    if event_type == "im.message.receive_v1" {
        if let Some(event_value) = &envelope.event {
            match serde_json::from_value::<ImMessageReceiveEvent>(event_value.clone()) {
                Ok(event) => {
                    handle_im_message_event(&event, runtime).await;
                }
                Err(e) => {
                    warn!(
                        account_id = %runtime.account_id,
                        error = %e,
                        "Failed to parse im.message.receive_v1 event"
                    );
                }
            }
        }
    } else {
        debug!(
            account_id = %runtime.account_id,
            event_type = %event_type,
            "Ignoring unhandled Feishu event type"
        );
    }
}

pub(super) async fn handle_im_message_event(
    event: &ImMessageReceiveEvent,
    runtime: FeishuRuntimeContext<'_>,
) {
    let message = match &event.message {
        Some(m) => m,
        None => return,
    };
    let sender = match &event.sender {
        Some(s) => s,
        None => return,
    };

    // Skip messages from bot apps
    if sender.sender_type == "app" {
        return;
    }

    let sender_open_id = sender
        .sender_id
        .as_ref()
        .map(|s| s.open_id.as_str())
        .unwrap_or("");
    let is_group = message.chat_type == "group";
    // For group messages, use chat_id as from_id so all users in the same group
    // share one thread. For DMs, use the sender's open_id for per-user threading.
    let from_id = if is_group || sender_open_id.is_empty() {
        message.chat_id.clone()
    } else {
        sender_open_id.to_owned()
    };
    let sender_name = resolve_sender_name(runtime.client, sender_open_id).await;
    let speaker = sender_name.as_deref().unwrap_or(sender_open_id);

    let text = extract_message_text(&message.message_type, &message.content);

    // Download image attachments to local disk so prompt metadata can carry
    // gateway-local absolute paths just like regular files.
    let image_attachments = extract_feishu_image_attachments(
        runtime.client,
        &message.message_id,
        &message.message_type,
        &message.content,
    )
    .await;

    // Download file/audio/video attachments to local disk so the agent
    // thread can reference them by path.
    let file_paths = extract_feishu_file_paths(
        runtime.client,
        &message.message_id,
        &message.message_type,
        &message.content,
    )
    .await;

    if text.is_empty() && file_paths.is_empty() && image_attachments.is_empty() {
        return;
    }

    info!(
        account_id = %runtime.account_id,
        chat_id = %message.chat_id,
        chat_type = %message.chat_type,
        message_id = %message.message_id,
        root_id = %message.root_id,
        parent_id = %message.parent_id,
        sender = %sender_open_id,
        text_len = text.len(),
        "Feishu message received"
    );

    // -----------------------------------------------------------------------
    // Mention handling and session scoping
    // -----------------------------------------------------------------------

    let topic_session_mode = resolve_topic_session_mode(runtime.account);

    let mention_targets = extract_mention_targets(&message.mentions, Some(runtime.bot_open_id));
    let is_mention_forward =
        is_mention_forward_request(&message.mentions, runtime.bot_open_id, &message.chat_type);
    let mention_prefix = build_mention_prefix(&mention_targets);

    // Strip mention tokens from text
    let clean_text = strip_mention_tokens(&text, &message.mentions);
    if clean_text.is_empty() && file_paths.is_empty() && image_attachments.is_empty() {
        return;
    }

    let mentioned_bot = is_mentioned(&message.mentions, runtime.bot_open_id);

    if is_group && requires_group_mention(runtime.account) && !mentioned_bot {
        let speaker = if sender_open_id.is_empty() {
            "unknown"
        } else {
            speaker
        };
        append_pending_history(
            runtime.account_id,
            &message.chat_id,
            format!("{speaker}: {clean_text}"),
            MENTION_CONTEXT_LIMIT,
        );
        record_policy_block("group", "mention_required");
        info!(
            account_id = %runtime.account_id,
            chat_id = %message.chat_id,
            sender = %sender_open_id,
            reason = "mention_required",
            "Feishu group message skipped: bot not mentioned"
        );
        record_terminal_inbound_event(
            runtime,
            message,
            &from_id,
            &clean_text,
            MessageLifecycleStatus::Filtered,
            MessageTerminalReason::RoutingRejected,
            json!({
                "source": "feishu_inbound",
                "reason": "mention_required",
            }),
        )
        .await;
        return;
    }

    let native_thread_scope = resolve_native_thread_scope(message, &topic_session_mode);

    let processing_reaction_id = match runtime
        .client
        .add_reaction(&message.message_id, PROCESSING_REACTION_EMOJI)
        .await
    {
        Ok(reaction_id) => reaction_id,
        Err(err) => {
            debug!(
                account_id = %runtime.account_id,
                message_id = %message.message_id,
                error = %err,
                "Feishu processing reaction skipped"
            );
            None
        }
    };

    let pending_history = if is_group {
        get_pending_history(runtime.account_id, &message.chat_id)
    } else {
        Vec::new()
    };
    let mut route_text = clean_text.clone();
    if !speaker.is_empty() {
        route_text = format!("{speaker}: {route_text}");
    }
    if !pending_history.is_empty() {
        route_text = format!("{}\n{}", pending_history.join("\n"), route_text);
    }
    if is_mention_forward && !mention_targets.is_empty() {
        let target_names = mention_targets
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        route_text.push_str(&format!(
            "\n\n[System: Your reply will automatically @mention: {target_names}. Do not write @xxx yourself.]"
        ));
    }
    // Resolve thread context, then dispatch through router one-call path.
    let reply_thread_id = if message.parent_id.is_empty() {
        None
    } else {
        let router_guard = runtime.router.lock().await;
        router_guard
            .resolve_reply_thread_for_chat(
                "feishu",
                runtime.account_id,
                Some(&message.chat_id),
                native_thread_scope.as_deref(),
                &message.parent_id,
            )
            .map(|s| s.to_owned())
    };
    if reply_thread_id.is_none()
        && !message.parent_id.is_empty()
        && let Ok(Some(quoted)) = runtime.client.fetch_message_text(&message.parent_id).await
    {
        route_text = format!("[Replying to: \"{quoted}\"]\n\n{route_text}");
    }

    let mut switched_thread_id_for_notice: Option<String> = None;
    if let Some(reply_thread_id) = reply_thread_id
        && MessageRouter::is_scheduled_thread(&reply_thread_id)
    {
        let thread_binding_key = native_thread_scope.as_deref().unwrap_or(&from_id);
        let binding_context_key = MessageRouter::build_binding_context_key(
            "feishu",
            runtime.account_id,
            thread_binding_key,
        );
        {
            let mut router_guard = runtime.router.lock().await;
            router_guard.switch_to_thread(&binding_context_key, &reply_thread_id);
        }
        switched_thread_id_for_notice = Some(reply_thread_id);
    }

    let run_id = uuid::Uuid::new_v4().to_string();

    let mut metadata = HashMap::new();
    metadata.insert("channel".to_owned(), Value::String("feishu".to_owned()));
    metadata.insert(
        "account_id".to_owned(),
        Value::String(runtime.account_id.to_owned()),
    );
    metadata.insert("chat_id".to_owned(), Value::String(message.chat_id.clone()));
    metadata.insert(
        "message_id".to_owned(),
        Value::String(message.message_id.clone()),
    );
    metadata.insert(
        "sender_open_id".to_owned(),
        Value::String(sender_open_id.to_owned()),
    );
    metadata.insert(
        garyx_router::NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String(clean_text.clone()),
    );
    metadata.insert(
        "message_type".to_owned(),
        Value::String(message.message_type.clone()),
    );
    metadata.insert("mentioned_bot".to_owned(), Value::Bool(mentioned_bot));
    metadata.insert(
        "event_type".to_owned(),
        Value::String("im.message.receive_v1".to_owned()),
    );
    metadata.insert("root_id".to_owned(), Value::String(message.root_id.clone()));
    metadata.insert(
        "parent_id".to_owned(),
        Value::String(message.parent_id.clone()),
    );
    if let Some(ref name) = sender_name {
        metadata.insert("sender_name".to_owned(), Value::String(name.clone()));
    }
    metadata.insert(
        "thread_history_limit".to_owned(),
        Value::Number(serde_json::Number::from(MENTION_CONTEXT_LIMIT)),
    );
    metadata.insert(
        "topic_session_mode".to_owned(),
        Value::String(
            match topic_session_mode {
                TopicSessionMode::Disabled => "disabled",
                TopicSessionMode::Enabled => "enabled",
            }
            .to_owned(),
        ),
    );
    if is_group {
        metadata.insert("is_group".to_owned(), Value::Bool(true));
    }
    metadata.insert(
        "is_mention_forward".to_owned(),
        Value::Bool(is_mention_forward),
    );
    metadata.insert(
        "mention_target_names".to_owned(),
        Value::Array(
            mention_targets
                .iter()
                .map(|target| Value::String(target.name.clone()))
                .collect(),
        ),
    );
    if !image_attachments.is_empty() {
        metadata.insert(
            "image_count".to_owned(),
            Value::Number(serde_json::Number::from(image_attachments.len() as u64)),
        );
        metadata.insert(
            ATTACHMENTS_METADATA_KEY.to_owned(),
            attachments_to_metadata_value(&image_attachments),
        );
    }
    match native_thread_scope.as_ref() {
        Some(thread_scope) => {
            metadata.insert(
                "delivery_thread_id".to_owned(),
                Value::String(thread_scope.clone()),
            );
        }
        None => {
            metadata.insert("delivery_thread_id".to_owned(), Value::Null);
        }
    }

    let (response_callback, thread_id_tx) =
        build_feishu_response_callback(FeishuStreamingCallbackConfig {
            client: runtime.client.clone(),
            router: runtime.router.clone(),
            account_id: runtime.account_id.to_owned(),
            receive_id_type: "chat_id".to_owned(),
            chat_id: message.chat_id.clone(),
            reply_message_id: Some(message.message_id.clone()),
            reply_in_thread: false,
            is_group_reply: is_group,
            mention_prefix: mention_prefix.clone(),
            native_thread_scope: native_thread_scope.clone(),
            processing_reaction_id,
        });

    let request = InboundRequest {
        channel: "feishu".to_owned(),
        account_id: runtime.account_id.to_owned(),
        from_id: from_id.clone(),
        is_group,
        thread_binding_key: native_thread_scope.unwrap_or_else(|| from_id.clone()),
        message: route_text,
        run_id,
        reply_to_message_id: if message.parent_id.is_empty() {
            None
        } else {
            Some(message.parent_id.clone())
        },
        images: Vec::new(),
        extra_metadata: metadata,
        file_paths,
    };

    let origin_endpoint_identity =
        endpoint_key("feishu", runtime.account_id, &request.thread_binding_key);
    let deferred_fanout = crate::bound_fanout::DeferredBoundStreamFanout::new(
        runtime.router.clone(),
        runtime.dispatcher.clone(),
        request.run_id.clone(),
        origin_endpoint_identity,
    );
    let fanout_consumer = deferred_fanout.consumer(response_callback);

    // Read this run's stream from the durable committed transcript: subscribe
    // before dispatch and let the replay adapter drive the Feishu sender.
    // Bound non-origin endpoints attach after route_and_dispatch resolves the
    // canonical thread id.
    let replay_subscription = match crate::committed_replay::committed_callback(
        runtime.bridge,
        &request.run_id,
        fanout_consumer,
    )
    .await
    {
        Ok(subscription) => subscription,
        Err(error) => {
            tracing::error!(run_id = %request.run_id, error = %error, "committed replay bus missing for Feishu dispatch");
            return;
        }
    };

    let thread_store = {
        let router_guard = runtime.router.lock().await;
        router_guard.thread_store()
    };
    let dispatch_delegate = crate::bound_fanout::DeferredFanoutAgentDispatcher::new(
        runtime.bridge.as_ref(),
        deferred_fanout.clone(),
        thread_store,
    );
    let dispatch_callback = replay_subscription.callback();

    let dispatch_result = {
        let mut router_guard = runtime.router.lock().await;
        router_guard
            .record_message_ledger_event(MessageLedgerEvent {
                ledger_id: format!(
                    "feishu:{}:{}:{}",
                    runtime.account_id, message.chat_id, message.message_id
                ),
                bot_id: format!("feishu:{}", runtime.account_id),
                status: MessageLifecycleStatus::Received,
                created_at: chrono::Utc::now().to_rfc3339(),
                thread_id: Some(request.thread_binding_key.clone()),
                run_id: Some(request.run_id.clone()),
                channel: Some(request.channel.clone()),
                account_id: Some(request.account_id.clone()),
                chat_id: Some(message.chat_id.clone()),
                from_id: Some(request.from_id.clone()),
                native_message_id: Some(message.message_id.clone()),
                text_excerpt: Some(request.message.chars().take(200).collect()),
                terminal_reason: None,
                reply_message_id: None,
                metadata: serde_json::json!({
                    "source": "feishu_inbound",
                    "is_group": is_group,
                    "message_type": message.message_type,
                }),
            })
            .await;
        router_guard
            .route_and_dispatch(request, &dispatch_delegate, dispatch_callback)
            .await
    };

    match dispatch_result {
        Ok(result) => {
            deferred_fanout.attach_thread(&result.thread_id).await;
            let _ = thread_id_tx.send(result.thread_id.clone());
            let local_reply = result.local_reply;
            if local_reply.is_some() {
                replay_subscription.abort();
            } else {
                replay_subscription.detach();
            }
            if let Some(local_reply) = local_reply {
                match send_native_command_reply(runtime.client, &message.message_id, &local_reply)
                    .await
                {
                    Ok(()) => {
                        info!(
                            account_id = %runtime.account_id,
                            chat_id = %message.chat_id,
                            thread_id = %result.thread_id,
                            "native command handled by router"
                        );
                    }
                    Err(err) => {
                        error!(
                            account_id = %runtime.account_id,
                            chat_id = %message.chat_id,
                            message_id = %message.message_id,
                            error = %err,
                            "failed to send native command reply"
                        );
                        notify_permission_error_if_needed(
                            runtime.client,
                            runtime.account_id,
                            &message.chat_id,
                            &message.message_id,
                            &err.to_string(),
                        )
                        .await;
                    }
                }
            } else {
                if is_group && !pending_history.is_empty() {
                    clear_pending_history(runtime.account_id, &message.chat_id);
                }
                info!(
                    account_id = %runtime.account_id,
                    chat_id = %message.chat_id,
                    thread_id = %result.thread_id,
                    "resolved thread for Feishu message"
                );
                if let Some(switched_thread_id) = switched_thread_id_for_notice {
                    let notice_content =
                        build_text_content(&format!("你已经切换到 thread:{switched_thread_id}"));
                    if let Err(err) = runtime
                        .client
                        .reply_message(&message.message_id, &notice_content, "text")
                        .await
                    {
                        warn!(
                            account_id = %runtime.account_id,
                            chat_id = %message.chat_id,
                            message_id = %message.message_id,
                            thread_id = %switched_thread_id,
                            error = %err,
                            "failed to send scheduled-thread switch notice"
                        );
                    }
                }
            }
        }
        Err(e) => {
            error!(
                account_id = %runtime.account_id,
                chat_id = %message.chat_id,
                error = %e,
                "failed to route+dispatch Feishu message"
            );
            notify_permission_error_if_needed(
                runtime.client,
                runtime.account_id,
                &message.chat_id,
                &message.message_id,
                &e,
            )
            .await;
        }
    }
}

async fn resolve_sender_name(client: &FeishuClient, sender_open_id: &str) -> Option<String> {
    if sender_open_id.is_empty() {
        return None;
    }

    {
        let now = Instant::now();
        match sender_name_cache().lock() {
            Ok(cache) => {
                if let Some((cached_name, expires_at)) = cache.get(sender_open_id)
                    && *expires_at > now
                {
                    return Some(cached_name.clone());
                }
            }
            Err(_) => {
                warn!("sender name cache mutex poisoned");
            }
        }
    }

    let fetched: Option<String> = match client.fetch_user_display_name(sender_open_id).await {
        Ok(name) => name,
        Err(err) => {
            warn!(
                sender_open_id = %sender_open_id,
                error = %err,
                "Failed to fetch Feishu user display name"
            );
            return None;
        }
    };
    let fetched = fetched?;

    {
        let now = Instant::now();
        match sender_name_cache().lock() {
            Ok(mut cache) => {
                prune_sender_name_cache_locked(&mut cache, now, Some(sender_open_id));
                cache.insert(
                    sender_open_id.to_owned(),
                    (fetched.clone(), now + SENDER_NAME_CACHE_TTL),
                );
            }
            Err(_) => {
                warn!("sender name cache mutex poisoned");
            }
        }
    }

    Some(fetched)
}

// ---------------------------------------------------------------------------
// Feishu file download helpers
// ---------------------------------------------------------------------------

fn sanitize_feishu_filename(name: &str) -> String {
    crate::sanitize_filename(name)
}

/// Max image size for base64 payload (10 MB).
const MAX_FEISHU_IMAGE_SIZE: usize = 10 * 1024 * 1024;

async fn persist_feishu_inbound_bytes(name: &str, bytes: &[u8]) -> Option<String> {
    let base_dir = std::env::temp_dir().join("garyx-feishu").join("inbound");
    if tokio::fs::create_dir_all(&base_dir).await.is_err() {
        warn!("failed to create feishu inbound temp dir for images");
        return None;
    }
    let path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), name));
    if tokio::fs::write(&path, bytes).await.is_err() {
        warn!("failed to write feishu image to disk");
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

/// Download image attachments from a Feishu message to local disk.
async fn extract_feishu_image_attachments(
    client: &FeishuClient,
    message_id: &str,
    message_type: &str,
    content_json: &str,
) -> Vec<PromptAttachment> {
    let keys = extract_image_keys(message_type, content_json);
    if keys.is_empty() {
        return Vec::new();
    }

    let mut attachments = Vec::new();
    for key in keys {
        match client
            .download_message_resource(message_id, &key, "image")
            .await
        {
            Ok(bytes) if !bytes.is_empty() && bytes.len() <= MAX_FEISHU_IMAGE_SIZE => {
                let media_type = infer_image_media_type(&bytes);
                let name = sanitize_feishu_filename(&format!(
                    "feishu-{}.{}",
                    key,
                    media_type.rsplit('/').next().unwrap_or("jpg")
                ));
                if let Some(path) = persist_feishu_inbound_bytes(&name, &bytes).await {
                    attachments.push(PromptAttachment {
                        kind: PromptAttachmentKind::Image,
                        path,
                        name,
                        media_type,
                    });
                }
            }
            Ok(bytes) if bytes.is_empty() => {
                warn!(image_key = %key, "feishu image download returned empty bytes");
            }
            Ok(bytes) => {
                warn!(
                    image_key = %key,
                    size = bytes.len(),
                    "feishu image too large, skipping"
                );
            }
            Err(e) => {
                warn!(image_key = %key, error = %e, "failed to download feishu image");
            }
        }
    }
    attachments
}

fn infer_image_media_type(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png".to_owned()
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg".to_owned()
    } else if bytes.starts_with(b"GIF8") {
        "image/gif".to_owned()
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "image/webp".to_owned()
    } else {
        "image/jpeg".to_owned()
    }
}

/// Download file/audio/video attachments from a Feishu message to local disk.
///
/// Returns a list of local file paths. Returns an empty vec for message types
/// that don't carry downloadable attachments (text, post, etc.).
async fn extract_feishu_file_paths(
    client: &FeishuClient,
    message_id: &str,
    message_type: &str,
    content_json: &str,
) -> Vec<String> {
    let parsed: serde_json::Value = match serde_json::from_str(content_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let items: Vec<(&str, &str, String)> = match message_type {
        "file" => {
            let file_key = parsed
                .get("file_key")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let file_name = parsed
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("file.bin");
            if file_key.is_empty() {
                return Vec::new();
            }
            vec![("file", file_key, sanitize_feishu_filename(file_name))]
        }
        "audio" => {
            let file_key = parsed
                .get("file_key")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if file_key.is_empty() {
                return Vec::new();
            }
            vec![("file", file_key, "audio.opus".to_owned())]
        }
        "video" => {
            let file_key = parsed
                .get("file_key")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if file_key.is_empty() {
                return Vec::new();
            }
            vec![("file", file_key, "video.mp4".to_owned())]
        }
        _ => return Vec::new(),
    };

    let base_dir = std::env::temp_dir().join("garyx-feishu").join("inbound");
    if tokio::fs::create_dir_all(&base_dir).await.is_err() {
        warn!("failed to create feishu inbound temp dir");
        return Vec::new();
    }

    let mut paths = Vec::new();
    for (resource_type, file_key, suggested_name) in items {
        match client
            .download_message_resource(message_id, file_key, resource_type)
            .await
        {
            Ok(bytes) if !bytes.is_empty() => {
                let local_path =
                    base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), suggested_name));
                if tokio::fs::write(&local_path, &bytes).await.is_ok() {
                    paths.push(local_path.to_string_lossy().to_string());
                } else {
                    warn!(file_key, "failed to write feishu file to disk");
                }
            }
            Ok(_) => {
                warn!(file_key, "feishu resource download returned empty bytes");
            }
            Err(e) => {
                warn!(file_key, error = %e, "failed to download feishu message resource");
            }
        }
    }

    paths
}
