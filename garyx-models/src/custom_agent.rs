use serde::{Deserialize, Serialize};

use crate::provider::ProviderType;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CustomAgentProfile {
    pub agent_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub provider_type: ProviderType,
    #[serde(default)]
    pub model: String,
    pub system_prompt: String,
    pub built_in: bool,
    #[serde(default = "crate::config::default_true")]
    pub standalone: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub fn builtin_provider_agent_profiles() -> Vec<CustomAgentProfile> {
    let now = chrono::Utc::now().to_rfc3339();
    vec![
        CustomAgentProfile {
            agent_id: "claude".to_owned(),
            display_name: "Claude".to_owned(),
            provider_type: ProviderType::ClaudeCode,
            model: String::new(),
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
            system_prompt: String::new(),
            built_in: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now,
        },
    ]
}

#[cfg(test)]
mod tests;
