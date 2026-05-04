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
const STREAM_UPDATE_TICK_MS: u64 = 200;
const STREAM_UPDATE_MIN_INTERVAL_MS: u64 = 800;
const STREAM_UPDATE_MIN_DELTA_CHARS: usize = 12;
const STREAM_UPDATE_INACTIVITY_FORCE_FINISH_MS: u64 = 15_000;
const LIVE_MESSAGE_MAX_GENERATING_SENDS: u8 = 7;
type ContextTokenStore = Arc<Mutex<HashMap<String, String>>>;
type Aes128EcbEnc = Encryptor<Aes128>;
type Aes128EcbDec = Decryptor<Aes128>;
type TypingTicketStore = Arc<Mutex<HashMap<String, String>>>;

// ---------------------------------------------------------------------------
// Session guard — pause all API calls for an account after errcode=-14.
// ---------------------------------------------------------------------------

type SessionPauseStore = Arc<Mutex<HashMap<String, std::time::Instant>>>;

fn session_pause_store() -> &'static SessionPauseStore {
    static STORE: OnceLock<SessionPauseStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Mark an account's session as paused (expired). All API calls for this
/// account should be suppressed for `SESSION_PAUSE_DURATION`.
pub async fn pause_session(account_id: &str) {
    let mut store = session_pause_store().lock().await;
    store.insert(account_id.to_owned(), std::time::Instant::now());
    error!(
        account_id = account_id,
        pause_secs = SESSION_PAUSE_DURATION.as_secs(),
        "weixin session paused (errcode=-14); suppressing API calls"
    );
}

/// Returns `true` if the account's session is currently paused.
/// Removes expired entries on check (SDK parity).
pub async fn is_session_paused(account_id: &str) -> bool {
    let mut store = session_pause_store().lock().await;
    if let Some(paused_at) = store.get(account_id) {
        if paused_at.elapsed() < SESSION_PAUSE_DURATION {
            return true;
        }
        store.remove(account_id);
    }
    false
}

/// Clear session pause (e.g. after a successful getUpdates).
pub async fn clear_session_pause(account_id: &str) {
    let mut store = session_pause_store().lock().await;
    store.remove(account_id);
}

// ---------------------------------------------------------------------------
// get_updates_buf persistence — survive restarts without re-delivery.
// ---------------------------------------------------------------------------

fn get_updates_buf_persistence_path() -> std::path::PathBuf {
    garyx_models::local_paths::default_session_data_dir().join("weixin_getupdates_buf.json")
}

fn load_get_updates_buf() -> HashMap<String, String> {
    let path = get_updates_buf_persistence_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    serde_json::from_str(&data)
        .map_err(|e| warn!(path = %path.display(), error = %e, "failed to parse weixin getupdates_buf JSON"))
        .unwrap_or_default()
}

fn persist_get_updates_buf(store: &HashMap<String, String>) {
    let path = get_updates_buf_persistence_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(store) {
        let _ = std::fs::write(&path, json);
    }
}

type GetUpdatesBufStore = Arc<Mutex<HashMap<String, String>>>;

fn get_updates_buf_store() -> &'static GetUpdatesBufStore {
    static STORE: OnceLock<GetUpdatesBufStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let map = load_get_updates_buf();
        if !map.is_empty() {
            info!(count = map.len(), "loaded persisted weixin getupdates_buf");
        }
        Arc::new(Mutex::new(map))
    })
}

async fn get_persisted_cursor(account_id: &str) -> String {
    let store = get_updates_buf_store().lock().await;
    store.get(account_id).cloned().unwrap_or_default()
}

async fn set_persisted_cursor(account_id: &str, cursor: &str) {
    let snapshot = {
        let mut store = get_updates_buf_store().lock().await;
        store.insert(account_id.to_owned(), cursor.to_owned());
        store.clone()
    };
    // Persist outside the lock to avoid blocking the async runtime.
    tokio::task::spawn_blocking(move || persist_get_updates_buf(&snapshot));
}

// ---------------------------------------------------------------------------
// Token send counter — each context_token can only be used for ~10 sends.
// We track usage and treat the token as exhausted at TOKEN_SEND_LIMIT.
// ---------------------------------------------------------------------------

/// Maximum sends per context_token before we consider it exhausted.
/// Empirically verified 2026-05-02: hard cap is 10; we leave 1 reserve for
/// retry safety, so the in-process soft cap is 9.
const TOKEN_SEND_LIMIT: u32 = 9;

type TokenSendCountStore = Arc<Mutex<HashMap<String, u32>>>;

fn token_send_count_store() -> &'static TokenSendCountStore {
    static STORE: OnceLock<TokenSendCountStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Increment the send counter for a token and return the new count.
/// Returns `None` if the token has already reached the limit (caller should not send).
pub async fn token_send_increment(token: &str) -> Option<u32> {
    if token.trim().is_empty() {
        return None;
    }
    let mut store = token_send_count_store().lock().await;
    let count = store.entry(token.to_owned()).or_insert(0);
    if *count >= TOKEN_SEND_LIMIT {
        return None;
    }
    *count += 1;
    Some(*count)
}

/// Check how many sends remain for the given token.
pub async fn token_sends_remaining(token: &str) -> u32 {
    let store = token_send_count_store().lock().await;
    let used = store.get(token).copied().unwrap_or(0);
    TOKEN_SEND_LIMIT.saturating_sub(used)
}

/// Reset the send counter for a specific token (called when we get a fresh token).
pub async fn token_send_count_reset(token: &str) {
    let mut store = token_send_count_store().lock().await;
    store.remove(token);
}

/// Prune counters for tokens that are no longer in use (housekeeping).
pub async fn token_send_count_prune(active_tokens: &[&str]) {
    let mut store = token_send_count_store().lock().await;
    store.retain(|k, _| active_tokens.contains(&k.as_str()));
}

// ---------------------------------------------------------------------------
// Lightweight observability counters for Weixin streaming-update rollout.
// ---------------------------------------------------------------------------

static WEIXIN_MEDIA_DROPPED_TOTAL: AtomicU64 = AtomicU64::new(0);
static WEIXIN_SEND_CALLS_TOTAL: AtomicU64 = AtomicU64::new(0);

type FinalizeReasonCounterStore = Arc<Mutex<HashMap<&'static str, u64>>>;

fn finalize_reason_counter_store() -> &'static FinalizeReasonCounterStore {
    static STORE: OnceLock<FinalizeReasonCounterStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

async fn record_weixin_finalize_reason(reason: FinalizeReason) {
    let label = reason.metric_label();
    let snapshot_value = {
        let mut counters = finalize_reason_counter_store().lock().await;
        let counter = counters.entry(label).or_insert(0);
        *counter += 1;
        *counter
    };
    debug!(
        metric = "weixin_finalize_reason_total",
        reason = label,
        value = snapshot_value,
        "weixin streaming finalize reason"
    );
}

fn record_weixin_media_dropped(count: usize) {
    if count == 0 {
        return;
    }
    let value =
        WEIXIN_MEDIA_DROPPED_TOTAL.fetch_add(count as u64, Ordering::Relaxed) + count as u64;
    warn!(
        metric = "weixin_media_dropped_total",
        dropped = count,
        value,
        "dropped weixin media refs"
    );
}

fn record_weixin_send_calls_per_inbound(count: u32) {
    WEIXIN_SEND_CALLS_TOTAL.fetch_add(count as u64, Ordering::Relaxed);
    debug!(
        metric = "weixin_send_calls_per_inbound",
        count, "weixin send calls used for inbound"
    );
}

// ---------------------------------------------------------------------------
// Pending outbound message queue — when weixin sends fail (e.g. token expired),
// messages are queued here and flushed when a new inbound message arrives with
// a fresh context_token.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PendingOutboundMessage {
    pub to_user_id: String,
    pub text: String,
    pub queued_at: std::time::Instant,
}

type PendingOutboundStore = Arc<Mutex<HashMap<String, Vec<PendingOutboundMessage>>>>;

fn pending_outbound_store() -> &'static PendingOutboundStore {
    static STORE: OnceLock<PendingOutboundStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Queue a failed outbound message for later delivery when a fresh token arrives.
pub async fn queue_pending_outbound(account_id: &str, to_user_id: &str, text: &str) {
    let key = format!("{account_id}:{to_user_id}");
    let mut store = pending_outbound_store().lock().await;
    let queue = store.entry(key).or_default();
    // Cap at 20 messages to avoid unbounded growth
    if queue.len() >= 20 {
        queue.remove(0);
    }
    queue.push(PendingOutboundMessage {
        to_user_id: to_user_id.to_owned(),
        text: text.to_owned(),
        queued_at: std::time::Instant::now(),
    });
    info!(
        account_id = account_id,
        to_user_id = to_user_id,
        queue_len = queue.len(),
        "queued pending weixin outbound message (token expired)"
    );
}

/// Drain and return pending outbound messages for a user (called when a fresh
/// context_token arrives via an inbound message).
pub async fn drain_pending_outbound(
    account_id: &str,
    user_id: &str,
) -> Vec<PendingOutboundMessage> {
    let key = format!("{account_id}:{user_id}");
    let mut store = pending_outbound_store().lock().await;
    store.remove(&key).unwrap_or_default()
}

/// Returns the count of pending outbound messages for a user.
pub async fn pending_outbound_count(account_id: &str, user_id: &str) -> usize {
    let key = format!("{account_id}:{user_id}");
    let store = pending_outbound_store().lock().await;
    store.get(&key).map(|q| q.len()).unwrap_or(0)
}

fn apply_weixin_stream_boundary(
    stream_text: &mut String,
    kind: StreamBoundaryKind,
) -> crate::streaming_core::BoundaryTextEffect {
    crate::streaming_core::apply_stream_boundary_text(stream_text, kind)
}

fn context_token_persistence_path() -> std::path::PathBuf {
    garyx_models::local_paths::default_session_data_dir().join("weixin_context_tokens.json")
}

fn context_token_store() -> &'static ContextTokenStore {
    static STORE: OnceLock<ContextTokenStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let map = load_context_tokens_from_disk().unwrap_or_default();
        if !map.is_empty() {
            info!(count = map.len(), "loaded persisted weixin context tokens");
        }
        Arc::new(Mutex::new(map))
    })
}

fn load_context_tokens_from_disk() -> Option<HashMap<String, String>> {
    let path = context_token_persistence_path();
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data)
        .map_err(|e| warn!(path = %path.display(), error = %e, "failed to parse weixin context tokens JSON"))
        .ok()
}

fn persist_context_tokens(store: &HashMap<String, String>) {
    let path = context_token_persistence_path();
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), error = %e, "failed to create parent dir for context tokens");
    }
    match serde_json::to_string_pretty(store) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!(error = %e, "failed to persist weixin context tokens");
            }
        }
        Err(e) => warn!(error = %e, "failed to serialize weixin context tokens"),
    }
}

fn context_token_key(account_id: &str, user_id: &str) -> String {
    format!("{account_id}:{user_id}")
}

fn context_token_thread_key(account_id: &str, user_id: &str, thread_id: &str) -> String {
    format!("{account_id}:{user_id}:thread:{thread_id}")
}

fn typing_ticket_store() -> &'static TypingTicketStore {
    static STORE: OnceLock<TypingTicketStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub async fn set_typing_ticket(account_id: &str, user_id: &str, ticket: &str) {
    if account_id.trim().is_empty() || user_id.trim().is_empty() || ticket.trim().is_empty() {
        return;
    }
    let mut store = typing_ticket_store().lock().await;
    store.insert(context_token_key(account_id, user_id), ticket.to_owned());
}

pub async fn get_typing_ticket(account_id: &str, user_id: &str) -> Option<String> {
    if account_id.trim().is_empty() || user_id.trim().is_empty() {
        return None;
    }
    let store = typing_ticket_store().lock().await;
    store.get(&context_token_key(account_id, user_id)).cloned()
}

pub async fn set_context_token(account_id: &str, user_id: &str, token: &str) {
    set_context_token_for_thread(account_id, user_id, None, token).await;
}

pub async fn set_context_token_for_thread(
    account_id: &str,
    user_id: &str,
    thread_id: Option<&str>,
    token: &str,
) {
    if account_id.trim().is_empty() || user_id.trim().is_empty() || token.trim().is_empty() {
        return;
    }
    let mut store = context_token_store().lock().await;
    let user_key = context_token_key(account_id, user_id);
    if let Some(thread_id) = thread_id.map(str::trim).filter(|value| !value.is_empty()) {
        store.insert(
            context_token_thread_key(account_id, user_id, thread_id),
            token.to_owned(),
        );
        store.entry(user_key).or_insert_with(|| token.to_owned());
    } else {
        store.insert(user_key, token.to_owned());
    }
    persist_context_tokens(&store);
}

pub async fn get_context_token(account_id: &str, user_id: &str) -> Option<String> {
    get_context_token_for_thread(account_id, user_id, None).await
}

pub async fn get_context_token_for_thread(
    account_id: &str,
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<String> {
    if account_id.trim().is_empty() || user_id.trim().is_empty() {
        return None;
    }
    let store = context_token_store().lock().await;
    if let Some(thread_id) = thread_id.map(str::trim).filter(|value| !value.is_empty())
        && let Some(token) = store
            .get(&context_token_thread_key(account_id, user_id, thread_id))
            .cloned()
    {
        return Some(token);
    }
    store.get(&context_token_key(account_id, user_id)).cloned()
}

/// Extract account_id from a weixin bot token.
/// Token format is typically `"account_id@im.bot:secret"`.
fn account_id_from_token(token: &str) -> &str {
    token.split(':').next().unwrap_or(token).trim()
}

fn build_api_url(base_url: &str, endpoint: &str) -> String {
    format!(
        "{}/ilink/bot/{endpoint}",
        base_url.trim_end_matches('/').trim_end()
    )
}

fn build_cdn_upload_url(cdn_base_url: &str, upload_param: &str, filekey: &str) -> String {
    format!(
        "{}/upload?encrypted_query_param={}&filekey={}",
        cdn_base_url.trim_end_matches('/').trim_end(),
        urlencoding::encode(upload_param),
        urlencoding::encode(filekey)
    )
}

fn build_cdn_download_url(cdn_base_url: &str, encrypted_query_param: &str) -> String {
    format!(
        "{}/download?encrypted_query_param={}",
        cdn_base_url.trim_end_matches('/').trim_end(),
        urlencoding::encode(encrypted_query_param)
    )
}

fn markdown_image_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!\[[^\]]*\]\(([^)]+)\)").expect("valid markdown image regex"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutboundMediaRef {
    RemoteUrl(String),
    LocalPath(String),
    InlineImage {
        id: String,
        bytes: Vec<u8>,
        file_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveMessageState {
    Pristine,
    Updating,
    Finalized,
    DeliveryDisabled { reason: PoisonReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoisonReason {
    TokenExhausted,
    SessionPaused,
    HttpFailure,
}

impl PoisonReason {
    fn metric_label(self) -> &'static str {
        match self {
            Self::TokenExhausted => "poisoned_token_exhausted",
            Self::SessionPaused => "poisoned_session_paused",
            Self::HttpFailure => "poisoned_http",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalizeReason {
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
    fn metric_label(self) -> &'static str {
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
struct LiveMessage {
    client_id: String,
    context_token: String,
    text_visible: String,
    text_raw: String,
    pending_media_refs: Vec<OutboundMediaRef>,
    last_sent_visible: String,
    last_sent_at: Option<Instant>,
    last_delta_at: Option<Instant>,
    sends_used: u8,
    state: LiveMessageState,
}

impl LiveMessage {
    async fn open(context_token: String) -> Self {
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

    fn append_delta(&mut self, delta: &str, sent_media_refs: &HashSet<String>, now: Instant) {
        self.text_raw = merge_stream_text(&self.text_raw, delta);
        self.collect_markdown_media_refs(sent_media_refs);
        self.text_visible = markdown_to_plain_text(&self.text_raw).trim().to_owned();
        self.last_delta_at = Some(now);
    }

    fn append_soft_boundary(&mut self) {
        if self.text_raw.trim().is_empty() {
            return;
        }
        self.text_raw.push_str("\n\n");
        self.text_visible = markdown_to_plain_text(&self.text_raw).trim().to_owned();
    }

    fn clear_text(&mut self) {
        self.text_raw.clear();
        self.text_visible.clear();
        self.last_sent_visible.clear();
        self.last_sent_at = None;
        self.last_delta_at = None;
        self.pending_media_refs.clear();
    }

    fn keep_only_sent_text_for_finish(&mut self) {
        self.text_raw = self.last_sent_visible.clone();
        self.text_visible = self.last_sent_visible.clone();
        self.pending_media_refs.clear();
        self.last_delta_at = None;
    }

    fn collect_markdown_media_refs(&mut self, sent_media_refs: &HashSet<String>) {
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

    fn collect_provider_media_refs(
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

    fn pending_delta_chars(&self) -> usize {
        self.text_visible
            .chars()
            .count()
            .saturating_sub(self.last_sent_visible.chars().count())
    }

    fn has_buffered_visible(&self) -> bool {
        self.text_visible != self.last_sent_visible
    }

    fn sentence_terminated_since_last_send(&self) -> bool {
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

    fn has_unterminated_markdown_image_ref_tail(&self) -> bool {
        unterminated_markdown_image_tail_regex().is_match(&self.text_raw)
    }

    async fn should_send_generating(&self, now: Instant) -> bool {
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
        let enough_text = self.pending_delta_chars() >= STREAM_UPDATE_MIN_DELTA_CHARS
            || self.sentence_terminated_since_last_send();
        if !enough_text {
            return false;
        }
        self.last_sent_at.is_none_or(|last| {
            now.duration_since(last) >= Duration::from_millis(STREAM_UPDATE_MIN_INTERVAL_MS)
        })
    }

    fn should_force_inactivity_finish(&self, now: Instant) -> bool {
        matches!(self.state, LiveMessageState::Updating)
            && self.last_delta_at.is_some_and(|last_delta| {
                now.duration_since(last_delta)
                    >= Duration::from_millis(STREAM_UPDATE_INACTIVITY_FORCE_FINISH_MS)
            })
    }

    async fn needs_budget_finalize(&self) -> Option<FinalizeReason> {
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

    fn take_poisoned_text(&mut self) -> Option<String> {
        if !matches!(self.state, LiveMessageState::DeliveryDisabled { .. }) {
            return None;
        }
        let text = self.text_visible.trim().to_owned();
        self.text_visible.clear();
        self.text_raw.clear();
        if text.is_empty() { None } else { Some(text) }
    }
}

fn is_stream_update_sentence_terminator(ch: char) -> bool {
    matches!(ch, '.' | '?' | '!' | '。' | '？' | '！' | '…' | ':' | '：')
}

fn unterminated_markdown_image_tail_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*$").expect("valid markdown tail regex"))
}

impl OutboundMediaRef {
    fn dedupe_key(&self) -> String {
        match self {
            Self::RemoteUrl(url) => format!("url:{url}"),
            Self::LocalPath(path) => format!("path:{path}"),
            Self::InlineImage { id, .. } => format!("inline:{id}"),
        }
    }

    fn classify_media_type(&self) -> i64 {
        match self {
            Self::RemoteUrl(url) => classify_media_type_from_url(url),
            Self::LocalPath(path) => classify_media_type_from_url(path),
            Self::InlineImage { .. } => 1,
        }
    }

    /// Best-effort filename for WeChat `file_item.file_name`. Strips query
    /// strings from URLs and takes the last path segment; returns `None` when
    /// nothing usable can be recovered (callers fall back to `attachment.bin`).
    fn file_name(&self) -> Option<String> {
        let raw = match self {
            Self::RemoteUrl(url) => url.as_str(),
            Self::LocalPath(path) => path.as_str(),
            Self::InlineImage { file_name, .. } => return Some(file_name.clone()),
        };
        // Drop URL query/fragment before taking the basename.
        let without_query = raw.split(['?', '#']).next().unwrap_or(raw);
        let last_segment = without_query
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())?
            .to_owned();
        if last_segment.is_empty() {
            return None;
        }
        Some(last_segment)
    }
}

fn auth_headers(builder: RequestBuilder, account: &WeixinAccount) -> RequestBuilder {
    let wechat_uin = if account.uin.trim().is_empty() {
        random_wechat_uin()
    } else {
        account.uin.trim().to_owned()
    };

    builder
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {}", account.token.trim()))
        .header("X-WECHAT-UIN", wechat_uin)
        // Reference SDK parity: always send app-id and client version headers.
        // The SDK packs the version as (major << 16) | (minor << 8) | patch.
        .header("iLink-App-Id", "bot")
        .header("iLink-App-ClientVersion", build_client_version())
}

/// Build a packed uint32 client version matching the SDK's format:
/// `(major << 16) | (minor << 8) | patch`.
fn build_client_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let parts: Vec<u32> = version
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    let patch = parts.get(2).copied().unwrap_or(0);
    ((major << 16) | (minor << 8) | patch).to_string()
}

fn random_wechat_uin() -> String {
    let uuid_bytes = uuid::Uuid::new_v4();
    let bytes = uuid_bytes.as_bytes();
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    STANDARD.encode(value.to_string())
}

fn random_16_bytes() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn parse_hex_key_16(hex: &str) -> Option<[u8; 16]> {
    let hex = hex.trim();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0_u8; 16];
    let bytes = hex.as_bytes();
    for index in 0..16 {
        let hi = bytes[index * 2] as char;
        let lo = bytes[index * 2 + 1] as char;
        let pair = [hi, lo].iter().collect::<String>();
        out[index] = u8::from_str_radix(&pair, 16).ok()?;
    }
    Some(out)
}

fn aes_ecb_padded_size(plaintext_size: usize) -> usize {
    ((plaintext_size + 1).div_ceil(16)) * 16
}

fn encrypt_aes_ecb(plaintext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, ChannelError> {
    let enc = Aes128EcbEnc::new_from_slice(key)
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes init failed: {error}")))?;
    let block_size = 16_usize;
    let padded_len = aes_ecb_padded_size(plaintext.len());
    let mut buffer = vec![0_u8; padded_len.max(block_size)];
    buffer[..plaintext.len()].copy_from_slice(plaintext);
    let encrypted = enc
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes encrypt failed: {error}")))?;
    Ok(encrypted.to_vec())
}

fn decrypt_aes_ecb(ciphertext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, ChannelError> {
    let dec = Aes128EcbDec::new_from_slice(key)
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes init failed: {error}")))?;
    let mut buffer = ciphertext.to_vec();
    dec.decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map(|value| value.to_vec())
        .map_err(|error| ChannelError::SendFailed(format!("weixin aes decrypt failed: {error}")))
}

fn parse_aes_key_base64(aes_key_base64: &str) -> Option<[u8; 16]> {
    let decoded = STANDARD.decode(aes_key_base64).ok()?;
    if decoded.len() == 16 {
        return decoded.try_into().ok();
    }
    if decoded.len() == 32 {
        let as_text = std::str::from_utf8(&decoded).ok()?;
        return parse_hex_key_16(as_text);
    }
    None
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinTextItem {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinVoiceItem {
    #[serde(default)]
    text: String,
    #[allow(dead_code)]
    #[serde(default)]
    media: Option<WeixinCdnMedia>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinImageItem {
    #[serde(default)]
    url: String,
    #[serde(default)]
    media: Option<WeixinCdnMedia>,
    #[serde(default)]
    aeskey: String,
    #[serde(default)]
    #[allow(dead_code)] // serde-populated, used in forwarded JSON
    mid_size: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinFileItem {
    #[serde(default)]
    media: Option<WeixinCdnMedia>,
    #[serde(default)]
    file_name: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinVideoItem {
    #[serde(default)]
    media: Option<WeixinCdnMedia>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinCdnMedia {
    #[serde(default)]
    encrypt_query_param: String,
    #[serde(default)]
    aes_key: String,
    #[serde(default)]
    #[allow(dead_code)] // serde-populated, used in forwarded JSON
    encrypt_type: i64,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinRefMessage {
    #[serde(default)]
    title: String,
    #[serde(default)]
    message_item: Option<Box<WeixinMessageItem>>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinMessageItem {
    #[serde(default)]
    r#type: i64,
    #[serde(default)]
    ref_msg: Option<WeixinRefMessage>,
    #[serde(default)]
    text_item: Option<WeixinTextItem>,
    #[serde(default)]
    image_item: Option<WeixinImageItem>,
    #[serde(default)]
    voice_item: Option<WeixinVoiceItem>,
    #[serde(default)]
    file_item: Option<WeixinFileItem>,
    #[serde(default)]
    video_item: Option<WeixinVideoItem>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct WeixinMessage {
    #[serde(default)]
    message_type: i64,
    #[serde(default)]
    from_user_id: String,
    #[serde(default)]
    context_token: String,
    #[serde(default)]
    item_list: Vec<WeixinMessageItem>,
}

#[derive(Debug, Deserialize, Default)]
struct WeixinGetUpdatesResp {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: String,
    #[serde(default)]
    msgs: Vec<WeixinMessage>,
    #[serde(default)]
    get_updates_buf: String,
    #[serde(default)]
    longpolling_timeout_ms: u64,
}

#[derive(Debug, Deserialize, Default)]
struct WeixinGetUploadUrlResp {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errmsg: String,
    #[serde(default)]
    upload_param: String,
    /// SDK parity: server may return a complete upload URL instead of just upload_param.
    #[serde(default)]
    upload_full_url: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct WeixinGetConfigResp {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errmsg: String,
    #[serde(default)]
    typing_ticket: String,
}

#[derive(Debug, Clone)]
struct UploadedWeixinMedia {
    download_encrypted_query_param: String,
    aes_key_raw: [u8; 16],
    plaintext_size: usize,
    ciphertext_size: usize,
    media_type: i64,
    /// Original file name for type=3 file messages. When `None`, falls back to
    /// `"attachment.bin"` in `build_send_media_message_body`. WeChat uses this
    /// field to render the filename + icon in the chat bubble; without it the
    /// recipient sees a useless "attachment.bin".
    file_name: Option<String>,
}

#[derive(Debug)]
enum UploadUrlResult {
    /// Server returned a complete upload URL (preferred).
    FullUrl(String),
    /// Server returned upload_param for client-side URL construction.
    Param(String),
}

fn is_media_item(item: &WeixinMessageItem) -> bool {
    matches!(item.r#type, 2..=5)
}

fn body_from_message_item(item: &WeixinMessageItem) -> String {
    match item.r#type {
        1 => {
            let text = item
                .text_item
                .as_ref()
                .map(|value| value.text.trim().to_owned())
                .unwrap_or_default();
            let Some(ref_msg) = item.ref_msg.as_ref() else {
                return text;
            };
            let Some(ref_item) = ref_msg.message_item.as_ref() else {
                return text;
            };
            if is_media_item(ref_item) {
                return text;
            }
            let mut quote_parts = Vec::new();
            if !ref_msg.title.trim().is_empty() {
                quote_parts.push(ref_msg.title.trim().to_owned());
            }
            let ref_body = body_from_message_item(ref_item);
            if !ref_body.trim().is_empty() {
                quote_parts.push(ref_body.trim().to_owned());
            }
            if quote_parts.is_empty() {
                text
            } else if text.trim().is_empty() {
                format!("[引用: {}]", quote_parts.join(" | "))
            } else {
                format!("[引用: {}]\n{text}", quote_parts.join(" | "))
            }
        }
        2 => {
            let Some(image_item) = item.image_item.as_ref() else {
                return "[图片]".to_owned();
            };
            let url = image_item.url.trim();
            if !url.is_empty() {
                format!("[图片] {url}")
            } else {
                "[图片]".to_owned()
            }
        }
        3 => {
            let text = item
                .voice_item
                .as_ref()
                .map(|value| value.text.trim())
                .unwrap_or_default();
            if text.is_empty() {
                "[语音]".to_owned()
            } else {
                text.to_owned()
            }
        }
        4 => "[文件]".to_owned(),
        5 => "[视频]".to_owned(),
        _ => String::new(),
    }
}

fn extract_text(items: &[WeixinMessageItem]) -> String {
    items
        .iter()
        .map(body_from_message_item)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn extract_inline_image_attachments(items: &[WeixinMessageItem]) -> Vec<PromptAttachment> {
    let mut attachments = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if item.r#type != 2 {
            continue;
        }
        let Some(url) = item
            .image_item
            .as_ref()
            .map(|value| value.url.trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(rest) = url.strip_prefix("data:") else {
            continue;
        };
        let Some((meta, data)) = rest.split_once(',') else {
            continue;
        };
        if !meta.contains(";base64") {
            continue;
        }
        let media_type = meta
            .split(';')
            .next()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("image/jpeg")
            .to_owned();
        if data.trim().is_empty() {
            continue;
        }
        let Ok(bytes) = STANDARD.decode(data.trim()) else {
            warn!(index, "failed to decode inline weixin image payload");
            continue;
        };
        let name = sanitize_filename(&format!(
            "weixin-inline-{}.{}",
            index + 1,
            media_type.rsplit('/').next().unwrap_or("jpg")
        ));
        let Some(path) = persist_inbound_media_bytes("image", Some(&name), &bytes).await else {
            warn!(index, "failed to persist inline weixin image payload");
            continue;
        };
        attachments.push(PromptAttachment {
            kind: PromptAttachmentKind::Image,
            path,
            name,
            media_type,
        });
    }
    attachments
}

async fn extract_cdn_image_attachments(
    http: &Client,
    items: &[WeixinMessageItem],
) -> Vec<PromptAttachment> {
    let mut attachments = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if item.r#type != 2 {
            continue;
        }
        let Some(image_item) = item.image_item.as_ref() else {
            continue;
        };
        let Some(media) = image_item.media.as_ref() else {
            continue;
        };
        let encrypted_query_param = media.encrypt_query_param.trim();
        if encrypted_query_param.is_empty() {
            continue;
        }
        let aes_key = if !image_item.aeskey.trim().is_empty() {
            parse_hex_key_16(image_item.aeskey.trim())
        } else {
            parse_aes_key_base64(media.aes_key.trim())
        };
        let Some(aes_key) = aes_key else {
            warn!("weixin image item missing parsable aes key");
            continue;
        };
        match download_and_decrypt_cdn(
            http,
            DEFAULT_WEIXIN_CDN_BASE_URL,
            encrypted_query_param,
            &aes_key,
        )
        .await
        {
            Ok(bytes) => {
                let name_hint = if encrypted_query_param.len() >= 12 {
                    encrypted_query_param[..12].to_owned()
                } else {
                    format!("{:02}", index + 1)
                };
                let name = sanitize_filename(&format!("weixin-cdn-{name_hint}.jpg"));
                let Some(path) = persist_inbound_media_bytes("image", Some(&name), &bytes).await
                else {
                    warn!(name_hint, "failed to persist weixin image from cdn");
                    continue;
                };
                attachments.push(PromptAttachment {
                    kind: PromptAttachmentKind::Image,
                    path,
                    name,
                    media_type: "image/jpeg".to_owned(),
                });
            }
            Err(error) => warn!(error = %error, "failed to decrypt weixin image from cdn"),
        }
    }
    attachments
}

fn sanitize_filename(name: &str) -> String {
    crate::sanitize_filename(name.trim())
}

async fn persist_inbound_media_bytes(
    media_kind: &str,
    suggested_name: Option<&str>,
    bytes: &[u8],
) -> Option<String> {
    let base_dir = std::env::temp_dir().join("garyx-weixin").join("inbound");
    if fs::create_dir_all(&base_dir).await.is_err() {
        return None;
    }
    let stem = suggested_name
        .map(sanitize_filename)
        .unwrap_or_else(|| format!("{media_kind}.bin"));
    let path = base_dir.join(format!("{}-{}", uuid::Uuid::new_v4(), stem));
    if fs::write(&path, bytes).await.is_err() {
        return None;
    }
    Some(path.to_string_lossy().to_string())
}

async fn extract_cdn_non_image_media_metadata(
    http: &Client,
    cdn_base_url: &str,
    items: &[WeixinMessageItem],
) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    let mut video_paths = Vec::new();
    let mut file_paths = Vec::new();

    for item in items {
        let (kind, media, name_hint) = match item.r#type {
            // Skip voice (type 3): SILK format cannot be processed by AI.
            // The transcribed text is already extracted via voice_item.text
            // in body_from_message_item, so we only lose the raw audio file.
            3 => continue,
            4 => (
                "file",
                item.file_item
                    .as_ref()
                    .and_then(|value| value.media.as_ref()),
                item.file_item
                    .as_ref()
                    .map(|value| value.file_name.as_str())
                    .or(Some("file.bin")),
            ),
            5 => (
                "video",
                item.video_item
                    .as_ref()
                    .and_then(|value| value.media.as_ref()),
                Some("video.mp4"),
            ),
            _ => continue,
        };
        let Some(media) = media else {
            continue;
        };
        let encrypted_query_param = media.encrypt_query_param.trim();
        if encrypted_query_param.is_empty() {
            continue;
        }
        let Some(aes_key) = parse_aes_key_base64(media.aes_key.trim()) else {
            warn!(kind = %kind, "weixin media item missing parsable aes key");
            continue;
        };
        let decrypted =
            match download_and_decrypt_cdn(http, cdn_base_url, encrypted_query_param, &aes_key)
                .await
            {
                Ok(bytes) => bytes,
                Err(error) => {
                    warn!(kind = %kind, error = %error, "failed to decrypt weixin media from cdn");
                    continue;
                }
            };
        if let Some(saved_path) = persist_inbound_media_bytes(kind, name_hint, &decrypted).await {
            match kind {
                "video" => video_paths.push(Value::String(saved_path)),
                "file" => file_paths.push(Value::String(saved_path)),
                _ => {}
            }
        }
    }

    if !video_paths.is_empty() {
        metadata.insert("video_paths".to_owned(), Value::Array(video_paths));
    }
    if !file_paths.is_empty() {
        metadata.insert("file_paths".to_owned(), Value::Array(file_paths));
    }
    metadata
}

fn build_send_text_message_body(
    to_user_id: &str,
    text: &str,
    context_token: &str,
    client_id: &str,
    message_state: u8,
) -> Value {
    json!({
        "msg": {
            "from_user_id": "",
            "to_user_id": to_user_id,
            "client_id": client_id,
            "message_type": 2,
            "message_state": message_state,
            "context_token": context_token,
            "item_list": [
                {
                    "type": 1,
                    "text_item": {
                        "text": text
                    }
                }
            ]
        },
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

async fn send_text_message_with_state(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    text: &str,
    context_token: Option<&str>,
    client_id: &str,
    message_state: u8,
) -> Result<(), ChannelError> {
    // SDK parity: refuse to send if session is paused (errcode=-14).
    let acct_id = account_id_from_token(&account.token);
    if !acct_id.is_empty() && is_session_paused(acct_id).await {
        return Err(ChannelError::SendFailed(
            "Weixin session paused (errcode=-14); suppressing send".to_owned(),
        ));
    }

    let target = to_user_id.trim();
    if target.is_empty() {
        return Err(ChannelError::Config(
            "Weixin target user_id is empty".to_owned(),
        ));
    }

    let token = context_token.map(str::trim).unwrap_or_default();
    if token.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin context_token is missing; cannot send reply".to_owned(),
        ));
    }

    // Check token send counter — refuse if already at limit
    let remaining = token_sends_remaining(token).await;
    if remaining == 0 {
        warn!(
            to_user_id = target,
            token_limit = TOKEN_SEND_LIMIT,
            "context_token exhausted (hit send limit), refusing send"
        );
        return Err(ChannelError::SendFailed(
            "Weixin context_token exhausted (send limit reached)".to_owned(),
        ));
    }

    let body = build_send_text_message_body(target, text, token, client_id, message_state);

    let url = build_api_url(&account.base_url, "sendmessage");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin sendmessage failed: {error}")))?;

    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendmessage HTTP {status}: {raw}"
        )));
    }

    if let Ok(payload) = serde_json::from_str::<Value>(&raw) {
        let ret = payload.get("ret").and_then(Value::as_i64);
        if ret.is_some_and(|r| r != 0) {
            let ret_code = ret.unwrap_or(-1);
            let message = payload
                .get("errmsg")
                .and_then(Value::as_str)
                .unwrap_or("unknown");

            // ret=-14: session expired — bot needs re-login on the phone.
            // SDK parity: pause all API calls for this account for 1 hour.
            if ret_code == -14 {
                error!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin session expired (ret=-14): bot needs re-login on the phone"
                );
                if !acct_id.is_empty() {
                    pause_session(acct_id).await;
                }
            }
            // ret=-2: parameter error, typically context_token expired or exhausted
            if ret_code == -2 {
                warn!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin context_token likely expired or exhausted (ret=-2)"
                );
            }

            tracing::warn!(
                ret = ret_code,
                errmsg = message,
                to_user_id = target,
                text_len = text.len(),
                response_body = %raw,
                "Weixin sendmessage (text) failed"
            );
            return Err(ChannelError::SendFailed(format!(
                "Weixin sendmessage ret={ret_code}: {message}"
            )));
        }
    }

    // Successful send — increment the token's usage counter
    let count = token_send_increment(token).await;
    if let Some(c) = count {
        let left = TOKEN_SEND_LIMIT.saturating_sub(c);
        if left <= 2 {
            warn!(
                to_user_id = target,
                sends_used = c,
                sends_remaining = left,
                "context_token nearing send limit"
            );
        }
    }

    Ok(())
}

pub async fn send_text_message(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    text: &str,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    let client_id = uuid::Uuid::new_v4().to_string();
    send_text_message_with_state(
        http,
        account,
        to_user_id,
        text,
        context_token,
        &client_id,
        2,
    )
    .await?;
    Ok(client_id)
}

async fn fetch_typing_ticket(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    context_token: Option<&str>,
) -> Result<Option<String>, ChannelError> {
    let body = build_get_config_body(to_user_id, context_token);
    let url = build_api_url(&account.base_url, "getconfig");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin getconfig failed: {error}")))?;
    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getconfig HTTP {status}: {raw}"
        )));
    }
    let payload: WeixinGetConfigResp = serde_json::from_str(&raw).map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin getconfig parse failed: {error}; body={raw}"
        ))
    })?;
    if payload.ret != 0 {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getconfig ret!=0: {}",
            payload.errmsg
        )));
    }
    let ticket = payload.typing_ticket.trim().to_owned();
    if ticket.is_empty() {
        return Ok(None);
    }
    Ok(Some(ticket))
}

async fn send_typing_status(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    typing_ticket: &str,
    status: i64,
) -> Result<(), ChannelError> {
    if typing_ticket.trim().is_empty() {
        return Ok(());
    }
    let body = build_send_typing_body(to_user_id, typing_ticket, status);
    let url = build_api_url(&account.base_url, "sendtyping");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin sendtyping failed: {error}")))?;
    let status_code = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status_code.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendtyping HTTP {status_code}: {raw}"
        )));
    }
    if let Ok(payload) = serde_json::from_str::<Value>(&raw)
        && payload
            .get("ret")
            .and_then(Value::as_i64)
            .is_some_and(|ret| ret != 0)
    {
        let message = payload
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendtyping ret!=0: {message}"
        )));
    }
    Ok(())
}

async fn notify_subscription(
    http: &Client,
    account: &WeixinAccount,
    endpoint: &str,
) -> Result<(), ChannelError> {
    let body = json!({
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    });
    let url = build_api_url(&account.base_url, endpoint);
    let response = auth_headers(
        http.post(url).timeout(Duration::from_secs(2)).json(&body),
        account,
    )
    .send()
    .await
    .map_err(|error| ChannelError::SendFailed(format!("Weixin {endpoint} failed: {error}")))?;
    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin {endpoint} HTTP {status}: {raw}"
        )));
    }
    if let Ok(payload) = serde_json::from_str::<Value>(&raw)
        && payload
            .get("ret")
            .and_then(Value::as_i64)
            .is_some_and(|ret| ret != 0)
    {
        let message = payload
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(ChannelError::SendFailed(format!(
            "Weixin {endpoint} ret!=0: {message}"
        )));
    }
    Ok(())
}

async fn notify_start(http: &Client, account: &WeixinAccount) -> Result<(), ChannelError> {
    notify_subscription(http, account, "msg/notifystart").await
}

async fn notify_stop(http: &Client, account: &WeixinAccount) -> Result<(), ChannelError> {
    notify_subscription(http, account, "msg/notifystop").await
}

fn build_get_config_body(to_user_id: &str, context_token: Option<&str>) -> Value {
    let mut body = json!({
        "ilink_user_id": to_user_id,
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    });
    if let Some(token) = context_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        body["context_token"] = Value::String(token.to_owned());
    }
    body
}

fn build_send_typing_body(to_user_id: &str, typing_ticket: &str, status: i64) -> Value {
    json!({
        "ilink_user_id": to_user_id,
        "typing_ticket": typing_ticket,
        "status": status,
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn get_upload_url(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    media_type: i64,
    filekey: &str,
    plaintext: &[u8],
    ciphertext_size: usize,
    aes_key_hex: &str,
) -> Result<UploadUrlResult, ChannelError> {
    let body = build_get_upload_url_body(
        filekey,
        to_user_id,
        media_type,
        plaintext,
        ciphertext_size,
        aes_key_hex,
    );
    let url = build_api_url(&account.base_url, "getuploadurl");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| {
            ChannelError::SendFailed(format!("Weixin getuploadurl failed: {error}"))
        })?;
    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getuploadurl HTTP {status}: {raw}"
        )));
    }
    let payload: WeixinGetUploadUrlResp = serde_json::from_str(&raw).map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin getuploadurl parse failed: {error}; body={raw}"
        ))
    })?;
    if payload.ret != 0 {
        return Err(ChannelError::SendFailed(format!(
            "Weixin getuploadurl ret!=0: {}",
            payload.errmsg
        )));
    }
    // SDK parity: prefer upload_full_url when present; fall back to upload_param.
    if let Some(full_url) = payload
        .upload_full_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        return Ok(UploadUrlResult::FullUrl(full_url.to_owned()));
    }
    let upload_param = payload.upload_param.trim().to_owned();
    if upload_param.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin getuploadurl returned empty upload_param and no upload_full_url".to_owned(),
        ));
    }
    Ok(UploadUrlResult::Param(upload_param))
}

fn build_get_upload_url_body(
    filekey: &str,
    to_user_id: &str,
    media_type: i64,
    plaintext: &[u8],
    ciphertext_size: usize,
    aes_key_hex: &str,
) -> Value {
    json!({
        "filekey": filekey,
        "media_type": media_type,
        "to_user_id": to_user_id,
        "rawsize": plaintext.len(),
        "rawfilemd5": format!("{:x}", md5::compute(plaintext)),
        "filesize": ciphertext_size,
        "no_need_thumb": true,
        "aeskey": aes_key_hex,
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn build_send_media_message_body(
    to_user_id: &str,
    context_token: &str,
    client_id: &str,
    uploaded: &UploadedWeixinMedia,
) -> Value {
    // Reference SDK parity: outbound `media.aes_key` is base64(hex(aes_key_raw)),
    // not base64(raw bytes).
    let aes_key_outbound = STANDARD.encode(bytes_to_hex(&uploaded.aes_key_raw).as_bytes());
    let media = json!({
        "encrypt_query_param": uploaded.download_encrypted_query_param,
        "aes_key": aes_key_outbound,
        "encrypt_type": 1
    });
    let media_item = match uploaded.media_type {
        2 => json!({
            "type": 5,
            "video_item": {
                "media": media,
                "video_size": uploaded.ciphertext_size
            }
        }),
        3 => json!({
            "type": 4,
            "file_item": {
                "media": media,
                "file_name": uploaded
                    .file_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("attachment.bin"),
                "len": uploaded.plaintext_size.to_string()
            }
        }),
        _ => json!({
            "type": 2,
            "image_item": {
                "media": media,
                "mid_size": uploaded.ciphertext_size
            }
        }),
    };
    json!({
        "msg": {
            "from_user_id": "",
            "to_user_id": to_user_id,
            "client_id": client_id,
            "message_type": 2,
            "message_state": 2,
            "context_token": context_token,
            "item_list": [media_item]
        },
        "base_info": {
            "channel_version": env!("CARGO_PKG_VERSION")
        }
    })
}

async fn upload_media_to_cdn(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    plaintext: &[u8],
    media_type: i64,
    file_name: Option<String>,
) -> Result<UploadedWeixinMedia, ChannelError> {
    upload_media_to_cdn_with_base(
        http,
        account,
        to_user_id,
        plaintext,
        media_type,
        DEFAULT_WEIXIN_CDN_BASE_URL,
        file_name,
    )
    .await
}

async fn upload_media_to_cdn_with_base(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    plaintext: &[u8],
    media_type: i64,
    cdn_base_url: &str,
    file_name: Option<String>,
) -> Result<UploadedWeixinMedia, ChannelError> {
    let aes_key_raw = random_16_bytes();
    let aes_key_hex = bytes_to_hex(&aes_key_raw);
    let ciphertext_size = aes_ecb_padded_size(plaintext.len());
    let filekey = bytes_to_hex(uuid::Uuid::new_v4().as_bytes());
    let upload_url_result = get_upload_url(
        http,
        account,
        to_user_id,
        media_type,
        &filekey,
        plaintext,
        ciphertext_size,
        &aes_key_hex,
    )
    .await?;
    let ciphertext = encrypt_aes_ecb(plaintext, &aes_key_raw)?;
    let cdn_url = match &upload_url_result {
        UploadUrlResult::FullUrl(full_url) => full_url.clone(),
        UploadUrlResult::Param(param) => build_cdn_upload_url(cdn_base_url, param, &filekey),
    };
    // SDK parity: retry up to CDN_UPLOAD_MAX_RETRIES times on 5xx errors.
    let mut last_error = String::new();
    let mut upload_ok = false;
    let mut response_holder: Option<reqwest::Response> = None;
    for attempt in 1..=CDN_UPLOAD_MAX_RETRIES {
        let resp = http
            .post(&cdn_url)
            .header("Content-Type", "application/octet-stream")
            .body(ciphertext.clone())
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                response_holder = Some(r);
                upload_ok = true;
                break;
            }
            Ok(r) if r.status().is_server_error() => {
                let status = r.status();
                let raw = r.text().await.unwrap_or_default();
                last_error = format!("HTTP {status}: {raw}");
                warn!(
                    attempt = attempt,
                    max = CDN_UPLOAD_MAX_RETRIES,
                    error = %last_error,
                    "weixin CDN upload 5xx, retrying"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Ok(r) => {
                // Client error (4xx) — don't retry
                let status = r.status();
                let raw = r.text().await.unwrap_or_default();
                return Err(ChannelError::SendFailed(format!(
                    "Weixin CDN upload HTTP {status}: {raw}"
                )));
            }
            Err(e) => {
                last_error = e.to_string();
                warn!(
                    attempt = attempt,
                    max = CDN_UPLOAD_MAX_RETRIES,
                    error = %last_error,
                    "weixin CDN upload network error, retrying"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    if !upload_ok {
        return Err(ChannelError::SendFailed(format!(
            "Weixin CDN upload failed after {CDN_UPLOAD_MAX_RETRIES} retries: {last_error}"
        )));
    }
    let response = response_holder.unwrap();
    let download_encrypted_query_param = response
        .headers()
        .get("x-encrypted-param")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if download_encrypted_query_param.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin CDN upload missing x-encrypted-param".to_owned(),
        ));
    }
    Ok(UploadedWeixinMedia {
        download_encrypted_query_param,
        aes_key_raw,
        plaintext_size: plaintext.len(),
        ciphertext_size,
        media_type,
        file_name: file_name
            .map(|n| sanitize_filename(&n))
            .filter(|n| !n.is_empty()),
    })
}

async fn download_remote_bytes(http: &Client, url: &str) -> Result<Vec<u8>, ChannelError> {
    let response = http.get(url).send().await.map_err(|error| {
        ChannelError::SendFailed(format!("Weixin media download failed: {error}"))
    })?;
    let status = response.status();
    if !status.is_success() {
        let raw = response.text().await.unwrap_or_default();
        return Err(ChannelError::SendFailed(format!(
            "Weixin media download HTTP {status}: {raw}"
        )));
    }
    response
        .bytes()
        .await
        .map(|value| value.to_vec())
        .map_err(|error| ChannelError::SendFailed(format!("Weixin media read failed: {error}")))
}

async fn download_and_decrypt_cdn(
    http: &Client,
    cdn_base_url: &str,
    encrypted_query_param: &str,
    aes_key: &[u8; 16],
) -> Result<Vec<u8>, ChannelError> {
    let url = build_cdn_download_url(cdn_base_url, encrypted_query_param);
    let encrypted = download_remote_bytes(http, &url).await?;
    decrypt_aes_ecb(&encrypted, aes_key)
}

async fn send_media_message(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    uploaded: &UploadedWeixinMedia,
    text: &str,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    // SDK parity: refuse to send if session is paused (errcode=-14).
    let acct_id = account_id_from_token(&account.token);
    if !acct_id.is_empty() && is_session_paused(acct_id).await {
        return Err(ChannelError::SendFailed(
            "Weixin session paused (errcode=-14); suppressing media send".to_owned(),
        ));
    }

    let target = to_user_id.trim();
    if target.is_empty() {
        return Err(ChannelError::Config(
            "Weixin target user_id is empty".to_owned(),
        ));
    }
    let token = context_token.map(str::trim).unwrap_or_default();
    if token.is_empty() {
        return Err(ChannelError::SendFailed(
            "Weixin context_token is missing; cannot send reply".to_owned(),
        ));
    }

    // Check token send counter — refuse if exhausted
    let remaining = token_sends_remaining(token).await;
    if remaining == 0 {
        warn!(
            to_user_id = target,
            token_limit = TOKEN_SEND_LIMIT,
            "context_token exhausted (hit send limit), refusing media send"
        );
        return Err(ChannelError::SendFailed(
            "Weixin context_token exhausted (send limit reached)".to_owned(),
        ));
    }

    // Reference SDK parity: send caption text as a standalone text message first,
    // then send media with a single item in item_list.
    if !text.trim().is_empty() {
        send_text_message(http, account, target, text, Some(token)).await?;
    }
    let client_id = uuid::Uuid::new_v4().to_string();
    let body = build_send_media_message_body(target, token, &client_id, uploaded);

    let url = build_api_url(&account.base_url, "sendmessage");
    let response = auth_headers(http.post(url).json(&body), account)
        .send()
        .await
        .map_err(|error| ChannelError::SendFailed(format!("Weixin sendmessage failed: {error}")))?;

    let status = response.status();
    let raw = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ChannelError::SendFailed(format!(
            "Weixin sendmessage HTTP {status}: {raw}"
        )));
    }

    if let Ok(payload) = serde_json::from_str::<Value>(&raw) {
        let ret = payload.get("ret").and_then(Value::as_i64);
        if ret.is_some_and(|r| r != 0) {
            let ret_code = ret.unwrap_or(-1);
            let message = payload
                .get("errmsg")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if ret_code == -14 {
                error!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin session expired during media send (ret=-14): bot needs re-login on the phone"
                );
                if !acct_id.is_empty() {
                    pause_session(acct_id).await;
                }
            }
            if ret_code == -2 {
                warn!(
                    to_user_id = target,
                    errmsg = message,
                    "Weixin context_token likely expired or exhausted during media send (ret=-2)"
                );
            }
            tracing::warn!(
                ret = ret_code,
                errmsg = message,
                response_body = %raw,
                "Weixin sendmessage (media) failed"
            );
            return Err(ChannelError::SendFailed(format!(
                "Weixin sendmessage ret={ret_code}: {message}"
            )));
        }
    }

    // Successful media send — increment token counter
    let count = token_send_increment(token).await;
    if let Some(c) = count {
        let left = TOKEN_SEND_LIMIT.saturating_sub(c);
        if left <= 2 {
            warn!(
                to_user_id = target,
                sends_used = c,
                sends_remaining = left,
                "context_token nearing send limit (media)"
            );
        }
    }

    Ok(client_id)
}

pub async fn send_image_message_from_path(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    image_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    send_image_message_from_path_with_cdn_base(
        http,
        account,
        to_user_id,
        image_path,
        caption,
        context_token,
        DEFAULT_WEIXIN_CDN_BASE_URL,
    )
    .await
}

pub async fn send_image_message_from_path_with_cdn_base(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    image_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
    cdn_base_url: &str,
) -> Result<String, ChannelError> {
    if !image_path.is_absolute() {
        return Err(ChannelError::SendFailed(
            "Weixin image path must be absolute".to_owned(),
        ));
    }
    let image_bytes = fs::read(image_path).await.map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin image read failed ({}): {error}",
            image_path.display()
        ))
    })?;
    let media_type = classify_media_type_from_url(image_path.to_string_lossy().as_ref());
    let uploaded = upload_media_to_cdn_with_base(
        http,
        account,
        to_user_id,
        &image_bytes,
        media_type,
        cdn_base_url,
        // Images render by their encrypted thumbnail, not filename — leave None.
        None,
    )
    .await?;
    send_media_message(
        http,
        account,
        to_user_id,
        &uploaded,
        caption.unwrap_or_default(),
        context_token,
    )
    .await
}

pub async fn send_file_message_from_path(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    file_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
) -> Result<String, ChannelError> {
    send_file_message_from_path_with_cdn_base(
        http,
        account,
        to_user_id,
        file_path,
        caption,
        context_token,
        DEFAULT_WEIXIN_CDN_BASE_URL,
    )
    .await
}

pub async fn send_file_message_from_path_with_cdn_base(
    http: &Client,
    account: &WeixinAccount,
    to_user_id: &str,
    file_path: &Path,
    caption: Option<&str>,
    context_token: Option<&str>,
    cdn_base_url: &str,
) -> Result<String, ChannelError> {
    if !file_path.is_absolute() {
        return Err(ChannelError::SendFailed(
            "Weixin file path must be absolute".to_owned(),
        ));
    }
    let file_bytes = fs::read(file_path).await.map_err(|error| {
        ChannelError::SendFailed(format!(
            "Weixin file read failed ({}): {error}",
            file_path.display()
        ))
    })?;
    let media_type = classify_media_type_from_url(file_path.to_string_lossy().as_ref());
    // Preserve the original file name so WeChat renders "foo.pdf" instead of
    // "attachment.bin" in the chat bubble. Fall back to None for paths whose
    // OsStr isn't valid UTF-8 — callers can still rely on the "attachment.bin"
    // default in `build_send_media_message_body`.
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_owned());
    let uploaded = upload_media_to_cdn_with_base(
        http,
        account,
        to_user_id,
        &file_bytes,
        media_type,
        cdn_base_url,
        file_name,
    )
    .await?;
    send_media_message(
        http,
        account,
        to_user_id,
        &uploaded,
        caption.unwrap_or_default(),
        context_token,
    )
    .await
}

/// Pre-compiled regexes for markdown-to-plain-text conversion.
/// Compiled once and cached for the process lifetime.
struct MarkdownRegexes {
    fenced_code: Regex,
    link: Regex,
    blockquote: Regex,
    heading: Regex,
    hr: Regex,
    bold_italic_star: Regex,
    bold_italic_under: Regex,
    bold_star: Regex,
    bold_under: Regex,
    italic_star: Regex,
    strikethrough: Regex,
    inline_code: Regex,
    table_separator: Regex,
    table_pipe: Regex,
}

fn markdown_regexes() -> &'static MarkdownRegexes {
    static REGEXES: OnceLock<MarkdownRegexes> = OnceLock::new();
    REGEXES.get_or_init(|| MarkdownRegexes {
        fenced_code: Regex::new(r"```[^\n]*\n?([\s\S]*?)```").unwrap(),
        link: Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap(),
        blockquote: Regex::new(r"(?m)^>\s?").unwrap(),
        heading: Regex::new(r"(?m)^#{1,6}\s+").unwrap(),
        hr: Regex::new(r"(?m)^[\s]*([-*_]){3,}[\s]*$").unwrap(),
        bold_italic_star: Regex::new(r"\*{3}(.+?)\*{3}").unwrap(),
        bold_italic_under: Regex::new(r"_{3}(.+?)_{3}").unwrap(),
        bold_star: Regex::new(r"\*{2}(.+?)\*{2}").unwrap(),
        bold_under: Regex::new(r"_{2}(.+?)_{2}").unwrap(),
        italic_star: Regex::new(r"\*(.+?)\*").unwrap(),
        strikethrough: Regex::new(r"~~(.+?)~~").unwrap(),
        inline_code: Regex::new(r"`([^`]+)`").unwrap(),
        // Only match lines that look like table rows (start/end with |)
        table_separator: Regex::new(r"(?m)^\|[\s:-]+(\|[\s:-]*)+\|?\s*$").unwrap(),
        table_pipe: Regex::new(r"(?m)^(\|.+\|)\s*$").unwrap(),
    })
}

fn markdown_to_plain_text(text: &str) -> String {
    let re = markdown_regexes();
    let mut result = text.to_owned();

    // 1. Fenced code blocks: ```lang\ncode``` → code
    result = re.fenced_code.replace_all(&result, "$1").to_string();
    // 2. Markdown images: ![alt](url) → (remove entirely)
    result = markdown_image_regex().replace_all(&result, "").to_string();
    // 3. Markdown links: [text](url) → text
    result = re.link.replace_all(&result, "$1").to_string();
    // 4. Blockquotes: > text → text
    result = re.blockquote.replace_all(&result, "").to_string();
    // 5. Headings: # ... → strip leading hashes
    result = re.heading.replace_all(&result, "").to_string();
    // 6. Horizontal rules: ---, ***, ___ → empty
    result = re.hr.replace_all(&result, "").to_string();
    // 7. Bold + italic: ***text*** / ___text___
    result = re.bold_italic_star.replace_all(&result, "$1").to_string();
    result = re.bold_italic_under.replace_all(&result, "$1").to_string();
    // 8. Bold: **text** / __text__
    result = re.bold_star.replace_all(&result, "$1").to_string();
    result = re.bold_under.replace_all(&result, "$1").to_string();
    // 9. Italic: *text*
    result = re.italic_star.replace_all(&result, "$1").to_string();
    // 10. Strikethrough: ~~text~~
    result = re.strikethrough.replace_all(&result, "$1").to_string();
    // 11. Inline code: `code`
    result = re.inline_code.replace_all(&result, "$1").to_string();
    // 12. Tables: remove separator rows, then strip pipes only from table rows
    //     (rows that start and end with |)
    result = re.table_separator.replace_all(&result, "").to_string();
    result = re
        .table_pipe
        .replace_all(&result, |caps: &regex::Captures| {
            caps[1].replace('|', " ").trim().to_owned()
        })
        .to_string();

    result
}

fn looks_like_local_media_path(input: &str) -> bool {
    let candidate = input.trim();
    if !candidate.starts_with('/') {
        return false;
    }
    Path::new(candidate).extension().is_some()
}

/// Check whether a URL/path looks like a known media file (image, video, or
/// a handful of common document types).  Arbitrary URLs that don't match a
/// known media extension are **not** treated as media – this prevents tool
/// results containing API URLs (e.g. `https://api.github.com/…`) from being
/// downloaded and sent as `attachment.bin`.
fn looks_like_known_media_url(url: &str) -> bool {
    let lower = url
        .split('?')
        .next()
        .unwrap_or(url)
        .split('#')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
        || lower.ends_with(".mp4")
        || lower.ends_with(".mov")
        || lower.ends_with(".mkv")
        || lower.ends_with(".webm")
        || lower.ends_with(".pdf")
}

fn media_ref_from_string(input: &str) -> Option<OutboundMediaRef> {
    let candidate = input
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`')
        .trim();
    if candidate.is_empty() {
        return None;
    }
    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        // Only treat remote URLs as media when they have a recognisable media
        // extension; otherwise tool-result URLs like GitHub API endpoints get
        // downloaded and sent as `attachment.bin`.
        if looks_like_known_media_url(candidate) {
            return Some(OutboundMediaRef::RemoteUrl(candidate.to_owned()));
        }
        return None;
    }
    if let Some(rest) = candidate.strip_prefix("file://") {
        let decoded = urlencoding::decode(rest).ok()?.to_string();
        if looks_like_local_media_path(&decoded) {
            return Some(OutboundMediaRef::LocalPath(decoded));
        }
        return None;
    }
    if looks_like_local_media_path(candidate) {
        return Some(OutboundMediaRef::LocalPath(candidate.to_owned()));
    }
    None
}

fn extract_markdown_media_refs(text: &str) -> Vec<OutboundMediaRef> {
    markdown_image_regex()
        .captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_owned()))
        .filter_map(|raw| media_ref_from_string(&raw))
        .collect()
}

fn extract_media_refs_from_value(value: &Value, out: &mut Vec<OutboundMediaRef>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match value {
        Value::String(text) => {
            if let Some(media_ref) = media_ref_from_string(text) {
                out.push(media_ref);
            }
            for media_ref in extract_markdown_media_refs(text) {
                if out.len() >= limit {
                    break;
                }
                out.push(media_ref);
            }
        }
        Value::Array(items) => {
            for item in items {
                if out.len() >= limit {
                    break;
                }
                extract_media_refs_from_value(item, out, limit);
            }
        }
        Value::Object(object) => {
            for key in ["image_path", "image", "image_url", "url", "path"] {
                if let Some(value) = object.get(key) {
                    extract_media_refs_from_value(value, out, limit);
                }
            }
            for value in object.values() {
                if out.len() >= limit {
                    break;
                }
                extract_media_refs_from_value(value, out, limit);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn extract_media_refs_from_provider_message(
    message: &garyx_models::provider::ProviderMessage,
) -> Vec<OutboundMediaRef> {
    if let Some(image) = extract_image_generation_result(message) {
        return vec![OutboundMediaRef::InlineImage {
            id: image.id.clone(),
            file_name: image.file_name(),
            bytes: image.bytes,
        }];
    }

    let mut refs = Vec::new();
    if let Some(text) = message.text.as_deref() {
        extract_media_refs_from_value(&Value::String(text.to_owned()), &mut refs, 4);
    }
    extract_media_refs_from_value(&message.content, &mut refs, 4);
    refs
}

async fn load_media_bytes(
    http: &Client,
    media_ref: &OutboundMediaRef,
) -> Result<Vec<u8>, ChannelError> {
    match media_ref {
        OutboundMediaRef::RemoteUrl(url) => download_remote_bytes(http, url).await,
        OutboundMediaRef::LocalPath(path) => fs::read(path).await.map_err(|error| {
            ChannelError::SendFailed(format!("Weixin local media read failed ({path}): {error}"))
        }),
        OutboundMediaRef::InlineImage { bytes, .. } => Ok(bytes.clone()),
    }
}

fn classify_media_type_from_url(url: &str) -> i64 {
    let lower = url
        .split('?')
        .next()
        .unwrap_or(url)
        .split('#')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if lower.ends_with(".mp4")
        || lower.ends_with(".mov")
        || lower.ends_with(".mkv")
        || lower.ends_with(".webm")
    {
        return 2;
    }
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
    {
        return 1;
    }
    3
}

#[derive(Clone)]
struct WeixinInboundRuntime {
    http: Client,
    account_id: String,
    account: WeixinAccount,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
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
}

#[derive(Clone)]
struct WeixinStreamConsumerContext {
    http: Client,
    account: WeixinAccount,
    account_id: String,
    user_id: String,
    context_token: String,
    router: Arc<Mutex<MessageRouter>>,
    thread_id: Arc<std::sync::Mutex<String>>,
    typing_ticket: Option<String>,
    running: Arc<AtomicBool>,
}

enum LiveTextSendResult {
    Sent,
    Noop,
    Poisoned,
}

fn current_stream_thread_id(ctx: &WeixinStreamConsumerContext) -> String {
    match ctx.thread_id.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => String::new(),
    }
}

async fn resolve_stream_context_token(
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

async fn open_live_message_for_context(
    ctx: &WeixinStreamConsumerContext,
    prefer_latest_token: bool,
) -> LiveMessage {
    LiveMessage::open(resolve_stream_context_token(ctx, prefer_latest_token).await).await
}

async fn record_stream_outbound(ctx: &WeixinStreamConsumerContext, client_id: &str) {
    let thread_id = current_stream_thread_id(ctx);
    if thread_id.trim().is_empty() {
        return;
    }
    let mut router_guard = ctx.router.lock().await;
    router_guard
        .record_outbound_message_with_persistence(
            &thread_id,
            "weixin",
            &ctx.account_id,
            &ctx.user_id,
            None,
            client_id,
        )
        .await;
}

async fn ensure_stream_typing(
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

async fn stop_stream_typing(
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

fn classify_text_send_error(error: &ChannelError) -> PoisonReason {
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

async fn send_live_generating(
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

async fn finalize_live_message(
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
            record_stream_outbound(ctx, &live.client_id).await;
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

async fn drain_live_media(
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
            Ok(message_id) => {
                sent_media_refs.insert(dedupe_key);
                *calls_used = calls_used.saturating_add(1);
                record_stream_outbound(ctx, &message_id).await;
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

fn collect_poisoned_text(live: &mut LiveMessage, poisoned_texts: &mut Vec<String>) {
    if let Some(text) = live.take_poisoned_text() {
        poisoned_texts.push(text);
    }
}

#[allow(clippy::too_many_arguments)]
async fn close_live_for_boundary(
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

async fn queue_poisoned_texts(ctx: &WeixinStreamConsumerContext, poisoned_texts: &[String]) {
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

async fn run_streaming_update_consumer(
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

impl WeixinChannel {
    pub fn new(
        config: WeixinConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_millis(DEFAULT_LONG_POLL_TIMEOUT_MS + 15_000))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            config,
            http,
            running: Arc::new(AtomicBool::new(false)),
            poll_tasks: Vec::new(),
            router,
            bridge,
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
        let response_router = runtime.router.clone();
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
                router: response_router,
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
                    response_router: &Arc<Mutex<MessageRouter>>,
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
                    let message_id = if let Some(message_id) = maybe_message_id {
                        message_id
                    } else if !plain_text.is_empty() {
                        match send_text_message(
                            response_http,
                            response_account,
                            response_user_id,
                            &plain_text,
                            token.as_deref(),
                        )
                        .await
                        {
                            Ok(message_id) => message_id,
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
                    } else {
                        return;
                    };

                    if !thread_id.trim().is_empty() {
                        let mut router_guard = response_router.lock().await;
                        router_guard
                            .record_outbound_message_with_persistence(
                                &thread_id,
                                "weixin",
                                response_account_id,
                                response_user_id,
                                None,
                                &message_id,
                            )
                            .await;
                    }
                }

                while let Some(event) = event_rx.recv().await {
                    match event {
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
                                    &response_router,
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
                                &response_router,
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
            reply_to_message_id: None,
            images: Vec::new(),
            extra_metadata: metadata,
            file_paths: file_paths_for_agent,
        };

        let result = {
            let mut router_guard = runtime.router.lock().await;
            router_guard
                .route_and_dispatch(request, runtime.bridge.as_ref(), Some(response_callback))
                .await
        };

        match result {
            Ok(result) => {
                if let Ok(mut holder) = thread_id_holder.lock() {
                    *holder = result.thread_id.clone();
                }
                if let Some(local_reply) = result.local_reply {
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
                                Ok(message_id) => {
                                    let mut router_guard = runtime.router.lock().await;
                                    router_guard
                                        .record_outbound_message_with_persistence(
                                            &result.thread_id,
                                            "weixin",
                                            &runtime.account_id,
                                            &from_id,
                                            None,
                                            &message_id,
                                        )
                                        .await;
                                }
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
            Err(error) => {
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

#[cfg(test)]
mod tests;
