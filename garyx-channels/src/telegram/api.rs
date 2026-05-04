use std::path::Path;

use reqwest::Client;
use serde::Serialize;

use crate::channel_trait::ChannelError;

use super::outbox::{
    DeleteMessageBody, EditMessageTextBody, SendMessageBody, enqueue_delete_message,
    enqueue_edit_message_text, enqueue_send_message, retry_backoff_duration,
    wait_for_outbound_slot,
};
use super::text::split_message;
use super::{MAX_MESSAGE_LENGTH, OUTBOUND_MAX_RETRIES, TgMessage, TgResponse};

/// Body for sendChatAction.
#[derive(Debug, Serialize)]
struct SendChatActionBody {
    chat_id: i64,
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<i64>,
}

#[derive(Clone, Copy)]
pub struct TelegramSendTarget<'a> {
    pub http: &'a Client,
    pub token: &'a str,
    pub chat_id: i64,
    pub message_thread_id: Option<i64>,
    pub api_base: &'a str,
}

impl<'a> TelegramSendTarget<'a> {
    pub fn new(
        http: &'a Client,
        token: &'a str,
        chat_id: i64,
        message_thread_id: Option<i64>,
        api_base: &'a str,
    ) -> Self {
        Self {
            http,
            token,
            chat_id,
            message_thread_id,
            api_base,
        }
    }
}

fn is_invalid_reply_target_error(description: &str) -> bool {
    let lowered = description.to_lowercase();
    lowered.contains("message to be replied not found")
        || lowered.contains("replied message not found")
}

/// Send a text response, splitting if necessary.
pub(crate) async fn send_response(
    target: TelegramSendTarget<'_>,
    text: &str,
    reply_to_message_id: Option<i64>,
) -> Result<Vec<i64>, ChannelError> {
    let chunks = split_message(text, MAX_MESSAGE_LENGTH);
    send_message_chunks(target, &chunks, reply_to_message_id).await
}

pub(super) async fn send_message_chunks(
    target: TelegramSendTarget<'_>,
    chunks: &[String],
    reply_to_message_id: Option<i64>,
) -> Result<Vec<i64>, ChannelError> {
    let mut message_ids = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        // Only reply to the original message for the first chunk
        let reply_to = if i == 0 { reply_to_message_id } else { None };

        let mut body = SendMessageBody {
            chat_id: target.chat_id,
            text: chunk.clone(),
            reply_to_message_id: reply_to,
            message_thread_id: target.message_thread_id,
            parse_mode: None,
        };

        let mut last_error: Option<String> = None;

        for attempt in 0..OUTBOUND_MAX_RETRIES {
            match enqueue_send_message(target.http, target.api_base, target.token, body.clone())
                .await
            {
                Ok(msg) => {
                    message_ids.push(msg.message_id);
                    last_error = None;
                    break;
                }
                Err(error) => {
                    if body.reply_to_message_id.is_some()
                        && is_invalid_reply_target_error(&error.message)
                    {
                        body.reply_to_message_id = None;
                        last_error = None;
                        continue;
                    }
                    last_error = Some(error.message.clone());
                    if error.retryable && attempt + 1 < OUTBOUND_MAX_RETRIES {
                        tokio::time::sleep(
                            error
                                .retry_after
                                .unwrap_or_else(|| retry_backoff_duration(attempt)),
                        )
                        .await;
                        continue;
                    }
                    break;
                }
            }
        }

        if let Some(err) = last_error {
            return Err(ChannelError::SendFailed(err));
        }
    }

    Ok(message_ids)
}

/// Send a photo message with an optional caption.
pub async fn send_photo(
    target: TelegramSendTarget<'_>,
    image_path: &Path,
    caption: Option<&str>,
    reply_to_message_id: Option<i64>,
) -> Result<i64, ChannelError> {
    let image_bytes = tokio::fs::read(image_path)
        .await
        .map_err(|e| ChannelError::SendFailed(format!("failed to read image file: {e}")))?;

    let file_name = image_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("image.jpg")
        .to_owned();

    let part = reqwest::multipart::Part::bytes(image_bytes).file_name(file_name);
    let mut form = reqwest::multipart::Form::new()
        .text("chat_id", target.chat_id.to_string())
        .part("photo", part);

    if let Some(caption) = caption
        && !caption.trim().is_empty()
    {
        form = form.text("caption", caption.to_owned());
    }
    if let Some(reply_to) = reply_to_message_id {
        form = form.text("reply_to_message_id", reply_to.to_string());
    }
    if let Some(thread_id) = target.message_thread_id {
        form = form.text("message_thread_id", thread_id.to_string());
    }

    let url = format!(
        "{api_base}/bot{token}/sendPhoto",
        api_base = target.api_base,
        token = target.token
    );
    wait_for_outbound_slot(target.api_base, target.token, target.chat_id).await?;
    let resp = target
        .http
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed(format!("sendPhoto failed: {e}")))?;

    let result: TgResponse<TgMessage> = resp
        .json()
        .await
        .map_err(|e| ChannelError::SendFailed(format!("sendPhoto parse failed: {e}")))?;

    if !result.ok {
        let desc = result.description.unwrap_or_default();
        return Err(ChannelError::SendFailed(format!(
            "sendPhoto API error: {desc}"
        )));
    }

    let message = result
        .result
        .ok_or_else(|| ChannelError::SendFailed("sendPhoto missing result".to_owned()))?;
    Ok(message.message_id)
}

/// Send a document/file message with an optional caption.
pub async fn send_document(
    target: TelegramSendTarget<'_>,
    file_path: &Path,
    caption: Option<&str>,
    reply_to_message_id: Option<i64>,
) -> Result<i64, ChannelError> {
    let file_bytes = tokio::fs::read(file_path)
        .await
        .map_err(|e| ChannelError::SendFailed(format!("failed to read document file: {e}")))?;

    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("attachment.bin")
        .to_owned();

    let part = reqwest::multipart::Part::bytes(file_bytes).file_name(file_name);
    let mut form = reqwest::multipart::Form::new()
        .text("chat_id", target.chat_id.to_string())
        .part("document", part);

    if let Some(caption) = caption
        && !caption.trim().is_empty()
    {
        form = form.text("caption", caption.to_owned());
    }
    if let Some(reply_to) = reply_to_message_id {
        form = form.text("reply_to_message_id", reply_to.to_string());
    }
    if let Some(thread_id) = target.message_thread_id {
        form = form.text("message_thread_id", thread_id.to_string());
    }

    let url = format!(
        "{api_base}/bot{token}/sendDocument",
        api_base = target.api_base,
        token = target.token
    );
    wait_for_outbound_slot(target.api_base, target.token, target.chat_id).await?;
    let resp = target
        .http
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed(format!("sendDocument failed: {e}")))?;

    let result: TgResponse<TgMessage> = resp
        .json()
        .await
        .map_err(|e| ChannelError::SendFailed(format!("sendDocument parse failed: {e}")))?;

    if !result.ok {
        let desc = result.description.unwrap_or_default();
        return Err(ChannelError::SendFailed(format!(
            "sendDocument API error: {desc}"
        )));
    }

    let message = result
        .result
        .ok_or_else(|| ChannelError::SendFailed("sendDocument missing result".to_owned()))?;
    Ok(message.message_id)
}

/// Edit an existing message's text.
pub(super) async fn edit_message_text(
    http: &Client,
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
    parse_mode: Option<&str>,
    api_base: &str,
) -> Result<(), ChannelError> {
    let body = EditMessageTextBody {
        chat_id,
        message_id,
        text: text.to_owned(),
        parse_mode: parse_mode.map(String::from),
    };

    enqueue_edit_message_text(http, api_base, token, body).await
}

/// Delete an existing message.
pub(super) async fn delete_message(
    http: &Client,
    token: &str,
    chat_id: i64,
    message_id: i64,
    api_base: &str,
) -> Result<(), ChannelError> {
    let body = DeleteMessageBody {
        chat_id,
        message_id,
    };

    enqueue_delete_message(http, api_base, token, body).await
}

/// Send a chat action (e.g., "typing").
pub(super) async fn send_chat_action(
    http: &Client,
    token: &str,
    chat_id: i64,
    action: &str,
    message_thread_id: Option<i64>,
    api_base: &str,
) -> Result<(), ChannelError> {
    let body = SendChatActionBody {
        chat_id,
        action: action.to_string(),
        message_thread_id,
    };

    let url = format!("{api_base}/bot{token}/sendChatAction");
    let _resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed(format!("sendChatAction failed: {e}")))?;

    Ok(())
}
