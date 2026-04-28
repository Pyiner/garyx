use std::collections::{HashMap, HashSet};

use crate::provider_trait::BridgeError;
use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::ProviderType;
use garyx_models::resolve_agent_reference;

use super::MultiProviderBridge;
use super::provider_factory::{compute_provider_key, create_provider};
use super::state::{BridgeRunIndex, BridgeTopologyState};

fn default_provider_config(provider_type: ProviderType) -> AgentProviderConfig {
    AgentProviderConfig {
        provider_type: match provider_type {
            ProviderType::CodexAppServer => "codex_app_server".to_owned(),
            ProviderType::ClaudeCode => "claude_code".to_owned(),
            ProviderType::GeminiCli => "gemini_cli".to_owned(),
            // Meta-provider with no backing CLI config.
            ProviderType::AgentTeam => "agent_team".to_owned(),
        },
        ..Default::default()
    }
}

impl MultiProviderBridge {
    /// Initialize all registered providers.
    pub async fn initialize(&self) -> Result<(), BridgeError> {
        let topology = self.inner.topology.read().await;
        let pool = &topology.provider_pool;
        for (key, provider) in pool.iter() {
            if !provider.is_ready() {
                tracing::warn!(provider_key = %key, "provider not ready after registration");
            }
        }
        Ok(())
    }

    /// Initialize bridge from config: create default provider, then
    /// pre-create providers for all enabled channel accounts.
    ///
    /// This mirrors the Python `MultiProviderBridge.initialize()` +
    /// `_initialize_account_providers()` flow.
    pub async fn initialize_from_config(&self, config: &GaryxConfig) -> Result<(), BridgeError> {
        self.reload_from_config(config).await
    }

    /// Reload bridge routing/provider topology from config.
    ///
    /// This is a strict reconciliation path used by runtime hot reload:
    /// stale channel routes and stale session affinities are removed so the
    /// bridge always reflects the latest config.
    pub async fn reload_from_config(&self, config: &GaryxConfig) -> Result<(), BridgeError> {
        let default_workspace = None;

        // 1. Create default Claude Code provider.
        let default_agent_cfg = default_provider_config(ProviderType::ClaudeCode);
        let default_key = self
            .get_or_create_provider(&default_agent_cfg, &default_workspace)
            .await?;
        tracing::info!(provider_key = %default_key, "registered default provider");

        let codex_default_agent_cfg = default_provider_config(ProviderType::CodexAppServer);
        let codex_default_key = match self
            .get_or_create_provider(&codex_default_agent_cfg, &default_workspace)
            .await
        {
            Ok(key) => {
                tracing::info!(provider_key = %key, "registered secondary default provider");
                Some(key)
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to register secondary default provider"
                );
                None
            }
        };

        let gemini_default_agent_cfg = default_provider_config(ProviderType::GeminiCli);
        let gemini_default_key = match self
            .get_or_create_provider(&gemini_default_agent_cfg, &default_workspace)
            .await
        {
            Ok(key) => {
                tracing::info!(provider_key = %key, "registered tertiary default provider");
                Some(key)
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to register tertiary default provider");
                None
            }
        };

        let mut desired_routes: HashMap<(String, String), String> = HashMap::new();

        // 4. Pre-create providers for API channel accounts and build desired routes.
        for (account_id, account) in &config.channels.api.accounts {
            if !account.enabled {
                continue;
            }
            let provider_key = self
                .resolve_account_provider_key(
                    "api",
                    account_id,
                    &account.agent_id,
                    &default_workspace,
                    &default_key,
                )
                .await;
            desired_routes.insert(("api".to_owned(), account_id.clone()), provider_key);
        }

        for (plugin_id, plugin_cfg) in &config.channels.plugins {
            for (account_id, account) in &plugin_cfg.accounts {
                if !account.enabled {
                    continue;
                }
                let agent_id = account.agent_id.as_deref().unwrap_or("claude");
                let provider_key = self
                    .resolve_account_provider_key(
                        plugin_id,
                        account_id,
                        agent_id,
                        &default_workspace,
                        &default_key,
                    )
                    .await;
                desired_routes.insert((plugin_id.clone(), account_id.clone()), provider_key);
            }
        }

        let mut desired_provider_keys: HashSet<String> = desired_routes.values().cloned().collect();
        desired_provider_keys.insert(default_key.clone());
        if let Some(ref key) = codex_default_key {
            desired_provider_keys.insert(key.clone());
        }
        if let Some(ref key) = gemini_default_key {
            desired_provider_keys.insert(key.clone());
        }
        // Preserve the AgentTeam meta-provider across reloads: it is not owned
        // by a channel account route, so the
        // "desired set from config" reconciliation above would otherwise drop
        // it. The AgentTeam provider is registered once at boot via
        // `AppStateBuilder::build` (see `AGENT_TEAM_PROVIDER_KEY` in
        // `garyx-gateway`) and must survive subsequent
        // `reload_from_config` calls triggered by runtime config edits, or
        // team-bound threads break with "provider not found" on the next run.
        let existing_agent_team_keys = {
            let topology = self.inner.topology.read().await;
            topology
                .provider_pool
                .iter()
                .filter(|(_, provider)| provider.provider_type() == ProviderType::AgentTeam)
                .map(|(key, _)| key.clone())
                .collect::<HashSet<_>>()
        };
        desired_provider_keys.extend(existing_agent_team_keys);

        // Keep providers backing active runs until they naturally drain.
        let run_index = self.inner.run_index.read().await;
        let active_run_provider_keys: HashSet<String> =
            run_index.active_runs.values().cloned().collect();
        drop(run_index);
        desired_provider_keys.extend(active_run_provider_keys);

        let mut topology = self.inner.topology.write().await;
        topology.default_provider_key = Some(default_key);
        topology.route_cache = desired_routes;

        topology
            .provider_pool
            .retain(|provider_key, _| desired_provider_keys.contains(provider_key));
        topology
            .provider_health
            .retain(|provider_key, _| desired_provider_keys.contains(provider_key));
        let retained_provider_keys: HashSet<String> =
            topology.provider_pool.keys().cloned().collect();
        let provider_count = topology.provider_pool.len();
        let route_count = topology.route_cache.len();
        drop(topology);

        self.inner
            .thread_affinity
            .write()
            .await
            .retain(|_, provider_key| desired_provider_keys.contains(provider_key));

        // Ensure no active-run index points to providers that no longer exist.
        let mut run_index = self.inner.run_index.write().await;
        run_index
            .active_runs
            .retain(|_, provider_key| retained_provider_keys.contains(provider_key));
        let active_run_ids: HashSet<String> = run_index.active_runs.keys().cloned().collect();
        run_index
            .run_sessions
            .retain(|run_id, _| active_run_ids.contains(run_id));
        drop(run_index);

        tracing::info!(
            provider_count,
            route_count,
            "MultiProviderBridge reconciled from config"
        );
        Ok(())
    }

    /// Create or reuse a provider for an account config, returning provider key.
    async fn resolve_account_provider_key(
        &self,
        channel: &str,
        account_id: &str,
        agent_id: &str,
        default_workspace: &Option<String>,
        default_key: &str,
    ) -> String {
        let provider_type = self
            .provider_type_for_agent(agent_id)
            .await
            .unwrap_or(ProviderType::ClaudeCode);
        let agent_cfg = default_provider_config(provider_type);

        // Compute provider key from config for dedup check.
        let key = compute_provider_key(&agent_cfg, default_workspace);

        // If already registered (e.g., same config as default), just reuse.
        if self.get_provider(&key).await.is_some() {
            tracing::info!(
                channel = channel,
                account_id = account_id,
                provider_key = %key,
                "reusing existing provider for route"
            );
            return key;
        }

        // Create new provider.
        match self
            .get_or_create_provider(&agent_cfg, default_workspace)
            .await
        {
            Ok(key) => {
                tracing::info!(
                    channel = channel,
                    account_id = account_id,
                    provider_key = %key,
                    "registered provider for route"
                );
                key
            }
            Err(e) => {
                tracing::warn!(
                    channel = channel,
                    account_id = account_id,
                    error = %e,
                    "failed to create provider, falling back to default"
                );
                default_key.to_owned()
            }
        }
    }

    /// Get or create a provider based on `AgentProviderConfig`, with dedup.
    async fn get_or_create_provider(
        &self,
        agent_cfg: &AgentProviderConfig,
        default_workspace: &Option<String>,
    ) -> Result<String, BridgeError> {
        let key = compute_provider_key(agent_cfg, default_workspace);

        // Check if already registered.
        if self.get_provider(&key).await.is_some() {
            return Ok(key);
        }

        // Create and initialize provider.
        let provider = create_provider(agent_cfg, default_workspace).await?;
        self.register_provider(&key, provider).await;
        Ok(key)
    }

    pub(crate) async fn provider_type_for_agent(&self, agent_id: &str) -> Option<ProviderType> {
        let normalized = agent_id.trim();
        if normalized.is_empty() {
            return None;
        }
        let agent_profiles = self
            .inner
            .agent_profiles
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let team_profiles = self
            .inner
            .team_profiles
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if let Ok(reference) = resolve_agent_reference(normalized, &agent_profiles, &team_profiles)
        {
            return Some(reference.provider_type());
        }
        match normalized {
            "codex" => Some(ProviderType::CodexAppServer),
            "claude" => Some(ProviderType::ClaudeCode),
            "gemini" => Some(ProviderType::GeminiCli),
            _ => None,
        }
    }

    /// Shutdown all providers and cancel active tasks.
    pub async fn shutdown(&self) {
        tracing::info!("shutting down MultiProviderBridge");

        // Cancel all active tasks.
        {
            let mut tasks = self.inner.active_tasks.lock().await;
            for (run_id, task) in tasks.drain() {
                task.abort();
                tracing::info!(run_id = %run_id, "cancelled active task");
            }
        }

        // Clear tracking state.
        *self.inner.topology.write().await = BridgeTopologyState::default();
        self.inner.thread_affinity.write().await.clear();
        self.inner.thread_workspace_bindings.write().await.clear();
        *self.inner.run_index.write().await = BridgeRunIndex::default();
        self.inner.active_thread_persistence.lock().await.clear();
    }
}
