use super::*;

pub(super) const DISCORD_TOOL_PLACEHOLDER_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct DiscordStreamingCallbackConfig {
    pub sender: DiscordSender,
    pub chat_id: String,
    pub reply_to_message_id: Option<String>,
}

struct DiscordStreamState {
    stream_text: PluginStreamSendState,
    markdown_image_scan_text: String,
    sent_markdown_image_keys: HashSet<String>,
    message_id: Option<String>,
    last_rendered_text: String,
    finalized: bool,
}

fn discord_stream_send_policy() -> PluginStreamSendPolicy {
    PluginStreamSendPolicy::buffered_until_tool_or_done()
        .with_tool_min_flush_interval(DISCORD_TOOL_PLACEHOLDER_UPDATE_INTERVAL)
}

impl Default for DiscordStreamState {
    fn default() -> Self {
        Self {
            stream_text: PluginStreamSendState::new(discord_stream_send_policy()),
            markdown_image_scan_text: String::new(),
            sent_markdown_image_keys: HashSet::new(),
            message_id: None,
            last_rendered_text: String::new(),
            finalized: false,
        }
    }
}

struct DiscordStreamingCallbackShared {
    cfg: DiscordStreamingCallbackConfig,
    state: Mutex<DiscordStreamState>,
}

impl DiscordStreamingCallbackShared {
    fn reset_for_fresh_message(state: &mut DiscordStreamState) {
        state.stream_text = PluginStreamSendState::new(discord_stream_send_policy());
        state.markdown_image_scan_text.clear();
        state.sent_markdown_image_keys.clear();
        state.message_id = None;
        state.last_rendered_text.clear();
        state.finalized = false;
    }

    async fn send_initial_text(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        display_text: &str,
    ) -> bool {
        if display_text.trim().is_empty() {
            return true;
        }
        match self
            .cfg
            .sender
            .send_text(
                &self.cfg.chat_id,
                display_text,
                self.cfg.reply_to_message_id.as_deref(),
            )
            .await
        {
            Ok(message_ids) => {
                state.message_id = message_ids.last().cloned();
                state.last_rendered_text = display_text.to_owned();
                true
            }
            Err(error) => {
                warn!(target: "garyx_channels::discord",
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to send Discord streamed response"
                );
                false
            }
        }
    }

    async fn edit_existing_text(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        display_text: &str,
    ) -> bool {
        let Some(message_id) = state.message_id.as_deref() else {
            return self.send_initial_text(thread_id, state, display_text).await;
        };
        if display_text.trim().is_empty() || display_text.trim() == state.last_rendered_text.trim()
        {
            return true;
        }
        match self
            .cfg
            .sender
            .edit_text(&self.cfg.chat_id, message_id, display_text)
            .await
        {
            Ok(edited_id) => {
                state.last_rendered_text = display_text.to_owned();
                let _ = edited_id;
                true
            }
            Err(error) => {
                warn!(target: "garyx_channels::discord",
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to edit Discord streamed response"
                );
                state.message_id = None;
                state.last_rendered_text.clear();
                false
            }
        }
    }

    async fn apply_stream_send_decision(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        decision: PluginStreamSendDecision,
        mark_tool_flush: bool,
    ) {
        match decision {
            PluginStreamSendDecision::Wait | PluginStreamSendDecision::ScheduleFlush { .. } => {}
            PluginStreamSendDecision::FlushNow { content_text } => {
                let display_text = discord_editable_text(&content_text);
                if state.message_id.is_none() {
                    let _ = self
                        .send_initial_text(thread_id, state, &display_text)
                        .await;
                } else {
                    let _ = self
                        .edit_existing_text(thread_id, state, &display_text)
                        .await;
                }
                if mark_tool_flush {
                    state.stream_text.mark_tool_flushed(Instant::now());
                } else {
                    state.stream_text.mark_flushed(Instant::now());
                }
            }
        }
    }

    async fn flush_pending_tool_placeholder(self: Arc<Self>, thread_id: String) {
        let mut state = self.state.lock().await;
        if state.finalized {
            return;
        }
        let decision = state.stream_text.scheduled_tool_flush();
        self.apply_stream_send_decision(&thread_id, &mut state, decision, true)
            .await;
    }

    async fn delete_runtime_only_message(&self, thread_id: &str, state: &mut DiscordStreamState) {
        let Some(message_id) = state.message_id.clone() else {
            return;
        };

        match self
            .cfg
            .sender
            .delete_text(&self.cfg.chat_id, &message_id)
            .await
        {
            Ok(()) => {
                state.message_id = None;
                state.last_rendered_text.clear();
            }
            Err(error) => {
                warn!(target: "garyx_channels::discord",
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to delete Discord runtime-only stream message"
                );
                state.message_id = None;
                state.last_rendered_text.clear();
            }
        }
    }

    async fn clear_tool_placeholder(&self, thread_id: &str, state: &mut DiscordStreamState) {
        if !state.stream_text.is_tool_placeholder_active() {
            return;
        }

        state.stream_text.clear_tool_placeholder();
        let display_text = discord_editable_text(state.stream_text.accumulated_text());
        let Some(message_id) = state.message_id.clone() else {
            state.last_rendered_text.clear();
            return;
        };

        if display_text.trim().is_empty() {
            match self
                .cfg
                .sender
                .delete_text(&self.cfg.chat_id, &message_id)
                .await
            {
                Ok(()) => {
                    state.message_id = None;
                    state.last_rendered_text.clear();
                }
                Err(error) => {
                    warn!(target: "garyx_channels::discord",
                        account_id = %self.cfg.sender.account_id,
                        chat_id = %self.cfg.chat_id,
                        thread_id,
                        error = %error,
                        "failed to delete Discord tool placeholder"
                    );
                }
            }
            return;
        }

        self.edit_existing_text(thread_id, state, &display_text)
            .await;
    }

    async fn process_boundary(&self, thread_id: &str, state: &mut DiscordStreamState) {
        let boundary_content_text = state.stream_text.accumulated_text().trim().to_owned();
        let boundary_text = strip_deliverable_markdown_images(&boundary_content_text);

        state.stream_text.clear_tool_placeholder();

        if boundary_content_text.is_empty() {
            self.delete_runtime_only_message(thread_id, state).await;
            Self::reset_for_fresh_message(state);
            return;
        }

        if boundary_text.trim().is_empty() {
            self.delete_runtime_only_message(thread_id, state).await;
            self.send_markdown_images_from_state(thread_id, state).await;
            Self::reset_for_fresh_message(state);
            return;
        }

        self.finalize_text(thread_id, state).await;
        self.send_markdown_images_from_state(thread_id, state).await;
        Self::reset_for_fresh_message(state);
    }

    async fn finalize_text(&self, thread_id: &str, state: &mut DiscordStreamState) {
        let display_text = strip_deliverable_markdown_images(state.stream_text.accumulated_text());
        if display_text.trim().is_empty() {
            return;
        }
        let chunks = split_discord_message(&display_text);
        let first_chunk = chunks.first().cloned().unwrap_or_default();
        if state.message_id.is_some() {
            if !self
                .edit_existing_text(thread_id, state, &first_chunk)
                .await
            {
                let _ = self
                    .send_initial_text(thread_id, state, &display_text)
                    .await;
                return;
            }
            if chunks.len() > 1 {
                match self
                    .cfg
                    .sender
                    .send_text(&self.cfg.chat_id, &chunks[1..].join(""), None)
                    .await
                {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(target: "garyx_channels::discord",
                            account_id = %self.cfg.sender.account_id,
                            chat_id = %self.cfg.chat_id,
                            thread_id,
                            error = %error,
                            "failed to send Discord overflow stream chunks"
                        );
                    }
                }
            }
        } else {
            let _ = self
                .send_initial_text(thread_id, state, &display_text)
                .await;
        }
    }

    async fn send_markdown_images_from_state(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
    ) {
        let image_refs = extract_markdown_image_refs(&state.markdown_image_scan_text);
        if image_refs.is_empty() {
            return;
        }

        for image_ref in image_refs {
            let key = markdown_image_key(&image_ref);
            if !state.sent_markdown_image_keys.insert(key) {
                continue;
            }
            let (path, remove_after_send) = match &image_ref.target {
                MarkdownImageTarget::Local(path) => (path.clone(), false),
                MarkdownImageTarget::Remote(url) => {
                    let Some(path) = download_remote_markdown_image(
                        &self.cfg.sender.http,
                        url,
                        image_ref.alt.as_deref(),
                    )
                    .await
                    else {
                        continue;
                    };
                    (path, true)
                }
            };
            match self
                .cfg
                .sender
                .send_file(
                    &self.cfg.chat_id,
                    &path,
                    image_ref.alt.as_deref(),
                    self.cfg.reply_to_message_id.as_deref(),
                )
                .await
            {
                Ok(_) => {}
                Err(error) => {
                    warn!(target: "garyx_channels::discord",
                        account_id = %self.cfg.sender.account_id,
                        chat_id = %self.cfg.chat_id,
                        thread_id,
                        path = %path.display(),
                        error = %error,
                        "failed to send Discord markdown image"
                    );
                }
            }
            if remove_after_send {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
    }

    async fn process_generated_image_result(
        &self,
        thread_id: &str,
        state: &mut DiscordStreamState,
        message: ProviderMessage,
    ) {
        let Some(image) = extract_image_generation_result(&message) else {
            return;
        };
        if state.stream_text.is_tool_placeholder_active() {
            self.clear_tool_placeholder(thread_id, state).await;
        }
        let image_path = match write_generated_image_temp("discord", &image).await {
            Ok(path) => path,
            Err(error) => {
                warn!(target: "garyx_channels::discord",
                    account_id = %self.cfg.sender.account_id,
                    error = %error,
                    "failed to write Discord generated image temp file"
                );
                return;
            }
        };
        let send_result = self
            .cfg
            .sender
            .send_file(
                &self.cfg.chat_id,
                &image_path,
                None,
                self.cfg.reply_to_message_id.as_deref(),
            )
            .await;
        let _ = tokio::fs::remove_file(&image_path).await;
        match send_result {
            Ok(_) => {}
            Err(error) => {
                warn!(target: "garyx_channels::discord",
                    account_id = %self.cfg.sender.account_id,
                    chat_id = %self.cfg.chat_id,
                    thread_id,
                    error = %error,
                    "failed to send Discord generated image"
                );
            }
        }
    }

    async fn process_event(self: &Arc<Self>, event: StreamEvent, thread_id: &str) {
        let mut state = self.state.lock().await;
        match event {
            StreamEvent::SessionBound { .. } | StreamEvent::ThreadTitleUpdated { .. } => {}
            StreamEvent::ToolUse { message } => {
                if should_hide_tool_call_display(&message) {
                    return;
                }
                let decision = state.stream_text.on_tool_call(&message, Instant::now());
                state.finalized = false;
                if let PluginStreamSendDecision::ScheduleFlush { after } = decision {
                    let shared = self.clone();
                    let thread_id = thread_id.to_owned();
                    tokio::spawn(async move {
                        tokio::time::sleep(after).await;
                        shared.flush_pending_tool_placeholder(thread_id).await;
                    });
                } else {
                    self.apply_stream_send_decision(thread_id, &mut state, decision, true)
                        .await;
                }
            }
            StreamEvent::ToolResult { message } => {
                self.process_generated_image_result(thread_id, &mut state, message)
                    .await;
            }
            StreamEvent::Boundary { kind, .. } => match kind {
                StreamBoundaryKind::UserAck => {
                    self.process_boundary(thread_id, &mut state).await;
                }
                StreamBoundaryKind::AssistantSegment => {
                    crate::streaming_core::apply_stream_boundary_text(
                        &mut state.markdown_image_scan_text,
                        StreamBoundaryKind::AssistantSegment,
                    );
                    state
                        .stream_text
                        .apply_boundary(StreamBoundaryKind::AssistantSegment);
                }
            },
            StreamEvent::Delta { text } => {
                if text.is_empty() {
                    return;
                }
                state.markdown_image_scan_text.push_str(&text);
                let decision = state.stream_text.on_delta(&text, Instant::now());
                state.finalized = false;
                self.apply_stream_send_decision(thread_id, &mut state, decision, false)
                    .await;
            }
            StreamEvent::Done => {
                if state.finalized {
                    return;
                }
                if state.stream_text.is_tool_placeholder_active() {
                    self.clear_tool_placeholder(thread_id, &mut state).await;
                }
                state.finalized = true;
                self.finalize_text(thread_id, &mut state).await;
                self.send_markdown_images_from_state(thread_id, &mut state)
                    .await;
                Self::reset_for_fresh_message(&mut state);
            }
        }
    }
}

pub(crate) fn build_discord_response_callback(
    cfg: DiscordStreamingCallbackConfig,
) -> (
    Arc<dyn Fn(StreamEvent) + Send + Sync>,
    watch::Sender<String>,
) {
    let shared = Arc::new(DiscordStreamingCallbackShared {
        cfg,
        state: Mutex::new(DiscordStreamState::default()),
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
            if thread_id.is_empty() {
                continue;
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
