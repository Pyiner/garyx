use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use reqwest::Client;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use garyx_bridge::MultiProviderBridge;
use garyx_models::config::{ReplyToMode, TelegramAccount};
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, PromptAttachment, attachments_to_metadata_value,
};
use garyx_models::{MessageLedgerEvent, MessageLifecycleStatus, MessageTerminalReason};
use garyx_router::{
    InboundRequest, MessageRouter, NATIVE_COMMAND_TEXT_METADATA_KEY, is_native_command_text,
};

use super::api::send_chat_action;
use super::dedup::{dedup_scope_id, is_duplicate_message};
use super::media::{extract_file_paths, extract_image_attachments};
use super::streaming::{StreamingCallbackConfig, build_response_callback};
use super::text::{safe_log_preview, strip_mention};
use super::{
    DEBOUNCE_MAX_CHARS, DEBOUNCE_MAX_FRAGMENTS, DEBOUNCE_WINDOW_MILLIS, MEDIA_GROUP_TIMEOUT_MILLIS,
    TelegramChannel, TelegramSendTarget, TgMessage, TgUpdate, build_group_thread_key,
    extract_message_content, is_mentioned, resolve_forum_thread_id, resolve_outbound_thread_id,
    resolve_reply_to, resolve_typing_thread_id, send_response,
};

#[derive(Clone)]
pub(crate) struct TelegramChannelResources<'a> {
    pub(crate) http: &'a Client,
    pub(crate) router: &'a Arc<Mutex<MessageRouter>>,
    pub(crate) bridge: &'a Arc<MultiProviderBridge>,
    pub(crate) api_base: &'a str,
}

#[derive(Clone)]
pub(crate) struct TelegramBotRuntime<'a> {
    pub(crate) account_id: &'a str,
    pub(crate) token: &'a str,
    pub(crate) bot_username: &'a str,
    pub(crate) bot_id: i64,
    pub(crate) account: &'a TelegramAccount,
}

#[derive(Clone)]
pub(crate) struct TelegramUpdateContext {
    pub(crate) http: Client,
    pub(crate) account_id: String,
    pub(crate) token: String,
    pub(crate) bot_username: String,
    pub(crate) bot_id: i64,
    pub(crate) account: TelegramAccount,
    pub(crate) router: Arc<Mutex<MessageRouter>>,
    pub(crate) bridge: Arc<MultiProviderBridge>,
    pub(crate) api_base: String,
}

impl TelegramUpdateContext {
    pub(crate) fn new(
        resources: TelegramChannelResources<'_>,
        bot: TelegramBotRuntime<'_>,
    ) -> Self {
        Self {
            http: resources.http.clone(),
            account_id: bot.account_id.to_owned(),
            token: bot.token.to_owned(),
            bot_username: bot.bot_username.to_owned(),
            bot_id: bot.bot_id,
            account: bot.account.clone(),
            router: resources.router.clone(),
            bridge: resources.bridge.clone(),
            api_base: resources.api_base.to_owned(),
        }
    }
}

struct MediaGroupEntry {
    messages: Vec<TgMessage>,
    context: TelegramUpdateContext,
    flush_task: Option<JoinHandle<()>>,
}

struct DebounceEntry {
    fragments: Vec<String>,
    base_message: TgMessage,
    context: TelegramUpdateContext,
    flush_task: Option<JoinHandle<()>>,
}

type MediaGroupBufferStore = Arc<Mutex<HashMap<String, MediaGroupEntry>>>;

fn media_group_buffers() -> &'static MediaGroupBufferStore {
    static STORE: OnceLock<MediaGroupBufferStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

type PreloadedMediaStore = Arc<Mutex<HashMap<String, Vec<PromptAttachment>>>>;

fn preloaded_media_store() -> &'static PreloadedMediaStore {
    static STORE: OnceLock<PreloadedMediaStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

type DebounceBufferStore = Arc<Mutex<HashMap<String, DebounceEntry>>>;

fn debounce_buffers() -> &'static DebounceBufferStore {
    static STORE: OnceLock<DebounceBufferStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

impl TelegramChannel {
    async fn record_inbound_terminal_event(
        context: &TelegramUpdateContext,
        msg: &TgMessage,
        status: MessageLifecycleStatus,
        terminal_reason: MessageTerminalReason,
        text_excerpt: &str,
        metadata: Value,
    ) {
        let chat_id = msg.chat.id;
        let is_group = matches!(msg.chat.chat_type.as_str(), "group" | "supergroup");
        let is_forum = msg.chat.is_forum.unwrap_or(false);
        let forum_thread_id = resolve_forum_thread_id(msg);
        let thread_id = if is_group {
            Some(build_group_thread_key(chat_id, is_forum, forum_thread_id))
        } else {
            None
        };
        let from_id = msg
            .from
            .as_ref()
            .map(|from| from.id.to_string())
            .unwrap_or_else(|| chat_id.to_string());
        let router = context.router.lock().await;
        router
            .record_message_ledger_event(MessageLedgerEvent {
                ledger_id: format!(
                    "telegram:{}:{chat_id}:{}",
                    context.account_id, msg.message_id
                ),
                bot_id: format!("telegram:{}", context.account_id),
                status,
                created_at: chrono::Utc::now().to_rfc3339(),
                thread_id,
                run_id: None,
                channel: Some("telegram".to_owned()),
                account_id: Some(context.account_id.clone()),
                chat_id: Some(chat_id.to_string()),
                from_id: Some(from_id),
                native_message_id: Some(msg.message_id.to_string()),
                text_excerpt: Some(text_excerpt.chars().take(200).collect()),
                terminal_reason: Some(terminal_reason),
                reply_message_id: None,
                metadata,
            })
            .await;
    }

    fn is_non_interrupting_native_command(text: &str) -> bool {
        is_native_command_text(text, "telegram")
    }

    fn media_group_key(
        dedup_scope_id: u64,
        account_id: &str,
        chat_id: i64,
        media_group_id: &str,
    ) -> String {
        format!("telegram::{dedup_scope_id}::{account_id}::{chat_id}::{media_group_id}")
    }

    fn preloaded_media_key(
        dedup_scope_id: u64,
        account_id: &str,
        chat_id: i64,
        message_id: i64,
    ) -> String {
        format!("telegram::{dedup_scope_id}::{account_id}::{chat_id}::{message_id}")
    }

    fn debounce_key(dedup_scope_id: u64, account_id: &str, chat_id: i64, from_id: &str) -> String {
        format!("telegram::{dedup_scope_id}::{account_id}::{chat_id}::{from_id}")
    }

    async fn buffer_debounce_text(
        key: String,
        text: String,
        context: TelegramUpdateContext,
        msg: &TgMessage,
    ) {
        let should_flush_now = {
            let mut buffers = debounce_buffers().lock().await;
            let entry = buffers.entry(key.clone()).or_insert_with(|| DebounceEntry {
                fragments: Vec::new(),
                base_message: msg.clone(),
                context: context.clone(),
                flush_task: None,
            });

            if entry.fragments.is_empty() {
                entry.base_message = msg.clone();
                entry.context = context;
            }
            entry.fragments.push(text);

            if let Some(handle) = entry.flush_task.take() {
                handle.abort();
            }

            let total_chars = entry.fragments.iter().map(|f| f.len()).sum::<usize>();
            entry.fragments.len() >= DEBOUNCE_MAX_FRAGMENTS || total_chars >= DEBOUNCE_MAX_CHARS
        };

        let key_for_task = key.clone();
        if should_flush_now {
            tokio::spawn(async move {
                TelegramChannel::flush_debounce_text(key_for_task).await;
            });
            return;
        }

        let mut buffers = debounce_buffers().lock().await;
        if let Some(entry) = buffers.get_mut(&key) {
            entry.flush_task = Some(tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(DEBOUNCE_WINDOW_MILLIS)).await;
                TelegramChannel::flush_debounce_text(key_for_task).await;
            }));
        }
    }

    async fn flush_debounce_for_key(key: String) {
        let has_pending = {
            let buffers = debounce_buffers().lock().await;
            buffers.contains_key(&key)
        };
        if has_pending {
            Self::flush_debounce_text(key).await;
        }
    }

    async fn flush_debounce_text(key: String) {
        let entry = {
            let mut buffers = debounce_buffers().lock().await;
            buffers.remove(&key)
        };
        let Some(entry) = entry else {
            return;
        };
        if entry.fragments.is_empty() {
            return;
        }

        let combined_text = entry.fragments.join("\n");
        if combined_text.trim().is_empty() {
            return;
        }

        let mut synthetic_msg = entry.base_message.clone();
        synthetic_msg.text = Some(combined_text);

        let synthetic_update = TgUpdate {
            update_id: synthetic_msg.message_id,
            message: Some(synthetic_msg),
        };

        Self::handle_update_core(&entry.context, &synthetic_update).await;
    }

    async fn buffer_media_group(
        context: TelegramUpdateContext,
        msg: &TgMessage,
        media_group_id: &str,
    ) {
        let dedup_scope = dedup_scope_id(&context.router).await;
        let key = Self::media_group_key(
            dedup_scope,
            &context.account_id,
            msg.chat.id,
            media_group_id,
        );

        let mut buffers = media_group_buffers().lock().await;
        let entry = buffers
            .entry(key.clone())
            .or_insert_with(|| MediaGroupEntry {
                messages: Vec::new(),
                context,
                flush_task: None,
            });
        entry.messages.push(msg.clone());

        if let Some(handle) = entry.flush_task.take() {
            handle.abort();
        }

        let key_for_task = key.clone();
        entry.flush_task = Some(tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(MEDIA_GROUP_TIMEOUT_MILLIS)).await;
            TelegramChannel::flush_media_group(key_for_task).await;
        }));
    }

    async fn flush_media_group(key: String) {
        let entry = {
            let mut buffers = media_group_buffers().lock().await;
            buffers.remove(&key)
        };
        let Some(entry) = entry else {
            return;
        };
        if entry.messages.is_empty() {
            return;
        }

        let mut image_attachments = Vec::new();
        let mut caption = String::new();
        for msg in &entry.messages {
            if caption.is_empty()
                && let Some(c) = msg.caption.as_deref().filter(|s| !s.trim().is_empty())
            {
                caption = c.to_owned();
            }
            let mut msg_images = extract_image_attachments(
                &entry.context.http,
                &entry.context.token,
                msg,
                &entry.context.api_base,
            )
            .await;
            image_attachments.append(&mut msg_images);
        }

        if image_attachments.is_empty() {
            debug!("media group had no supported images after download");
            return;
        }

        let mut synthetic_msg = entry.messages[0].clone();
        synthetic_msg.media_group_id = None;
        synthetic_msg.photo = None;
        synthetic_msg.document = None;
        synthetic_msg.voice = None;
        synthetic_msg.audio = None;
        synthetic_msg.video = None;
        synthetic_msg.animation = None;
        synthetic_msg.sticker = None;

        let summary = format!("[用户发送了 {} 张图片]", image_attachments.len());
        synthetic_msg.text = Some(if caption.is_empty() {
            summary
        } else {
            format!("{summary} {caption}")
        });
        synthetic_msg.caption = None;

        let dedup_scope = dedup_scope_id(&entry.context.router).await;
        let preload_key = Self::preloaded_media_key(
            dedup_scope,
            &entry.context.account_id,
            synthetic_msg.chat.id,
            synthetic_msg.message_id,
        );
        {
            let mut preloaded = preloaded_media_store().lock().await;
            preloaded.insert(preload_key, image_attachments);
        }

        let synthetic_update = TgUpdate {
            update_id: synthetic_msg.message_id,
            message: Some(synthetic_msg),
        };

        Self::handle_update_core(&entry.context, &synthetic_update).await;
    }

    /// Process a single update.
    pub(crate) async fn handle_update(context: &TelegramUpdateContext, update: &TgUpdate) {
        if let Some(msg) = &update.message {
            if msg.from.as_ref().is_some_and(|from| from.is_bot) {
                return;
            }
            let dedup_scope_id = dedup_scope_id(&context.router).await;
            if is_duplicate_message(
                dedup_scope_id,
                &context.account_id,
                &context.api_base,
                msg.chat.id,
                msg.message_id,
            )
            .await
            {
                debug!(
                    context.account_id,
                    chat_id = msg.chat.id,
                    message_id = msg.message_id,
                    "duplicate Telegram message ignored"
                );
                return;
            }

            let from_id = msg.from.as_ref().map(|f| f.id.to_string());
            let debounce_key = from_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(|id| Self::debounce_key(dedup_scope_id, &context.account_id, msg.chat.id, id));

            let is_media_message = msg.photo.is_some()
                || msg.document.is_some()
                || msg.voice.is_some()
                || msg.audio.is_some()
                || msg.video.is_some()
                || msg.animation.is_some()
                || msg.sticker.is_some();

            // Media handlers bypass debounce and flush buffered text first.
            if is_media_message && let Some(key) = debounce_key.clone() {
                Self::flush_debounce_for_key(key).await;
            }

            // Buffer rapid text messages and process as a merged message.
            if !is_media_message
                && let Some(text) = msg.text.as_deref()
                && !text.starts_with('/')
                && let Some(key) = debounce_key
            {
                Self::buffer_debounce_text(key, text.to_owned(), context.clone(), msg).await;
                return;
            }

            if let Some(media_group_id) = msg.media_group_id.as_deref()
                && (msg.photo.is_some() || msg.document.is_some())
            {
                Self::buffer_media_group(context.clone(), msg, media_group_id).await;
                return;
            }
        }

        Self::handle_update_core(context, update).await;
    }

    async fn handle_update_core(context: &TelegramUpdateContext, update: &TgUpdate) {
        let msg = match &update.message {
            Some(m) => m,
            None => return,
        };

        // Skip messages from bots (including ourselves)
        if let Some(from) = &msg.from
            && from.is_bot
        {
            return;
        }

        // Extract text content: text for text messages, caption for media, or media type description
        let (raw_text, mut media_type) = extract_message_content(msg);
        let raw_text = match raw_text {
            Some(t) => t,
            None => return, // No content to process
        };

        let chat_id = msg.chat.id;
        let is_group = matches!(msg.chat.chat_type.as_str(), "group" | "supergroup");
        let is_forum = msg.chat.is_forum.unwrap_or(false);
        let forum_thread_id = resolve_forum_thread_id(msg);
        let typing_thread_id = resolve_typing_thread_id(is_forum, forum_thread_id);
        let outbound_thread_id = resolve_outbound_thread_id(is_forum, forum_thread_id);
        let group_thread_key = build_group_thread_key(chat_id, is_forum, forum_thread_id);
        let mut effective_system_prompt: Option<String> = None;

        // -----------------------------------------------------------------
        // Per-group/per-topic policy enforcement
        // -----------------------------------------------------------------
        if is_group {
            let chat_id_str = chat_id.to_string();
            let group_config = context.account.groups.get(&chat_id_str);

            // Check group enabled
            if let Some(gc) = group_config
                && !gc.enabled
            {
                debug!(context.account_id, chat_id, "skipping disabled group");
                Self::record_inbound_terminal_event(
                    context,
                    msg,
                    MessageLifecycleStatus::Filtered,
                    MessageTerminalReason::PolicyFiltered,
                    &raw_text,
                    serde_json::json!({
                        "source": "telegram_inbound",
                        "reason": "group_disabled",
                    }),
                )
                .await;
                return;
            }

            // Check topic enabled (if forum)
            let topic_config = if is_forum {
                forum_thread_id
                    .and_then(|tid| group_config.and_then(|gc| gc.topics.get(&tid.to_string())))
            } else {
                None
            };
            effective_system_prompt = topic_config
                .and_then(|tc| tc.system_prompt.clone())
                .or_else(|| group_config.and_then(|gc| gc.system_prompt.clone()));

            if let Some(tc) = topic_config
                && !tc.enabled
            {
                debug!(context.account_id, chat_id, "skipping disabled topic");
                Self::record_inbound_terminal_event(
                    context,
                    msg,
                    MessageLifecycleStatus::Filtered,
                    MessageTerminalReason::PolicyFiltered,
                    &raw_text,
                    serde_json::json!({
                        "source": "telegram_inbound",
                        "reason": "topic_disabled",
                    }),
                )
                .await;
                return;
            }

            // Resolve effective allow_from (topic overrides group)
            let effective_allow_from: Option<&Vec<String>> = topic_config
                .and_then(|tc| tc.allow_from.as_ref())
                .or_else(|| group_config.and_then(|gc| gc.allow_from.as_ref()));

            if let Some(allowlist) = effective_allow_from
                && let Some(from) = &msg.from
            {
                let user_id_str = from.id.to_string();
                let username_str = from.username.as_deref().unwrap_or("");
                if !allowlist
                    .iter()
                    .any(|a| a == &user_id_str || a == username_str)
                {
                    debug!(
                        context.account_id,
                        user_id = %from.id,
                        "user not in group/topic allow_from"
                    );
                    Self::record_inbound_terminal_event(
                        context,
                        msg,
                        MessageLifecycleStatus::Filtered,
                        MessageTerminalReason::PolicyFiltered,
                        &raw_text,
                        serde_json::json!({
                            "source": "telegram_inbound",
                            "reason": "group_allow_from",
                        }),
                    )
                    .await;
                    return;
                }
            }

            // Resolve effective require_mention (topic overrides group)
            let effective_require_mention = topic_config
                .and_then(|tc| tc.require_mention)
                .unwrap_or_else(|| group_config.is_none_or(|gc| gc.require_mention));

            if effective_require_mention
                && !is_mentioned(&raw_text, &context.bot_username, context.bot_id, msg)
            {
                debug!(context.account_id, "skipping group message (no mention)");
                Self::record_inbound_terminal_event(
                    context,
                    msg,
                    MessageLifecycleStatus::Filtered,
                    MessageTerminalReason::RoutingRejected,
                    &raw_text,
                    serde_json::json!({
                        "source": "telegram_inbound",
                        "reason": "mention_required",
                    }),
                )
                .await;
                return;
            }
        }

        // Strip @mention from text
        let clean_text = strip_mention(&raw_text, &context.bot_username);

        // Send typing indicator
        if let Err(e) = send_chat_action(
            &context.http,
            &context.token,
            chat_id,
            "typing",
            typing_thread_id,
            &context.api_base,
        )
        .await
        {
            debug!(chat_id, error = %e, "failed to send typing indicator");
        }

        // Download supported media payloads (photos / image-documents) so
        // provider-side vision input can match Python behavior.
        let mut image_attachments =
            extract_image_attachments(&context.http, &context.token, msg, &context.api_base).await;
        let dedup_scope = dedup_scope_id(&context.router).await;
        let preload_key =
            Self::preloaded_media_key(dedup_scope, &context.account_id, chat_id, msg.message_id);
        if let Some(mut buffered_images) = preloaded_media_store().lock().await.remove(&preload_key)
        {
            if media_type.is_none() {
                media_type = Some("photo".to_owned());
            }
            image_attachments.append(&mut buffered_images);
        }
        // Download non-image file attachments (documents, voice, audio, video)
        // to local disk so the agent thread can reference them by path.
        let file_paths =
            extract_file_paths(&context.http, &context.token, msg, &context.api_base).await;

        // Prepend quoted (reply-to) message context so the agent can see what
        // the user is replying to.
        let reply_context = msg.reply_to_message.as_ref().and_then(|reply| {
            let (reply_text, _) = extract_message_content(reply);
            reply_text.map(|t| {
                let preview = if t.len() > 500 {
                    format!("{}…", super::text::safe_log_preview(&t, 500))
                } else {
                    t
                };
                format!("[引用消息]\n{preview}\n---\n")
            })
        });

        let dispatch_message = if !image_attachments.is_empty()
            && (clean_text.trim().is_empty()
                || clean_text.trim() == "[photo]"
                || clean_text.trim().starts_with("[document:"))
        {
            "请描述这张图片。".to_owned()
        } else {
            match reply_context {
                Some(ctx) => format!("{ctx}{clean_text}"),
                None => clean_text.clone(),
            }
        };

        let from_id = msg
            .from
            .as_ref()
            .map(|f| f.id.to_string())
            .unwrap_or_default();

        let text_preview = safe_log_preview(&clean_text, 50);
        info!(
            context.account_id,
            chat_id,
            from_id = %from_id,
            text = text_preview,
            "received message"
        );

        // Once user sends a new message, stop editing any previous in-flight
        // assistant stream on this endpoint. The next response should start as
        // a new Telegram bubble after the user message.
        let existing_thread_id = {
            let mut router_guard = context.router.lock().await;
            let thread_binding_key = if is_group {
                group_thread_key.as_str()
            } else {
                from_id.as_str()
            };
            router_guard
                .resolve_endpoint_thread_id("telegram", &context.account_id, thread_binding_key)
                .await
        };
        // Try to append the message into the already-running Claude session
        // via streaming input instead of interrupting + starting a new run.
        // This preserves full conversation context when the user sends
        // follow-up messages while the agent is still working.
        if !Self::is_non_interrupting_native_command(&clean_text)
            && let Some(thread_id) = existing_thread_id.as_deref()
        {
            let queued = context
                .bridge
                .add_streaming_input(
                    thread_id,
                    &dispatch_message,
                    None,
                    None,
                    Some(image_attachments.clone()),
                )
                .await;
            if queued.is_some() {
                debug!(
                    context.account_id,
                    chat_id,
                    thread_id,
                    "telegram message queued as streaming input into active session"
                );
                return;
            }
            // No active session — fall through to normal dispatch.
        }

        // Dispatch to bridge
        let run_id = uuid::Uuid::new_v4().to_string();

        // Telegram account config no longer exposes reply-to tuning; keep the
        // established default behavior.
        let reply_to_mode = ReplyToMode::default();
        let reply_to = resolve_reply_to(&reply_to_mode, msg.message_id, true);

        // Build metadata for the run
        let mut metadata = HashMap::new();
        metadata.insert("channel".to_owned(), Value::String("telegram".to_owned()));
        metadata.insert(
            "account_id".to_owned(),
            Value::String(context.account_id.to_owned()),
        );
        metadata.insert("chat_id".to_owned(), Value::Number(chat_id.into()));
        metadata.insert("from_id".to_owned(), Value::String(from_id.clone()));
        metadata.insert(
            NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
            Value::String(clean_text.clone()),
        );
        if !image_attachments.is_empty() {
            metadata.insert(
                "image_count".to_owned(),
                Value::Number((image_attachments.len() as u64).into()),
            );
            metadata.insert(
                ATTACHMENTS_METADATA_KEY.to_owned(),
                attachments_to_metadata_value(&image_attachments),
            );
        }
        if is_group {
            metadata.insert("is_group".to_owned(), Value::Bool(true));
        }
        if let Some(mt) = &media_type {
            metadata.insert("media_type".to_owned(), Value::String(mt.clone()));
        }
        if let Some(prompt) = effective_system_prompt {
            metadata.insert("system_prompt".to_owned(), Value::String(prompt));
        }
        match outbound_thread_id {
            Some(thread_id) => {
                metadata.insert(
                    "delivery_thread_id".to_owned(),
                    Value::String(thread_id.to_string()),
                );
            }
            None => {
                metadata.insert("delivery_thread_id".to_owned(), Value::Null);
            }
        }

        let (response_callback, session_key_tx) =
            build_response_callback(StreamingCallbackConfig {
                http: context.http.clone(),
                token: context.token.to_owned(),
                router: context.router.clone(),
                account_id: context.account_id.to_owned(),
                chat_id,
                api_base: context.api_base.to_owned(),
                reply_to_mode,
                reply_to,
                outbound_thread_id,
                outbound_thread_scope: if is_group {
                    Some(group_thread_key.clone())
                } else {
                    None
                },
            });

        let request = InboundRequest {
            channel: "telegram".to_owned(),
            account_id: context.account_id.to_owned(),
            from_id: from_id.clone(),
            is_group,
            thread_binding_key: if is_group {
                group_thread_key.clone()
            } else {
                from_id.clone()
            },
            message: dispatch_message,
            run_id,
            reply_to_message_id: msg
                .reply_to_message
                .as_ref()
                .map(|reply| reply.message_id.to_string()),
            images: Vec::new(),
            extra_metadata: metadata,
            file_paths,
        };

        let dispatch_result = {
            let mut router_guard = context.router.lock().await;
            router_guard
                .record_message_ledger_event(MessageLedgerEvent {
                    ledger_id: format!(
                        "telegram:{}:{chat_id}:{}",
                        context.account_id, msg.message_id
                    ),
                    bot_id: format!("telegram:{}", context.account_id),
                    status: MessageLifecycleStatus::Received,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    thread_id: Some(request.thread_binding_key.clone()),
                    run_id: Some(request.run_id.clone()),
                    channel: Some(request.channel.clone()),
                    account_id: Some(request.account_id.clone()),
                    chat_id: Some(chat_id.to_string()),
                    from_id: Some(request.from_id.clone()),
                    native_message_id: Some(msg.message_id.to_string()),
                    text_excerpt: Some(request.message.chars().take(200).collect()),
                    terminal_reason: None,
                    reply_message_id: None,
                    metadata: serde_json::json!({
                        "source": "telegram_inbound",
                        "is_group": is_group,
                    }),
                })
                .await;
            router_guard
                .route_and_dispatch(request, context.bridge.as_ref(), Some(response_callback))
                .await
        };

        match dispatch_result {
            Ok(result) => {
                if let Some(local_reply) = result.local_reply {
                    if let Err(e) = send_response(
                        TelegramSendTarget::new(
                            &context.http,
                            &context.token,
                            chat_id,
                            outbound_thread_id,
                            &context.api_base,
                        ),
                        &local_reply,
                        reply_to,
                    )
                    .await
                    {
                        warn!(chat_id, error = %e, "failed to send local reply to telegram");
                    }
                    info!(
                        context.account_id,
                        chat_id,
                        session_key = %result.thread_id,
                        "native command handled by router"
                    );
                } else {
                    let _ = session_key_tx.send(result.thread_id.clone());
                    info!(
                        context.account_id,
                        chat_id,
                        session_key = %result.thread_id,
                        "resolved thread for message"
                    );
                }
            }
            Err(e) => {
                error!(
                    context.account_id,
                    chat_id,
                    error = %e,
                    "failed to route+dispatch message"
                );
                if let Err(send_err) = send_response(
                    TelegramSendTarget::new(
                        &context.http,
                        &context.token,
                        chat_id,
                        outbound_thread_id,
                        &context.api_base,
                    ),
                    &format!("Error: {e}"),
                    reply_to,
                )
                .await
                {
                    warn!(chat_id, error = %send_err, "failed to send error reply to telegram");
                }
            }
        }
    }
}
