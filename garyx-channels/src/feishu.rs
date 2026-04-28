//! Feishu/Lark channel implementation.
//!
//! Provides multi-account Feishu bot integration with:
//! - Tenant access token management with automatic refresh
//! - WebSocket-based event listening
//! - Message sending/replying via HTTP API
//! - Interactive card message formatting
//! - Bot mention detection

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

#[cfg(test)]
use reqwest::Client as HttpClient;
#[cfg(test)]
use serde_json::Value;
use tokio::sync::Mutex;
#[cfg(test)]
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{info, warn};

use garyx_bridge::MultiProviderBridge;
#[cfg(test)]
use garyx_models::config::FeishuDomain;
use garyx_models::config::{FeishuAccount, FeishuConfig, TopicSessionMode};
use garyx_router::MessageRouter;

use crate::channel_trait::{Channel, ChannelError};

mod auth_flow_executor;
mod client;
mod device_auth;
mod mentions;
mod message;
mod pbbp2;
mod policy;
mod types;
mod ws;

use client::FeishuClient;
use types::{FeishuResponseStreamState, MentionTarget};

pub use auth_flow_executor::FeishuAuthExecutor;
pub use device_auth::{
    DeviceFlowBegin, DeviceFlowError, DeviceFlowResult, PollStatus, begin_app_registration,
    build_verification_url, poll_once, run_device_flow,
};
#[doc(hidden)]
pub use device_auth::{begin_app_registration_at, poll_once_at};
pub use mentions::{is_mentioned, strip_mention_tokens};
pub use message::{build_card_content, build_text_content, extract_message_text};
pub use policy::{
    apply_mention_context_limit, is_dm_message_allowed, is_group_message_allowed,
    requires_group_mention, resolve_topic_session_mode,
};
pub use types::{
    FeishuEventEnvelope, FeishuEventHeader, ImMention, ImMentionId, ImMessage,
    ImMessageReceiveEvent, ImSender, ImSenderId,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FEISHU_API_BASE: &str = "https://open.feishu.cn/open-apis";
const LARK_API_BASE: &str = "https://open.larksuite.com/open-apis";

/// How many minutes before expiry to proactively refresh the token.
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

/// Default token lifetime when the API doesn't return one (2 hours).
const DEFAULT_TOKEN_LIFETIME: Duration = Duration::from_secs(7200);

/// Reconnect delay after WebSocket disconnect.
const WS_RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// Maximum reconnect delay (exponential backoff cap).
const WS_MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const SENDER_NAME_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const PROCESSING_REACTION_EMOJI: &str = "Typing";
const PERMISSION_ERROR_CODE: i64 = 99991672;
const PERMISSION_ERROR_COOLDOWN: Duration = Duration::from_secs(300);
const POLICY_BLOCK_COUNTER_MAX_KEYS: usize = 2048;
const PENDING_GROUP_HISTORY_MAX_KEYS: usize = 4096;
const SENDER_NAME_CACHE_MAX_KEYS: usize = 8192;
const PERMISSION_NOTICE_CACHE_MAX_KEYS: usize = 4096;
const SEEN_EVENT_ID_TTL: Duration = Duration::from_secs(30 * 60);
const SEEN_EVENT_ID_MAX_KEYS: usize = 16384;
const MENTION_CONTEXT_LIMIT: i64 = 20;

fn policy_block_counters() -> &'static StdMutex<HashMap<String, u64>> {
    static COUNTERS: OnceLock<StdMutex<HashMap<String, u64>>> = OnceLock::new();
    COUNTERS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn pending_group_history() -> &'static StdMutex<HashMap<String, Vec<String>>> {
    static HISTORY: OnceLock<StdMutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    HISTORY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn sender_name_cache() -> &'static StdMutex<HashMap<String, (String, Instant)>> {
    static CACHE: OnceLock<StdMutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn permission_error_notice_cache() -> &'static StdMutex<HashMap<String, Instant>> {
    static CACHE: OnceLock<StdMutex<HashMap<String, Instant>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Returns true if this event_id has already been seen (duplicate), false if it's new.
/// New event_ids are inserted into the cache automatically.
fn is_duplicate_event(event_id: &str) -> bool {
    static CACHE: OnceLock<StdMutex<HashMap<String, Instant>>> = OnceLock::new();
    let cache_ref = CACHE.get_or_init(|| StdMutex::new(HashMap::new()));
    let now = Instant::now();
    match cache_ref.lock() {
        Ok(mut cache) => {
            // Prune expired entries
            cache.retain(|_, expires_at| *expires_at > now);
            evict_excess_entries(
                &mut cache,
                SEEN_EVENT_ID_MAX_KEYS,
                Some(&event_id.to_owned()),
            );
            if cache.contains_key(event_id) {
                return true;
            }
            cache.insert(event_id.to_owned(), now + SEEN_EVENT_ID_TTL);
            false
        }
        Err(_) => {
            warn!("seen_event_ids cache mutex poisoned");
            false
        }
    }
}

fn pending_history_key(account_id: &str, chat_id: &str) -> String {
    format!("{account_id}:{chat_id}")
}

fn evict_excess_entries<K, V>(map: &mut HashMap<K, V>, max_entries: usize, keep_key: Option<&K>)
where
    K: Eq + Hash + Clone,
{
    if map.len() <= max_entries {
        return;
    }
    while map.len() > max_entries {
        let to_remove = map
            .keys()
            .find(|k| keep_key != Some(*k))
            .cloned()
            .or_else(|| map.keys().next().cloned());
        if let Some(key) = to_remove {
            map.remove(&key);
        } else {
            break;
        }
    }
}

fn prune_sender_name_cache_locked(
    cache: &mut HashMap<String, (String, Instant)>,
    now: Instant,
    keep_key: Option<&str>,
) {
    cache.retain(|_, (_, expires_at)| *expires_at > now);
    let keep_owned = keep_key.map(str::to_owned);
    evict_excess_entries(cache, SENDER_NAME_CACHE_MAX_KEYS, keep_owned.as_ref());
}

fn append_pending_history(account_id: &str, chat_id: &str, entry: String, limit: i64) {
    if limit <= 0 {
        return;
    }

    let key = pending_history_key(account_id, chat_id);
    let mut history_map = match pending_group_history().lock() {
        Ok(history_map) => history_map,
        Err(_) => {
            warn!("pending group history mutex poisoned");
            return;
        }
    };
    let history = history_map.entry(key).or_default();
    history.push(entry);
    apply_mention_context_limit(history, limit);
    let keep_key = pending_history_key(account_id, chat_id);
    evict_excess_entries(
        &mut history_map,
        PENDING_GROUP_HISTORY_MAX_KEYS,
        Some(&keep_key),
    );
}

fn get_pending_history(account_id: &str, chat_id: &str) -> Vec<String> {
    let key = pending_history_key(account_id, chat_id);
    let history_map = match pending_group_history().lock() {
        Ok(history_map) => history_map,
        Err(_) => {
            warn!("pending group history mutex poisoned");
            return Vec::new();
        }
    };
    history_map.get(&key).cloned().unwrap_or_default()
}

fn clear_pending_history(account_id: &str, chat_id: &str) {
    let key = pending_history_key(account_id, chat_id);
    if let Ok(mut history_map) = pending_group_history().lock() {
        history_map.remove(&key);
    } else {
        warn!("pending group history mutex poisoned");
    }
}

fn record_policy_block(scope: &str, reason: &str) {
    let key = format!("{scope}:{reason}");
    let mut counters = match policy_block_counters().lock() {
        Ok(counters) => counters,
        Err(_) => {
            warn!("policy block counter mutex poisoned");
            return;
        }
    };
    *counters.entry(key).or_insert(0) += 1;
    evict_excess_entries(
        &mut counters,
        POLICY_BLOCK_COUNTER_MAX_KEYS,
        None::<&String>,
    );
}

/// Snapshot Feishu policy rejection counters by `scope:reason`.
pub fn policy_block_counters_snapshot() -> HashMap<String, u64> {
    match policy_block_counters().lock() {
        Ok(counters) => counters.clone(),
        Err(_) => {
            warn!("policy block counter mutex poisoned");
            HashMap::new()
        }
    }
}

#[cfg(test)]
fn reset_permission_error_notice_cache() {
    permission_error_notice_cache()
        .lock()
        .expect("permission error notice cache mutex poisoned")
        .clear();
}

fn extract_permission_grant_url(err_text: &str) -> Option<Option<String>> {
    if !err_text.contains(&PERMISSION_ERROR_CODE.to_string()) {
        return None;
    }

    let http_idx = err_text
        .find("http://")
        .or_else(|| err_text.find("https://"));
    let Some(start) = http_idx else {
        return Some(None);
    };
    let suffix = &err_text[start..];
    let raw_url = suffix.split_whitespace().next().unwrap_or_default();
    if raw_url.is_empty() || !raw_url.contains("appPermission") {
        return Some(None);
    }
    let cleaned = raw_url
        .trim_matches(|c: char| {
            matches!(
                c,
                '.' | ',' | ';' | ':' | '"' | '\'' | ')' | ']' | '>' | '(' | '[' | '<'
            )
        })
        .to_owned();
    if cleaned.is_empty() {
        Some(None)
    } else {
        Some(Some(cleaned))
    }
}

fn should_emit_permission_notice(account_id: &str, chat_id: &str) -> bool {
    let cache_key = format!("{account_id}:{chat_id}");
    let now = Instant::now();
    let mut cache = match permission_error_notice_cache().lock() {
        Ok(cache) => cache,
        Err(_) => {
            warn!("permission error notice cache mutex poisoned");
            return true;
        }
    };
    cache.retain(|_, last_at| now.saturating_duration_since(*last_at) < PERMISSION_ERROR_COOLDOWN);
    evict_excess_entries(
        &mut cache,
        PERMISSION_NOTICE_CACHE_MAX_KEYS,
        Some(&cache_key),
    );
    if let Some(last_at) = cache.get(&cache_key) {
        if now.saturating_duration_since(*last_at) < PERMISSION_ERROR_COOLDOWN {
            return false;
        }
    }
    cache.insert(cache_key, now);
    true
}

async fn notify_permission_error_if_needed(
    client: &FeishuClient,
    account_id: &str,
    chat_id: &str,
    reply_to_message_id: &str,
    err_text: &str,
) {
    let Some(grant_url) = extract_permission_grant_url(err_text) else {
        return;
    };
    if !should_emit_permission_notice(account_id, chat_id) {
        return;
    }

    let mut msg = "Bot encountered a Feishu API permission error.".to_owned();
    if let Some(url) = grant_url {
        msg.push_str("\nPlease ask your admin to grant permissions: ");
        msg.push_str(&url);
    } else {
        msg.push_str("\nPlease check the bot's API permissions in Feishu admin console.");
    }

    if let Err(err) = send_native_command_reply(client, reply_to_message_id, &msg).await {
        warn!(
            account_id = %account_id,
            chat_id = %chat_id,
            message_id = %reply_to_message_id,
            error = %err,
            "failed to send permission error notice"
        );
    }
}

// ---------------------------------------------------------------------------
// FeishuChannel — the top-level channel
// ---------------------------------------------------------------------------

/// Multi-account Feishu channel handler.
pub struct FeishuChannel {
    config: FeishuConfig,
    clients: HashMap<String, FeishuClient>,
    running: Arc<AtomicBool>,
    ws_tasks: Vec<JoinHandle<()>>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    public_url: String,
}

impl FeishuChannel {
    /// Create a new Feishu channel from configuration.
    pub fn new(
        config: FeishuConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        public_url: String,
    ) -> Self {
        let mut clients = HashMap::new();
        for (id, account) in &config.accounts {
            if account.enabled {
                clients.insert(id.clone(), FeishuClient::new(account));
            }
        }

        Self {
            config,
            clients,
            running: Arc::new(AtomicBool::new(false)),
            ws_tasks: Vec::new(),
            router,
            bridge,
            public_url,
        }
    }

    /// Start all enabled accounts: fetch tokens and spawn WebSocket listeners.
    async fn start_inner(&mut self) -> Result<(), FeishuError> {
        self.running.store(true, Ordering::SeqCst);

        for (account_id, client) in &self.clients {
            // Pre-fetch access token (non-fatal: ws_listen_loop will retry)
            match client.get_access_token().await {
                Ok(_) => info!(account_id = %account_id, "Feishu access token acquired"),
                Err(e) => {
                    warn!(account_id = %account_id, error = %e, "Failed to acquire Feishu access token on startup, will retry in WS loop");
                }
            }

            // Spawn WebSocket listener task
            let running = self.running.clone();
            let aid = account_id.clone();
            let client = client.clone();
            let router = self.router.clone();
            let bridge = self.bridge.clone();
            let Some(account_cfg) = self.config.accounts.get(account_id).cloned() else {
                warn!(
                    account_id = %account_id,
                    "skipping Feishu account with missing runtime config"
                );
                continue;
            };

            let public_url = self.public_url.clone();
            let handle = tokio::spawn(async move {
                ws_listen_loop(
                    &aid,
                    &client,
                    &account_cfg,
                    running,
                    router,
                    bridge,
                    &public_url,
                )
                .await;
            });
            self.ws_tasks.push(handle);
        }

        Ok(())
    }

    /// Stop all WebSocket listeners and clean up.
    async fn stop_inner(&mut self) {
        self.running.store(false, Ordering::SeqCst);

        for handle in self.ws_tasks.drain(..) {
            handle.abort();
        }

        info!("Feishu channel stopped");
    }

    /// Send a message to a chat via the specified account.
    pub async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        content: &str,
        msg_type: &str,
    ) -> Result<String, FeishuError> {
        let client = self
            .clients
            .get(account_id)
            .ok_or_else(|| FeishuError::Api {
                code: -1,
                msg: format!("account not found: {account_id}"),
            })?;
        client.send_message(chat_id, content, msg_type).await
    }

    /// Reply to a message via the specified account.
    pub async fn reply_message(
        &self,
        account_id: &str,
        message_id: &str,
        content: &str,
        msg_type: &str,
    ) -> Result<String, FeishuError> {
        let client = self
            .clients
            .get(account_id)
            .ok_or_else(|| FeishuError::Api {
                code: -1,
                msg: format!("account not found: {account_id}"),
            })?;
        client.reply_message(message_id, content, msg_type).await
    }

    /// Build an interactive card content JSON string from markdown text.
    pub fn build_card_content(text: &str) -> String {
        build_card_content(text)
    }

    /// Check if a bot was mentioned in the event's mention list.
    pub fn is_mentioned(mentions: &[ImMention], bot_open_id: &str) -> bool {
        is_mentioned(mentions, bot_open_id)
    }

    /// Get a reference to the config.
    pub fn config(&self) -> &FeishuConfig {
        &self.config
    }

    /// Check if the channel is running.
    pub fn is_channel_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Channel for FeishuChannel {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn start(&mut self) -> Result<(), ChannelError> {
        self.start_inner()
            .await
            .map_err(|e| ChannelError::Connection(e.to_string()))
    }

    async fn stop(&mut self) -> Result<(), ChannelError> {
        self.stop_inner().await;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.is_channel_running()
    }
}

// ---------------------------------------------------------------------------
// WebSocket listener loop
// ---------------------------------------------------------------------------

async fn ws_listen_loop(
    account_id: &str,
    client: &FeishuClient,
    account: &FeishuAccount,
    running: Arc<AtomicBool>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    public_url: &str,
) {
    ws::ws_listen_loop(
        account_id, client, account, running, router, bridge, public_url,
    )
    .await;
}

async fn send_native_command_reply(
    client: &FeishuClient,
    reply_to_message_id: &str,
    text: &str,
) -> Result<(), FeishuError> {
    let content = build_text_content(text);
    client
        .reply_message(reply_to_message_id, &content, "text")
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FeishuError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Feishu API error (code={code}): {msg}")]
    Api { code: i64, msg: String },

    #[error("WebSocket error: {0}")]
    WebSocket(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
