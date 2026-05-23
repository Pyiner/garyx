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

const BUILTIN_CLAUDE_AVATAR_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAACKElEQVR42s2Xy2sTURTGf/emD4kaW2lBsU3VxhGbDkalkIKvFCULN4aAXWmp0OpfIIgi6l/gSgVxJSioCx8bpVEsghJDK2pSRU2ysNZaLK0hrZGWjAtBrdR0JvOoZzVw55zvu2fuufN9AoNx6mCHVmr97JWHwkg9YQWoGTLSLnC9ucIOYCPdkE6Al6opnQAvVVs6Bf4vDMkih3Ry9/NhiXLAvUor7eEoXkXFvcxDYTrPcPo18b5bvH+VMDQZFUbZb9/XyZ4DPQjxe6rcy1egBIIogSCxG5d5fPeq7nqGCPjUNvZ29gLw7kWcvuuXmBj7ROMGP/t7juGpraMj2s1Qop/x0Y/6rmIj7T984hxNG1XGhrNcONlLsVj8tdbcuo1QpIvEgzuknvUzOztjbQcqq6pp9LUAkIw/mgMOkE4OkE4OlD8FC4VnZR3S5QJg8stn68dwwW8lXH88C+cJ5L+Oo2k/j0tN/SrnCRSmpxjJvgXA37YTKeemNjRv4siZ82zZEaaisko3AdeuzetO6315KjeJGgyx1FPLaq+P0Q8ZZr4XaFJUokePU79mLUogyMsnMb7lc9aPIcDuyCFCka551zRN4/61izy9d9OYJDNKYr1/K+3hKA2+FqqXuMnnJhjJvCEeu0126Lm9VzFAJjVIJjXo7CG09XdsVEpbqRFluXreKoH6/ygip7rwN4Y0a63MegNphb8zY0yEndZMz2akFQbTTK5YbHv+A82Ryy/aLtp+AAAAAElFTkSuQmCC";
const BUILTIN_CODEX_AVATAR_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAACYklEQVR42s2X3UuTcRTHP8/DFouRduG0QDIo60K7iU3oD2je6CLzZdJeRFC6iqhcRle9EZQvTVlvTp11UYiVuVC7CszILRdC6CLobkm4IC8GefOwLrLFxM1tz/bMc/k7v9/ve94453wF0hTdntJoMn34R0hI5z8hG6ByjBFzBZ7qWyEXwOlEQ1QCPNmfohLgyf4WlQJPhCGSZxGV9H4zLFFp8I1GyE7B7MxbHB0X5KcgE9ldWEj5wQP4/R8zNyDV8O9Qq6k/Vcekdzx2ZjDoiUajzAc+UV93EmdvN5UVFWmlQbXVpZKSYuw2K3arBZ2uCEmSYroqg56lYJBIJIJmp4ZmcyPN5kbmfH4G3ENMTk3H3d9MEhpQVraPTkcHptoa1GoVweAXhj0jTHhf/zegyoDP9zf8o6NjrKyEOWGqodpoZHDgAd+Xlxn2PKav35V4FiRKgeV0Mz1dt/m1ukrn5Su8HJ+I06vVKr59DXL23HnGX3njdAUFu7hx7SrmpgYkSWJv6f70i3Dm3Sxjz1+g1Wp5eN/F3PsZLjkucvhQOQBHKivRaDT4/fOxGqk2Hueeq4+FgB9zUwOhUIjrN28ln4ZbFaFOV0SL3YbdaqG4WBfz6Ex7G+1trRw1HIuLGMCHOR8D7iGmpt9kXgOxMRr+yZ2uHpzOfkymWlpbbOv51+Nb9x5g7fcaT5+N8sg9yOLiUuobUaZd8PNCgO7eu3hGnshqZEK+WvG/JWX7TMO8GpDuKp3NHVHMdJ/P1oK6vWpAiShsxBDlUiu53EDMBr+TQ0yEXFKzVJwRs0Ew5bwV8k3P/wDare9iAccUEAAAAABJRU5ErkJggg==";
const BUILTIN_GEMINI_AVATAR_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAACIklEQVR42s2XQUgUYRTHf9+3M2PuuusOtgvpwQQhECSCCHO7RJBI1KFbCLHkLeiQRkS3OhVEQrcgjDpHHsJu0SFcuglRQZQWlWzZ4pprmznjToeNFMv1+3ZnR99tmHnv///em/e+9xdoWu+Zd16l95nRTqETT/gBWgsZWS9wVV9RD2CdbMggwCvFlEGAV4otgwLfCEOyxSaDPP3/sEQ14O27TE4djbO/q5GWZoNlxyM37/L8ZZEHTxbI5hzlzjB02ff3Rrl4OoFlrnaVZQqawha7Wy2OHYoxNJLl9fSSXglUrKujgUvpMvj0zDLnb2bpO/eeE0MfuD+eByAalgwP7FSOaeikP33cxggJvi+ucPbaDIViCYBF4PbDOeLREKYhGHu6oPwvKJfANAQHuyMAjE8U/oKvtev3vlXfBZtZwjaQf77+9MXxrSOUM+CtKVSptPqQtA3GbrT/W64rn3n78Zd/GcjNuzhuGbijzfJ/EG1mjuuReVEEoK8nSixSdp3Nu6QGp0gNTjE8kq0fAYC7j/I4rocdC3HrQiv79jRimYKW5hCpvREG+uPaBITuFDxyoInL6QQ7Gjbm/mzyB1fvzFJcKqmtZLok2pImJw/H6OkOk7QNpICvcytMvvnJ44kCrxSnYGa0U4igL6L1BLbHday7Svu5I8pq93m/FtTtsxEFlYX1GLJWaVWrNpB+6LtahImopzRTOYz0Q2DW4iu2Wp7/BqspyjyJ+p+sAAAAAElFTkSuQmCC";

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
            avatar_data_url: Some(BUILTIN_CLAUDE_AVATAR_DATA_URL.to_owned()),
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
            avatar_data_url: Some(BUILTIN_CODEX_AVATAR_DATA_URL.to_owned()),
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
            avatar_data_url: Some(BUILTIN_GEMINI_AVATAR_DATA_URL.to_owned()),
            system_prompt: String::new(),
            built_in: true,
            standalone: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    ]
}

pub fn is_builtin_provider_agent_id(agent_id: &str) -> bool {
    matches!(agent_id.trim(), "claude" | "codex" | "gemini")
}

#[cfg(test)]
mod tests;
