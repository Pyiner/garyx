//! ChannelDispatcher — outbound message delivery to channels.
//!
//! Allows any component (MCP tools, cron jobs, API endpoints) to send
//! messages OUT through channel transports (Telegram, Feishu) without needing
//! direct access to channel internals.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use garyx_models::ChannelOutboundContent;
use garyx_models::provider::{StreamBoundaryKind, StreamEvent};
use garyx_models::routing::{infer_delivery_target_id, infer_delivery_target_type};
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use garyx_models::config::{ChannelsConfig, FeishuDomain};

use crate::channel_trait::ChannelError;
use crate::plugin_host::{DispatchOutbound, DispatchStreamEvent, PluginSenderHandle};

pub use crate::discord::outbound::{DiscordChannelSender, DiscordSender};
pub use crate::feishu::outbound::{FeishuChannelSender, FeishuSender};
pub use crate::telegram::outbound::{TelegramChannelSender, TelegramSender};
#[cfg(test)]
pub(crate) use crate::telegram::outbound::{
    extract_telegram_markdown_image_refs, normalize_telegram_thread_id,
};
pub use crate::weixin::outbound::{WeixinChannelSender, WeixinSender};

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

// ---------------------------------------------------------------------------
// Discord sender handle
// ---------------------------------------------------------------------------

/// A clonable handle that can send Discord messages without owning the channel.

// ---------------------------------------------------------------------------
// Feishu sender handle
// ---------------------------------------------------------------------------

/// A clonable handle that can send Feishu messages without owning the channel.

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

pub use crate::builtin_catalog::RESERVED_CHANNEL_NAMES;

/// Uniform outbound contract every registered channel conforms to —
/// built-in or plugin-backed. The method set mirrors the subprocess
/// plugin wire contract (dispatch / stream-event callback / accounts /
/// capability-driven legacy selection), so a built-in channel is
/// simply the in-process implementation of the same shape a
/// `PluginSenderHandle` provides over JSON-RPC (Phase-6 B2:
/// 内外插件真同构, 改内不改外).
///
/// The DTO stays the host-side [`OutboundMessage`] rather than the
/// wire `DispatchOutbound`: the plugin impl converts at its boundary,
/// exactly where the process boundary sits.
#[async_trait]
pub trait OutboundChannelSender: Send + Sync {
    /// Canonical channel id — the registry's primary routing key.
    fn channel_id(&self) -> &str;

    /// Alias spellings that resolve to this sender (`lark`, `wechat`).
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// Per-account rows for [`ChannelDispatcher::available_channels`].
    /// Built-ins list one row per registered account; plugins expose a
    /// single presence-marker row (empty `account_id`).
    fn accounts(&self) -> Vec<ChannelInfo>;

    /// Shared running flag, when the channel exposes one (weixin).
    fn running_handle(&self) -> Option<Arc<AtomicBool>> {
        None
    }

    /// Deliver one outbound message. `request.channel` may be an alias
    /// of [`Self::channel_id`]; account resolution and the
    /// unregistered-account error text are owned by the sender.
    async fn dispatch(&self, request: OutboundMessage) -> Result<SendMessageResult, ChannelError>;

    /// Construct the native per-target stream-event callback, or
    /// `None` when the target's account is unknown or the sender lacks
    /// the native streaming capability.
    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback>;

    /// Whether the host-rendered legacy outbound stream adapter
    /// applies (§9.4 capability gate: `outbound` without
    /// `dispatch_stream_event`). Built-ins are native and default to
    /// `false`.
    fn supports_legacy_stream_adapter(&self, target: &StreamingDispatchTarget) -> bool {
        let _ = target;
        false
    }
}

/// Built-in Telegram channel: account map + the in-process
/// [`OutboundChannelSender`] implementation.

/// Built-in Discord channel.

/// Built-in Feishu channel (alias: `lark`).

/// Built-in Weixin channel (alias: `wechat`). Owns the shared
/// process-wide running flag; registration reconciles every account
/// sender onto that one `AtomicBool` (first registered sender may
/// donate its handle when the channel-level one was defaulted).

/// A subprocess plugin is the out-of-process implementation of the
/// same contract: `dispatch` crosses the wire as `dispatch_outbound`,
/// the stream callback as `dispatch_stream_event`, and the legacy
/// adapter selection is driven by the plugin's advertised
/// capabilities.
#[async_trait]
impl OutboundChannelSender for PluginSenderHandle {
    fn channel_id(&self) -> &str {
        self.plugin_id()
    }

    fn accounts(&self) -> Vec<ChannelInfo> {
        // Plugin-backed channels: the dispatcher only knows the plugin
        // id, not per-account state. The manager holds the full
        // plugin-account map and exposes it via `list-channel-accounts`
        // IPC; this entry is a presence marker so a caller that only
        // talks to the dispatcher still sees the plugin exists.
        vec![ChannelInfo {
            channel: self.plugin_id().to_owned(),
            account_id: String::new(),
            is_running: true,
        }]
    }

    async fn dispatch(&self, request: OutboundMessage) -> Result<SendMessageResult, ChannelError> {
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
        let result = PluginSenderHandle::dispatch(self, dispatch_req).await?;
        Ok(SendMessageResult {
            message_ids: result.message_ids,
        })
    }

    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        if self.capabilities().dispatch_stream_event {
            Some(build_plugin_stream_event_callback(self.clone(), target))
        } else {
            None
        }
    }

    fn supports_legacy_stream_adapter(&self, _target: &StreamingDispatchTarget) -> bool {
        let capabilities = self.capabilities();
        capabilities.outbound && !capabilities.dispatch_stream_event
    }
}

/// Concrete dispatcher that routes outbound messages through the
/// uniform [`OutboundChannelSender`] registry: four always-present
/// built-in channel senders plus one entry per running subprocess
/// plugin. Routing is data-driven (`channel_id()` / `aliases()`);
/// there are no channel-name match arms here.
///
/// **Clone semantics.** All inner account sender types
/// (`TelegramSender`, `FeishuSender`, `WeixinSender`,
/// `PluginSenderHandle`) are Clone and reference-counted internally.
/// Cloning the dispatcher produces a shallow copy that still shares
/// the underlying HTTP clients, RPC writers, token caches, and the
/// weixin running flag. The §9.4 respawn path relies on this to build
/// a forked dispatcher cheaply and hot-swap it into
/// [`SwappableDispatcher`] without disturbing in-flight calls.
#[derive(Clone)]
pub struct ChannelDispatcherImpl {
    /// Type-erased built-in channel senders, injected at construction
    /// from [`crate::builtin_catalog::builtin_sender_registry`]. The
    /// downcast capability lives inside
    /// [`crate::outbound_registry::BuiltinSenderRegistry`] (private
    /// trait + private field): the dispatcher core can only `route`,
    /// `iter`, or feed consume-only sealed registrations through
    /// `register`. Adding a built-in channel touches the catalog, the
    /// channel's own module, and the registry's `Sealed` allowlist.
    builtin_senders: crate::outbound_registry::BuiltinSenderRegistry,
    /// Plugin-backed senders keyed by their manifest `plugin.id`. The
    /// manager registers one entry per plugin whose lifecycle state is
    /// `Running` and unregisters on stop/respawn (§9.4). The entry's
    /// identifier is the channel string callers pass in
    /// [`OutboundMessage::channel`].
    plugin_senders: HashMap<String, PluginSenderHandle>,
}

impl ChannelDispatcherImpl {
    pub fn new() -> Self {
        Self::with_weixin_running(Arc::new(AtomicBool::new(false)))
    }

    fn with_weixin_running(weixin_running: Arc<AtomicBool>) -> Self {
        Self {
            builtin_senders: crate::builtin_catalog::builtin_sender_registry(weixin_running),
            plugin_senders: HashMap::new(),
        }
    }

    /// Resolve a channel name (canonical id or alias) to its sender.
    /// Built-ins win over plugins by construction: `register_plugin`
    /// rejects reserved names, so the two key spaces are disjoint.
    fn route(&self, name: &str) -> Option<&dyn OutboundChannelSender> {
        self.builtin_senders.route(name).or_else(|| {
            self.plugin_senders
                .get(name)
                .map(|sender| sender as &dyn OutboundChannelSender)
        })
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
        let mut dispatcher = Self::with_weixin_running(weixin_running.clone());
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
            // The registry's weixin wrapper was constructed with this
            // exact handle above; per-account senders share it.
            let running = weixin_running.clone();
            dispatcher.register_weixin(WeixinSender {
                account_id: account_id.clone(),
                account: account.clone(),
                http: http.clone(),
                is_running: true,
                running,
            });
        }

        dispatcher
    }

    pub fn channel_running_handle(&self, channel: &str) -> Option<Arc<AtomicBool>> {
        self.route(channel)?.running_handle()
    }

    pub fn register_telegram(&mut self, sender: TelegramSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Telegram sender for dispatch"
        );
        self.builtin_senders.register(sender);
    }

    pub fn register_discord(&mut self, sender: DiscordSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Discord sender for dispatch"
        );
        self.builtin_senders.register(sender);
    }

    pub fn register_feishu(&mut self, sender: FeishuSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Feishu sender for dispatch"
        );
        self.builtin_senders.register(sender);
    }

    pub fn register_weixin(&mut self, sender: WeixinSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Weixin sender for dispatch"
        );
        self.builtin_senders.register(sender);
    }

    /// Register a plugin-backed outbound sender (§9.4). The handle's
    /// `plugin_id` becomes the channel string accepted by
    /// `send_message`. Re-registering the same id overwrites the prior
    /// handle, which is what `respawn_plugin` relies on.
    ///
    /// Returns `ChannelError::Config` if `plugin_id` collides with a
    /// reserved built-in route name
    /// ([`crate::builtin_catalog::RESERVED_CHANNEL_NAMES`]). Without
    /// this guard a colliding registration would succeed silently but
    /// `route` resolves built-ins first, producing an "unroutable"
    /// channel that appears in `available_channels` but never receives
    /// traffic.
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

        // §9.4 routing: one uniform registry lookup (canonical id or
        // alias for built-ins, plugin_id for plugins). An unknown name
        // is a genuine config error.
        match self.route(request.channel.as_str()) {
            Some(sender) => sender.dispatch(request).await,
            None => Err(ChannelError::Config(format!(
                "Unknown channel type: '{}'",
                request.channel
            ))),
        }
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        let mut channels: Vec<ChannelInfo> = self
            .builtin_senders
            .iter()
            .flat_map(|sender| sender.accounts())
            .collect();
        for plugin in self.plugin_senders.values() {
            channels.extend(OutboundChannelSender::accounts(plugin));
        }
        channels.sort_by(|a, b| (&a.channel, &a.account_id).cmp(&(&b.channel, &b.account_id)));
        channels
    }

    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        self.route(target.channel.as_str())?
            .build_stream_event_callback(target)
    }

    fn supports_legacy_stream_adapter(&self, target: &StreamingDispatchTarget) -> bool {
        self.route(target.channel.as_str())
            .map(|sender| sender.supports_legacy_stream_adapter(target))
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod characterization;
#[cfg(test)]
mod tests;
