use std::collections::HashMap;
use std::sync::Arc;

use garyx_models::provider::ProviderType;

use crate::provider_trait::{AgentLoopProvider, ProviderHealth};

use super::MultiProviderBridge;
use super::resolver::resolve_provider_impl;
use super::state::Inner;

impl MultiProviderBridge {
    /// Register a provider under the given key.
    pub async fn register_provider(
        &self,
        key: impl Into<String>,
        provider: Arc<dyn AgentLoopProvider>,
    ) {
        self.inner
            .topology
            .write()
            .await
            .provider_pool
            .insert(key.into(), provider);
    }

    /// Synchronous variant of [`register_provider`] for sync bootstrap paths
    /// (e.g. [`AppStateBuilder::build`] in `garyx-gateway`). Returns an error
    /// if the topology lock is currently held — at boot time it should not be
    /// contended, and the caller (startup) can treat lock contention as a
    /// fatal wiring bug. Mirrors the `try_write` approach used by
    /// `MultiProviderBridge::set_thread_history` for the same reason.
    pub fn register_provider_blocking(
        &self,
        key: impl Into<String>,
        provider: Arc<dyn AgentLoopProvider>,
    ) -> Result<(), &'static str> {
        let mut guard = self
            .inner
            .topology
            .try_write()
            .map_err(|_| "bridge topology busy during blocking provider registration")?;
        guard.provider_pool.insert(key.into(), provider);
        Ok(())
    }

    /// Pick any registered provider whose `provider_type()` matches `target`.
    ///
    /// Used by the AgentTeam meta-provider to route a child-thread run to the
    /// provider backing that child's configured `provider_type`. This is an
    /// O(n) scan of the provider pool; for MVP the pool is tiny (≤ a few
    /// entries) and this is called at most once per child turn.
    pub async fn pick_provider_by_type(
        &self,
        target: ProviderType,
    ) -> Option<Arc<dyn AgentLoopProvider>> {
        let topology = self.inner.topology.read().await;
        topology
            .provider_pool
            .values()
            .find(|provider| provider.provider_type() == target)
            .cloned()
    }

    /// Get a reference-counted handle to a provider by key.
    pub async fn get_provider(&self, key: &str) -> Option<Arc<dyn AgentLoopProvider>> {
        self.inner
            .topology
            .read()
            .await
            .provider_pool
            .get(key)
            .cloned()
    }

    /// Set the default provider key.
    pub async fn set_default_provider_key(&self, key: impl Into<String>) {
        self.inner.topology.write().await.default_provider_key = Some(key.into());
    }

    /// Get the default provider key.
    pub async fn default_provider_key(&self) -> Option<String> {
        self.inner
            .topology
            .read()
            .await
            .default_provider_key
            .clone()
    }

    /// List all registered provider keys.
    pub async fn provider_keys(&self) -> Vec<String> {
        self.inner
            .topology
            .read()
            .await
            .provider_pool
            .keys()
            .cloned()
            .collect()
    }

    /// Get health status for a specific provider.
    pub async fn get_provider_health(&self, provider_key: &str) -> Option<ProviderHealth> {
        self.inner
            .topology
            .read()
            .await
            .provider_health
            .get(provider_key)
            .cloned()
    }

    /// Get health status for all providers.
    pub async fn get_all_provider_health(&self) -> HashMap<String, ProviderHealth> {
        self.inner.topology.read().await.provider_health.clone()
    }

    /// Record a successful run for health tracking.
    pub(super) async fn record_health_success(inner: &Inner, provider_key: &str, latency_ms: f64) {
        let mut topology = inner.topology.write().await;
        let health = topology
            .provider_health
            .entry(provider_key.to_owned())
            .or_insert_with(|| ProviderHealth::new(provider_key));
        health.record_success(latency_ms);
    }

    /// Record a failed run for health tracking.
    pub(super) async fn record_health_failure(inner: &Inner, provider_key: &str, error: &str) {
        let mut topology = inner.topology.write().await;
        let health = topology
            .provider_health
            .entry(provider_key.to_owned())
            .or_insert_with(|| ProviderHealth::new(provider_key));
        health.record_failure(error);
    }

    /// Bind a (channel, account_id) pair to a provider key.
    pub async fn set_route(
        &self,
        channel: impl Into<String>,
        account_id: impl Into<String>,
        provider_key: impl Into<String>,
    ) {
        self.inner
            .topology
            .write()
            .await
            .route_cache
            .insert((channel.into(), account_id.into()), provider_key.into());
    }

    /// Bind a session key to a provider key.
    pub async fn set_thread_affinity(
        &self,
        thread_id: impl Into<String>,
        provider_key: impl Into<String>,
    ) {
        self.inner
            .thread_affinity
            .write()
            .await
            .insert(thread_id.into(), provider_key.into());
    }

    /// Resolve which provider key should handle a thread, using the
    /// following priority:
    ///
    /// 1. Explicit session affinity
    /// 2. Route cache `(channel, account_id)`
    /// 3. Default provider
    pub async fn resolve_provider_for_thread(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
    ) -> Option<String> {
        resolve_provider_impl(&self.inner, thread_id, channel, account_id).await
    }

    pub async fn resolve_provider_for_request(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        requested_provider: Option<ProviderType>,
    ) -> Option<String> {
        let affinity = self
            .inner
            .thread_affinity
            .read()
            .await
            .get(thread_id)
            .cloned();
        let fallback = resolve_provider_impl(&self.inner, thread_id, channel, account_id).await;
        if let Some(affinity_key) = affinity {
            if requested_provider.is_none() {
                return Some(affinity_key);
            }
            if let Some(provider) = self.get_provider(&affinity_key).await {
                if Some(provider.provider_type()) == requested_provider {
                    return Some(affinity_key);
                }
            }
        }

        let requested_type = match requested_provider {
            Some(provider_type) => provider_type,
            None => {
                let Some(candidate_key) = fallback.as_deref() else {
                    return self.select_best_provider(None, true).await;
                };
                let Some(provider) = self.get_provider(candidate_key).await else {
                    return self.select_best_provider(None, true).await;
                };
                if provider.is_ready() && self.provider_active_run_count(candidate_key).await == 0 {
                    return fallback;
                }
                return self
                    .select_best_provider(Some(provider.provider_type()), true)
                    .await
                    .or(fallback);
            }
        };

        if let Some(candidate_key) = fallback.as_deref() {
            if let Some(provider) = self.get_provider(candidate_key).await {
                if provider.provider_type() == requested_type
                    && self.provider_active_run_count(candidate_key).await == 0
                    && provider.is_ready()
                {
                    return fallback;
                }
            }
        }

        self.select_best_provider(Some(requested_type), true).await
    }

    async fn provider_active_run_count(&self, provider_key: &str) -> usize {
        self.inner
            .run_index
            .read()
            .await
            .active_runs
            .values()
            .filter(|value| value.as_str() == provider_key)
            .count()
    }

    pub(super) async fn select_best_provider(
        &self,
        requested_provider: Option<ProviderType>,
        prefer_local: bool,
    ) -> Option<String> {
        let topology = self.inner.topology.read().await;
        let active_runs = self.inner.run_index.read().await.active_runs.clone();

        let mut candidates = Vec::new();

        for (key, provider) in &topology.provider_pool {
            if requested_provider
                .as_ref()
                .is_some_and(|requested| provider.provider_type() != *requested)
            {
                continue;
            }
            if !provider.is_ready() {
                continue;
            }
            let active_count = active_runs.values().filter(|value| *value == key).count();
            candidates.push((key.clone(), active_count));
        }

        candidates.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let _ = prefer_local;
        candidates.into_iter().map(|entry| entry.0).next()
    }
}
