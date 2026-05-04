use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

use tokio::sync::Mutex;

use garyx_router::MessageRouter;

use super::{DEDUP_MAX_SIZE, DEDUP_TTL_SECONDS};

type MessageDedupStore = Arc<Mutex<HashMap<String, std::time::Instant>>>;

fn message_dedup_store() -> &'static MessageDedupStore {
    static STORE: OnceLock<MessageDedupStore> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

struct DedupScopeRegistry {
    next_id: u64,
    scopes: HashMap<usize, (Weak<Mutex<MessageRouter>>, u64)>,
}

type DedupScopeRegistryStore = Arc<Mutex<DedupScopeRegistry>>;

fn dedup_scope_registry() -> &'static DedupScopeRegistryStore {
    static STORE: OnceLock<DedupScopeRegistryStore> = OnceLock::new();
    STORE.get_or_init(|| {
        Arc::new(Mutex::new(DedupScopeRegistry {
            next_id: 1,
            scopes: HashMap::new(),
        }))
    })
}

pub(super) async fn dedup_scope_id(router: &Arc<Mutex<MessageRouter>>) -> u64 {
    let ptr = Arc::as_ptr(router) as usize;
    let mut registry = dedup_scope_registry().lock().await;

    if let Some((weak_router, scope_id)) = registry.scopes.get(&ptr)
        && weak_router.upgrade().is_some()
    {
        return *scope_id;
    }

    let scope_id = registry.next_id;
    registry.next_id = registry.next_id.saturating_add(1);
    registry
        .scopes
        .insert(ptr, (Arc::downgrade(router), scope_id));
    scope_id
}

fn dedup_key(
    dedup_scope_id: u64,
    account_id: &str,
    api_base: &str,
    chat_id: i64,
    message_id: i64,
) -> String {
    format!("telegram::{dedup_scope_id}::{account_id}::{api_base}::{chat_id}::{message_id}")
}

pub(super) async fn is_duplicate_message(
    dedup_scope_id: u64,
    account_id: &str,
    api_base: &str,
    chat_id: i64,
    message_id: i64,
) -> bool {
    let key = dedup_key(dedup_scope_id, account_id, api_base, chat_id, message_id);
    let now = std::time::Instant::now();
    let ttl = std::time::Duration::from_secs(DEDUP_TTL_SECONDS);
    let mut store = message_dedup_store().lock().await;

    if let Some(existing_ts) = store.get(&key)
        && now.saturating_duration_since(*existing_ts) < ttl
    {
        return true;
    }

    store.insert(key, now);

    // Prune expired entries first.
    store.retain(|_, ts| now.saturating_duration_since(*ts) < ttl);

    // Enforce maximum cache size by evicting oldest entries.
    if store.len() > DEDUP_MAX_SIZE {
        let mut ordered = store
            .iter()
            .map(|(k, ts)| (k.clone(), *ts))
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(_, ts)| *ts);
        let excess = store.len() - DEDUP_MAX_SIZE;
        for (old_key, _) in ordered.into_iter().take(excess) {
            store.remove(&old_key);
        }
    }

    false
}
