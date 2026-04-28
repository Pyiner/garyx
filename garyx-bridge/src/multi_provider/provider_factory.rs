use std::sync::Arc;

use garyx_models::config::AgentProviderConfig;
use garyx_models::provider::{ClaudeCodeConfig, CodexAppServerConfig, GeminiCliConfig};

use crate::claude_provider::ClaudeCliProvider;
use crate::codex_provider::CodexAgentProvider;
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

/// Compute a stable provider key for the configured local provider type.
///
/// Garyx threads bind to a single provider type for their lifetime. Keep the
/// local provider key stable so thread affinity and persisted SDK session ids
/// don't drift when unrelated config fields change.
pub(super) fn compute_provider_key(
    agent_cfg: &AgentProviderConfig,
    _default_workspace: &Option<String>,
) -> String {
    match agent_cfg.provider_type.as_str() {
        "codex_app_server" => "codex_app_server".to_owned(),
        "gemini_cli" => "gemini_cli".to_owned(),
        _ => "claude_code".to_owned(),
    }
}

/// Create and initialize a provider from `AgentProviderConfig`.
pub(super) async fn create_provider(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> Result<Arc<dyn AgentLoopProvider>, BridgeError> {
    match agent_cfg.provider_type.as_str() {
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
        _ => {
            // Default to Claude Code
            let config = build_claude_config(agent_cfg, default_workspace);
            let mut provider = ClaudeCliProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
    }
}
