//! Discord outbound senders: per-account [`DiscordSender`], the
//! registry-facing [`DiscordChannelSender`], REST retry/error
//! helpers. Moved verbatim from dispatcher.rs (Phase-6 B2b-2 pure
//! code motion).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode, header, multipart};
use serde_json::Value;
use tracing::warn;

use crate::channel_trait::ChannelError;
use crate::dispatcher::{
    ChannelInfo, OutboundChannelSender, OutboundMessage, OutboundSender, SendMessageResult,
    StreamDispatchCallback, StreamingDispatchTarget,
};

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

#[derive(Clone, Default)]
pub struct DiscordChannelSender {
    accounts: HashMap<String, DiscordSender>,
}

impl DiscordChannelSender {
    pub(crate) fn register(&mut self, sender: DiscordSender) {
        self.accounts.insert(sender.account_id.clone(), sender);
    }
}

#[async_trait]
impl OutboundChannelSender for DiscordChannelSender {
    fn clone_box(&self) -> Box<dyn OutboundChannelSender> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn channel_id(&self) -> &str {
        "discord"
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
                "Discord account '{}' not registered in dispatcher",
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
        let (stream_callback, thread_id_tx) = crate::discord::build_discord_response_callback(
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
}
