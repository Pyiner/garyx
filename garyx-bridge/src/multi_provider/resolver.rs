use super::state::Inner;

/// Free function for resolution (avoids `&self` lifetime issues in spawned tasks).
pub(super) async fn resolve_provider_impl(
    inner: &Inner,
    thread_id: &str,
    channel: &str,
    account_id: &str,
) -> Option<String> {
    let affinity_key = inner.thread_affinity.read().await.get(thread_id).cloned();
    let topology = inner.topology.read().await;

    // 1. Session affinity
    if let Some(key) = affinity_key {
        if topology.provider_pool.contains_key(&key) {
            return Some(key);
        }
    }

    // 2. Route cache
    let route_key = (channel.to_owned(), account_id.to_owned());
    if let Some(key) = topology.route_cache.get(&route_key) {
        if topology.provider_pool.contains_key(key) {
            return Some(key.clone());
        }
    }

    // 3. Default
    let default = topology.default_provider_key.clone();
    if let Some(ref key) = default {
        if topology.provider_pool.contains_key(key) {
            return default;
        }
    }

    None
}
