use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use garyx_bridge::MultiProviderBridge;
use garyx_models::config::FeishuAccount;
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, PromptAttachment, PromptAttachmentKind, StreamBoundaryKind,
    StreamEvent, attachments_to_metadata_value,
};
use garyx_models::{MessageLedgerEvent, MessageLifecycleStatus, MessageTerminalReason};
use garyx_router::{InboundRequest, MessageRouter};
use prost::Message as ProstMessage;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use super::client::WsConnectInfo;
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
use crate::generated_images::{extract_image_generation_result, write_generated_image_temp};

#[derive(Clone, Copy)]
pub(super) struct FeishuRuntimeContext<'a> {
    pub(super) account_id: &'a str,
    pub(super) router: &'a Arc<Mutex<MessageRouter>>,
    pub(super) bridge: &'a Arc<MultiProviderBridge>,
    pub(super) client: &'a FeishuClient,
    pub(super) account: &'a FeishuAccount,
    pub(super) bot_open_id: &'a str,
}

impl<'a> FeishuRuntimeContext<'a> {
    pub(super) fn new(
        account_id: &'a str,
        router: &'a Arc<Mutex<MessageRouter>>,
        bridge: &'a Arc<MultiProviderBridge>,
        client: &'a FeishuClient,
        account: &'a FeishuAccount,
        bot_open_id: &'a str,
    ) -> Self {
        Self {
            account_id,
            router,
            bridge,
            client,
            account,
            bot_open_id,
        }
    }
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
        let runtime =
            FeishuRuntimeContext::new(account_id, &router, &bridge, client, account, &bot_open_id);

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
                            handle_ws_event_payload(payload, runtime).await;
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
                handle_ws_event_payload(&text, runtime).await;
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
    let from_id = if is_group {
        message.chat_id.clone()
    } else if sender_open_id.is_empty() {
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

    if is_group {
        if requires_group_mention(runtime.account) && !mentioned_bot {
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
    if reply_thread_id.is_none() && !message.parent_id.is_empty() {
        if let Ok(Some(quoted)) = runtime.client.fetch_message_text(&message.parent_id).await {
            route_text = format!("[Replying to: \"{quoted}\"]\n\n{route_text}");
        }
    }

    let mut switched_thread_id_for_notice: Option<String> = None;
    if let Some(reply_thread_id) = reply_thread_id {
        if MessageRouter::is_scheduled_thread(&reply_thread_id) {
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

    // Set up response callback that sends the reply via Feishu API and
    // records the outbound message for reply routing.
    //
    // Reply target: always reply to the user's inbound message, but keep the
    // delivery in the main group chat. Feishu does not currently give us a
    // reliable inbound signal to distinguish a normal reply chain from a user
    // intentionally opening a separate topic.
    let reply_in_thread = false;

    let reply_client = runtime.client.clone();
    let reply_message_id = message.message_id.clone();
    let reply_chat_id = message.chat_id.clone();
    let reply_is_group = is_group;
    let router_cb = runtime.router.clone();
    let account_id_cb = runtime.account_id.to_owned();
    let canonical_thread_id_holder = Arc::new(std::sync::Mutex::new(String::new()));
    let canonical_thread_id_for_stream = canonical_thread_id_holder.clone();
    let stream_state = Arc::new(tokio::sync::Mutex::new(FeishuResponseStreamState {
        processing_reaction_id,
        ..FeishuResponseStreamState::default()
    }));
    let mention_prefix_cb = mention_prefix.clone();

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<StreamEvent>();
    let worker_client = reply_client.clone();
    let worker_msg_id = reply_message_id.clone();
    let worker_reply_in_thread = reply_in_thread;
    let worker_chat_id = reply_chat_id.clone();
    let worker_router = router_cb.clone();
    let worker_account_id = account_id_cb.clone();
    let worker_is_group = reply_is_group;
    let worker_mention_prefix = mention_prefix_cb.clone();
    let worker_native_thread_scope = native_thread_scope.clone();
    let worker_state = stream_state.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let canonical_thread_id = loop {
                let current = match canonical_thread_id_for_stream.lock() {
                    Ok(holder) => holder.clone(),
                    Err(_) => {
                        warn!(
                            account_id = %worker_account_id,
                            "thread id holder mutex poisoned"
                        );
                        String::new()
                    }
                };
                if !current.is_empty() {
                    break current;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            };

            let mut state = worker_state.lock().await;
            let mut text_to_send = String::new();
            let mut send_interactive_fallback = false;
            let mut send_result: Option<Result<String, FeishuError>> = None;
            match event {
                StreamEvent::Boundary { kind, .. } => match kind {
                    StreamBoundaryKind::UserAck => {
                        let boundary_outbound = state.stream_reply_message_id.clone();
                        crate::streaming_core::apply_stream_boundary_text(
                            &mut state.stream_text,
                            StreamBoundaryKind::UserAck,
                        );
                        state.stream_card_id = None;
                        state.stream_card_seq = 0;
                        state.stream_reply_message_id = None;
                        state.last_stream_sent_text.clear();
                        drop(state);

                        if let Some(outbound_msg_id) = boundary_outbound {
                            if !outbound_msg_id.is_empty() && !canonical_thread_id.is_empty() {
                                let mut r = worker_router.lock().await;
                                r.record_outbound_message_with_persistence(
                                    &canonical_thread_id,
                                    "feishu",
                                    &worker_account_id,
                                    &worker_chat_id,
                                    worker_native_thread_scope.as_deref(),
                                    &outbound_msg_id,
                                )
                                .await;
                            }
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
                    if !state.stream_text.is_empty() {
                        let mut reply_text = state.stream_text.clone();
                        if !worker_mention_prefix.is_empty() {
                            reply_text = format!("{worker_mention_prefix} {reply_text}");
                        }
                        reply_text = reply_text.trim().to_owned();

                        if !reply_text.is_empty() && reply_text != state.last_stream_sent_text {
                            if let Some(ref card_id) = state.stream_card_id.clone() {
                                // Update Card Kit streaming card element
                                state.stream_card_seq += 1;
                                let seq = state.stream_card_seq;
                                match worker_client
                                    .update_cardkit_element(card_id, "content", &reply_text, seq)
                                    .await
                                {
                                    Ok(()) => {
                                        state.last_stream_sent_text = reply_text;
                                    }
                                    Err(err) => {
                                        debug!(
                                            account_id = %worker_account_id,
                                            message_id = %worker_msg_id,
                                            error = %err,
                                            "Feishu Card Kit element update failed"
                                        );
                                    }
                                }
                            } else {
                                // First chunk: create Card Kit card, send reference message
                                match worker_client.create_cardkit_card(&reply_text).await {
                                    Ok(card_id) => {
                                        let card_ref =
                                            FeishuClient::card_reference_content(&card_id);
                                        debug!(
                                            account_id = %worker_account_id,
                                            reply_to = %worker_msg_id,
                                            is_group = worker_is_group,
                                            "Feishu streaming card: sending initial reply"
                                        );
                                        let msg_result = if worker_is_group {
                                            worker_client
                                                .reply_message_ext(
                                                    &worker_msg_id,
                                                    &card_ref,
                                                    "interactive",
                                                    worker_reply_in_thread,
                                                )
                                                .await
                                        } else {
                                            worker_client
                                                .send_message(
                                                    &worker_chat_id,
                                                    &card_ref,
                                                    "interactive",
                                                )
                                                .await
                                        };
                                        match msg_result {
                                            Ok(outbound_msg_id) => {
                                                state.stream_card_id = Some(card_id);
                                                state.stream_card_seq = 1;
                                                if !outbound_msg_id.is_empty() {
                                                    state.stream_reply_message_id =
                                                        Some(outbound_msg_id);
                                                }
                                                state.last_stream_sent_text = reply_text;
                                            }
                                            Err(err) => {
                                                debug!(
                                                    account_id = %worker_account_id,
                                                    message_id = %worker_msg_id,
                                                    error = %err,
                                                    "Feishu streaming card send failed"
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        debug!(
                                            account_id = %worker_account_id,
                                            message_id = %worker_msg_id,
                                            error = %err,
                                            "Feishu Card Kit create failed"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                StreamEvent::ToolUse { .. } => {
                    continue;
                }
                StreamEvent::ToolResult { message } => {
                    let Some(image) = extract_image_generation_result(&message) else {
                        continue;
                    };
                    drop(state);

                    let image_path = match write_generated_image_temp("feishu", &image).await {
                        Ok(path) => path,
                        Err(err) => {
                            warn!(
                                account_id = %worker_account_id,
                                error = %err,
                                "failed to write Feishu generated image temp file"
                            );
                            continue;
                        }
                    };

                    let image_send_result = if worker_is_group {
                        worker_client
                            .reply_image_ext(&worker_msg_id, &image_path, worker_reply_in_thread)
                            .await
                    } else {
                        worker_client.send_image(&worker_chat_id, &image_path).await
                    };
                    let _ = tokio::fs::remove_file(&image_path).await;

                    match image_send_result {
                        Ok(outbound_msg_id) => {
                            info!(
                                account_id = %worker_account_id,
                                outbound_message_id = %outbound_msg_id,
                                "Feishu generated image sent"
                            );
                            if !outbound_msg_id.is_empty() && !canonical_thread_id.is_empty() {
                                let mut r = worker_router.lock().await;
                                r.record_outbound_message_with_persistence(
                                    &canonical_thread_id,
                                    "feishu",
                                    &worker_account_id,
                                    &worker_chat_id,
                                    worker_native_thread_scope.as_deref(),
                                    &outbound_msg_id,
                                )
                                .await;
                            } else if canonical_thread_id.is_empty() {
                                warn!(
                                    account_id = %worker_account_id,
                                    outbound_message_id = %outbound_msg_id,
                                    "Feishu generated image not indexed: thread key unavailable"
                                );
                            }
                        }
                        Err(e) => {
                            error!(
                                account_id = %worker_account_id,
                                error = %e,
                                "Failed to send Feishu generated image"
                            );
                            notify_permission_error_if_needed(
                                &worker_client,
                                &worker_account_id,
                                &worker_chat_id,
                                &worker_msg_id,
                                &e.to_string(),
                            )
                            .await;
                        }
                    }
                    continue;
                }
                StreamEvent::Done => {}
            }

            let mut final_text = state.stream_text.clone();
            state.stream_text.clear();

            if !worker_mention_prefix.is_empty() {
                final_text = format!("{worker_mention_prefix} {final_text}");
            }
            final_text = final_text.trim().to_owned();
            if !final_text.is_empty() {
                text_to_send = final_text;
            }

            if !text_to_send.is_empty() {
                if let Some(card_id) = state.stream_card_id.clone() {
                    // Close Card Kit streaming with final text
                    state.stream_card_seq += 1;
                    let seq = state.stream_card_seq;
                    match worker_client
                        .close_cardkit_streaming(&card_id, &text_to_send, seq)
                        .await
                    {
                        Ok(()) => {
                            state.last_stream_sent_text = text_to_send.clone();
                            if let Some(ref msg_id) = state.stream_reply_message_id {
                                send_result = Some(Ok(msg_id.clone()));
                            }
                        }
                        Err(err) => {
                            error!(
                                account_id = %worker_account_id,
                                message_id = %worker_msg_id,
                                error = %err,
                                "Feishu Card Kit close streaming failed"
                            );
                            send_interactive_fallback = true;
                        }
                    }
                } else {
                    send_interactive_fallback = true;
                }
            }

            if send_interactive_fallback && !text_to_send.is_empty() {
                let content = build_card_content(&text_to_send);
                debug!(
                    account_id = %worker_account_id,
                    reply_to = %worker_msg_id,
                    is_group = worker_is_group,
                    "Feishu fallback card: sending reply"
                );
                send_result = Some(if worker_is_group {
                    worker_client
                        .reply_message_ext(
                            &worker_msg_id,
                            &content,
                            "interactive",
                            worker_reply_in_thread,
                        )
                        .await
                } else {
                    worker_client
                        .send_message(&worker_chat_id, &content, "interactive")
                        .await
                });
            }

            if !state.processing_reaction_removed {
                if let Some(reaction_id) = state.processing_reaction_id.take() {
                    if let Err(err) = worker_client
                        .remove_reaction(&worker_msg_id, &reaction_id)
                        .await
                    {
                        debug!(
                            account_id = %worker_account_id,
                            message_id = %worker_msg_id,
                            reaction_id = %reaction_id,
                            error = %err,
                            "Feishu remove processing reaction skipped"
                        );
                    }
                }
                state.processing_reaction_removed = true;
            }

            drop(state);

            if let Some(send_result) = send_result {
                match send_result {
                    Ok(outbound_msg_id) => {
                        info!(
                            account_id = %worker_account_id,
                            outbound_message_id = %outbound_msg_id,
                            "Feishu reply sent"
                        );
                        if !outbound_msg_id.is_empty() && !canonical_thread_id.is_empty() {
                            let mut r = worker_router.lock().await;
                            r.record_outbound_message_with_persistence(
                                &canonical_thread_id,
                                "feishu",
                                &worker_account_id,
                                &worker_chat_id,
                                worker_native_thread_scope.as_deref(),
                                &outbound_msg_id,
                            )
                            .await;
                        } else if canonical_thread_id.is_empty() {
                            warn!(
                                account_id = %worker_account_id,
                                outbound_message_id = %outbound_msg_id,
                                "Feishu outbound message not indexed: thread key unavailable"
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            account_id = %worker_account_id,
                            error = %e,
                            "Failed to send Feishu reply"
                        );
                        notify_permission_error_if_needed(
                            &worker_client,
                            &worker_account_id,
                            &worker_chat_id,
                            &worker_msg_id,
                            &e.to_string(),
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
            .route_and_dispatch(request, runtime.bridge.as_ref(), Some(response_callback))
            .await
    };

    match dispatch_result {
        Ok(result) => {
            if let Ok(mut holder) = canonical_thread_id_holder.lock() {
                *holder = result.thread_id.clone();
            } else {
                warn!(
                    account_id = %runtime.account_id,
                    "thread id holder mutex poisoned"
                );
            }
            if let Some(local_reply) = result.local_reply {
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
                if let Some((cached_name, expires_at)) = cache.get(sender_open_id) {
                    if *expires_at > now {
                        return Some(cached_name.clone());
                    }
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
