use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::sync::{Mutex, mpsc, watch};
use tracing::{error, warn};

use garyx_models::config::ReplyToMode;
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use garyx_router::MessageRouter;

use super::api::{TelegramSendTarget, delete_message, edit_message_text, send_message_chunks};
use super::text::split_message;
use super::{MAX_MESSAGE_LENGTH, resolve_reply_to, send_response};

#[derive(Debug)]
struct StreamState {
    message_id: Option<i64>,
    accumulated_text: String,
    last_rendered_text: String,
    last_edit_time: Instant,
    flush_scheduled: bool,
    finalized: bool,
    tool_placeholder_active: bool,
    pending_tool_names: Vec<String>,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            message_id: None,
            accumulated_text: String::new(),
            last_rendered_text: String::new(),
            last_edit_time: Instant::now(),
            flush_scheduled: false,
            finalized: false,
            tool_placeholder_active: false,
            pending_tool_names: Vec::new(),
        }
    }
}

pub(crate) struct StreamingCallbackConfig {
    pub http: Client,
    pub token: String,
    pub router: Arc<Mutex<MessageRouter>>,
    pub account_id: String,
    pub chat_id: i64,
    pub api_base: String,
    pub reply_to_mode: ReplyToMode,
    pub reply_to: Option<i64>,
    pub outbound_thread_id: Option<i64>,
    pub outbound_thread_scope: Option<String>,
}

struct StreamingCallbackShared {
    cfg: StreamingCallbackConfig,
    state: Mutex<StreamState>,
}

fn telegram_tool_display_name(message: &ProviderMessage) -> String {
    message
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            message
                .content
                .pointer("/name")
                .or_else(|| message.content.pointer("/tool_name"))
                .or_else(|| message.content.pointer("/tool"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "tool".to_owned())
}

fn render_tool_placeholder(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("🔧 {name}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_stream_display_text(state: &StreamState) -> String {
    if !state.tool_placeholder_active {
        return state.accumulated_text.clone();
    }

    let placeholder = render_tool_placeholder(&state.pending_tool_names);
    if placeholder.trim().is_empty() {
        return state.accumulated_text.clone();
    }
    if state.accumulated_text.trim().is_empty() {
        return placeholder;
    }
    if state.accumulated_text.ends_with("\n\n") {
        format!("{}{}", state.accumulated_text, placeholder)
    } else if state.accumulated_text.ends_with('\n') {
        format!("{}\n{}", state.accumulated_text, placeholder)
    } else {
        format!("{}\n\n{}", state.accumulated_text, placeholder)
    }
}

impl StreamingCallbackShared {
    async fn flush_pending_stream_text(&self, thread_id: &str) {
        let mut state = self.state.lock().await;
        state.flush_scheduled = false;

        if state.finalized || state.tool_placeholder_active {
            return;
        }

        let display_text = state.accumulated_text.clone();
        if display_text.trim().is_empty() {
            return;
        }
        if display_text.trim() == state.last_rendered_text.trim() {
            return;
        }

        if self
            .roll_stream_segment_if_needed(thread_id, &mut state, &display_text)
            .await
        {
            return;
        }

        if let Some(msg_id) = state.message_id {
            match edit_message_text(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                msg_id,
                &display_text,
                None,
                &self.cfg.api_base,
            )
            .await
            {
                Ok(()) => {
                    state.last_rendered_text = display_text;
                    state.last_edit_time = Instant::now();
                }
                Err(e) => {
                    warn!(
                        account_id = %self.cfg.account_id,
                        error = %e,
                        "delayed stream flush edit failed"
                    );
                }
            }
        }
    }

    fn effective_reply_to(&self) -> Option<i64> {
        resolve_reply_to(
            &self.cfg.reply_to_mode,
            self.cfg.reply_to.unwrap_or(0),
            true,
        )
    }

    async fn record_outbound_messages(&self, thread_id: &str, msg_ids: &[i64]) {
        if thread_id.is_empty() || msg_ids.is_empty() {
            return;
        }

        let mut router_guard = self.cfg.router.lock().await;
        for msg_id in msg_ids {
            router_guard
                .record_outbound_message_with_persistence(
                    thread_id,
                    "telegram",
                    &self.cfg.account_id,
                    &self.cfg.chat_id.to_string(),
                    self.cfg.outbound_thread_scope.as_deref(),
                    &msg_id.to_string(),
                )
                .await;
        }
    }

    async fn process_boundary(&self, thread_id: &str, state: &mut StreamState) {
        let mut delivered_msg_ids: Vec<i64> = Vec::new();

        let pending_boundary_text = state.accumulated_text.clone();
        let _ = self
            .roll_stream_segment_if_needed(thread_id, state, &pending_boundary_text)
            .await;
        let boundary_text = state.accumulated_text.trim().to_owned();

        if !boundary_text.is_empty() {
            if let Some(msg_id) = state.message_id {
                if boundary_text != state.last_rendered_text.trim() {
                    match edit_message_text(
                        &self.cfg.http,
                        &self.cfg.token,
                        self.cfg.chat_id,
                        msg_id,
                        &boundary_text,
                        None,
                        &self.cfg.api_base,
                    )
                    .await
                    {
                        Ok(()) => {
                            state.last_rendered_text = boundary_text.clone();
                            state.last_edit_time = Instant::now();
                        }
                        Err(e) => {
                            warn!(
                                account_id = %self.cfg.account_id,
                                error = %e,
                                "boundary flush edit failed; sending a fresh message"
                            );
                            state.message_id = None;
                        }
                    }
                }
            }

            if let Some(msg_id) = state.message_id {
                delivered_msg_ids.push(msg_id);
            } else {
                match send_response(
                    TelegramSendTarget::new(
                        &self.cfg.http,
                        &self.cfg.token,
                        self.cfg.chat_id,
                        self.cfg.outbound_thread_id,
                        &self.cfg.api_base,
                    ),
                    &boundary_text,
                    self.effective_reply_to(),
                )
                .await
                {
                    Ok(msg_ids) => {
                        if let Some(&last_id) = msg_ids.last() {
                            state.message_id = Some(last_id);
                            state.last_rendered_text = boundary_text.clone();
                            state.last_edit_time = Instant::now();
                        }
                        delivered_msg_ids = msg_ids;
                    }
                    Err(e) => {
                        error!(
                            account_id = %self.cfg.account_id,
                            chat_id = self.cfg.chat_id,
                            error = %e,
                            "failed to flush Telegram boundary segment"
                        );
                    }
                }
            }
        }

        self.record_outbound_messages(thread_id, &delivered_msg_ids)
            .await;

        state.message_id = None;
        state.accumulated_text.clear();
        state.last_rendered_text.clear();
        state.last_edit_time = Instant::now();
        state.flush_scheduled = false;
        state.finalized = false;
        state.tool_placeholder_active = false;
        state.pending_tool_names.clear();
    }

    async fn process_tool_use(
        &self,
        thread_id: &str,
        state: &mut StreamState,
        message: ProviderMessage,
    ) {
        if !state.accumulated_text.trim().is_empty() {
            let accumulated_text = state.accumulated_text.clone();
            if self
                .roll_stream_segment_if_needed(thread_id, state, &accumulated_text)
                .await
            {
                state.tool_placeholder_active = false;
                state.pending_tool_names.clear();
            }
        }

        let name = telegram_tool_display_name(&message);
        state.pending_tool_names.push(name);
        state.tool_placeholder_active = true;
        let display_text = render_stream_display_text(state);
        if display_text.trim().is_empty() {
            return;
        }
        if display_text.len() > MAX_MESSAGE_LENGTH {
            warn!(
                account_id = %self.cfg.account_id,
                display_len = display_text.len(),
                "Telegram tool placeholder skipped because it would exceed message length"
            );
            state.tool_placeholder_active = false;
            state.pending_tool_names.clear();
            return;
        }

        if let Some(msg_id) = state.message_id {
            match edit_message_text(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                msg_id,
                &display_text,
                None,
                &self.cfg.api_base,
            )
            .await
            {
                Ok(()) => {
                    state.last_rendered_text = display_text;
                    state.last_edit_time = Instant::now();
                    state.flush_scheduled = false;
                    state.finalized = false;
                    return;
                }
                Err(error) => {
                    warn!(
                        account_id = %self.cfg.account_id,
                        error = %error,
                        "Telegram tool placeholder edit failed; sending a fresh message"
                    );
                    state.message_id = None;
                }
            }
        }

        match send_response(
            TelegramSendTarget::new(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                self.cfg.outbound_thread_id,
                &self.cfg.api_base,
            ),
            &display_text,
            self.effective_reply_to(),
        )
        .await
        {
            Ok(msg_ids) => {
                if let Some(&last_id) = msg_ids.last() {
                    state.message_id = Some(last_id);
                    state.last_rendered_text = display_text;
                    state.last_edit_time = Instant::now();
                    state.flush_scheduled = false;
                    state.finalized = false;
                }
            }
            Err(error) => {
                error!(
                    account_id = %self.cfg.account_id,
                    chat_id = self.cfg.chat_id,
                    error = %error,
                    "failed to send Telegram tool placeholder"
                );
            }
        }
    }

    async fn clear_tool_placeholder(&self, state: &mut StreamState) {
        if !state.tool_placeholder_active {
            return;
        }

        state.tool_placeholder_active = false;
        state.pending_tool_names.clear();
        state.flush_scheduled = false;

        let display_text = state.accumulated_text.clone();
        let Some(msg_id) = state.message_id else {
            state.last_rendered_text.clear();
            return;
        };

        if display_text.trim().is_empty() {
            match delete_message(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                msg_id,
                &self.cfg.api_base,
            )
            .await
            {
                Ok(()) => {
                    state.message_id = None;
                    state.last_rendered_text.clear();
                    state.last_edit_time = Instant::now();
                }
                Err(error) => {
                    warn!(
                        account_id = %self.cfg.account_id,
                        error = %error,
                        "failed to delete Telegram tool placeholder"
                    );
                }
            }
            return;
        }

        if display_text.trim() == state.last_rendered_text.trim() {
            return;
        }

        match edit_message_text(
            &self.cfg.http,
            &self.cfg.token,
            self.cfg.chat_id,
            msg_id,
            &display_text,
            None,
            &self.cfg.api_base,
        )
        .await
        {
            Ok(()) => {
                state.last_rendered_text = display_text;
                state.last_edit_time = Instant::now();
            }
            Err(error) => {
                warn!(
                    account_id = %self.cfg.account_id,
                    error = %error,
                    "failed to clear Telegram tool placeholder"
                );
            }
        }
    }

    async fn roll_stream_segment_if_needed(
        &self,
        thread_id: &str,
        state: &mut StreamState,
        display_text: &str,
    ) -> bool {
        let chunks = split_message(display_text, MAX_MESSAGE_LENGTH);
        if chunks.len() <= 1 {
            return false;
        }

        let mut finalized_msg_ids = Vec::new();
        let active_chunk = chunks.last().cloned().unwrap_or_default();

        if let Some(msg_id) = state.message_id {
            if let Err(error) = edit_message_text(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                msg_id,
                &chunks[0],
                None,
                &self.cfg.api_base,
            )
            .await
            {
                warn!(
                    account_id = %self.cfg.account_id,
                    error = %error,
                    "stream segment rollover edit failed"
                );
                state.message_id = None;
                return false;
            }
            finalized_msg_ids.push(msg_id);

            match send_message_chunks(
                TelegramSendTarget::new(
                    &self.cfg.http,
                    &self.cfg.token,
                    self.cfg.chat_id,
                    self.cfg.outbound_thread_id,
                    &self.cfg.api_base,
                ),
                &chunks[1..],
                None,
            )
            .await
            {
                Ok(message_ids) => {
                    if message_ids.len() > 1 {
                        finalized_msg_ids.extend_from_slice(&message_ids[..message_ids.len() - 1]);
                    }
                    state.message_id = message_ids.last().copied();
                }
                Err(error) => {
                    error!(
                        account_id = %self.cfg.account_id,
                        chat_id = self.cfg.chat_id,
                        error = %error,
                        "failed to send rollover Telegram stream segment"
                    );
                    state.message_id = None;
                    return false;
                }
            }
        } else {
            match send_message_chunks(
                TelegramSendTarget::new(
                    &self.cfg.http,
                    &self.cfg.token,
                    self.cfg.chat_id,
                    self.cfg.outbound_thread_id,
                    &self.cfg.api_base,
                ),
                &chunks,
                self.effective_reply_to(),
            )
            .await
            {
                Ok(message_ids) => {
                    if message_ids.len() > 1 {
                        finalized_msg_ids.extend_from_slice(&message_ids[..message_ids.len() - 1]);
                    }
                    state.message_id = message_ids.last().copied();
                }
                Err(error) => {
                    error!(
                        account_id = %self.cfg.account_id,
                        chat_id = self.cfg.chat_id,
                        error = %error,
                        "failed to start rollover Telegram stream segment"
                    );
                    state.message_id = None;
                    return false;
                }
            }
        }

        state.accumulated_text = active_chunk.clone();
        state.last_rendered_text = active_chunk;
        state.last_edit_time = Instant::now();
        state.flush_scheduled = false;

        self.record_outbound_messages(thread_id, &finalized_msg_ids)
            .await;
        true
    }

    async fn process_event(self: &Arc<Self>, event: StreamEvent, thread_id: &str) {
        let mut state = self.state.lock().await;

        let is_final = match event {
            StreamEvent::Boundary { kind, .. } => match kind {
                StreamBoundaryKind::UserAck => {
                    self.process_boundary(thread_id, &mut state).await;
                    return;
                }
                StreamBoundaryKind::AssistantSegment => {
                    crate::streaming_core::apply_stream_boundary_text(
                        &mut state.accumulated_text,
                        StreamBoundaryKind::AssistantSegment,
                    );
                    false
                }
            },
            StreamEvent::Delta { text } => {
                if text.is_empty() {
                    return;
                }
                if state.tool_placeholder_active {
                    state.tool_placeholder_active = false;
                    state.pending_tool_names.clear();
                }
                state.accumulated_text =
                    crate::streaming_core::merge_stream_text(&state.accumulated_text, &text);
                state.finalized = false;
                false
            }
            StreamEvent::ToolUse { message } => {
                self.process_tool_use(thread_id, &mut state, message).await;
                return;
            }
            StreamEvent::ToolResult { .. } => {
                return;
            }
            StreamEvent::Done => true,
        };

        if is_final && state.tool_placeholder_active {
            self.clear_tool_placeholder(&mut state).await;
            state.finalized = true;
            state.flush_scheduled = false;
            if let Some(msg_id) = state.message_id {
                self.record_outbound_messages(thread_id, &[msg_id]).await;
            }
            return;
        }

        if is_final && state.flush_scheduled {
            let pending_text = state.accumulated_text.clone();
            if !pending_text.trim().is_empty()
                && pending_text.trim() != state.last_rendered_text.trim()
                && !self
                    .roll_stream_segment_if_needed(thread_id, &mut state, &pending_text)
                    .await
            {
                if let Some(msg_id) = state.message_id {
                    match edit_message_text(
                        &self.cfg.http,
                        &self.cfg.token,
                        self.cfg.chat_id,
                        msg_id,
                        &pending_text,
                        None,
                        &self.cfg.api_base,
                    )
                    .await
                    {
                        Ok(()) => {
                            state.last_rendered_text = pending_text;
                            state.last_edit_time = Instant::now();
                        }
                        Err(e) => {
                            warn!(
                                account_id = %self.cfg.account_id,
                                error = %e,
                                "pre-final delayed stream flush edit failed"
                            );
                        }
                    }
                }
            }
            state.flush_scheduled = false;
        }

        let display_text = state.accumulated_text.clone();

        if display_text.trim().is_empty() {
            return;
        }

        if self
            .roll_stream_segment_if_needed(thread_id, &mut state, &display_text)
            .await
        {
            if is_final {
                state.finalized = true;
                state.flush_scheduled = false;
            }
            if is_final {
                if let Some(msg_id) = state.message_id {
                    self.record_outbound_messages(thread_id, &[msg_id]).await;
                }
            }
            return;
        }

        if let Some(msg_id) = state.message_id {
            if !is_final {
                let now = Instant::now();
                let elapsed = now.duration_since(state.last_edit_time);
                if elapsed < Duration::from_millis(300) {
                    if !state.flush_scheduled {
                        state.flush_scheduled = true;
                        let shared = self.clone();
                        let thread_id = thread_id.to_owned();
                        let delay = Duration::from_millis(300) - elapsed;
                        tokio::spawn(async move {
                            tokio::time::sleep(delay).await;
                            shared.flush_pending_stream_text(&thread_id).await;
                        });
                    }
                    return;
                }
            }

            if display_text.trim() == state.last_rendered_text.trim() {
                if is_final {
                    state.finalized = true;
                    state.flush_scheduled = false;
                    if let Some(msg_id) = state.message_id {
                        self.record_outbound_messages(thread_id, &[msg_id]).await;
                    }
                }
                return;
            }

            match edit_message_text(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                msg_id,
                &display_text,
                None,
                &self.cfg.api_base,
            )
            .await
            {
                Ok(()) => {
                    state.last_rendered_text = display_text.clone();
                    state.last_edit_time = Instant::now();
                    state.flush_scheduled = false;
                }
                Err(e) => {
                    warn!(
                        account_id = %self.cfg.account_id,
                        error = %e,
                        "edit failed, will send new message on next chunk"
                    );
                    state.message_id = None;
                }
            }
        } else {
            match send_response(
                TelegramSendTarget::new(
                    &self.cfg.http,
                    &self.cfg.token,
                    self.cfg.chat_id,
                    self.cfg.outbound_thread_id,
                    &self.cfg.api_base,
                ),
                &display_text,
                self.effective_reply_to(),
            )
            .await
            {
                Ok(msg_ids) => {
                    if let Some(&last_id) = msg_ids.last() {
                        state.message_id = Some(last_id);
                        state.last_rendered_text = display_text.clone();
                        state.last_edit_time = Instant::now();
                        state.flush_scheduled = false;
                    }
                }
                Err(e) => {
                    error!(
                        account_id = %self.cfg.account_id,
                        chat_id = self.cfg.chat_id,
                        error = %e,
                        "failed to send response to Telegram"
                    );
                }
            }
        }

        if is_final {
            state.finalized = true;
            state.flush_scheduled = false;
        }

        if is_final {
            if let Some(msg_id) = state.message_id {
                self.record_outbound_messages(thread_id, &[msg_id]).await;
            }
        }
    }
}

pub(super) fn build_response_callback(
    cfg: StreamingCallbackConfig,
) -> (
    Arc<dyn Fn(StreamEvent) + Send + Sync>,
    watch::Sender<String>,
) {
    let shared = Arc::new(StreamingCallbackShared {
        cfg,
        state: Mutex::new(StreamState::default()),
    });
    let (thread_id_tx, thread_id_rx) = watch::channel(String::new());
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<StreamEvent>();

    let shared_for_worker = shared.clone();
    tokio::spawn(async move {
        let mut thread_id_rx = thread_id_rx;
        while let Some(event) = event_rx.recv().await {
            let mut thread_id = thread_id_rx.borrow().clone();
            if thread_id.is_empty() {
                while thread_id.is_empty() {
                    if thread_id_rx.changed().await.is_err() {
                        break;
                    }
                    thread_id = thread_id_rx.borrow().clone();
                }
            }

            shared_for_worker.process_event(event, &thread_id).await;
        }
    });

    let response_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> =
        Arc::new(move |event: StreamEvent| {
            let _ = event_tx.send(event);
        });

    (response_callback, thread_id_tx)
}

pub(crate) fn build_bound_response_callback(
    cfg: StreamingCallbackConfig,
    thread_id: String,
) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let (callback, thread_id_tx) = build_response_callback(cfg);
    let _ = thread_id_tx.send(thread_id);
    callback
}
