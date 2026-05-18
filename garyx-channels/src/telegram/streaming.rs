use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use reqwest::Client;
use tokio::sync::{Mutex, mpsc, watch};
use tracing::{error, warn};

use crate::generated_images::{extract_image_generation_result, write_generated_image_temp};
use crate::plugin_tools::{
    PluginStreamSendDecision, PluginStreamSendPolicy, PluginStreamSendState,
    should_hide_tool_call_display,
};
use garyx_models::config::ReplyToMode;
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use garyx_router::MessageRouter;

use super::api::{
    TelegramSendTarget, delete_message, edit_message_text, send_message_chunks, send_photo,
};
use super::markdown::MARKDOWN_V2_PARSE_MODE;
use super::text::split_message;
use super::{MAX_MESSAGE_LENGTH, resolve_reply_to, send_response};

#[derive(Debug)]
struct StreamState {
    message_id: Option<i64>,
    stream_text: PluginStreamSendState,
    markdown_image_scan_text: String,
    sent_markdown_image_paths: HashSet<String>,
    last_rendered_text: String,
    finalized: bool,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            message_id: None,
            stream_text: PluginStreamSendState::new(PluginStreamSendPolicy::telegram_like()),
            markdown_image_scan_text: String::new(),
            sent_markdown_image_paths: HashSet::new(),
            last_rendered_text: String::new(),
            finalized: false,
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

fn render_stream_content_text(state: &StreamState) -> String {
    state.stream_text.render_content_text()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownImageRef {
    path: PathBuf,
    source_range: Range<usize>,
}

fn supported_markdown_image_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

fn markdown_image_target_path(raw_target: &str) -> Option<PathBuf> {
    let mut target = raw_target.trim();
    if target.is_empty() {
        return None;
    }

    if let Some(stripped) = target
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    {
        target = stripped.trim();
    } else if let Some(index) = target.find(char::is_whitespace) {
        target = target[..index].trim();
    }

    let target = target.trim_matches(|value| value == '"' || value == '\'');
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("data:")
    {
        return None;
    }

    let path = target
        .strip_prefix("file://")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(target));
    path.is_absolute()
        .then_some(path)
        .filter(|path| supported_markdown_image_extension(path))
        .filter(|path| path.is_file())
}

fn scan_markdown_image_refs(text: &str) -> Vec<MarkdownImageRef> {
    let mut refs = Vec::new();
    let mut offset = 0;

    while let Some(relative_start) = text[offset..].find("![") {
        let start = offset + relative_start;
        let alt_start = start + 2;
        let Some(alt_end_relative) = text[alt_start..].find("](") else {
            offset = alt_start;
            continue;
        };
        let alt_end = alt_start + alt_end_relative;
        let target_start = alt_end + 2;
        let Some(target_end_relative) = text[target_start..].find(')') else {
            offset = target_start;
            continue;
        };
        let target_end = target_start + target_end_relative;
        let target = &text[target_start..target_end];

        if let Some(path) = markdown_image_target_path(target) {
            refs.push(MarkdownImageRef {
                path,
                source_range: start..target_end + 1,
            });
        }

        offset = target_end + 1;
    }

    refs
}

fn extract_markdown_image_refs(text: &str) -> Vec<MarkdownImageRef> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();

    for image_ref in scan_markdown_image_refs(text) {
        let key = image_ref.path.to_string_lossy().to_string();
        if seen.insert(key) {
            refs.push(image_ref);
        }
    }

    refs
}

fn strip_deliverable_markdown_images(text: &str) -> String {
    let image_refs = scan_markdown_image_refs(text);
    if image_refs.is_empty() {
        return text.to_owned();
    }

    let mut stripped = String::with_capacity(text.len());
    let mut cursor = 0;
    for image_ref in image_refs {
        if image_ref.source_range.start > cursor {
            stripped.push_str(&text[cursor..image_ref.source_range.start]);
        }
        cursor = image_ref.source_range.end;
    }
    if cursor < text.len() {
        stripped.push_str(&text[cursor..]);
    }

    while stripped.contains("\n\n\n") {
        stripped = stripped.replace("\n\n\n", "\n\n");
    }

    stripped.trim().to_owned()
}

fn render_stream_display_text(state: &StreamState) -> String {
    strip_deliverable_markdown_images(&render_stream_content_text(state))
}

impl StreamingCallbackShared {
    fn reset_for_fresh_message(state: &mut StreamState) {
        state.message_id = None;
        state.stream_text = PluginStreamSendState::new(PluginStreamSendPolicy::telegram_like());
        state.markdown_image_scan_text.clear();
        state.sent_markdown_image_paths.clear();
        state.last_rendered_text.clear();
        state.finalized = false;
    }

    async fn flush_pending_stream_text(self: &Arc<Self>, thread_id: &str) {
        let mut state = self.state.lock().await;

        if state.finalized {
            return;
        }

        let PluginStreamSendDecision::FlushNow { content_text } =
            state.stream_text.scheduled_flush()
        else {
            return;
        };

        if content_text.trim().is_empty() {
            return;
        }
        let display_text = strip_deliverable_markdown_images(&content_text);
        if display_text.trim().is_empty() {
            return;
        }
        if display_text.trim() == state.last_rendered_text.trim() {
            return;
        }

        if self
            .roll_stream_segment_if_needed(thread_id, &mut state, &content_text)
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
                Some(MARKDOWN_V2_PARSE_MODE),
                &self.cfg.api_base,
            )
            .await
            {
                Ok(()) => {
                    state.last_rendered_text = display_text;
                    state.stream_text.mark_flushed(Instant::now());
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

    async fn delete_runtime_only_message(&self, state: &mut StreamState) {
        let Some(msg_id) = state.message_id else {
            return;
        };

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
            }
            Err(error) => {
                warn!(
                    account_id = %self.cfg.account_id,
                    error = %error,
                    "failed to delete Telegram runtime-only stream message"
                );
                state.message_id = None;
                state.last_rendered_text.clear();
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

    async fn send_markdown_images_from_state(&self, thread_id: &str, state: &mut StreamState) {
        let image_refs = extract_markdown_image_refs(&state.markdown_image_scan_text);
        if image_refs.is_empty() {
            return;
        }

        let mut delivered_msg_ids = Vec::new();
        for image_ref in image_refs {
            let key = image_ref.path.to_string_lossy().to_string();
            if state.sent_markdown_image_paths.contains(&key) {
                continue;
            }

            match send_photo(
                TelegramSendTarget::new(
                    &self.cfg.http,
                    &self.cfg.token,
                    self.cfg.chat_id,
                    self.cfg.outbound_thread_id,
                    &self.cfg.api_base,
                ),
                &image_ref.path,
                None,
                self.effective_reply_to(),
            )
            .await
            {
                Ok(message_id) => {
                    state.sent_markdown_image_paths.insert(key);
                    delivered_msg_ids.push(message_id);
                }
                Err(error) => {
                    warn!(
                        account_id = %self.cfg.account_id,
                        chat_id = self.cfg.chat_id,
                        path = %image_ref.path.display(),
                        error = %error,
                        "failed to send Telegram markdown image"
                    );
                }
            }
        }

        self.record_outbound_messages(thread_id, &delivered_msg_ids)
            .await;
    }

    async fn process_boundary(&self, thread_id: &str, state: &mut StreamState) {
        let mut delivered_msg_ids: Vec<i64> = Vec::new();

        let pending_boundary_text = state.stream_text.accumulated_text().to_owned();
        let _ = self
            .roll_stream_segment_if_needed(thread_id, state, &pending_boundary_text)
            .await;
        let boundary_content_text = state.stream_text.accumulated_text().trim().to_owned();
        let boundary_text = strip_deliverable_markdown_images(&boundary_content_text);

        if boundary_content_text.is_empty() {
            self.delete_runtime_only_message(state).await;
            Self::reset_for_fresh_message(state);
            return;
        }

        if boundary_text.is_empty() {
            self.delete_runtime_only_message(state).await;
            self.send_markdown_images_from_state(thread_id, state).await;
            Self::reset_for_fresh_message(state);
            return;
        }

        if !boundary_text.is_empty() {
            if let Some(msg_id) = state.message_id
                && boundary_text != state.last_rendered_text.trim()
            {
                match edit_message_text(
                    &self.cfg.http,
                    &self.cfg.token,
                    self.cfg.chat_id,
                    msg_id,
                    &boundary_text,
                    Some(MARKDOWN_V2_PARSE_MODE),
                    &self.cfg.api_base,
                )
                .await
                {
                    Ok(()) => {
                        state.last_rendered_text = boundary_text.clone();
                        state.stream_text.mark_flushed(Instant::now());
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
                            state.stream_text.mark_flushed(Instant::now());
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

        self.send_markdown_images_from_state(thread_id, state).await;

        Self::reset_for_fresh_message(state);
    }

    async fn process_tool_use(
        self: &Arc<Self>,
        thread_id: &str,
        state: &mut StreamState,
        message: ProviderMessage,
    ) {
        if !state.stream_text.accumulated_text().trim().is_empty() {
            let accumulated_text = state.stream_text.accumulated_text().to_owned();
            if self
                .roll_stream_segment_if_needed(thread_id, state, &accumulated_text)
                .await
            {
                state.stream_text.clear_tool_placeholder();
            }
        }

        let decision = state.stream_text.on_tool_call(&message, Instant::now());
        let PluginStreamSendDecision::FlushNow { content_text } = decision else {
            return;
        };
        let display_text = strip_deliverable_markdown_images(&content_text);
        if display_text.trim().is_empty() {
            return;
        }
        if display_text.len() > MAX_MESSAGE_LENGTH {
            warn!(
                account_id = %self.cfg.account_id,
                display_len = display_text.len(),
                "Telegram tool placeholder skipped because it would exceed message length"
            );
            state.stream_text.clear_tool_placeholder();
            return;
        }

        if let Some(msg_id) = state.message_id {
            match edit_message_text(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                msg_id,
                &display_text,
                Some(MARKDOWN_V2_PARSE_MODE),
                &self.cfg.api_base,
            )
            .await
            {
                Ok(()) => {
                    state.last_rendered_text = display_text;
                    state.stream_text.mark_flushed(Instant::now());
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
                    state.stream_text.mark_flushed(Instant::now());
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
        if !state.stream_text.is_tool_placeholder_active() {
            return;
        }

        state.stream_text.clear_tool_placeholder();

        let display_text = strip_deliverable_markdown_images(state.stream_text.accumulated_text());
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
                    state.stream_text.mark_flushed(Instant::now());
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
            Some(MARKDOWN_V2_PARSE_MODE),
            &self.cfg.api_base,
        )
        .await
        {
            Ok(()) => {
                state.last_rendered_text = display_text;
                state.stream_text.mark_flushed(Instant::now());
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

    async fn process_image_generation_result(
        &self,
        thread_id: &str,
        state: &mut StreamState,
        message: ProviderMessage,
    ) {
        let Some(image) = extract_image_generation_result(&message) else {
            return;
        };

        if state.stream_text.is_tool_placeholder_active() {
            self.clear_tool_placeholder(state).await;
        }
        let prior_text_msg_id = state.message_id;

        let image_path = match write_generated_image_temp("telegram", &image).await {
            Ok(path) => path,
            Err(error) => {
                warn!(
                    account_id = %self.cfg.account_id,
                    error = %error,
                    "failed to write Telegram generated image temp file"
                );
                return;
            }
        };

        let send_result = send_photo(
            TelegramSendTarget::new(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                self.cfg.outbound_thread_id,
                &self.cfg.api_base,
            ),
            &image_path,
            None,
            self.effective_reply_to(),
        )
        .await;
        let _ = tokio::fs::remove_file(&image_path).await;

        match send_result {
            Ok(photo_msg_id) => {
                let mut delivered_msg_ids = Vec::new();
                if let Some(msg_id) = prior_text_msg_id {
                    delivered_msg_ids.push(msg_id);
                }
                delivered_msg_ids.push(photo_msg_id);
                self.record_outbound_messages(thread_id, &delivered_msg_ids)
                    .await;
                Self::reset_for_fresh_message(state);
            }
            Err(error) => {
                warn!(
                    account_id = %self.cfg.account_id,
                    chat_id = self.cfg.chat_id,
                    error = %error,
                    "failed to send Telegram generated image"
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
        let display_text = strip_deliverable_markdown_images(display_text);
        if display_text.trim().is_empty() {
            return false;
        }

        let chunks = split_message(&display_text, MAX_MESSAGE_LENGTH);
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
                Some(MARKDOWN_V2_PARSE_MODE),
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

        state.stream_text.set_accumulated_text(active_chunk.clone());
        state.last_rendered_text = active_chunk;
        state.stream_text.mark_flushed(Instant::now());

        self.record_outbound_messages(thread_id, &finalized_msg_ids)
            .await;
        true
    }

    async fn process_event(self: &Arc<Self>, event: StreamEvent, thread_id: &str) {
        let mut state = self.state.lock().await;

        let is_final = match event {
            StreamEvent::SessionBound { .. } => return,
            StreamEvent::Boundary { kind, .. } => match kind {
                StreamBoundaryKind::UserAck => {
                    self.process_boundary(thread_id, &mut state).await;
                    return;
                }
                StreamBoundaryKind::AssistantSegment => {
                    crate::streaming_core::apply_stream_boundary_text(
                        &mut state.markdown_image_scan_text,
                        StreamBoundaryKind::AssistantSegment,
                    );
                    state
                        .stream_text
                        .apply_boundary(StreamBoundaryKind::AssistantSegment);
                    false
                }
            },
            StreamEvent::Delta { text } => {
                if text.is_empty() {
                    return;
                }
                state.markdown_image_scan_text.push_str(&text);
                let decision = state.stream_text.on_delta(&text, Instant::now());
                state.finalized = false;
                match decision {
                    PluginStreamSendDecision::Wait => return,
                    PluginStreamSendDecision::ScheduleFlush { after } => {
                        let shared = self.clone();
                        let thread_id = thread_id.to_owned();
                        tokio::spawn(async move {
                            tokio::time::sleep(after).await;
                            shared.flush_pending_stream_text(&thread_id).await;
                        });
                        return;
                    }
                    PluginStreamSendDecision::FlushNow { .. } => false,
                }
            }
            StreamEvent::ToolUse { message } => {
                if should_hide_tool_call_display(&message) {
                    return;
                }
                self.process_tool_use(thread_id, &mut state, message).await;
                return;
            }
            StreamEvent::ToolResult { message } => {
                self.process_image_generation_result(thread_id, &mut state, message)
                    .await;
                return;
            }
            StreamEvent::ThreadTitleUpdated { .. } => return,
            StreamEvent::Done => {
                let _ = state.stream_text.on_done(Instant::now());
                true
            }
        };

        if is_final && state.stream_text.is_tool_placeholder_active() {
            self.clear_tool_placeholder(&mut state).await;
            state.finalized = true;
            if let Some(msg_id) = state.message_id {
                self.record_outbound_messages(thread_id, &[msg_id]).await;
            }
            self.send_markdown_images_from_state(thread_id, &mut state)
                .await;
            return;
        }

        let content_text = render_stream_content_text(&state);

        if content_text.trim().is_empty() {
            if is_final {
                self.send_markdown_images_from_state(thread_id, &mut state)
                    .await;
            }
            return;
        }

        if self
            .roll_stream_segment_if_needed(thread_id, &mut state, &content_text)
            .await
        {
            if is_final {
                state.finalized = true;
            }
            if is_final && let Some(msg_id) = state.message_id {
                self.record_outbound_messages(thread_id, &[msg_id]).await;
            }
            if is_final {
                self.send_markdown_images_from_state(thread_id, &mut state)
                    .await;
            }
            return;
        }

        let display_text = render_stream_display_text(&state);
        if display_text.trim().is_empty() {
            if is_final {
                self.delete_runtime_only_message(&mut state).await;
                state.finalized = true;
                self.send_markdown_images_from_state(thread_id, &mut state)
                    .await;
            }
            return;
        }

        if let Some(msg_id) = state.message_id {
            if display_text.trim() == state.last_rendered_text.trim() {
                if is_final {
                    state.finalized = true;
                    if let Some(msg_id) = state.message_id {
                        self.record_outbound_messages(thread_id, &[msg_id]).await;
                    }
                    self.send_markdown_images_from_state(thread_id, &mut state)
                        .await;
                }
                return;
            }

            match edit_message_text(
                &self.cfg.http,
                &self.cfg.token,
                self.cfg.chat_id,
                msg_id,
                &display_text,
                Some(MARKDOWN_V2_PARSE_MODE),
                &self.cfg.api_base,
            )
            .await
            {
                Ok(()) => {
                    state.last_rendered_text = display_text.clone();
                    state.stream_text.mark_flushed(Instant::now());
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
                        state.stream_text.mark_flushed(Instant::now());
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
        }

        if is_final && let Some(msg_id) = state.message_id {
            self.record_outbound_messages(thread_id, &[msg_id]).await;
        }
        if is_final {
            self.send_markdown_images_from_state(thread_id, &mut state)
                .await;
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
