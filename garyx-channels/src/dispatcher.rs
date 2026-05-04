//! ChannelDispatcher — outbound message delivery to channels.
//!
//! Allows any component (MCP tools, cron jobs, API endpoints) to send
//! messages OUT through channel transports (Telegram, Feishu) without needing
//! direct access to channel internals.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use garyx_models::ChannelOutboundContent;
use garyx_models::provider::StreamEvent;
use garyx_models::routing::{infer_delivery_target_id, infer_delivery_target_type};
use garyx_router::MessageRouter;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use garyx_models::config::{ChannelsConfig, FeishuDomain};

use crate::channel_trait::ChannelError;
use crate::plugin_host::{DispatchOutbound, PluginSenderHandle};
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
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    pub thread_id: Option<String>,
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

    fn build_streaming_callback(
        &self,
        _target: StreamingDispatchTarget,
        _router: Arc<Mutex<MessageRouter>>,
    ) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Channel-blind outbound sender trait
// ---------------------------------------------------------------------------

/// A clone-cheap, per-account outbound sender. Every built-in
/// channel's sender handle (`TelegramSender`, `FeishuSender`,
/// `WeixinSender`) implements this so [`crate::plugin::ChannelPlugin::dispatch_outbound`]
/// can delegate without a `match channel` branch.
///
/// The trait's `send_outbound` method owns every channel-specific
/// wire quirk: Telegram's integer id parsing, Feishu's reply-target
/// resolution, Weixin's `context_token` retry + queue-on-failure.
/// `dispatcher.rs::send_message` progressively migrates away from
/// its hand-written branches towards calling this method through
/// the `ChannelPlugin` trait.
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
            self.send_text(chat_id, text, reply_to, thread_id).await?
        } else if let Some((image_path, _alt)) = request.image_content() {
            self.send_image(chat_id, Path::new(image_path), None, reply_to, thread_id)
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
    &["telegram", "feishu", "lark", "weixin", "wechat"];

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
    feishu_senders: HashMap<String, FeishuSender>,
    weixin_senders: HashMap<String, WeixinSender>,
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
            feishu_senders: HashMap::new(),
            weixin_senders: HashMap::new(),
            plugin_senders: HashMap::new(),
        }
    }

    /// Build a dispatcher from the channels configuration.
    ///
    /// Registers senders for all enabled accounts so they can be used for
    /// outbound delivery even though the channels themselves are started
    /// separately via the plugin manager.
    pub fn from_config(channels: &ChannelsConfig) -> Self {
        let mut dispatcher = Self::new();
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        let telegram = channels.resolved_telegram_config().unwrap_or_else(|error| {
            warn!(error = %error, "failed to resolve telegram plugin config");
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
            });
        }

        dispatcher
    }

    pub fn register_telegram(&mut self, sender: TelegramSender) {
        info!(
            account_id = %sender.account_id,
            "Registered Telegram sender for dispatch"
        );
        self.telegram_senders
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

    pub fn register_weixin(&mut self, sender: WeixinSender) {
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

    fn build_streaming_callback(
        &self,
        target: StreamingDispatchTarget,
        router: Arc<Mutex<MessageRouter>>,
    ) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
        match target.channel.as_str() {
            "telegram" => {
                let sender = self.telegram_senders.get(&target.account_id)?;
                let chat_id = parse_telegram_id("chat_id", &target.chat_id).ok()?;
                let outbound_thread_id =
                    normalize_telegram_thread_id(chat_id, target.thread_id.as_deref());

                Some(crate::telegram::build_bound_response_callback(
                    crate::telegram::StreamingCallbackConfig {
                        http: sender.http.clone(),
                        token: sender.token.clone(),
                        router,
                        account_id: sender.account_id.clone(),
                        chat_id,
                        api_base: sender.api_base.clone(),
                        reply_to_mode: garyx_models::config::ReplyToMode::Off,
                        reply_to: None,
                        outbound_thread_id,
                        outbound_thread_scope: target.thread_id,
                    },
                    target.target_thread_id,
                ))
            }
            _ => None,
        }
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

    fn build_streaming_callback(
        &self,
        target: StreamingDispatchTarget,
        router: Arc<Mutex<MessageRouter>>,
    ) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
        self.inner.load().build_streaming_callback(target, router)
    }
}

/// A clonable handle that can send Weixin messages without owning the channel.
#[derive(Clone)]
pub struct WeixinSender {
    pub account_id: String,
    pub account: garyx_models::config::WeixinAccount,
    pub http: Client,
    pub is_running: bool,
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
