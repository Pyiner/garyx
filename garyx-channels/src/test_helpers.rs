//! Shared test utilities for channel E2E tests.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use garyx_bridge::provider_trait::StreamCallback;
use garyx_bridge::{AgentLoopProvider, BridgeError, MultiProviderBridge};
use garyx_models::ThreadHistoryBackend;
use garyx_models::config::GaryxConfig;
use garyx_models::provider::{
    PromptAttachment, PromptAttachmentKind, ProviderRunOptions, ProviderRunResult, ProviderType,
    StreamEvent, attachments_from_metadata,
};
use garyx_router::{
    InMemoryThreadStore, MessageRouter, ThreadHistoryRepository, ThreadStore, ThreadTranscriptStore,
};
use serde_json::Value;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// ConfigurableTestProvider
// ---------------------------------------------------------------------------

/// Tracks a single provider call.
#[derive(Debug, Clone)]
pub struct ProviderCall {
    pub thread_id: String,
    pub message: String,
    pub image_count: usize,
    pub attachments: Vec<PromptAttachment>,
    pub metadata: std::collections::HashMap<String, Value>,
}

/// Configurable mock provider for E2E tests.
///
/// Supports custom response functions, call tracking, and optional delays.
pub struct ConfigurableTestProvider {
    pub call_count: AtomicUsize,
    pub calls: std::sync::Mutex<Vec<ProviderCall>>,
    response_fn: Box<dyn Fn(&str) -> String + Send + Sync>,
}

impl ConfigurableTestProvider {
    /// Create an echo provider: responds with `"echo: {message}"`.
    pub fn echo() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            calls: std::sync::Mutex::new(Vec::new()),
            response_fn: Box::new(|msg| format!("echo: {}", msg)),
        }
    }

    /// Create a provider with a custom response function.
    pub fn with_response(f: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            calls: std::sync::Mutex::new(Vec::new()),
            response_fn: Box::new(f),
        }
    }
}

pub async fn wait_for_provider_calls(provider: &ConfigurableTestProvider, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if provider.call_count.load(Ordering::Relaxed) >= expected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("provider call count should reach expected value");
}

pub async fn wait_for_thread_delivery_persistence(store: &Arc<dyn ThreadStore>, thread_id: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let persisted = store.get(thread_id).await.is_some_and(|value| {
                let has_outbound = value
                    .get("outbound_message_ids")
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty());
                let has_delivery = value
                    .get("delivery_context")
                    .or_else(|| value.get("last_delivery"))
                    .and_then(Value::as_object)
                    .is_some_and(|delivery| delivery.get("chat_id").is_some());
                has_outbound && has_delivery
            });
            if persisted {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("thread delivery persistence should complete");
}

#[async_trait]
impl AgentLoopProvider for ConfigurableTestProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let attachments = attachments_from_metadata(&options.metadata);
        self.calls.lock().unwrap().push(ProviderCall {
            thread_id: options.thread_id.clone(),
            message: options.message.clone(),
            image_count: options.images.as_ref().map_or(0, Vec::len)
                + attachments
                    .iter()
                    .filter(|attachment| attachment.kind == PromptAttachmentKind::Image)
                    .count(),
            attachments,
            metadata: options.metadata.clone(),
        });

        let response = (self.response_fn)(&options.message);
        on_chunk(StreamEvent::Delta {
            text: response.clone(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "test-run".into(),
            thread_id: options.thread_id.clone(),
            response,
            session_messages: Vec::new(),
            sdk_session_id: None,
            actual_model: None,
            success: true,
            error: None,
            input_tokens: 10,
            output_tokens: 5,
            cost: 0.001,
            duration_ms: 42,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

// ---------------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------------

/// Create a router with in-memory thread store.
pub fn make_router() -> Arc<Mutex<MessageRouter>> {
    let store = Arc::new(InMemoryThreadStore::new());
    let config = GaryxConfig::default();
    Arc::new(Mutex::new(MessageRouter::new(store, config)))
}

/// Create a bridge with a given provider registered as default.
pub async fn make_bridge_with(provider: Arc<dyn AgentLoopProvider>) -> Arc<MultiProviderBridge> {
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge.register_provider("test-provider", provider).await;
    bridge.set_default_provider_key("test-provider").await;
    bridge
}

/// Create a bridge with thread store attached (for persistence tests).
pub async fn make_bridge_with_store(
    provider: Arc<dyn AgentLoopProvider>,
    store: Arc<dyn ThreadStore>,
) -> Arc<MultiProviderBridge> {
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge.register_provider("test-provider", provider).await;
    bridge.set_default_provider_key("test-provider").await;
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(Arc::new(ThreadHistoryRepository::new(
        store,
        Arc::new(ThreadTranscriptStore::memory()),
        ThreadHistoryBackend::TranscriptV1,
    )));
    bridge
}

// ---------------------------------------------------------------------------
// TgUpdateBuilder
// ---------------------------------------------------------------------------

use crate::telegram::{
    TgAnimation, TgAudio, TgChat, TgDocument, TgMessage, TgMessageEntity, TgPhotoSize, TgSticker,
    TgUpdate, TgUser, TgVideo, TgVoice,
};

/// Builder for constructing `TgUpdate` payloads in tests.
pub struct TgUpdateBuilder {
    update_id: i64,
    message_id: i64,
    chat_id: i64,
    chat_type: String,
    from_id: i64,
    from_username: Option<String>,
    text: Option<String>,
    caption: Option<String>,
    media_group_id: Option<String>,
    message_thread_id: Option<i64>,
    reply_to_message: Option<Box<TgMessage>>,
    entities: Option<Vec<TgMessageEntity>>,
    is_forum: Option<bool>,
    photo: Option<Vec<TgPhotoSize>>,
    voice: Option<TgVoice>,
    audio: Option<TgAudio>,
    document: Option<TgDocument>,
    video: Option<TgVideo>,
    animation: Option<TgAnimation>,
    sticker: Option<TgSticker>,
}

impl TgUpdateBuilder {
    /// Create a DM update from a user.
    pub fn dm(from_id: i64, text: &str) -> Self {
        Self {
            update_id: 1,
            message_id: 100,
            chat_id: from_id, // In DMs, chat_id == user_id
            chat_type: "private".to_string(),
            from_id,
            from_username: None,
            text: Some(text.to_string()),
            caption: None,
            media_group_id: None,
            message_thread_id: None,
            reply_to_message: None,
            entities: None,
            is_forum: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        }
    }

    /// Create a group message update.
    pub fn group(from_id: i64, chat_id: i64, text: &str) -> Self {
        Self {
            update_id: 1,
            message_id: 100,
            chat_id,
            chat_type: "supergroup".to_string(),
            from_id,
            from_username: None,
            text: Some(text.to_string()),
            caption: None,
            media_group_id: None,
            message_thread_id: None,
            reply_to_message: None,
            entities: None,
            is_forum: None,
            photo: None,
            voice: None,
            audio: None,
            document: None,
            video: None,
            animation: None,
            sticker: None,
        }
    }

    /// Create a photo message update.
    pub fn photo(from_id: i64, chat_id: i64, caption: Option<&str>) -> Self {
        let mut builder = Self::group(from_id, chat_id, "");
        builder.text = None;
        builder.caption = caption.map(|c| c.to_string());
        builder.photo = Some(vec![TgPhotoSize {
            file_id: "photo_file_id".to_string(),
            width: 800,
            height: 600,
        }]);
        builder
    }

    /// Create a voice message update.
    pub fn voice(from_id: i64, chat_id: i64) -> Self {
        let mut builder = Self::group(from_id, chat_id, "");
        builder.text = None;
        builder.voice = Some(TgVoice {
            file_id: "voice_file_id".to_string(),
            duration: 5,
        });
        builder
    }

    /// Create a document message update.
    pub fn document(from_id: i64, chat_id: i64, file_name: &str) -> Self {
        let mut builder = Self::group(from_id, chat_id, "");
        builder.text = None;
        builder.document = Some(TgDocument {
            file_id: "doc_file_id".to_string(),
            file_name: Some(file_name.to_string()),
            mime_type: None,
        });
        builder
    }

    /// Create a sticker message update.
    pub fn sticker(from_id: i64, chat_id: i64, emoji: &str) -> Self {
        let mut builder = Self::group(from_id, chat_id, "");
        builder.text = None;
        builder.sticker = Some(TgSticker {
            file_id: "sticker_file_id".to_string(),
            emoji: Some(emoji.to_string()),
            is_animated: false,
            is_video: false,
        });
        builder
    }

    pub fn with_message_id(mut self, id: i64) -> Self {
        self.message_id = id;
        self
    }

    /// Add a @mention entity for the bot.
    pub fn with_mention(mut self, bot_username: &str) -> Self {
        let mention = format!("@{}", bot_username);
        // Prepend mention to text
        let current_text = self.text.take().unwrap_or_default();
        let new_text = format!("{} {}", mention, current_text);
        let entity = TgMessageEntity {
            entity_type: "mention".to_string(),
            offset: 0,
            length: mention.len(),
        };
        self.text = Some(new_text);
        self.entities = Some(vec![entity]);
        self
    }

    /// Set the reply_to_message (for reply routing tests).
    pub fn with_reply_to(mut self, reply_msg: TgMessage) -> Self {
        self.reply_to_message = Some(Box::new(reply_msg));
        self
    }

    /// Set the thread_id (for forum/topic groups).
    pub fn with_thread_id(mut self, thread_id: i64) -> Self {
        self.message_thread_id = Some(thread_id);
        self
    }

    /// Mark message as part of a Telegram media group (album).
    pub fn with_media_group_id(mut self, media_group_id: &str) -> Self {
        self.media_group_id = Some(media_group_id.to_string());
        self
    }

    /// Mark the chat as a forum.
    pub fn with_forum(mut self) -> Self {
        self.is_forum = Some(true);
        self
    }

    /// Set the chat type to private (for DM callback queries).
    pub fn with_private_chat(mut self) -> Self {
        self.chat_type = "private".to_string();
        self
    }

    /// Set from_username.
    pub fn with_username(mut self, username: &str) -> Self {
        self.from_username = Some(username.to_string());
        self
    }

    /// Build the `TgUpdate`.
    pub fn build(self) -> TgUpdate {
        TgUpdate {
            update_id: self.update_id,
            message: Some(TgMessage {
                message_id: self.message_id,
                chat: TgChat {
                    id: self.chat_id,
                    chat_type: self.chat_type,
                    title: None,
                    is_forum: self.is_forum,
                },
                from: Some(TgUser {
                    id: self.from_id,
                    is_bot: false,
                    first_name: "TestUser".to_string(),
                    last_name: None,
                    username: self.from_username,
                }),
                text: self.text,
                caption: self.caption,
                media_group_id: self.media_group_id,
                date: 1700000000,
                message_thread_id: self.message_thread_id,
                reply_to_message: self.reply_to_message,
                entities: self.entities,
                photo: self.photo,
                voice: self.voice,
                audio: self.audio,
                document: self.document,
                video: self.video,
                animation: self.animation,
                sticker: self.sticker,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// FeishuEventBuilder
// ---------------------------------------------------------------------------

use crate::feishu::{
    ImMention, ImMentionId, ImMessage, ImMessageReceiveEvent, ImSender, ImSenderId,
};

/// Builder for constructing `ImMessageReceiveEvent` payloads in tests.
pub struct FeishuEventBuilder {
    chat_id: String,
    chat_type: String,
    message_id: String,
    message_type: String,
    sender_open_id: String,
    sender_type: String,
    text: String,
    parent_id: String,
    root_id: String,
    mentions: Vec<ImMention>,
    raw_content: Option<String>,
}

impl FeishuEventBuilder {
    /// Create a DM event.
    pub fn dm(sender_open_id: &str, text: &str) -> Self {
        Self {
            chat_id: format!("oc_dm_{}", sender_open_id),
            chat_type: "p2p".to_string(),
            message_id: "om_test_msg_001".to_string(),
            message_type: "text".to_string(),
            sender_open_id: sender_open_id.to_string(),
            sender_type: "user".to_string(),
            text: text.to_string(),
            parent_id: String::new(),
            root_id: String::new(),
            mentions: Vec::new(),
            raw_content: None,
        }
    }

    /// Create a group event.
    pub fn group(sender_open_id: &str, chat_id: &str, text: &str) -> Self {
        Self {
            chat_id: chat_id.to_string(),
            chat_type: "group".to_string(),
            message_id: "om_test_msg_001".to_string(),
            message_type: "text".to_string(),
            sender_open_id: sender_open_id.to_string(),
            sender_type: "user".to_string(),
            text: text.to_string(),
            parent_id: String::new(),
            root_id: String::new(),
            mentions: Vec::new(),
            raw_content: None,
        }
    }

    pub fn file_dm(sender_open_id: &str, file_key: &str, file_name: &str) -> Self {
        Self {
            chat_id: format!("oc_dm_{}", sender_open_id),
            chat_type: "p2p".to_string(),
            message_id: "om_test_file_msg_001".to_string(),
            message_type: "file".to_string(),
            sender_open_id: sender_open_id.to_string(),
            sender_type: "user".to_string(),
            text: String::new(),
            parent_id: String::new(),
            root_id: String::new(),
            mentions: Vec::new(),
            raw_content: Some(
                serde_json::json!({
                    "file_key": file_key,
                    "file_name": file_name,
                })
                .to_string(),
            ),
        }
    }

    pub fn with_message_id(mut self, id: &str) -> Self {
        self.message_id = id.to_string();
        self
    }

    pub fn with_parent_id(mut self, parent_id: &str) -> Self {
        self.parent_id = parent_id.to_string();
        self
    }

    pub fn with_root_id(mut self, root_id: &str) -> Self {
        self.root_id = root_id.to_string();
        self
    }

    pub fn with_mention(mut self, key: &str, name: &str, open_id: &str) -> Self {
        self.mentions.push(ImMention {
            key: key.to_string(),
            name: name.to_string(),
            id: Some(ImMentionId {
                open_id: open_id.to_string(),
            }),
        });
        self
    }

    /// Build the `ImMessageReceiveEvent`.
    pub fn build(self) -> ImMessageReceiveEvent {
        // Feishu message content is JSON-encoded
        let content = self
            .raw_content
            .unwrap_or_else(|| serde_json::json!({"text": self.text}).to_string());

        ImMessageReceiveEvent {
            message: Some(ImMessage {
                chat_id: self.chat_id,
                chat_type: self.chat_type,
                message_id: self.message_id,
                message_type: self.message_type,
                content,
                mentions: self.mentions,
                parent_id: self.parent_id,
                root_id: self.root_id,
            }),
            sender: Some(ImSender {
                sender_id: Some(ImSenderId {
                    open_id: self.sender_open_id,
                }),
                sender_type: self.sender_type,
            }),
        }
    }
}
