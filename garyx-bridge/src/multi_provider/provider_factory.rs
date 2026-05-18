use std::sync::Arc;

use garyx_models::config::AgentProviderConfig;
use garyx_models::provider::{
    ClaudeCodeConfig, CodexAppServerConfig, GaryxNativeConfig, GeminiCliConfig, ProviderType,
};

use crate::claude_provider::ClaudeCliProvider;
use crate::claude_tty_provider::ClaudeTtyProvider;
use crate::codex_provider::CodexAgentProvider;
use crate::garyx_native_provider::GaryxNativeProvider;
use crate::gemini_provider::GeminiCliProvider;
use crate::provider_trait::{AgentLoopProvider, BridgeError};

/// Build a `ClaudeCodeConfig` from an agent runtime config.
fn build_claude_config(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> ClaudeCodeConfig {
    ClaudeCodeConfig {
        workspace_dir: agent_cfg
            .workspace_dir
            .clone()
            .or_else(|| default_workspace.clone()),
        permission_mode: agent_cfg.permission_mode.clone(),
        mcp_base_url: agent_cfg.mcp_base_url.clone(),
        default_model: agent_cfg.default_model.clone(),
        max_turns: agent_cfg.max_turns,
        timeout_seconds: agent_cfg.timeout_seconds,
        env: agent_cfg.env.clone(),
        ..Default::default()
    }
}

/// Build a `CodexAppServerConfig` from an agent runtime config.
fn build_codex_config(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> CodexAppServerConfig {
    CodexAppServerConfig {
        workspace_dir: agent_cfg
            .workspace_dir
            .clone()
            .or_else(|| default_workspace.clone()),
        default_model: agent_cfg.default_model.clone(),
        mcp_base_url: agent_cfg.mcp_base_url.clone(),
        model: if agent_cfg.model.is_empty() {
            agent_cfg.default_model.clone()
        } else {
            agent_cfg.model.clone()
        },
        model_reasoning_effort: agent_cfg.model_reasoning_effort.clone(),
        max_turns: agent_cfg.max_turns,
        timeout_seconds: agent_cfg.timeout_seconds,
        experimental_api: agent_cfg.experimental_api,
        env: agent_cfg.env.clone(),
        ..Default::default()
    }
}

/// Build a `GeminiCliConfig` from an agent runtime config.
fn build_gemini_config(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> GeminiCliConfig {
    GeminiCliConfig {
        workspace_dir: agent_cfg
            .workspace_dir
            .clone()
            .or_else(|| default_workspace.clone()),
        default_model: agent_cfg.default_model.clone(),
        mcp_base_url: agent_cfg.mcp_base_url.clone(),
        gemini_bin: agent_cfg.gemini_bin.clone(),
        approval_mode: agent_cfg.approval_mode.clone(),
        model: if agent_cfg.model.is_empty() {
            agent_cfg.default_model.clone()
        } else {
            agent_cfg.model.clone()
        },
        max_turns: agent_cfg.max_turns,
        timeout_seconds: agent_cfg.timeout_seconds,
        env: agent_cfg.env.clone(),
        ..Default::default()
    }
}

/// Build the native-loop GPT backend config from an agent runtime config.
fn build_garyx_native_config(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> GaryxNativeConfig {
    let provider_type = garyx_native_provider_type(agent_cfg);
    let default_model = if agent_cfg.default_model.trim().is_empty() {
        garyx_native_default_model(&provider_type).to_owned()
    } else {
        agent_cfg.default_model.clone()
    };
    GaryxNativeConfig {
        provider_type,
        workspace_dir: agent_cfg
            .workspace_dir
            .clone()
            .or_else(|| default_workspace.clone()),
        default_model,
        model: if agent_cfg.model.is_empty() {
            agent_cfg.default_model.clone()
        } else {
            agent_cfg.model.clone()
        },
        model_reasoning_effort: agent_cfg.model_reasoning_effort.clone(),
        model_service_tier: agent_cfg.model_service_tier.clone(),
        max_turns: agent_cfg.max_turns,
        timeout_seconds: agent_cfg.timeout_seconds,
        env: agent_cfg.env.clone(),
        auth_source: agent_cfg.auth_source.clone(),
        base_url: agent_cfg.base_url.clone(),
        codex_home: agent_cfg.codex_home.clone(),
        max_tool_iterations: agent_cfg.max_tool_iterations,
        request_timeout_seconds: agent_cfg.request_timeout_seconds,
        ..Default::default()
    }
}

fn garyx_native_provider_type(agent_cfg: &AgentProviderConfig) -> ProviderType {
    match ProviderType::from_slug(&agent_cfg.provider_type) {
        Some(ProviderType::ClaudeLlm) => ProviderType::ClaudeLlm,
        Some(ProviderType::GeminiLlm) => ProviderType::GeminiLlm,
        _ => ProviderType::Gpt,
    }
}

fn garyx_native_default_model(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeLlm => "claude-sonnet-4-6",
        ProviderType::GeminiLlm => "gemini-3-flash-preview",
        _ => "gpt-5.5",
    }
}

/// Compute a stable provider key for the configured local provider type.
///
/// Garyx threads bind to a single provider type for their lifetime. Keep the
/// local provider key stable so thread affinity and persisted SDK session ids
/// don't drift when unrelated config fields change.
pub(super) fn compute_provider_key(
    agent_cfg: &AgentProviderConfig,
    _default_workspace: &Option<String>,
) -> String {
    let provider_type = ProviderType::from_slug(&agent_cfg.provider_type);
    let provider_id = agent_cfg.provider_id.trim();
    if !provider_id.is_empty()
        && matches!(
            provider_type,
            Some(ProviderType::Gpt | ProviderType::ClaudeLlm | ProviderType::GeminiLlm)
        )
    {
        return format!("agent:{provider_id}");
    }
    match agent_cfg.provider_type.as_str() {
        "claude_tty" => "claude_tty".to_owned(),
        "codex_app_server" => "codex_app_server".to_owned(),
        "gemini_cli" => "gemini_cli".to_owned(),
        "gpt" | "openai" | "openai_gpt" | "garyx_native" | "garyx" | "native" => "gpt".to_owned(),
        "claude_llm" | "anthropic" | "claude_model" => "claude_llm".to_owned(),
        "gemini_llm" | "google" | "google_gemini" | "gemini_model" => "gemini_llm".to_owned(),
        _ => "claude_code".to_owned(),
    }
}

/// Create and initialize a provider from `AgentProviderConfig`.
pub(super) async fn create_provider(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> Result<Arc<dyn AgentLoopProvider>, BridgeError> {
    match agent_cfg.provider_type.as_str() {
        "claude_tty" => {
            let config = build_claude_config(agent_cfg, default_workspace);
            let mut provider = ClaudeTtyProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        "codex_app_server" => {
            let config = build_codex_config(agent_cfg, default_workspace);
            let mut provider = CodexAgentProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        "gemini_cli" => {
            let config = build_gemini_config(agent_cfg, default_workspace);
            let mut provider = GeminiCliProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        "gpt" | "openai" | "openai_gpt" | "garyx_native" | "garyx" | "native" => {
            let config = build_garyx_native_config(agent_cfg, default_workspace);
            let mut provider = GaryxNativeProvider::new_gpt(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        "claude_llm" | "anthropic" | "claude_model" => {
            let config = build_garyx_native_config(agent_cfg, default_workspace);
            let mut provider = GaryxNativeProvider::new_claude(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        "gemini_llm" | "google" | "google_gemini" | "gemini_model" => {
            let config = build_garyx_native_config(agent_cfg, default_workspace);
            let mut provider = GaryxNativeProvider::new_gemini(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        _ => {
            // Default to Claude Code
            let config = build_claude_config(agent_cfg, default_workspace);
            let mut provider = ClaudeCliProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
    }
}
