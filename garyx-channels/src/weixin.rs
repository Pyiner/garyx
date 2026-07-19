use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use aes::Aes128;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, block_padding::Pkcs7};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ecb::{Decryptor, Encryptor};
use regex::Regex;
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::fs;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{debug, error, info, warn};

use garyx_bridge::MultiProviderBridge;
use garyx_models::config::{WeixinAccount, WeixinConfig};
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, PromptAttachment, PromptAttachmentKind, StreamBoundaryKind,
    StreamEvent, attachments_to_metadata_value,
};
use garyx_router::{
    InboundRequest, MessageRouter, NATIVE_COMMAND_TEXT_METADATA_KEY, is_native_command_text,
};

use crate::channel_trait::{Channel, ChannelError};
use crate::dispatcher::ChannelDispatcher;
use crate::generated_images::extract_image_generation_result;
use crate::streaming_core::merge_stream_text;

const DEFAULT_LONG_POLL_TIMEOUT_MS: u64 = 35_000;
const POLL_RETRY_DELAY: Duration = Duration::from_secs(2);
const DEFAULT_WEIXIN_CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";

/// Maximum consecutive poll failures before we switch to a longer backoff.
const MAX_CONSECUTIVE_POLL_FAILURES: u32 = 3;
/// Backoff duration after `MAX_CONSECUTIVE_POLL_FAILURES` consecutive failures.
const POLL_BACKOFF_DELAY: Duration = Duration::from_secs(30);
/// How long to pause all API calls after receiving session-expired (errcode=-14).
const SESSION_PAUSE_DURATION: Duration = Duration::from_secs(3600);
/// Maximum CDN upload retry attempts on server errors (5xx).
const CDN_UPLOAD_MAX_RETRIES: u32 = 3;

#[derive(Clone)]
struct WeixinInboundRuntime {
    http: Client,
    account_id: String,
    account: WeixinAccount,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    dispatcher: Arc<dyn ChannelDispatcher>,
    notify_started: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

pub struct WeixinChannel {
    config: WeixinConfig,
    http: Client,
    running: Arc<AtomicBool>,
    poll_tasks: Vec<JoinHandle<()>>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    dispatcher: Arc<dyn ChannelDispatcher>,
}

impl WeixinChannel {
    pub fn new(
        config: WeixinConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        dispatcher: Arc<dyn ChannelDispatcher>,
    ) -> Self {
        Self::with_running(
            config,
            router,
            bridge,
            dispatcher,
            Arc::new(AtomicBool::new(false)),
        )
    }

    pub fn with_running(
        config: WeixinConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        dispatcher: Arc<dyn ChannelDispatcher>,
        running: Arc<AtomicBool>,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_millis(DEFAULT_LONG_POLL_TIMEOUT_MS + 15_000))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            config,
            http,
            running,
            poll_tasks: Vec::new(),
            router,
            bridge,
            dispatcher,
        }
    }

    async fn poll_loop(runtime: WeixinInboundRuntime, running: Arc<AtomicBool>) {
        // SDK parity: restore cursor from disk so we don't re-deliver old messages.
        let mut cursor = get_persisted_cursor(&runtime.account_id).await;
        let mut timeout_ms = DEFAULT_LONG_POLL_TIMEOUT_MS;
        let mut consecutive_failures: u32 = 0;

        while running.load(Ordering::Relaxed) {
            // SDK parity: if session is paused (errcode=-14), sleep and skip.
            if is_session_paused(&runtime.account_id).await {
                debug!(
                    account_id = %runtime.account_id,
                    "weixin session paused (errcode=-14), sleeping"
                );
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }

            let body = json!({
                "get_updates_buf": cursor,
                "base_info": {
                    "channel_version": env!("CARGO_PKG_VERSION")
                }
            });
            let url = build_api_url(&runtime.account.base_url, "getupdates");
            let response = auth_headers(
                runtime
                    .http
                    .post(url)
                    .timeout(Duration::from_millis(timeout_ms + 10_000))
                    .json(&body),
                &runtime.account,
            )
            .send()
            .await;

            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    consecutive_failures += 1;
                    let delay = if consecutive_failures >= MAX_CONSECUTIVE_POLL_FAILURES {
                        warn!(
                            account_id = %runtime.account_id,
                            error = %error,
                            consecutive_failures = consecutive_failures,
                            "weixin getupdates failed (backoff)"
                        );
                        // SDK parity: reset counter after backoff so we get
                        // another round of short retries before next backoff.
                        consecutive_failures = 0;
                        POLL_BACKOFF_DELAY
                    } else {
                        warn!(
                            account_id = %runtime.account_id,
                            error = %error,
                            "weixin getupdates failed, retrying"
                        );
                        POLL_RETRY_DELAY
                    };
                    tokio::time::sleep(delay).await;
                    continue;
                }
            };

            let status = response.status();
            if !status.is_success() {
                consecutive_failures += 1;
                let body = response.text().await.unwrap_or_default();
                let is_backoff = consecutive_failures >= MAX_CONSECUTIVE_POLL_FAILURES;
                warn!(
                    account_id = %runtime.account_id,
                    status = %status,
                    body = %body,
                    consecutive_failures = consecutive_failures,
                    backoff = is_backoff,
                    "weixin getupdates non-success response"
                );
                let delay = if is_backoff {
                    consecutive_failures = 0;
                    POLL_BACKOFF_DELAY
                } else {
                    POLL_RETRY_DELAY
                };
                tokio::time::sleep(delay).await;
                continue;
            }

            let payload = match response.json::<WeixinGetUpdatesResp>().await {
                Ok(payload) => payload,
                Err(error) => {
                    consecutive_failures += 1;
                    warn!(
                        account_id = %runtime.account_id,
                        error = %error,
                        "weixin getupdates parse failed"
                    );
                    tokio::time::sleep(POLL_RETRY_DELAY).await;
                    continue;
                }
            };

            if payload.ret != 0 {
                // SDK parity: errcode=-14 means session expired, pause for 1 hour.
                if payload.errcode == -14 || payload.ret == -14 {
                    pause_session(&runtime.account_id).await;
                    continue;
                }
                consecutive_failures += 1;
                let delay = if consecutive_failures >= MAX_CONSECUTIVE_POLL_FAILURES {
                    consecutive_failures = 0;
                    POLL_BACKOFF_DELAY
                } else {
                    POLL_RETRY_DELAY
                };
                warn!(
                    account_id = %runtime.account_id,
                    errcode = payload.errcode,
                    errmsg = %payload.errmsg,
                    "weixin getupdates returned error"
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            // Successful response — reset failure counter and session pause.
            consecutive_failures = 0;
            clear_session_pause(&runtime.account_id).await;
            if runtime
                .notify_started
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
                && let Err(error) = notify_start(&runtime.http, &runtime.account).await
            {
                warn!(
                    account_id = %runtime.account_id,
                    error = %error,
                    "weixin notifystart failed"
                );
            }

            if !payload.get_updates_buf.is_empty() {
                cursor.clone_from(&payload.get_updates_buf);
                // SDK parity: persist cursor to disk.
                set_persisted_cursor(&runtime.account_id, &cursor).await;
            }
            if payload.longpolling_timeout_ms >= 1_000 {
                timeout_ms = payload.longpolling_timeout_ms;
            }

            for message in payload.msgs {
                Self::handle_message(&runtime, message).await;
            }
        }
    }

    async fn handle_message(runtime: &WeixinInboundRuntime, message: WeixinMessage) {
        if message.message_type != 1 {
            return;
        }
        let from_id = message.from_user_id.trim().to_owned();
        if from_id.is_empty() {
            return;
        }

        if !message.context_token.trim().is_empty() {
            set_context_token(&runtime.account_id, &from_id, message.context_token.trim()).await;
        }

        let incoming_text = extract_text(&message.item_list);
        let mut image_attachments = extract_inline_image_attachments(&message.item_list).await;
        image_attachments
            .extend(extract_cdn_image_attachments(&runtime.http, &message.item_list).await);
        let non_image_media_metadata = extract_cdn_non_image_media_metadata(
            &runtime.http,
            DEFAULT_WEIXIN_CDN_BASE_URL,
            &message.item_list,
        )
        .await;
        let clean_text = incoming_text.trim().to_owned();
        let has_non_image_media = ["file_paths", "voice_paths", "video_paths"]
            .iter()
            .any(|key| {
                non_image_media_metadata
                    .get(*key)
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty())
            });
        if clean_text.is_empty() && image_attachments.is_empty() && !has_non_image_media {
            return;
        }

        let existing_thread_id = {
            let mut router_guard = runtime.router.lock().await;
            let endpoint_thread = router_guard
                .resolve_endpoint_thread_id("weixin", &runtime.account_id, &from_id)
                .await;
            endpoint_thread.or_else(|| {
                router_guard
                    .get_current_thread_id_for_binding("weixin", &runtime.account_id, &from_id)
                    .map(ToOwned::to_owned)
            })
        };
        if !message.context_token.trim().is_empty() {
            set_context_token_for_thread(
                &runtime.account_id,
                &from_id,
                existing_thread_id.as_deref(),
                message.context_token.trim(),
            )
            .await;

            // Flush any pending outbound messages that failed due to expired token.
            // Merge all queued messages into a single send to conserve token quota.
            let pending = drain_pending_outbound(&runtime.account_id, &from_id).await;
            if !pending.is_empty() {
                let fresh_token = message.context_token.trim();
                let valid: Vec<_> = pending
                    .iter()
                    .filter(|q| q.queued_at.elapsed() < Duration::from_secs(30 * 60))
                    .collect();
                if valid.is_empty() {
                    info!(
                        account_id = %runtime.account_id,
                        user_id = %from_id,
                        total = pending.len(),
                        "all pending outbound messages expired, skipping flush"
                    );
                } else {
                    // Merge all valid messages into one, separated by blank lines
                    let merged = valid
                        .iter()
                        .map(|q| q.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    info!(
                        account_id = %runtime.account_id,
                        user_id = %from_id,
                        merged_count = valid.len(),
                        merged_len = merged.len(),
                        "flushing merged pending outbound messages with fresh token"
                    );
                    match send_text_message(
                        &runtime.http,
                        &runtime.account,
                        &from_id,
                        &merged,
                        Some(fresh_token),
                    )
                    .await
                    {
                        Ok(_) => {
                            info!(
                                user_id = %from_id,
                                merged_count = valid.len(),
                                "successfully delivered merged queued weixin messages"
                            );
                        }
                        Err(error) => {
                            warn!(
                                user_id = %from_id,
                                error = %error,
                                merged_count = valid.len(),
                                "failed to deliver merged queued messages, re-queueing"
                            );
                            // Re-queue as a single merged message
                            queue_pending_outbound(&runtime.account_id, &from_id, &merged).await;
                        }
                    }
                }
            }
        }

        // Try to append the message into the already-running Claude session
        // via streaming input instead of interrupting + starting a new run.
        // This preserves full conversation context when the user sends
        // follow-up messages while the agent is still working.
        if !is_native_command_text(&clean_text, "weixin")
            && let Some(thread_id) = existing_thread_id.as_deref()
        {
            let queued = runtime
                .bridge
                .add_streaming_input(
                    thread_id,
                    &clean_text,
                    None,
                    None,
                    Some(image_attachments.clone()),
                    None,
                )
                .await;
            if queued.is_some() {
                tracing::info!(
                    account_id = %runtime.account_id,
                    user_id = %from_id,
                    thread_id,
                    "weixin message queued as streaming input into active session"
                );
                return;
            }
            // No active session to queue into — proceed with a normal
            // new run below.
        }

        let run_id = uuid::Uuid::new_v4().to_string();
        let mut metadata = HashMap::new();
        metadata.insert("channel".to_owned(), Value::String("weixin".to_owned()));
        metadata.insert(
            "account_id".to_owned(),
            Value::String(runtime.account_id.clone()),
        );
        metadata.insert("chat_id".to_owned(), Value::String(from_id.clone()));
        metadata.insert("from_id".to_owned(), Value::String(from_id.clone()));
        if !image_attachments.is_empty() {
            metadata.insert(
                "image_count".to_owned(),
                Value::Number(serde_json::Number::from(image_attachments.len() as u64)),
            );
            metadata.insert(
                ATTACHMENTS_METADATA_KEY.to_owned(),
                attachments_to_metadata_value(&image_attachments),
            );
        }
        metadata.insert(
            NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
            Value::String(clean_text.clone()),
        );
        if !message.context_token.trim().is_empty() {
            metadata.insert(
                "context_token".to_owned(),
                Value::String(message.context_token.clone()),
            );
        }
        // Collect all downloaded media paths for the file_paths field so the
        // agent thread can reference them inline.
        let mut file_paths_for_agent: Vec<String> = Vec::new();
        for key in &["file_paths", "voice_paths", "video_paths"] {
            if let Some(Value::Array(arr)) = non_image_media_metadata.get(*key) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        file_paths_for_agent.push(s.to_owned());
                    }
                }
            }
        }
        for (key, value) in non_image_media_metadata {
            metadata.insert(key, value);
        }

        let response_account = runtime.account.clone();
        let response_http = runtime.http.clone();
        let response_account_id = runtime.account_id.clone();
        let response_user_id = from_id.clone();
        let response_context_token = message.context_token.clone();
        let typing_ticket = match get_typing_ticket(&runtime.account_id, &from_id).await {
            Some(ticket) => Some(ticket),
            None => {
                let ticket = fetch_typing_ticket(
                    &runtime.http,
                    &runtime.account,
                    &from_id,
                    Some(&response_context_token),
                )
                .await
                .map_err(|e| warn!(account_id = %runtime.account_id, from_id = %from_id, error = %e, "failed to fetch weixin typing ticket"))
                .ok()
                .flatten();
                if let Some(ticket) = ticket.as_deref() {
                    set_typing_ticket(&runtime.account_id, &from_id, ticket).await;
                }
                ticket
            }
        };
        let thread_id_holder = Arc::new(std::sync::Mutex::new(String::new()));
        let thread_id_cb = thread_id_holder.clone();
        let final_done_flush_sent = Arc::new(AtomicBool::new(false));
        let final_done_flush_sent_cb = final_done_flush_sent.clone();
        let seen_done_event = Arc::new(AtomicBool::new(false));
        let seen_done_event_cb = seen_done_event.clone();
        let (stream_done_tx, stream_done_rx) = oneshot::channel::<()>();

        let (event_tx, event_rx) = mpsc::unbounded_channel::<StreamEvent>();
        let use_streaming_update = response_account.streaming_update;
        if use_streaming_update {
            let ctx = WeixinStreamConsumerContext {
                http: response_http,
                account: response_account,
                account_id: response_account_id,
                user_id: response_user_id,
                context_token: response_context_token,
                thread_id: thread_id_cb,
                typing_ticket,
                running: runtime.running.clone(),
            };
            tokio::spawn(run_streaming_update_consumer(
                ctx,
                event_rx,
                stream_done_tx,
                final_done_flush_sent_cb,
                seen_done_event_cb,
            ));
        } else {
            let mut event_rx = event_rx;
            tokio::spawn(async move {
                let mut stream_text = String::new();
                let mut typing_keepalive_task: Option<tokio::task::JoinHandle<()>> = None;
                let mut typing_active = false;
                let mut stream_done_tx = Some(stream_done_tx);
                let mut sent_media_refs = HashSet::<String>::new();
                let mut pending_media_refs = Vec::<OutboundMediaRef>::new();

                #[allow(clippy::too_many_arguments)]
                async fn flush_text(
                    text: &str,
                    extra_media_refs: &[OutboundMediaRef],
                    response_http: &Client,
                    response_account: &WeixinAccount,
                    response_account_id: &str,
                    response_user_id: &str,
                    response_context_token: &str,
                    thread_id_cb: &Arc<std::sync::Mutex<String>>,
                    sent_media_refs: &mut HashSet<String>,
                ) {
                    let outbound = text.trim().to_owned();
                    let thread_id = match thread_id_cb.lock() {
                        Ok(guard) => guard.clone(),
                        Err(_) => String::new(),
                    };
                    let mut media_refs = extract_markdown_media_refs(&outbound);
                    media_refs.extend(extra_media_refs.iter().cloned());
                    if outbound.is_empty() && media_refs.is_empty() {
                        return;
                    }

                    // Always prefer the freshest persisted token (may have been
                    // refreshed by a newer inbound message during a long-running
                    // streaming session).  Only fall back to the captured
                    // response_context_token when the store has nothing.
                    let persisted = get_context_token_for_thread(
                        response_account_id,
                        response_user_id,
                        if thread_id.is_empty() {
                            None
                        } else {
                            Some(thread_id.as_str())
                        },
                    )
                    .await;
                    let token = persisted.or_else(|| {
                        let t = response_context_token.trim();
                        if t.is_empty() {
                            None
                        } else {
                            Some(t.to_owned())
                        }
                    });
                    // Short-circuit: if the resolved token is exhausted, queue directly
                    // without attempting the send (avoids unnecessary API call + error).
                    // Note: media refs are included as markdown links in the queued text
                    // so they can be retried when a fresh token arrives.
                    if let Some(ref t) = token
                        && token_sends_remaining(t).await == 0
                    {
                        // Build the full message including media refs as markdown
                        // so nothing is lost when we queue for later delivery.
                        let mut queue_text = outbound.clone();
                        for media_ref in &media_refs {
                            let dedupe_key = media_ref.dedupe_key();
                            if !sent_media_refs.contains(&dedupe_key) {
                                // Append media source as text so it's preserved in queue
                                let media_str = match media_ref {
                                    OutboundMediaRef::RemoteUrl(url) => url.clone(),
                                    OutboundMediaRef::LocalPath(path) => path.clone(),
                                    OutboundMediaRef::InlineImage { file_name, .. } => {
                                        format!("[generated image: {file_name}]")
                                    }
                                };
                                if !queue_text.is_empty() {
                                    queue_text.push('\n');
                                }
                                queue_text.push_str(&media_str);
                            }
                        }
                        let plain = markdown_to_plain_text(&queue_text).trim().to_owned();
                        if !plain.is_empty() {
                            warn!(
                                account_id = %response_account_id,
                                user_id = %response_user_id,
                                has_media = !media_refs.is_empty(),
                                "flush_text: token exhausted, queueing directly"
                            );
                            queue_pending_outbound(response_account_id, response_user_id, &plain)
                                .await;
                        }
                        return;
                    }
                    let plain_text = markdown_to_plain_text(&outbound).trim().to_owned();
                    let mut maybe_message_id: Option<String> = None;
                    for media_ref in media_refs {
                        let dedupe_key = media_ref.dedupe_key();
                        if sent_media_refs.contains(&dedupe_key) {
                            continue;
                        }
                        let media_bytes = match load_media_bytes(response_http, &media_ref).await {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                warn!(
                                    account_id = %response_account_id,
                                    user_id = %response_user_id,
                                    error = %error,
                                    "failed to load weixin media reference"
                                );
                                continue;
                            }
                        };
                        let uploaded = match upload_media_to_cdn(
                            response_http,
                            response_account,
                            response_user_id,
                            &media_bytes,
                            media_ref.classify_media_type(),
                            media_ref.file_name(),
                        )
                        .await
                        {
                            Ok(value) => value,
                            Err(error) => {
                                warn!(
                                    account_id = %response_account_id,
                                    user_id = %response_user_id,
                                    error = %error,
                                    "failed to upload weixin media reference"
                                );
                                continue;
                            }
                        };
                        match send_media_message(
                            response_http,
                            response_account,
                            response_user_id,
                            &uploaded,
                            &plain_text,
                            token.as_deref(),
                        )
                        .await
                        {
                            Ok(message_id) => {
                                sent_media_refs.insert(dedupe_key);
                                maybe_message_id = Some(message_id);
                                break;
                            }
                            Err(error) => {
                                warn!(
                                    account_id = %response_account_id,
                                    user_id = %response_user_id,
                                    error = %error,
                                    "failed to send weixin media message"
                                );
                            }
                        }
                    }
                    if maybe_message_id.is_none() && !plain_text.is_empty() {
                        match send_text_message(
                            response_http,
                            response_account,
                            response_user_id,
                            &plain_text,
                            token.as_deref(),
                        )
                        .await
                        {
                            Ok(_) => {}
                            Err(error) => {
                                error!(
                                    account_id = %response_account_id,
                                    user_id = %response_user_id,
                                    error = %error,
                                    "failed to send weixin response"
                                );
                                // Queue for later delivery when a fresh token arrives
                                let err_str = error.to_string();
                                if err_str.contains("ret=")
                                    || err_str.contains("ret!=0")
                                    || err_str.contains("context_token")
                                    || err_str.contains("send limit")
                                {
                                    queue_pending_outbound(
                                        response_account_id,
                                        response_user_id,
                                        &plain_text,
                                    )
                                    .await;
                                }
                                return;
                            }
                        }
                    }
                }

                while let Some(event) = event_rx.recv().await {
                    match event {
                        StreamEvent::SessionBound { .. } => {}
                        StreamEvent::Delta { text } => {
                            if !typing_active && let Some(ticket) = typing_ticket.clone() {
                                let http = response_http.clone();
                                let account = response_account.clone();
                                let user_id = response_user_id.clone();
                                let ticket_for_task = ticket.clone();
                                if let Err(e) = send_typing_status(
                                    &http,
                                    &account,
                                    &user_id,
                                    &ticket_for_task,
                                    1,
                                )
                                .await
                                {
                                    debug!(account_id = %response_account_id, user_id = %response_user_id, error = %e, "failed to send weixin typing start");
                                }
                                typing_keepalive_task = Some(tokio::spawn(async move {
                                    let mut logged_failure = false;
                                    loop {
                                        tokio::time::sleep(Duration::from_secs(5)).await;
                                        if let Err(e) = send_typing_status(
                                            &http, &account, &user_id, &ticket, 1,
                                        )
                                        .await
                                        {
                                            if !logged_failure {
                                                debug!(error = %e, "weixin typing keepalive failed (suppressing further)");
                                                logged_failure = true;
                                            }
                                        } else {
                                            logged_failure = false;
                                        }
                                    }
                                }));
                                typing_active = true;
                            }
                            stream_text = merge_stream_text(&stream_text, &text);
                        }
                        StreamEvent::Boundary { kind, .. } => match kind {
                            StreamBoundaryKind::UserAck => {
                                // UserAck marks provider-side acceptance of a queued user input.
                                // Do not emit buffered text here, otherwise some providers can
                                // surface user-echo text back to Weixin as an assistant reply.
                                if !stream_text.trim().is_empty() {
                                    info!(
                                        account_id = %response_account_id,
                                        user_id = %response_user_id,
                                        dropped_len = stream_text.len(),
                                        "dropping buffered weixin stream text on user_ack boundary"
                                    );
                                }
                                apply_weixin_stream_boundary(
                                    &mut stream_text,
                                    StreamBoundaryKind::UserAck,
                                );
                            }
                            StreamBoundaryKind::AssistantSegment => {
                                apply_weixin_stream_boundary(
                                    &mut stream_text,
                                    StreamBoundaryKind::AssistantSegment,
                                );
                            }
                        },
                        StreamEvent::ToolUse { .. } => {
                            // Weixin UX prefers fewer, coherent chunks. Flush buffered assistant text
                            // only when a new tool phase starts — BUT conserve token sends.
                            // Each context_token only supports ~10 sends, so if we're running low,
                            // accumulate everything and send it all in the final Done flush.
                            let remaining = token_sends_remaining(&response_context_token).await;
                            if remaining > 2 && !stream_text.trim().is_empty() {
                                flush_text(
                                    &stream_text,
                                    &pending_media_refs,
                                    &response_http,
                                    &response_account,
                                    &response_account_id,
                                    &response_user_id,
                                    &response_context_token,
                                    &thread_id_cb,
                                    &mut sent_media_refs,
                                )
                                .await;
                                stream_text.clear();
                                pending_media_refs.clear();
                            } else if remaining <= 2 {
                                info!(
                                    sends_remaining = remaining,
                                    "conserving token sends — deferring ToolUse flush to final Done"
                                );
                                // Don't flush; accumulate for the final Done event
                            }
                        }
                        StreamEvent::ToolResult { message } => {
                            pending_media_refs
                                .extend(extract_media_refs_from_provider_message(&message));
                        }
                        StreamEvent::ThreadTitleUpdated { .. } => {}
                        StreamEvent::Done => {
                            seen_done_event_cb.store(true, Ordering::Relaxed);
                            if let Some(task) = typing_keepalive_task.take() {
                                task.abort();
                            }
                            if typing_active
                                && let Some(ticket) = typing_ticket.as_deref()
                                && let Err(e) = send_typing_status(
                                    &response_http,
                                    &response_account,
                                    &response_user_id,
                                    ticket,
                                    2,
                                )
                                .await
                            {
                                debug!(account_id = %response_account_id, user_id = %response_user_id, error = %e, "failed to stop weixin typing on done");
                            }
                            let has_final_text = !stream_text.trim().is_empty();
                            flush_text(
                                &stream_text,
                                &pending_media_refs,
                                &response_http,
                                &response_account,
                                &response_account_id,
                                &response_user_id,
                                &response_context_token,
                                &thread_id_cb,
                                &mut sent_media_refs,
                            )
                            .await;
                            pending_media_refs.clear();
                            if has_final_text {
                                final_done_flush_sent_cb.store(true, Ordering::Relaxed);
                            }
                            break;
                        }
                    }
                }
                if let Some(task) = typing_keepalive_task.take() {
                    task.abort();
                }
                if typing_active
                    && let Some(ticket) = typing_ticket.as_deref()
                    && let Err(e) = send_typing_status(
                        &response_http,
                        &response_account,
                        &response_user_id,
                        ticket,
                        2,
                    )
                    .await
                {
                    debug!(account_id = %response_account_id, user_id = %response_user_id, error = %e, "failed to stop weixin typing on cleanup");
                }
                if let Some(done_tx) = stream_done_tx.take() {
                    let _ = done_tx.send(());
                }
            });
        }

        let response_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> =
            Arc::new(move |event: StreamEvent| {
                let _ = event_tx.send(event);
            });

        let request = InboundRequest {
            channel: "weixin".to_owned(),
            account_id: runtime.account_id.clone(),
            from_id: from_id.clone(),
            is_group: false,
            thread_binding_key: from_id.clone(),
            message: clean_text,
            run_id,
            images: Vec::new(),
            extra_metadata: metadata,
            file_paths: file_paths_for_agent,
        };

        let run_id_for_log = request.run_id.clone();
        let holder_for_resolved = thread_id_holder.clone();
        let pipeline = crate::inbound::InboundPipeline {
            router: &runtime.router,
            bridge: &runtime.bridge,
            dispatcher: &runtime.dispatcher,
        };
        let result = pipeline
            .dispatch(
                request,
                response_callback,
                None,
                move |thread_id| async move {
                    if let Ok(mut holder) = holder_for_resolved.lock() {
                        *holder = thread_id;
                    }
                },
            )
            .await;

        match result {
            Ok(result) => {
                let local_reply = result.local_reply;
                if let Some(local_reply) = local_reply {
                    let token = if message.context_token.trim().is_empty() {
                        get_context_token_for_thread(
                            &runtime.account_id,
                            &from_id,
                            Some(&result.thread_id),
                        )
                        .await
                    } else {
                        Some(message.context_token.clone())
                    };
                    if let Err(error) = send_text_message(
                        &runtime.http,
                        &runtime.account,
                        &from_id,
                        &local_reply,
                        token.as_deref(),
                    )
                    .await
                    {
                        error!(
                            account_id = %runtime.account_id,
                            user_id = %from_id,
                            error = %error,
                            "failed to send weixin native command reply"
                        );
                    }
                } else {
                    let _ = tokio::time::timeout(Duration::from_secs(2), stream_done_rx).await;
                    // Only fallback when this callback observed a real provider Done event.
                    // For queued-input runs, callback can be dropped without any stream events.
                    if seen_done_event.load(Ordering::Relaxed)
                        && !final_done_flush_sent.load(Ordering::Relaxed)
                    {
                        let fallback_text = {
                            let router_guard = runtime.router.lock().await;
                            router_guard
                                .latest_assistant_message_text_for_thread(&result.thread_id)
                                .await
                        };
                        if let Some(fallback_text) =
                            fallback_text.map(|text| text.trim().to_owned())
                            && !fallback_text.is_empty()
                        {
                            let token = if message.context_token.trim().is_empty() {
                                get_context_token_for_thread(
                                    &runtime.account_id,
                                    &from_id,
                                    Some(&result.thread_id),
                                )
                                .await
                            } else {
                                Some(message.context_token.clone())
                            };
                            match send_text_message(
                                &runtime.http,
                                &runtime.account,
                                &from_id,
                                &markdown_to_plain_text(&fallback_text),
                                token.as_deref(),
                            )
                            .await
                            {
                                Ok(_) => {}
                                Err(error) => {
                                    error!(
                                        account_id = %runtime.account_id,
                                        user_id = %from_id,
                                        error = %error,
                                        "failed to send weixin fallback final response"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Err(crate::inbound::InboundDispatchFailure::CommittedReplay(error)) => {
                tracing::error!(run_id = %run_id_for_log, error = %error, "committed replay bus missing for Weixin dispatch");
            }
            Err(crate::inbound::InboundDispatchFailure::Dispatch(error)) => {
                let token = if message.context_token.trim().is_empty() {
                    get_context_token_for_thread(
                        &runtime.account_id,
                        &from_id,
                        existing_thread_id.as_deref(),
                    )
                    .await
                } else {
                    Some(message.context_token.clone())
                };
                if let Err(e) = send_text_message(
                    &runtime.http,
                    &runtime.account,
                    &from_id,
                    &format!("Error: {error}"),
                    token.as_deref(),
                )
                .await
                {
                    warn!(account_id = %runtime.account_id, from_id = %from_id, error = %e, "failed to send error reply to weixin user");
                }
            }
        }
    }
}

#[async_trait]
impl Channel for WeixinChannel {
    fn name(&self) -> &str {
        "weixin"
    }

    async fn start(&mut self) -> Result<(), ChannelError> {
        if self.running.load(Ordering::Relaxed) {
            return Err(ChannelError::Internal("already running".to_owned()));
        }
        self.running.store(true, Ordering::Relaxed);

        for (account_id, account) in &self.config.accounts {
            if !account.enabled {
                continue;
            }
            if account.token.trim().is_empty() {
                warn!(
                    account_id = %account_id,
                    "weixin account is enabled but missing token; skipping"
                );
                continue;
            }
            let runtime = WeixinInboundRuntime {
                http: self.http.clone(),
                account_id: account_id.clone(),
                account: account.clone(),
                router: self.router.clone(),
                bridge: self.bridge.clone(),
                dispatcher: self.dispatcher.clone(),
                notify_started: Arc::new(AtomicBool::new(false)),
                running: self.running.clone(),
            };
            let running = self.running.clone();
            self.poll_tasks
                .push(tokio::spawn(Self::poll_loop(runtime, running)));
            info!(account_id = %account_id, "Weixin account polling started");
        }

        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ChannelError> {
        let stop_futures = self
            .config
            .accounts
            .iter()
            .filter(|(_, account)| account.enabled)
            .map(|(account_id, account)| {
                let http = self.http.clone();
                let account = account.clone();
                let account_id = account_id.clone();
                async move {
                    match tokio::time::timeout(Duration::from_secs(2), notify_stop(&http, &account))
                        .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => warn!(
                            account_id = %account_id,
                            error = %error,
                            "weixin notifystop failed"
                        ),
                        Err(_) => warn!(
                            account_id = %account_id,
                            "weixin notifystop timed out"
                        ),
                    }
                }
            })
            .collect::<Vec<_>>();
        futures_util::future::join_all(stop_futures).await;
        self.running.store(false, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(300)).await;
        for handle in self.poll_tasks.drain(..) {
            handle.abort();
            let _ = handle.await;
        }
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

mod media;
mod protocol;
mod send;
mod state;
mod streaming;

pub(crate) use streaming::{WeixinStreamingCallbackConfig, build_weixin_response_callback};

// The split preserves the crate-public weixin surface exactly: every
// symbol that was `pub` at `garyx_channels::weixin::*` before the
// Phase-7 motion is re-exported here (pinned by the
// weixin_public_api integration probe).
pub use send::{
    send_file_message_from_path, send_file_message_from_path_with_cdn_base,
    send_image_message_from_path, send_image_message_from_path_with_cdn_base, send_text_message,
};
pub use state::{
    PendingOutboundMessage, clear_session_pause, drain_pending_outbound, get_context_token,
    get_context_token_for_thread, get_typing_ticket, is_session_paused, pause_session,
    pending_outbound_count, queue_pending_outbound, set_context_token,
    set_context_token_for_thread, set_typing_ticket, token_send_count_prune,
    token_send_count_reset, token_send_increment, token_sends_remaining,
};

use media::*;
use protocol::*;
use send::*;
use state::*;
use streaming::*;

pub mod outbound;
#[cfg(test)]
mod tests;
