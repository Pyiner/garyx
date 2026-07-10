use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use garyx_models::config::AgentProviderConfig;
use garyx_models::provider::{
    AntigravityCliConfig, ClaudeCodeConfig, CodexAppServerConfig, GaryxNativeConfig,
    GeminiCliConfig, ProviderType, default_antigravity_model, default_claude_cli_mode,
};

use crate::antigravity_provider::AntigravityCliProvider;
use crate::claude_provider::ClaudeCliProvider;
use crate::codex_provider::CodexAgentProvider;
use crate::garyx_native_provider::GaryxNativeProvider;
use crate::gemini_provider::GeminiCliProvider;
use crate::provider_trait::{AgentLoopProvider, BridgeError};

/// Build a `ClaudeCodeConfig` from an agent runtime config.
fn build_claude_config(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> ClaudeCodeConfig {
    let claude_cli_path = agent_cfg.claude_cli_path.trim();
    ClaudeCodeConfig {
        workspace_dir: agent_cfg
            .workspace_dir
            .clone()
            .or_else(|| default_workspace.clone()),
        claude_cli_mode: agent_cfg.claude_cli_mode.clone(),
        claude_cli_path: (!claude_cli_path.is_empty()).then(|| claude_cli_path.to_owned()),
        permission_mode: agent_cfg.permission_mode.clone(),
        mcp_base_url: agent_cfg.mcp_base_url.clone(),
        default_model: agent_cfg.default_model.clone(),
        model_reasoning_effort: agent_cfg.model_reasoning_effort.clone(),
        max_turns: agent_cfg.max_turns,
        timeout_seconds: agent_cfg.timeout_seconds,
        env: agent_cfg.env.clone(),
        ..Default::default()
    }
}

/// Build a `CodexAppServerConfig` from an agent runtime config.
///
/// Shared by the Codex and Traex providers: TRAE CLI (`traex`) is forked from
/// Codex and speaks the identical app-server protocol, so both reuse the same
/// config and provider implementation and differ only by `provider_type` and
/// the launched `codex_bin` binary (`codex` vs `traex`).
fn build_codex_config(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
    provider_type: ProviderType,
    default_bin: &str,
) -> CodexAppServerConfig {
    CodexAppServerConfig {
        provider_type,
        codex_bin: default_bin.to_owned(),
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
        model_service_tier: agent_cfg.model_service_tier.clone(),
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

/// Build an Antigravity CLI config from an agent runtime config.
fn build_antigravity_config(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> AntigravityCliConfig {
    let default_model = if agent_cfg.default_model.trim().is_empty() {
        default_antigravity_model()
    } else {
        agent_cfg.default_model.clone()
    };
    let model = if agent_cfg.model.trim().is_empty() {
        default_model.clone()
    } else {
        agent_cfg.model.clone()
    };
    AntigravityCliConfig {
        workspace_dir: agent_cfg
            .workspace_dir
            .clone()
            .or_else(|| default_workspace.clone()),
        default_model,
        antigravity_bin: agent_cfg.antigravity_bin.clone(),
        antigravity_brain_root: agent_cfg.antigravity_brain_root.clone(),
        model,
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

pub(super) fn agent_provider_requires_dedicated_key(agent_cfg: &AgentProviderConfig) -> bool {
    let Some(provider_type) = ProviderType::from_slug(&agent_cfg.provider_type) else {
        return false;
    };
    if matches!(
        provider_type,
        ProviderType::Gpt | ProviderType::ClaudeLlm | ProviderType::GeminiLlm
    ) {
        return true;
    }

    let claude_cli_mode = agent_cfg.claude_cli_mode.trim();
    let custom_claude_cli_mode =
        !claude_cli_mode.is_empty() && claude_cli_mode != default_claude_cli_mode();

    custom_claude_cli_mode
        || !agent_cfg.claude_cli_path.trim().is_empty()
        || agent_cfg.experimental_api
        || !agent_cfg.gemini_bin.trim().is_empty()
        || !agent_cfg.antigravity_bin.trim().is_empty()
        || !agent_cfg.antigravity_brain_root.trim().is_empty()
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
    if !provider_id.is_empty() && agent_provider_requires_dedicated_key(agent_cfg) {
        return format!("agent:{provider_id}");
    }
    if provider_type == Some(ProviderType::ClaudeCode) {
        let mode = agent_cfg.claude_cli_mode.trim().to_ascii_lowercase();
        let path = agent_cfg.claude_cli_path.trim();
        if mode == "cctty" || !path.is_empty() {
            let mode = if mode.is_empty() {
                "native"
            } else {
                mode.as_str()
            };
            let mut hasher = DefaultHasher::new();
            path.hash(&mut hasher);
            return format!("claude_code:cli:{mode}:{:016x}", hasher.finish());
        }
    }
    provider_type
        .map(|provider_type| provider_type.as_slug().to_owned())
        .unwrap_or_else(|| "claude_code".to_owned())
}

/// Create and initialize a provider from `AgentProviderConfig`.
pub(super) async fn create_provider(
    agent_cfg: &AgentProviderConfig,
    default_workspace: &Option<String>,
) -> Result<Arc<dyn AgentLoopProvider>, BridgeError> {
    match ProviderType::from_slug(&agent_cfg.provider_type).unwrap_or(ProviderType::ClaudeCode) {
        ProviderType::ClaudeCode => {
            let config = build_claude_config(agent_cfg, default_workspace);
            let mut provider = ClaudeCliProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        ProviderType::CodexAppServer => {
            let config = build_codex_config(
                agent_cfg,
                default_workspace,
                ProviderType::CodexAppServer,
                "codex",
            );
            let mut provider = CodexAgentProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        ProviderType::Traex => {
            // TRAE CLI reuses the entire Codex app-server pipeline; only the
            // launched binary differs.
            let config =
                build_codex_config(agent_cfg, default_workspace, ProviderType::Traex, "traex");
            let mut provider = CodexAgentProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        ProviderType::GeminiCli => {
            let config = build_gemini_config(agent_cfg, default_workspace);
            let mut provider = GeminiCliProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        ProviderType::AntigravityCli => {
            let config = build_antigravity_config(agent_cfg, default_workspace);
            let mut provider = AntigravityCliProvider::new(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        ProviderType::Gpt => {
            let config = build_garyx_native_config(agent_cfg, default_workspace);
            let mut provider = GaryxNativeProvider::new_gpt(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        ProviderType::ClaudeLlm => {
            let config = build_garyx_native_config(agent_cfg, default_workspace);
            let mut provider = GaryxNativeProvider::new_claude(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
        ProviderType::GeminiLlm => {
            let config = build_garyx_native_config(agent_cfg, default_workspace);
            let mut provider = GaryxNativeProvider::new_gemini(config);
            provider.initialize().await?;
            Ok(Arc::new(provider))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_claude_config_carries_default_model_reasoning_and_env() {
        let agent_cfg = AgentProviderConfig {
            provider_type: ProviderType::ClaudeCode.as_slug().to_owned(),
            default_model: "claude-opus-4-8".to_owned(),
            model_reasoning_effort: "max".to_owned(),
            env: std::collections::HashMap::from([(
                "ANTHROPIC_BASE_URL".to_owned(),
                "http://127.0.0.1:15721".to_owned(),
            )]),
            ..Default::default()
        };

        let config = build_claude_config(&agent_cfg, &None);

        assert_eq!(config.default_model, "claude-opus-4-8");
        assert_eq!(config.model_reasoning_effort, "max");
        assert_eq!(
            config.env.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("http://127.0.0.1:15721")
        );
    }

    #[test]
    fn compute_provider_key_ignores_env_only_custom_claude_agent_id() {
        let agent_cfg = AgentProviderConfig {
            provider_id: "super-junior".to_owned(),
            provider_type: ProviderType::ClaudeCode.as_slug().to_owned(),
            env: std::collections::HashMap::from([(
                "ANTHROPIC_BASE_URL".to_owned(),
                "http://127.0.0.1:15721".to_owned(),
            )]),
            ..Default::default()
        };

        assert_eq!(compute_provider_key(&agent_cfg, &None), "claude_code");
    }

    #[test]
    fn compute_provider_key_ignores_model_only_custom_claude_agent_id() {
        let agent_cfg = AgentProviderConfig {
            provider_id: "model-only-claude".to_owned(),
            provider_type: ProviderType::ClaudeCode.as_slug().to_owned(),
            default_model: "claude-opus-4-8".to_owned(),
            ..Default::default()
        };

        assert_eq!(compute_provider_key(&agent_cfg, &None), "claude_code");
    }

    #[test]
    fn compute_provider_key_keeps_default_claude_code_key_stable() {
        let agent_cfg = AgentProviderConfig {
            provider_type: ProviderType::ClaudeCode.as_slug().to_owned(),
            ..Default::default()
        };

        assert_eq!(compute_provider_key(&agent_cfg, &None), "claude_code");
    }

    #[test]
    fn compute_provider_key_changes_for_claude_cli_launcher_config() {
        let cctty_cfg = AgentProviderConfig {
            provider_type: ProviderType::ClaudeCode.as_slug().to_owned(),
            claude_cli_mode: "cctty".to_owned(),
            ..Default::default()
        };
        let path_cfg = AgentProviderConfig {
            provider_type: ProviderType::ClaudeCode.as_slug().to_owned(),
            claude_cli_path: "/opt/garyx/bin/custom-cctty".to_owned(),
            ..Default::default()
        };

        let cctty_key = compute_provider_key(&cctty_cfg, &None);
        let path_key = compute_provider_key(&path_cfg, &None);
        assert!(cctty_key.starts_with("claude_code:cli:cctty:"));
        assert!(path_key.starts_with("claude_code:cli:native:"));
        assert_ne!(cctty_key, path_key);
    }
}
