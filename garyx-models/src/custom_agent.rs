use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::{
    AgentProviderConfig, default_garyx_native_auth_source,
    default_garyx_native_max_tool_iterations, default_native_request_timeout,
};
use crate::provider::ProviderType;

fn is_default_max_tool_iterations(value: &u32) -> bool {
    *value == default_garyx_native_max_tool_iterations()
}

fn default_native_request_timeout_u32() -> u32 {
    default_native_request_timeout() as u32
}

fn is_default_request_timeout_u32(value: &u32) -> bool {
    *value == default_native_request_timeout_u32()
}

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
        alias = "authSource",
        skip_serializing_if = "String::is_empty"
    )]
    pub auth_source: String,
    #[serde(default, alias = "baseUrl", skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(default, alias = "codexHome", skip_serializing_if = "String::is_empty")]
    pub codex_home: String,
    #[serde(
        default = "default_garyx_native_max_tool_iterations",
        alias = "maxToolIterations",
        skip_serializing_if = "is_default_max_tool_iterations"
    )]
    pub max_tool_iterations: u32,
    #[serde(
        default = "default_native_request_timeout_u32",
        alias = "requestTimeoutSeconds",
        skip_serializing_if = "is_default_request_timeout_u32"
    )]
    pub request_timeout_seconds: u32,
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
    pub system_prompt: String,
    pub built_in: bool,
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
            auth_source: if self.auth_source.trim().is_empty() {
                default_garyx_native_auth_source()
            } else {
                self.auth_source.trim().to_owned()
            },
            base_url: self.base_url.trim().to_owned(),
            codex_home: self.codex_home.trim().to_owned(),
            max_tool_iterations: self.max_tool_iterations,
            request_timeout_seconds: f64::from(self.request_timeout_seconds),
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
            auth_source: String::new(),
            base_url: String::new(),
            codex_home: String::new(),
            max_tool_iterations: default_garyx_native_max_tool_iterations(),
            request_timeout_seconds: default_native_request_timeout_u32(),
            default_workspace_dir: None,
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_CLAUDE_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
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
            auth_source: String::new(),
            base_url: String::new(),
            codex_home: String::new(),
            max_tool_iterations: default_garyx_native_max_tool_iterations(),
            request_timeout_seconds: default_native_request_timeout_u32(),
            default_workspace_dir: None,
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_CODEX_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        CustomAgentProfile {
            agent_id: "traex".to_owned(),
            display_name: "Trae".to_owned(),
            provider_type: ProviderType::Traex,
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            provider_env: HashMap::new(),
            auth_source: String::new(),
            base_url: String::new(),
            codex_home: String::new(),
            max_tool_iterations: default_garyx_native_max_tool_iterations(),
            request_timeout_seconds: default_native_request_timeout_u32(),
            default_workspace_dir: None,
            // TRAE CLI is a Codex fork; reuse the Codex avatar until a dedicated
            // Trae asset is provided.
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_CODEX_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        CustomAgentProfile {
            agent_id: "gemini".to_owned(),
            display_name: "Gemini".to_owned(),
            provider_type: ProviderType::GeminiCli,
            model: "gemini-3-flash-preview".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            provider_env: HashMap::new(),
            auth_source: String::new(),
            base_url: String::new(),
            codex_home: String::new(),
            max_tool_iterations: default_garyx_native_max_tool_iterations(),
            request_timeout_seconds: default_native_request_timeout_u32(),
            default_workspace_dir: None,
            avatar_data_url: Some(builtin_avatar_data_url(BUILTIN_GEMINI_AVATAR_PNG)),
            system_prompt: String::new(),
            built_in: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    ]
}

pub fn is_builtin_provider_agent_id(agent_id: &str) -> bool {
    matches!(agent_id.trim(), "claude" | "codex" | "traex" | "gemini")
}

#[cfg(test)]
mod tests;
