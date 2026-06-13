use std::collections::HashSet;
use std::process::Stdio;
use std::time::Duration;

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
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct GeminiModelDiscovery {
    models: Vec<ProviderModelOption>,
    default_model: Option<String>,
}

struct GptModelDiscovery {
    models: Vec<ProviderModelOption>,
    default_model: Option<String>,
    reasoning_efforts: Vec<ProviderReasoningEffortOption>,
    service_tiers: Vec<ProviderModelOption>,
    source: &'static str,
    error: Option<String>,
}

pub(crate) async fn list_provider_models(
    config: &GaryxConfig,
    provider_type: ProviderType,
) -> ProviderModelsResponse {
    match provider_type {
        ProviderType::GeminiCli => {
            let configured_default = configured_default_model(
                config,
                ProviderType::GeminiCli,
                &["gemini", "gemini_cli"],
            );
            match fetch_gemini_acp_models(config).await {
                Ok(discovery) if !discovery.models.is_empty() => ProviderModelsResponse {
                    provider_type,
                    supports_model_selection: true,
                    models: discovery.models,
                    supports_reasoning_effort_selection: false,
                    reasoning_efforts: Vec::new(),
                    supports_service_tier_selection: false,
                    service_tiers: Vec::new(),
                    default_model: configured_default.or(discovery.default_model),
                    source: "gemini_acp",
                    error: None,
                },
                Ok(_) => unsupported(
                    provider_type,
                    "gemini_acp",
                    Some("local Gemini ACP returned no models".to_owned()),
                ),
                Err(error) => unsupported(provider_type, "gemini_acp", Some(error)),
            }
        }
        ProviderType::ClaudeCode => {
            // The CLI's actual default model is account/plan dependent and not
            // statically knowable unless the gateway config pins one. Without
            // a chosen model, only the levels every model supports are offered.
            let models = claude_code_models();
            let reasoning_efforts = common_reasoning_efforts(&models);
            ProviderModelsResponse {
                provider_type,
                supports_model_selection: true,
                supports_reasoning_effort_selection: !reasoning_efforts.is_empty(),
                reasoning_efforts,
                models,
                supports_service_tier_selection: false,
                service_tiers: Vec::new(),
                default_model: configured_default_model(
                    config,
                    ProviderType::ClaudeCode,
                    &["claude", "claude_code", "claude_tty"],
                ),
                source: "claude_code_builtin",
                error: None,
            }
        }
        ProviderType::CodexAppServer => {
            let discovery = apply_default_model_to_gpt_discovery(
                gpt_builtin_models(None),
                configured_default_model(
                    config,
                    ProviderType::CodexAppServer,
                    &["codex", "codex_app_server"],
                ),
            );
            ProviderModelsResponse {
                provider_type,
                supports_model_selection: true,
                models: discovery.models,
                supports_reasoning_effort_selection: true,
                reasoning_efforts: discovery.reasoning_efforts,
                supports_service_tier_selection: !discovery.service_tiers.is_empty(),
                service_tiers: discovery.service_tiers,
                default_model: discovery.default_model,
                source: discovery.source,
                error: discovery.error,
            }
        }
        ProviderType::Gpt => match fetch_gpt_codex_models(config).await {
            Ok(discovery) if !discovery.models.is_empty() => {
                let discovery = apply_default_model_to_gpt_discovery(
                    discovery,
                    configured_default_model(
                        config,
                        ProviderType::Gpt,
                        &["gpt", "openai", "garyx", "garyx_native", "native"],
                    ),
                );
                ProviderModelsResponse {
                    provider_type,
                    supports_model_selection: true,
                    models: discovery.models,
                    supports_reasoning_effort_selection: true,
                    reasoning_efforts: discovery.reasoning_efforts,
                    supports_service_tier_selection: !discovery.service_tiers.is_empty(),
                    service_tiers: discovery.service_tiers,
                    default_model: discovery.default_model,
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
                let discovery = apply_default_model_to_gpt_discovery(
                    gpt_builtin_models(Some(error)),
                    configured_default_model(
                        config,
                        ProviderType::Gpt,
                        &["gpt", "openai", "garyx", "garyx_native", "native"],
                    ),
                );
                ProviderModelsResponse {
                    provider_type,
                    supports_model_selection: true,
                    models: discovery.models,
                    supports_reasoning_effort_selection: true,
                    reasoning_efforts: discovery.reasoning_efforts,
                    supports_service_tier_selection: !discovery.service_tiers.is_empty(),
                    service_tiers: discovery.service_tiers,
                    default_model: discovery.default_model,
                    source: discovery.source,
                    error: discovery.error,
                }
            }
        },
        ProviderType::ClaudeLlm => {
            let default_model = configured_default_model(
                config,
                ProviderType::ClaudeLlm,
                &["anthropic", "claude_llm"],
            )
            .unwrap_or_else(|| "claude-sonnet-4-6".to_owned());
            native_model_catalog_response(
                provider_type,
                "native_builtin",
                native_claude_models(),
                &default_model,
            )
        }
        ProviderType::GeminiLlm => {
            let default_model = configured_default_model(
                config,
                ProviderType::GeminiLlm,
                &["google", "gemini_llm"],
            )
            .unwrap_or_else(|| "gemini-3-flash-preview".to_owned());
            native_model_catalog_response(
                provider_type,
                "native_builtin",
                native_gemini_models(),
                &default_model,
            )
        }
        ProviderType::AgentTeam => unsupported(provider_type, "provider", None),
    }
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
        source,
        error,
    }
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
    for value in config.agents.values() {
        if let Ok(mut agent_config) = serde_json::from_value::<AgentProviderConfig>(value.clone())
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
) -> ProviderModelsResponse {
    let reasoning_efforts = models
        .iter()
        .find(|model| model.id == default_model)
        .or_else(|| models.first())
        .map(|model| model.supported_reasoning_efforts.clone())
        .unwrap_or_default();
    ProviderModelsResponse {
        provider_type,
        supports_model_selection: true,
        models,
        supports_reasoning_effort_selection: !reasoning_efforts.is_empty(),
        reasoning_efforts,
        supports_service_tier_selection: false,
        service_tiers: Vec::new(),
        default_model: Some(default_model.to_owned()),
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
    async fn codex_app_server_model_catalog_supports_selection_and_reasoning() {
        let response =
            list_provider_models(&GaryxConfig::default(), ProviderType::CodexAppServer).await;

        assert_eq!(response.provider_type, ProviderType::CodexAppServer);
        assert!(response.supports_model_selection);
        assert!(response.supports_reasoning_effort_selection);
        assert_eq!(response.source, "codex_builtin");
        assert!(response.default_model.is_some());
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
