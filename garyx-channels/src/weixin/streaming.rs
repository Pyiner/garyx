//! Weixin streaming presentation: the LiveMessage state machine and
//! budgets, the streaming/non-streaming consumers, and the response
//! callback builder. Decision values live in
//! plugin_tools::TextFlushGate (Phase-6 B3); this module owns the
//! measurements and rendering. Moved verbatim from weixin.rs
//! (Phase-7 pure code motion).

use super::*;

pub(super) const STREAM_UPDATE_TICK_MS: u64 = 200;
pub(super) const STREAM_UPDATE_MIN_INTERVAL_MS: u64 = 800;
pub(super) const STREAM_UPDATE_MIN_DELTA_CHARS: usize = 12;
pub(super) const STREAM_UPDATE_INACTIVITY_FORCE_FINISH_MS: u64 = 15_000;

/// The one production construction point of Weixin's streaming text
/// flush decision values (Phase-6 B3): the rule lives in the shared
/// engine, the measurements stay channel-side.
pub(super) fn weixin_streaming_text_flush_gate() -> crate::plugin_tools::TextFlushGate {
    crate::plugin_tools::TextFlushGate {
        min_flush_interval: Duration::from_millis(STREAM_UPDATE_MIN_INTERVAL_MS),
        min_delta_chars: STREAM_UPDATE_MIN_DELTA_CHARS,
        flush_on_sentence_terminator: true,
    }
}
pub(super) const LIVE_MESSAGE_MAX_GENERATING_SENDS: u8 = 7;

pub(super) fn apply_weixin_stream_boundary(
    stream_text: &mut String,
    kind: StreamBoundaryKind,
) -> crate::streaming_core::BoundaryTextEffect {
    crate::streaming_core::apply_stream_boundary_text(stream_text, kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LiveMessageState {
    Pristine,
    Updating,
    Finalized,
    DeliveryDisabled { reason: PoisonReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PoisonReason {
    TokenExhausted,
    SessionPaused,
    HttpFailure,
}

impl PoisonReason {
    pub(super) fn metric_label(self) -> &'static str {
        match self {
            Self::TokenExhausted => "poisoned_token_exhausted",
            Self::SessionPaused => "poisoned_session_paused",
            Self::HttpFailure => "poisoned_http",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinalizeReason {
    Done,
    ToolBoundary,
    UserAck,
    #[allow(dead_code)]
    Media,
    BudgetMessage,
    BudgetToken,
    Inactivity,
    Poisoned(PoisonReason),
}

impl FinalizeReason {
    pub(super) fn metric_label(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::ToolBoundary => "tool_boundary",
            Self::UserAck => "user_ack",
            Self::Media => "media",
            Self::BudgetMessage => "budget_msg",
            Self::BudgetToken => "budget_token",
            Self::Inactivity => "inactivity",
            Self::Poisoned(reason) => reason.metric_label(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct LiveMessage {
    pub(super) client_id: String,
    pub(super) context_token: String,
    pub(super) text_visible: String,
    pub(super) text_raw: String,
    pub(super) pending_media_refs: Vec<OutboundMediaRef>,
    pub(super) last_sent_visible: String,
    pub(super) last_sent_at: Option<Instant>,
    pub(super) last_delta_at: Option<Instant>,
    pub(super) sends_used: u8,
    pub(super) state: LiveMessageState,
}

impl LiveMessage {
    pub(super) async fn open(context_token: String) -> Self {
        let state = if context_token.trim().is_empty()
            || token_sends_remaining(context_token.trim()).await == 0
        {
            LiveMessageState::DeliveryDisabled {
                reason: PoisonReason::TokenExhausted,
            }
        } else {
            LiveMessageState::Pristine
        };
        Self {
            client_id: uuid::Uuid::new_v4().to_string(),
            context_token,
            text_visible: String::new(),
            text_raw: String::new(),
            pending_media_refs: Vec::new(),
            last_sent_visible: String::new(),
            last_sent_at: None,
            last_delta_at: None,
            sends_used: 0,
            state,
        }
    }

    pub(super) fn append_delta(
        &mut self,
        delta: &str,
        sent_media_refs: &HashSet<String>,
        now: Instant,
    ) {
        self.text_raw = merge_stream_text(&self.text_raw, delta);
        self.collect_markdown_media_refs(sent_media_refs);
        self.text_visible = markdown_to_plain_text(&self.text_raw).trim().to_owned();
        self.last_delta_at = Some(now);
    }

    pub(super) fn append_soft_boundary(&mut self) {
        if self.text_raw.trim().is_empty() {
            return;
        }
        self.text_raw.push_str("\n\n");
        self.text_visible = markdown_to_plain_text(&self.text_raw).trim().to_owned();
    }

    pub(super) fn clear_text(&mut self) {
        self.text_raw.clear();
        self.text_visible.clear();
        self.last_sent_visible.clear();
        self.last_sent_at = None;
        self.last_delta_at = None;
        self.pending_media_refs.clear();
    }

    pub(super) fn keep_only_sent_text_for_finish(&mut self) {
        self.text_raw = self.last_sent_visible.clone();
        self.text_visible = self.last_sent_visible.clone();
        self.pending_media_refs.clear();
        self.last_delta_at = None;
    }

    pub(super) fn collect_markdown_media_refs(&mut self, sent_media_refs: &HashSet<String>) {
        for media_ref in extract_markdown_media_refs(&self.text_raw) {
            let dedupe_key = media_ref.dedupe_key();
            if sent_media_refs.contains(&dedupe_key)
                || self
                    .pending_media_refs
                    .iter()
                    .any(|existing| existing.dedupe_key() == dedupe_key)
            {
                continue;
            }
            self.pending_media_refs.push(media_ref);
        }
    }

    pub(super) fn collect_provider_media_refs(
        &mut self,
        refs: Vec<OutboundMediaRef>,
        sent_media_refs: &HashSet<String>,
    ) {
        for media_ref in refs {
            let dedupe_key = media_ref.dedupe_key();
            if sent_media_refs.contains(&dedupe_key)
                || self
                    .pending_media_refs
                    .iter()
                    .any(|existing| existing.dedupe_key() == dedupe_key)
            {
                continue;
            }
            self.pending_media_refs.push(media_ref);
        }
    }

    pub(super) fn pending_delta_chars(&self) -> usize {
        self.text_visible
            .chars()
            .count()
            .saturating_sub(self.last_sent_visible.chars().count())
    }

    pub(super) fn has_buffered_visible(&self) -> bool {
        self.text_visible != self.last_sent_visible
    }

    pub(super) fn sentence_terminated_since_last_send(&self) -> bool {
        let suffix = if self.last_sent_visible.is_empty()
            || !self.text_visible.starts_with(&self.last_sent_visible)
        {
            self.text_visible.as_str()
        } else {
            &self.text_visible[self.last_sent_visible.len()..]
        };
        suffix
            .trim_end()
            .chars()
            .last()
            .is_some_and(is_stream_update_sentence_terminator)
    }

    pub(super) fn has_unterminated_markdown_image_ref_tail(&self) -> bool {
        unterminated_markdown_image_tail_regex().is_match(&self.text_raw)
    }

    pub(super) async fn should_send_generating(&self, now: Instant) -> bool {
        if !matches!(
            self.state,
            LiveMessageState::Pristine | LiveMessageState::Updating
        ) {
            return false;
        }
        if self.sends_used >= LIVE_MESSAGE_MAX_GENERATING_SENDS {
            return false;
        }
        if token_sends_remaining(self.context_token.trim()).await <= 1 {
            return false;
        }
        if !self.has_buffered_visible() || self.has_unterminated_markdown_image_ref_tail() {
            return false;
        }
        weixin_streaming_text_flush_gate().should_flush(
            self.last_sent_at.map(|last| now.duration_since(last)),
            self.pending_delta_chars(),
            self.sentence_terminated_since_last_send(),
        )
    }

    pub(super) fn should_force_inactivity_finish(&self, now: Instant) -> bool {
        matches!(self.state, LiveMessageState::Updating)
            && self.last_delta_at.is_some_and(|last_delta| {
                now.duration_since(last_delta)
                    >= Duration::from_millis(STREAM_UPDATE_INACTIVITY_FORCE_FINISH_MS)
            })
    }

    pub(super) async fn needs_budget_finalize(&self) -> Option<FinalizeReason> {
        if !matches!(self.state, LiveMessageState::Updating) {
            return None;
        }
        if self.sends_used >= LIVE_MESSAGE_MAX_GENERATING_SENDS {
            return Some(FinalizeReason::BudgetMessage);
        }
        if token_sends_remaining(self.context_token.trim()).await <= 1 {
            return Some(FinalizeReason::BudgetToken);
        }
        None
    }

    pub(super) fn take_poisoned_text(&mut self) -> Option<String> {
        if !matches!(self.state, LiveMessageState::DeliveryDisabled { .. }) {
            return None;
        }
        let text = self.text_visible.trim().to_owned();
        self.text_visible.clear();
        self.text_raw.clear();
        if text.is_empty() { None } else { Some(text) }
    }
}

pub(super) fn is_stream_update_sentence_terminator(ch: char) -> bool {
    matches!(ch, '.' | '?' | '!' | '。' | '？' | '！' | '…' | ':' | '：')
}

pub(super) fn unterminated_markdown_image_tail_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*$").expect("valid markdown tail regex"))
}

#[derive(Clone)]
pub(super) struct WeixinStreamConsumerContext {
    pub(super) http: Client,
    pub(super) account: WeixinAccount,
    pub(super) account_id: String,
    pub(super) user_id: String,
    pub(super) context_token: String,
    pub(super) thread_id: Arc<std::sync::Mutex<String>>,
    pub(super) typing_ticket: Option<String>,
    pub(super) running: Arc<AtomicBool>,
}

pub(super) enum LiveTextSendResult {
    Sent,
    Noop,
    Poisoned,
}

pub(super) fn current_stream_thread_id(ctx: &WeixinStreamConsumerContext) -> String {
    match ctx.thread_id.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => String::new(),
    }
}

pub(super) async fn resolve_stream_context_token(
    ctx: &WeixinStreamConsumerContext,
    prefer_latest: bool,
) -> String {
    let thread_id = current_stream_thread_id(ctx);
    let persisted = get_context_token_for_thread(
        &ctx.account_id,
        &ctx.user_id,
        if thread_id.is_empty() {
            None
        } else {
            Some(thread_id.as_str())
        },
    )
    .await
    .and_then(|value| {
        let token = value.trim();
        if token.is_empty() {
            None
        } else {
            Some(token.to_owned())
        }
    });
    let captured = ctx.context_token.trim();
    if prefer_latest {
        persisted.or_else(|| (!captured.is_empty()).then(|| captured.to_owned()))
    } else if captured.is_empty() {
        persisted
    } else {
        Some(captured.to_owned())
    }
    .unwrap_or_default()
}

pub(super) async fn open_live_message_for_context(
    ctx: &WeixinStreamConsumerContext,
    prefer_latest_token: bool,
) -> LiveMessage {
    LiveMessage::open(resolve_stream_context_token(ctx, prefer_latest_token).await).await
}

pub(super) async fn ensure_stream_typing(
    ctx: &WeixinStreamConsumerContext,
    typing_keepalive_task: &mut Option<JoinHandle<()>>,
    typing_active: &mut bool,
) {
    if *typing_active {
        return;
    }
    let Some(ticket) = ctx.typing_ticket.clone() else {
        return;
    };
    if let Err(error) = send_typing_status(&ctx.http, &ctx.account, &ctx.user_id, &ticket, 1).await
    {
        debug!(
            account_id = %ctx.account_id,
            user_id = %ctx.user_id,
            error = %error,
            "failed to send weixin typing start"
        );
    }
    let http = ctx.http.clone();
    let account = ctx.account.clone();
    let user_id = ctx.user_id.clone();
    *typing_keepalive_task = Some(tokio::spawn(async move {
        let mut logged_failure = false;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if let Err(error) = send_typing_status(&http, &account, &user_id, &ticket, 1).await {
                if !logged_failure {
                    debug!(error = %error, "weixin typing keepalive failed (suppressing further)");
                    logged_failure = true;
                }
            } else {
                logged_failure = false;
            }
        }
    }));
    *typing_active = true;
}

pub(super) async fn stop_stream_typing(
    ctx: &WeixinStreamConsumerContext,
    typing_keepalive_task: &mut Option<JoinHandle<()>>,
    typing_active: &mut bool,
) {
    if let Some(task) = typing_keepalive_task.take() {
        task.abort();
    }
    if !*typing_active {
        return;
    }
    *typing_active = false;
    if let Some(ticket) = ctx.typing_ticket.as_deref()
        && let Err(error) =
            send_typing_status(&ctx.http, &ctx.account, &ctx.user_id, ticket, 2).await
    {
        debug!(
            account_id = %ctx.account_id,
            user_id = %ctx.user_id,
            error = %error,
            "failed to stop weixin typing"
        );
    }
}

pub(super) fn classify_text_send_error(error: &ChannelError) -> PoisonReason {
    let value = error.to_string();
    if value.contains("ret=-14") || value.contains("session paused") {
        PoisonReason::SessionPaused
    } else if value.contains("ret=-2")
        || value.contains("context_token exhausted")
        || value.contains("send limit")
        || value.contains("context_token is missing")
    {
        PoisonReason::TokenExhausted
    } else {
        PoisonReason::HttpFailure
    }
}

pub(super) async fn send_live_generating(
    ctx: &WeixinStreamConsumerContext,
    live: &mut LiveMessage,
    now: Instant,
    calls_used: &mut u32,
) -> LiveTextSendResult {
    if !live.should_send_generating(now).await {
        return LiveTextSendResult::Noop;
    }
    match send_text_message_with_state(
        &ctx.http,
        &ctx.account,
        &ctx.user_id,
        &live.text_visible,
        Some(live.context_token.as_str()),
        &live.client_id,
        1,
    )
    .await
    {
        Ok(()) => {
            live.state = LiveMessageState::Updating;
            live.sends_used = live.sends_used.saturating_add(1);
            live.last_sent_visible = live.text_visible.clone();
            live.last_sent_at = Some(now);
            *calls_used = calls_used.saturating_add(1);
            LiveTextSendResult::Sent
        }
        Err(error) => {
            let reason = classify_text_send_error(&error);
            warn!(
                account_id = %ctx.account_id,
                user_id = %ctx.user_id,
                reason = reason.metric_label(),
                error = %error,
                "weixin streaming GENERATING send failed; disabling live message delivery"
            );
            live.state = LiveMessageState::DeliveryDisabled { reason };
            LiveTextSendResult::Poisoned
        }
    }
}

pub(super) async fn finalize_live_message(
    ctx: &WeixinStreamConsumerContext,
    live: &mut LiveMessage,
    reason: FinalizeReason,
    calls_used: &mut u32,
) -> LiveTextSendResult {
    match live.state {
        LiveMessageState::Finalized => return LiveTextSendResult::Noop,
        LiveMessageState::DeliveryDisabled { reason } => {
            record_weixin_finalize_reason(FinalizeReason::Poisoned(reason)).await;
            return LiveTextSendResult::Noop;
        }
        LiveMessageState::Pristine if live.text_visible.trim().is_empty() => {
            live.state = LiveMessageState::Finalized;
            return LiveTextSendResult::Noop;
        }
        LiveMessageState::Pristine | LiveMessageState::Updating => {}
    }

    if token_sends_remaining(live.context_token.trim()).await == 0 {
        live.state = LiveMessageState::DeliveryDisabled {
            reason: PoisonReason::TokenExhausted,
        };
        record_weixin_finalize_reason(FinalizeReason::Poisoned(PoisonReason::TokenExhausted)).await;
        return LiveTextSendResult::Poisoned;
    }

    match send_text_message_with_state(
        &ctx.http,
        &ctx.account,
        &ctx.user_id,
        &live.text_visible,
        Some(live.context_token.as_str()),
        &live.client_id,
        2,
    )
    .await
    {
        Ok(()) => {
            live.state = LiveMessageState::Finalized;
            live.sends_used = live.sends_used.saturating_add(1);
            live.last_sent_visible = live.text_visible.clone();
            live.last_sent_at = Some(Instant::now());
            *calls_used = calls_used.saturating_add(1);
            record_weixin_finalize_reason(reason).await;
            LiveTextSendResult::Sent
        }
        Err(error) => {
            let poison = classify_text_send_error(&error);
            warn!(
                account_id = %ctx.account_id,
                user_id = %ctx.user_id,
                reason = poison.metric_label(),
                error = %error,
                "weixin streaming FINISH send failed; disabling live message delivery"
            );
            live.state = LiveMessageState::DeliveryDisabled { reason: poison };
            record_weixin_finalize_reason(FinalizeReason::Poisoned(poison)).await;
            LiveTextSendResult::Poisoned
        }
    }
}

pub(super) async fn drain_live_media(
    ctx: &WeixinStreamConsumerContext,
    live: &mut LiveMessage,
    sent_media_refs: &mut HashSet<String>,
    calls_used: &mut u32,
) {
    let refs = std::mem::take(&mut live.pending_media_refs);
    if refs.is_empty() {
        return;
    }
    if matches!(live.state, LiveMessageState::DeliveryDisabled { .. }) {
        record_weixin_media_dropped(refs.len());
        warn!(
            account_id = %ctx.account_id,
            user_id = %ctx.user_id,
            dropped = refs.len(),
            "dropping weixin media refs because live text delivery is disabled"
        );
        return;
    }

    let mut remaining_refs = refs.len();
    for media_ref in refs {
        remaining_refs = remaining_refs.saturating_sub(1);
        if token_sends_remaining(live.context_token.trim()).await == 0 {
            record_weixin_media_dropped(remaining_refs + 1);
            warn!(
                account_id = %ctx.account_id,
                user_id = %ctx.user_id,
                "dropping weixin media refs because context_token budget is exhausted"
            );
            break;
        }
        let dedupe_key = media_ref.dedupe_key();
        if sent_media_refs.contains(&dedupe_key) {
            continue;
        }
        let media_bytes = match load_media_bytes(&ctx.http, &media_ref).await {
            Ok(bytes) => bytes,
            Err(error) => {
                record_weixin_media_dropped(1);
                warn!(
                    account_id = %ctx.account_id,
                    user_id = %ctx.user_id,
                    error = %error,
                    "failed to load weixin media reference"
                );
                continue;
            }
        };
        let uploaded = match upload_media_to_cdn(
            &ctx.http,
            &ctx.account,
            &ctx.user_id,
            &media_bytes,
            media_ref.classify_media_type(),
            media_ref.file_name(),
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                record_weixin_media_dropped(1);
                warn!(
                    account_id = %ctx.account_id,
                    user_id = %ctx.user_id,
                    error = %error,
                    "failed to upload weixin media reference"
                );
                continue;
            }
        };
        match send_media_message(
            &ctx.http,
            &ctx.account,
            &ctx.user_id,
            &uploaded,
            "",
            Some(live.context_token.as_str()),
        )
        .await
        {
            Ok(_) => {
                sent_media_refs.insert(dedupe_key);
                *calls_used = calls_used.saturating_add(1);
            }
            Err(error) => {
                let reason = classify_text_send_error(&error);
                record_weixin_media_dropped(1);
                warn!(
                    account_id = %ctx.account_id,
                    user_id = %ctx.user_id,
                    reason = reason.metric_label(),
                    error = %error,
                    "failed to send weixin media message"
                );
                if matches!(
                    reason,
                    PoisonReason::TokenExhausted | PoisonReason::SessionPaused
                ) {
                    if remaining_refs > 0 {
                        record_weixin_media_dropped(remaining_refs);
                    }
                    break;
                }
            }
        }
    }
}

pub(super) fn collect_poisoned_text(live: &mut LiveMessage, poisoned_texts: &mut Vec<String>) {
    if let Some(text) = live.take_poisoned_text() {
        poisoned_texts.push(text);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn close_live_for_boundary(
    ctx: &WeixinStreamConsumerContext,
    live: &mut LiveMessage,
    reason: FinalizeReason,
    sent_media_refs: &mut HashSet<String>,
    poisoned_texts: &mut Vec<String>,
    typing_keepalive_task: &mut Option<JoinHandle<()>>,
    typing_active: &mut bool,
    calls_used: &mut u32,
) -> bool {
    let finalize_result = finalize_live_message(ctx, live, reason, calls_used).await;
    if matches!(finalize_result, LiveTextSendResult::Poisoned) {
        stop_stream_typing(ctx, typing_keepalive_task, typing_active).await;
    }
    let text_sent = matches!(finalize_result, LiveTextSendResult::Sent);
    drain_live_media(ctx, live, sent_media_refs, calls_used).await;
    collect_poisoned_text(live, poisoned_texts);
    text_sent
}

pub(super) async fn queue_poisoned_texts(
    ctx: &WeixinStreamConsumerContext,
    poisoned_texts: &[String],
) {
    let merged = poisoned_texts
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if merged.is_empty() {
        return;
    }
    queue_pending_outbound(&ctx.account_id, &ctx.user_id, &merged).await;
}

pub(super) async fn run_streaming_update_consumer(
    ctx: WeixinStreamConsumerContext,
    mut event_rx: mpsc::UnboundedReceiver<StreamEvent>,
    stream_done_tx: oneshot::Sender<()>,
    final_done_flush_sent: Arc<AtomicBool>,
    seen_done_event: Arc<AtomicBool>,
) {
    let mut live = open_live_message_for_context(&ctx, false).await;
    let mut tick = tokio::time::interval(Duration::from_millis(STREAM_UPDATE_TICK_MS));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut typing_keepalive_task: Option<JoinHandle<()>> = None;
    let mut typing_active = false;
    let mut sent_media_refs = HashSet::<String>::new();
    let mut poisoned_texts = Vec::<String>::new();
    let mut send_calls_used = 0_u32;
    let mut any_text_output_sent = false;

    loop {
        tokio::select! {
            maybe_event = event_rx.recv() => {
                let Some(event) = maybe_event else {
                    let _ = close_live_for_boundary(
                        &ctx,
                        &mut live,
                        FinalizeReason::Done,
                        &mut sent_media_refs,
                        &mut poisoned_texts,
                        &mut typing_keepalive_task,
                        &mut typing_active,
                        &mut send_calls_used,
                    ).await;
                    break;
                };
                match event {
                    StreamEvent::SessionBound { .. } => {}
                    StreamEvent::Delta { text } => {
                        ensure_stream_typing(&ctx, &mut typing_keepalive_task, &mut typing_active).await;
                        let now = Instant::now();
                        live.append_delta(&text, &sent_media_refs, now);
                        if let Some(reason) = live.needs_budget_finalize().await {
                            any_text_output_sent |= close_live_for_boundary(
                                &ctx,
                                &mut live,
                                reason,
                                &mut sent_media_refs,
                                &mut poisoned_texts,
                                &mut typing_keepalive_task,
                                &mut typing_active,
                                &mut send_calls_used,
                            ).await;
                            live = open_live_message_for_context(&ctx, false).await;
                        } else if matches!(
                            send_live_generating(&ctx, &mut live, now, &mut send_calls_used).await,
                            LiveTextSendResult::Poisoned
                        ) {
                            stop_stream_typing(&ctx, &mut typing_keepalive_task, &mut typing_active).await;
                        }
                    }
                    StreamEvent::Boundary { kind, .. } => match kind {
                        StreamBoundaryKind::UserAck => {
                            if matches!(live.state, LiveMessageState::Updating)
                                && !live.last_sent_visible.trim().is_empty()
                            {
                                live.keep_only_sent_text_for_finish();
                                any_text_output_sent |= close_live_for_boundary(
                                    &ctx,
                                    &mut live,
                                    FinalizeReason::UserAck,
                                    &mut sent_media_refs,
                                    &mut poisoned_texts,
                                    &mut typing_keepalive_task,
                                    &mut typing_active,
                                    &mut send_calls_used,
                                ).await;
                                live = open_live_message_for_context(&ctx, true).await;
                            } else {
                                if !live.text_visible.trim().is_empty()
                                    || !live.pending_media_refs.is_empty()
                                {
                                    info!(
                                        account_id = %ctx.account_id,
                                        user_id = %ctx.user_id,
                                        dropped_len = live.text_visible.len(),
                                        dropped_media_refs = live.pending_media_refs.len(),
                                        "dropping buffered weixin stream output on user_ack boundary"
                                    );
                                }
                                live.clear_text();
                                live = open_live_message_for_context(&ctx, true).await;
                            }
                        }
                        StreamBoundaryKind::AssistantSegment => {
                            if token_sends_remaining(live.context_token.trim()).await <= 3 {
                                live.append_soft_boundary();
                            } else {
                                any_text_output_sent |= close_live_for_boundary(
                                    &ctx,
                                    &mut live,
                                    FinalizeReason::ToolBoundary,
                                    &mut sent_media_refs,
                                    &mut poisoned_texts,
                                    &mut typing_keepalive_task,
                                    &mut typing_active,
                                    &mut send_calls_used,
                                ).await;
                                live = open_live_message_for_context(&ctx, false).await;
                            }
                        }
                    },
                    StreamEvent::ToolUse { .. } => {}
                    StreamEvent::ToolResult { message } => {
                        live.collect_provider_media_refs(
                            extract_media_refs_from_provider_message(&message),
                            &sent_media_refs,
                        );
                    }
                    StreamEvent::ThreadTitleUpdated { .. } => {}
                    StreamEvent::Done => {
                        seen_done_event.store(true, Ordering::Relaxed);
                        any_text_output_sent |= close_live_for_boundary(
                            &ctx,
                            &mut live,
                            FinalizeReason::Done,
                            &mut sent_media_refs,
                            &mut poisoned_texts,
                            &mut typing_keepalive_task,
                            &mut typing_active,
                            &mut send_calls_used,
                        ).await;
                        if !poisoned_texts.is_empty() {
                            queue_poisoned_texts(&ctx, &poisoned_texts).await;
                        }
                        if any_text_output_sent || !poisoned_texts.is_empty() {
                            final_done_flush_sent.store(true, Ordering::Relaxed);
                        }
                        stop_stream_typing(&ctx, &mut typing_keepalive_task, &mut typing_active).await;
                        break;
                    }
                }
            }
            _ = tick.tick() => {
                if !ctx.running.load(Ordering::Relaxed) {
                    let _ = close_live_for_boundary(
                        &ctx,
                        &mut live,
                        FinalizeReason::Done,
                        &mut sent_media_refs,
                        &mut poisoned_texts,
                        &mut typing_keepalive_task,
                        &mut typing_active,
                        &mut send_calls_used,
                    ).await;
                    stop_stream_typing(&ctx, &mut typing_keepalive_task, &mut typing_active).await;
                    break;
                }
                let now = Instant::now();
                if let Some(reason) = live.needs_budget_finalize().await {
                    any_text_output_sent |= close_live_for_boundary(
                        &ctx,
                        &mut live,
                        reason,
                        &mut sent_media_refs,
                        &mut poisoned_texts,
                        &mut typing_keepalive_task,
                        &mut typing_active,
                        &mut send_calls_used,
                    ).await;
                    live = open_live_message_for_context(&ctx, false).await;
                } else if live.should_force_inactivity_finish(now) {
                    any_text_output_sent |= close_live_for_boundary(
                        &ctx,
                        &mut live,
                        FinalizeReason::Inactivity,
                        &mut sent_media_refs,
                        &mut poisoned_texts,
                        &mut typing_keepalive_task,
                        &mut typing_active,
                        &mut send_calls_used,
                    ).await;
                    live = open_live_message_for_context(&ctx, false).await;
                } else if matches!(
                    send_live_generating(&ctx, &mut live, now, &mut send_calls_used).await,
                    LiveTextSendResult::Poisoned
                ) {
                    stop_stream_typing(&ctx, &mut typing_keepalive_task, &mut typing_active).await;
                }
            }
        }
    }

    stop_stream_typing(&ctx, &mut typing_keepalive_task, &mut typing_active).await;
    record_weixin_send_calls_per_inbound(send_calls_used);
    let _ = stream_done_tx.send(());
}

pub(crate) struct WeixinStreamingCallbackConfig {
    pub(crate) http: Client,
    pub(crate) account: WeixinAccount,
    pub(crate) account_id: String,
    pub(crate) user_id: String,
    pub(crate) context_token: String,
    pub(crate) thread_id: String,
    pub(crate) typing_ticket: Option<String>,
    pub(crate) running: Arc<AtomicBool>,
}

pub(crate) fn build_weixin_response_callback(
    cfg: WeixinStreamingCallbackConfig,
) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let (event_tx, event_rx) = mpsc::unbounded_channel::<StreamEvent>();
    let (stream_done_tx, _stream_done_rx) = oneshot::channel::<()>();
    let use_streaming_update = cfg.account.streaming_update;
    let ctx = WeixinStreamConsumerContext {
        http: cfg.http,
        account: cfg.account,
        account_id: cfg.account_id,
        user_id: cfg.user_id,
        context_token: cfg.context_token,
        thread_id: Arc::new(std::sync::Mutex::new(cfg.thread_id)),
        typing_ticket: cfg.typing_ticket,
        running: cfg.running,
    };
    if use_streaming_update {
        tokio::spawn(run_streaming_update_consumer(
            ctx,
            event_rx,
            stream_done_tx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        ));
    } else {
        tokio::spawn(run_non_streaming_update_consumer(
            ctx,
            event_rx,
            stream_done_tx,
        ));
    }

    Arc::new(move |event: StreamEvent| {
        let _ = event_tx.send(event);
    })
}

pub(super) async fn flush_non_streaming_weixin_text(
    ctx: &WeixinStreamConsumerContext,
    text: &str,
    extra_media_refs: &[OutboundMediaRef],
    sent_media_refs: &mut HashSet<String>,
) {
    if !ctx.running.load(Ordering::Relaxed) {
        return;
    }

    let outbound = text.trim().to_owned();
    let thread_id = current_stream_thread_id(ctx);
    let mut media_refs = extract_markdown_media_refs(&outbound);
    media_refs.extend(extra_media_refs.iter().cloned());
    if outbound.is_empty() && media_refs.is_empty() {
        return;
    }

    let token = get_context_token_for_thread(
        &ctx.account_id,
        &ctx.user_id,
        if thread_id.is_empty() {
            None
        } else {
            Some(thread_id.as_str())
        },
    )
    .await
    .or_else(|| {
        let token = ctx.context_token.trim();
        (!token.is_empty()).then(|| token.to_owned())
    });

    if let Some(token) = token.as_deref()
        && token_sends_remaining(token).await == 0
    {
        let mut queue_text = outbound.clone();
        for media_ref in &media_refs {
            let dedupe_key = media_ref.dedupe_key();
            if sent_media_refs.contains(&dedupe_key) {
                continue;
            }
            let media_text = match media_ref {
                OutboundMediaRef::RemoteUrl(url) => url.clone(),
                OutboundMediaRef::LocalPath(path) => path.clone(),
                OutboundMediaRef::InlineImage { file_name, .. } => {
                    format!("[generated image: {file_name}]")
                }
            };
            if !queue_text.is_empty() {
                queue_text.push('\n');
            }
            queue_text.push_str(&media_text);
        }
        let plain = markdown_to_plain_text(&queue_text).trim().to_owned();
        if !plain.is_empty() {
            queue_pending_outbound(&ctx.account_id, &ctx.user_id, &plain).await;
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
        let media_bytes = match load_media_bytes(&ctx.http, &media_ref).await {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(
                    account_id = %ctx.account_id,
                    user_id = %ctx.user_id,
                    error = %error,
                    "failed to load weixin media reference"
                );
                continue;
            }
        };
        let uploaded = match upload_media_to_cdn(
            &ctx.http,
            &ctx.account,
            &ctx.user_id,
            &media_bytes,
            media_ref.classify_media_type(),
            media_ref.file_name(),
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                warn!(
                    account_id = %ctx.account_id,
                    user_id = %ctx.user_id,
                    error = %error,
                    "failed to upload weixin media reference"
                );
                continue;
            }
        };
        match send_media_message(
            &ctx.http,
            &ctx.account,
            &ctx.user_id,
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
                    account_id = %ctx.account_id,
                    user_id = %ctx.user_id,
                    error = %error,
                    "failed to send weixin media message"
                );
            }
        }
    }

    if maybe_message_id.is_none() && !plain_text.is_empty() {
        match send_text_message(
            &ctx.http,
            &ctx.account,
            &ctx.user_id,
            &plain_text,
            token.as_deref(),
        )
        .await
        {
            Ok(_) => {}
            Err(error) => {
                error!(
                    account_id = %ctx.account_id,
                    user_id = %ctx.user_id,
                    error = %error,
                    "failed to send weixin non-streaming response"
                );
                let error_text = error.to_string();
                if error_text.contains("ret=")
                    || error_text.contains("ret!=0")
                    || error_text.contains("context_token")
                    || error_text.contains("send limit")
                {
                    queue_pending_outbound(&ctx.account_id, &ctx.user_id, &plain_text).await;
                }
                return;
            }
        }
    }
}

pub(super) async fn run_non_streaming_update_consumer(
    ctx: WeixinStreamConsumerContext,
    mut event_rx: mpsc::UnboundedReceiver<StreamEvent>,
    stream_done_tx: oneshot::Sender<()>,
) {
    let mut stream_text = String::new();
    let mut sent_media_refs = HashSet::<String>::new();
    let mut pending_media_refs = Vec::<OutboundMediaRef>::new();

    loop {
        if !ctx.running.load(Ordering::Relaxed) {
            break;
        }
        let event = tokio::select! {
            event = event_rx.recv() => event,
            _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
        };
        let Some(event) = event else {
            break;
        };
        if !ctx.running.load(Ordering::Relaxed) {
            break;
        }
        match event {
            StreamEvent::SessionBound { .. } | StreamEvent::ThreadTitleUpdated { .. } => {}
            StreamEvent::Delta { text } => {
                stream_text = merge_stream_text(&stream_text, &text);
            }
            StreamEvent::Boundary { kind, .. } => match kind {
                StreamBoundaryKind::UserAck => {
                    if !stream_text.trim().is_empty() {
                        info!(
                            account_id = %ctx.account_id,
                            user_id = %ctx.user_id,
                            dropped_len = stream_text.len(),
                            "dropping buffered weixin stream text on user_ack boundary"
                        );
                    }
                    apply_weixin_stream_boundary(&mut stream_text, StreamBoundaryKind::UserAck);
                }
                StreamBoundaryKind::AssistantSegment => {
                    apply_weixin_stream_boundary(
                        &mut stream_text,
                        StreamBoundaryKind::AssistantSegment,
                    );
                }
            },
            StreamEvent::ToolUse { .. } => {
                let remaining = token_sends_remaining(&ctx.context_token).await;
                if remaining > 2 && !stream_text.trim().is_empty() {
                    flush_non_streaming_weixin_text(
                        &ctx,
                        &stream_text,
                        &pending_media_refs,
                        &mut sent_media_refs,
                    )
                    .await;
                    stream_text.clear();
                    pending_media_refs.clear();
                }
            }
            StreamEvent::ToolResult { message } => {
                pending_media_refs.extend(extract_media_refs_from_provider_message(&message));
            }
            StreamEvent::Done => {
                flush_non_streaming_weixin_text(
                    &ctx,
                    &stream_text,
                    &pending_media_refs,
                    &mut sent_media_refs,
                )
                .await;
                break;
            }
        }
    }

    let _ = stream_done_tx.send(());
}
