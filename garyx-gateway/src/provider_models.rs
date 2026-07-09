use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use garyx_models::codex_models::{
    CodexModelPreset, CodexModelServiceTier, CodexModelsResponse, CodexReasoningEffort,
    CodexReasoningEffortPreset, available_codex_model_presets, codex_builtin_model_presets,
    effective_codex_models_client_version, models_endpoint, parse_codex_cli_version,
    resolve_codex_auth,
};
use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::{GaryxNativeConfig, ProviderType};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

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

mod app_server;
mod cache;
mod claude_code;
mod codex;
mod gemini_acp;
mod native;
mod parse;

use app_server::*;
use cache::*;
use claude_code::*;
use codex::*;
use gemini_acp::*;
use native::*;
use parse::*;

#[cfg(test)]
mod tests;
