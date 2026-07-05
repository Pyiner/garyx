use std::collections::{HashMap, HashSet};

use crate::provider_trait::{BridgeError, ProviderModelDefaults};
use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::ProviderType;
use garyx_models::{
    agent_runtime_snapshot_metadata, merge_thread_agent_runtime_snapshot, resolve_agent_reference,
};

use super::MultiProviderBridge;
use super::provider_factory::{
    agent_provider_requires_dedicated_key, compute_provider_key, create_provider,
};
use super::state::{BridgeRunIndex, BridgeTopologyState};

pub(super) fn default_provider_config(provider_type: ProviderType) -> AgentProviderConfig {
    AgentProviderConfig {
        provider_type: provider_type.as_slug().to_owned(),
        ..Default::default()
    }
}

fn configured_provider_config(
    config: &GaryxConfig,
    provider_type: ProviderType,
    keys: &[&str],
) -> Option<AgentProviderConfig> {
    for key in keys {
        if let Some(value) = config.agents.get(*key)
            && let Ok(mut agent_cfg) = serde_json::from_value::<AgentProviderConfig>(value.clone())
            && ProviderType::from_slug(&agent_cfg.provider_type) == Some(provider_type.clone())
        {
            agent_cfg.provider_type = provider_type.as_slug().to_owned();
            return Some(agent_cfg);
        }
    }
    None
}

fn configured_default_provider_config(
    config: &GaryxConfig,
    provider_type: ProviderType,
    keys: &[&str],
) -> AgentProviderConfig {
    configured_provider_config(config, provider_type.clone(), keys)
        .unwrap_or_else(|| default_provider_config(provider_type))
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

        // 1. Create default Claude Code provider. cctty/native is configured
        // inside this provider as the Agent SDK executable, not as a separate
        // provider type.
        let default_agent_cfg = configured_default_provider_config(
            config,
            ProviderType::ClaudeCode,
            &["claude", "claude_code", "claude_tty"],
        );
        let default_key = self
            .get_or_create_provider(&default_agent_cfg, &default_workspace)
            .await?;
        tracing::info!(provider_key = %default_key, "registered default provider");

        let codex_default_agent_cfg = configured_default_provider_config(
            config,
            ProviderType::CodexAppServer,
            &["codex", "codex_app_server"],
        );
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

        let gemini_default_agent_cfg = configured_default_provider_config(
            config,
            ProviderType::GeminiCli,
            &["gemini", "gemini_cli"],
        );
        let gemini_default_key = match self
            .get_or_create_provider(&gemini_default_agent_cfg, &default_workspace)
            .await
        {
            Ok(key) => {
                tracing::info!(provider_key = %key, "registered tertiary default provider");
                Some(key)
            }
            Err(error) => {
                tracing::debug!(error = %error, "optional gemini provider unavailable");
                None
            }
        };

        let traex_default_agent_cfg = configured_default_provider_config(
            config,
            ProviderType::Traex,
            &["traex", "trae", "trae_cli", "traecli"],
        );
        let traex_default_key = match self
            .get_or_create_provider(&traex_default_agent_cfg, &default_workspace)
            .await
        {
            Ok(key) => {
                tracing::info!(provider_key = %key, "registered traex default provider");
                Some(key)
            }
            Err(error) => {
                tracing::debug!(error = %error, "optional traex provider unavailable");
                None
            }
        };

        let antigravity_default_agent_cfg = configured_default_provider_config(
            config,
            ProviderType::AntigravityCli,
            &["antigravity", "agy", "antigravity_cli"],
        );
        let antigravity_default_key = match self
            .get_or_create_provider(&antigravity_default_agent_cfg, &default_workspace)
            .await
        {
            Ok(key) => {
                tracing::info!(provider_key = %key, "registered antigravity default provider");
                Some(key)
            }
            Err(error) => {
                tracing::debug!(error = %error, "optional antigravity provider unavailable");
                None
            }
        };

        let mut default_provider_configs = vec![
            default_agent_cfg.clone(),
            codex_default_agent_cfg.clone(),
            gemini_default_agent_cfg.clone(),
            traex_default_agent_cfg.clone(),
            antigravity_default_agent_cfg.clone(),
        ];
        default_provider_configs.extend(
            [
                configured_provider_config(
                    config,
                    ProviderType::Gpt,
                    &["gpt", "openai", "garyx", "garyx_native", "native"],
                ),
                configured_provider_config(
                    config,
                    ProviderType::ClaudeLlm,
                    &["anthropic", "claude_llm"],
                ),
                configured_provider_config(
                    config,
                    ProviderType::GeminiLlm,
                    &["google", "gemini_llm"],
                ),
            ]
            .into_iter()
            .flatten(),
        );
        self.replace_default_provider_configs(default_provider_configs)
            .await;

        let configured_agent_provider_keys = self
            .register_configured_agent_providers(&default_workspace)
            .await;

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
        if let Some(ref key) = traex_default_key {
            desired_provider_keys.insert(key.clone());
        }
        if let Some(ref key) = antigravity_default_key {
            desired_provider_keys.insert(key.clone());
        }
        desired_provider_keys.extend(configured_agent_provider_keys);
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
        let agent_cfg = self
            .provider_config_for_agent(agent_id)
            .await
            .unwrap_or_else(|| default_provider_config(ProviderType::ClaudeCode));

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
    pub(super) async fn get_or_create_provider(
        &self,
        agent_cfg: &AgentProviderConfig,
        default_workspace: &Option<String>,
    ) -> Result<String, BridgeError> {
        let key = compute_provider_key(agent_cfg, default_workspace);

        // Already registered: hot-apply the (possibly reloaded) model
        // defaults onto the live instance. The provider key intentionally
        // excludes model defaults so thread affinity and SDK sessions stay
        // stable; a config edit must therefore reconcile the existing
        // instance instead of being silently dropped. Active runs are
        // untouched — only default resolution for future runs changes.
        if let Some(provider) = self.get_provider(&key).await {
            provider.update_model_defaults(&ProviderModelDefaults::from(agent_cfg));
            return Ok(key);
        }

        // Create and initialize provider.
        let provider = create_provider(agent_cfg, default_workspace).await?;
        self.register_provider(&key, provider).await;
        Ok(key)
    }

    pub(super) async fn provider_config_for_agent(
        &self,
        agent_id: &str,
    ) -> Option<AgentProviderConfig> {
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
            if let garyx_models::AgentReference::Standalone { profile, .. } = reference {
                if !profile.built_in {
                    return Some(profile.to_provider_config());
                }
                return Some(
                    self.default_provider_config_for_type(profile.provider_type)
                        .await,
                );
            }
            return Some(default_provider_config(ProviderType::AgentTeam));
        }
        match normalized {
            "codex" => Some(
                self.default_provider_config_for_type(ProviderType::CodexAppServer)
                    .await,
            ),
            "traex" | "trae" | "trae_cli" | "traecli" => Some(
                self.default_provider_config_for_type(ProviderType::Traex)
                    .await,
            ),
            "claude" | "claude-tty" | "claude_tty" => Some(
                self.default_provider_config_for_type(ProviderType::ClaudeCode)
                    .await,
            ),
            "gemini" => Some(
                self.default_provider_config_for_type(ProviderType::GeminiCli)
                    .await,
            ),
            "antigravity" | "agy" | "antigravity_cli" => Some(
                self.default_provider_config_for_type(ProviderType::AntigravityCli)
                    .await,
            ),
            "gpt" | "openai" | "garyx" | "garyx_native" | "native" => {
                self.configured_provider_config_for_type(ProviderType::Gpt)
                    .await
            }
            "anthropic" | "claude_llm" => {
                self.configured_provider_config_for_type(ProviderType::ClaudeLlm)
                    .await
            }
            "google" | "gemini_llm" => {
                self.configured_provider_config_for_type(ProviderType::GeminiLlm)
                    .await
            }
            _ => None,
        }
    }

    async fn replace_default_provider_configs(
        &self,
        configs: impl IntoIterator<Item = AgentProviderConfig>,
    ) {
        let mut default_provider_configs = self.inner.default_provider_configs.write().await;
        default_provider_configs.clear();
        for config in configs {
            if let Some(provider_type) = ProviderType::from_slug(&config.provider_type) {
                default_provider_configs.insert(provider_type, config);
            }
        }
    }

    async fn default_provider_config_for_type(
        &self,
        provider_type: ProviderType,
    ) -> AgentProviderConfig {
        self.inner
            .default_provider_configs
            .read()
            .await
            .get(&provider_type)
            .cloned()
            .unwrap_or_else(|| default_provider_config(provider_type))
    }

    async fn configured_provider_config_for_type(
        &self,
        provider_type: ProviderType,
    ) -> Option<AgentProviderConfig> {
        self.inner
            .default_provider_configs
            .read()
            .await
            .get(&provider_type)
            .cloned()
    }

    /// Backfill run metadata with the thread's runtime configuration: the
    /// thread's model cells first (single-cell semantics; legacy `*_override`
    /// keys coalesce in front until migrated), then the bound agent's profile
    /// (model, effort, tier, system prompt, identity). Shared providers only
    /// see per-run metadata, so a dispatch that carries no such fields would
    /// otherwise run the provider's defaults. Existing metadata values always
    /// win, giving the precedence: explicit request > thread cell (legacy
    /// override first) > agent profile default.
    pub(super) async fn backfill_bound_agent_runtime_metadata(
        &self,
        thread_id: &str,
        metadata: &mut HashMap<String, serde_json::Value>,
    ) {
        use serde_json::Value;

        let thread_store = self.inner.thread_store.read().await.clone();
        let mut thread_record = match thread_store.as_ref() {
            Some(store) => store.get(thread_id).await,
            None => None,
        };
        if let Some(record) = thread_record.as_ref() {
            garyx_models::provider::merge_thread_model_cells(record, metadata);
            merge_thread_agent_runtime_snapshot(record, metadata);
        }

        let metadata_agent_id = metadata
            .get("agent_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let agent_id = match metadata_agent_id {
            Some(value) => Some(value),
            None => thread_record.as_ref().and_then(|record| {
                record
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            }),
        };
        let Some(agent_id) = agent_id else {
            return;
        };
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
        let Ok(reference) = resolve_agent_reference(&agent_id, &agent_profiles, &team_profiles)
        else {
            return;
        };
        let snapshot = agent_runtime_snapshot_metadata(&reference);
        for (key, value) in &snapshot {
            metadata.entry(key.clone()).or_insert_with(|| value.clone());
        }
        if let (Some(store), Some(record)) = (thread_store, thread_record.as_mut()) {
            let changed = {
                let Some(obj) = record.as_object_mut() else {
                    return;
                };
                let metadata_value = obj
                    .entry("metadata".to_owned())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !metadata_value.is_object() {
                    *metadata_value = Value::Object(serde_json::Map::new());
                }
                let Some(thread_metadata) = metadata_value.as_object_mut() else {
                    return;
                };
                let mut changed = false;
                for (key, value) in snapshot {
                    if !thread_metadata.contains_key(&key) {
                        thread_metadata.insert(key, value);
                        changed = true;
                    }
                }
                changed
            };
            if changed {
                if let Some(obj) = record.as_object_mut() {
                    obj.insert(
                        "updated_at".to_owned(),
                        Value::String(chrono::Utc::now().to_rfc3339()),
                    );
                }
                store.set(thread_id, record.clone()).await;
            }
        }
    }

    pub(super) async fn provider_key_for_agent_id(
        &self,
        agent_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        let Some(agent_cfg) = self.provider_config_for_agent(agent_id).await else {
            return Ok(None);
        };
        let Some(provider_type) = ProviderType::from_slug(&agent_cfg.provider_type) else {
            return Ok(None);
        };
        if matches!(provider_type, ProviderType::AgentTeam)
            || !agent_provider_requires_dedicated_key(&agent_cfg)
        {
            return Ok(None);
        }
        let default_workspace = None;
        self.get_or_create_provider(&agent_cfg, &default_workspace)
            .await
            .map(Some)
    }

    async fn register_configured_agent_providers(
        &self,
        default_workspace: &Option<String>,
    ) -> HashSet<String> {
        let profiles = self
            .inner
            .agent_profiles
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut keys = HashSet::new();
        for profile in profiles {
            if profile.built_in {
                continue;
            }
            let agent_cfg = profile.to_provider_config();
            if !agent_provider_requires_dedicated_key(&agent_cfg) {
                continue;
            }
            match self
                .get_or_create_provider(&agent_cfg, default_workspace)
                .await
            {
                Ok(key) => {
                    tracing::info!(
                        agent_id = %profile.agent_id,
                        provider_key = %key,
                        "registered configured agent provider"
                    );
                    keys.insert(key);
                }
                Err(error) => {
                    tracing::warn!(
                        agent_id = %profile.agent_id,
                        error = %error,
                        "failed to register configured agent provider"
                    );
                }
            }
        }
        keys
    }

    #[cfg_attr(not(test), allow(dead_code))]
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
            "traex" | "trae" | "trae_cli" | "traecli" => Some(ProviderType::Traex),
            "claude" | "claude-tty" | "claude_tty" => Some(ProviderType::ClaudeCode),
            "gemini" => Some(ProviderType::GeminiCli),
            "antigravity" | "agy" | "antigravity_cli" => Some(ProviderType::AntigravityCli),
            "gpt" | "openai" | "garyx" | "garyx_native" | "native" => self
                .configured_provider_config_for_type(ProviderType::Gpt)
                .await
                .map(|_| ProviderType::Gpt),
            "anthropic" | "claude_llm" => self
                .configured_provider_config_for_type(ProviderType::ClaudeLlm)
                .await
                .map(|_| ProviderType::ClaudeLlm),
            "google" | "gemini_llm" => self
                .configured_provider_config_for_type(ProviderType::GeminiLlm)
                .await
                .map(|_| ProviderType::GeminiLlm),
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
