use std::collections::HashMap;
use std::sync::Arc;

use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::{
    MODEL_METADATA_KEY, MODEL_OVERRIDE_METADATA_KEY, MODEL_REASONING_EFFORT_METADATA_KEY,
    MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY, MODEL_SERVICE_TIER_METADATA_KEY,
    MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY, ProviderType,
};
use garyx_models::{agent_runtime_metadata, resolve_agent_reference};
use serde_json::{Value, json};

use crate::server::AppState;

fn provider_type_from_value(value: &Value) -> Option<ProviderType> {
    serde_json::from_value(value.clone()).ok()
}

pub(crate) fn provider_type_from_key(value: &str) -> Option<ProviderType> {
    let prefix = value.trim().split(':').next().unwrap_or_default();
    ProviderType::from_slug(prefix)
}

fn provider_type_value(thread_value: &Value) -> Option<ProviderType> {
    thread_value
        .get("provider_type")
        .and_then(provider_type_from_value)
}

pub(crate) fn provider_label(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "Claude",
        ProviderType::CodexAppServer => "Codex",
        ProviderType::Traex => "Traex",
        ProviderType::GeminiCli => "Gemini",
        ProviderType::AntigravityCli => "Antigravity",
        ProviderType::Gpt => "GPT",
        ProviderType::ClaudeLlm => "Claude",
        ProviderType::GeminiLlm => "Gemini",
        ProviderType::AgentTeam => "Team",
    }
}

fn trimmed_json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn thread_metadata_string(thread_value: &Value, key: &str) -> Option<String> {
    trimmed_json_string(
        thread_value
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get(key))
            .or_else(|| thread_value.get(key)),
    )
}

fn provider_default_config_keys(provider_type: &ProviderType) -> &'static [&'static str] {
    match provider_type {
        ProviderType::ClaudeCode => &["claude", "claude_code", "claude_tty"],
        ProviderType::CodexAppServer => &["codex", "codex_app_server"],
        ProviderType::Traex => &["traex", "trae", "trae_cli", "traecli"],
        ProviderType::GeminiCli => &["gemini", "gemini_cli"],
        ProviderType::AntigravityCli => &["antigravity", "agy", "antigravity_cli"],
        ProviderType::Gpt => &["gpt", "openai", "garyx", "garyx_native", "native"],
        ProviderType::ClaudeLlm => &["anthropic", "claude_llm"],
        ProviderType::GeminiLlm => &["google", "gemini_llm"],
        ProviderType::AgentTeam => &[],
    }
}

fn configured_provider_default_config(
    config: &GaryxConfig,
    provider_type: &ProviderType,
) -> Option<AgentProviderConfig> {
    for key in provider_default_config_keys(provider_type) {
        if let Some(value) = config.agents.get(*key)
            && let Ok(mut agent_cfg) = serde_json::from_value::<AgentProviderConfig>(value.clone())
            && ProviderType::from_slug(&agent_cfg.provider_type) == Some(provider_type.clone())
        {
            agent_cfg.provider_type = provider_type.as_slug().to_owned();
            return Some(agent_cfg);
        }
    }
    None
}

async fn current_agent_runtime_metadata(
    state: &Arc<AppState>,
    agent_id: &str,
) -> HashMap<String, Value> {
    let agents = state.ops.custom_agents.list_agents().await;
    let teams = state.ops.agent_teams.list_teams().await;
    resolve_agent_reference(agent_id, &agents, &teams)
        .map(|reference| agent_runtime_metadata(&reference))
        .unwrap_or_default()
}

pub(crate) async fn build_thread_runtime_summary(
    state: &Arc<AppState>,
    thread_value: Option<&Value>,
) -> Value {
    let Some(thread_value) = thread_value else {
        return Value::Null;
    };

    let provider_type = provider_type_value(thread_value).or_else(|| {
        thread_value
            .get("provider_key")
            .and_then(Value::as_str)
            .and_then(provider_type_from_key)
    });
    let agent_id = trimmed_json_string(thread_value.get("agent_id"));
    let agent_metadata = match agent_id.as_deref() {
        Some(agent_id) => current_agent_runtime_metadata(state, agent_id).await,
        None => HashMap::new(),
    };
    let provider_type = provider_type.or_else(|| {
        agent_metadata
            .get("requested_provider_type")
            .and_then(Value::as_str)
            .and_then(ProviderType::from_slug)
    });
    let provider_default_config = provider_type.as_ref().and_then(|value| {
        configured_provider_default_config(state.config_snapshot().as_ref(), value)
    });
    let provider_default_model = provider_default_config.as_ref().and_then(|config| {
        let default_model = config.default_model.trim();
        if !default_model.is_empty() {
            Some(default_model.to_owned())
        } else {
            let model = config.model.trim();
            (!model.is_empty()).then(|| model.to_owned())
        }
    });
    let provider_default_reasoning_effort = provider_default_config.as_ref().and_then(|config| {
        let value = config.model_reasoning_effort.trim();
        (!value.is_empty()).then(|| value.to_owned())
    });
    let provider_default_service_tier = provider_default_config.as_ref().and_then(|config| {
        let value = config.model_service_tier.trim();
        (!value.is_empty()).then(|| value.to_owned())
    });
    let provider_catalog_default = provider_type
        .clone()
        .map(crate::provider_models::builtin_provider_catalog_default)
        .unwrap_or_default();
    let model_override = thread_metadata_string(thread_value, MODEL_OVERRIDE_METADATA_KEY);
    let reasoning_effort_override =
        thread_metadata_string(thread_value, MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY);
    let service_tier_override =
        thread_metadata_string(thread_value, MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY);
    let snapshot_model = thread_metadata_string(thread_value, MODEL_METADATA_KEY);
    let snapshot_reasoning_effort =
        thread_metadata_string(thread_value, MODEL_REASONING_EFFORT_METADATA_KEY);
    let snapshot_service_tier =
        thread_metadata_string(thread_value, MODEL_SERVICE_TIER_METADATA_KEY);
    let agent_model = trimmed_json_string(agent_metadata.get(MODEL_METADATA_KEY));
    let agent_reasoning_effort =
        trimmed_json_string(agent_metadata.get(MODEL_REASONING_EFFORT_METADATA_KEY));
    let agent_service_tier =
        trimmed_json_string(agent_metadata.get(MODEL_SERVICE_TIER_METADATA_KEY));
    let model = model_override
        .clone()
        .or(snapshot_model)
        .or(agent_model)
        .or(provider_default_model)
        .or(provider_catalog_default.model);
    let reasoning_effort = reasoning_effort_override
        .clone()
        .or(snapshot_reasoning_effort)
        .or(agent_reasoning_effort)
        .or(provider_default_reasoning_effort)
        .or(provider_catalog_default.reasoning_effort);
    let service_tier = service_tier_override
        .clone()
        .or(snapshot_service_tier)
        .or(agent_service_tier)
        .or(provider_default_service_tier)
        .or(provider_catalog_default.service_tier);
    let sdk_session_id = trimmed_json_string(thread_value.get("sdk_session_id"));
    json!({
        "agent_id": agent_id,
        "provider_type": provider_type.as_ref().and_then(|value| serde_json::to_value(value).ok()).unwrap_or(Value::Null),
        "provider_label": provider_type.as_ref().map(provider_label).unwrap_or("-"),
        "model": model,
        "model_reasoning_effort": reasoning_effort,
        "model_service_tier": service_tier,
        "model_override": model_override,
        "model_reasoning_effort_override": reasoning_effort_override,
        "model_service_tier_override": service_tier_override,
        "sdk_session_id": sdk_session_id,
        "active_run": Value::Null,
    })
}
