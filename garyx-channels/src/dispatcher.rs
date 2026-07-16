//! ChannelDispatcher — outbound message delivery to channels.
//!
//! Allows any component (MCP tools, cron jobs, API endpoints) to send
//! messages OUT through channel transports (Telegram, Feishu) without needing
//! direct access to channel internals.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use garyx_models::ChannelOutboundContent;
use garyx_models::provider::{StreamBoundaryKind, StreamEvent};
use garyx_models::routing::{infer_delivery_target_id, infer_delivery_target_type};
use regex::Regex;
use reqwest::{Client, StatusCode, header, multipart};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use garyx_models::config::{ChannelsConfig, FeishuDomain};

use crate::channel_trait::ChannelError;
use crate::plugin_host::{DispatchOutbound, DispatchStreamEvent, PluginSenderHandle};
use crate::weixin;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An outbound message to be delivered through a channel.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    /// Channel type: "telegram" or "feishu".
    pub channel: String,
    /// Which bot account within the channel.
    pub account_id: String,
    /// Target chat/conversation ID.
    pub chat_id: String,
    /// Channel-specific delivery target type. Defaults to `chat_id`.
    pub delivery_target_type: String,
    /// Channel-specific delivery target value. Falls back to `chat_id`.
    pub delivery_target_id: String,
    /// Structured channel-facing content.
    pub content: ChannelOutboundContent,
    /// Optional message ID to reply to.
    pub reply_to: Option<String>,
    /// Optional thread/topic ID (Telegram forum topics, Feishu threads).
    pub thread_id: Option<String>,
}

/// Result for outbound message delivery.
#[derive(Debug, Clone, Default)]
pub struct SendMessageResult {
    /// Platform-specific outbound message ids.
    pub message_ids: Vec<String>,
}

/// Summary info about an available channel account.
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub channel: String,
    pub account_id: String,
    pub is_running: bool,
}

#[derive(Debug, Clone)]
pub struct StreamingDispatchTarget {
    pub target_thread_id: String,
    pub endpoint_identity: String,
    pub run_id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StreamDispatchEnvelope {
    pub account_id: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    pub endpoint_identity: String,
    pub thread_id: String,
    pub run_id: String,
    pub event: StreamEvent,
    pub delivery_thread_id: Option<String>,
}

impl StreamDispatchEnvelope {
    pub fn from_target(target: &StreamingDispatchTarget, event: StreamEvent) -> Self {
        Self {
            account_id: target.account_id.clone(),
            chat_id: target.chat_id.clone(),
            delivery_target_type: target.delivery_target_type.clone(),
            delivery_target_id: target.delivery_target_id.clone(),
            endpoint_identity: target.endpoint_identity.clone(),
            thread_id: target.target_thread_id.clone(),
            run_id: target.run_id.clone(),
            event,
            delivery_thread_id: target.thread_id.clone(),
        }
    }
}

impl From<StreamDispatchEnvelope> for DispatchStreamEvent {
    fn from(envelope: StreamDispatchEnvelope) -> Self {
        Self {
            account_id: envelope.account_id,
            chat_id: envelope.chat_id,
            delivery_target_type: envelope.delivery_target_type,
            delivery_target_id: envelope.delivery_target_id,
            endpoint_identity: envelope.endpoint_identity,
            thread_id: envelope.thread_id,
            run_id: envelope.run_id,
            event: envelope.event.into(),
            delivery_thread_id: envelope.delivery_thread_id,
        }
    }
}

pub type StreamDispatchCallback = Arc<dyn Fn(StreamDispatchEnvelope) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDispatchRole {
    Origin,
    BoundTarget,
}

impl StreamDispatchRole {
    fn legacy_adapter_flushes_user_ack(self) -> bool {
        matches!(self, Self::Origin)
    }

    fn dispatches_user_ack(self) -> bool {
        matches!(self, Self::Origin)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuChatSummary {
    pub name: Option<String>,
    pub chat_mode: Option<String>,
    pub chat_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Outbound message delivery to channels.
#[async_trait]
pub trait ChannelDispatcher: Send + Sync {
    /// Send a text message to a specific channel/account/chat.
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError>;

    /// List available channels and their status.
    fn available_channels(&self) -> Vec<ChannelInfo>;

    fn build_stream_event_callback(
        &self,
        _target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        None
    }

    fn supports_legacy_stream_adapter(&self, _target: &StreamingDispatchTarget) -> bool {
        false
    }

    fn channel_running_handle(&self, _channel: &str) -> Option<Arc<AtomicBool>> {
        None
    }
}

#[derive(Default)]
struct OutboundStreamState {
    pending_text: String,
}

impl OutboundStreamState {
    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.pending_text = crate::streaming_core::merge_stream_text(&self.pending_text, text);
    }

    fn take_text(&mut self) -> Option<ChannelOutboundContent> {
        let text = std::mem::take(&mut self.pending_text);
        (!text.trim().is_empty()).then(|| ChannelOutboundContent::text(text))
    }
}

struct OutboundStreamCallbackShared {
    dispatcher: Arc<dyn ChannelDispatcher>,
    target: StreamingDispatchTarget,
    state: std::sync::Mutex<OutboundStreamState>,
    delivery_gate: Mutex<()>,
    flush_user_ack_boundary: bool,
}

struct PluginStreamEventCallbackShared {
    sender: PluginSenderHandle,
    target: StreamingDispatchTarget,
    delivery_gate: Mutex<()>,
}

impl PluginStreamEventCallbackShared {
    async fn dispatch_event(&self, envelope: StreamDispatchEnvelope) {
        let _guard = self.delivery_gate.lock().await;
        let request = DispatchStreamEvent::from(envelope);

        match self.sender.dispatch_stream_event(request).await {
            Ok(_) => {}
            Err(error) => {
                warn!(
                    plugin_id = %self.sender.plugin_id(),
                    account_id = %self.target.account_id,
                    chat_id = %self.target.chat_id,
                    endpoint_identity = %self.target.endpoint_identity,
                    target_thread_id = %self.target.target_thread_id,
                    run_id = %self.target.run_id,
                    error = %error,
                    "plugin dispatch_stream_event failed"
                );
            }
        }
    }
}

fn build_plugin_stream_event_callback(
    sender: PluginSenderHandle,
    target: StreamingDispatchTarget,
) -> StreamDispatchCallback {
    let shared = Arc::new(PluginStreamEventCallbackShared {
        sender,
        target,
        delivery_gate: Mutex::new(()),
    });

    Arc::new(move |envelope| {
        let shared = shared.clone();
        tokio::spawn(async move {
            shared.dispatch_event(envelope).await;
        });
    })
}

impl OutboundStreamCallbackShared {
    fn contents_for_event(&self, event: StreamEvent) -> Vec<ChannelOutboundContent> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => {
                warn!("outbound stream callback state lock poisoned");
                return Vec::new();
            }
        };

        match event {
            StreamEvent::SessionBound { .. } | StreamEvent::ThreadTitleUpdated { .. } => {
                debug!(
                    channel = %self.target.channel,
                    account_id = %self.target.account_id,
                    endpoint_identity = %self.target.endpoint_identity,
                    target_thread_id = %self.target.target_thread_id,
                    "legacy outbound stream adapter deliberately ignored metadata event"
                );
                Vec::new()
            }
            StreamEvent::Delta { text } => {
                state.push_text(&text);
                Vec::new()
            }
            StreamEvent::Boundary { kind, .. } => match kind {
                StreamBoundaryKind::AssistantSegment => state.take_text().into_iter().collect(),
                StreamBoundaryKind::UserAck => {
                    if self.flush_user_ack_boundary {
                        state.take_text().into_iter().collect()
                    } else {
                        debug!(
                            channel = %self.target.channel,
                            account_id = %self.target.account_id,
                            endpoint_identity = %self.target.endpoint_identity,
                            target_thread_id = %self.target.target_thread_id,
                            "legacy outbound stream adapter ignored non-origin user_ack boundary"
                        );
                        Vec::new()
                    }
                }
            },
            StreamEvent::Done => state.take_text().into_iter().collect(),
            StreamEvent::ToolUse { message } => {
                let mut contents: Vec<_> = state.take_text().into_iter().collect();
                contents.push(ChannelOutboundContent::ToolUse { message });
                contents
            }
            StreamEvent::ToolResult { message } => {
                let mut contents: Vec<_> = state.take_text().into_iter().collect();
                contents.push(ChannelOutboundContent::ToolResult { message });
                contents
            }
        }
    }

    async fn dispatch_contents(self: Arc<Self>, contents: Vec<ChannelOutboundContent>) {
        let _guard = self.delivery_gate.lock().await;
        for content in contents {
            self.dispatch_content(content).await;
        }
    }

    async fn dispatch_content(&self, content: ChannelOutboundContent) {
        let request = OutboundMessage {
            channel: self.target.channel.clone(),
            account_id: self.target.account_id.clone(),
            chat_id: self.target.chat_id.clone(),
            delivery_target_type: self.target.delivery_target_type.clone(),
            delivery_target_id: self.target.delivery_target_id.clone(),
            content,
            reply_to: None,
            thread_id: self.target.thread_id.clone(),
        };

        match self.dispatcher.send_message(request).await {
            Ok(_) => {}
            Err(error) => {
                warn!(
                    channel = %self.target.channel,
                    account_id = %self.target.account_id,
                    chat_id = %self.target.chat_id,
                    target_thread_id = %self.target.target_thread_id,
                    error = %error,
                    "outbound stream callback failed to send channel message"
                );
            }
        }
    }
}

/// Build a generic channel-layer consumer for committed provider stream events.
///
/// Legacy subprocess plugins can still receive best-effort structured outbound
/// content through this adapter while they migrate to native stream-event
/// dispatch. Native channels and new plugins should use
/// [`build_stream_dispatch_callback`] instead.
pub fn build_outbound_stream_callback(
    dispatcher: Arc<dyn ChannelDispatcher>,
    target: StreamingDispatchTarget,
    role: StreamDispatchRole,
) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let shared = Arc::new(OutboundStreamCallbackShared {
        dispatcher,
        target,
        state: std::sync::Mutex::new(OutboundStreamState::default()),
        delivery_gate: Mutex::new(()),
        flush_user_ack_boundary: role.legacy_adapter_flushes_user_ack(),
    });

    Arc::new(move |event| {
        let contents = shared.contents_for_event(event);
        if contents.is_empty() {
            return;
        }
        let shared = shared.clone();
        tokio::spawn(async move {
            shared.dispatch_contents(contents).await;
        });
    })
}

pub fn build_stream_dispatch_callback(
    dispatcher: Arc<dyn ChannelDispatcher>,
    target: StreamingDispatchTarget,
    role: StreamDispatchRole,
) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
    if let Some(callback) = dispatcher.build_stream_event_callback(target.clone()) {
        return Some(Arc::new(move |event| {
            if matches!(
                event,
                StreamEvent::Boundary {
                    kind: StreamBoundaryKind::UserAck,
                    ..
                }
            ) && !role.dispatches_user_ack()
            {
                debug!(
                    channel = %target.channel,
                    account_id = %target.account_id,
                    endpoint_identity = %target.endpoint_identity,
                    target_thread_id = %target.target_thread_id,
                    "stream event fanout ignored non-origin user_ack boundary"
                );
                return;
            }
            callback(StreamDispatchEnvelope::from_target(&target, event));
        }));
    }

    if !dispatcher.supports_legacy_stream_adapter(&target) {
        return None;
    }

    Some(build_outbound_stream_callback(dispatcher, target, role))
}

// ---------------------------------------------------------------------------
// Channel-blind outbound sender trait
// ---------------------------------------------------------------------------

/// A clone-cheap, per-account outbound sender. Every built-in
/// channel's sender handle (`TelegramSender`, `FeishuSender`,
/// `WeixinSender`) implements this so the dispatcher's per-channel
/// sender maps can delegate uniformly.
///
/// The trait's `send_outbound` method owns every channel-specific
/// wire quirk: Telegram's integer id parsing, Feishu's reply-target
/// resolution, Weixin's `context_token` retry + queue-on-failure.
#[async_trait]
pub trait OutboundSender: Send + Sync {
    /// Send one outbound message. `request.account_id` is
    /// redundant when the caller already picked this sender out of
    /// a per-account map — kept in the struct for logging /
    /// symmetry with the subprocess `DispatchOutbound` RPC shape.
    async fn send_outbound(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError>;
}

// ---------------------------------------------------------------------------
// Telegram sender handle
// ---------------------------------------------------------------------------

/// A clonable handle that can send Telegram messages without owning the channel.
#[derive(Clone)]
pub struct TelegramSender {
    pub account_id: String,
    pub token: String,
    pub http: Client,
    pub api_base: String,
    pub is_running: bool,
}

#[async_trait]
impl OutboundSender for TelegramSender {
    async fn send_outbound(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        let chat_id = parse_telegram_id("chat_id", &request.chat_id)?;
        let reply_to = parse_optional_telegram_id("reply_to", request.reply_to.as_deref())?;
        // `thread_id` may carry a Garyx-internal thread key or a
        // legacy private-chat binding. Only a real numeric topic id
        // distinct from `chat_id` is a valid Telegram thread.
        let thread_id = normalize_telegram_thread_id(chat_id, request.thread_id.as_deref());
        let message_ids = if let Some(text) = request.text_content() {
            let (text, image_refs) = extract_telegram_markdown_image_refs(text);
            if image_refs.is_empty() {
                self.send_text(chat_id, text.as_str(), reply_to, thread_id)
                    .await?
            } else {
                self.send_text_with_markdown_images(
                    chat_id,
                    text.as_str(),
                    &image_refs,
                    reply_to,
                    thread_id,
                )
                .await?
            }
        } else if let Some((image_path, _alt)) = request.image_content() {
            self.send_image(chat_id, Path::new(image_path), None, reply_to, thread_id)
                .await?
        } else if let Some((file_path, caption)) = request.file_content() {
            self.send_file(chat_id, Path::new(file_path), caption, reply_to, thread_id)
                .await?
        } else {
            return Ok(SendMessageResult::default());
        };
        Ok(SendMessageResult {
            message_ids: message_ids.into_iter().map(|id| id.to_string()).collect(),
        })
    }
}

impl TelegramSender {
    /// Send a text message via the Telegram Bot API.
    pub async fn send_text(
        &self,
        chat_id: i64,
        text: &str,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<Vec<i64>, ChannelError> {
        crate::telegram::send_response(
            crate::telegram::TelegramSendTarget::new(
                &self.http,
                &self.token,
                chat_id,
                message_thread_id,
                &self.api_base,
            ),
            text,
            reply_to_message_id,
        )
        .await
    }

    pub async fn send_image(
        &self,
        chat_id: i64,
        image_path: &Path,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<Vec<i64>, ChannelError> {
        let message_id = crate::telegram::send_photo(
            crate::telegram::TelegramSendTarget::new(
                &self.http,
                &self.token,
                chat_id,
                message_thread_id,
                &self.api_base,
            ),
            image_path,
            caption,
            reply_to_message_id,
        )
        .await?;
        Ok(vec![message_id])
    }

    pub async fn send_file(
        &self,
        chat_id: i64,
        file_path: &Path,
        caption: Option<&str>,
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<Vec<i64>, ChannelError> {
        let message_id = crate::telegram::send_document(
            crate::telegram::TelegramSendTarget::new(
                &self.http,
                &self.token,
                chat_id,
                message_thread_id,
                &self.api_base,
            ),
            file_path,
            caption,
            reply_to_message_id,
        )
        .await?;
        Ok(vec![message_id])
    }

    async fn send_text_with_markdown_images(
        &self,
        chat_id: i64,
        text: &str,
        image_refs: &[TelegramMarkdownImageRef],
        reply_to_message_id: Option<i64>,
        message_thread_id: Option<i64>,
    ) -> Result<Vec<i64>, ChannelError> {
        let mut message_ids = Vec::new();
        let mut reply_to_next = reply_to_message_id;

        if !text.trim().is_empty() {
            message_ids.extend(
                self.send_text(chat_id, text, reply_to_next, message_thread_id)
                    .await?,
            );
            reply_to_next = None;
        }

        for image_ref in image_refs {
            message_ids.extend(
                self.send_image(
                    chat_id,
                    Path::new(&image_ref.path),
                    image_ref.caption.as_deref(),
                    reply_to_next,
                    message_thread_id,
                )
                .await?,
            );
            reply_to_next = None;
        }

        Ok(message_ids)
    }
}

// ---------------------------------------------------------------------------
// Discord sender handle
// ---------------------------------------------------------------------------

pub(crate) const DISCORD_MAX_MESSAGE_LENGTH: usize = 2000;
const DISCORD_REQUEST_MAX_RETRIES: usize = 5;
const DISCORD_RETRY_DEFAULT_DELAY: Duration = Duration::from_secs(1);
const DISCORD_RETRY_MAX_DELAY: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct DiscordApiError {
    status: StatusCode,
    code: Option<i64>,
    message: String,
    retry_after: Option<Duration>,
    global: bool,
    scope: Option<String>,
}

impl DiscordApiError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: None,
            message: message.into(),
            retry_after: None,
            global: false,
            scope: None,
        }
    }

    fn is_reply_reference_rejection(&self) -> bool {
        self.code == Some(10008)
            || (self.code == Some(50035)
                && (self.message.contains("Cannot reply to a system message")
                    || self.message.contains("message_reference")))
    }

    fn is_rate_limited(&self) -> bool {
        self.status == StatusCode::TOO_MANY_REQUESTS
    }

    fn is_transient(&self) -> bool {
        self.is_rate_limited() || self.status.is_server_error()
    }

    fn retry_delay(&self, attempt: usize) -> Duration {
        if self.is_rate_limited() {
            return self
                .retry_after
                .unwrap_or(DISCORD_RETRY_DEFAULT_DELAY)
                .min(DISCORD_RETRY_MAX_DELAY);
        }

        let multiplier = 1_u32.checked_shl(attempt.min(5) as u32).unwrap_or(32);
        DISCORD_RETRY_DEFAULT_DELAY
            .saturating_mul(multiplier)
            .min(DISCORD_RETRY_MAX_DELAY)
    }
}

/// A clonable handle that can send Discord messages without owning the channel.
#[derive(Clone)]
pub struct DiscordSender {
    pub account_id: String,
    pub token: String,
    pub http: Client,
    pub api_base: String,
    pub is_running: bool,
}

#[async_trait]
impl OutboundSender for DiscordSender {
    async fn send_outbound(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        let target_id = discord_target_channel_id(&request);
        let message_ids = if let Some(text) = request.text_content() {
            self.send_text(&target_id, text, request.reply_to.as_deref())
                .await?
        } else if let Some((image_path, alt)) = request.image_content() {
            self.send_file(
                &target_id,
                Path::new(image_path),
                alt,
                request.reply_to.as_deref(),
            )
            .await?
        } else if let Some((file_path, caption)) = request.file_content() {
            self.send_file(
                &target_id,
                Path::new(file_path),
                caption,
                request.reply_to.as_deref(),
            )
            .await?
        } else {
            return Ok(SendMessageResult::default());
        };
        Ok(SendMessageResult { message_ids })
    }
}

impl DiscordSender {
    pub async fn send_text(
        &self,
        channel_id: &str,
        text: &str,
        reply_to_message_id: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let chunks = split_discord_message(text);
        let mut message_ids = Vec::new();
        let mut reply_to = reply_to_message_id;
        for chunk in chunks {
            match self.post_message_json(channel_id, &chunk, reply_to).await {
                Ok(message_id) => message_ids.push(message_id),
                Err(error) if reply_to.is_some() && error.is_reply_reference_rejection() => {
                    warn!(
                        account_id = %self.account_id,
                        channel_id,
                        status = %error.status,
                        code = ?error.code,
                        "Discord rejected reply reference; retrying without reference"
                    );
                    message_ids.push(
                        self.post_message_json(channel_id, &chunk, None)
                            .await
                            .map_err(discord_send_error)?,
                    );
                }
                Err(error) => return Err(discord_send_error(error)),
            }
            reply_to = None;
        }
        Ok(message_ids)
    }

    pub async fn send_file(
        &self,
        channel_id: &str,
        path: &Path,
        caption: Option<&str>,
        reply_to_message_id: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        match self
            .post_message_file(channel_id, path, caption, reply_to_message_id)
            .await
        {
            Ok(message_id) => Ok(vec![message_id]),
            Err(error) if reply_to_message_id.is_some() && error.is_reply_reference_rejection() => {
                warn!(
                    account_id = %self.account_id,
                    channel_id,
                    status = %error.status,
                    code = ?error.code,
                    "Discord rejected file reply reference; retrying without reference"
                );
                self.post_message_file(channel_id, path, caption, None)
                    .await
                    .map(|message_id| vec![message_id])
                    .map_err(discord_send_error)
            }
            Err(error) => Err(discord_send_error(error)),
        }
    }

    pub async fn edit_text(
        &self,
        channel_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<String, ChannelError> {
        self.patch_message_json(channel_id, message_id, text)
            .await
            .map_err(discord_send_error)
    }

    pub async fn delete_text(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), ChannelError> {
        self.delete_message(channel_id, message_id)
            .await
            .map_err(discord_send_error)
    }

    async fn post_message_json(
        &self,
        channel_id: &str,
        content: &str,
        reply_to_message_id: Option<&str>,
    ) -> Result<String, DiscordApiError> {
        let body = discord_message_payload(content, channel_id, reply_to_message_id, None);
        let mut attempt = 0;
        loop {
            let response = match self
                .http
                .post(discord_channel_messages_url(&self.api_base, channel_id))
                .header("Authorization", format!("Bot {}", self.token))
                .json(&body)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let error = DiscordApiError::internal(error.to_string());
                    if self
                        .sleep_before_discord_request_retry("create message", attempt, &error)
                        .await
                    {
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            };
            match parse_discord_message_response(response).await {
                Err(error)
                    if self
                        .sleep_before_discord_request_retry("create message", attempt, &error)
                        .await =>
                {
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn post_message_file(
        &self,
        channel_id: &str,
        path: &Path,
        caption: Option<&str>,
        reply_to_message_id: Option<&str>,
    ) -> Result<String, DiscordApiError> {
        let bytes = tokio::fs::read(path).await.map_err(|error| {
            DiscordApiError::internal(format!(
                "failed to read attachment '{}': {error}",
                path.display()
            ))
        })?;
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("file.bin")
            .to_owned();
        let body = discord_message_payload(
            caption.unwrap_or_default(),
            channel_id,
            reply_to_message_id,
            Some(&filename),
        );
        let payload_json = serde_json::to_string(&body).map_err(|error| {
            DiscordApiError::internal(format!(
                "failed to encode Discord multipart payload: {error}"
            ))
        })?;
        let mut attempt = 0;
        loop {
            let part = multipart::Part::bytes(bytes.clone()).file_name(filename.clone());
            let form = multipart::Form::new()
                .text("payload_json", payload_json.clone())
                .part("files[0]", part);
            let response = match self
                .http
                .post(discord_channel_messages_url(&self.api_base, channel_id))
                .header("Authorization", format!("Bot {}", self.token))
                .multipart(form)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let error = DiscordApiError::internal(error.to_string());
                    if self
                        .sleep_before_discord_request_retry("create file message", attempt, &error)
                        .await
                    {
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            };
            match parse_discord_message_response(response).await {
                Err(error)
                    if self
                        .sleep_before_discord_request_retry("create file message", attempt, &error)
                        .await =>
                {
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn patch_message_json(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<String, DiscordApiError> {
        let body = discord_edit_message_payload(content);
        let mut attempt = 0;
        loop {
            let response = match self
                .http
                .patch(discord_message_url(&self.api_base, channel_id, message_id))
                .header("Authorization", format!("Bot {}", self.token))
                .json(&body)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let error = DiscordApiError::internal(error.to_string());
                    if self
                        .sleep_before_discord_request_retry("edit message", attempt, &error)
                        .await
                    {
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            };
            match parse_discord_message_response(response).await {
                Err(error)
                    if self
                        .sleep_before_discord_request_retry("edit message", attempt, &error)
                        .await =>
                {
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn delete_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), DiscordApiError> {
        let mut attempt = 0;
        loop {
            let response = match self
                .http
                .delete(discord_message_url(&self.api_base, channel_id, message_id))
                .header("Authorization", format!("Bot {}", self.token))
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let error = DiscordApiError::internal(error.to_string());
                    if self
                        .sleep_before_discord_request_retry("delete message", attempt, &error)
                        .await
                    {
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            };
            match parse_discord_empty_response(response).await {
                Err(error)
                    if self
                        .sleep_before_discord_request_retry("delete message", attempt, &error)
                        .await =>
                {
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn sleep_before_discord_request_retry(
        &self,
        operation: &str,
        attempt: usize,
        error: &DiscordApiError,
    ) -> bool {
        if !error.is_transient() || attempt >= DISCORD_REQUEST_MAX_RETRIES {
            return false;
        }
        let delay = error.retry_delay(attempt);
        warn!(
            account_id = %self.account_id,
            operation,
            status = %error.status,
            code = ?error.code,
            retry_after_ms = delay.as_millis(),
            attempt = attempt + 1,
            max_retries = DISCORD_REQUEST_MAX_RETRIES,
            global = error.global,
            scope = error.scope.as_deref().unwrap_or(""),
            "Discord request failed transiently; retrying after delay"
        );
        tokio::time::sleep(delay).await;
        true
    }
}

fn discord_target_channel_id(request: &OutboundMessage) -> String {
    if let Some(thread_id) = request
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return thread_id.to_owned();
    }
    let target = request.resolved_delivery_target_id();
    let target = target.trim();
    if target.is_empty() {
        request.chat_id.trim().to_owned()
    } else {
        target.to_owned()
    }
}

fn discord_channel_messages_url(api_base: &str, channel_id: &str) -> String {
    format!(
        "{}/channels/{}/messages",
        api_base.trim_end_matches('/'),
        channel_id
    )
}

fn discord_message_url(api_base: &str, channel_id: &str, message_id: &str) -> String {
    format!(
        "{}/channels/{}/messages/{}",
        api_base.trim_end_matches('/'),
        channel_id,
        message_id
    )
}

fn discord_message_payload(
    content: &str,
    channel_id: &str,
    reply_to_message_id: Option<&str>,
    attachment_filename: Option<&str>,
) -> Value {
    let mut body = serde_json::json!({
        "content": content,
        "allowed_mentions": {
            "parse": ["users"],
            "replied_user": true
        }
    });
    if let Some(reply_to) = reply_to_message_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["message_reference"] = serde_json::json!({
            "message_id": reply_to,
            "channel_id": channel_id,
            "fail_if_not_exists": false
        });
    }
    if let Some(filename) = attachment_filename {
        body["attachments"] = serde_json::json!([{
            "id": 0,
            "filename": filename
        }]);
    }
    body
}

fn discord_edit_message_payload(content: &str) -> Value {
    serde_json::json!({
        "content": content,
        "allowed_mentions": {
            "parse": ["users"],
            "replied_user": true
        }
    })
}

pub(crate) fn split_discord_message(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.len() + ch.len_utf8() > DISCORD_MAX_MESSAGE_LENGTH {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

async fn parse_discord_message_response(
    response: reqwest::Response,
) -> Result<String, DiscordApiError> {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.bytes().await.map_err(|error| DiscordApiError {
        status,
        code: None,
        message: error.to_string(),
        retry_after: None,
        global: false,
        scope: None,
    })?;
    let payload: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()));
    if !status.is_success() {
        return Err(DiscordApiError {
            status,
            code: payload.get("code").and_then(Value::as_i64),
            message: payload
                .get("message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| payload.to_string()),
            retry_after: discord_retry_after(&headers, &payload),
            global: discord_rate_limit_global(&headers, &payload),
            scope: discord_rate_limit_scope(&headers),
        });
    }
    payload
        .get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| DiscordApiError {
            status,
            code: None,
            message: "Discord create message response did not include id".to_owned(),
            retry_after: None,
            global: false,
            scope: None,
        })
}

async fn parse_discord_empty_response(response: reqwest::Response) -> Result<(), DiscordApiError> {
    let status = response.status();
    let headers = response.headers().clone();
    if status.is_success() {
        return Ok(());
    }

    let bytes = response.bytes().await.map_err(|error| DiscordApiError {
        status,
        code: None,
        message: error.to_string(),
        retry_after: None,
        global: false,
        scope: None,
    })?;
    let payload: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()));
    Err(DiscordApiError {
        status,
        code: payload.get("code").and_then(Value::as_i64),
        message: payload
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| payload.to_string()),
        retry_after: discord_retry_after(&headers, &payload),
        global: discord_rate_limit_global(&headers, &payload),
        scope: discord_rate_limit_scope(&headers),
    })
}

fn discord_retry_after(headers: &header::HeaderMap, payload: &Value) -> Option<Duration> {
    payload
        .get("retry_after")
        .and_then(Value::as_f64)
        .or_else(|| {
            headers
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<f64>().ok())
        })
        .or_else(|| {
            headers
                .get("x-ratelimit-reset-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<f64>().ok())
        })
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(Duration::from_secs_f64)
}

fn discord_rate_limit_global(headers: &header::HeaderMap, payload: &Value) -> bool {
    payload
        .get("global")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            headers
                .get("x-ratelimit-global")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        })
}

fn discord_rate_limit_scope(headers: &header::HeaderMap) -> Option<String> {
    headers
        .get("x-ratelimit-scope")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn discord_send_error(error: DiscordApiError) -> ChannelError {
    ChannelError::SendFailed(format!(
        "Discord API HTTP {}{}: {}",
        error.status,
        error
            .code
            .map(|code| format!(" (code={code})"))
            .unwrap_or_default(),
        error.message
    ))
}

// ---------------------------------------------------------------------------
// Feishu sender handle
// ---------------------------------------------------------------------------

/// A clonable handle that can send Feishu messages without owning the channel.
#[derive(Clone)]
pub struct FeishuSender {
    pub account_id: String,
    pub app_id: String,
    pub app_secret: String,
    pub api_base: String,
    pub is_running: bool,
    http: Client,
    /// Token and its expiry stored atomically to prevent inconsistent reads.
    token_state: Arc<RwLock<Option<(String, tokio::time::Instant)>>>,
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
}

#[async_trait]
impl OutboundSender for FeishuSender {
    async fn send_outbound(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        let reply_target =
            resolve_feishu_reply_target(request.reply_to.as_deref(), request.thread_id.as_deref());
        let delivery_target_type = request.resolved_delivery_target_type();
        let delivery_target_id = request.resolved_delivery_target_id();
        let message_ids = if let Some(text) = request.text_content() {
            self.send_text(
                &delivery_target_type,
                &delivery_target_id,
                text,
                reply_target.as_deref(),
            )
            .await?
        } else if let Some((image_path, _alt)) = request.image_content() {
            self.send_image(
                &delivery_target_type,
                &delivery_target_id,
                Path::new(image_path),
                reply_target.as_deref(),
            )
            .await?
        } else if request.file_content().is_some() {
            return Err(ChannelError::SendFailed(
                "file sending is currently supported only for telegram".to_owned(),
            ));
        } else {
            return Ok(SendMessageResult::default());
        };
        Ok(SendMessageResult { message_ids })
    }
}

impl FeishuSender {
    pub fn new(
        account_id: String,
        app_id: String,
        app_secret: String,
        api_base: String,
        is_running: bool,
    ) -> Self {
        Self {
            account_id,
            app_id,
            app_secret,
            api_base,
            is_running,
            http: Client::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub(crate) fn stream_client(&self) -> crate::feishu::FeishuClient {
        crate::feishu::FeishuClient::from_sender_parts(
            self.app_id.clone(),
            self.app_secret.clone(),
            self.api_base.clone(),
            self.http.clone(),
            self.token_state.clone(),
            self.refresh_lock.clone(),
        )
    }

    /// Refresh access token if needed then send a message.
    pub async fn send_text(
        &self,
        delivery_target_type: &str,
        delivery_target_id: &str,
        text: &str,
        reply_to_message_id: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let token = self.get_access_token().await?;

        let content = crate::feishu::build_card_content(text);

        if let Some(reply_id) = reply_to_message_id {
            // Reply to a specific message.
            let url = format!("{}/im/v1/messages/{}/reply", self.api_base, reply_id);
            let body = serde_json::json!({
                "msg_type": "interactive",
                "content": content,
            });

            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed(format!("Feishu reply failed: {e}")))?;

            return Self::parse_message_ids_from_response(resp, "reply").await;
        } else {
            // Send a new message.
            let receive_id_type = match delivery_target_type.trim() {
                "open_id" => "open_id",
                _ => "chat_id",
            };
            let url = format!(
                "{}/im/v1/messages?receive_id_type={receive_id_type}",
                self.api_base
            );
            let body = serde_json::json!({
                "receive_id": delivery_target_id,
                "msg_type": "interactive",
                "content": content,
            });

            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed(format!("Feishu send failed: {e}")))?;

            return Self::parse_message_ids_from_response(resp, "send").await;
        }
    }

    pub async fn send_image(
        &self,
        delivery_target_type: &str,
        delivery_target_id: &str,
        image_path: &Path,
        reply_to_message_id: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let token = self.get_access_token().await?;

        let image_bytes = tokio::fs::read(image_path).await.map_err(|e| {
            ChannelError::SendFailed(format!(
                "Feishu image read failed ({}): {e}",
                image_path.display()
            ))
        })?;
        let upload_url = format!("{}/im/v1/images", self.api_base);
        let filename = image_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("image.png")
            .to_owned();
        let image_part = reqwest::multipart::Part::bytes(image_bytes).file_name(filename);
        let upload_form = reqwest::multipart::Form::new()
            .text("image_type", "message")
            .part("image", image_part);
        let upload_resp = self
            .http
            .post(&upload_url)
            .header("Authorization", format!("Bearer {token}"))
            .multipart(upload_form)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(format!("Feishu image upload failed: {e}")))?;
        let upload_status = upload_resp.status();
        let upload_body = upload_resp.text().await.unwrap_or_default();
        if !upload_status.is_success() {
            return Err(ChannelError::SendFailed(format!(
                "Feishu image upload HTTP {upload_status}: {upload_body}"
            )));
        }
        let upload_json: Value = serde_json::from_str(&upload_body).map_err(|e| {
            ChannelError::SendFailed(format!(
                "Feishu image upload parse failed: {e}; body={upload_body}"
            ))
        })?;
        let upload_code = upload_json.get("code").and_then(Value::as_i64).unwrap_or(0);
        if upload_code != 0 {
            let msg = upload_json
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(ChannelError::SendFailed(format!(
                "Feishu image upload error (code={upload_code}): {msg}"
            )));
        }
        let image_key = upload_json
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("image_key"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if image_key.is_empty() {
            return Err(ChannelError::SendFailed(
                "Feishu image upload returned empty image_key".to_owned(),
            ));
        }

        let content = serde_json::json!({ "image_key": image_key }).to_string();
        if let Some(reply_id) = reply_to_message_id {
            let url = format!("{}/im/v1/messages/{}/reply", self.api_base, reply_id);
            let body = serde_json::json!({
                "msg_type": "image",
                "content": content,
            });
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed(format!("Feishu image reply failed: {e}")))?;
            Self::parse_message_ids_from_response(resp, "image reply").await
        } else {
            let receive_id_type = match delivery_target_type.trim() {
                "open_id" => "open_id",
                _ => "chat_id",
            };
            let url = format!(
                "{}/im/v1/messages?receive_id_type={receive_id_type}",
                self.api_base
            );
            let body = serde_json::json!({
                "receive_id": delivery_target_id,
                "msg_type": "image",
                "content": content,
            });
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed(format!("Feishu image send failed: {e}")))?;
            Self::parse_message_ids_from_response(resp, "image send").await
        }
    }

    pub async fn send_file(
        &self,
        delivery_target_type: &str,
        delivery_target_id: &str,
        file_path: &Path,
        reply_to_message_id: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let token = self.get_access_token().await?;

        let file_bytes = tokio::fs::read(file_path).await.map_err(|e| {
            ChannelError::SendFailed(format!(
                "Feishu file read failed ({}): {e}",
                file_path.display()
            ))
        })?;
        let upload_url = format!("{}/im/v1/files", self.api_base);
        let filename = file_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("attachment.bin")
            .to_owned();
        let file_part = reqwest::multipart::Part::bytes(file_bytes).file_name(filename.clone());
        let upload_form = reqwest::multipart::Form::new()
            .text("file_type", "stream")
            .text("file_name", filename)
            .part("file", file_part);
        let upload_resp = self
            .http
            .post(&upload_url)
            .header("Authorization", format!("Bearer {token}"))
            .multipart(upload_form)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(format!("Feishu file upload failed: {e}")))?;
        let upload_status = upload_resp.status();
        let upload_body = upload_resp.text().await.unwrap_or_default();
        if !upload_status.is_success() {
            return Err(ChannelError::SendFailed(format!(
                "Feishu file upload HTTP {upload_status}: {upload_body}"
            )));
        }
        let upload_json: Value = serde_json::from_str(&upload_body).map_err(|e| {
            ChannelError::SendFailed(format!(
                "Feishu file upload parse failed: {e}; body={upload_body}"
            ))
        })?;
        let upload_code = upload_json.get("code").and_then(Value::as_i64).unwrap_or(0);
        if upload_code != 0 {
            let msg = upload_json
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(ChannelError::SendFailed(format!(
                "Feishu file upload error (code={upload_code}): {msg}"
            )));
        }
        let file_key = upload_json
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("file_key"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if file_key.is_empty() {
            return Err(ChannelError::SendFailed(
                "Feishu file upload returned empty file_key".to_owned(),
            ));
        }

        let content = serde_json::json!({ "file_key": file_key }).to_string();
        if let Some(reply_id) = reply_to_message_id {
            let url = format!("{}/im/v1/messages/{}/reply", self.api_base, reply_id);
            let body = serde_json::json!({
                "msg_type": "file",
                "content": content,
            });
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed(format!("Feishu file reply failed: {e}")))?;
            Self::parse_message_ids_from_response(resp, "file reply").await
        } else {
            let receive_id_type = match delivery_target_type.trim() {
                "open_id" => "open_id",
                _ => "chat_id",
            };
            let url = format!(
                "{}/im/v1/messages?receive_id_type={receive_id_type}",
                self.api_base
            );
            let body = serde_json::json!({
                "receive_id": delivery_target_id,
                "msg_type": "file",
                "content": content,
            });
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed(format!("Feishu file send failed: {e}")))?;
            Self::parse_message_ids_from_response(resp, "file send").await
        }
    }

    async fn parse_message_ids_from_response(
        resp: reqwest::Response,
        op: &str,
    ) -> Result<Vec<String>, ChannelError> {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ChannelError::SendFailed(format!(
                "Feishu {op} HTTP {status}: {body_text}"
            )));
        }

        let payload: Value = serde_json::from_str(&body_text).map_err(|e| {
            ChannelError::SendFailed(format!("Feishu {op} parse failed: {e}; body={body_text}"))
        })?;
        let code = payload.get("code").and_then(Value::as_i64).unwrap_or(0);
        if code != 0 {
            let msg = payload
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(ChannelError::SendFailed(format!(
                "Feishu {op} error (code={code}): {msg}"
            )));
        }

        let mut message_ids = Vec::new();
        if let Some(mid) = payload
            .get("data")
            .and_then(Value::as_object)
            .and_then(|d| d.get("message_id"))
            .and_then(Value::as_str)
        {
            message_ids.push(mid.to_owned());
        }
        Ok(message_ids)
    }

    async fn get_access_token(&self) -> Result<String, ChannelError> {
        const TOKEN_REFRESH_MARGIN: std::time::Duration = std::time::Duration::from_secs(300);

        // Fast path: read-lock check.
        {
            let state = self.token_state.read().await;
            if let Some((token, exp)) = state.as_ref()
                && tokio::time::Instant::now() + TOKEN_REFRESH_MARGIN < *exp
            {
                return Ok(token.clone());
            }
        }

        let _refresh_guard = self.refresh_lock.lock().await;

        // Re-check after acquiring the mutex.
        {
            let state = self.token_state.read().await;
            if let Some((token, exp)) = state.as_ref()
                && tokio::time::Instant::now() + TOKEN_REFRESH_MARGIN < *exp
            {
                return Ok(token.clone());
            }
        }

        // Refresh the token.
        let url = format!("{}/auth/v3/tenant_access_token/internal", self.api_base);
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });

        let resp =
            self.http.post(&url).json(&body).send().await.map_err(|e| {
                ChannelError::Connection(format!("Feishu token refresh failed: {e}"))
            })?;

        #[derive(serde::Deserialize)]
        struct TokenResp {
            code: i64,
            #[serde(default)]
            tenant_access_token: String,
            #[serde(default)]
            expire: u64,
            #[serde(default)]
            msg: String,
        }

        let token_resp: TokenResp = resp
            .json()
            .await
            .map_err(|e| ChannelError::Connection(format!("Feishu token parse failed: {e}")))?;

        if token_resp.code != 0 {
            return Err(ChannelError::Connection(format!(
                "Feishu token error (code={}): {}",
                token_resp.code, token_resp.msg
            )));
        }

        let lifetime = if token_resp.expire > 0 {
            std::time::Duration::from_secs(token_resp.expire)
        } else {
            std::time::Duration::from_secs(7200)
        };

        let new_expires = tokio::time::Instant::now() + lifetime;
        {
            let mut state = self.token_state.write().await;
            *state = Some((token_resp.tenant_access_token.clone(), new_expires));
        }

        Ok(token_resp.tenant_access_token)
    }

    pub async fn fetch_app_owner_open_id(&self) -> Result<Option<String>, ChannelError> {
        let token = self.get_access_token().await?;
        let url = format!(
            "{}/application/v6/applications/{}?lang=zh_cn",
            self.api_base, self.app_id
        );
        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(format!("Feishu app owner fetch failed: {e}")))?;
        let payload: Value = response
            .json()
            .await
            .map_err(|e| ChannelError::SendFailed(format!("Feishu app owner parse failed: {e}")))?;
        let code = payload.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code != 0 {
            let msg = payload
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(ChannelError::SendFailed(format!(
                "Feishu app owner fetch error (code={code}): {msg}"
            )));
        }
        Ok(payload
            .get("data")
            .and_then(|value| value.get("app"))
            .and_then(|value| value.get("owner"))
            .and_then(|value| value.get("owner_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned))
    }

    pub async fn fetch_chat_summary(
        &self,
        chat_id: &str,
    ) -> Result<Option<FeishuChatSummary>, ChannelError> {
        let chat_id = chat_id.trim();
        if chat_id.is_empty() {
            return Ok(None);
        }

        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/chats/{chat_id}", self.api_base);
        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(format!("Feishu chat fetch failed: {e}")))?;
        let payload: Value = response
            .json()
            .await
            .map_err(|e| ChannelError::SendFailed(format!("Feishu chat parse failed: {e}")))?;
        let code = payload.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code != 0 {
            let msg = payload
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(ChannelError::SendFailed(format!(
                "Feishu chat fetch error (code={code}): {msg}"
            )));
        }

        let Some(chat) = payload.get("data").and_then(|value| value.as_object()) else {
            return Ok(None);
        };

        let read_optional = |field: &str| {
            chat.get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        };

        Ok(Some(FeishuChatSummary {
            name: read_optional("name"),
            chat_mode: read_optional("chat_mode"),
            chat_type: read_optional("chat_type"),
        }))
    }
}

impl OutboundMessage {
    pub fn text(
        channel: impl Into<String>,
        account_id: impl Into<String>,
        chat_id: impl Into<String>,
        delivery_target_type: impl Into<String>,
        delivery_target_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            account_id: account_id.into(),
            chat_id: chat_id.into(),
            delivery_target_type: delivery_target_type.into(),
            delivery_target_id: delivery_target_id.into(),
            content: ChannelOutboundContent::text(text),
            reply_to: None,
            thread_id: None,
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        self.content.as_text()
    }

    pub fn image_content(&self) -> Option<(&str, Option<&str>)> {
        self.content.as_image()
    }

    pub fn file_content(&self) -> Option<(&str, Option<&str>)> {
        self.content.as_file()
    }

    pub fn resolved_delivery_target_type(&self) -> String {
        infer_delivery_target_type(
            &self.channel,
            Some(&self.delivery_target_type),
            Some(&self.delivery_target_id),
            &self.chat_id,
            &self.chat_id,
        )
    }

    pub fn resolved_delivery_target_id(&self) -> String {
        infer_delivery_target_id(
            &self.channel,
            Some(&self.delivery_target_type),
            Some(&self.delivery_target_id),
            &self.chat_id,
            &self.chat_id,
        )
    }
}

// ---------------------------------------------------------------------------
// Concrete implementation
// ---------------------------------------------------------------------------

/// Channel names the built-in `send_message` match arms already
/// claim. A plugin registering under one of these would be shadowed
/// by the built-in routing, so `register_plugin` rejects them up
/// front. Single source of truth for both the guard
/// (`is_reserved_channel`) and the `register_plugin_rejects_reserved_builtin_names`
/// test — keep in lockstep with the arms in
/// [`ChannelDispatcher::send_message`].
pub(crate) const RESERVED_CHANNEL_NAMES: &[&str] =
    &["telegram", "discord", "feishu", "lark", "weixin", "wechat"];

/// Concrete dispatcher that routes outbound messages to registered channel senders.
///
/// **Clone semantics.** All inner sender types (`TelegramSender`,
/// `FeishuSender`, `WeixinSender`, `PluginSenderHandle`) are Clone and
/// reference-counted internally. Cloning the dispatcher produces a
/// shallow copy that still shares the underlying HTTP clients, RPC
/// writers, and token caches. The §9.4 respawn path relies on this to
/// build a forked dispatcher cheaply and hot-swap it into
/// [`SwappableDispatcher`] without disturbing in-flight calls.
#[derive(Clone)]
pub struct ChannelDispatcherImpl {
    telegram_senders: HashMap<String, TelegramSender>,
    discord_senders: HashMap<String, DiscordSender>,
    feishu_senders: HashMap<String, FeishuSender>,
    weixin_senders: HashMap<String, WeixinSender>,
    weixin_running: Arc<AtomicBool>,
    /// Plugin-backed senders keyed by their manifest `plugin.id`. The
    /// manager registers one entry per plugin whose lifecycle state is
    /// `Running` and unregisters on stop/respawn (§9.4). The entry's
    /// identifier is the channel string callers pass in
    /// [`OutboundMessage::channel`].
    plugin_senders: HashMap<String, PluginSenderHandle>,
}

impl ChannelDispatcherImpl {
    pub fn new() -> Self {
        Self {
            telegram_senders: HashMap::new(),
            discord_senders: HashMap::new(),
            feishu_senders: HashMap::new(),
            weixin_senders: HashMap::new(),
            weixin_running: Arc::new(AtomicBool::new(false)),
            plugin_senders: HashMap::new(),
        }
    }

    /// Build a dispatcher from the channels configuration.
    ///
    /// Registers senders for all enabled accounts so they can be used for
    /// outbound delivery even though the channels themselves are started
    /// separately via the plugin manager.
    pub fn from_config(channels: &ChannelsConfig) -> Self {
        Self::from_config_with_weixin_running(channels, Arc::new(AtomicBool::new(false)))
    }

    pub fn from_config_with_weixin_running(
        channels: &ChannelsConfig,
        weixin_running: Arc<AtomicBool>,
    ) -> Self {
        let mut dispatcher = Self::new();
        dispatcher.weixin_running = weixin_running;
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        let telegram = channels.resolved_telegram_config().unwrap_or_else(|error| {
            warn!(error = %error, "failed to resolve telegram plugin config");
            Default::default()
        });
        let discord = channels.resolved_discord_config().unwrap_or_else(|error| {
            warn!(error = %error, "failed to resolve discord plugin config");
            Default::default()
        });
        let feishu = channels.resolved_feishu_config().unwrap_or_else(|error| {
            warn!(error = %error, "failed to resolve feishu plugin config");
            Default::default()
        });
        let weixin = channels.resolved_weixin_config().unwrap_or_else(|error| {
            warn!(error = %error, "failed to resolve weixin plugin config");
            Default::default()
        });

        // Register Telegram senders.
        for (account_id, account) in &telegram.accounts {
            if !account.enabled {
                continue;
            }
            dispatcher.register_telegram(TelegramSender {
                account_id: account_id.clone(),
                token: account.token.clone(),
                http: http.clone(),
                api_base: "https://api.telegram.org".to_string(),
                is_running: true,
            });
        }

        // Register Discord senders.
        for (account_id, account) in &discord.accounts {
            if !account.enabled {
                continue;
            }
            dispatcher.register_discord(DiscordSender {
                account_id: account_id.clone(),
                token: account.token.clone(),
                http: http.clone(),
                api_base: account.api_base.clone(),
                is_running: true,
            });
        }

        // Register Feishu senders.
        for (account_id, account) in &feishu.accounts {
            if !account.enabled {
                continue;
            }
            let api_base = match account.domain {
                FeishuDomain::Lark => "https://open.larksuite.com/open-apis",
                FeishuDomain::Feishu => "https://open.feishu.cn/open-apis",
            };
            dispatcher.register_feishu(FeishuSender::new(
                account_id.clone(),
                account.app_id.clone(),
                account.app_secret.clone(),
                api_base.to_string(),
                true,
            ));
        }

        // Register Weixin senders.
        for (account_id, account) in &weixin.accounts {
            if !account.enabled {
                continue;
            }
            dispatcher.register_weixin(WeixinSender {
                account_id: account_id.clone(),
                account: account.clone(),
                http: http.clone(),
                is_running: true,
                running: dispatcher.weixin_running.clone(),
            });
        }

        dispatcher
    }

    pub fn channel_running_handle(&self, channel: &str) -> Option<Arc<AtomicBool>> {
        match channel {
            "weixin" | "wechat" => Some(self.weixin_running.clone()),
            _ => None,
        }
    }

    pub fn register_telegram(&mut self, sender: TelegramSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Telegram sender for dispatch"
        );
        self.telegram_senders
            .insert(sender.account_id.clone(), sender);
    }

    pub fn register_discord(&mut self, sender: DiscordSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Discord sender for dispatch"
        );
        self.discord_senders
            .insert(sender.account_id.clone(), sender);
    }

    pub fn register_feishu(&mut self, sender: FeishuSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Feishu sender for dispatch"
        );
        self.feishu_senders
            .insert(sender.account_id.clone(), sender);
    }

    pub fn register_weixin(&mut self, mut sender: WeixinSender) {
        if self.weixin_senders.is_empty() && !Arc::ptr_eq(&sender.running, &self.weixin_running) {
            self.weixin_running = sender.running.clone();
        } else {
            sender.running = self.weixin_running.clone();
        }
        info!(
            account_id = %sender.account_id,
            "Registered Weixin sender for dispatch"
        );
        self.weixin_senders
            .insert(sender.account_id.clone(), sender);
    }

    /// Register a plugin-backed outbound sender (§9.4). The handle's
    /// `plugin_id` becomes the channel string accepted by
    /// `send_message`. Re-registering the same id overwrites the prior
    /// handle, which is what `respawn_plugin` relies on.
    ///
    /// Returns `ChannelError::Config` if `plugin_id` collides with a
    /// reserved built-in route name (`telegram`, `feishu`, `lark`,
    /// `weixin`, `wechat`). Without this guard a colliding registration
    /// would succeed silently but `send_message`'s built-in match arms
    /// would shadow the plugin, producing an "unroutable" channel that
    /// appears in `available_channels` but never receives traffic.
    pub fn register_plugin(&mut self, sender: PluginSenderHandle) -> Result<(), ChannelError> {
        let id = sender.plugin_id();
        if Self::is_reserved_channel(id) {
            return Err(ChannelError::Config(format!(
                "plugin id '{id}' collides with a reserved built-in channel name"
            )));
        }
        info!(plugin_id = %id, "Registered plugin sender for dispatch");
        self.plugin_senders.insert(id.to_owned(), sender);
        Ok(())
    }

    fn is_reserved_channel(name: &str) -> bool {
        RESERVED_CHANNEL_NAMES.contains(&name)
    }

    /// Remove a plugin sender by `plugin_id`. Returns the removed
    /// handle if present; useful for respawn paths that want to take
    /// ownership of the old RPC client before discarding it.
    pub fn unregister_plugin(&mut self, plugin_id: &str) -> Option<PluginSenderHandle> {
        self.plugin_senders.remove(plugin_id)
    }

    /// Clone the [`PluginSenderHandle`] for `plugin_id`, if present.
    /// Used by the streaming inbound path to reach the plugin's
    /// transport for `inbound/stream_frame` notifications without
    /// going through the request-shaped [`Self::send_message`] path.
    pub fn plugin_sender(&self, plugin_id: &str) -> Option<PluginSenderHandle> {
        self.plugin_senders.get(plugin_id).cloned()
    }

    /// Snapshot every currently-registered subprocess plugin sender.
    /// Used by `apply_runtime_config` to carry the dynamic
    /// `register_subprocess_plugin` / `respawn_plugin` wiring across a
    /// `ChannelDispatcherImpl::from_config` rebuild — `from_config`
    /// only seeds built-in channels declared in `GaryxConfig` and would
    /// otherwise wipe every plugin sender on each config reload.
    pub fn plugin_senders_snapshot(&self) -> Vec<PluginSenderHandle> {
        self.plugin_senders.values().cloned().collect()
    }

    /// Build a forked dispatcher that is identical to `self` except the
    /// plugin-sender entry for `sender.plugin_id()` points at `sender`.
    /// Used by [`crate::plugin::ChannelPluginManager::respawn_plugin`]
    /// to stage the new wiring before hot-swapping it into
    /// [`SwappableDispatcher`] (§9.4 step 1).
    ///
    /// Returns [`ChannelError::Config`] when the incoming plugin id
    /// collides with a reserved built-in channel name — the same guard
    /// [`Self::register_plugin`] enforces, repeated here because
    /// `fork_with_plugin_sender` is a second write path.
    pub fn fork_with_plugin_sender(
        &self,
        sender: PluginSenderHandle,
    ) -> Result<Self, ChannelError> {
        let id = sender.plugin_id();
        if Self::is_reserved_channel(id) {
            return Err(ChannelError::Config(format!(
                "plugin id '{id}' collides with a reserved built-in channel name"
            )));
        }
        let mut forked = self.clone();
        forked.plugin_senders.insert(id.to_owned(), sender);
        Ok(forked)
    }
}

impl Default for ChannelDispatcherImpl {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_telegram_id(field: &str, value: &str) -> Result<i64, ChannelError> {
    value.parse().map_err(|error| {
        ChannelError::Config(format!("Invalid Telegram {field} '{value}': {error}"))
    })
}

fn parse_optional_telegram_id(
    field: &str,
    value: Option<&str>,
) -> Result<Option<i64>, ChannelError> {
    value.map(|raw| parse_telegram_id(field, raw)).transpose()
}

fn normalize_telegram_thread_id(chat_id: i64, raw_thread_id: Option<&str>) -> Option<i64> {
    let parsed = raw_thread_id.and_then(|raw| raw.trim().parse::<i64>().ok())?;
    (parsed != chat_id).then_some(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramMarkdownImageRef {
    path: String,
    caption: Option<String>,
}

fn telegram_markdown_link_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"!?\[([^\]]*)\]\(([^)]+)\)").expect("valid telegram markdown link regex")
    })
}

fn extract_telegram_markdown_image_refs(text: &str) -> (String, Vec<TelegramMarkdownImageRef>) {
    let mut cleaned = String::new();
    let mut image_refs = Vec::new();
    let mut last_end = 0;

    for caps in telegram_markdown_link_regex().captures_iter(text) {
        let Some(whole) = caps.get(0) else {
            continue;
        };
        let Some(destination) = caps.get(2).map(|m| m.as_str()) else {
            continue;
        };
        let Some(path) = telegram_local_image_path_from_markdown_destination(destination) else {
            continue;
        };

        cleaned.push_str(&text[last_end..whole.start()]);
        last_end = whole.end();

        let caption = caps
            .get(1)
            .map(|m| m.as_str().trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(512).collect::<String>());
        image_refs.push(TelegramMarkdownImageRef { path, caption });
    }

    if image_refs.is_empty() {
        return (text.to_owned(), image_refs);
    }

    cleaned.push_str(&text[last_end..]);
    (
        compact_text_after_telegram_markdown_image_removal(&cleaned),
        image_refs,
    )
}

fn telegram_local_image_path_from_markdown_destination(raw: &str) -> Option<String> {
    let candidate = markdown_destination_without_title(raw)
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
        .trim();
    if candidate.is_empty() {
        return None;
    }

    let path = if let Some(rest) = candidate.strip_prefix("file://") {
        if let Some(localhost_path) = rest.strip_prefix("localhost/") {
            format!("/{localhost_path}")
        } else {
            rest.to_owned()
        }
    } else if let Some(rest) = candidate.strip_prefix("file:") {
        rest.to_owned()
    } else {
        candidate.to_owned()
    };

    let decoded = urlencoding::decode(&path).ok()?.into_owned();
    if is_telegram_local_image_path(&decoded) {
        Some(decoded)
    } else {
        None
    }
}

fn markdown_destination_without_title(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix('<')
        && let Some(end) = rest.find('>')
    {
        return &rest[..end];
    }

    for marker in [" \"", " '", " ("] {
        if let Some(index) = trimmed.find(marker) {
            return &trimmed[..index];
        }
    }

    trimmed
}

fn is_telegram_local_image_path(path: &str) -> bool {
    if !Path::new(path).is_absolute() {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
}

fn compact_text_after_telegram_markdown_image_removal(text: &str) -> String {
    let mut lines = Vec::new();
    let mut last_was_blank = false;

    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if !last_was_blank {
                lines.push("");
            }
            last_was_blank = true;
        } else {
            lines.push(line);
            last_was_blank = false;
        }
    }

    lines.join("\n").trim().to_owned()
}

fn extract_feishu_thread_reply_target(thread_id: &str) -> Option<&str> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((_, root_id)) = trimmed.rsplit_once(":topic:") {
        let normalized = root_id.trim();
        return normalized.starts_with("om_").then_some(normalized);
    }
    trimmed.starts_with("om_").then_some(trimmed)
}

fn resolve_feishu_reply_target(reply_to: Option<&str>, thread_id: Option<&str>) -> Option<String> {
    let explicit_reply = reply_to
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    explicit_reply.or_else(|| {
        thread_id
            .and_then(extract_feishu_thread_reply_target)
            .map(ToOwned::to_owned)
    })
}

#[async_trait]
impl ChannelDispatcher for ChannelDispatcherImpl {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        debug!(
            channel = %request.channel,
            account = %request.account_id,
            chat = %request.chat_id,
            content_kind = %request.content.kind(),
            delivery_target_type = %request.resolved_delivery_target_type(),
            delivery_target_id = %request.resolved_delivery_target_id(),
            "Dispatching outbound message"
        );

        match request.channel.as_str() {
            "telegram" => {
                let sender = self
                    .telegram_senders
                    .get(&request.account_id)
                    .ok_or_else(|| {
                        ChannelError::Config(format!(
                            "Telegram account '{}' not registered in dispatcher",
                            request.account_id
                        ))
                    })?;
                sender.send_outbound(request).await
            }
            "discord" => {
                let sender = self
                    .discord_senders
                    .get(&request.account_id)
                    .ok_or_else(|| {
                        ChannelError::Config(format!(
                            "Discord account '{}' not registered in dispatcher",
                            request.account_id
                        ))
                    })?;
                sender.send_outbound(request).await
            }
            "feishu" | "lark" => {
                let sender = self
                    .feishu_senders
                    .get(&request.account_id)
                    .ok_or_else(|| {
                        ChannelError::Config(format!(
                            "Feishu account '{}' not registered in dispatcher",
                            request.account_id
                        ))
                    })?;
                sender.send_outbound(request).await
            }
            "weixin" | "wechat" => {
                let sender = self
                    .weixin_senders
                    .get(&request.account_id)
                    .ok_or_else(|| {
                        ChannelError::Config(format!(
                            "Weixin account '{}' not registered in dispatcher",
                            request.account_id
                        ))
                    })?;
                sender.send_outbound(request).await
            }
            other => {
                // §9.4 routing order: built-in match exhausted; fall
                // back to plugin senders keyed by `plugin_id`. An
                // unknown name after that is a genuine config error.
                if let Some(plugin) = self.plugin_senders.get(other) {
                    let delivery_target_type = request.resolved_delivery_target_type();
                    let delivery_target_id = request.resolved_delivery_target_id();
                    let dispatch_req = DispatchOutbound {
                        account_id: request.account_id.clone(),
                        chat_id: request.chat_id.clone(),
                        delivery_target_type,
                        delivery_target_id,
                        content: request.content.clone(),
                        reply_to: request.reply_to.clone(),
                        thread_id: request.thread_id.clone(),
                    };
                    let result = plugin.dispatch(dispatch_req).await?;
                    Ok(SendMessageResult {
                        message_ids: result.message_ids,
                    })
                } else {
                    Err(ChannelError::Config(format!(
                        "Unknown channel type: '{other}'"
                    )))
                }
            }
        }
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        let mut channels = Vec::new();

        for sender in self.telegram_senders.values() {
            channels.push(ChannelInfo {
                channel: "telegram".to_string(),
                account_id: sender.account_id.clone(),
                is_running: sender.is_running,
            });
        }

        for sender in self.discord_senders.values() {
            channels.push(ChannelInfo {
                channel: "discord".to_string(),
                account_id: sender.account_id.clone(),
                is_running: sender.is_running,
            });
        }

        for sender in self.feishu_senders.values() {
            channels.push(ChannelInfo {
                channel: "feishu".to_string(),
                account_id: sender.account_id.clone(),
                is_running: sender.is_running,
            });
        }

        for sender in self.weixin_senders.values() {
            channels.push(ChannelInfo {
                channel: "weixin".to_string(),
                account_id: sender.account_id.clone(),
                is_running: sender.is_running,
            });
        }

        // Plugin-backed channels: the dispatcher only knows the plugin
        // id, not per-account state. The manager holds the full
        // plugin-account map and exposes it via `list-channel-accounts`
        // IPC; this entry is a presence marker so a caller that only
        // talks to the dispatcher still sees the plugin exists.
        for plugin in self.plugin_senders.values() {
            channels.push(ChannelInfo {
                channel: plugin.plugin_id().to_owned(),
                account_id: String::new(),
                is_running: true,
            });
        }

        channels.sort_by(|a, b| (&a.channel, &a.account_id).cmp(&(&b.channel, &b.account_id)));
        channels
    }

    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        match target.channel.as_str() {
            "telegram" => {
                let sender = self.telegram_senders.get(&target.account_id)?;
                let chat_id = parse_telegram_id("chat_id", &target.chat_id).ok()?;
                let outbound_thread_id =
                    normalize_telegram_thread_id(chat_id, target.thread_id.as_deref());

                let stream_callback = crate::telegram::build_bound_response_callback(
                    crate::telegram::StreamingCallbackConfig {
                        http: sender.http.clone(),
                        token: sender.token.clone(),
                        account_id: sender.account_id.clone(),
                        chat_id,
                        api_base: sender.api_base.clone(),
                        reply_to_mode: garyx_models::config::ReplyToMode::Off,
                        reply_to: None,
                        outbound_thread_id,
                    },
                );
                Some(Arc::new(move |envelope| {
                    stream_callback(envelope.event);
                }))
            }
            "feishu" => {
                let sender = self.feishu_senders.get(&target.account_id)?;
                let (stream_callback, thread_id_tx) = crate::feishu::build_feishu_response_callback(
                    crate::feishu::FeishuStreamingCallbackConfig {
                        client: sender.stream_client(),
                        account_id: sender.account_id.clone(),
                        receive_id_type: target.delivery_target_type.clone(),
                        chat_id: target.delivery_target_id.clone(),
                        reply_message_id: None,
                        reply_in_thread: false,
                        is_group_reply: false,
                        mention_prefix: String::new(),
                        processing_reaction_id: None,
                    },
                );
                let _ = thread_id_tx.send(target.target_thread_id.clone());
                Some(Arc::new(move |envelope| {
                    stream_callback(envelope.event);
                }))
            }
            "discord" => {
                let sender = self.discord_senders.get(&target.account_id)?;
                let (stream_callback, thread_id_tx) =
                    crate::discord::build_discord_response_callback(
                        crate::discord::DiscordStreamingCallbackConfig {
                            sender: sender.clone(),
                            chat_id: target.chat_id.clone(),
                            reply_to_message_id: None,
                        },
                    );
                let _ = thread_id_tx.send(target.target_thread_id.clone());
                Some(Arc::new(move |envelope| {
                    stream_callback(envelope.event);
                }))
            }
            "weixin" => {
                let sender = self.weixin_senders.get(&target.account_id)?;
                let stream_callback = crate::weixin::build_weixin_response_callback(
                    crate::weixin::WeixinStreamingCallbackConfig {
                        http: sender.http.clone(),
                        account: sender.account.clone(),
                        account_id: sender.account_id.clone(),
                        user_id: target.delivery_target_id.clone(),
                        context_token: String::new(),
                        thread_id: target.target_thread_id.clone(),
                        typing_ticket: None,
                        running: sender.running.clone(),
                    },
                );
                Some(Arc::new(move |envelope| {
                    stream_callback(envelope.event);
                }))
            }
            other => {
                let sender = self.plugin_senders.get(other)?;
                if sender.capabilities().dispatch_stream_event {
                    Some(build_plugin_stream_event_callback(sender.clone(), target))
                } else {
                    None
                }
            }
        }
    }

    fn supports_legacy_stream_adapter(&self, target: &StreamingDispatchTarget) -> bool {
        self.plugin_senders
            .get(target.channel.as_str())
            .map(|sender| {
                let capabilities = sender.capabilities();
                capabilities.outbound && !capabilities.dispatch_stream_event
            })
            .unwrap_or(false)
    }

    fn channel_running_handle(&self, channel: &str) -> Option<Arc<AtomicBool>> {
        ChannelDispatcherImpl::channel_running_handle(self, channel)
    }
}

// ---------------------------------------------------------------------------
// SwappableDispatcher — atomic hot-swap around ChannelDispatcherImpl
// ---------------------------------------------------------------------------

/// Atomic-swap container around [`ChannelDispatcherImpl`] (§9.4).
///
/// Callers interact with it through the [`ChannelDispatcher`] trait
/// exactly as if it were the underlying impl. Under the hood every
/// call loads a snapshot Arc without locking, so an ongoing
/// `send_message` runs against a stable snapshot — even if a
/// concurrent [`Self::store`] publishes a new one mid-flight.
///
/// **Cancellation.** If the caller drops the `send_message` future,
/// the captured Arc chain drops with it and the transport's
/// `PendingGuard` (see
/// [`super::plugin_host::transport`]) removes the in-flight entry
/// from the plugin's `pending` map. The in-flight RPC is abandoned
/// cleanly; there is no waiter leak.
///
/// The swap itself takes **no locks** on the read path and completes
/// synchronously; the old `ChannelDispatcherImpl` stays live until the
/// last outstanding snapshot is dropped. This is the mechanism §9.4
/// prescribes for "publish new dispatcher → stop old child → drain
/// window → shutdown".
pub struct SwappableDispatcher {
    inner: ArcSwap<ChannelDispatcherImpl>,
}

impl SwappableDispatcher {
    pub fn new(initial: ChannelDispatcherImpl) -> Self {
        Self {
            inner: ArcSwap::from_pointee(initial),
        }
    }

    /// Current dispatcher snapshot. Cheap clone (Arc bump).
    pub fn load(&self) -> Arc<ChannelDispatcherImpl> {
        self.inner.load_full()
    }

    /// Publish a new dispatcher. The previous one stays alive for any
    /// in-flight RPCs that already captured a snapshot.
    pub fn store(&self, next: Arc<ChannelDispatcherImpl>) {
        self.inner.store(next);
    }

    /// Snapshot-and-lookup helper: clone the [`PluginSenderHandle`]
    /// for `plugin_id` from the currently-published dispatcher. The
    /// snapshot stays stable for the caller's own lifetime — a
    /// concurrent `store` publishes a new dispatcher but doesn't
    /// invalidate the returned handle (senders are Arc-backed).
    pub fn plugin_sender(&self, plugin_id: &str) -> Option<PluginSenderHandle> {
        self.inner.load().plugin_sender(plugin_id)
    }
}

#[async_trait]
impl ChannelDispatcher for SwappableDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        // Capture the snapshot before the await so a concurrent swap
        // cannot yank the dispatcher out from under the future.
        let snapshot = self.inner.load_full();
        snapshot.send_message(request).await
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        self.inner.load().available_channels()
    }

    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        self.inner.load().build_stream_event_callback(target)
    }

    fn supports_legacy_stream_adapter(&self, target: &StreamingDispatchTarget) -> bool {
        self.inner.load().supports_legacy_stream_adapter(target)
    }

    fn channel_running_handle(&self, channel: &str) -> Option<Arc<AtomicBool>> {
        self.inner.load().channel_running_handle(channel)
    }
}

/// A clonable handle that can send Weixin messages without owning the channel.
#[derive(Clone)]
pub struct WeixinSender {
    pub account_id: String,
    pub account: garyx_models::config::WeixinAccount,
    pub http: Client,
    pub is_running: bool,
    pub running: Arc<std::sync::atomic::AtomicBool>,
}

impl WeixinSender {
    pub async fn send_text(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let message_id = crate::weixin::send_text_message(
            &self.http,
            &self.account,
            to_user_id,
            text,
            context_token,
        )
        .await?;
        Ok(vec![message_id])
    }

    pub async fn send_image(
        &self,
        to_user_id: &str,
        image_path: &Path,
        caption: Option<&str>,
        context_token: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let message_id = crate::weixin::send_image_message_from_path(
            &self.http,
            &self.account,
            to_user_id,
            image_path,
            caption,
            context_token,
        )
        .await?;
        Ok(vec![message_id])
    }
}

#[async_trait]
impl OutboundSender for WeixinSender {
    async fn send_outbound(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        let delivery_target_id = request.resolved_delivery_target_id();
        let context_token = weixin::get_context_token_for_thread(
            &request.account_id,
            &delivery_target_id,
            request.thread_id.as_deref(),
        )
        .await;
        if let Some(text) = request.text_content() {
            match self
                .send_text(&delivery_target_id, text, context_token.as_deref())
                .await
            {
                Ok(message_ids) => Ok(SendMessageResult { message_ids }),
                Err(error) => {
                    // Queue the failed message for later delivery when a
                    // fresh context_token arrives via an inbound message.
                    // Heuristic: only retry-queue on token-shaped errors;
                    // other failures propagate so the caller's retry
                    // policy can make its own decision.
                    let error_str = error.to_string();
                    let is_token_error = error_str.contains("ret=")
                        || error_str.contains("ret!=0")
                        || error_str.contains("context_token")
                        || error_str.contains("send limit");
                    if is_token_error {
                        weixin::queue_pending_outbound(
                            &request.account_id,
                            &delivery_target_id,
                            text,
                        )
                        .await;
                    }
                    Err(error)
                }
            }
        } else if let Some((image_path, _alt)) = request.image_content() {
            self.send_image(
                &delivery_target_id,
                Path::new(image_path),
                None,
                context_token.as_deref(),
            )
            .await
            .map(|message_ids| SendMessageResult { message_ids })
        } else if request.file_content().is_some() {
            Err(ChannelError::SendFailed(
                "file sending is currently supported only for telegram".to_owned(),
            ))
        } else {
            Ok(SendMessageResult::default())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
