//! Additional observability and operations API endpoints.
//!
//! Supplements the existing routes in `routes.rs` and `dashboard.rs` with
//! thread history, cron data, settings mutation, and restart
//! controls.

use std::collections::HashSet;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use garyx_models::ChannelOutboundContent;
use garyx_models::config_loader::{
    ConfigLoadOptions, ConfigWriteOptions, load_config, write_config_value_atomic,
};
use garyx_models::provider::{ProviderMessage, ProviderType};
use garyx_router::{
    ThreadHistoryError, bindings_from_value, history_message_count, is_thread_key,
    workspace_dir_from_value,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_teams::UpsertAgentTeamRequest;
use crate::auto_research::{
    CreateAutoResearchRunRequest, PatchAutoResearchRun, SelectCandidateError,
    StopAutoResearchRunRequest, StopRunError,
};
use crate::custom_agents::UpsertCustomAgentRequest;
use crate::server::AppState;
use crate::wikis::UpsertWikiRequest;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
// ---------------------------------------------------------------------------

/// Tracks the last restart timestamp for cooldown enforcement.
#[derive(Default)]
pub struct RestartTracker {
    last_restart: Option<Instant>,
}

impl RestartTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cooldown_remaining_secs(&self, cooldown_secs: u64) -> Option<u64> {
        let last = self.last_restart?;
        let elapsed = last.elapsed().as_secs();
        if elapsed < cooldown_secs {
            Some(cooldown_secs - elapsed)
        } else {
            None
        }
    }

    pub fn mark_restart_now(&mut self) {
        self.last_restart = Some(Instant::now());
    }
}

/// Minimum seconds between restart requests.
const RESTART_COOLDOWN_SECS: u64 = 30;

fn provider_type_from_value(value: &Value) -> Option<ProviderType> {
    serde_json::from_value(value.clone()).ok()
}

fn provider_type_from_key(value: &str) -> Option<ProviderType> {
    let prefix = value.trim().split(':').next().unwrap_or_default();
    match prefix {
        "claude_code" => Some(ProviderType::ClaudeCode),
        "codex_app_server" => Some(ProviderType::CodexAppServer),
        "gemini_cli" => Some(ProviderType::GeminiCli),
        _ => None,
    }
}

fn provider_type_value(thread_value: &Value) -> Option<ProviderType> {
    thread_value
        .get("provider_type")
        .and_then(provider_type_from_value)
}

fn provider_label(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "Claude",
        ProviderType::CodexAppServer => "Codex",
        ProviderType::GeminiCli => "Gemini",
        ProviderType::AgentTeam => "Team",
    }
}

async fn team_bound_thread_ids(state: &Arc<AppState>, team_id: &str) -> Vec<String> {
    let keys = state.threads.thread_store.list_keys(None).await;
    let mut thread_ids = Vec::new();

    for key in keys {
        if !is_thread_key(&key) {
            continue;
        }
        let Some(data) = state.threads.thread_store.get(&key).await else {
            continue;
        };
        let is_team_thread = data
            .get("agent_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|agent_id| agent_id == team_id);
        let is_deleted_team_thread = data
            .get("team_deleted_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|deleted_id| deleted_id == team_id);
        if is_team_thread || is_deleted_team_thread {
            thread_ids.push(key);
        }
    }

    thread_ids
}

async fn reconcile_team_group_state(
    state: &Arc<AppState>,
    team: &garyx_models::AgentTeamProfile,
) -> Result<(), String> {
    let valid_members = team
        .member_agent_ids
        .iter()
        .cloned()
        .chain(std::iter::once(team.leader_agent_id.clone()))
        .collect::<HashSet<_>>();

    for thread_id in team_bound_thread_ids(state, &team.team_id).await {
        let Some(mut group) = state.ops.agent_team_group_store.load(&thread_id).await else {
            continue;
        };

        let before_child_count = group.child_threads.len();
        let before_offset_count = group.catch_up_offsets.len();
        group
            .child_threads
            .retain(|agent_id, _| valid_members.contains(agent_id));
        group
            .catch_up_offsets
            .retain(|agent_id, _| valid_members.contains(agent_id));

        if before_child_count != group.child_threads.len()
            || before_offset_count != group.catch_up_offsets.len()
        {
            state.ops.agent_team_group_store.save(&group).await;
        }
    }

    Ok(())
}

async fn clear_team_deleted_markers(state: &Arc<AppState>, team_id: &str) -> Result<(), String> {
    for thread_id in team_bound_thread_ids(state, team_id).await {
        let Some(mut data) = state.threads.thread_store.get(&thread_id).await else {
            continue;
        };
        let Some(obj) = data.as_object_mut() else {
            continue;
        };
        let mut changed = false;
        for key in ["team_deleted", "team_deleted_at", "team_deleted_id"] {
            changed |= obj.remove(key).is_some();
        }
        if changed {
            obj.insert(
                "updated_at".to_owned(),
                Value::String(Utc::now().to_rfc3339()),
            );
            state.threads.thread_store.set(&thread_id, data).await;
        }
    }
    Ok(())
}

async fn mark_deleted_team_threads(state: &Arc<AppState>, team_id: &str) -> Result<(), String> {
    let deleted_at = Utc::now().to_rfc3339();
    for thread_id in team_bound_thread_ids(state, team_id).await {
        let Some(mut data) = state.threads.thread_store.get(&thread_id).await else {
            continue;
        };
        if let Some(obj) = data.as_object_mut() {
            obj.insert("team_deleted".to_owned(), Value::Bool(true));
            obj.insert(
                "team_deleted_id".to_owned(),
                Value::String(team_id.to_owned()),
            );
            obj.insert(
                "team_deleted_at".to_owned(),
                Value::String(deleted_at.clone()),
            );
            obj.insert("updated_at".to_owned(), Value::String(deleted_at.clone()));
            state.threads.thread_store.set(&thread_id, data).await;
        }
        state.ops.agent_team_group_store.delete(&thread_id).await;
    }
    Ok(())
}

async fn reload_team_registry(state: &Arc<AppState>) -> Result<(), String> {
    let profiles = state.ops.agent_teams.list_teams().await;
    state
        .integration
        .bridge
        .replace_team_profiles(profiles)
        .await;
    let config = state.config_snapshot();
    state
        .integration
        .bridge
        .reload_from_config(config.as_ref())
        .await
        .map_err(|error| error.to_string())
}

fn build_debug_thread_runtime(thread_value: Option<&Value>) -> Value {
    let Some(thread_value) = thread_value else {
        return Value::Null;
    };

    let provider_type = provider_type_value(thread_value).or_else(|| {
        thread_value
            .get("provider_key")
            .and_then(Value::as_str)
            .and_then(provider_type_from_key)
    });
    let sdk_session_id = thread_value
        .get("sdk_session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let active_run_snapshot = thread_value
        .get("history")
        .and_then(|history| history.get("active_run_snapshot"));
    let active_run = active_run_snapshot.map(|snapshot| {
        let provider_type = snapshot
            .get("provider_type")
            .and_then(provider_type_from_value)
            .or_else(|| {
                snapshot
                    .get("provider_key")
                    .and_then(Value::as_str)
                    .and_then(provider_type_from_key)
            });
        json!({
            "run_id": snapshot.get("run_id").cloned().unwrap_or(Value::Null),
            "provider_type": provider_type.as_ref().and_then(|value| serde_json::to_value(value).ok()).unwrap_or(Value::Null),
            "provider_label": provider_type.as_ref().map(provider_label).unwrap_or("-"),
            "assistant_response": snapshot.get("assistant_response").cloned().unwrap_or(Value::Null),
            "updated_at": snapshot.get("updated_at").cloned().unwrap_or(Value::Null),
            "pending_user_input_count": snapshot.get("pending_user_inputs").and_then(Value::as_array).map(|items| items.len()).unwrap_or(0),
        })
    });

    json!({
        "provider_type": provider_type.as_ref().and_then(|value| serde_json::to_value(value).ok()).unwrap_or(Value::Null),
        "provider_label": provider_type.as_ref().map(provider_label).unwrap_or("-"),
        "sdk_session_id": sdk_session_id,
        "active_run": active_run.unwrap_or(Value::Null),
    })
}

fn clear_active_run_snapshot(thread_value: &mut Value) -> bool {
    let Some(object) = thread_value.as_object_mut() else {
        return false;
    };
    let should_remove_history = object
        .get_mut("history")
        .and_then(Value::as_object_mut)
        .map(|history| {
            let removed = history.remove("active_run_snapshot").is_some();
            (removed, history.is_empty())
        })
        .unwrap_or((false, false));
    if should_remove_history.1 {
        object.remove("history");
    }
    should_remove_history.0
}

async fn repair_inactive_active_run_snapshot(
    state: &Arc<AppState>,
    thread_id: &str,
    thread_value: &mut Value,
) -> bool {
    let run_id = thread_value
        .get("history")
        .and_then(|history| history.get("active_run_snapshot"))
        .and_then(|snapshot| snapshot.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(run_id) = run_id else {
        return clear_active_run_snapshot(thread_value);
    };
    if state.integration.bridge.is_run_active(run_id).await {
        return false;
    }
    let repaired = clear_active_run_snapshot(thread_value);
    if repaired {
        state
            .threads
            .thread_store
            .set(thread_id, thread_value.clone())
            .await;
    }
    repaired
}

// ---------------------------------------------------------------------------
// GET /api/threads/history
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ThreadHistoryParams {
    /// Maximum number of history items to return.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Optional prefix filter for thread ids.
    #[serde(default)]
    pub prefix: Option<String>,
    /// Whether to include message content (tool messages, etc.).
    #[serde(default)]
    pub include_messages: bool,
    /// Optional single thread lookup for detailed history.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Whether detailed history should keep tool-related messages.
    #[serde(default = "default_include_tool_messages")]
    pub include_tool_messages: bool,
}

fn default_limit() -> usize {
    50
}

fn default_include_tool_messages() -> bool {
    true
}

const MAX_THREAD_HISTORY_LIMIT: usize = 500;

#[derive(Deserialize)]
pub struct DebugThreadParams {
    pub thread_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Deserialize)]
pub struct DebugBotParams {
    pub bot_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Deserialize)]
pub struct AutoResearchRunsParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomAgentUpsertPayload {
    pub agent_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub provider_type: garyx_models::ProviderType,
    #[serde(default)]
    pub model: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTeamUpsertPayload {
    #[serde(alias = "team_id")]
    pub team_id: String,
    #[serde(alias = "display_name")]
    pub display_name: String,
    #[serde(alias = "leader_agent_id")]
    pub leader_agent_id: String,
    #[serde(default)]
    #[serde(alias = "member_agent_ids")]
    pub member_agent_ids: Vec<String>,
    #[serde(alias = "workflow_text")]
    pub workflow_text: String,
}

pub async fn list_auto_research_runs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AutoResearchRunsParams>,
) -> impl IntoResponse {
    let limit = params.limit.min(200);
    let items = state.ops.auto_research.list_runs(limit).await;
    (
        StatusCode::OK,
        Json(json!({
            "items": items,
        })),
    )
        .into_response()
}

pub async fn create_auto_research_run(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateAutoResearchRunRequest>,
) -> impl IntoResponse {
    match state.ops.auto_research.create_run(request).await {
        Ok(run) => {
            crate::auto_research::spawn_auto_research_loop(state.clone(), run.run_id.clone());
            (StatusCode::CREATED, Json(json!(run))).into_response()
        }
        Err(error) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "error": error,
            })),
        )
            .into_response(),
    }
}

pub async fn get_auto_research_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let Some(run) = state.ops.auto_research.get_run(&run_id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": "not_found",
            })),
        )
            .into_response();
    };
    let latest_iteration = state.ops.auto_research.latest_iteration(&run_id).await;
    (
        StatusCode::OK,
        Json(json!({
            "run": run,
            "latest_iteration": latest_iteration,
            "active_thread_id": run.active_thread_id,
        })),
    )
        .into_response()
}

pub async fn patch_auto_research_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(patch): Json<PatchAutoResearchRun>,
) -> impl IntoResponse {
    match state.ops.auto_research.patch_run(&run_id, &patch).await {
        Ok(run) => (StatusCode::OK, Json(json!(run))).into_response(),
        Err(e) if e.contains("not found") => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

pub async fn list_auto_research_iterations(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let Some(items) = state.ops.auto_research.list_iterations(&run_id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": "not_found",
            })),
        )
            .into_response();
    };
    (
        StatusCode::OK,
        Json(json!({
            "run_id": run_id,
            "items": items,
        })),
    )
        .into_response()
}

pub async fn stop_auto_research_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(request): Json<StopAutoResearchRunRequest>,
) -> impl IntoResponse {
    match state
        .ops
        .auto_research
        .stop_run(&run_id, request.reason)
        .await
    {
        Ok(run) => (StatusCode::OK, Json(json!(run))).into_response(),
        Err(StopRunError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": "not_found",
            })),
        )
            .into_response(),
        Err(StopRunError::InvalidState) => (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "error": "invalid_state",
            })),
        )
            .into_response(),
    }
}

/// DELETE /api/auto-research/runs/{run_id}
pub async fn delete_auto_research_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    if state.ops.auto_research.delete_run(&run_id).await {
        (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "not_found" })),
        )
            .into_response()
    }
}

/// GET /api/auto-research/runs/{run_id}/candidates
pub async fn list_auto_research_candidates(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let Some(run) = state.ops.auto_research.get_run(&run_id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": "not_found",
            })),
        )
            .into_response();
    };
    let mut candidates = run.candidates.clone();
    // Sort by score descending (best first); candidates without a verdict sort last.
    candidates.sort_by(|a, b| {
        let score_a = a
            .verdict
            .as_ref()
            .map(|v| v.score)
            .unwrap_or(f32::NEG_INFINITY);
        let score_b = b
            .verdict
            .as_ref()
            .map(|v| v.score)
            .unwrap_or(f32::NEG_INFINITY);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let best_candidate_id = candidates.first().map(|c| c.candidate_id.clone());
    (
        StatusCode::OK,
        Json(json!({
            "candidates": candidates,
            "best_candidate_id": best_candidate_id,
        })),
    )
        .into_response()
}

/// POST /api/auto-research/runs/{run_id}/select/{candidate_id}
pub async fn select_auto_research_candidate(
    State(state): State<Arc<AppState>>,
    Path((run_id, candidate_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .ops
        .auto_research
        .select_candidate(&run_id, &candidate_id)
        .await
    {
        Ok(run) => (StatusCode::OK, Json(json!(run))).into_response(),
        Err(SelectCandidateError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": "not_found",
            })),
        )
            .into_response(),
        Err(SelectCandidateError::InvalidIndex) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "error": "invalid_candidate_id",
            })),
        )
            .into_response(),
    }
}

/// POST /api/auto-research/runs/{run_id}/feedback
#[derive(Debug, Clone, Deserialize)]
pub struct InjectFeedbackRequest {
    pub message: String,
}

pub async fn inject_auto_research_feedback(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(request): Json<InjectFeedbackRequest>,
) -> impl IntoResponse {
    if request.message.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"ok": false, "error": "message must not be empty"})),
        )
            .into_response();
    }
    match state
        .ops
        .auto_research
        .inject_feedback(&run_id, request.message)
        .await
    {
        Ok(run) => (StatusCode::OK, Json(json!(run))).into_response(),
        Err(e) => {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            };
            (status, Json(json!({"ok": false, "error": e}))).into_response()
        }
    }
}

/// POST /api/auto-research/runs/{run_id}/reverify
#[derive(Debug, Clone, Deserialize)]
pub struct ReverifyRequest {
    pub candidate_id: String,
    #[serde(default)]
    pub guidance: Option<String>,
}

pub async fn reverify_auto_research_candidate(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
    Json(request): Json<ReverifyRequest>,
) -> impl IntoResponse {
    match state
        .ops
        .auto_research
        .request_reverify(&run_id, request.candidate_id, request.guidance)
        .await
    {
        Ok(run) => (StatusCode::OK, Json(json!(run))).into_response(),
        Err(e) => {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            };
            (status, Json(json!({"ok": false, "error": e}))).into_response()
        }
    }
}

/// GET /api/threads/history - thread history with optional filtering.
pub async fn thread_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ThreadHistoryParams>,
) -> impl IntoResponse {
    if let Some(thread_id) = params.thread_id.as_deref() {
        let payload = thread_history_for_key(
            &state,
            thread_id,
            params.limit,
            params.include_tool_messages,
        )
        .await;
        return Json(payload);
    }

    let keys = state
        .threads
        .thread_store
        .list_keys(params.prefix.as_deref())
        .await;
    let keys: Vec<String> = keys.into_iter().filter(|key| is_thread_key(key)).collect();

    let limited_keys: Vec<&String> = keys.iter().take(params.limit).collect();
    let mut threads = Vec::new();

    for key in &limited_keys {
        if params.include_messages {
            if let Some(data) = state.threads.thread_store.get(key).await {
                threads.push(json!({
                    "key": key,
                    "data": data,
                }));
            } else {
                threads.push(json!({
                    "key": key,
                    "data": null,
                }));
            }
        } else {
            // Summary only: key + existence
            let exists = state.threads.thread_store.exists(key).await;
            threads.push(json!({
                "key": key,
                "active": exists,
            }));
        }
    }

    Json(json!({
        "threads": threads,
        "total": keys.len(),
        "limit": params.limit,
        "include_messages": params.include_messages,
    }))
}

pub async fn debug_thread(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DebugThreadParams>,
) -> impl IntoResponse {
    let thread_id = params.thread_id.trim();
    if thread_id.is_empty() {
        return Json(json!({
            "ok": false,
            "reason": "missing-thread-id",
        }));
    }

    let limit = params.limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    let mut thread_value = state.threads.thread_store.get(thread_id).await;
    if let Some(thread_value_ref) = thread_value.as_mut() {
        let _ = repair_inactive_active_run_snapshot(&state, thread_id, thread_value_ref).await;
    }
    let bindings = thread_value
        .as_ref()
        .map(bindings_from_value)
        .unwrap_or_default()
        .into_iter()
        .map(|binding| {
            let delivery_thread_id =
                crate::routes::binding_delivery_thread_id(&binding.binding_key, &binding.chat_id);
            json!({
                "channel": binding.channel,
                "account_id": binding.account_id,
                "chat_id": binding.chat_id,
                "binding_key": binding.binding_key,
                "peer_id": binding.binding_key,
                "thread_binding_key": binding.binding_key,
                "thread_scope": delivery_thread_id,
                "delivery_target_type": binding.delivery_target_type,
                "delivery_target_id": binding.delivery_target_id,
            })
        })
        .collect::<Vec<_>>();

    let transcript_path = state
        .threads
        .history
        .transcript_store()
        .transcript_path(thread_id)
        .map(|path| path.display().to_string());

    let ledger = match state
        .threads
        .message_ledger
        .records_for_thread(thread_id, limit)
        .await
    {
        Ok(records) => json!({
            "ok": true,
            "records": records,
        }),
        Err(error) => json!({
            "ok": false,
            "error": error.to_string(),
            "records": [],
        }),
    };

    Json(json!({
        "ok": true,
        "thread_id": thread_id,
        "thread": thread_value,
        "thread_runtime": build_debug_thread_runtime(thread_value.as_ref()),
        "bindings": bindings,
        "history": thread_history_for_key(&state, thread_id, limit, true).await,
        "message_ledger": ledger,
        "transcript_path": transcript_path,
    }))
}

pub async fn debug_bot(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DebugBotParams>,
) -> impl IntoResponse {
    let bot_id = params.bot_id.trim();
    if bot_id.is_empty() {
        return Json(json!({
            "ok": false,
            "reason": "missing-bot-id",
        }));
    }

    let limit = params.limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    let recent_records = match state
        .threads
        .message_ledger
        .records_for_bot(bot_id, limit)
        .await
    {
        Ok(records) => records,
        Err(error) => {
            return Json(json!({
                "ok": false,
                "bot_id": bot_id,
                "reason": format!("message-ledger-error:{error}"),
            }));
        }
    };
    let problem_threads = match state
        .threads
        .message_ledger
        .problem_threads_for_bot(bot_id, limit)
        .await
    {
        Ok(threads) => threads,
        Err(error) => {
            return Json(json!({
                "ok": false,
                "bot_id": bot_id,
                "reason": format!("message-ledger-error:{error}"),
            }));
        }
    };

    let active_threads = distinct_thread_count(&recent_records);
    Json(json!({
        "ok": true,
        "bot_id": bot_id,
        "recent_records": recent_records,
        "problem_threads": problem_threads,
        "stats": {
            "recent_messages": recent_records.len(),
            "problem_threads": problem_threads.len(),
            "active_threads": active_threads,
        }
    }))
}

pub async fn debug_bot_threads(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DebugBotParams>,
) -> impl IntoResponse {
    let bot_id = params.bot_id.trim();
    if bot_id.is_empty() {
        return Json(json!({
            "ok": false,
            "reason": "missing-bot-id",
        }));
    }

    let limit = params.limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    match state
        .threads
        .message_ledger
        .problem_threads_for_bot(bot_id, limit)
        .await
    {
        Ok(problem_threads) => Json(json!({
            "ok": true,
            "bot_id": bot_id,
            "threads": problem_threads,
        })),
        Err(error) => Json(json!({
            "ok": false,
            "bot_id": bot_id,
            "reason": format!("message-ledger-error:{error}"),
        })),
    }
}

pub(crate) async fn thread_history_for_key(
    state: &Arc<AppState>,
    thread_id: &str,
    limit: usize,
    include_tool_messages: bool,
) -> Value {
    let key = thread_id.trim();
    if key.is_empty() {
        let thread = Value::Null;
        return json!({
            "ok": false,
            "reason": "missing-thread-id",
            "thread": thread,
            "session": thread,
            "team": Value::Null,
            "messages": [],
            "pending_user_inputs": [],
            "outbound_deliveries": [],
            "message_stats": { "returned_messages": 0 },
        });
    }

    let bounded_limit = limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    let snapshot = match state
        .threads
        .history
        .thread_snapshot(key, bounded_limit)
        .await
    {
        Ok(snapshot) => snapshot,
        Err(ThreadHistoryError::ThreadNotFound(_)) => {
            let thread = json!({ "thread_id": key, "thread_key": key });
            return json!({
                "ok": false,
                "reason": "thread-not-found",
                "thread": thread,
                "session": thread,
                "team": Value::Null,
                "messages": [],
                "pending_user_inputs": [],
                "outbound_deliveries": [],
                "message_stats": { "returned_messages": 0 },
            });
        }
        Err(ThreadHistoryError::MissingTranscript(_)) => {
            let thread = json!({ "thread_id": key, "thread_key": key });
            return json!({
                "ok": false,
                "reason": "thread-transcript-missing",
                "thread": thread,
                "session": thread,
                "team": Value::Null,
                "messages": [],
                "pending_user_inputs": [],
                "outbound_deliveries": [],
                "message_stats": { "returned_messages": 0 },
            });
        }
        Err(error) => {
            let thread = json!({ "thread_id": key, "thread_key": key });
            return json!({
                "ok": false,
                "reason": format!("thread-history-error:{error}"),
                "thread": thread,
                "session": thread,
                "team": Value::Null,
                "messages": [],
                "pending_user_inputs": [],
                "outbound_deliveries": [],
                "message_stats": { "returned_messages": 0 },
            });
        }
    };
    let messages = snapshot.combined_messages();
    let total_messages = snapshot.total_messages();
    let mut data_raw = snapshot.thread_data;

    let mut history_messages = Vec::new();
    let mut kind_counts = serde_json::Map::new();
    let mut role_counts = serde_json::Map::new();
    let mut tool_related_count = 0_u64;
    let mut likely_user_visible_count = 0_u64;

    for (idx, message) in messages.iter().enumerate() {
        let Some(obj) = message.as_object() else {
            continue;
        };

        let role = obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .trim()
            .to_ascii_lowercase();

        let tool_related = is_tool_related_message(&role, obj);
        if !include_tool_messages && tool_related {
            continue;
        }

        let kind = resolve_message_kind(&role, tool_related);
        let likely_user_visible = matches!(kind, "user_input" | "assistant_reply");
        if tool_related {
            tool_related_count += 1;
        }
        if likely_user_visible {
            likely_user_visible_count += 1;
        }
        increment_counter(&mut kind_counts, kind);
        increment_counter(&mut role_counts, &role);

        let normalized_message = ProviderMessage::from_value(message);
        let raw_content = normalized_message
            .as_ref()
            .map(|entry| entry.content.clone())
            .or_else(|| obj.get("content").cloned())
            .unwrap_or(Value::Null);
        let content = enrich_message_content_for_history(&raw_content);
        let message_value = if let Some(mut entry) = normalized_message.clone() {
            entry.content = content.clone();
            entry.to_json_value()
        } else {
            let mut value = message.clone();
            if let Some(map) = value.as_object_mut() {
                map.insert("content".to_owned(), content.clone());
            }
            value
        };
        let text = normalized_message
            .as_ref()
            .and_then(|entry| entry.text.clone())
            .unwrap_or_else(|| stringify_message_content(&content));
        history_messages.push(json!({
            "index": idx,
            "role": if role.is_empty() { "unknown" } else { role.as_str() },
            "kind": kind,
            "tool_related": tool_related,
            "likely_user_visible": likely_user_visible,
            "internal": obj.get("internal").cloned().unwrap_or(Value::Bool(false)),
            "internal_kind": obj.get("internal_kind").cloned().unwrap_or(Value::Null),
            "loop_origin": obj.get("loop_origin").cloned().unwrap_or(Value::Null),
            "timestamp": obj.get("timestamp").cloned().unwrap_or(Value::Null),
            "text": text,
            "content": stringify_message_content(&content),
            "raw_content_type": raw_content_type_name(&content),
            "message": message_value,
        }));
    }

    let outbound_raw = data_raw
        .get("outbound_message_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut outbound_deliveries = Vec::new();
    for (idx, record) in outbound_raw.iter().enumerate() {
        let Some(obj) = record.as_object() else {
            continue;
        };
        outbound_deliveries.push(json!({
            "index": idx,
            "channel": obj.get("channel").cloned().unwrap_or(Value::Null),
            "account_id": obj.get("account_id").cloned().unwrap_or(Value::Null),
            "chat_id": obj.get("chat_id").cloned().unwrap_or(Value::Null),
            "message_id": stringify_optional(obj.get("message_id")),
            "timestamp": obj.get("timestamp").cloned().unwrap_or(Value::Null),
        }));
    }
    if outbound_deliveries.len() > 100 {
        let drop_count = outbound_deliveries.len() - 100;
        outbound_deliveries.drain(0..drop_count);
    }
    let returned_messages = history_messages.len();
    let outbound_total = outbound_deliveries.len();
    let pending_raw = data_raw
        .get("pending_user_inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut pending_user_inputs = Vec::new();
    let mut persisted_pending_user_inputs = Vec::new();
    let mut active_pending_user_input_count = 0_u64;
    let mut pending_inputs_repaired = false;
    for record in pending_raw {
        let Some(obj) = record.as_object() else {
            pending_inputs_repaired = true;
            continue;
        };
        let run_id = obj
            .get("bridge_run_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let run_active = if let Some(run_id) = run_id.as_deref() {
            state.integration.bridge.is_run_active(run_id).await
        } else {
            false
        };
        let stored_status = obj
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("queued");
        if stored_status.eq_ignore_ascii_case("abandoned") || !run_active {
            pending_inputs_repaired = true;
            continue;
        }

        active_pending_user_input_count += 1;
        let id = obj.get("id").cloned().unwrap_or(Value::Null);
        let timestamp = obj.get("queued_at").cloned().unwrap_or(Value::Null);
        let content = obj.get("content").cloned().unwrap_or(Value::Null);
        let text = obj
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| stringify_message_content(&content));
        let raw_content_type = raw_content_type_name(&content);
        persisted_pending_user_inputs.push(record);
        pending_user_inputs.push(json!({
            "id": id,
            "run_id": run_id,
            "timestamp": timestamp,
            "status": "awaiting_ack",
            "active": true,
            "text": text,
            "content": content,
            "raw_content_type": raw_content_type,
        }));
    }
    if pending_inputs_repaired {
        if let Some(obj) = data_raw.as_object_mut() {
            obj.insert(
                "pending_user_inputs".to_owned(),
                Value::Array(persisted_pending_user_inputs),
            );
            state.threads.thread_store.set(key, data_raw.clone()).await;
        }
    }
    let thread = summarize_thread(key, &data_raw, &messages);
    // Unlike `routes::thread_metadata_response` (which nests `team` inside
    // the thread object because the response IS the thread), this envelope
    // wraps `thread`/`session`/`messages` etc. so `team` rides alongside
    // them at the top level. Emit `Value::Null` for non-team threads so
    // the desktop client can probe a single field without checking for
    // presence.
    let team = crate::routes::team_block_for_thread(state, key, &data_raw)
        .await
        .unwrap_or(Value::Null);

    json!({
        "ok": true,
        "thread": thread,
        "session": thread,
        "team": team,
        "messages": history_messages,
        "pending_user_inputs": pending_user_inputs,
        "message_stats": {
            "total_messages_in_thread": total_messages,
            "total_messages_in_session": total_messages,
            "returned_messages": returned_messages,
            "tool_related_count": tool_related_count,
            "likely_user_visible_count": likely_user_visible_count,
            "pending_user_input_count": pending_user_inputs.len(),
            "active_pending_user_input_count": active_pending_user_input_count,
            "kind_counts": Value::Object(kind_counts),
            "role_counts": Value::Object(role_counts),
        },
        "outbound_deliveries": outbound_deliveries,
        "outbound_total": outbound_total,
        "include_tool_messages": include_tool_messages,
    })
}

fn distinct_thread_count(records: &[garyx_models::MessageLedgerRecord]) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    for record in records {
        if let Some(thread_id) = record.thread_id.as_deref() {
            seen.insert(thread_id.to_owned());
        }
    }
    seen.len()
}

fn summarize_thread(thread_id: &str, data: &Value, messages: &[Value]) -> Value {
    let message_count = history_message_count(data);

    let last_user_message = last_message_content(messages, "user");
    let last_assistant_message = last_message_content(messages, "assistant");

    let get_value = |primary: &str, fallback: &str| {
        data.get(primary)
            .cloned()
            .or_else(|| data.get(fallback).cloned())
            .unwrap_or(Value::Null)
    };

    json!({
        "thread_id": thread_id,
        "thread_key": thread_id,
        "label": data.get("label").cloned().unwrap_or(Value::Null),
        "channel": get_value("channel", "last_channel"),
        "account_id": get_value("account_id", "last_account_id"),
        "from_id": get_value("from_id", "last_to"),
        "workspace_dir": workspace_dir_from_value(data).map(Value::String).unwrap_or(Value::Null),
        "channel_bindings": serde_json::to_value(bindings_from_value(data)).unwrap_or_else(|_| Value::Array(Vec::new())),
        "message_count": message_count,
        "updated_at": get_value("updated_at", "_updated_at"),
        "created_at": get_value("created_at", "_created_at"),
        "last_user_message": last_user_message,
        "last_assistant_message": last_assistant_message,
        "session_type": infer_thread_type(thread_id),
        "thread_type": infer_thread_type(thread_id),
    })
}

fn last_message_content(messages: &[Value], role: &str) -> Option<String> {
    for message in messages.iter().rev() {
        let Some(obj) = message.as_object() else {
            continue;
        };
        let role_match = obj
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|value| value == role);
        if !role_match {
            continue;
        }
        if let Some(content) = obj.get("content") {
            let text = stringify_message_content(content);
            if !text.trim().is_empty() {
                return Some(truncate_text(text, 260));
            }
        }
    }
    None
}

fn infer_thread_type(thread_id: &str) -> &'static str {
    if thread_id.starts_with("cron::") {
        "cron"
    } else if thread_id.contains("::group::") {
        "group"
    } else {
        "chat"
    }
}

fn resolve_message_kind(role: &str, tool_related: bool) -> &'static str {
    match role {
        "user" => "user_input",
        "assistant" => {
            if tool_related {
                "tool_trace"
            } else {
                "assistant_reply"
            }
        }
        "tool" | "tool_use" | "tool_result" => "tool_trace",
        "system" => "system",
        _ if tool_related => "tool_trace",
        _ => "internal",
    }
}

fn is_tool_related_message(role: &str, message: &serde_json::Map<String, Value>) -> bool {
    if matches!(role, "tool" | "tool_use" | "tool_result") {
        return true;
    }

    if message
        .get("tool_use_result")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }

    if message
        .get("tool_name")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
    {
        return true;
    }

    contains_tool_hint(message.get("content"))
        || contains_tool_hint(message.get("metadata"))
        || contains_tool_hint(message.get("input"))
        || contains_tool_hint(message.get("result"))
}

fn contains_tool_hint(value: Option<&Value>) -> bool {
    fn inner(value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match value {
            Value::String(text) => {
                let lower = text.to_ascii_lowercase();
                lower.contains("tool_use")
                    || lower.contains("tool_result")
                    || lower.contains("tool_call")
                    || lower.contains("mcp__")
            }
            Value::Array(items) => items.iter().any(|item| inner(item, depth + 1)),
            Value::Object(map) => map.iter().any(|(key, item)| {
                let lower = key.to_ascii_lowercase();
                lower == "tool_use_id"
                    || lower == "tool_call_id"
                    || lower == "tool_calls"
                    || lower.contains("mcp__")
                    || lower.contains("tool_")
                    || inner(item, depth + 1)
            }),
            _ => false,
        }
    }

    value.is_some_and(|value| inner(value, 0))
}

fn raw_content_type_name(content: &Value) -> &'static str {
    match content {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

fn summarize_content_block(content: &Value, parts: &mut Vec<String>, image_count: &mut usize) {
    let Some(obj) = content.as_object() else {
        return;
    };

    match obj.get("type").and_then(Value::as_str).unwrap_or_default() {
        "text" => {
            if let Some(text) = obj.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_owned());
                }
            }
        }
        "image" => {
            *image_count += 1;
        }
        "file" => {
            let label = obj
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| obj.get("path").and_then(Value::as_str))
                .unwrap_or("file");
            parts.push(format!("[File] {label}"));
        }
        _ => {}
    }
}

const HISTORY_IMAGE_INLINE_MAX_BYTES: u64 = 12 * 1024 * 1024;

fn enrich_image_block_for_history(block: &serde_json::Map<String, Value>) -> Value {
    if block.get("url").and_then(Value::as_str).is_some()
        || block
            .get("source")
            .and_then(Value::as_object)
            .and_then(|source| source.get("data"))
            .and_then(Value::as_str)
            .is_some()
    {
        return Value::Object(block.clone());
    }

    let Some(path) = block.get("path").and_then(Value::as_str).map(str::trim) else {
        return Value::Object(block.clone());
    };
    if path.is_empty() {
        return Value::Object(block.clone());
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return Value::Object(block.clone());
    };
    if !metadata.is_file() || metadata.len() > HISTORY_IMAGE_INLINE_MAX_BYTES {
        return Value::Object(block.clone());
    }
    let Ok(bytes) = std::fs::read(path) else {
        return Value::Object(block.clone());
    };

    let mut hydrated = block.clone();
    let media_type = hydrated
        .get("media_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            FsPath::new(path)
                .extension()
                .and_then(|value| value.to_str())
                .map(|ext| match ext.to_ascii_lowercase().as_str() {
                    "png" => "image/png".to_owned(),
                    "gif" => "image/gif".to_owned(),
                    "webp" => "image/webp".to_owned(),
                    _ => "image/jpeg".to_owned(),
                })
        })
        .unwrap_or_else(|| "image/jpeg".to_owned());
    hydrated.insert(
        "source".to_owned(),
        json!({
            "type": "base64",
            "media_type": media_type,
            "data": BASE64.encode(bytes),
        }),
    );
    Value::Object(hydrated)
}

fn enrich_message_content_for_history(content: &Value) -> Value {
    match content {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(enrich_message_content_for_history)
                .collect(),
        ),
        Value::Object(block) => match block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "image" => enrich_image_block_for_history(block),
            _ => Value::Object(block.clone()),
        },
        _ => content.clone(),
    }
}

fn humanize_structured_content(content: &Value) -> Option<String> {
    let mut parts = Vec::new();
    let mut image_count = 0_usize;

    match content {
        Value::Array(items) => {
            for item in items {
                summarize_content_block(item, &mut parts, &mut image_count);
            }
        }
        Value::Object(_) => {
            summarize_content_block(content, &mut parts, &mut image_count);
        }
        _ => {}
    }

    if image_count > 0 {
        parts.push(if image_count == 1 {
            "[1 image]".to_owned()
        } else {
            format!("[{image_count} images]")
        });
    }

    let summary = parts.join("\n\n");
    if summary.trim().is_empty() {
        None
    } else {
        Some(summary)
    }
}

fn stringify_message_content(content: &Value) -> String {
    let text = match content {
        Value::Null => String::new(),
        Value::String(text) => text.to_owned(),
        Value::Array(_) | Value::Object(_) => {
            if let Some(summary) = humanize_structured_content(content) {
                summary
            } else {
                serde_json::to_string(content).unwrap_or_else(|_| format!("{content}"))
            }
        }
        _ => content.to_string(),
    };
    truncate_text(text, 5000)
}

fn truncate_text(text: String, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text;
    }
    text.chars().take(max_len).collect()
}

fn stringify_optional(value: Option<&Value>) -> Option<String> {
    value.and_then(|v| match v {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Number(num) => Some(num.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        other => Some(other.to_string()),
    })
}

fn increment_counter(map: &mut serde_json::Map<String, Value>, key: &str) {
    let entry = map
        .entry(key.to_owned())
        .or_insert_with(|| Value::from(0_u64));
    let next = entry.as_u64().unwrap_or(0) + 1;
    *entry = Value::from(next);
}

// ---------------------------------------------------------------------------
// GET /api/cron/jobs
// ---------------------------------------------------------------------------

/// GET /api/cron/jobs - list scheduled cron jobs from CronService.
pub async fn cron_jobs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cron = match &state.ops.cron_service {
        Some(svc) => svc,
        None => {
            return Json(json!({
                "jobs": [],
                "count": 0,
                "service_available": false,
            }));
        }
    };

    let jobs = cron.list().await;
    let count = jobs.len();
    let job_list: Vec<Value> = jobs
        .into_iter()
        .map(|j| {
            json!({
                "id": j.id,
                "schedule": j.schedule,
                "action": j.action,
                "target": j.target,
                "message": j.message,
                "delete_after_run": j.delete_after_run,
                "enabled": j.enabled,
                "next_run": j.next_run.to_rfc3339(),
                "last_status": j.last_status,
                "run_count": j.run_count,
                "last_run_at": j.last_run_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();

    Json(json!({
        "jobs": job_list,
        "count": count,
        "service_available": true,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/cron/runs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CronRunsParams {
    /// Maximum number of recent runs to return.
    #[serde(default = "default_cron_runs_limit")]
    pub limit: usize,
    /// Pagination offset from most-recent-first run list.
    #[serde(default)]
    pub offset: usize,
}

fn default_cron_runs_limit() -> usize {
    50
}

/// GET /api/cron/runs - list recent cron run statuses from CronService.
pub async fn cron_runs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CronRunsParams>,
) -> impl IntoResponse {
    let cron = match &state.ops.cron_service {
        Some(svc) => svc,
        None => {
            return Json(json!({
                "runs": [],
                "count": 0,
                "service_available": false,
            }));
        }
    };

    let runs = cron.list_runs(params.limit, params.offset).await;
    let total = cron.total_runs().await;
    let count = runs.len();
    let runs: Vec<Value> = runs
        .into_iter()
        .map(|r| {
            json!({
                "run_id": r.run_id,
                "job_id": r.job_id,
                "status": r.status,
                "started_at": r.started_at.to_rfc3339(),
                "finished_at": r.finished_at.map(|t| t.to_rfc3339()),
                "duration_ms": r.duration_ms,
                "error": r.error,
            })
        })
        .collect();

    Json(json!({
        "runs": runs,
        "count": count,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
        "service_available": true,
    }))
}

// ---------------------------------------------------------------------------
// PUT /api/settings
// ---------------------------------------------------------------------------

/// Deep-merge two JSON values.  For objects, keys from `overlay` override
/// keys in `base`; keys only in `base` are preserved.  All other types are
/// replaced outright by `overlay`.
fn deep_merge_json(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (key, overlay_val) in overlay_map {
                let merged = if let Some(base_val) = base_map.remove(&key) {
                    deep_merge_json(base_val, overlay_val)
                } else {
                    overlay_val
                };
                base_map.insert(key, merged);
            }
            Value::Object(base_map)
        }
        // Non-object overlay replaces base entirely (arrays, scalars, null).
        (_base, overlay) => overlay,
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct SettingsUpdateQuery {
    /// When false, the incoming body fully replaces the stored config instead
    /// of being deep-merged onto it. Needed so callers sending the full doc
    /// (desktop app) can actually delete keys (e.g. a channel account);
    /// additive merge can never express deletion. Defaults to true to stay
    /// safe for partial-payload callers.
    #[serde(default)]
    pub merge: Option<bool>,
}

/// PUT /api/settings - validate, persist, and apply configuration.
pub async fn settings_update(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SettingsUpdateQuery>,
    Json(mut body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    // Validate that the body is a JSON object
    if !body.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": ["request body must be a JSON object"],
            })),
        );
    }

    // Serialize settings updates to prevent TOCTOU races between concurrent
    // PUT requests (read snapshot → modify → persist → apply).
    let _settings_guard = state.ops.settings_mutex.lock().await;

    let existing_config = serde_json::to_value(state.config_snapshot()).unwrap_or_default();

    // Deep-merge by default: prevents partial payloads (e.g. `{"commands":[]}`)
    // from wiping unrelated sections. Callers sending the full document opt
    // out with `?merge=false` so deletions (e.g. removing a channel account)
    // actually take effect — additive merge has no delete semantics.
    if params.merge.unwrap_or(true) {
        body = deep_merge_json(existing_config.clone(), body);
    }

    garyx_models::config_loader::strip_redundant_config_fields(&mut body);

    // Attempt to deserialize as GaryxConfig for validation.
    let config = match serde_json::from_value::<garyx_models::config::GaryxConfig>(body.clone()) {
        Ok(config) => config,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "errors": [format!("invalid configuration: {e}")],
                })),
            );
        }
    };
    // Strict unknown-field validation: compare user input with normalized schema output.
    let normalized = match serde_json::to_value(&config) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "errors": [format!("failed to normalize config for validation: {e}")],
                })),
            );
        }
    };
    let mut unknown_fields = Vec::new();
    collect_unknown_fields("$", &body, &normalized, &mut unknown_fields);
    if !unknown_fields.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": unknown_fields,
            })),
        );
    }

    let mcp_servers = config.mcp_servers.clone();
    body = normalized.clone();

    // Apply runtime config FIRST so we can detect errors before persisting.
    // This prevents writing a broken config to disk that would survive restarts.
    if let Err(error) = state.apply_runtime_config(config).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "errors": [format!("failed to apply runtime config: {error}")],
            })),
        );
    }

    // Persist to disk only after successful runtime apply.
    if let Some(path) = state.ops.config_path.clone() {
        let body = body.clone();
        let write_result = tokio::task::spawn_blocking(move || {
            let write_opts = ConfigWriteOptions {
                backup_keep: 3,
                mode: Some(0o600),
            };
            write_config_value_atomic(&path, &body, &write_opts)
        })
        .await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "ok": false,
                        "errors": [format!("failed to persist config file: {e}")],
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "ok": false,
                        "errors": [format!("failed to persist config file: {e}")],
                    })),
                );
            }
        }
    }

    let mut warnings: Vec<String> = Vec::new();
    if let Err(error) = crate::mcp_config::sync_external_configs_from_servers(&mcp_servers).await {
        tracing::warn!("MCP external config sync failed (non-fatal): {error}");
        warnings.push(format!("MCP config sync: {error}"));
    }

    let mut result = json!({
        "ok": true,
        "message": "settings validated, persisted, and applied",
    });
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }

    (StatusCode::OK, Json(result))
}

/// POST /api/settings/reload - reload config from disk and apply it.
pub async fn settings_reload(State(state): State<Arc<AppState>>) -> (StatusCode, Json<Value>) {
    let _settings_guard = state.ops.settings_mutex.lock().await;

    let Some(path) = state.ops.config_path.clone() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": ["runtime config path is unavailable"],
            })),
        );
    };

    let loaded = match tokio::task::spawn_blocking({
        let path = path.clone();
        move || {
            load_config(
                &path,
                &ConfigLoadOptions {
                    default_path: path.clone(),
                    runtime_overrides: Default::default(),
                },
            )
        }
    })
    .await
    {
        Ok(Ok(loaded)) => loaded,
        Ok(Err(error)) => {
            let errors = if error.diagnostics.errors.is_empty() {
                vec![error.to_string()]
            } else {
                error
                    .diagnostics
                    .errors
                    .iter()
                    .map(|item| item.message.clone())
                    .collect::<Vec<_>>()
            };
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "errors": errors,
                    "config_path": path.display().to_string(),
                })),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "errors": [format!("failed to load config file: {error}")],
                    "config_path": path.display().to_string(),
                })),
            );
        }
    };

    if let Err(error) = state.apply_runtime_config(loaded.config).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "errors": [format!("failed to apply runtime config: {error}")],
                "config_path": loaded.path.display().to_string(),
            })),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "message": "config reloaded",
            "config_path": loaded.path.display().to_string(),
            "warnings": loaded
                .diagnostics
                .warnings
                .iter()
                .map(|item| item.message.clone())
                .collect::<Vec<_>>(),
        })),
    )
}

// ---------------------------------------------------------------------------
// POST /api/restart
// ---------------------------------------------------------------------------

/// POST /api/restart - restart the service with auth and cooldown protection.
/// `GET /api/channels/plugins` — list of every channel the host
/// knows about (built-in AND subprocess-plugin), with the full
/// metadata the desktop UI needs to render a schema-driven account
/// configuration form (§11).
///
/// Returns `[{ id, display_name, version, description, state,
/// last_error?, capabilities, schema, auth_flows, accounts[] }]`.
/// Built-in channels (telegram / feishu / weixin) are synthesized
/// from the live `ChannelsConfig` via [`crate::channel_catalog`];
/// subprocess plugins come from
/// [`garyx_channels::ChannelPluginManager::subprocess_plugin_catalog`].
/// The UI treats both identically — no hardcoded per-channel
/// branching.
pub async fn list_channel_plugins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let manager = state.channel_plugin_manager();
    let subprocess = manager.lock().await.subprocess_plugin_catalog();
    let config = state.config_snapshot();
    let builtin = crate::channel_catalog::builtin_channel_catalog(&config.channels);

    // Built-in entries come first (stable display order the UI can
    // rely on without a secondary sort). Subprocess plugins append
    // in discovery order. A plugin id colliding with a built-in name
    // is prevented upstream by `ChannelDispatcherImpl::register_plugin`.
    let mut combined = builtin;
    combined.extend(subprocess);
    for entry in &mut combined {
        entry.project_account_configs_through_schema();
    }

    Json(json!({
        "ok": true,
        "plugins": combined,
    }))
}

/// Body of `POST /api/channels/plugins/:id/auth_flow/start`.
///
/// `form_state` carries whatever the user has typed into the
/// JSON-Schema form so far — the plugin picks the fields it needs,
/// applies its own defaults for the rest, and kicks off its
/// internal state machine. Sending `{}` is valid: the plugin runs
/// with full defaults (this is how a pristine "Click to auto-login"
/// button works).
#[derive(Debug, Clone, Deserialize)]
pub struct AuthFlowStartBody {
    #[serde(default)]
    pub form_state: Value,
}

/// Body of `POST /api/channels/plugins/:id/auth_flow/poll`.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthFlowPollBody {
    pub session_id: String,
}

/// Body of `POST /api/channels/plugins/:id/validate_account`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelAccountValidationBody {
    pub account_id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub config: Value,
}

fn default_enabled() -> bool {
    true
}

/// `POST /api/channels/plugins/{id}/validate_account`.
///
/// Validates one account payload before the desktop persists it into
/// `~/.garyx/garyx.json`. Built-ins perform real provider probes when
/// safe; plugins without a validator return `validated=false` so callers
/// can distinguish a real check from a deliberate skip.
pub async fn channel_account_validate(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
    Json(body): Json<ChannelAccountValidationBody>,
) -> impl IntoResponse {
    let account_id = body.account_id.trim();
    if account_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "reason": "invalid_account",
                "message": "account_id is required",
            })),
        );
    }

    let plugin = {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        guard.plugin(&plugin_id)
    };
    let Some(plugin) = plugin else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "reason": "unknown_plugin",
                "message": format!("plugin '{plugin_id}' is not registered"),
            })),
        );
    };

    let account = garyx_channels::plugin_host::AccountDescriptor {
        id: account_id.to_owned(),
        enabled: body.enabled,
        config: body.config,
    };
    match plugin.validate_account_config(account).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "validated": result.validated,
                "message": result.message,
            })),
        ),
        Err(message) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "reason": "validation_failed",
                "message": message,
            })),
        ),
    }
}

/// `POST /api/channels/plugins/{id}/auth_flow/start`.
///
/// Starts an auto-login session against the plugin identified by
/// `{id}` (canonical id or alias — feishu accepts `lark` etc.). On
/// success returns the plugin's `AuthSession` with a rendered
/// display list, session id, TTL, and poll cadence; the desktop
/// client then polls `/poll` at `poll_interval_secs` until Confirmed
/// or Failed.
///
/// Returns 404 when the plugin is unknown or doesn't support an
/// auto-login path (its `config_methods` didn't include
/// `AutoLogin`). Returns 400 on transport / protocol failures the
/// executor couldn't recover from so the UI stops polling.
pub async fn channel_auth_flow_start(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
    Json(body): Json<AuthFlowStartBody>,
) -> impl IntoResponse {
    let executor = {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        guard.auth_flow_executor(&plugin_id)
    };
    let Some(executor) = executor else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "reason": "no_auth_flow",
                "message": format!(
                    "plugin '{plugin_id}' does not expose an auto-login flow"
                ),
            })),
        );
    };

    match executor.start(body.form_state).await {
        Ok(session) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "session_id": session.session_id,
                "display": session.display,
                "expires_in_secs": session.expires_in_secs,
                "poll_interval_secs": session.poll_interval_secs,
            })),
        ),
        Err(err) => (
            auth_flow_err_status(&err),
            Json(json!({
                "ok": false,
                "reason": "start_failed",
                "message": err.to_string(),
            })),
        ),
    }
}

/// `POST /api/channels/plugins/{id}/auth_flow/poll`.
///
/// Advances the named session by one tick. Returns the executor's
/// 3-state outcome verbatim (`pending` / `confirmed` / `failed`).
/// Unknown session_id surfaces as 404.
pub async fn channel_auth_flow_poll(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
    Json(body): Json<AuthFlowPollBody>,
) -> impl IntoResponse {
    let executor = {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        guard.auth_flow_executor(&plugin_id)
    };
    let Some(executor) = executor else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "reason": "no_auth_flow",
                "message": format!(
                    "plugin '{plugin_id}' does not expose an auto-login flow"
                ),
            })),
        );
    };

    match executor.poll(&body.session_id).await {
        Ok(result) => (
            StatusCode::OK,
            Json(
                serde_json::to_value(&result)
                    .map(|mut v| {
                        if let Value::Object(map) = &mut v {
                            map.insert("ok".into(), Value::Bool(true));
                        }
                        v
                    })
                    .unwrap_or_else(|_| json!({ "ok": true })),
            ),
        ),
        Err(err) => (
            auth_flow_err_status(&err),
            Json(json!({
                "ok": false,
                "reason": "poll_failed",
                "message": err.to_string(),
            })),
        ),
    }
}

/// Map an `AuthFlowError` to the right HTTP status so the desktop
/// can tell "I sent a bad session id" (404) from "the plugin died"
/// (502) from "the plugin's reply didn't parse" (500). Kept local
/// so the two handlers share exactly one mapping.
fn auth_flow_err_status(err: &garyx_channels::auth_flow::AuthFlowError) -> StatusCode {
    use garyx_channels::auth_flow::AuthFlowError;
    match err {
        AuthFlowError::UnknownSession(_) => StatusCode::NOT_FOUND,
        AuthFlowError::InvalidArgs(_) => StatusCode::BAD_REQUEST,
        AuthFlowError::Transport(_) => StatusCode::BAD_GATEWAY,
        AuthFlowError::Protocol(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn restart(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    // Authorization check: if restart_tokens are configured, require a valid token.
    if !state.ops.restart_tokens.is_empty() {
        let provided_token = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.strip_prefix("Bearer ").unwrap_or(s))
            .unwrap_or("");

        if !state
            .ops
            .restart_tokens
            .iter()
            .any(|t| crate::gateway_auth::constant_time_eq(t.as_bytes(), provided_token.as_bytes()))
        {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "ok": false,
                    "reason": "unauthorized",
                    "message": "valid authorization token required for restart",
                })),
            );
        }
    }

    let mut tracker = state.ops.restart_tracker.lock().await;

    // Cooldown check
    if let Some(remaining) = tracker.cooldown_remaining_secs(RESTART_COOLDOWN_SECS) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "ok": false,
                "reason": "cooldown",
                "message": format!("restart cooldown active, try again in {remaining}s"),
                "cooldown_remaining_secs": remaining,
            })),
        );
    }

    tracker.mark_restart_now();
    drop(tracker);

    if let Err(e) = crate::restart::request_restart("api".to_owned()).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "reason": "restart_failed",
                "message": format!("failed to initiate restart: {e}"),
            })),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "message": "restart initiated",
        })),
    )
}

// ---------------------------------------------------------------------------
// POST /api/send — lightweight outbound message endpoint
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SendPayload {
    /// Bot selector: `channel:account_id`, e.g. `telegram:main`.
    pub bot: String,
    /// Message text.
    #[serde(default)]
    pub text: Option<String>,
}

pub async fn send_message(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<SendPayload>,
) -> impl IntoResponse {
    // Auth — reuse restart_tokens if configured.
    if !state.ops.restart_tokens.is_empty() {
        let provided_token = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.strip_prefix("Bearer ").unwrap_or(s))
            .unwrap_or("");

        if !state
            .ops
            .restart_tokens
            .iter()
            .any(|t| crate::gateway_auth::constant_time_eq(t.as_bytes(), provided_token.as_bytes()))
        {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "ok": false, "error": "unauthorized" })),
            );
        }
    }

    let text = payload.text.unwrap_or_default();
    if text.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "text is required" })),
        );
    }

    // Parse bot selector.
    let Some((channel, account_id)) = payload.bot.split_once(':') else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "bot must be `channel:account_id`" })),
        );
    };

    // Resolve main endpoint for the bot.
    let Some(endpoint) =
        crate::routes::resolve_main_endpoint_by_bot(&state, channel, account_id).await
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(
                json!({ "ok": false, "error": format!("no main endpoint for bot '{}'", payload.bot) }),
            ),
        );
    };

    // Send.
    let dispatcher = state.channel_dispatcher();
    let result = dispatcher
        .send_message(garyx_channels::OutboundMessage {
            channel: endpoint.channel.clone(),
            account_id: endpoint.account_id.clone(),
            chat_id: endpoint.chat_id.clone(),
            delivery_target_type: endpoint.delivery_target_type.clone(),
            delivery_target_id: endpoint.delivery_target_id.clone(),
            content: ChannelOutboundContent::text(text),
            reply_to: None,
            thread_id: endpoint.delivery_thread_id.clone(),
        })
        .await;

    match result {
        Ok(res) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "message_ids": res.message_ids,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("{e}") })),
        ),
    }
}

fn collect_unknown_fields(
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
    Json(json!({
        "agents": state.ops.custom_agents.list_agents().await,
    }))
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
        Some(agent) => (StatusCode::OK, Json(json!(agent))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "custom agent not found" })),
        )
            .into_response(),
    }
}

pub async fn create_custom_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CustomAgentUpsertPayload>,
) -> impl IntoResponse {
    match state
        .ops
        .custom_agents
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id: payload.agent_id,
            display_name: payload.display_name,
            provider_type: payload.provider_type,
            model: payload.model,
            system_prompt: payload.system_prompt,
        })
        .await
    {
        Ok(agent) => {
            let profiles = state.ops.custom_agents.list_agents().await;
            state
                .integration
                .bridge
                .replace_agent_profiles(profiles)
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
            (StatusCode::CREATED, Json(json!(agent))).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

pub async fn update_custom_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(payload): Json<CustomAgentUpsertPayload>,
) -> impl IntoResponse {
    match state
        .ops
        .custom_agents
        .upsert_agent(UpsertCustomAgentRequest {
            agent_id,
            display_name: payload.display_name,
            provider_type: payload.provider_type,
            model: payload.model,
            system_prompt: payload.system_prompt,
        })
        .await
    {
        Ok(agent) => {
            let profiles = state.ops.custom_agents.list_agents().await;
            state
                .integration
                .bridge
                .replace_agent_profiles(profiles)
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
            (StatusCode::OK, Json(json!(agent))).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

pub async fn delete_custom_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.custom_agents.delete_agent(&agent_id).await {
        Ok(()) => {
            let profiles = state.ops.custom_agents.list_agents().await;
            state
                .integration
                .bridge
                .replace_agent_profiles(profiles)
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
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Wiki API
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct WikiUpsertPayload {
    pub wiki_id: String,
    pub display_name: String,
    pub path: String,
    pub topic: String,
    pub agent_id: Option<String>,
}

pub async fn list_wikis(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "wikis": state.ops.wikis.list_wikis().await,
    }))
}

pub async fn get_wiki(
    State(state): State<Arc<AppState>>,
    Path(wiki_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.wikis.get_wiki(&wiki_id).await {
        Some(wiki) => (StatusCode::OK, Json(json!(wiki))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "wiki not found" })),
        )
            .into_response(),
    }
}

pub async fn create_wiki(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<WikiUpsertPayload>,
) -> impl IntoResponse {
    match state
        .ops
        .wikis
        .upsert_wiki(UpsertWikiRequest {
            wiki_id: payload.wiki_id,
            display_name: payload.display_name,
            path: payload.path,
            topic: payload.topic,
            agent_id: payload.agent_id,
        })
        .await
    {
        Ok(wiki) => (StatusCode::CREATED, Json(json!(wiki))).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

pub async fn update_wiki(
    State(state): State<Arc<AppState>>,
    Path(wiki_id): Path<String>,
    Json(payload): Json<WikiUpsertPayload>,
) -> impl IntoResponse {
    match state
        .ops
        .wikis
        .upsert_wiki(UpsertWikiRequest {
            wiki_id,
            display_name: payload.display_name,
            path: payload.path,
            topic: payload.topic,
            agent_id: payload.agent_id,
        })
        .await
    {
        Ok(wiki) => (StatusCode::OK, Json(json!(wiki))).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

pub async fn delete_wiki(
    State(state): State<Arc<AppState>>,
    Path(wiki_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.wikis.delete_wiki(&wiki_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

pub async fn list_agent_teams(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "teams": state.ops.agent_teams.list_teams().await,
    }))
}

pub async fn get_agent_team(
    State(state): State<Arc<AppState>>,
    Path(team_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.agent_teams.get_team(&team_id).await {
        Some(team) => (StatusCode::OK, Json(json!(team))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "agent team not found" })),
        )
            .into_response(),
    }
}

pub async fn create_agent_team(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AgentTeamUpsertPayload>,
) -> impl IntoResponse {
    match state
        .ops
        .agent_teams
        .upsert_team(UpsertAgentTeamRequest {
            team_id: payload.team_id,
            display_name: payload.display_name,
            leader_agent_id: payload.leader_agent_id,
            member_agent_ids: payload.member_agent_ids,
            workflow_text: payload.workflow_text,
        })
        .await
    {
        Ok(team) => {
            if let Err(error) = clear_team_deleted_markers(&state, &team.team_id).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            if let Err(error) = reconcile_team_group_state(&state, &team).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            if let Err(error) = reload_team_registry(&state).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            (StatusCode::CREATED, Json(json!(team))).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

pub async fn update_agent_team(
    State(state): State<Arc<AppState>>,
    Path(team_id): Path<String>,
    Json(payload): Json<AgentTeamUpsertPayload>,
) -> impl IntoResponse {
    match state
        .ops
        .agent_teams
        .upsert_team(UpsertAgentTeamRequest {
            team_id,
            display_name: payload.display_name,
            leader_agent_id: payload.leader_agent_id,
            member_agent_ids: payload.member_agent_ids,
            workflow_text: payload.workflow_text,
        })
        .await
    {
        Ok(team) => {
            if let Err(error) = clear_team_deleted_markers(&state, &team.team_id).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            if let Err(error) = reconcile_team_group_state(&state, &team).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            if let Err(error) = reload_team_registry(&state).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            (StatusCode::OK, Json(json!(team))).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

pub async fn delete_agent_team(
    State(state): State<Arc<AppState>>,
    Path(team_id): Path<String>,
) -> impl IntoResponse {
    match state.ops.agent_teams.delete_team(&team_id).await {
        Ok(()) => {
            if let Err(error) = mark_deleted_team_threads(&state, &team_id).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            if let Err(error) = reload_team_registry(&state).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                )
                    .into_response();
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
