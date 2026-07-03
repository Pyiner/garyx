use super::*;

pub(super) const PROVIDER_MODEL_DISCOVERY_SUCCESS_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone)]
pub(super) struct ProviderModelDiscoveryCacheEntry {
    fetched_at: Instant,
    discovery: ProviderModelDiscovery,
}

pub(super) fn provider_model_discovery_cache()
-> &'static Mutex<HashMap<&'static str, ProviderModelDiscoveryCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, ProviderModelDiscoveryCacheEntry>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn fresh_cached_discovery(cache_key: &'static str) -> Option<ProviderModelDiscovery> {
    let guard = provider_model_discovery_cache().lock().ok()?;
    let entry = guard.get(cache_key)?;
    (entry.fetched_at.elapsed() < PROVIDER_MODEL_DISCOVERY_SUCCESS_TTL)
        .then(|| entry.discovery.clone())
}

pub(super) fn cached_discovery(cache_key: &'static str) -> Option<ProviderModelDiscovery> {
    let guard = provider_model_discovery_cache().lock().ok()?;
    guard.get(cache_key).map(|entry| entry.discovery.clone())
}

pub(super) fn store_discovery(cache_key: &'static str, discovery: ProviderModelDiscovery) {
    if let Ok(mut guard) = provider_model_discovery_cache().lock() {
        guard.insert(
            cache_key,
            ProviderModelDiscoveryCacheEntry {
                fetched_at: Instant::now(),
                discovery,
            },
        );
    }
}

pub(super) fn discover_or_fallback(
    cache_key: &'static str,
    discover_result: Result<ProviderModelDiscovery, String>,
    fallback: impl FnOnce(String) -> ProviderModelDiscovery,
) -> ProviderModelDiscovery {
    match discover_result {
        Ok(discovery) if !discovery.models.is_empty() => {
            store_discovery(cache_key, discovery.clone());
            discovery
        }
        Ok(discovery) => {
            let error = format!("{} returned no models", discovery.source);
            stale_or_fallback(cache_key, error, fallback)
        }
        Err(error) => stale_or_fallback(cache_key, error, fallback),
    }
}

pub(super) fn stale_or_fallback(
    cache_key: &'static str,
    error: String,
    fallback: impl FnOnce(String) -> ProviderModelDiscovery,
) -> ProviderModelDiscovery {
    if let Some(mut discovery) = cached_discovery(cache_key) {
        discovery.error = Some(error);
        return discovery;
    }
    fallback(error)
}

#[cfg(test)]
pub(super) fn clear_provider_model_discovery_cache_for_tests() {
    if let Ok(mut guard) = provider_model_discovery_cache().lock() {
        guard.clear();
    }
}

pub(super) fn provider_supports_reasoning_effort_selection(models: &[ProviderModelOption]) -> bool {
    models
        .iter()
        .any(|model| !model.supported_reasoning_efforts.is_empty())
}

pub(super) fn reasoning_efforts_for_default_model(
    models: &[ProviderModelOption],
    default_model: Option<&str>,
) -> Vec<ProviderReasoningEffortOption> {
    if let Some(default_model) = default_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return models
            .iter()
            .find(|model| model.id == default_model)
            .map(|model| model.supported_reasoning_efforts.clone())
            .unwrap_or_default();
    }
    common_reasoning_efforts(models)
}

pub(super) fn configured_agent_provider_config(
    config: &GaryxConfig,
    provider_type: ProviderType,
    keys: &[&str],
) -> Option<AgentProviderConfig> {
    for key in keys {
        if let Some(value) = config.agents.get(*key)
            && let Ok(mut agent_config) =
                serde_json::from_value::<AgentProviderConfig>(value.clone())
            && ProviderType::from_slug(&agent_config.provider_type) == Some(provider_type.clone())
        {
            agent_config.provider_type = provider_type.as_slug().to_owned();
            return Some(agent_config);
        }
    }
    None
}

pub(super) fn configured_default_model(
    config: &GaryxConfig,
    provider_type: ProviderType,
    keys: &[&str],
) -> Option<String> {
    configured_agent_provider_config(config, provider_type, keys)
        .map(|config| config.default_model.trim().to_owned())
        .filter(|model| !model.is_empty())
}

pub(super) fn configured_default_reasoning_effort(
    config: &GaryxConfig,
    provider_type: ProviderType,
    keys: &[&str],
) -> Option<String> {
    configured_agent_provider_config(config, provider_type, keys)
        .map(|config| config.model_reasoning_effort.trim().to_owned())
        .filter(|effort| !effort.is_empty())
}
