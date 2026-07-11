//! Additional observability and operations API endpoints.
//!
//! Supplements the existing routes in `routes.rs` and `dashboard.rs` with
//! thread history, cron data, settings mutation, and restart
//! controls.

use garyx_router::ThreadStoreExt;
use std::collections::{HashMap, HashSet};
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use garyx_channels::builtin_catalog::builtin_channel_descriptor;
use garyx_models::config_loader::{
    ConfigLoadOptions, ConfigWriteOptions, load_config, write_config_value_atomic,
};
use garyx_models::provider::{ProviderMessage, ProviderType};
use garyx_models::transcript_kind::is_tool_related_message;
use garyx_models::{ChannelOutboundContent, CustomAgentProfile};
use garyx_router::{
    ChannelBinding, ThreadHistoryError, bindings_from_value, count_user_query_messages,
    default_workspace_mode_for_channel_account, history_message_count, is_thread_key,
    validate_thread_accepts_bot_binding, workspace_dir_from_value,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::custom_agents::UpsertCustomAgentRequest;
use crate::optimistic_write::{StoreWriteError, WriteExpectation};
use crate::server::AppState;
use crate::thread_runtime::{build_thread_runtime_summary, provider_type_from_key};
use crate::thread_type::thread_summary_type_from_record;
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

fn provider_icon_descriptor(provider_type: &ProviderType) -> Option<Value> {
    let (key, label) = match provider_type {
        ProviderType::ClaudeCode | ProviderType::ClaudeLlm => ("claude", "Claude"),
        ProviderType::CodexAppServer => ("codex", "Codex"),
        ProviderType::Traex => ("traex", "Traex"),
        ProviderType::GeminiLlm => ("gemini", "Gemini"),
        ProviderType::AntigravityCli => ("gemini", "Antigravity"),
        ProviderType::Gpt => return None,
    };
    Some(json!({
        "key": key,
        "provider_type": provider_type,
        "label": label,
    }))
}

fn custom_agent_response(agent: &CustomAgentProfile) -> Value {
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
    /// Return messages before this zero-based global message index.
    #[serde(default, alias = "before")]
    pub before_index: Option<usize>,
    /// Return a page containing this many human user query turns.
    #[serde(default, alias = "limit_user_queries", alias = "user_turn_limit")]
    pub user_query_limit: Option<usize>,
    /// Return committed messages strictly after this zero-based global index
    /// (forward / delta cursor). Takes precedence over before_index and
    /// user_query_limit.
    #[serde(default, alias = "after")]
    pub after_index: Option<usize>,
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
const MAX_THREAD_HISTORY_USER_QUERY_LIMIT: usize = 50;

#[derive(Deserialize)]
pub struct ThreadDiagnosticsParams {
    pub thread_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Deserialize)]
pub struct BotStatusParams {
    pub bot_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotBindBody {
    #[serde(alias = "bot")]
    pub bot_id: String,
    pub thread_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotUnbindBody {
    #[serde(alias = "bot")]
    pub bot_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomAgentUpsertPayload {
    pub agent_id: String,
    #[serde(alias = "name")]
    pub display_name: String,
    pub provider_type: garyx_models::ProviderType,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, alias = "modelReasoningEffort")]
    pub model_reasoning_effort: Option<String>,
    #[serde(default, alias = "modelServiceTier")]
    pub model_service_tier: Option<String>,
    #[serde(default, alias = "env", alias = "providerEnv")]
    pub provider_env: Option<HashMap<String, String>>,
    #[serde(default, alias = "authSource")]
    pub auth_source: Option<String>,
    #[serde(default, alias = "baseUrl")]
    pub base_url: Option<String>,
    #[serde(default, alias = "codexHome")]
    pub codex_home: Option<String>,
    #[serde(default, alias = "maxToolIterations")]
    pub max_tool_iterations: Option<u32>,
    #[serde(default, alias = "requestTimeoutSeconds")]
    pub request_timeout_seconds: Option<u32>,
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

/// GET /api/threads/history - thread history with optional filtering.
pub async fn thread_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ThreadHistoryParams>,
) -> axum::response::Response {
    if let Some(thread_id) = params.thread_id.as_deref() {
        let payload = thread_history_for_key(
            &state,
            thread_id,
            params.limit,
            params.include_tool_messages,
            params.before_index,
            params.user_query_limit,
            params.after_index,
        )
        .await;
        return Json(payload).into_response();
    }

    // List mode is a request boundary: a store failure must surface as a
    // 500, never as a successful empty listing (#TASK-2099).
    match thread_history_listing(&state, &params).await {
        Ok(payload) => Json(payload).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "reason": "thread-store-error",
                "error": error.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn thread_history_listing(
    state: &Arc<AppState>,
    params: &ThreadHistoryParams,
) -> Result<Value, garyx_router::ThreadStoreError> {
    let keys = state
        .threads
        .thread_store
        .list_keys(params.prefix.as_deref())
        .await?;
    let keys: Vec<String> = keys.into_iter().filter(|key| is_thread_key(key)).collect();

    let limited_keys: Vec<&String> = keys.iter().take(params.limit).collect();
    let mut threads = Vec::new();

    for key in &limited_keys {
        if params.include_messages {
            // A missing record (deleted between list and get) is Ok(None)
            // and keeps its `data: null` row; only real store failures
            // abort the listing.
            let data = state.threads.thread_store.get(key).await?;
            threads.push(json!({
                "key": key,
                "data": data,
            }));
        } else {
            // Summary only: key + existence
            let exists = state.threads.thread_store.exists(key).await?;
            threads.push(json!({
                "key": key,
                "active": exists,
            }));
        }
    }

    Ok(json!({
        "threads": threads,
        "total": keys.len(),
        "limit": params.limit,
        "include_messages": params.include_messages,
    }))
}

pub async fn thread_diagnostics(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ThreadDiagnosticsParams>,
) -> impl IntoResponse {
    let thread_id = params.thread_id.trim();
    if thread_id.is_empty() {
        return Json(json!({
            "ok": false,
            "reason": "missing-thread-id",
        }));
    }

    let limit = params.limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    let thread_value = state.threads.thread_store.get_logged(thread_id).await;
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
        "thread_runtime": build_thread_runtime_summary(&state, thread_value.as_ref()).await,
        "bindings": bindings,
        "history": thread_history_for_key(&state, thread_id, limit, true, None, None, None).await,
        "message_ledger": ledger,
        "transcript_path": transcript_path,
    }))
}

pub async fn bot_status(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BotStatusParams>,
) -> axum::response::Response {
    let bot_id = params.bot_id.trim();
    if bot_id.is_empty() {
        return Json(json!({
            "ok": false,
            "reason": "missing-bot-id",
        }))
        .into_response();
    }

    match build_bot_status_payload(&state, bot_id).await {
        Ok(payload) => Json(payload).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "bot_id": bot_id,
                "reason": "thread-store-error",
                "error": error.to_string(),
            })),
        )
            .into_response(),
    }
}

pub async fn bot_bind(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BotBindBody>,
) -> impl IntoResponse {
    let requested_bot_id = body.bot_id.trim();
    let (channel, account_id) = match parse_bot_selector(requested_bot_id) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "bot_id": requested_bot_id,
                    "reason": "invalid-bot-id",
                    "error": error,
                })),
            );
        }
    };
    let bot_id = format!("{channel}:{account_id}");

    let thread_id = body.thread_id.trim();
    if thread_id.is_empty() || !is_thread_key(thread_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "bot_id": &bot_id,
                "thread_id": thread_id,
                "reason": "thread-not-found",
                "error": "thread not found",
            })),
        );
    }
    let thread_data = match state.threads.thread_store.get(thread_id).await {
        Ok(Some(thread_data)) => thread_data,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "ok": false,
                    "bot_id": &bot_id,
                    "thread_id": thread_id,
                    "reason": "thread-not-found",
                    "error": "thread not found",
                })),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "bot_id": &bot_id,
                    "thread_id": thread_id,
                    "reason": "thread-store-error",
                    "error": error.to_string(),
                })),
            );
        }
    };

    let endpoint =
        match crate::routes::resolve_main_endpoint_by_bot(&state, channel, account_id).await {
            Ok(Some(endpoint)) => endpoint,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "ok": false,
                        "bot_id": &bot_id,
                        "channel": channel,
                        "account_id": account_id,
                        "reason": "main-endpoint-unresolved",
                        "error": format!("bot '{bot_id}' has no resolved main endpoint"),
                    })),
                );
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "ok": false,
                        "bot_id": &bot_id,
                        "channel": channel,
                        "account_id": account_id,
                        "reason": "thread-store-error",
                        "error": error.to_string(),
                    })),
                );
            }
        };

    if let Err(error) =
        validate_thread_accepts_bot_binding(thread_id, &thread_data, channel, account_id)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "bot_id": &bot_id,
                "thread_id": thread_id,
                "reason": "thread-not-compatible",
                "error": error,
            })),
        );
    }

    let result = match crate::routes::bind_channel_endpoint_key_to_thread(
        &state,
        &endpoint.endpoint_key,
        thread_id,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return (
                error.status,
                Json(json!({
                    "ok": false,
                    "bot_id": &bot_id,
                    "thread_id": thread_id,
                    "reason": "bind-failed",
                    "error": error.message,
                })),
            );
        }
    };

    let mut payload = match build_bot_status_payload(&state, &bot_id).await {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "bot_id": &bot_id,
                    "reason": "thread-store-error",
                    "error": error.to_string(),
                })),
            );
        }
    };
    enrich_bot_binding_payload(
        &mut payload,
        "bind",
        Some(&result.thread_id),
        result.previous_thread_id.as_deref(),
        &result.endpoint_key,
        Some(&result.binding),
    );
    (StatusCode::OK, Json(payload))
}

pub async fn bot_unbind(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BotUnbindBody>,
) -> impl IntoResponse {
    let requested_bot_id = body.bot_id.trim();
    let (channel, account_id) = match parse_bot_selector(requested_bot_id) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "bot_id": requested_bot_id,
                    "reason": "invalid-bot-id",
                    "error": error,
                })),
            );
        }
    };
    let bot_id = format!("{channel}:{account_id}");

    let endpoint =
        match crate::routes::resolve_main_endpoint_by_bot(&state, channel, account_id).await {
            Ok(Some(endpoint)) => endpoint,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "ok": false,
                        "bot_id": &bot_id,
                        "channel": channel,
                        "account_id": account_id,
                        "reason": "main-endpoint-unresolved",
                        "error": format!("bot '{bot_id}' has no resolved main endpoint"),
                    })),
                );
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "ok": false,
                        "bot_id": &bot_id,
                        "channel": channel,
                        "account_id": account_id,
                        "reason": "thread-store-error",
                        "error": error.to_string(),
                    })),
                );
            }
        };

    let result =
        match crate::routes::detach_channel_endpoint_key(&state, &endpoint.endpoint_key).await {
            Ok(result) => result,
            Err(error) => {
                return (
                    error.status,
                    Json(json!({
                        "ok": false,
                        "bot_id": &bot_id,
                        "reason": "unbind-failed",
                        "error": error.message,
                    })),
                );
            }
        };

    let mut payload = match build_bot_status_payload(&state, &bot_id).await {
        Ok(payload) => payload,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "bot_id": &bot_id,
                    "reason": "thread-store-error",
                    "error": error.to_string(),
                })),
            );
        }
    };
    enrich_bot_binding_payload(
        &mut payload,
        "unbind",
        None,
        result.previous_thread_id.as_deref(),
        &result.endpoint_key,
        result.binding.as_ref(),
    );
    (StatusCode::OK, Json(payload))
}

fn parse_bot_selector(bot_id: &str) -> Result<(&str, &str), String> {
    let bot_id = bot_id.trim();
    let Some((channel, account_id)) = bot_id.split_once(':') else {
        return Err("bot_id must be `channel:account_id`".to_owned());
    };
    let channel = channel.trim();
    let account_id = account_id.trim();
    if channel.is_empty() || account_id.is_empty() {
        return Err("bot_id must be `channel:account_id`".to_owned());
    }
    Ok((channel, account_id))
}

fn enrich_bot_binding_payload(
    payload: &mut Value,
    action: &str,
    thread_id: Option<&str>,
    previous_thread_id: Option<&str>,
    endpoint_key: &str,
    binding: Option<&ChannelBinding>,
) {
    let Some(obj) = payload.as_object_mut() else {
        return;
    };
    obj.insert("action".to_owned(), Value::String(action.to_owned()));
    obj.insert(
        "thread_id".to_owned(),
        thread_id
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null),
    );
    obj.insert(
        "previous_thread_id".to_owned(),
        previous_thread_id
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null),
    );
    obj.insert(
        "endpoint_key".to_owned(),
        Value::String(endpoint_key.to_owned()),
    );
    obj.insert(
        "binding".to_owned(),
        binding
            .and_then(|value| serde_json::to_value(value).ok())
            .unwrap_or(Value::Null),
    );
}

async fn build_bot_status_payload(
    state: &Arc<AppState>,
    bot_id: &str,
) -> Result<Value, garyx_router::ThreadStoreError> {
    let Some((channel, account_id)) = bot_id.split_once(':') else {
        return Ok(json!({
            "ok": false,
            "bot_id": bot_id,
            "reason": "invalid-bot-id",
            "error": "bot_id must be `channel:account_id`",
        }));
    };

    // `unresolved` means the bot genuinely has no main endpoint; a store
    // failure propagates instead of masquerading as unresolved
    // (#TASK-2128), and the status response never rides a recent
    // snapshot cache hit through a live outage (#TASK-2134).
    let Some(endpoint) =
        crate::routes::resolve_main_endpoint_by_bot_fresh(state, channel, account_id).await?
    else {
        return Ok(json!({
            "ok": true,
            "bot_id": bot_id,
            "channel": channel,
            "account_id": account_id,
            "workspace_mode": default_workspace_mode_for_channel_account(
                &state.config_snapshot(),
                channel,
                account_id,
            ),
            "main_endpoint_status": "unresolved",
            "main_endpoint": Value::Null,
            "current_thread_status": "unresolved",
            "current_thread_id": Value::Null,
            "current_thread": Value::Null,
            "thread_runtime": build_thread_runtime_summary(state, None).await,
        }));
    };

    let current_thread_id = endpoint.thread_id.clone();
    // Bound thread bodies feed decision-making in this payload: a store
    // failure propagates instead of reporting the thread as missing.
    let current_thread = match current_thread_id.as_deref() {
        Some(thread_id) => state.threads.thread_store.get(thread_id).await?,
        None => None,
    };
    let current_thread_status = if current_thread_id.is_some() {
        "bound"
    } else {
        "unbound"
    };
    let default_workspace_mode =
        default_workspace_mode_for_channel_account(&state.config_snapshot(), channel, account_id);

    Ok(json!({
        "ok": true,
        "bot_id": bot_id,
        "channel": channel,
        "account_id": account_id,
        "workspace_mode": default_workspace_mode,
        "main_endpoint_status": "resolved",
        "main_endpoint": endpoint.to_value(),
        "current_thread_status": current_thread_status,
        "current_thread_id": current_thread_id,
        "current_thread": current_thread,
        "thread_runtime": build_thread_runtime_summary(state, current_thread.as_ref()).await,
    }))
}

pub(crate) async fn thread_history_for_key(
    state: &Arc<AppState>,
    thread_id: &str,
    limit: usize,
    include_tool_messages: bool,
    before_index: Option<usize>,
    user_query_limit: Option<usize>,
    after_index: Option<usize>,
) -> Value {
    let key = thread_id.trim();
    if key.is_empty() {
        let thread = Value::Null;
        return json!({
            "ok": false,
            "reason": "missing-thread-id",
            "thread": thread,
            "session": thread,
            "thread_runtime": Value::Null,
            "messages": [],
            "pending_user_inputs": [],
            "outbound_deliveries": [],
            "message_stats": { "returned_messages": 0 },
        });
    }

    let bounded_limit = limit.clamp(1, MAX_THREAD_HISTORY_LIMIT);
    let bounded_user_query_limit =
        user_query_limit.map(|value| value.clamp(1, MAX_THREAD_HISTORY_USER_QUERY_LIMIT));
    // When the client sends both an `after_index` cursor and a `user_query_limit`,
    // bound the catch-up to the newest N user turns: fetch that window once, then if
    // the cursor is older than it, return the window and flag `reset` so the client
    // overwrites its cache instead of replaying the whole delta; otherwise trim to the
    // forward delta within the window.
    let mut reset_to_newest = false;
    let snapshot_result = if let (Some(after_index), Some(user_query_limit)) =
        (after_index, bounded_user_query_limit)
    {
        match state
            .threads
            .history
            .thread_snapshot_user_query_page(key, bounded_limit, None, user_query_limit)
            .await
        {
            Ok(mut snapshot) => {
                let floor = snapshot.committed_start_index;
                if after_index + 1 < floor {
                    reset_to_newest = true;
                } else {
                    let drop = (after_index + 1 - floor).min(snapshot.committed_messages.len());
                    snapshot.committed_messages.drain(0..drop);
                    snapshot.committed_start_index = after_index + 1;
                }
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    } else if let Some(after_index) = after_index {
        state
            .threads
            .history
            .thread_snapshot_after_index(key, after_index, bounded_limit)
            .await
    } else if let Some(user_query_limit) = bounded_user_query_limit {
        state
            .threads
            .history
            .thread_snapshot_user_query_page(key, bounded_limit, before_index, user_query_limit)
            .await
    } else {
        state
            .threads
            .history
            .thread_snapshot_page(key, bounded_limit, before_index)
            .await
    };
    let snapshot = match snapshot_result {
        Ok(snapshot) => snapshot,
        Err(ThreadHistoryError::ThreadNotFound(_)) => {
            let thread = json!({ "thread_id": key, "thread_key": key });
            return json!({
                "ok": false,
                "reason": "thread-not-found",
                "thread": thread,
                "session": thread,
                "thread_runtime": Value::Null,
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
                "thread_runtime": Value::Null,
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
                "thread_runtime": Value::Null,
                "messages": [],
                "pending_user_inputs": [],
                "outbound_deliveries": [],
                "message_stats": { "returned_messages": 0 },
            });
        }
    };
    let messages = snapshot.combined_messages();
    let total_messages = snapshot.total_messages();

    let mut history_messages = Vec::new();
    let mut kind_counts = serde_json::Map::new();
    let mut role_counts = serde_json::Map::new();
    let mut tool_related_count = 0_u64;
    let mut likely_user_visible_count = 0_u64;

    for (idx, message) in messages.iter().enumerate() {
        let Some(obj) = message.as_object() else {
            continue;
        };
        let global_index = snapshot.message_index_at(idx);

        let role = obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .trim()
            .to_ascii_lowercase();

        let is_control = garyx_models::is_control_message(obj);
        let tool_related = if is_control {
            false
        } else {
            is_tool_related_message(&role, obj)
        };
        if !include_tool_messages && tool_related {
            continue;
        }

        let kind = garyx_models::resolve_message_kind_for_object(&role, obj, tool_related);
        let likely_user_visible = matches!(kind, "user_input" | "assistant_reply");
        if tool_related {
            tool_related_count += 1;
        }
        if likely_user_visible {
            likely_user_visible_count += 1;
        }
        increment_counter(&mut kind_counts, kind);
        increment_counter(&mut role_counts, &role);

        let normalized_message = (!is_control)
            .then(|| ProviderMessage::from_value(message))
            .flatten();
        let raw_content = normalized_message
            .as_ref()
            .map(|entry| entry.content.clone())
            .or_else(|| obj.get("content").cloned())
            .unwrap_or(Value::Null);
        let content = if is_control {
            Value::String(String::new())
        } else {
            enrich_message_content_for_history(&raw_content)
        };
        let message_value = if let Some(mut entry) = normalized_message.clone() {
            entry.content = content.clone();
            entry.to_json_value()
        } else if is_control {
            message.clone()
        } else {
            let mut value = message.clone();
            if let Some(map) = value.as_object_mut() {
                map.insert("content".to_owned(), content.clone());
            }
            value
        };
        let text = if is_control {
            String::new()
        } else {
            normalized_message
                .as_ref()
                .and_then(|entry| entry.text.clone())
                .unwrap_or_else(|| stringify_message_content(&content))
        };
        history_messages.push(json!({
            "index": global_index,
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

    let returned_messages = history_messages.len();
    let returned_user_queries = count_user_query_messages(&messages);
    let page_start_index = snapshot.first_message_index().unwrap_or(0);
    let page_end_index = messages
        .len()
        .checked_sub(1)
        .map(|offset| snapshot.message_index_at(offset) + 1)
        .unwrap_or(page_start_index);
    let has_more_before = page_start_index > 0;
    let next_before_index = if has_more_before {
        Value::Number(serde_json::Number::from(page_start_index as u64))
    } else {
        Value::Null
    };
    // An empty page means caught up: page_end_index falls back to page_start_index
    // (0 when nothing is returned), so guard on a non-empty page or a caught-up
    // client would be told "more after, resume at 0" → full re-fetch loop.
    let has_more_after = !messages.is_empty() && page_end_index < total_messages;
    let next_after_index = if has_more_after {
        Value::Number(serde_json::Number::from(
            page_end_index.saturating_sub(1) as u64
        ))
    } else {
        Value::Null
    };
    let mut data_raw = snapshot.thread_data;

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
    if pending_inputs_repaired && let Some(obj) = data_raw.as_object_mut() {
        obj.insert(
            "pending_user_inputs".to_owned(),
            Value::Array(persisted_pending_user_inputs),
        );
        state
            .threads
            .thread_store
            .set_logged(key, data_raw.clone())
            .await;
    }
    let thread = summarize_thread(key, &data_raw, &messages);
    json!({
        "ok": true,
        "thread": thread,
        "session": thread,
        "thread_runtime": build_thread_runtime_summary(state, Some(&data_raw)).await,
        "messages": history_messages,
        "pending_user_inputs": pending_user_inputs,
        "message_stats": {
            "total_messages_in_thread": total_messages,
            "total_messages_in_session": total_messages,
            "committed_message_count": snapshot.total_committed_messages,
            "returned_messages": returned_messages,
            "returned_user_queries": returned_user_queries,
            "returned_start_index": page_start_index,
            "returned_end_index": page_end_index,
            "has_more_before": has_more_before,
            "next_before_index": next_before_index,
            "has_more_after": has_more_after,
            "next_after_index": next_after_index,
            "reset": reset_to_newest,
            "user_query_limit": bounded_user_query_limit,
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

fn summarize_thread(thread_id: &str, data: &Value, messages: &[Value]) -> Value {
    let message_count = history_message_count(data);
    let thread_type = thread_summary_type_from_record(data);

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
        "thread_type": thread_type,
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

fn trimmed_string_field<'a>(
    block: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| block.get(*key).and_then(Value::as_str).map(str::trim))
        .find(|value| !value.is_empty())
}

fn has_inline_image_source(block: &serde_json::Map<String, Value>) -> bool {
    block.get("url").and_then(Value::as_str).is_some()
        || block
            .get("source")
            .and_then(Value::as_object)
            .and_then(|source| source.get("data"))
            .and_then(Value::as_str)
            .is_some()
}

fn history_image_media_type(block: &serde_json::Map<String, Value>, path: &str) -> String {
    trimmed_string_field(
        block,
        &[
            "media_type",
            "mediaType",
            "mime_type",
            "mimeType",
            "contentType",
        ],
    )
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
    .unwrap_or_else(|| "image/jpeg".to_owned())
}

fn image_source_for_history_path(
    block: &serde_json::Map<String, Value>,
    path: &str,
) -> Option<Value> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return None;
    };
    if !metadata.is_file() || metadata.len() > HISTORY_IMAGE_INLINE_MAX_BYTES {
        return None;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return None;
    };

    let media_type = history_image_media_type(block, path);
    Some(json!({
        "type": "base64",
        "media_type": media_type,
        "data": BASE64.encode(bytes),
    }))
}

fn enrich_image_block_for_history(block: &serde_json::Map<String, Value>) -> Value {
    if has_inline_image_source(block) {
        return Value::Object(block.clone());
    }

    let Some(path) = trimmed_string_field(block, &["path"]) else {
        return Value::Object(block.clone());
    };
    let Some(source) = image_source_for_history_path(block, path) else {
        return Value::Object(block.clone());
    };

    let mut hydrated = block.clone();
    hydrated.insert("source".to_owned(), source);
    Value::Object(hydrated)
}

/// Upper bound on any single text/string segment embedded in a thread-history
/// `message` payload. Large tool inputs/outputs (file reads, command output,
/// agent reports) can be hundreds of KB each; clipping them here keeps the
/// history response small without affecting the already-capped `text`/`content`
/// summary fields. Images are never clipped (see below).
const MAX_HISTORY_CONTENT_TEXT_CHARS: usize = 8000;

fn cap_history_text(text: &str) -> Value {
    let total = text.chars().count();
    if total <= MAX_HISTORY_CONTENT_TEXT_CHARS {
        return Value::String(text.to_owned());
    }
    let kept: String = text.chars().take(MAX_HISTORY_CONTENT_TEXT_CHARS).collect();
    Value::String(format!(
        "{kept}\n[truncated: showing {MAX_HISTORY_CONTENT_TEXT_CHARS} of {total} chars]"
    ))
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
            // Images are hydrated to inline base64 and must not be length-capped.
            "image" => enrich_image_block_for_history(block),
            // Recurse into other objects so nested tool text is capped too.
            _ => Value::Object(
                block
                    .iter()
                    .map(|(key, value)| (key.clone(), enrich_message_content_for_history(value)))
                    .collect(),
            ),
        },
        Value::String(text) => cap_history_text(text),
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
// GET /api/debug/system-cron-jobs
// ---------------------------------------------------------------------------
//
// Debug observability for system-managed cron jobs (AXON-692). The default
// user-facing `GET /api/cron/jobs` filters `system == true` jobs out, so
// `schedule_followup`-created followups are invisible there. When an incident
// like "agent promised a followup but it never fired" needs triage, SREs /
// developers reach for this endpoint to see the pending system jobs and each
// job's recent RunRecord history.
//
// Auth: registered under the protected router, so `enforce_gateway_auth`
// already gates it — loopback requests pass, everything else needs a valid
// gateway token. It reuses the existing gateway token rather than introducing
// a separate debug-token config surface. It is never exposed unauthenticated
// to non-loopback callers.

/// Default number of recent RunRecords attached to each job.
fn default_debug_runs_limit() -> usize {
    20
}

#[derive(Deserialize)]
pub struct DebugSystemCronParams {
    /// Optional thread filter. Matches `CronJob.thread_id` exactly. An empty
    /// or whitespace-only value is ignored (returns all system jobs) rather
    /// than matching jobs whose `thread_id` is unset.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Optional lower bound on job `created_at`. Accepts either a unix-second
    /// timestamp (all digits) or an RFC3339 datetime. Jobs created strictly
    /// before this instant are filtered out. A value that parses as neither
    /// form yields `400`, never a silent full list.
    #[serde(default)]
    pub since: Option<String>,
    /// Max recent RunRecords attached per job (most-recent-first).
    #[serde(default = "default_debug_runs_limit")]
    pub runs_limit: usize,
}

/// Parse a `since` query value as a unix-second timestamp or RFC3339 datetime.
fn parse_since(raw: &str) -> Option<chrono::DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(secs) = trimmed.parse::<i64>() {
        return chrono::DateTime::from_timestamp(secs, 0);
    }
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Render a single system cron job (plus its recent runs) into the debug shape.
fn debug_job_json(job: &crate::cron::CronJob, recent_runs: Vec<Value>) -> Value {
    let kind = match &job.kind {
        garyx_models::config::CronJobKind::AutomationPrompt => {
            json!({ "type": "automation_prompt" })
        }
        garyx_models::config::CronJobKind::InternalDispatch { payload } => json!({
            "type": "internal_dispatch",
            "reason": payload.reason,
            "originating_run_id": payload.originating_run_id,
            "scheduled_at": payload.scheduled_at.to_rfc3339(),
            "delay_seconds_requested": payload.delay_seconds_requested,
        }),
    };
    json!({
        "id": job.id,
        "label": job.label,
        "kind": kind,
        "schedule": job.schedule,
        "thread_id": job.thread_id,
        "agent_id": job.agent_id,
        "enabled": job.enabled,
        "system": job.system,
        "delete_after_run": job.delete_after_run,
        "next_run": job.next_run.to_rfc3339(),
        "last_status": job.last_status,
        "run_count": job.run_count,
        "created_at": job.created_at.to_rfc3339(),
        "last_run_at": job.last_run_at.map(|t| t.to_rfc3339()),
        "recent_runs": recent_runs,
    })
}

/// Render a RunRecord into JSON (mirrors the `cron_runs` shape, adds thread_id).
fn debug_run_json(r: &crate::cron::RunRecord) -> Value {
    json!({
        "run_id": r.run_id,
        "job_id": r.job_id,
        "status": r.status,
        "started_at": r.started_at.to_rfc3339(),
        "finished_at": r.finished_at.map(|t| t.to_rfc3339()),
        "duration_ms": r.duration_ms,
        "thread_id": r.thread_id,
        "error": r.error,
    })
}

/// GET /api/debug/system-cron-jobs - list system cron jobs + RunRecord history.
pub async fn debug_system_cron_jobs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<DebugSystemCronParams>,
) -> impl IntoResponse {
    let cron = match &state.ops.cron_service {
        Some(svc) => svc,
        None => {
            return Json(json!({
                "jobs": [],
                "count": 0,
                "service_available": false,
            }))
            .into_response();
        }
    };

    // Parse `since` up front so a bad value fails loudly instead of returning
    // an unfiltered list that an SRE might misread as "no jobs since X".
    let since = match params.since.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => match parse_since(raw) {
            Some(ts) => Some(ts),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "invalid_since",
                        "message": "since must be a unix-second timestamp or an RFC3339 datetime",
                        "got": raw,
                    })),
                )
                    .into_response();
            }
        },
        _ => None,
    };

    let thread_filter = params
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut jobs: Vec<Value> = Vec::new();
    for job in cron.list_all().await.into_iter().filter(|j| j.system) {
        if let Some(tid) = thread_filter
            && job.thread_id.as_deref() != Some(tid)
        {
            continue;
        }
        if let Some(since_ts) = since
            && job.created_at < since_ts
        {
            continue;
        }
        let recent_runs: Vec<Value> = cron
            .list_runs_for_job(&job.id, params.runs_limit, 0)
            .await
            .iter()
            .map(debug_run_json)
            .collect();
        jobs.push(debug_job_json(&job, recent_runs));
    }

    Json(json!({
        "jobs": jobs,
        "count": jobs.len(),
        "thread_id": thread_filter,
        "since": since.map(|t| t.to_rfc3339()),
        "runs_limit": params.runs_limit,
        "service_available": true,
    }))
    .into_response()
}

/// POST /api/debug/system-cron-jobs/{id}/run - manually fire a system cron job.
///
/// System-only wrapper around `CronService::run_now` (AXON-692 goal #3): the
/// debug channel must never be a back door to trigger user-visible automations,
/// so a non-system job (or a missing one) returns `404`. A job that exists but
/// can't run right now (disabled / already running) returns `409`.
pub async fn debug_run_system_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let cron = match &state.ops.cron_service {
        Some(svc) => svc,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "service_unavailable",
                    "message": "cron service is not running",
                })),
            )
                .into_response();
        }
    };

    match cron.get(&id).await {
        // Hide non-system jobs behind the same 404 as a missing one — the debug
        // channel only fires system jobs and must not enumerate user automations.
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "no such system cron job", "id": id })),
        )
            .into_response(),
        Some(job) if !job.system => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "no such system cron job", "id": id })),
        )
            .into_response(),
        Some(_) => match cron.run_now(&id).await {
            Some(record) => Json(json!({
                "ran": true,
                "run": debug_run_json(&record),
            }))
            .into_response(),
            None => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "not_runnable",
                    "message": "job is disabled or already running",
                    "id": id,
                })),
            )
                .into_response(),
        },
    }
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

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn collect_channel_account_config_errors(
    body: &Value,
    channel_schemas: &HashMap<String, Value>,
    scoped_accounts: Option<&HashSet<(String, String)>>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(channels) = body.get("channels").and_then(Value::as_object) else {
        return errors;
    };

    for (channel_id, channel_value) in channels {
        if channel_id == "api" {
            continue;
        }

        let Some(accounts) = channel_value.get("accounts").and_then(Value::as_object) else {
            continue;
        };

        for (account_id, account_value) in accounts {
            if let Some(scoped_accounts) = scoped_accounts {
                let key = (channel_id.clone(), account_id.clone());
                if !scoped_accounts.contains(&key) {
                    continue;
                }
            }
            let Some(account) = account_value.as_object() else {
                continue;
            };
            let path = format!("$.channels.{channel_id}.accounts.{account_id}.config");
            match account.get("config") {
                None => errors.push(format!("{path} is required for channel accounts")),
                Some(Value::Object(config)) => {
                    if let Some(schema) = channel_schemas.get(channel_id) {
                        collect_required_account_config_errors(
                            channel_id,
                            &path,
                            config,
                            schema,
                            &mut errors,
                        );
                    }
                }
                Some(value) => errors.push(format!(
                    "{path} must be a JSON object, got {}",
                    json_type_name(value)
                )),
            }
        }
    }

    errors
}

fn collect_touched_channel_accounts(body: &Value) -> HashSet<(String, String)> {
    let mut accounts = HashSet::new();
    let Some(channels) = body.get("channels").and_then(Value::as_object) else {
        return accounts;
    };

    for (channel_id, channel_value) in channels {
        if channel_id == "api" {
            continue;
        }

        let Some(channel_accounts) = channel_value.get("accounts").and_then(Value::as_object)
        else {
            continue;
        };

        for account_id in channel_accounts.keys() {
            accounts.insert((channel_id.clone(), account_id.clone()));
        }
    }

    accounts
}

fn collect_required_account_config_errors(
    channel_id: &str,
    config_path: &str,
    config: &serde_json::Map<String, Value>,
    schema: &Value,
    errors: &mut Vec<String>,
) {
    let Some(required) = schema.get("required").and_then(Value::as_array) else {
        return;
    };
    let properties = schema.get("properties").and_then(Value::as_object);

    for required_field in required.iter().filter_map(Value::as_str) {
        let field_path = format!("{config_path}.{required_field}");
        let field_schema = properties.and_then(|props| props.get(required_field));
        match config.get(required_field) {
            None => errors.push(format!("{field_path} is required by channel schema")),
            Some(Value::Null) => errors.push(format!("{field_path} must not be null")),
            Some(Value::String(value))
                if required_string_field_rejects_blank(channel_id, field_schema)
                    && value.trim().is_empty() =>
            {
                errors.push(format!("{field_path} must not be blank"));
            }
            Some(_) => {}
        }
    }
}

fn required_string_field_rejects_blank(channel_id: &str, field_schema: Option<&Value>) -> bool {
    if matches!(channel_id, "telegram" | "feishu" | "weixin") {
        return true;
    }

    matches!(
        field_schema.and_then(|schema| schema.get("type")),
        Some(Value::String(kind)) if kind == "string"
    )
}

fn builtin_channel_account_schemas() -> HashMap<String, Value> {
    ["telegram", "feishu", "weixin"]
        .into_iter()
        .filter_map(|plugin_id| {
            builtin_channel_descriptor(plugin_id)
                .map(|descriptor| (plugin_id.to_owned(), descriptor.schema()))
        })
        .collect()
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

    let merge_settings = params.merge.unwrap_or(true);
    let mut patch_body = body.clone();
    let existing_config = serde_json::to_value(state.config_snapshot()).unwrap_or_default();

    // Deep-merge by default: prevents partial payloads (e.g. `{"commands":[]}`)
    // from wiping unrelated sections. Callers sending the full document opt
    // out with `?merge=false` so deletions (e.g. removing a channel account)
    // actually take effect — additive merge has no delete semantics.
    if merge_settings {
        body = deep_merge_json(existing_config.clone(), body);
    }

    garyx_models::config_loader::strip_redundant_config_fields(&mut body);
    garyx_models::config_loader::strip_redundant_config_fields(&mut patch_body);

    let mut channel_schemas = builtin_channel_account_schemas();
    {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        for entry in guard.subprocess_plugin_catalog() {
            channel_schemas.insert(entry.id, entry.schema);
        }
    }

    let touched_channel_accounts = if merge_settings {
        Some(collect_touched_channel_accounts(&patch_body))
    } else {
        None
    };
    let account_config_errors = collect_channel_account_config_errors(
        &body,
        &channel_schemas,
        touched_channel_accounts.as_ref(),
    );
    if !account_config_errors.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": account_config_errors,
            })),
        );
    }

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
    let unknown_field_input = if merge_settings { &patch_body } else { &body };
    collect_unknown_fields("$", unknown_field_input, &normalized, &mut unknown_fields);
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
    #[serde(default)]
    pub bot: Option<String>,
    /// Message text.
    #[serde(default)]
    pub text: Option<String>,
    /// Optional local image path. When text is also provided, it is used as the caption.
    #[serde(default)]
    pub image: Option<String>,
    /// Optional local file path. When text is also provided, it is used as the caption.
    #[serde(default)]
    pub file: Option<String>,
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
    let image = payload
        .image
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let file = payload
        .file
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if image.is_some() && file.is_some() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "ok": false, "error": "message supports at most one attachment: choose image or file" }),
            ),
        );
    }
    if text.trim().is_empty() && image.is_none() && file.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "text, image, or file is required" })),
        );
    }
    if let Some(image_path) = image.as_deref() {
        let path = FsPath::new(image_path);
        if !path.is_absolute() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": "image path must be absolute" })),
            );
        }
        if !path.is_file() {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({ "ok": false, "error": format!("image file not found: {image_path}") }),
                ),
            );
        }
    }
    if let Some(file_path) = file.as_deref() {
        let path = FsPath::new(file_path);
        if !path.is_absolute() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": "file path must be absolute" })),
            );
        }
        if !path.is_file() {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": format!("file not found: {file_path}") })),
            );
        }
    }

    // Parse bot selector.
    let bot = payload
        .bot
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(bot) = bot else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "bot is required" })),
        );
    };
    let Some((channel, account_id)) = bot.split_once(':') else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "bot must be `channel:account_id`" })),
        );
    };

    // Resolve main endpoint for the bot.
    let endpoint = match crate::routes::resolve_main_endpoint_by_bot(&state, channel, account_id)
        .await
    {
        Ok(Some(endpoint)) => endpoint,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "ok": false, "error": format!("no main endpoint for bot '{bot}'") })),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    json!({ "ok": false, "reason": "thread-store-error", "error": error.to_string() }),
                ),
            );
        }
    };

    // Send.
    let content = if let Some(image_path) = image {
        let caption = if text.trim().is_empty() {
            None
        } else {
            Some(text.clone())
        };
        ChannelOutboundContent::image(image_path, caption)
    } else if let Some(file_path) = file {
        let caption = if text.trim().is_empty() {
            None
        } else {
            Some(text.clone())
        };
        ChannelOutboundContent::file(file_path, caption)
    } else {
        ChannelOutboundContent::text(text)
    };
    let dispatcher = state.channel_dispatcher();
    let result = dispatcher
        .send_message(garyx_channels::OutboundMessage {
            channel: endpoint.channel.clone(),
            account_id: endpoint.account_id.clone(),
            chat_id: endpoint.chat_id.clone(),
            delivery_target_type: endpoint.delivery_target_type.clone(),
            delivery_target_id: endpoint.delivery_target_id.clone(),
            content,
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
    let agents = state
        .ops
        .custom_agents
        .list_agents()
        .await
        .into_iter()
        .map(|agent| custom_agent_response(&agent))
        .collect::<Vec<_>>();
    Json(json!({
        "agents": agents,
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
fn store_write_error_response(error: StoreWriteError) -> axum::response::Response {
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
fn require_expected_updated_at(
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
                model: payload.model,
                model_reasoning_effort: payload.model_reasoning_effort,
                model_service_tier: payload.model_service_tier,
                provider_env: payload.provider_env,
                auth_source: payload.auth_source,
                base_url: payload.base_url,
                codex_home: payload.codex_home,
                max_tool_iterations: payload.max_tool_iterations,
                request_timeout_seconds: payload.request_timeout_seconds,
                default_workspace_dir: payload.default_workspace_dir,
                avatar_data_url: payload.avatar_data_url,
                system_prompt: payload.system_prompt,
            },
            WriteExpectation::Create,
        )
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
                model: payload.model,
                model_reasoning_effort: payload.model_reasoning_effort,
                model_service_tier: payload.model_service_tier,
                provider_env: payload.provider_env,
                auth_source: payload.auth_source,
                base_url: payload.base_url,
                codex_home: payload.codex_home,
                max_tool_iterations: payload.max_tool_iterations,
                request_timeout_seconds: payload.request_timeout_seconds,
                default_workspace_dir: payload.default_workspace_dir,
                avatar_data_url: payload.avatar_data_url,
                system_prompt: payload.system_prompt,
            },
            expectation,
        )
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
