//! Custom-agent CRUD and provider-model listing handlers.

use crate::custom_agents::UpsertCustomAgentRequest;
use crate::optimistic_write::{StoreWriteError, WriteExpectation};
use crate::server::AppState;
use crate::thread_runtime::provider_type_from_key;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use garyx_models::CustomAgentProfile;
use garyx_models::provider::ProviderType;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
// ---------------------------------------------------------------------------

pub(super) fn provider_icon_descriptor(provider_type: &ProviderType) -> Option<Value> {
    let (key, label) = match provider_type {
        ProviderType::ClaudeCode => ("claude", "Claude"),
        ProviderType::CodexAppServer => ("codex", "Codex"),
        ProviderType::Traex => ("traex", "Traex"),
        ProviderType::AntigravityCli => ("gemini", "Antigravity"),
    };
    Some(json!({
        "key": key,
        "provider_type": provider_type,
        "label": label,
    }))
}

pub(super) fn custom_agent_response(agent: &CustomAgentProfile) -> Value {
    let mut value = serde_json::to_value(agent).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object
            .entry("avatar_data_url".to_owned())
            .or_insert(Value::Null);
        object.insert(
            "provider_icon".to_owned(),
            provider_icon_descriptor(&agent.provider_type).unwrap_or(Value::Null),
        );
    }
    value
}

// ---------------------------------------------------------------------------
// GET /api/threads/history
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomAgentUpsertPayload {
    pub agent_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub provider_type: garyx_models::ProviderType,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, alias = "modelReasoningEffort")]
    pub model_reasoning_effort: Option<String>,
    #[serde(default, alias = "modelServiceTier")]
    pub model_service_tier: Option<String>,
    #[serde(default, alias = "env", alias = "providerEnv")]
    pub provider_env: Option<HashMap<String, String>>,
    #[serde(
        default,
        alias = "defaultWorkspaceDir",
        alias = "workspace_dir",
        alias = "workspaceDir"
    )]
    pub default_workspace_dir: Option<String>,
    #[serde(default, alias = "avatarDataUrl")]
    pub avatar_data_url: Option<String>,
    #[serde(default, alias = "systemPrompt")]
    pub system_prompt: Option<String>,
    /// Concurrency token for updates: the `updated_at` of the profile the
    /// client based its edit on. Required on PUT; ignored on POST.
    #[serde(default, alias = "expectedUpdatedAt")]
    pub expected_updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomAgentTogglePayload {
    pub enabled: bool,
}

pub(super) fn collect_unknown_fields(
    path: &str,
    input: &Value,
    normalized: &Value,
    unknown: &mut Vec<String>,
) {
    match (input, normalized) {
        (Value::Object(input_map), Value::Object(normalized_map)) => {
            for (key, input_val) in input_map {
                let next_path = format!("{path}.{key}");
                match normalized_map.get(key) {
                    Some(normalized_val) => {
                        collect_unknown_fields(&next_path, input_val, normalized_val, unknown);
                    }
                    None => unknown.push(format!("unknown field: {next_path}")),
                }
            }
        }
        (Value::Array(input_arr), Value::Array(normalized_arr)) => {
            for (idx, input_val) in input_arr.iter().enumerate() {
                if let Some(normalized_val) = normalized_arr.get(idx) {
                    let next_path = format!("{path}[{idx}]");
                    collect_unknown_fields(&next_path, input_val, normalized_val, unknown);
                }
            }
        }
        _ => {}
    }
}

pub async fn list_custom_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let snapshot = state.ops.custom_agents.snapshot().await;
    let effective_default_agent_id =
        garyx_models::resolve_effective_default(&snapshot).map(|binding| binding.agent_id);
    let agents = snapshot
        .agents
        .iter()
        .into_iter()
        .map(|agent| custom_agent_response(&agent))
        .collect::<Vec<_>>();
    Json(json!({
        "agents": agents,
        "default_agent_id": snapshot.default_agent_id,
        "effective_default_agent_id": effective_default_agent_id,
    }))
}

pub(super) async fn publish_custom_agent_snapshot(state: &AppState) {
    state
        .integration
        .bridge
        .replace_agent_profiles(state.ops.custom_agents.snapshot().await)
        .await;
}

pub async fn toggle_custom_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(payload): Json<CustomAgentTogglePayload>,
) -> impl IntoResponse {
    match state
        .ops
        .custom_agents
        .set_enabled(&agent_id, payload.enabled)
        .await
    {
        Ok(agent) => {
            publish_custom_agent_snapshot(&state).await;
            (StatusCode::OK, Json(custom_agent_response(&agent))).into_response()
        }
        Err(error) => store_write_error_response(error),
    }
}

pub async fn set_default_custom_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.custom_agents.set_default_agent(&agent_id).await {
        Ok(agent) => {
            publish_custom_agent_snapshot(&state).await;
            (StatusCode::OK, Json(custom_agent_response(&agent))).into_response()
        }
        Err(error) => store_write_error_response(error),
    }
}

pub async fn list_provider_models(
    State(state): State<Arc<AppState>>,
    Path(provider_type): Path<String>,
) -> impl IntoResponse {
    let Some(provider_type) = provider_type_from_key(&provider_type) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "unsupported provider type" })),
        )
            .into_response();
    };

    let config = state.config_snapshot();
    let response =
        crate::provider_models::list_provider_models(config.as_ref(), provider_type).await;
    (StatusCode::OK, Json(response)).into_response()
}

pub async fn get_custom_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.custom_agents.get_agent(&agent_id).await {
        Some(agent) => (StatusCode::OK, Json(custom_agent_response(&agent))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "custom agent not found" })),
        )
            .into_response(),
    }
}

/// Map a classified store write failure onto its HTTP status: 400 invalid,
/// 404 missing update target, 409 concurrency conflict (with the stored
/// `updated_at` so clients can re-read), 500 persist failure.
pub(super) fn store_write_error_response(error: StoreWriteError) -> axum::response::Response {
    match error {
        StoreWriteError::Invalid(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
        }
        StoreWriteError::NotFound(message) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
        }
        StoreWriteError::Conflict {
            message,
            current_updated_at,
        } => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": message,
                "current_updated_at": current_updated_at,
            })),
        )
            .into_response(),
        StoreWriteError::Persist(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": message })),
        )
            .into_response(),
    }
}

/// The concurrency token every PUT must carry (`expected_updated_at`).
pub(super) fn require_expected_updated_at(
    expected_updated_at: Option<String>,
    what: &str,
) -> Result<WriteExpectation, axum::response::Response> {
    match expected_updated_at
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        Some(token) => Ok(WriteExpectation::UpdatedAt(token)),
        None => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "expected_updated_at is required — send the {what}'s current updated_at from a fresh GET"
                ),
            })),
        )
            .into_response()),
    }
}

pub async fn create_custom_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CustomAgentUpsertPayload>,
) -> impl IntoResponse {
    match state
        .ops
        .custom_agents
        .upsert_agent(
            UpsertCustomAgentRequest {
                agent_id: payload.agent_id,
                display_name: payload.display_name,
                provider_type: payload.provider_type,
                enabled: payload.enabled,
                model: payload.model,
                model_reasoning_effort: payload.model_reasoning_effort,
                model_service_tier: payload.model_service_tier,
                provider_env: payload.provider_env,
                default_workspace_dir: payload.default_workspace_dir,
                avatar_data_url: payload.avatar_data_url,
                system_prompt: payload.system_prompt,
            },
            WriteExpectation::Create,
        )
        .await
    {
        Ok(agent) => {
            let snapshot = state.ops.custom_agents.snapshot().await;
            state
                .integration
                .bridge
                .replace_agent_profiles(snapshot)
                .await;
            let config = state.config_snapshot();
            if let Err(error) = state
                .integration
                .bridge
                .reload_from_config(config.as_ref())
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error.to_string() })),
                )
                    .into_response();
            }
            (StatusCode::CREATED, Json(custom_agent_response(&agent))).into_response()
        }
        Err(error) => store_write_error_response(error),
    }
}

pub async fn update_custom_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(payload): Json<CustomAgentUpsertPayload>,
) -> impl IntoResponse {
    let expectation = match require_expected_updated_at(payload.expected_updated_at, "custom agent")
    {
        Ok(expectation) => expectation,
        Err(response) => return response,
    };
    match state
        .ops
        .custom_agents
        .upsert_agent(
            UpsertCustomAgentRequest {
                agent_id,
                display_name: payload.display_name,
                provider_type: payload.provider_type,
                enabled: payload.enabled,
                model: payload.model,
                model_reasoning_effort: payload.model_reasoning_effort,
                model_service_tier: payload.model_service_tier,
                provider_env: payload.provider_env,
                default_workspace_dir: payload.default_workspace_dir,
                avatar_data_url: payload.avatar_data_url,
                system_prompt: payload.system_prompt,
            },
            expectation,
        )
        .await
    {
        Ok(agent) => {
            let snapshot = state.ops.custom_agents.snapshot().await;
            state
                .integration
                .bridge
                .replace_agent_profiles(snapshot)
                .await;
            let config = state.config_snapshot();
            if let Err(error) = state
                .integration
                .bridge
                .reload_from_config(config.as_ref())
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error.to_string() })),
                )
                    .into_response();
            }
            (StatusCode::OK, Json(custom_agent_response(&agent))).into_response()
        }
        Err(error) => store_write_error_response(error),
    }
}

pub async fn delete_custom_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.custom_agents.delete_agent(&agent_id).await {
        Ok(()) => {
            let snapshot = state.ops.custom_agents.snapshot().await;
            state
                .integration
                .bridge
                .replace_agent_profiles(snapshot)
                .await;
            let config = state.config_snapshot();
            if let Err(error) = state
                .integration
                .bridge
                .reload_from_config(config.as_ref())
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error.to_string() })),
                )
                    .into_response();
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => store_write_error_response(error),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
