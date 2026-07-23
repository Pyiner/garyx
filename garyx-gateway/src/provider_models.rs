use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use garyx_models::codex_models::{
    CodexModelPreset, CodexModelServiceTier, CodexReasoningEffort, CodexReasoningEffortPreset,
    codex_builtin_model_presets,
};
use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::ProviderType;
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

#[derive(Debug, Clone)]
struct ProviderModelDiscovery {
    models: Vec<ProviderModelOption>,
    default_model: Option<String>,
    reasoning_efforts: Vec<ProviderReasoningEffortOption>,
    service_tiers: Vec<ProviderModelOption>,
    source: &'static str,
    error: Option<String>,
}

type CodexModelDiscovery = ProviderModelDiscovery;

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
            builtin_model_catalog_response(
                provider_type,
                "antigravity_builtin",
                antigravity_models(),
                &default_model,
                configured_default_reasoning_effort(config, ProviderType::AntigravityCli, aliases),
            )
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
                            codex_builtin_models(Some(error))
                        })
                    }
                }
            };
            if let Some(default_model) =
                configured_default_model(config, provider_type.clone(), aliases)
            {
                discovery = apply_default_model_to_codex_discovery(discovery, Some(default_model));
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
        ProviderType::GrokBuild => {
            let aliases = &["grok", "grok_build", "grok-build"];
            let configured_model =
                configured_default_model(config, ProviderType::GrokBuild, aliases);
            let configured_reasoning =
                configured_default_reasoning_effort(config, ProviderType::GrokBuild, aliases);
            let discovery = match fresh_cached_discovery("grok_acp") {
                Some(discovery) => discovery,
                None => discover_or_fallback(
                    "grok_acp",
                    fetch_grok_models(config).await,
                    grok_unavailable_models,
                ),
            };
            let default_model = configured_model.or(discovery.default_model.clone());
            let reasoning_efforts =
                reasoning_efforts_for_default_model(&discovery.models, default_model.as_deref());
            ProviderModelsResponse {
                provider_type,
                supports_model_selection: !discovery.models.is_empty(),
                supports_reasoning_effort_selection: provider_supports_reasoning_effort_selection(
                    &discovery.models,
                ),
                models: discovery.models,
                reasoning_efforts,
                supports_service_tier_selection: false,
                service_tiers: Vec::new(),
                default_model,
                default_reasoning_effort: configured_reasoning,
                source: discovery.source,
                error: discovery.error,
            }
        }
    }
}

pub(crate) fn builtin_provider_catalog_default(
    provider_type: ProviderType,
) -> ProviderCatalogDefault {
    match provider_type {
        ProviderType::AntigravityCli => ProviderCatalogDefault {
            model: Some(garyx_models::provider::default_antigravity_model()),
            reasoning_effort: None,
            service_tier: None,
        },
        ProviderType::ClaudeCode
        | ProviderType::CodexAppServer
        | ProviderType::Traex
        | ProviderType::GrokBuild => ProviderCatalogDefault::default(),
    }
}

mod app_server;
mod cache;
mod catalog;
mod claude_code;
mod codex;
mod grok;
mod process_rpc;

use app_server::*;
use cache::*;
use catalog::*;
use claude_code::*;
use codex::*;
use grok::*;
use process_rpc::*;

#[cfg(test)]
mod tests;
