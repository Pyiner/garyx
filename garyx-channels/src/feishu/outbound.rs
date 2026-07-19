//! Feishu outbound senders: per-account [`FeishuSender`], the
//! registry-facing [`FeishuChannelSender`], and reply-target
//! helpers. Moved verbatim from dispatcher.rs (Phase-6 B2b-2 pure
//! code motion).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::channel_trait::ChannelError;
use crate::dispatcher::FeishuChatSummary;
use crate::dispatcher::{
    ChannelInfo, OutboundChannelSender, OutboundMessage, OutboundSender, SendMessageResult,
    StreamDispatchCallback, StreamingDispatchTarget,
};

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

    pub(crate) async fn get_access_token(&self) -> Result<String, ChannelError> {
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

#[derive(Clone, Default)]
pub struct FeishuChannelSender {
    accounts: HashMap<String, FeishuSender>,
}

impl FeishuChannelSender {
    pub(crate) fn register(&mut self, sender: FeishuSender) {
        self.accounts.insert(sender.account_id.clone(), sender);
    }
}

#[async_trait]
impl OutboundChannelSender for FeishuChannelSender {
    fn channel_id(&self) -> &str {
        "feishu"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["lark"]
    }

    fn accounts(&self) -> Vec<ChannelInfo> {
        self.accounts
            .values()
            .map(|sender| ChannelInfo {
                channel: self.channel_id().to_owned(),
                account_id: sender.account_id.clone(),
                is_running: sender.is_running,
            })
            .collect()
    }

    async fn dispatch(&self, request: OutboundMessage) -> Result<SendMessageResult, ChannelError> {
        let sender = self.accounts.get(&request.account_id).ok_or_else(|| {
            ChannelError::Config(format!(
                "Feishu account '{}' not registered in dispatcher",
                request.account_id
            ))
        })?;
        sender.send_outbound(request).await
    }

    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        let sender = self.accounts.get(&target.account_id)?;
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

impl crate::outbound_registry::AccountRegistration for FeishuSender {
    type Host = FeishuChannelSender;

    fn register_into(self, host: &mut FeishuChannelSender) {
        host.register(self);
    }
}
