use std::collections::HashMap;
use std::sync::Arc;

use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::{
    MODEL_METADATA_KEY, MODEL_OVERRIDE_METADATA_KEY, MODEL_REASONING_EFFORT_METADATA_KEY,
    MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY, MODEL_SERVICE_TIER_METADATA_KEY,
    MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY, ProviderType,
};
use garyx_models::{CustomAgentProfile, agent_runtime_metadata, resolve_agent_reference};
use serde_json::{Value, json};

use crate::garyx_db::ThreadMetaRecord;
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
        ProviderType::AntigravityCli => "Antigravity",
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
        ProviderType::AntigravityCli => &["antigravity", "agy", "antigravity_cli"],
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

/// Agent catalog snapshot for one request. List routes resolve it once
/// and build every row's runtime summary against it instead of cloning the
/// full catalogs per thread.
pub(crate) struct AgentCatalogSnapshot {
    agents: Vec<CustomAgentProfile>,
}

impl AgentCatalogSnapshot {
    pub(crate) async fn load(state: &Arc<AppState>) -> Self {
        Self {
            agents: state.ops.custom_agents.list_agents().await,
        }
    }

    fn agent_runtime_metadata(&self, agent_id: &str) -> HashMap<String, Value> {
        resolve_agent_reference(agent_id, &self.agents)
            .map(|reference| agent_runtime_metadata(&reference))
            .unwrap_or_default()
    }
}

pub(crate) async fn build_thread_runtime_summary(
    state: &Arc<AppState>,
    thread_value: Option<&Value>,
) -> Value {
    if thread_value.is_none() {
        return Value::Null;
    }
    let catalog = AgentCatalogSnapshot::load(state).await;
    build_thread_runtime_summary_with_catalog(state, thread_value, &catalog)
}

/// The thread-owned inputs of a runtime summary. Extracted from the full
/// thread record on the slow path, or read back from the `thread_meta`
/// projection columns on the list fast path — both must resolve the same
/// values, so the extraction lives here next to the summary builder.
pub(crate) struct ThreadRuntimeSelection {
    pub provider_type: Option<ProviderType>,
    pub agent_id: Option<String>,
    pub selected_model: Option<String>,
    pub selected_reasoning_effort: Option<String>,
    pub selected_service_tier: Option<String>,
    pub sdk_session_id: Option<String>,
}

/// Single-cell selection: coalesce(legacy `*_override` key, `metadata.*`
/// cell) for model / reasoning effort / service tier. Shared by the live
/// summary path and the thread_meta projection writer.
pub(crate) fn selected_model_cells_from_thread_value(
    thread_value: &Value,
) -> (Option<String>, Option<String>, Option<String>) {
    let legacy_model_override = thread_metadata_string(thread_value, MODEL_OVERRIDE_METADATA_KEY);
    let legacy_reasoning_effort_override =
        thread_metadata_string(thread_value, MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY);
    let legacy_service_tier_override =
        thread_metadata_string(thread_value, MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY);
    let cell_model = thread_metadata_string(thread_value, MODEL_METADATA_KEY);
    let cell_reasoning_effort =
        thread_metadata_string(thread_value, MODEL_REASONING_EFFORT_METADATA_KEY);
    let cell_service_tier = thread_metadata_string(thread_value, MODEL_SERVICE_TIER_METADATA_KEY);
    (
        legacy_model_override.or(cell_model),
        legacy_reasoning_effort_override.or(cell_reasoning_effort),
        legacy_service_tier_override.or(cell_service_tier),
    )
}

pub(crate) fn thread_runtime_selection_from_thread_value(
    thread_value: &Value,
) -> ThreadRuntimeSelection {
    let provider_type = provider_type_value(thread_value).or_else(|| {
        thread_value
            .get("provider_key")
            .and_then(Value::as_str)
            .and_then(provider_type_from_key)
    });
    let (selected_model, selected_reasoning_effort, selected_service_tier) =
        selected_model_cells_from_thread_value(thread_value);
    ThreadRuntimeSelection {
        provider_type,
        agent_id: trimmed_json_string(thread_value.get("agent_id")),
        selected_model,
        selected_reasoning_effort,
        selected_service_tier,
        sdk_session_id: trimmed_json_string(thread_value.get("sdk_session_id")),
    }
}

pub(crate) fn build_thread_runtime_summary_with_catalog(
    state: &Arc<AppState>,
    thread_value: Option<&Value>,
    catalog: &AgentCatalogSnapshot,
) -> Value {
    let Some(thread_value) = thread_value else {
        return Value::Null;
    };
    build_thread_runtime_summary_from_selection(
        state,
        thread_runtime_selection_from_thread_value(thread_value),
        catalog,
    )
}

/// List fast path: build the runtime summary from the `thread_meta`
/// projection row alone — no per-row thread_store read. The projection
/// writer persists the same selection the live path extracts, so both
/// paths resolve identical summaries (guarded by a parity test).
pub(crate) fn build_thread_runtime_summary_from_meta(
    state: &Arc<AppState>,
    record: &ThreadMetaRecord,
    catalog: &AgentCatalogSnapshot,
) -> Value {
    let provider_type = record
        .provider_type
        .as_deref()
        .and_then(ProviderType::from_slug)
        .or_else(|| {
            record
                .provider_key
                .as_deref()
                .and_then(provider_type_from_key)
        });
    build_thread_runtime_summary_from_selection(
        state,
        ThreadRuntimeSelection {
            provider_type,
            agent_id: record.agent_id.clone(),
            selected_model: record.selected_model.clone(),
            selected_reasoning_effort: record.selected_model_reasoning_effort.clone(),
            selected_service_tier: record.selected_model_service_tier.clone(),
            sdk_session_id: record.sdk_session_id.clone(),
        },
        catalog,
    )
}

pub(crate) fn build_thread_runtime_summary_from_selection(
    state: &Arc<AppState>,
    selection: ThreadRuntimeSelection,
    catalog: &AgentCatalogSnapshot,
) -> Value {
    let ThreadRuntimeSelection {
        provider_type,
        agent_id,
        selected_model,
        selected_reasoning_effort,
        selected_service_tier,
        sdk_session_id,
    } = selection;
    let agent_metadata = match agent_id.as_deref() {
        Some(agent_id) => catalog.agent_runtime_metadata(agent_id),
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
    // Single-cell semantics: `selected_*` already carries
    // coalesce(legacy override, metadata cell) — computed by
    // selected_model_cells_from_thread_value on the live path or read back
    // from the thread_meta projection columns on the list path.
    let agent_model = trimmed_json_string(agent_metadata.get(MODEL_METADATA_KEY));
    let agent_reasoning_effort =
        trimmed_json_string(agent_metadata.get(MODEL_REASONING_EFFORT_METADATA_KEY));
    let agent_service_tier =
        trimmed_json_string(agent_metadata.get(MODEL_SERVICE_TIER_METADATA_KEY));
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
    /// Parity oracle: the projection fast path (thread_meta columns ->
    /// build_thread_runtime_summary_from_meta) must resolve the exact same
    /// summary JSON as the live path (full thread record ->
    /// build_thread_runtime_summary_with_catalog) for every field shape the
    /// selection carries: metadata cells, legacy overrides, provider_key
    /// fallback, sdk_session_id, and agent-metadata provider fallback.
    #[tokio::test]
    async fn projection_summary_matches_thread_value_summary() {
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
        let catalog = AgentCatalogSnapshot::load(&state).await;

        let cases = vec![
            json!({
                "thread_id": "thread::parity-cell",
                "agent_id": "claude",
                "provider_type": "claude_code",
                "sdk_session_id": "sess-1",
                "metadata": {
                    "model": "claude-opus-4-8",
                    "model_reasoning_effort": "low",
                    "model_service_tier": "flex"
                }
            }),
            json!({
                "thread_id": "thread::parity-legacy-override",
                "agent_id": "claude",
                "metadata": {
                    "model": "cell-model",
                    "model_override": "legacy-wins",
                    "model_reasoning_effort_override": "legacy-effort"
                }
            }),
            json!({
                "thread_id": "thread::parity-provider-key",
                "provider_key": "codex:main",
            }),
            json!({
                "thread_id": "thread::parity-agent-fallback",
                "agent_id": "claude",
            }),
            json!({
                "thread_id": "thread::parity-empty",
            }),
        ];

        for thread_value in cases {
            let live =
                build_thread_runtime_summary_with_catalog(&state, Some(&thread_value), &catalog);
            let thread_id = thread_value["thread_id"].as_str().unwrap_or("thread::x");
            let draft = crate::thread_meta_projection::
                thread_meta_projection_from_thread_data_with_active_run(
                    thread_id,
                    &thread_value,
                    None,
                )
                .expect("projection draft")
                .thread_meta;
            let record = ThreadMetaRecord {
                thread_id: draft.thread_id,
                workspace_dir: draft.workspace_dir,
                thread_type: draft.thread_type,
                thread_label: draft.thread_label,
                agent_id: draft.agent_id,
                provider_type: draft.provider_type,
                provider_key: draft.provider_key,
                selected_model: draft.selected_model,
                selected_model_reasoning_effort: draft.selected_model_reasoning_effort,
                selected_model_service_tier: draft.selected_model_service_tier,
                sdk_session_id: draft.sdk_session_id,
                created_at: draft.created_at,
                updated_at: draft.updated_at,
                message_count: draft.message_count,
                last_user_message: draft.last_user_message,
                last_assistant_message: draft.last_assistant_message,
                last_message_preview: draft.last_message_preview,
                recent_run_id: draft.recent_run_id,
                active_run_id: draft.active_run_id,
                worktree_json: draft.worktree_json,
                last_delivery_context_json: draft.last_delivery_context_json,
                last_delivery_updated_at: draft.last_delivery_updated_at,
                default_list_hidden: draft.default_list_hidden,
                sort_updated_at_us: draft.sort_updated_at_us,
                search_text: draft.search_text,
                root_workspace_path: draft.root_workspace_path,
                workspace_origin: draft.workspace_origin,
                projection_version: 6,
                projected_at: "2026-01-01T00:00:00Z".to_owned(),
            };
            let projected = build_thread_runtime_summary_from_meta(&state, &record, &catalog);
            assert_eq!(
                live, projected,
                "projection summary must match live summary for {thread_id}",
            );
        }
    }

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
