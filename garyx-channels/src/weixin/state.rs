//! Weixin process-level shared state: session pauses, poll-cursor
//! persistence, token send budgets, metrics counters, the pending
//! outbound queue, and context-token/typing-ticket stores. Moved
//! verbatim from weixin.rs (Phase-7 pure code motion).

use super::*;

pub(super) type ContextTokenStore = Arc<Mutex<HashMap<String, String>>>;

pub(super) type TypingTicketStore = Arc<Mutex<HashMap<String, String>>>;

// ---------------------------------------------------------------------------
// Session guard — pause all API calls for an account after errcode=-14.
// ---------------------------------------------------------------------------

pub(super) type SessionPauseStore = Arc<Mutex<HashMap<String, std::time::Instant>>>;

pub(super) fn session_pause_store() -> &'static SessionPauseStore {
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

pub(super) fn get_updates_buf_persistence_path() -> std::path::PathBuf {
    garyx_models::local_paths::default_session_data_dir().join("weixin_getupdates_buf.json")
}

pub(super) fn load_get_updates_buf() -> HashMap<String, String> {
    let path = get_updates_buf_persistence_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return HashMap::new(),
    };
    serde_json::from_str(&data)
        .map_err(|e| warn!(path = %path.display(), error = %e, "failed to parse weixin getupdates_buf JSON"))
        .unwrap_or_default()
}

pub(super) fn persist_get_updates_buf(store: &HashMap<String, String>) {
    let path = get_updates_buf_persistence_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(store) {
        let _ = std::fs::write(&path, json);
    }
}

pub(super) type GetUpdatesBufStore = Arc<Mutex<HashMap<String, String>>>;

pub(super) fn get_updates_buf_store() -> &'static GetUpdatesBufStore {
    static STORE: OnceLock<GetUpdatesBufStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let map = load_get_updates_buf();
        if !map.is_empty() {
            info!(count = map.len(), "loaded persisted weixin getupdates_buf");
        }
        Arc::new(Mutex::new(map))
    })
}

pub(super) async fn get_persisted_cursor(account_id: &str) -> String {
    let store = get_updates_buf_store().lock().await;
    store.get(account_id).cloned().unwrap_or_default()
}

pub(super) async fn set_persisted_cursor(account_id: &str, cursor: &str) {
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
pub(super) const TOKEN_SEND_LIMIT: u32 = 9;

pub(super) type TokenSendCountStore = Arc<Mutex<HashMap<String, u32>>>;

pub(super) fn token_send_count_store() -> &'static TokenSendCountStore {
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

pub(super) static WEIXIN_MEDIA_DROPPED_TOTAL: AtomicU64 = AtomicU64::new(0);
pub(super) static WEIXIN_SEND_CALLS_TOTAL: AtomicU64 = AtomicU64::new(0);

pub(super) type FinalizeReasonCounterStore = Arc<Mutex<HashMap<&'static str, u64>>>;

pub(super) fn finalize_reason_counter_store() -> &'static FinalizeReasonCounterStore {
    static STORE: OnceLock<FinalizeReasonCounterStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub(super) async fn record_weixin_finalize_reason(reason: FinalizeReason) {
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

pub(super) fn record_weixin_media_dropped(count: usize) {
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

pub(super) fn record_weixin_send_calls_per_inbound(count: u32) {
    WEIXIN_SEND_CALLS_TOTAL.fetch_add(count as u64, Ordering::Relaxed);
    debug!(
        metric = "weixin_send_calls_per_inbound",
        count, "weixin send calls used for inbound"
    );
}

#[derive(Clone, Debug)]
// ---------------------------------------------------------------------------
// Pending outbound message queue — when weixin sends fail (e.g. token expired),
// messages are queued here and flushed when a new inbound message arrives with
// a fresh context_token.
// ---------------------------------------------------------------------------

pub struct PendingOutboundMessage {
    pub to_user_id: String,
    pub text: String,
    pub queued_at: std::time::Instant,
}

pub(super) type PendingOutboundStore = Arc<Mutex<HashMap<String, Vec<PendingOutboundMessage>>>>;

pub(super) fn pending_outbound_store() -> &'static PendingOutboundStore {
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

pub(super) fn context_token_persistence_path() -> std::path::PathBuf {
    garyx_models::local_paths::default_session_data_dir().join("weixin_context_tokens.json")
}

pub(super) fn context_token_store() -> &'static ContextTokenStore {
    static STORE: OnceLock<ContextTokenStore> = OnceLock::new();
    STORE.get_or_init(|| {
        let map = load_context_tokens_from_disk().unwrap_or_default();
        if !map.is_empty() {
            info!(count = map.len(), "loaded persisted weixin context tokens");
        }
        Arc::new(Mutex::new(map))
    })
}

pub(super) fn load_context_tokens_from_disk() -> Option<HashMap<String, String>> {
    let path = context_token_persistence_path();
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data)
        .map_err(|e| warn!(path = %path.display(), error = %e, "failed to parse weixin context tokens JSON"))
        .ok()
}

pub(super) fn persist_context_tokens(store: &HashMap<String, String>) {
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

pub(super) fn context_token_key(account_id: &str, user_id: &str) -> String {
    format!("{account_id}:{user_id}")
}

pub(super) fn context_token_thread_key(account_id: &str, user_id: &str, thread_id: &str) -> String {
    format!("{account_id}:{user_id}:thread:{thread_id}")
}

pub(super) fn typing_ticket_store() -> &'static TypingTicketStore {
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
