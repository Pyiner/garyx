use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::LazyLock;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::Instant;
use tracing::{info, warn};

use crate::channel_trait::ChannelError;

use super::{OUTBOUND_MAX_RETRIES, TgMessage, TgResponse};

const TELEGRAM_CHAT_REQUEST_INTERVAL: Duration = Duration::from_secs(1);

/// Body for sendMessage.
#[derive(Debug, Clone, Serialize)]
pub(super) struct SendMessageBody {
    pub(super) chat_id: i64,
    pub(super) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reply_to_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) message_thread_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) parse_mode: Option<String>,
}

/// Body for editMessageText.
#[derive(Debug, Clone, Serialize)]
pub(super) struct EditMessageTextBody {
    pub(super) chat_id: i64,
    pub(super) message_id: i64,
    pub(super) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) parse_mode: Option<String>,
}

/// Body for deleteMessage.
#[derive(Debug, Clone, Serialize)]
pub(super) struct DeleteMessageBody {
    pub(super) chat_id: i64,
    pub(super) message_id: i64,
}

#[derive(Debug)]
pub(super) struct TelegramApiCallError {
    pub(super) message: String,
    pub(super) retry_after: Option<Duration>,
    pub(super) retryable: bool,
}

impl TelegramApiCallError {
    fn new(message: impl Into<String>, retry_after: Option<Duration>, retryable: bool) -> Self {
        Self {
            message: message.into(),
            retry_after,
            retryable,
        }
    }
}

impl fmt::Display for TelegramApiCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct TelegramOutboxKey {
    api_base: String,
    token: String,
    chat_id: i64,
}

enum TelegramOutboxCommand {
    SendMessage {
        http: Client,
        api_base: String,
        token: String,
        body: SendMessageBody,
        queued_at: Instant,
        respond_to: oneshot::Sender<Result<TgMessage, TelegramApiCallError>>,
    },
    EditMessageText {
        http: Client,
        api_base: String,
        token: String,
        body: EditMessageTextBody,
        attempt: usize,
        queued_at: Instant,
        coalesced_count: usize,
    },
    DeleteMessage {
        http: Client,
        api_base: String,
        token: String,
        body: DeleteMessageBody,
        queued_at: Instant,
        respond_to: oneshot::Sender<Result<(), TelegramApiCallError>>,
    },
    Permit {
        queued_at: Instant,
        respond_to: oneshot::Sender<()>,
    },
}

static TELEGRAM_OUTBOXES: LazyLock<
    Mutex<HashMap<TelegramOutboxKey, mpsc::UnboundedSender<TelegramOutboxCommand>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn retry_backoff_duration(attempt: usize) -> Duration {
    let millis = 200_u64.saturating_mul(1_u64 << attempt.min(5));
    Duration::from_millis(millis)
}

pub(super) async fn enqueue_send_message(
    http: &Client,
    api_base: &str,
    token: &str,
    body: SendMessageBody,
) -> Result<TgMessage, TelegramApiCallError> {
    let sender = telegram_outbox_sender(api_base, token, body.chat_id).await;
    let (respond_to, response) = oneshot::channel();
    sender
        .send(TelegramOutboxCommand::SendMessage {
            http: http.clone(),
            api_base: api_base.to_owned(),
            token: token.to_owned(),
            body,
            queued_at: Instant::now(),
            respond_to,
        })
        .map_err(|_| TelegramApiCallError::new("Telegram outbox stopped", None, true))?;
    response
        .await
        .map_err(|_| TelegramApiCallError::new("Telegram outbox response dropped", None, true))?
}

pub(super) async fn enqueue_edit_message_text(
    http: &Client,
    api_base: &str,
    token: &str,
    body: EditMessageTextBody,
) -> Result<(), ChannelError> {
    let sender = telegram_outbox_sender(api_base, token, body.chat_id).await;
    sender
        .send(TelegramOutboxCommand::EditMessageText {
            http: http.clone(),
            api_base: api_base.to_owned(),
            token: token.to_owned(),
            body,
            attempt: 0,
            queued_at: Instant::now(),
            coalesced_count: 0,
        })
        .map_err(|_| ChannelError::SendFailed("Telegram outbox stopped".to_owned()))
}

pub(super) async fn enqueue_delete_message(
    http: &Client,
    api_base: &str,
    token: &str,
    body: DeleteMessageBody,
) -> Result<(), ChannelError> {
    let sender = telegram_outbox_sender(api_base, token, body.chat_id).await;
    let (respond_to, response) = oneshot::channel();
    sender
        .send(TelegramOutboxCommand::DeleteMessage {
            http: http.clone(),
            api_base: api_base.to_owned(),
            token: token.to_owned(),
            body,
            queued_at: Instant::now(),
            respond_to,
        })
        .map_err(|_| ChannelError::SendFailed("Telegram outbox stopped".to_owned()))?;
    response
        .await
        .map_err(|_| ChannelError::SendFailed("Telegram outbox response dropped".to_owned()))?
        .map_err(|error| ChannelError::SendFailed(error.message))
}

pub(super) async fn wait_for_outbound_slot(
    api_base: &str,
    token: &str,
    chat_id: i64,
) -> Result<(), ChannelError> {
    let sender = telegram_outbox_sender(api_base, token, chat_id).await;
    let (respond_to, response) = oneshot::channel();
    sender
        .send(TelegramOutboxCommand::Permit {
            queued_at: Instant::now(),
            respond_to,
        })
        .map_err(|_| ChannelError::SendFailed("Telegram outbox stopped".to_owned()))?;
    response
        .await
        .map_err(|_| ChannelError::SendFailed("Telegram outbox response dropped".to_owned()))
}

async fn telegram_outbox_sender(
    api_base: &str,
    token: &str,
    chat_id: i64,
) -> mpsc::UnboundedSender<TelegramOutboxCommand> {
    let key = TelegramOutboxKey {
        api_base: api_base.to_owned(),
        token: token.to_owned(),
        chat_id,
    };

    let mut outboxes = TELEGRAM_OUTBOXES.lock().await;
    if let Some(sender) = outboxes.get(&key) {
        if !sender.is_closed() {
            return sender.clone();
        }
    }

    let (sender, receiver) = mpsc::unbounded_channel();
    tokio::spawn(run_telegram_outbox(key.clone(), receiver));
    outboxes.insert(key, sender.clone());
    sender
}

async fn run_telegram_outbox(
    key: TelegramOutboxKey,
    mut receiver: mpsc::UnboundedReceiver<TelegramOutboxCommand>,
) {
    let mut queue = VecDeque::new();
    let mut next_allowed = tokio::time::Instant::now();

    loop {
        if queue.is_empty() {
            match receiver.recv().await {
                Some(command) => insert_outbox_command(&mut queue, command),
                None => break,
            }
        }

        let delay = next_allowed.saturating_duration_since(tokio::time::Instant::now());
        if !delay.is_zero() {
            tokio::select! {
                maybe_command = receiver.recv() => {
                    match maybe_command {
                        Some(command) => insert_outbox_command(&mut queue, command),
                        None => break,
                    }
                }
                _ = tokio::time::sleep(delay) => {}
            }
            continue;
        }

        let Some(command) = queue.pop_front() else {
            continue;
        };

        let retry_after = execute_outbox_command(&key, command, &mut queue).await;
        let base_next = tokio::time::Instant::now() + TELEGRAM_CHAT_REQUEST_INTERVAL;
        next_allowed = if let Some(retry_after) = retry_after {
            let retry_next = tokio::time::Instant::now() + retry_after;
            base_next.max(retry_next)
        } else {
            base_next
        };
    }
}

fn insert_outbox_command(
    queue: &mut VecDeque<TelegramOutboxCommand>,
    command: TelegramOutboxCommand,
) {
    match command {
        TelegramOutboxCommand::EditMessageText {
            http,
            api_base,
            token,
            body,
            attempt,
            queued_at,
            coalesced_count,
        } => {
            let message_id = body.message_id;
            let mut coalesced_count = coalesced_count;
            if let Some(index) = queue.iter().position(|queued| {
                matches!(
                    queued,
                    TelegramOutboxCommand::EditMessageText { body: queued_body, .. }
                        if queued_body.message_id == message_id
                )
            }) {
                if let Some(TelegramOutboxCommand::EditMessageText {
                    coalesced_count: queued_count,
                    ..
                }) = queue.remove(index)
                {
                    coalesced_count = coalesced_count.saturating_add(queued_count + 1);
                }

                let mut extra_coalesced = 0usize;
                queue.retain(|queued| {
                    if let TelegramOutboxCommand::EditMessageText {
                        body: queued_body,
                        coalesced_count: queued_count,
                        ..
                    } = queued
                    {
                        if queued_body.message_id == message_id {
                            extra_coalesced = extra_coalesced.saturating_add(*queued_count + 1);
                            return false;
                        }
                    }
                    true
                });
                coalesced_count = coalesced_count.saturating_add(extra_coalesced);

                info!(
                    api_base = %api_base,
                    chat_id = body.chat_id,
                    message_id = body.message_id,
                    coalesced_count,
                    queue_len = queue.len(),
                    "Telegram outbox coalesced pending edit"
                );
                queue.insert(
                    index.min(queue.len()),
                    TelegramOutboxCommand::EditMessageText {
                        http,
                        api_base,
                        token,
                        body,
                        attempt,
                        queued_at,
                        coalesced_count,
                    },
                );
                return;
            }

            queue.push_back(TelegramOutboxCommand::EditMessageText {
                http,
                api_base,
                token,
                body,
                attempt,
                queued_at,
                coalesced_count,
            });
        }
        TelegramOutboxCommand::DeleteMessage {
            http,
            api_base,
            token,
            body,
            queued_at,
            respond_to,
        } => {
            let message_id = body.message_id;
            let before_len = queue.len();
            queue.retain(|queued| {
                !matches!(
                    queued,
                    TelegramOutboxCommand::EditMessageText { body: queued_body, .. }
                        if queued_body.message_id == message_id
                )
            });
            let dropped_edit_count = before_len.saturating_sub(queue.len());
            if dropped_edit_count > 0 {
                info!(
                    api_base = %api_base,
                    chat_id = body.chat_id,
                    message_id,
                    dropped_edit_count,
                    queue_len = queue.len(),
                    "Telegram outbox dropped pending edit before delete"
                );
            }
            queue.push_back(TelegramOutboxCommand::DeleteMessage {
                http,
                api_base,
                token,
                body,
                queued_at,
                respond_to,
            });
        }
        other => queue.push_back(other),
    }
}

async fn execute_outbox_command(
    key: &TelegramOutboxKey,
    command: TelegramOutboxCommand,
    queue: &mut VecDeque<TelegramOutboxCommand>,
) -> Option<Duration> {
    match command {
        TelegramOutboxCommand::SendMessage {
            http,
            api_base,
            token,
            body,
            queued_at,
            respond_to,
        } => {
            let wait_ms = duration_ms(queued_at.elapsed());
            let result = send_message_once(&http, &api_base, &token, &body).await;
            let retry_after = result.as_ref().err().and_then(|error| error.retry_after);
            match &result {
                Ok(message) => {
                    info!(
                        api_base = %key.api_base,
                        chat_id = key.chat_id,
                        kind = "sendMessage",
                        wait_ms,
                        message_id = message.message_id,
                        queue_len = queue.len(),
                        "Telegram outbox sent request"
                    );
                }
                Err(error) => {
                    warn!(
                        api_base = %key.api_base,
                        chat_id = key.chat_id,
                        kind = "sendMessage",
                        wait_ms,
                        retryable = error.retryable,
                        retry_after_ms = retry_after.map(duration_ms).unwrap_or(0),
                        queue_len = queue.len(),
                        error = %error,
                        "Telegram outbox request failed"
                    );
                }
            }
            let _ = respond_to.send(result);
            retry_after
        }
        TelegramOutboxCommand::EditMessageText {
            http,
            api_base,
            token,
            body,
            attempt,
            queued_at,
            coalesced_count,
        } => match edit_message_text_once(&http, &api_base, &token, &body).await {
            Ok(()) => {
                info!(
                    api_base = %key.api_base,
                    chat_id = key.chat_id,
                    kind = "editMessageText",
                    wait_ms = duration_ms(queued_at.elapsed()),
                    message_id = body.message_id,
                    attempt,
                    coalesced_count,
                    queue_len = queue.len(),
                    "Telegram outbox sent request"
                );
                None
            }
            Err(error) => {
                let retry_after = error.retry_after;
                if error.retryable && attempt + 1 < OUTBOUND_MAX_RETRIES {
                    if queue.iter().any(|queued| {
                        matches!(
                            queued,
                            TelegramOutboxCommand::EditMessageText { body: queued_body, .. }
                                if queued_body.message_id == body.message_id
                        )
                    }) {
                        info!(
                            api_base = %key.api_base,
                            chat_id = key.chat_id,
                            kind = "editMessageText",
                            wait_ms = duration_ms(queued_at.elapsed()),
                            message_id = body.message_id,
                            attempt,
                            coalesced_count,
                            retry_after_ms = retry_after.map(duration_ms).unwrap_or(0),
                            queue_len = queue.len(),
                            error = %error,
                            "Telegram outbox skipped retry because a newer edit is queued"
                        );
                        return retry_after;
                    }
                    let delay = retry_after.unwrap_or_else(|| retry_backoff_duration(attempt));
                    info!(
                        api_base = %key.api_base,
                        chat_id = key.chat_id,
                        kind = "editMessageText",
                        wait_ms = duration_ms(queued_at.elapsed()),
                        message_id = body.message_id,
                        attempt,
                        next_attempt = attempt + 1,
                        coalesced_count,
                        delay_ms = duration_ms(delay),
                        retry_after_ms = retry_after.map(duration_ms).unwrap_or(0),
                        queue_len = queue.len(),
                        error = %error,
                        "Telegram outbox retrying request"
                    );
                    queue.push_front(TelegramOutboxCommand::EditMessageText {
                        http,
                        api_base,
                        token,
                        body,
                        attempt: attempt + 1,
                        queued_at,
                        coalesced_count,
                    });
                    Some(delay)
                } else {
                    warn!(
                        api_base = %key.api_base,
                        chat_id = key.chat_id,
                        kind = "editMessageText",
                        wait_ms = duration_ms(queued_at.elapsed()),
                        message_id = body.message_id,
                        attempt,
                        coalesced_count,
                        retry_after_ms = retry_after.map(duration_ms).unwrap_or(0),
                        queue_len = queue.len(),
                        error = %error,
                        "Telegram editMessageText failed in rate-limited outbox"
                    );
                    retry_after
                }
            }
        },
        TelegramOutboxCommand::DeleteMessage {
            http,
            api_base,
            token,
            body,
            queued_at,
            respond_to,
        } => {
            let wait_ms = duration_ms(queued_at.elapsed());
            let result = delete_message_once(&http, &api_base, &token, &body).await;
            let retry_after = result.as_ref().err().and_then(|error| error.retry_after);
            match &result {
                Ok(()) => {
                    info!(
                        api_base = %key.api_base,
                        chat_id = key.chat_id,
                        kind = "deleteMessage",
                        wait_ms,
                        message_id = body.message_id,
                        queue_len = queue.len(),
                        "Telegram outbox sent request"
                    );
                }
                Err(error) => {
                    warn!(
                        api_base = %key.api_base,
                        chat_id = key.chat_id,
                        kind = "deleteMessage",
                        wait_ms,
                        message_id = body.message_id,
                        retryable = error.retryable,
                        retry_after_ms = retry_after.map(duration_ms).unwrap_or(0),
                        queue_len = queue.len(),
                        error = %error,
                        "Telegram outbox request failed"
                    );
                }
            }
            let _ = respond_to.send(result);
            retry_after
        }
        TelegramOutboxCommand::Permit {
            queued_at,
            respond_to,
        } => {
            info!(
                api_base = %key.api_base,
                chat_id = key.chat_id,
                kind = "permit",
                wait_ms = duration_ms(queued_at.elapsed()),
                queue_len = queue.len(),
                "Telegram outbox reserved outbound slot"
            );
            let _ = respond_to.send(());
            None
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

async fn send_message_once(
    http: &Client,
    api_base: &str,
    token: &str,
    body: &SendMessageBody,
) -> Result<TgMessage, TelegramApiCallError> {
    let url = format!("{api_base}/bot{token}/sendMessage");
    let resp =
        http.post(&url).json(body).send().await.map_err(|e| {
            TelegramApiCallError::new(format!("sendMessage failed: {e}"), None, false)
        })?;

    let result: TgResponse<TgMessage> = resp.json().await.map_err(|e| {
        TelegramApiCallError::new(format!("sendMessage parse failed: {e}"), None, false)
    })?;

    if !result.ok {
        let desc = result.description.clone().unwrap_or_default();
        return Err(TelegramApiCallError::new(
            format!("sendMessage error: {desc}"),
            retry_after_duration(&result),
            is_transient_api_error(&desc),
        ));
    }

    result
        .result
        .ok_or_else(|| TelegramApiCallError::new("sendMessage missing result", None, false))
}

async fn edit_message_text_once(
    http: &Client,
    api_base: &str,
    token: &str,
    body: &EditMessageTextBody,
) -> Result<(), TelegramApiCallError> {
    let url = format!("{api_base}/bot{token}/editMessageText");
    let resp = http.post(&url).json(body).send().await.map_err(|e| {
        TelegramApiCallError::new(format!("editMessageText failed: {e}"), None, true)
    })?;

    let result: TgResponse<TgMessage> = resp.json().await.map_err(|e| {
        TelegramApiCallError::new(format!("editMessageText parse failed: {e}"), None, true)
    })?;

    if !result.ok {
        let desc = result.description.clone().unwrap_or_default();
        if !desc.contains("message is not modified") {
            return Err(TelegramApiCallError::new(
                format!("editMessageText error: {desc}"),
                retry_after_duration(&result),
                is_transient_api_error(&desc),
            ));
        }
    }

    Ok(())
}

async fn delete_message_once(
    http: &Client,
    api_base: &str,
    token: &str,
    body: &DeleteMessageBody,
) -> Result<(), TelegramApiCallError> {
    let url = format!("{api_base}/bot{token}/deleteMessage");
    let resp =
        http.post(&url).json(body).send().await.map_err(|e| {
            TelegramApiCallError::new(format!("deleteMessage failed: {e}"), None, true)
        })?;

    let result: TgResponse<bool> = resp.json().await.map_err(|e| {
        TelegramApiCallError::new(format!("deleteMessage parse failed: {e}"), None, true)
    })?;

    if !result.ok {
        let desc = result.description.clone().unwrap_or_default();
        return Err(TelegramApiCallError::new(
            format!("deleteMessage error: {desc}"),
            retry_after_duration(&result),
            is_transient_api_error(&desc),
        ));
    }

    Ok(())
}

fn retry_after_duration<T>(response: &TgResponse<T>) -> Option<Duration> {
    response
        .parameters
        .as_ref()
        .and_then(|parameters| parameters.retry_after)
        .map(Duration::from_secs)
}

fn is_transient_api_error(description: &str) -> bool {
    let lowered = description.to_lowercase();
    lowered.contains("too many requests")
        || lowered.contains("internal server error")
        || lowered.contains("bad gateway")
        || lowered.contains("gateway timeout")
        || lowered.contains("timeout")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ok_message(message_id: i64, text: &str) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "result": {
                "message_id": message_id,
                "chat": {"id": 42, "type": "private"},
                "date": 1700000000,
                "text": text
            }
        })
    }

    async fn mount_text_methods(server: &MockServer, token: &str) {
        Mock::given(method("POST"))
            .and(path_regex(&format!(r"/bot{token}/sendMessage")))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_message(1000, "sent")))
            .mount(server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"/bot{token}/editMessageText")))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_message(1000, "edited")))
            .mount(server)
            .await;

        Mock::given(method("POST"))
            .and(path_regex(&format!(r"/bot{token}/deleteMessage")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": true})),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn telegram_outbox_coalesces_pending_edits_to_latest() {
        let server = MockServer::start().await;
        let token = "outbox-coalesce-token";
        mount_text_methods(&server, token).await;

        let http = Client::new();
        let api_base = server.uri();
        let body = SendMessageBody {
            chat_id: 42,
            text: "first".to_owned(),
            reply_to_message_id: None,
            message_thread_id: None,
            parse_mode: None,
        };
        let message = enqueue_send_message(&http, &api_base, token, body)
            .await
            .unwrap();
        assert_eq!(message.message_id, 1000);

        for text in ["one", "two", "three"] {
            enqueue_edit_message_text(
                &http,
                &api_base,
                token,
                EditMessageTextBody {
                    chat_id: 42,
                    message_id: 1000,
                    text: text.to_owned(),
                    parse_mode: None,
                },
            )
            .await
            .unwrap();
        }

        tokio::time::sleep(Duration::from_millis(1300)).await;

        let requests = server.received_requests().await.unwrap();
        let edit_bodies = requests
            .iter()
            .filter(|request| request.url.path() == format!("/bot{token}/editMessageText"))
            .map(|request| serde_json::from_slice::<serde_json::Value>(&request.body).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            edit_bodies.len(),
            1,
            "pending edits to one Telegram message should be collapsed"
        );
        assert_eq!(edit_bodies[0]["text"], "three");
    }

    #[tokio::test]
    async fn telegram_outbox_delete_drops_pending_edit_for_same_message() {
        let server = MockServer::start().await;
        let token = "outbox-delete-token";
        mount_text_methods(&server, token).await;

        let http = Client::new();
        let api_base = server.uri();
        let body = SendMessageBody {
            chat_id: 42,
            text: "first".to_owned(),
            reply_to_message_id: None,
            message_thread_id: None,
            parse_mode: None,
        };
        let message = enqueue_send_message(&http, &api_base, token, body)
            .await
            .unwrap();
        assert_eq!(message.message_id, 1000);

        enqueue_edit_message_text(
            &http,
            &api_base,
            token,
            EditMessageTextBody {
                chat_id: 42,
                message_id: 1000,
                text: "stale".to_owned(),
                parse_mode: None,
            },
        )
        .await
        .unwrap();
        enqueue_delete_message(
            &http,
            &api_base,
            token,
            DeleteMessageBody {
                chat_id: 42,
                message_id: 1000,
            },
        )
        .await
        .unwrap();

        let requests = server.received_requests().await.unwrap();
        let edit_count = requests
            .iter()
            .filter(|request| request.url.path() == format!("/bot{token}/editMessageText"))
            .count();
        let delete_count = requests
            .iter()
            .filter(|request| request.url.path() == format!("/bot{token}/deleteMessage"))
            .count();

        assert_eq!(edit_count, 0, "delete should clear stale queued edits");
        assert_eq!(delete_count, 1);
    }

    #[tokio::test]
    async fn telegram_outbox_serializes_text_sends_per_chat() {
        let server = MockServer::start().await;
        let token = "outbox-send-token";
        mount_text_methods(&server, token).await;

        let http = Client::new();
        let api_base = server.uri();
        let start = std::time::Instant::now();
        for text in ["first", "second"] {
            enqueue_send_message(
                &http,
                &api_base,
                token,
                SendMessageBody {
                    chat_id: 42,
                    text: text.to_owned(),
                    reply_to_message_id: None,
                    message_thread_id: None,
                    parse_mode: None,
                },
            )
            .await
            .unwrap();
        }

        assert!(
            start.elapsed() >= Duration::from_millis(900),
            "second send should wait for the per-chat outbox slot"
        );
    }
}
