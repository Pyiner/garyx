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
    // Single-cell semantics: `metadata.model` (plus effort / tier) is the
    // thread's one model cell — "what this thread actually runs". Legacy
    // stored threads may still carry the old dual-track `*_override` keys;
    // reads coalesce(legacy override, cell) until write paths migrate them.
    let legacy_model_override = thread_metadata_string(thread_value, MODEL_OVERRIDE_METADATA_KEY);
    let legacy_reasoning_effort_override =
        thread_metadata_string(thread_value, MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY);
    let legacy_service_tier_override =
        thread_metadata_string(thread_value, MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY);
    let cell_model = thread_metadata_string(thread_value, MODEL_METADATA_KEY);
    let cell_reasoning_effort =
        thread_metadata_string(thread_value, MODEL_REASONING_EFFORT_METADATA_KEY);
    let cell_service_tier = thread_metadata_string(thread_value, MODEL_SERVICE_TIER_METADATA_KEY);
    let agent_model = trimmed_json_string(agent_metadata.get(MODEL_METADATA_KEY));
    let agent_reasoning_effort =
        trimmed_json_string(agent_metadata.get(MODEL_REASONING_EFFORT_METADATA_KEY));
    let agent_service_tier =
        trimmed_json_string(agent_metadata.get(MODEL_SERVICE_TIER_METADATA_KEY));
    let selected_model = legacy_model_override.or(cell_model);
    let selected_reasoning_effort = legacy_reasoning_effort_override.or(cell_reasoning_effort);
    let selected_service_tier = legacy_service_tier_override.or(cell_service_tier);
    let model = selected_model
        .clone()
        .or(agent_model)
        .or(provider_default_model)
        .or(provider_catalog_default.model);
    let reasoning_effort = selected_reasoning_effort
        .clone()
        .or(agent_reasoning_effort)
        .or(provider_default_reasoning_effort)
        .or(provider_catalog_default.reasoning_effort);
    let service_tier = selected_service_tier
        .clone()
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
        // Kept for client compatibility: desktop (`modelOverride` ->
        // composer selected state) and mobile decode these fields. Under
        // single-cell semantics they report the thread's own selection —
        // coalesce(legacy override, cell) — not just the legacy override key.
        "model_override": selected_model,
        "model_reasoning_effort_override": selected_reasoning_effort,
        "model_service_tier_override": selected_service_tier,
        "sdk_session_id": sdk_session_id,
        "active_run": Value::Null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::AppStateBuilder;
    use garyx_models::config::GaryxConfig;
    use serde_json::json;

    /// Single-cell contract (guard): thread `metadata.model` (plus effort /
    /// tier) is THE model cell — "what this thread actually runs". A thread
    /// whose cell was pinned at first run keeps that model even after the
    /// provider default changes on disk; changing the global default is only
    /// expected to affect new threads and threads with an empty cell.
    ///
    /// Target priority: cell (`metadata.model`, legacy `model_override`
    /// coalesced in front) > agent model > provider `default_model` > catalog.
    #[tokio::test]
    async fn thread_runtime_summary_pins_thread_to_model_cell_over_provider_default() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "claude".to_owned(),
            json!({
                "provider_type": "claude_code",
                "default_model": "claude-fable-5",
                "model_reasoning_effort": "high",
                "model_service_tier": "priority"
            }),
        );
        let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(config)).build();

        // Thread that already ran once: the first run pinned the then-current
        // effective defaults into the cell. No legacy override present.
        let thread_value = json!({
            "thread_id": "thread-runtime-cell",
            "provider_type": "claude_code",
            "metadata": {
                "model": "claude-opus-4-8",
                "model_reasoning_effort": "low",
                "model_service_tier": "flex"
            }
        });

        let summary = build_thread_runtime_summary(&state, Some(&thread_value)).await;

        assert_eq!(
            summary["model"],
            json!("claude-opus-4-8"),
            "a filled model cell pins the thread; the provider default must not leak in"
        );
        assert_eq!(
            summary["model_reasoning_effort"],
            json!("low"),
            "a filled effort cell pins the thread; the provider default must not leak in"
        );
        assert_eq!(
            summary["model_service_tier"],
            json!("flex"),
            "a filled service-tier cell pins the thread; the provider default must not leak in"
        );
    }

    /// Single-cell contract (guard): with an empty cell the runtime resolves
    /// straight to the current provider default, which is what makes the
    /// hot-reload fix (Bug A) visible to new threads and cleared threads.
    #[tokio::test]
    async fn thread_runtime_summary_resolves_provider_default_when_cell_is_empty() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "claude".to_owned(),
            json!({
                "provider_type": "claude_code",
                "default_model": "claude-fable-5",
                "model_reasoning_effort": "high"
            }),
        );
        let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(config)).build();

        let thread_value = json!({
            "thread_id": "thread-runtime-empty-cell",
            "provider_type": "claude_code",
            "metadata": {}
        });

        let summary = build_thread_runtime_summary(&state, Some(&thread_value)).await;

        assert_eq!(summary["model"], json!("claude-fable-5"));
        assert_eq!(summary["model_reasoning_effort"], json!("high"));
    }

    /// Legacy compatibility (guard): stored threads may still carry the old
    /// dual-track `model_override`. Reads must coalesce(override, cell), so
    /// the legacy override keeps the highest priority until the write paths
    /// migrate it into the cell.
    #[tokio::test]
    async fn thread_runtime_summary_keeps_legacy_override_highest_priority() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "claude".to_owned(),
            json!({
                "provider_type": "claude_code",
                "default_model": "claude-fable-5"
            }),
        );
        let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(config)).build();

        let thread_value = json!({
            "thread_id": "thread-runtime-override",
            "provider_type": "claude_code",
            "metadata": {
                "model": "claude-opus-4-8",
                "model_override": "claude-haiku-4-6"
            }
        });

        let summary = build_thread_runtime_summary(&state, Some(&thread_value)).await;

        assert_eq!(summary["model"], json!("claude-haiku-4-6"));
        assert_eq!(summary["model_override"], json!("claude-haiku-4-6"));
    }
}
