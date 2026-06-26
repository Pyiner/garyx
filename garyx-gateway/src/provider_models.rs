use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use garyx_models::codex_models::{
    CODEX_MODELS_CLIENT_VERSION_FLOOR, CodexModelPreset, CodexModelServiceTier,
    CodexModelsResponse, CodexReasoningEffort, CodexReasoningEffortPreset,
    available_codex_model_presets, codex_builtin_model_presets, models_endpoint,
    parse_codex_cli_version, resolve_codex_auth,
};
use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::{GaryxNativeConfig, ProviderType};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

const GEMINI_ACP_MODEL_METHODS: &[&str] = &[
    "model/list",
    "models/list",
    "model/list_models",
    "models/list_models",
    "session/list_models",
    "session/models",
    "list_models",
];
const CLAUDE_MODELS_BASE_URL: &str = "https://api.anthropic.com";
const CLAUDE_MODELS_TIMEOUT: Duration = Duration::from_secs(5);
const PROVIDER_MODEL_DISCOVERY_SUCCESS_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderModelOption {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recommended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_reasoning_efforts: Vec<ProviderReasoningEffortOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_tiers: Vec<ProviderModelOption>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderReasoningEffortOption {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderModelsResponse {
    pub provider_type: ProviderType,
    pub supports_model_selection: bool,
    pub models: Vec<ProviderModelOption>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub supports_reasoning_effort_selection: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_efforts: Vec<ProviderReasoningEffortOption>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub supports_service_tier_selection: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_tiers: Vec<ProviderModelOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderCatalogDefault {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
}

struct GeminiModelDiscovery {
    models: Vec<ProviderModelOption>,
    default_model: Option<String>,
}

#[derive(Debug, Clone)]
struct ProviderModelDiscovery {
    models: Vec<ProviderModelOption>,
    default_model: Option<String>,
    reasoning_efforts: Vec<ProviderReasoningEffortOption>,
    service_tiers: Vec<ProviderModelOption>,
    source: &'static str,
    error: Option<String>,
}

type GptModelDiscovery = ProviderModelDiscovery;

pub(crate) async fn list_provider_models(
    config: &GaryxConfig,
    provider_type: ProviderType,
) -> ProviderModelsResponse {
    match provider_type {
        ProviderType::AntigravityCli => {
            let aliases = &["antigravity", "agy", "antigravity_cli"];
            let default_model =
                configured_default_model(config, ProviderType::AntigravityCli, aliases)
                    .unwrap_or_else(garyx_models::provider::default_antigravity_model);
            native_model_catalog_response(
                provider_type,
                "antigravity_builtin",
                antigravity_models(),
                &default_model,
                configured_default_reasoning_effort(config, ProviderType::AntigravityCli, aliases),
            )
        }
        ProviderType::GeminiCli => {
            let aliases = &["gemini", "gemini_cli"];
            let configured_default =
                configured_default_model(config, ProviderType::GeminiCli, aliases);
            let configured_default_reasoning_effort =
                configured_default_reasoning_effort(config, ProviderType::GeminiCli, aliases);
            let discovery = match fresh_cached_discovery("gemini_cli") {
                Some(discovery) => discovery,
                None => {
                    let result = fetch_gemini_acp_models(config).await.map(|discovery| {
                        ProviderModelDiscovery {
                            models: discovery.models,
                            default_model: discovery.default_model,
                            reasoning_efforts: Vec::new(),
                            service_tiers: Vec::new(),
                            source: "gemini_acp",
                            error: None,
                        }
                    });
                    discover_or_fallback("gemini_cli", result, gemini_cli_preset_models)
                }
            };
            let supports_reasoning_effort_selection =
                provider_supports_reasoning_effort_selection(&discovery.models);
            ProviderModelsResponse {
                provider_type,
                supports_model_selection: !discovery.models.is_empty(),
                supports_reasoning_effort_selection,
                reasoning_efforts: discovery.reasoning_efforts,
                models: discovery.models,
                supports_service_tier_selection: !discovery.service_tiers.is_empty(),
                service_tiers: discovery.service_tiers,
                default_model: configured_default.or(discovery.default_model),
                default_reasoning_effort: configured_default_reasoning_effort,
                source: discovery.source,
                error: discovery.error,
            }
        }
        ProviderType::ClaudeCode => {
            // The CLI's actual default model is account/plan dependent and not
            // statically knowable unless the gateway config pins one. Without
            // a chosen model, only the levels every model supports are offered.
            let aliases = &["claude", "claude_code", "claude_tty"];
            let default_model = configured_default_model(config, ProviderType::ClaudeCode, aliases);
            let default_reasoning_effort =
                configured_default_reasoning_effort(config, ProviderType::ClaudeCode, aliases);
            let mut discovery = match fresh_cached_discovery("claude_code") {
                Some(discovery) => discovery,
                None => {
                    let result = fetch_claude_code_models().await;
                    discover_or_fallback("claude_code", result, |error| {
                        claude_code_builtin_models(Some(error))
                    })
                }
            };
            discovery.reasoning_efforts =
                reasoning_efforts_for_default_model(&discovery.models, default_model.as_deref());
            let supports_reasoning_effort_selection =
                provider_supports_reasoning_effort_selection(&discovery.models);
            ProviderModelsResponse {
                provider_type,
                supports_model_selection: true,
                supports_reasoning_effort_selection,
                reasoning_efforts: discovery.reasoning_efforts,
                models: discovery.models,
                supports_service_tier_selection: false,
                service_tiers: Vec::new(),
                default_model,
                default_reasoning_effort,
                source: discovery.source,
                error: discovery.error,
            }
        }
        ProviderType::CodexAppServer | ProviderType::Traex => {
            let aliases: &[&str] = if provider_type == ProviderType::Traex {
                &["traex", "trae", "trae_cli", "traecli"]
            } else {
                &["codex", "codex_app_server"]
            };
            // Discover models dynamically from the app-server's `model/list`
            // (reflects the real backend catalog); fall back to the static
            // preset list if the binary is unavailable or discovery fails.
            let source: &'static str = if provider_type == ProviderType::Traex {
                "traex_app_server"
            } else {
                "codex_app_server"
            };
            let configured_default_reasoning_effort =
                configured_default_reasoning_effort(config, provider_type.clone(), aliases);
            let bin = app_server_model_bin(&provider_type);
            let cache_key = if provider_type == ProviderType::Traex {
                "traex"
            } else {
                "codex_app_server"
            };
            let mut discovery = match fresh_cached_discovery(cache_key) {
                Some(discovery) => discovery,
                None => {
                    let result = fetch_app_server_models(bin, source).await;
                    if provider_type == ProviderType::Traex {
                        discover_or_fallback(cache_key, result, traex_unavailable_models)
                    } else {
                        discover_or_fallback(cache_key, result, |error| {
                            gpt_builtin_models(Some(error))
                        })
                    }
                }
            };
            if let Some(default_model) =
                configured_default_model(config, provider_type.clone(), aliases)
            {
                discovery = apply_default_model_to_gpt_discovery(discovery, Some(default_model));
            } else if discovery.source == "codex_builtin" {
                // Builtin presets have no meaningful default; dynamic discovery
                // keeps the backend-reported default.
                discovery.default_model = None;
            }
            let supports_reasoning_effort_selection =
                provider_supports_reasoning_effort_selection(&discovery.models);
            ProviderModelsResponse {
                provider_type,
                supports_model_selection: !discovery.models.is_empty(),
                models: discovery.models,
                // Derive from the full discovered catalog: some providers expose
                // a default model with no effort controls while another model
                // does support them.
                supports_reasoning_effort_selection,
                reasoning_efforts: discovery.reasoning_efforts,
                supports_service_tier_selection: !discovery.service_tiers.is_empty(),
                service_tiers: discovery.service_tiers,
                default_model: discovery.default_model,
                default_reasoning_effort: configured_default_reasoning_effort,
                source: discovery.source,
                error: discovery.error,
            }
        }
        ProviderType::Gpt => match fetch_gpt_codex_models(config).await {
            Ok(discovery) if !discovery.models.is_empty() => {
                let aliases = &["gpt", "openai", "garyx", "garyx_native", "native"];
                let discovery = apply_default_model_to_gpt_discovery(
                    discovery,
                    configured_default_model(config, ProviderType::Gpt, aliases),
                );
                let supports_reasoning_effort_selection =
                    provider_supports_reasoning_effort_selection(&discovery.models);
                ProviderModelsResponse {
                    provider_type,
                    supports_model_selection: true,
                    models: discovery.models,
                    supports_reasoning_effort_selection,
                    reasoning_efforts: discovery.reasoning_efforts,
                    supports_service_tier_selection: !discovery.service_tiers.is_empty(),
                    service_tiers: discovery.service_tiers,
                    default_model: discovery.default_model,
                    default_reasoning_effort: configured_default_reasoning_effort(
                        config,
                        ProviderType::Gpt,
                        aliases,
                    ),
                    source: discovery.source,
                    error: discovery.error,
                }
            }
            Ok(discovery) => unsupported(
                provider_type,
                discovery.source,
                Some("Codex model catalog returned no picker-visible models".to_owned()),
            ),
            Err(error) => {
                let aliases = &["gpt", "openai", "garyx", "garyx_native", "native"];
                let discovery = apply_default_model_to_gpt_discovery(
                    gpt_builtin_models(Some(error)),
                    configured_default_model(config, ProviderType::Gpt, aliases),
                );
                let supports_reasoning_effort_selection =
                    provider_supports_reasoning_effort_selection(&discovery.models);
                ProviderModelsResponse {
                    provider_type,
                    supports_model_selection: true,
                    models: discovery.models,
                    supports_reasoning_effort_selection,
                    reasoning_efforts: discovery.reasoning_efforts,
                    supports_service_tier_selection: !discovery.service_tiers.is_empty(),
                    service_tiers: discovery.service_tiers,
                    default_model: discovery.default_model,
                    default_reasoning_effort: configured_default_reasoning_effort(
                        config,
                        ProviderType::Gpt,
                        aliases,
                    ),
                    source: discovery.source,
                    error: discovery.error,
                }
            }
        },
        ProviderType::ClaudeLlm => {
            let aliases = &["anthropic", "claude_llm"];
            let default_model = configured_default_model(config, ProviderType::ClaudeLlm, aliases)
                .unwrap_or_else(|| "claude-sonnet-4-6".to_owned());
            native_model_catalog_response(
                provider_type,
                "native_builtin",
                native_claude_models(),
                &default_model,
                configured_default_reasoning_effort(config, ProviderType::ClaudeLlm, aliases),
            )
        }
        ProviderType::GeminiLlm => {
            let aliases = &["google", "gemini_llm"];
            let default_model = configured_default_model(config, ProviderType::GeminiLlm, aliases)
                .unwrap_or_else(|| "gemini-3-flash-preview".to_owned());
            native_model_catalog_response(
                provider_type,
                "native_builtin",
                native_gemini_models(),
                &default_model,
                configured_default_reasoning_effort(config, ProviderType::GeminiLlm, aliases),
            )
        }
        ProviderType::AgentTeam => unsupported(provider_type, "provider", None),
    }
}

pub(crate) fn builtin_provider_catalog_default(
    provider_type: ProviderType,
) -> ProviderCatalogDefault {
    match provider_type {
        ProviderType::Gpt => {
            let discovery = gpt_builtin_models(None);
            ProviderCatalogDefault {
                model: discovery.default_model,
                reasoning_effort: recommended_reasoning_effort(&discovery.reasoning_efforts),
                service_tier: recommended_model_option(&discovery.service_tiers),
            }
        }
        ProviderType::ClaudeLlm => {
            let response = native_model_catalog_response(
                ProviderType::ClaudeLlm,
                "native_builtin",
                native_claude_models(),
                "claude-sonnet-4-6",
                None,
            );
            ProviderCatalogDefault {
                model: response.default_model,
                reasoning_effort: recommended_reasoning_effort(&response.reasoning_efforts),
                service_tier: recommended_model_option(&response.service_tiers),
            }
        }
        ProviderType::GeminiLlm => {
            let response = native_model_catalog_response(
                ProviderType::GeminiLlm,
                "native_builtin",
                native_gemini_models(),
                "gemini-3-flash-preview",
                None,
            );
            ProviderCatalogDefault {
                model: response.default_model,
                reasoning_effort: recommended_reasoning_effort(&response.reasoning_efforts),
                service_tier: recommended_model_option(&response.service_tiers),
            }
        }
        ProviderType::AntigravityCli => ProviderCatalogDefault {
            model: Some(garyx_models::provider::default_antigravity_model()),
            reasoning_effort: None,
            service_tier: None,
        },
        ProviderType::ClaudeCode
        | ProviderType::CodexAppServer
        | ProviderType::Traex
        | ProviderType::GeminiCli
        | ProviderType::AgentTeam => ProviderCatalogDefault::default(),
    }
}

fn recommended_reasoning_effort(options: &[ProviderReasoningEffortOption]) -> Option<String> {
    options
        .iter()
        .find(|option| option.recommended)
        .or_else(|| options.first())
        .map(|option| option.id.clone())
}

fn recommended_model_option(options: &[ProviderModelOption]) -> Option<String> {
    options
        .iter()
        .find(|option| option.recommended)
        .or_else(|| options.first())
        .map(|option| option.id.clone())
}

fn unsupported(
    provider_type: ProviderType,
    source: &'static str,
    error: Option<String>,
) -> ProviderModelsResponse {
    ProviderModelsResponse {
        provider_type,
        supports_model_selection: false,
        models: Vec::new(),
        supports_reasoning_effort_selection: false,
        reasoning_efforts: Vec::new(),
        supports_service_tier_selection: false,
        service_tiers: Vec::new(),
        default_model: None,
        default_reasoning_effort: None,
        source,
        error,
    }
}

#[derive(Debug, Clone)]
struct ProviderModelDiscoveryCacheEntry {
    fetched_at: Instant,
    discovery: ProviderModelDiscovery,
}

fn provider_model_discovery_cache()
-> &'static Mutex<HashMap<&'static str, ProviderModelDiscoveryCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, ProviderModelDiscoveryCacheEntry>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fresh_cached_discovery(cache_key: &'static str) -> Option<ProviderModelDiscovery> {
    let guard = provider_model_discovery_cache().lock().ok()?;
    let entry = guard.get(cache_key)?;
    (entry.fetched_at.elapsed() < PROVIDER_MODEL_DISCOVERY_SUCCESS_TTL)
        .then(|| entry.discovery.clone())
}

fn cached_discovery(cache_key: &'static str) -> Option<ProviderModelDiscovery> {
    let guard = provider_model_discovery_cache().lock().ok()?;
    guard.get(cache_key).map(|entry| entry.discovery.clone())
}

fn store_discovery(cache_key: &'static str, discovery: ProviderModelDiscovery) {
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

fn discover_or_fallback(
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

fn stale_or_fallback(
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
fn clear_provider_model_discovery_cache_for_tests() {
    if let Ok(mut guard) = provider_model_discovery_cache().lock() {
        guard.clear();
    }
}

fn provider_supports_reasoning_effort_selection(models: &[ProviderModelOption]) -> bool {
    models
        .iter()
        .any(|model| !model.supported_reasoning_efforts.is_empty())
}

fn reasoning_efforts_for_default_model(
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

fn configured_agent_provider_config(
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

fn configured_default_model(
    config: &GaryxConfig,
    provider_type: ProviderType,
    keys: &[&str],
) -> Option<String> {
    configured_agent_provider_config(config, provider_type, keys)
        .map(|config| config.default_model.trim().to_owned())
        .filter(|model| !model.is_empty())
}

fn configured_default_reasoning_effort(
    config: &GaryxConfig,
    provider_type: ProviderType,
    keys: &[&str],
) -> Option<String> {
    configured_agent_provider_config(config, provider_type, keys)
        .map(|config| config.model_reasoning_effort.trim().to_owned())
        .filter(|effort| !effort.is_empty())
}

async fn fetch_claude_code_models() -> Result<ProviderModelDiscovery, String> {
    #[cfg(test)]
    if std::env::var_os("GARYX_ALLOW_REAL_CLAUDE_MODEL_FETCH").is_none() {
        return Err("Claude model catalog fetch disabled in tests".to_owned());
    }

    let token = crate::claude_oauth::read_oauth_token().await?;
    fetch_claude_code_models_from_endpoint(CLAUDE_MODELS_BASE_URL, &token, CLAUDE_MODELS_TIMEOUT)
        .await
}

async fn fetch_claude_code_models_from_endpoint(
    base_url: &str,
    token: &str,
    request_timeout: Duration,
) -> Result<ProviderModelDiscovery, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Claude OAuth token was empty".to_owned());
    }
    let client = reqwest::Client::builder()
        .timeout(request_timeout)
        .build()
        .map_err(|error| format!("failed to build Claude model catalog HTTP client: {error}"))?;
    let endpoint = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let response = timeout(
        request_timeout,
        client
            .get(endpoint)
            .bearer_auth(token)
            .header(
                "anthropic-version",
                crate::claude_oauth::CLAUDE_ANTHROPIC_VERSION,
            )
            .header("anthropic-beta", crate::claude_oauth::CLAUDE_OAUTH_BETA)
            .header(
                reqwest::header::USER_AGENT,
                crate::claude_oauth::CLAUDE_USER_AGENT,
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .send(),
    )
    .await
    .map_err(|_| "Claude model catalog request timed out".to_owned())?
    .map_err(|error| {
        if error.is_timeout() {
            "Claude model catalog request timed out".to_owned()
        } else {
            format!("Claude model catalog request failed: {error}")
        }
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Claude model catalog request returned HTTP {status}"
        ));
    }
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Claude model catalog response was invalid: {error}"))?;
    Ok(parse_claude_code_models_response(&value))
}

#[derive(Debug)]
struct ClaudeApiModelOption {
    index: usize,
    created_at: Option<String>,
    model: ProviderModelOption,
}

fn parse_claude_code_models_response(value: &Value) -> ProviderModelDiscovery {
    let entries = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(id) = entry
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let label = entry
            .get("display_name")
            .or_else(|| entry.get("displayName"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| friendly_model_label(id));
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let created_at = entry
            .get("created_at")
            .or_else(|| entry.get("createdAt"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        models.push(ClaudeApiModelOption {
            index,
            created_at,
            model: ProviderModelOption {
                id: id.to_owned(),
                label,
                description,
                recommended: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: parse_claude_code_model_reasoning_efforts(&entry),
                service_tiers: Vec::new(),
            },
        });
    }
    models.sort_by(compare_claude_api_models);
    let models = models
        .into_iter()
        .map(|entry| entry.model)
        .collect::<Vec<_>>();
    let reasoning_efforts = common_reasoning_efforts(&models);
    ProviderModelDiscovery {
        models,
        default_model: None,
        reasoning_efforts,
        service_tiers: Vec::new(),
        source: "claude_code_api",
        error: None,
    }
}

fn compare_claude_api_models(
    left: &ClaudeApiModelOption,
    right: &ClaudeApiModelOption,
) -> Ordering {
    match (&left.created_at, &right.created_at) {
        (Some(left_created), Some(right_created)) => right_created
            .cmp(left_created)
            .then_with(|| left.model.id.cmp(&right.model.id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left
            .index
            .cmp(&right.index)
            .then_with(|| left.model.id.cmp(&right.model.id)),
    }
}

fn parse_claude_code_model_reasoning_efforts(entry: &Value) -> Vec<ProviderReasoningEffortOption> {
    let Some(effort) = entry
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("effort"))
    else {
        return Vec::new();
    };
    if effort.get("supported").and_then(Value::as_bool) != Some(true) {
        return Vec::new();
    }
    ["low", "medium", "high", "xhigh", "max"]
        .iter()
        .filter_map(|id| {
            let supported = effort
                .get(*id)
                .and_then(|value| value.get("supported"))
                .and_then(Value::as_bool)
                == Some(true);
            if !supported {
                return None;
            }
            native_reasoning_effort_metadata(id).map(|(id, label, description)| {
                ProviderReasoningEffortOption {
                    id: id.to_owned(),
                    label: label.to_owned(),
                    description: Some(description.to_owned()),
                    recommended: false,
                }
            })
        })
        .collect()
}

fn friendly_model_label(id: &str) -> String {
    let without_prefix = id.strip_prefix("models/").unwrap_or(id);
    let mut parts = without_prefix
        .split(['-', '_'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts
        .last()
        .is_some_and(|part| part.len() >= 6 && part.chars().all(|ch| ch.is_ascii_digit()))
    {
        parts.pop();
    }
    let label = parts
        .into_iter()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    format!(
                        "{}{}",
                        first.to_ascii_uppercase(),
                        chars.as_str().to_ascii_lowercase()
                    )
                }
                None => String::new(),
            }
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        id.to_owned()
    } else {
        label
    }
}

async fn fetch_gpt_codex_models(config: &GaryxConfig) -> Result<GptModelDiscovery, String> {
    #[cfg(test)]
    if std::env::var_os("GARYX_ALLOW_REAL_CODEX_MODEL_FETCH").is_none() {
        return Err("Codex model catalog fetch disabled in tests".to_owned());
    }

    let gpt_config = configured_gpt_config(config);
    let auth =
        resolve_codex_auth(&gpt_config, &gpt_config.env).map_err(|error| error.to_string())?;
    let client_version = resolve_codex_models_client_version().await;
    let response = reqwest::Client::new()
        .get(models_endpoint(&auth.base_url, &client_version))
        .bearer_auth(&auth.bearer_token)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::USER_AGENT,
            format!("garyx-provider-models/{}", env!("CARGO_PKG_VERSION")),
        );
    let response = if let Some(account_id) = auth.account_id.as_deref()
        && !account_id.trim().is_empty()
    {
        response.header("ChatGPT-Account-ID", account_id)
    } else {
        response
    };
    let response = timeout(Duration::from_secs(10), response.send())
        .await
        .map_err(|_| "Codex model catalog request timed out".to_owned())?
        .map_err(|error| format!("Codex model catalog request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Codex model catalog request failed with {status}: {body}"
        ));
    }
    let catalog = response
        .json::<CodexModelsResponse>()
        .await
        .map_err(|error| format!("Codex model catalog response was invalid: {error}"))?;
    let presets = available_codex_model_presets(catalog.models, auth.uses_codex_backend());
    Ok(gpt_discovery_from_presets(presets, "codex_models", None))
}

fn gpt_builtin_models(error: Option<String>) -> GptModelDiscovery {
    gpt_discovery_from_presets(codex_builtin_model_presets(), "codex_builtin", error)
}

fn claude_code_builtin_models(error: Option<String>) -> ProviderModelDiscovery {
    let models = claude_code_models();
    let reasoning_efforts = common_reasoning_efforts(&models);
    ProviderModelDiscovery {
        models,
        default_model: None,
        reasoning_efforts,
        service_tiers: Vec::new(),
        source: "claude_code_builtin",
        error,
    }
}

fn gemini_cli_preset_models(error: String) -> ProviderModelDiscovery {
    let models = vec![
        ProviderModelOption {
            id: "gemini-3-flash-preview".to_owned(),
            label: "Gemini 3 Flash Preview".to_owned(),
            description: Some("Default Gemini CLI model fallback.".to_owned()),
            recommended: true,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "gemini-2.5-pro".to_owned(),
            label: "Gemini 2.5 Pro".to_owned(),
            description: Some("Stable pro Gemini CLI model fallback.".to_owned()),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "gemini-2.5-flash".to_owned(),
            label: "Gemini 2.5 Flash".to_owned(),
            description: Some("Lower-latency Gemini CLI model fallback.".to_owned()),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
    ];
    ProviderModelDiscovery {
        models,
        default_model: Some("gemini-3-flash-preview".to_owned()),
        reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
        source: "gemini_cli_builtin",
        error: Some(error),
    }
}

fn traex_unavailable_models(error: String) -> ProviderModelDiscovery {
    ProviderModelDiscovery {
        models: Vec::new(),
        default_model: None,
        reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
        source: "traex_app_server",
        error: Some(error),
    }
}

fn apply_default_model_to_gpt_discovery(
    mut discovery: GptModelDiscovery,
    default_model: Option<String>,
) -> GptModelDiscovery {
    let Some(default_model) = default_model else {
        return discovery;
    };
    let trimmed = default_model.trim();
    if trimmed.is_empty() {
        return discovery;
    }
    discovery.default_model = Some(trimmed.to_owned());
    if let Some(model) = discovery.models.iter().find(|model| model.id == trimmed) {
        discovery.reasoning_efforts = model.supported_reasoning_efforts.clone();
        discovery.service_tiers = model.service_tiers.clone();
    } else {
        // A configured model can be newer than the built-in catalog. Avoid
        // showing reasoning or tier options that belong to the previous default.
        discovery.reasoning_efforts.clear();
        discovery.service_tiers.clear();
    }
    discovery
}

fn gpt_discovery_from_presets(
    presets: Vec<CodexModelPreset>,
    source: &'static str,
    error: Option<String>,
) -> GptModelDiscovery {
    let default_model = presets
        .iter()
        .find(|preset| preset.is_default)
        .or_else(|| presets.iter().find(|preset| preset.show_in_picker));
    let reasoning_efforts = default_model
        .map(|preset| {
            provider_reasoning_effort_options(
                &preset.supported_reasoning_efforts,
                preset.default_reasoning_effort,
            )
        })
        .unwrap_or_default();
    let service_tiers = default_model
        .map(|preset| provider_service_tier_options(&preset.service_tiers))
        .unwrap_or_default();
    let default_model = default_model.map(|preset| preset.model.clone());
    let models = presets
        .into_iter()
        .filter(|preset| preset.show_in_picker)
        .map(provider_model_option_from_codex_preset)
        .collect();

    GptModelDiscovery {
        models,
        default_model,
        reasoning_efforts,
        service_tiers,
        source,
        error,
    }
}

fn provider_model_option_from_codex_preset(preset: CodexModelPreset) -> ProviderModelOption {
    ProviderModelOption {
        id: preset.model.clone(),
        label: preset.display_name,
        description: (!preset.description.trim().is_empty()).then_some(preset.description),
        recommended: preset.is_default,
        default_reasoning_effort: Some(preset.default_reasoning_effort.to_string()),
        supported_reasoning_efforts: provider_reasoning_effort_options(
            &preset.supported_reasoning_efforts,
            preset.default_reasoning_effort,
        ),
        service_tiers: provider_service_tier_options(&preset.service_tiers),
    }
}

fn provider_reasoning_effort_options(
    presets: &[CodexReasoningEffortPreset],
    default_effort: CodexReasoningEffort,
) -> Vec<ProviderReasoningEffortOption> {
    presets
        .iter()
        .map(|preset| ProviderReasoningEffortOption {
            id: preset.effort.to_string(),
            label: preset.effort.label().to_owned(),
            description: (!preset.description.trim().is_empty())
                .then(|| preset.description.clone()),
            recommended: preset.effort == default_effort,
        })
        .collect()
}

fn provider_service_tier_options(tiers: &[CodexModelServiceTier]) -> Vec<ProviderModelOption> {
    tiers
        .iter()
        .filter_map(|tier| {
            let id = tier.id.trim();
            if id.is_empty() {
                return None;
            }
            let label = if tier.name.trim().is_empty() {
                id.to_owned()
            } else {
                tier.name.trim().to_owned()
            };
            Some(ProviderModelOption {
                id: id.to_owned(),
                label,
                description: (!tier.description.trim().is_empty())
                    .then(|| tier.description.clone()),
                recommended: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                service_tiers: Vec::new(),
            })
        })
        .collect()
}

fn native_model_catalog_response(
    provider_type: ProviderType,
    source: &'static str,
    models: Vec<ProviderModelOption>,
    default_model: &str,
    default_reasoning_effort: Option<String>,
) -> ProviderModelsResponse {
    let reasoning_efforts = models
        .iter()
        .find(|model| model.id == default_model)
        .or_else(|| models.first())
        .map(|model| model.supported_reasoning_efforts.clone())
        .unwrap_or_default();
    let supports_reasoning_effort_selection = provider_supports_reasoning_effort_selection(&models);
    ProviderModelsResponse {
        provider_type,
        supports_model_selection: true,
        models,
        supports_reasoning_effort_selection,
        reasoning_efforts,
        supports_service_tier_selection: false,
        service_tiers: Vec::new(),
        default_model: Some(default_model.to_owned()),
        default_reasoning_effort,
        source,
        error: None,
    }
}

fn native_reasoning_efforts(
    default: &str,
    supported: &[&str],
) -> Vec<ProviderReasoningEffortOption> {
    supported
        .iter()
        .copied()
        .filter_map(|id| {
            native_reasoning_effort_metadata(id).map(|(id, label, description)| {
                ProviderReasoningEffortOption {
                    id: id.to_owned(),
                    label: label.to_owned(),
                    description: Some(description.to_owned()),
                    recommended: id == default,
                }
            })
        })
        .collect()
}

fn native_reasoning_effort_metadata(
    id: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match id {
        "off" => Some(("off", "Off", "Disable explicit thinking controls.")),
        "minimal" => Some((
            "minimal",
            "Minimal",
            "Use the smallest explicit reasoning budget.",
        )),
        "low" => Some((
            "low",
            "Low",
            "Prefer lower latency and lower reasoning budget.",
        )),
        "medium" => Some((
            "medium",
            "Medium",
            "Balanced reasoning budget for normal coding tasks.",
        )),
        "high" => Some((
            "high",
            "High",
            "Use a larger reasoning budget for harder tasks.",
        )),
        "xhigh" => Some((
            "xhigh",
            "Extra High",
            "Use the highest supported adaptive reasoning effort.",
        )),
        "max" => Some(("max", "Max", "Maximum capability with deepest reasoning.")),
        _ => None,
    }
}

fn native_model_option(
    id: &str,
    label: &str,
    description: &str,
    recommended: bool,
    default_reasoning_effort: &str,
    reasoning_efforts: Vec<ProviderReasoningEffortOption>,
) -> ProviderModelOption {
    ProviderModelOption {
        id: id.to_owned(),
        label: label.to_owned(),
        description: Some(description.to_owned()),
        recommended,
        default_reasoning_effort: Some(default_reasoning_effort.to_owned()),
        supported_reasoning_efforts: reasoning_efforts,
        service_tiers: Vec::new(),
    }
}

/// Reasoning levels supported by every model in the catalog, preserving the
/// first model's ordering. Used when no model is chosen so any selectable
/// level stays valid regardless of which model the CLI resolves.
fn common_reasoning_efforts(models: &[ProviderModelOption]) -> Vec<ProviderReasoningEffortOption> {
    let Some(first) = models.first() else {
        return Vec::new();
    };
    first
        .supported_reasoning_efforts
        .iter()
        .filter(|effort| {
            models.iter().all(|model| {
                model
                    .supported_reasoning_efforts
                    .iter()
                    .any(|candidate| candidate.id == effort.id)
            })
        })
        .cloned()
        .collect()
}

// Mirrors the Claude Code CLI model picker; effort levels map to the CLI's
// `--effort` values. Per the CLI's own gating, `xhigh` is available on
// Fable 5 and Opus 4.8 only, and `max` additionally on Sonnet 4.6.
fn claude_code_models() -> Vec<ProviderModelOption> {
    let claude_haiku_efforts = native_reasoning_efforts("high", &["low", "medium", "high"]);
    let claude_sonnet_efforts = native_reasoning_efforts("high", &["low", "medium", "high", "max"]);
    let claude_deep_efforts =
        native_reasoning_efforts("high", &["low", "medium", "high", "xhigh", "max"]);
    vec![
        native_model_option(
            "claude-fable-5",
            "Fable 5",
            "Newest model for complex, long-running work.",
            false,
            "high",
            claude_deep_efforts.clone(),
        ),
        native_model_option(
            "claude-opus-4-8",
            "Claude Opus 4.8",
            "Most capable for the hardest and longest-running tasks.",
            false,
            "high",
            claude_deep_efforts,
        ),
        native_model_option(
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            "Best for everyday, complex tasks.",
            false,
            "high",
            claude_sonnet_efforts,
        ),
        native_model_option(
            "claude-haiku-4-5",
            "Claude Haiku 4.5",
            "Fastest for quick answers.",
            false,
            "high",
            claude_haiku_efforts,
        ),
    ]
}

fn native_claude_models() -> Vec<ProviderModelOption> {
    let claude_standard_efforts =
        native_reasoning_efforts("high", &["off", "minimal", "low", "medium", "high"]);
    let claude_opus_efforts = native_reasoning_efforts(
        "xhigh",
        &["off", "minimal", "low", "medium", "high", "xhigh"],
    );
    vec![
        native_model_option(
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            "Default Claude model backend for Garyx's native agent loop.",
            true,
            "high",
            claude_standard_efforts.clone(),
        ),
        native_model_option(
            "claude-opus-4-7",
            "Claude Opus 4.7",
            "Highest-depth Claude model option.",
            false,
            "xhigh",
            claude_opus_efforts,
        ),
        native_model_option(
            "claude-haiku-4-5",
            "Claude Haiku 4.5",
            "Lower-latency Claude model option.",
            false,
            "high",
            claude_standard_efforts,
        ),
    ]
}

fn native_gemini_models() -> Vec<ProviderModelOption> {
    let gemini_25_efforts =
        native_reasoning_efforts("high", &["off", "minimal", "low", "medium", "high"]);
    let gemini_3_flash_efforts =
        native_reasoning_efforts("high", &["minimal", "low", "medium", "high"]);
    let gemini_31_pro_efforts = native_reasoning_efforts("high", &["low", "high"]);
    vec![
        native_model_option(
            "gemini-3-flash-preview",
            "Gemini 3 Flash Preview",
            "Default Gemini model backend for Garyx's native agent loop.",
            true,
            "high",
            gemini_3_flash_efforts,
        ),
        native_model_option(
            "gemini-3.1-pro-preview",
            "Gemini 3.1 Pro Preview",
            "Higher-depth Gemini model option.",
            false,
            "high",
            gemini_31_pro_efforts,
        ),
        native_model_option(
            "gemini-2.5-pro",
            "Gemini 2.5 Pro",
            "Stable pro Gemini model option.",
            false,
            "high",
            gemini_25_efforts.clone(),
        ),
        native_model_option(
            "gemini-2.5-flash",
            "Gemini 2.5 Flash",
            "Lower-latency Gemini model option.",
            false,
            "high",
            gemini_25_efforts,
        ),
    ]
}

fn antigravity_models() -> Vec<ProviderModelOption> {
    vec![
        ProviderModelOption {
            id: "Claude Opus 4.6 (Thinking)".to_owned(),
            label: "Claude Opus 4.6 (Thinking)".to_owned(),
            description: Some(
                "Default Antigravity model; uses the Claude backend through the local agy CLI."
                    .to_owned(),
            ),
            recommended: true,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Claude Sonnet 4.6 (Thinking)".to_owned(),
            label: "Claude Sonnet 4.6 (Thinking)".to_owned(),
            description: Some("Claude option exposed by the local agy CLI.".to_owned()),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.5 Flash (Low)".to_owned(),
            label: "Gemini 3.5 Flash (Low)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.5 Flash (Medium)".to_owned(),
            label: "Gemini 3.5 Flash (Medium)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.5 Flash (High)".to_owned(),
            label: "Gemini 3.5 Flash (High)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.1 Pro (Low)".to_owned(),
            label: "Gemini 3.1 Pro (Low)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.1 Pro (High)".to_owned(),
            label: "Gemini 3.1 Pro (High)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "GPT-OSS 120B (Medium)".to_owned(),
            label: "GPT-OSS 120B (Medium)".to_owned(),
            description: Some(
                "Antigravity open model option exposed by the local agy CLI.".to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
    ]
}

async fn resolve_codex_models_client_version() -> String {
    if let Ok(value) = std::env::var("CODEX_CLIENT_VERSION")
        && let Some(version) = parse_codex_cli_version(&value)
    {
        return version;
    }
    let version = timeout(
        Duration::from_secs(2),
        Command::new("codex").arg("--version").output(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .and_then(|output| parse_codex_cli_version(&output));
    version.unwrap_or_else(|| CODEX_MODELS_CLIENT_VERSION_FLOOR.to_owned())
}

fn configured_gpt_config(config: &GaryxConfig) -> GaryxNativeConfig {
    for key in ["gpt", "openai", "garyx", "garyx_native", "native"] {
        if let Some(value) = config.agents.get(key)
            && let Some(config) = gpt_config_from_agent_config(value)
        {
            return config;
        }
    }
    for value in config.agents.values() {
        if let Some(config) = gpt_config_from_agent_config(value) {
            return config;
        }
    }
    GaryxNativeConfig::default()
}

fn gpt_config_from_agent_config(value: &Value) -> Option<GaryxNativeConfig> {
    let config = serde_json::from_value::<AgentProviderConfig>(value.clone()).ok()?;
    if !matches!(
        config.provider_type.as_str(),
        "gpt" | "openai" | "openai_gpt" | "garyx_native" | "garyx" | "native"
    ) {
        return None;
    }
    Some(GaryxNativeConfig {
        default_model: config.default_model,
        model: config.model,
        model_reasoning_effort: config.model_reasoning_effort,
        model_service_tier: config.model_service_tier,
        max_turns: config.max_turns,
        timeout_seconds: config.timeout_seconds,
        workspace_dir: config.workspace_dir,
        env: config.env,
        auth_source: config.auth_source,
        base_url: config.base_url,
        codex_home: config.codex_home,
        max_tool_iterations: config.max_tool_iterations,
        request_timeout_seconds: config.request_timeout_seconds,
        ..Default::default()
    })
}

async fn fetch_gemini_acp_models(config: &GaryxConfig) -> Result<GeminiModelDiscovery, String> {
    let gemini_bin = configured_gemini_bin(config);
    let mut child = Command::new(&gemini_bin)
        .arg("--acp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start local Gemini ACP `{gemini_bin}`: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "local Gemini ACP stdin was unavailable".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "local Gemini ACP stdout was unavailable".to_owned())?;
    let mut lines = BufReader::new(stdout).lines();

    let result = async {
        send_acp_request(
            &mut stdin,
            1,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": {
                        "readTextFile": false,
                        "writeTextFile": false,
                    }
                },
                "clientInfo": {
                    "name": "garyx-provider-models",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )
        .await?;
        let initialize = read_acp_response(&mut lines, 1, Duration::from_secs(10)).await?;
        if let Some(message) = acp_error_message(&initialize) {
            return Err(format!("local Gemini ACP initialize failed: {message}"));
        }

        let mut missing_method_count = 0usize;
        let mut other_errors = Vec::new();
        for (index, method) in GEMINI_ACP_MODEL_METHODS.iter().enumerate() {
            let request_id = (index as u64) + 2;
            send_acp_request(&mut stdin, request_id, method, json!({})).await?;
            let response =
                match read_acp_response(&mut lines, request_id, Duration::from_secs(2)).await {
                    Ok(response) => response,
                    Err(error) => {
                        other_errors.push(format!("{method}: {error}"));
                        continue;
                    }
                };
            if acp_error_code(&response) == Some(-32601) {
                missing_method_count += 1;
                continue;
            }
            if let Some(message) = acp_error_message(&response) {
                other_errors.push(format!("{method}: {message}"));
                continue;
            }
            if let Some(result) = response.get("result") {
                let discovery = parse_gemini_models_result(result);
                if !discovery.models.is_empty() {
                    return Ok(discovery);
                }
            }
        }

        if missing_method_count == GEMINI_ACP_MODEL_METHODS.len() {
            return Err("local Gemini ACP does not expose a model list method".to_owned());
        }
        if !other_errors.is_empty() {
            return Err(format!(
                "local Gemini ACP did not return a model list: {}",
                other_errors.join("; ")
            ));
        }
        Err("local Gemini ACP returned no model list".to_owned())
    }
    .await;

    shutdown_child(&mut child).await;
    result
}

/// Resolve the app-server binary for a Codex-family provider (Codex or Traex).
/// Mirrors the binary defaulting in the bridge provider factory.
fn app_server_model_bin(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::Traex => "traex",
        _ => "codex",
    }
}

/// Dynamically discover models from a Codex-family app-server (Codex or Traex)
/// by spawning `<bin> app-server` and calling the `model/list` JSON-RPC method.
/// This reflects the real backend catalog (e.g. Traex exposes Doubao/GLM/Gemini/
/// GPT/etc.) instead of a hardcoded preset list.
async fn fetch_app_server_models(
    codex_bin: &str,
    source: &'static str,
) -> Result<GptModelDiscovery, String> {
    #[cfg(test)]
    if std::env::var_os("GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH").is_none() {
        return Err("app-server model fetch disabled in tests".to_owned());
    }

    let mut child = Command::new(codex_bin)
        // Mirror the SDK transport invocation (codex-sdk transport.rs) so we
        // pin the stdio transport instead of relying on the default, which a
        // user config or future build could change.
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start `{codex_bin} app-server`: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "app-server stdin was unavailable".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "app-server stdout was unavailable".to_owned())?;
    let mut lines = BufReader::new(stdout).lines();

    let result = async {
        send_acp_request(
            &mut stdin,
            1,
            "initialize",
            json!({
                "clientInfo": {
                    "name": "garyx-provider-models",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": { "experimentalApi": true },
            }),
        )
        .await?;
        let initialize = read_acp_response(&mut lines, 1, Duration::from_secs(10)).await?;
        if let Some(message) = acp_error_message(&initialize) {
            return Err(format!("app-server initialize failed: {message}"));
        }
        send_jsonrpc_notification(&mut stdin, "initialized", json!({})).await?;

        send_acp_request(&mut stdin, 2, "model/list", json!({})).await?;
        let response = read_acp_response(&mut lines, 2, Duration::from_secs(15)).await?;
        if acp_error_code(&response) == Some(-32601) {
            return Err("app-server does not expose model/list".to_owned());
        }
        if let Some(message) = acp_error_message(&response) {
            return Err(format!("app-server model/list failed: {message}"));
        }
        let result = response
            .get("result")
            .ok_or_else(|| "app-server model/list returned no result".to_owned())?;
        let discovery = parse_app_server_models(result, source);
        if discovery.models.is_empty() {
            return Err("app-server model/list returned no models".to_owned());
        }
        Ok(discovery)
    }
    .await;

    shutdown_child(&mut child).await;
    result
}

/// Parse a `model/list` response (`{ data: [Model] }`) into a model discovery.
fn parse_app_server_models(result: &Value, source: &'static str) -> GptModelDiscovery {
    let entries = result
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut models = Vec::new();
    let mut default_model: Option<String> = None;
    let mut default_reasoning: Vec<ProviderReasoningEffortOption> = Vec::new();
    let mut first_reasoning: Vec<ProviderReasoningEffortOption> = Vec::new();
    let mut default_service_tiers: Vec<ProviderModelOption> = Vec::new();
    let mut first_service_tiers: Vec<ProviderModelOption> = Vec::new();
    let mut saw_default_model = false;

    for entry in entries {
        if entry
            .get("hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(id) = entry
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let label = entry
            .get("displayName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(id)
            .to_owned();
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let default_effort = entry
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let supported = parse_app_server_reasoning_efforts(&entry, default_effort.as_deref());
        // Service tiers are plumbed through to thread/start (serviceTier), so we
        // advertise whatever the backend reports per model.
        let service_tiers = parse_app_server_service_tiers(&entry);
        let is_default = entry
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if first_reasoning.is_empty() && !supported.is_empty() {
            first_reasoning = supported.clone();
        }
        if first_service_tiers.is_empty() && !service_tiers.is_empty() {
            first_service_tiers = service_tiers.clone();
        }
        if is_default {
            saw_default_model = true;
            default_reasoning = supported.clone();
            default_service_tiers = service_tiers.clone();
        }

        // Expand context-window variants the backend advertises under
        // businessMetadata.variants (e.g. TRAE's "Standard" 272K vs "Max" 1M)
        // into separate selectable models, matching the TRAE picker. Only expand
        // when a distinct Max variant exists; single-variant models stay as the
        // plain base model.
        let variants = entry
            .get("businessMetadata")
            .and_then(|meta| meta.get("variants"));
        let standard_key = variants
            .and_then(|v| v.get("standard_key"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let max_key = variants
            .and_then(|v| v.get("max_key"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let (Some(standard_key), Some(max_key)) = (standard_key, max_key) {
            let context_label = |key: &str| {
                variants
                    .and_then(|v| v.get(key))
                    .and_then(Value::as_u64)
                    .map(|ctx| format!("{} context window", format_context_window(ctx)))
            };
            if is_default {
                default_model = Some(standard_key.to_owned());
            }
            models.push(ProviderModelOption {
                id: standard_key.to_owned(),
                label: format!("{label} / Standard"),
                description: context_label("standard_context_window"),
                recommended: is_default,
                default_reasoning_effort: default_effort.clone(),
                supported_reasoning_efforts: supported.clone(),
                service_tiers: service_tiers.clone(),
            });
            models.push(ProviderModelOption {
                id: max_key.to_owned(),
                label: format!("{label} / Max"),
                description: context_label("max_context_window"),
                recommended: false,
                default_reasoning_effort: default_effort,
                supported_reasoning_efforts: supported,
                service_tiers,
            });
        } else {
            if is_default {
                default_model = Some(id.to_owned());
            }
            models.push(ProviderModelOption {
                id: id.to_owned(),
                label,
                description,
                recommended: is_default,
                default_reasoning_effort: default_effort,
                supported_reasoning_efforts: supported,
                service_tiers,
            });
        }
    }

    let reasoning_efforts = if saw_default_model {
        default_reasoning
    } else {
        first_reasoning
    };
    let service_tiers = if saw_default_model {
        default_service_tiers
    } else {
        first_service_tiers
    };

    GptModelDiscovery {
        models,
        default_model,
        reasoning_efforts,
        service_tiers,
        source,
        error: None,
    }
}

/// Render a token count as a compact context-window label (e.g. 272000 -> "272K",
/// 1000000 -> "1M").
fn format_context_window(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let millions = tokens as f64 / 1_000_000.0;
        if millions.fract().abs() < f64::EPSILON {
            format!("{}M", millions as u64)
        } else {
            format!("{millions:.1}M")
        }
    } else {
        format!("{}K", (tokens as f64 / 1000.0).round() as u64)
    }
}

fn parse_app_server_service_tiers(entry: &Value) -> Vec<ProviderModelOption> {
    entry
        .get("serviceTiers")
        .and_then(Value::as_array)
        .map(|tiers| {
            tiers
                .iter()
                .filter_map(|tier| {
                    let id = tier
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let label = tier
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(id)
                        .to_owned();
                    let description = tier
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned);
                    Some(ProviderModelOption {
                        id: id.to_owned(),
                        label,
                        description,
                        recommended: false,
                        default_reasoning_effort: None,
                        supported_reasoning_efforts: Vec::new(),
                        service_tiers: Vec::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_app_server_reasoning_efforts(
    entry: &Value,
    default_effort: Option<&str>,
) -> Vec<ProviderReasoningEffortOption> {
    entry
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let effort = option
                        .get("reasoningEffort")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let metadata = native_reasoning_effort_metadata(effort);
                    let label = metadata
                        .map(|(_, label, _)| label.to_owned())
                        .unwrap_or_else(|| effort.to_owned());
                    let description = option
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                        .or_else(|| metadata.map(|(_, _, description)| description.to_owned()));
                    Some(ProviderReasoningEffortOption {
                        id: effort.to_owned(),
                        label,
                        description,
                        recommended: Some(effort) == default_effort,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn configured_gemini_bin(config: &GaryxConfig) -> String {
    for key in ["gemini", "gemini_cli"] {
        if let Some(value) = config.agents.get(key)
            && let Some(bin) = gemini_bin_from_agent_config(value)
        {
            return bin;
        }
    }
    for value in config.agents.values() {
        if let Some(bin) = gemini_bin_from_agent_config(value) {
            return bin;
        }
    }
    "gemini".to_owned()
}

fn gemini_bin_from_agent_config(value: &Value) -> Option<String> {
    let config = serde_json::from_value::<AgentProviderConfig>(value.clone()).ok()?;
    if config.provider_type != "gemini_cli" {
        return None;
    }
    let bin = config.gemini_bin.trim();
    (!bin.is_empty()).then(|| bin.to_owned())
}

async fn send_acp_request(
    stdin: &mut ChildStdin,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    stdin
        .write_all(payload.to_string().as_bytes())
        .await
        .map_err(|error| format!("failed to write Gemini ACP request: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("failed to finish Gemini ACP request: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("failed to flush Gemini ACP request: {error}"))
}

async fn send_jsonrpc_notification(
    stdin: &mut ChildStdin,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    stdin
        .write_all(payload.to_string().as_bytes())
        .await
        .map_err(|error| format!("failed to write {method} notification: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("failed to finish {method} notification: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("failed to flush {method} notification: {error}"))
}

async fn read_acp_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    expected_id: u64,
    duration: Duration,
) -> Result<Value, String> {
    let future = async {
        loop {
            let Some(line) = lines
                .next_line()
                .await
                .map_err(|error| format!("failed to read Gemini ACP response: {error}"))?
            else {
                return Err("local Gemini ACP closed before responding".to_owned());
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value = serde_json::from_str::<Value>(trimmed)
                .map_err(|error| format!("invalid Gemini ACP JSON response: {error}"))?;
            if value.get("id").and_then(Value::as_u64) == Some(expected_id) {
                return Ok(value);
            }
        }
    };
    timeout(duration, future)
        .await
        .map_err(|_| format!("timed out waiting for Gemini ACP request {expected_id}"))?
}

async fn shutdown_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn acp_error_code(response: &Value) -> Option<i64> {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
}

fn acp_error_message(response: &Value) -> Option<String> {
    let error = response.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    let details = error
        .get("data")
        .and_then(|data| data.get("details"))
        .and_then(Value::as_str);
    Some(match details {
        Some(details) if !details.is_empty() => format!("{message} ({details})"),
        _ => message.to_owned(),
    })
}

fn parse_gemini_models_result(result: &Value) -> GeminiModelDiscovery {
    let candidates = result
        .get("models")
        .or_else(|| result.get("modelOptions"))
        .or_else(|| result.get("model_options"))
        .or_else(|| result.get("data").and_then(|data| data.get("models")))
        .unwrap_or(result);

    let models = candidates
        .as_array()
        .map(|values| parse_model_array(values))
        .unwrap_or_default();
    let default_model = string_field(result, &["default_model", "defaultModel", "default"]);

    GeminiModelDiscovery {
        models,
        default_model,
    }
}

fn parse_model_array(values: &[Value]) -> Vec<ProviderModelOption> {
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for value in values {
        let Some(option) = parse_model_option(value) else {
            continue;
        };
        if seen.insert(option.id.clone()) {
            models.push(option);
        }
    }
    models
}

fn parse_model_option(value: &Value) -> Option<ProviderModelOption> {
    if let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) {
        return Some(ProviderModelOption {
            id: id.to_owned(),
            label: model_label(id, None),
            description: None,
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        });
    }

    let object = value.as_object()?;
    let id = ["id", "name", "model", "model_id", "modelId"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    let label = ["label", "display_name", "displayName", "title"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|label| !label.is_empty());
    let description = ["description", "summary"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_owned);

    Some(ProviderModelOption {
        id: id.to_owned(),
        label: model_label(id, label),
        description,
        recommended: object
            .get("recommended")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
    })
}

fn model_label(id: &str, label: Option<&str>) -> String {
    label
        .map(str::to_owned)
        .unwrap_or_else(|| id.strip_prefix("models/").unwrap_or(id).to_owned())
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_flexible_model_payloads() {
        let discovery = parse_gemini_models_result(&json!({
            "models": [
                "gemini-2.5-pro",
                {
                    "name": "models/gemini-2.5-flash",
                    "displayName": "Gemini 2.5 Flash",
                    "description": "Fast model",
                    "recommended": true
                },
                { "id": "" },
                "gemini-2.5-pro"
            ],
            "defaultModel": "gemini-2.5-flash"
        }));

        assert_eq!(discovery.default_model.as_deref(), Some("gemini-2.5-flash"));
        assert_eq!(discovery.models.len(), 2);
        assert_eq!(discovery.models[0].id, "gemini-2.5-pro");
        assert_eq!(discovery.models[0].label, "gemini-2.5-pro");
        assert_eq!(discovery.models[1].id, "models/gemini-2.5-flash");
        assert_eq!(discovery.models[1].label, "Gemini 2.5 Flash");
        assert!(discovery.models[1].recommended);
    }

    #[test]
    fn reads_configured_gemini_binary() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "custom-gemini".to_owned(),
            json!({
                "provider_type": "gemini_cli",
                "gemini_bin": "/tmp/gemini-acp"
            }),
        );

        assert_eq!(configured_gemini_bin(&config), "/tmp/gemini-acp");
    }

    #[test]
    fn maps_codex_presets_with_model_specific_reasoning() {
        let discovery = gpt_builtin_models(None);

        assert_eq!(discovery.source, "codex_builtin");
        assert_eq!(discovery.default_model.as_deref(), Some("gpt-5.5"));
        assert_eq!(discovery.models[0].id, "gpt-5.5");
        assert!(discovery.models[0].recommended);
        assert_eq!(discovery.models[0].service_tiers[0].id, "priority");
        assert_eq!(discovery.models[0].service_tiers[0].label, "Fast");
        assert_eq!(
            discovery.models[0].default_reasoning_effort.as_deref(),
            Some("medium")
        );
        assert_eq!(discovery.models[0].supported_reasoning_efforts[0].id, "low");
        assert_eq!(discovery.service_tiers[0].id, "priority");
        assert_eq!(discovery.reasoning_efforts[1].id, "medium");
        assert!(discovery.reasoning_efforts[1].recommended);
    }

    #[test]
    fn gpt_configured_unknown_default_model_does_not_reuse_previous_options() {
        let discovery = apply_default_model_to_gpt_discovery(
            gpt_builtin_models(None),
            Some("gpt-6-turbo".to_owned()),
        );

        assert_eq!(discovery.default_model.as_deref(), Some("gpt-6-turbo"));
        assert!(discovery.reasoning_efforts.is_empty());
        assert!(discovery.service_tiers.is_empty());
    }

    #[test]
    fn configured_default_model_does_not_scan_non_default_agent_keys() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "custom-gpt".to_owned(),
            json!({
                "provider_type": "gpt",
                "default_model": "gpt-custom-shadow"
            }),
        );

        let default_model = configured_default_model(
            &config,
            ProviderType::Gpt,
            &["gpt", "openai", "garyx", "garyx_native", "native"],
        );

        assert_eq!(default_model, None);
    }

    #[tokio::test]
    async fn claude_code_catalog_ignores_empty_configured_provider_default_reasoning_effort() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "claude".to_owned(),
            json!({
                "provider_type": "claude_code",
                "default_model": "claude-opus-4-8",
                "model_reasoning_effort": "  "
            }),
        );

        let response = list_provider_models(&config, ProviderType::ClaudeCode).await;
        let payload = serde_json::to_value(response).expect("provider models response");

        assert_eq!(payload["default_model"], "claude-opus-4-8");
        assert!(payload.get("default_reasoning_effort").is_none());
    }

    #[tokio::test]
    async fn antigravity_model_catalog_defaults_to_claude_opus() {
        let response =
            list_provider_models(&GaryxConfig::default(), ProviderType::AntigravityCli).await;

        assert_eq!(response.provider_type, ProviderType::AntigravityCli);
        assert!(response.supports_model_selection);
        assert_eq!(response.source, "antigravity_builtin");
        assert_eq!(
            response.default_model.as_deref(),
            Some("Claude Opus 4.6 (Thinking)")
        );
        assert!(
            response
                .models
                .iter()
                .any(|model| model.id == "Claude Sonnet 4.6 (Thinking)")
        );
    }

    #[tokio::test]
    async fn claude_code_model_catalog_supports_selection_and_reasoning() {
        let response =
            list_provider_models(&GaryxConfig::default(), ProviderType::ClaudeCode).await;

        assert_eq!(response.provider_type, ProviderType::ClaudeCode);
        assert!(response.supports_model_selection);
        assert!(response.supports_reasoning_effort_selection);
        assert_eq!(response.source, "claude_code_builtin");
        // The CLI's account default is unknowable, so no default is claimed and
        // the model-less effort list is the intersection every model supports.
        assert_eq!(response.default_model, None);
        assert_eq!(
            response
                .reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            response
                .models
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "claude-fable-5",
                "claude-opus-4-8",
                "claude-sonnet-4-6",
                "claude-haiku-4-5",
            ]
        );
        for deep_model in ["claude-fable-5", "claude-opus-4-8"] {
            assert_eq!(
                response
                    .models
                    .iter()
                    .find(|model| model.id == deep_model)
                    .expect("deep model")
                    .supported_reasoning_efforts
                    .iter()
                    .map(|effort| effort.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["low", "medium", "high", "xhigh", "max"]
            );
        }
        assert_eq!(
            response
                .models
                .iter()
                .find(|model| model.id == "claude-haiku-4-5")
                .expect("haiku model")
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high"]
        );
        assert!(!response.supports_service_tier_selection);
    }

    #[tokio::test]
    async fn claude_code_dynamic_catalog_maps_models_and_efforts() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer test-claude-token"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(header("anthropic-beta", "oauth-2025-04-20"))
            .and(header("user-agent", crate::claude_oauth::CLAUDE_USER_AGENT))
            .and(header("accept", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {
                        "id": "claude-empty-effort-20260101",
                        "display_name": "Claude Empty Effort",
                        "created_at": "2026-01-01T00:00:00Z",
                        "capabilities": {
                            "effort": {
                                "supported": true,
                                "low": { "supported": false },
                                "medium": { "supported": false }
                            }
                        }
                    },
                    {
                        "id": "claude-opus-4-8-20260201",
                        "display_name": "  Claude Opus 4.8  ",
                        "created_at": "2026-02-01T00:00:00Z",
                        "capabilities": {
                            "effort": {
                                "supported": true,
                                "low": { "supported": true },
                                "medium": { "supported": true },
                                "high": { "supported": true },
                                "xhigh": { "supported": true },
                                "max": { "supported": true }
                            }
                        }
                    },
                    {
                        "id": "claude-sonnet-4-6-20260115",
                        "display_name": "",
                        "created_at": "2026-01-15T00:00:00Z",
                        "capabilities": {
                            "effort": {
                                "supported": true,
                                "low": { "supported": true },
                                "medium": { "supported": true },
                                "high": { "supported": true }
                            }
                        }
                    },
                    {
                        "id": "claude-haiku-4-5-20251215",
                        "display_name": "Claude Haiku 4.5",
                        "created_at": "2025-12-15T00:00:00Z",
                        "capabilities": {
                            "effort": {
                                "supported": true,
                                "low": { "supported": true },
                                "medium": { "supported": true },
                                "high": { "supported": true }
                            }
                        }
                    },
                    {
                        "id": "claude-fable-5-20251201",
                        "display_name": "Claude Fable 5",
                        "created_at": "2025-12-01T00:00:00Z",
                        "capabilities": {
                            "effort": {
                                "supported": true,
                                "low": { "supported": true },
                                "medium": { "supported": true },
                                "high": { "supported": true },
                                "xhigh": { "supported": true },
                                "max": { "supported": true }
                            }
                        }
                    },
                    {
                        "id": "claude-opus-4-7-20251101",
                        "display_name": "Claude Opus 4.7",
                        "created_at": "2025-11-01T00:00:00Z",
                        "capabilities": {
                            "effort": {
                                "supported": true,
                                "low": { "supported": true },
                                "medium": { "supported": true },
                                "high": { "supported": true },
                                "xhigh": { "supported": true }
                            }
                        }
                    },
                    {
                        "id": "claude-sonnet-4-5-20251001",
                        "display_name": "Claude Sonnet 4.5",
                        "created_at": "2025-10-01T00:00:00Z",
                        "capabilities": {
                            "effort": {
                                "supported": true,
                                "low": { "supported": true },
                                "medium": { "supported": true },
                                "high": { "supported": true },
                                "max": { "supported": true }
                            }
                        }
                    },
                    {
                        "id": "claude-no-effort",
                        "display_name": "Claude No Effort",
                        "created_at": null,
                        "capabilities": { "effort": { "supported": false } }
                    },
                    {
                        "id": "claude-missing-created-b",
                        "display_name": "Claude Missing Created B",
                        "created_at": null,
                        "capabilities": { "effort": { "supported": false } }
                    },
                    {
                        "id": "",
                        "display_name": "Skipped"
                    }
                ]
            })))
            .mount(&server)
            .await;

        let discovery = fetch_claude_code_models_from_endpoint(
            &server.uri(),
            "test-claude-token",
            Duration::from_secs(5),
        )
        .await
        .expect("mock Claude model catalog");

        assert_eq!(discovery.source, "claude_code_api");
        assert_eq!(
            discovery
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "claude-opus-4-8-20260201",
                "claude-sonnet-4-6-20260115",
                "claude-empty-effort-20260101",
                "claude-haiku-4-5-20251215",
                "claude-fable-5-20251201",
                "claude-opus-4-7-20251101",
                "claude-sonnet-4-5-20251001",
                "claude-no-effort",
                "claude-missing-created-b",
            ]
        );
        let opus = &discovery.models[0];
        assert_eq!(opus.label, "Claude Opus 4.8");
        assert_eq!(opus.default_reasoning_effort, None);
        assert_eq!(
            opus.supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
        let sonnet = &discovery.models[1];
        assert_eq!(sonnet.label, "Claude Sonnet 4 6");
        assert_eq!(
            sonnet
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high"]
        );
        assert_eq!(discovery.models.len(), 9);
        assert!(discovery.models[2].supported_reasoning_efforts.is_empty());
        assert!(discovery.models[7].supported_reasoning_efforts.is_empty());
        assert!(discovery.models[8].supported_reasoning_efforts.is_empty());
        assert!(discovery.default_model.is_none());
        assert!(discovery.reasoning_efforts.is_empty());
    }

    #[tokio::test]
    async fn claude_code_dynamic_catalog_non_200_and_timeout_are_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(503).set_body_string("upstream details stay short"))
            .mount(&server)
            .await;

        let error = fetch_claude_code_models_from_endpoint(
            &server.uri(),
            "test-claude-token",
            Duration::from_secs(5),
        )
        .await
        .expect_err("non-200 should error");
        assert!(error.contains("HTTP 503"), "unexpected error: {error}");
        assert!(!error.contains("upstream details stay short"));

        let slow_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(50))
                    .set_body_json(json!({ "data": [] })),
            )
            .mount(&slow_server)
            .await;
        let error = fetch_claude_code_models_from_endpoint(
            &slow_server.uri(),
            "test-claude-token",
            Duration::from_millis(1),
        )
        .await
        .expect_err("timeout should error");
        assert!(error.contains("timed out"), "unexpected error: {error}");
    }

    #[test]
    fn claude_code_fallback_preserves_nonempty_builtin_catalog() {
        clear_provider_model_discovery_cache_for_tests();

        let discovery = discover_or_fallback(
            "test_claude_fallback",
            Err("Claude OAuth token unavailable".to_owned()),
            |error| claude_code_builtin_models(Some(error)),
        );

        assert_eq!(discovery.source, "claude_code_builtin");
        assert!(discovery.error.as_deref().unwrap_or("").contains("OAuth"));
        assert!(!discovery.models.is_empty());
        assert!(provider_supports_reasoning_effort_selection(
            &discovery.models
        ));
    }

    #[test]
    fn discover_or_fallback_prefers_stale_success_before_builtin_preset() {
        clear_provider_model_discovery_cache_for_tests();
        let cached = ProviderModelDiscovery {
            models: vec![ProviderModelOption {
                id: "claude-stale".to_owned(),
                label: "Claude Stale".to_owned(),
                description: None,
                recommended: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: native_reasoning_efforts("low", &["low"]),
                service_tiers: Vec::new(),
            }],
            default_model: None,
            reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
            source: "claude_code_api",
            error: None,
        };
        let success = discover_or_fallback("test_claude_stale", Ok(cached), |error| {
            claude_code_builtin_models(Some(error))
        });
        assert_eq!(success.source, "claude_code_api");

        let stale = discover_or_fallback(
            "test_claude_stale",
            Err("network down".to_owned()),
            |error| claude_code_builtin_models(Some(error)),
        );

        assert_eq!(stale.source, "claude_code_api");
        assert_eq!(stale.models[0].id, "claude-stale");
        assert_eq!(stale.error.as_deref(), Some("network down"));
    }

    #[tokio::test]
    async fn gemini_cli_discovery_failure_uses_nonempty_preset() {
        let response = list_provider_models(&GaryxConfig::default(), ProviderType::GeminiCli).await;

        assert_eq!(response.provider_type, ProviderType::GeminiCli);
        assert!(response.supports_model_selection);
        assert_eq!(response.source, "gemini_cli_builtin");
        assert!(response.error.is_some());
        assert!(!response.models.is_empty());
    }

    #[tokio::test]
    async fn claude_code_catalog_uses_configured_provider_default_model() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "claude".to_owned(),
            json!({
                "provider_type": "claude_code",
                "default_model": "claude-opus-4-8",
                "model_reasoning_effort": "max"
            }),
        );

        let response = list_provider_models(&config, ProviderType::ClaudeCode).await;

        assert_eq!(response.default_model.as_deref(), Some("claude-opus-4-8"));
    }

    #[tokio::test]
    async fn claude_code_catalog_exposes_configured_provider_default_reasoning_effort() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "claude".to_owned(),
            json!({
                "provider_type": "claude_code",
                "default_model": "claude-opus-4-8",
                "model_reasoning_effort": "max"
            }),
        );

        let response = list_provider_models(&config, ProviderType::ClaudeCode).await;
        let payload = serde_json::to_value(response).expect("provider models response");

        assert_eq!(payload["default_reasoning_effort"], "max");
    }

    #[tokio::test]
    async fn codex_app_server_model_catalog_supports_selection_and_reasoning() {
        let response =
            list_provider_models(&GaryxConfig::default(), ProviderType::CodexAppServer).await;

        assert_eq!(response.provider_type, ProviderType::CodexAppServer);
        assert!(response.supports_model_selection);
        assert!(response.supports_reasoning_effort_selection);
        assert_eq!(response.source, "codex_builtin");
        assert!(response.default_model.is_none());
        assert!(!response.models.is_empty());
        assert!(!response.reasoning_efforts.is_empty());
    }

    #[tokio::test]
    async fn codex_app_server_catalog_uses_configured_provider_default_model() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "codex".to_owned(),
            json!({
                "provider_type": "codex_app_server",
                "default_model": "gpt-5.4"
            }),
        );

        let response = list_provider_models(&config, ProviderType::CodexAppServer).await;

        assert_eq!(response.default_model.as_deref(), Some("gpt-5.4"));
    }

    #[test]
    fn parse_app_server_models_maps_catalog_and_reasoning() {
        let result = json!({
            "data": [
                {
                    "id": "glm-5", "model": "glm-5", "displayName": "GLM-5",
                    "description": "smart", "defaultReasoningEffort": "medium",
                    "isDefault": false, "hidden": false,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "low", "description": "lo"},
                        {"reasoningEffort": "medium", "description": "med"}
                    ]
                },
                {
                    "id": "gpt-5.5", "model": "gpt-5.5", "displayName": "GPT-5.5",
                    "description": "", "defaultReasoningEffort": "high",
                    "isDefault": true, "hidden": false,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "high", "description": "hi"}
                    ]
                },
                {
                    "id": "secret", "model": "secret", "displayName": "Secret",
                    "hidden": true, "defaultReasoningEffort": "low",
                    "isDefault": false, "supportedReasoningEfforts": []
                }
            ]
        });

        let discovery = parse_app_server_models(&result, "traex_app_server");

        assert_eq!(discovery.source, "traex_app_server");
        // Hidden models are filtered out.
        assert_eq!(discovery.models.len(), 2);
        assert!(!discovery.models.iter().any(|model| model.id == "secret"));
        // Default comes from the `isDefault` flag.
        assert_eq!(discovery.default_model.as_deref(), Some("gpt-5.5"));
        // Top-level reasoning efforts come from the default model.
        assert_eq!(discovery.reasoning_efforts.len(), 1);
        assert_eq!(discovery.reasoning_efforts[0].id, "high");
        assert!(discovery.reasoning_efforts[0].recommended);
        // Per-model reasoning options are mapped with the default marked.
        let glm = discovery
            .models
            .iter()
            .find(|model| model.id == "glm-5")
            .expect("glm-5 present");
        assert_eq!(glm.label, "GLM-5");
        assert_eq!(glm.default_reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(glm.supported_reasoning_efforts.len(), 2);
        assert!(
            glm.supported_reasoning_efforts
                .iter()
                .any(|effort| effort.id == "medium" && effort.recommended)
        );
    }

    #[test]
    fn provider_reasoning_support_uses_any_model_effort() {
        let result = json!({
            "data": [
                {
                    "id": "doubao-empty", "model": "doubao-empty", "displayName": "Doubao",
                    "isDefault": true, "hidden": false, "supportedReasoningEfforts": []
                },
                {
                    "id": "openrouter-3o", "model": "openrouter-3o", "displayName": null,
                    "isDefault": false, "hidden": false,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "low"},
                        {"reasoningEffort": "medium"},
                        {"reasoningEffort": "high"},
                        {"reasoningEffort": "xhigh"},
                        {"reasoningEffort": "max"}
                    ]
                }
            ]
        });

        let discovery = parse_app_server_models(&result, "traex_app_server");

        assert!(discovery.reasoning_efforts.is_empty());
        assert!(provider_supports_reasoning_effort_selection(
            &discovery.models
        ));
        let openrouter = discovery
            .models
            .iter()
            .find(|model| model.id == "openrouter-3o")
            .expect("openrouter model");
        assert_eq!(openrouter.label, "openrouter-3o");
        assert_eq!(
            openrouter
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn parse_app_server_models_expands_context_window_variants() {
        let result = json!({
            "data": [
                {
                    "id": "gpt-5.5", "model": "GPT-5.5", "displayName": "GPT-5.5",
                    "description": "frontier", "defaultReasoningEffort": "medium",
                    "isDefault": false, "hidden": false, "supportedReasoningEfforts": [],
                    "businessMetadata": { "variants": {
                        "standard_key": "gpt-5.5__dev", "standard_context_window": 272000,
                        "max_key": "gpt-5.5__max", "max_context_window": 1000000
                    }}
                },
                {
                    "id": "glm-5", "model": "glm-5", "displayName": "GLM-5",
                    "description": "", "defaultReasoningEffort": "medium",
                    "isDefault": false, "hidden": false, "supportedReasoningEfforts": [],
                    "businessMetadata": { "variants": {
                        "standard_key": "glm-5__dev", "standard_context_window": 200000,
                        "max_key": null, "max_context_window": null
                    }}
                }
            ]
        });

        let discovery = parse_app_server_models(&result, "traex_app_server");
        let ids: Vec<&str> = discovery.models.iter().map(|m| m.id.as_str()).collect();
        // gpt-5.5 has a Max variant -> two options; glm-5 has only Standard -> one.
        assert_eq!(ids, vec!["gpt-5.5__dev", "gpt-5.5__max", "glm-5"]);
        let std = &discovery.models[0];
        assert_eq!(std.label, "GPT-5.5 / Standard");
        assert_eq!(std.description.as_deref(), Some("272K context window"));
        let max = &discovery.models[1];
        assert_eq!(max.label, "GPT-5.5 / Max");
        assert_eq!(max.description.as_deref(), Some("1M context window"));
        // Single-variant model keeps its plain display name.
        assert_eq!(discovery.models[2].label, "GLM-5");
    }

    // Real end-to-end discovery against the local `traex` binary. Opt-in via
    // GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH=1 (mirrors the Codex fetch guard)
    // because it spawns `traex app-server`.
    #[tokio::test]
    async fn traex_app_server_real_discovery_lists_models() {
        if std::env::var_os("GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH").is_none() {
            return;
        }
        let response = list_provider_models(&GaryxConfig::default(), ProviderType::Traex).await;
        assert_eq!(response.provider_type, ProviderType::Traex);
        assert_eq!(response.source, "traex_app_server");
        assert!(
            !response.models.is_empty(),
            "expected dynamically discovered traex models"
        );
        assert!(response.supports_model_selection);
        // The picker should be available when any discovered Traex model
        // advertises selectable reasoning efforts, even if the default model does
        // not.
        assert!(response.supports_reasoning_effort_selection);
    }

    #[tokio::test]
    async fn codex_app_server_real_discovery_lists_models_with_reasoning() {
        if std::env::var_os("GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH").is_none() {
            return;
        }
        let response =
            list_provider_models(&GaryxConfig::default(), ProviderType::CodexAppServer).await;
        assert_eq!(response.provider_type, ProviderType::CodexAppServer);
        assert_eq!(response.source, "codex_app_server");
        assert!(!response.models.is_empty());
        // Codex models advertise reasoning efforts; the picker should be on.
        assert!(response.supports_reasoning_effort_selection);
        // Service tiers are now plumbed to thread/start, so Codex advertises
        // them (e.g. Fast/priority).
        assert!(response.supports_service_tier_selection);
        assert!(!response.service_tiers.is_empty());
    }

    #[tokio::test]
    async fn native_claude_model_catalog_supports_selection_and_reasoning() {
        let response = list_provider_models(&GaryxConfig::default(), ProviderType::ClaudeLlm).await;

        assert_eq!(response.provider_type, ProviderType::ClaudeLlm);
        assert!(response.supports_model_selection);
        assert!(response.supports_reasoning_effort_selection);
        assert_eq!(response.default_model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(response.models[0].id, "claude-sonnet-4-6");
        assert!(response.models[0].recommended);
        assert_eq!(
            response.models[0]
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["off", "minimal", "low", "medium", "high"]
        );
        assert!(
            response
                .models
                .iter()
                .find(|model| model.id == "claude-opus-4-7")
                .expect("opus model")
                .supported_reasoning_efforts
                .iter()
                .any(|effort| effort.id == "xhigh")
        );
        assert_eq!(
            response
                .reasoning_efforts
                .last()
                .map(|effort| effort.id.as_str()),
            Some("high")
        );
    }

    #[tokio::test]
    async fn native_claude_catalog_uses_configured_provider_default_model() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "anthropic".to_owned(),
            json!({
                "provider_type": "anthropic",
                "default_model": "claude-opus-4-7"
            }),
        );

        let response = list_provider_models(&config, ProviderType::ClaudeLlm).await;

        assert_eq!(response.default_model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(
            response
                .reasoning_efforts
                .last()
                .map(|effort| effort.id.as_str()),
            Some("xhigh")
        );
    }

    #[tokio::test]
    async fn native_gemini_model_catalog_supports_selection_and_reasoning() {
        let response = list_provider_models(&GaryxConfig::default(), ProviderType::GeminiLlm).await;

        assert_eq!(response.provider_type, ProviderType::GeminiLlm);
        assert!(response.supports_model_selection);
        assert!(response.supports_reasoning_effort_selection);
        assert_eq!(
            response.default_model.as_deref(),
            Some("gemini-3-flash-preview")
        );
        assert_eq!(response.models[0].id, "gemini-3-flash-preview");
        assert!(response.models[0].recommended);
        assert_eq!(
            response.models[0]
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["minimal", "low", "medium", "high"]
        );
        assert_eq!(
            response
                .models
                .iter()
                .find(|model| model.id == "gemini-3.1-pro-preview")
                .expect("gemini pro preview model")
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "high"]
        );
    }

    #[test]
    fn reads_configured_gpt_codex_home() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "custom-gpt".to_owned(),
            json!({
                "provider_type": "gpt",
                "codex_home": "/tmp/test-codex-home",
                "base_url": "https://example.invalid/codex"
            }),
        );

        let gpt = configured_gpt_config(&config);

        assert_eq!(gpt.codex_home, "/tmp/test-codex-home");
        assert_eq!(gpt.base_url, "https://example.invalid/codex");
    }
}
