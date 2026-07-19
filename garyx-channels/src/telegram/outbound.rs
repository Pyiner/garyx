//! Telegram outbound senders: per-account [`TelegramSender`], the
//! registry-facing [`TelegramChannelSender`], and telegram-specific
//! outbound helpers. Moved verbatim from dispatcher.rs (Phase-6
//! B2b-2 pure code motion).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;

use crate::channel_trait::ChannelError;
use crate::dispatcher::{
    ChannelInfo, OutboundChannelSender, OutboundMessage, OutboundSender, SendMessageResult,
    StreamDispatchCallback, StreamingDispatchTarget,
};

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

#[derive(Clone, Default)]
pub struct TelegramChannelSender {
    accounts: HashMap<String, TelegramSender>,
}

impl TelegramChannelSender {
    pub(crate) fn register(&mut self, sender: TelegramSender) {
        self.accounts.insert(sender.account_id.clone(), sender);
    }
}

#[async_trait]
impl OutboundChannelSender for TelegramChannelSender {
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
        "telegram"
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
                "Telegram account '{}' not registered in dispatcher",
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
        let chat_id = parse_telegram_id("chat_id", &target.chat_id).ok()?;
        let outbound_thread_id = normalize_telegram_thread_id(chat_id, target.thread_id.as_deref());

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

pub(crate) fn normalize_telegram_thread_id(
    chat_id: i64,
    raw_thread_id: Option<&str>,
) -> Option<i64> {
    let parsed = raw_thread_id.and_then(|raw| raw.trim().parse::<i64>().ok())?;
    (parsed != chat_id).then_some(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TelegramMarkdownImageRef {
    pub(crate) path: String,
    pub(crate) caption: Option<String>,
}

fn telegram_markdown_link_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"!?\[([^\]]*)\]\(([^)]+)\)").expect("valid telegram markdown link regex")
    })
}

pub(crate) fn extract_telegram_markdown_image_refs(
    text: &str,
) -> (String, Vec<TelegramMarkdownImageRef>) {
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
