use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::AgentProviderConfig;
use crate::provider::ProviderType;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CustomAgentProfile {
    pub agent_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub provider_type: ProviderType,
    #[serde(default)]
    pub model: String,
    #[serde(default, alias = "modelReasoningEffort")]
    pub model_reasoning_effort: String,
    #[serde(default, alias = "modelServiceTier")]
    pub model_service_tier: String,
    #[serde(
        default,
        alias = "env",
        alias = "providerEnv",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub provider_env: HashMap<String, String>,
    #[serde(
        default,
        alias = "defaultWorkspaceDir",
        alias = "workspace_dir",
        alias = "workspaceDir",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_workspace_dir: Option<String>,
    #[serde(
        default,
        alias = "avatarDataUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub avatar_data_url: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
    pub built_in: bool,
    #[serde(default = "crate::config::default_true")]
    pub enabled: bool,
    #[serde(default = "crate::config::default_true")]
    pub standalone: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl CustomAgentProfile {
    pub fn to_provider_config(&self) -> AgentProviderConfig {
        let default_model = self.model.trim().to_owned();
        AgentProviderConfig {
            provider_id: self.agent_id.clone(),
            provider_type: self.provider_type.as_slug().to_owned(),
            workspace_dir: self.default_workspace_dir.clone(),
            default_model: default_model.clone(),
            model: default_model,
            model_reasoning_effort: self.model_reasoning_effort.clone(),
            model_service_tier: self.model_service_tier.clone(),
            env: self.provider_env.clone(),
            ..Default::default()
        }
    }
}

const BUILTIN_CLAUDE_AVATAR_PNG: &[u8] =
    include_bytes!("../assets/builtin_agent_avatars/claude.png");
const BUILTIN_CODEX_AVATAR_PNG: &[u8] = include_bytes!("../assets/builtin_agent_avatars/codex.png");
const BUILTIN_GEMINI_AVATAR_PNG: &[u8] =
    include_bytes!("../assets/builtin_agent_avatars/gemini.png");

fn builtin_avatar_data_url(bytes: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    format!("data:image/png;base64,{}", BASE64.encode(bytes))
}

pub fn builtin_provider_agent_profiles() -> Vec<CustomAgentProfile> {
    let now = chrono::Utc::now().to_rfc3339();
    vec![
        CustomAgentProfile {
            agent_id: "claude".to_owned(),
            display_name: "Claude".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            provider_env: HashMap::new(),
            default_workspace_dir: None,
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_CLAUDE_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
            enabled: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        CustomAgentProfile {
            agent_id: "codex".to_owned(),
            display_name: "Codex".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            provider_env: HashMap::new(),
            default_workspace_dir: None,
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_CODEX_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
            enabled: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        CustomAgentProfile {
            agent_id: "traex".to_owned(),
            display_name: "Traex".to_owned(),
            provider_type: ProviderType::Traex,
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            provider_env: HashMap::new(),
            default_workspace_dir: None,
            // TRAE CLI is a Codex fork; reuse the Codex avatar until a dedicated
            // Trae asset is provided.
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_CODEX_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
            enabled: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        CustomAgentProfile {
            agent_id: "antigravity".to_owned(),
            display_name: "Antigravity".to_owned(),
            provider_type: ProviderType::AntigravityCli,
            model: crate::provider::default_antigravity_model(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            provider_env: HashMap::new(),
            default_workspace_dir: None,
            // Antigravity is a Google CLI surface; reuse the Gemini avatar
            // until a dedicated Antigravity asset is added.
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_GEMINI_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
            enabled: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    ]
}

pub fn is_builtin_provider_agent_id(agent_id: &str) -> bool {
    matches!(
        agent_id.trim(),
        "claude" | "codex" | "traex" | "antigravity"
    )
}

/// Whether a string is a valid POSIX-style environment variable name
/// (`^[A-Za-z_][A-Za-z0-9_]*$`).
///
/// Shared with the CLI to validate `--env KEY=VALUE` keys before sending an
/// agent env map to the gateway. Empty keys and keys containing `=`, spaces, or
/// other punctuation are rejected.
pub fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests;
