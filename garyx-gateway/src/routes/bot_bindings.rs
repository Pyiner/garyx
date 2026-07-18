//! Bot status and endpoint bind/unbind handlers.

use crate::server::AppState;
use crate::thread_runtime::build_thread_runtime_summary;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use garyx_router::{
    ChannelBinding, default_workspace_mode_for_channel_account, is_thread_key,
    validate_thread_accepts_bot_binding,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
// ---------------------------------------------------------------------------

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

pub(super) fn parse_bot_selector(bot_id: &str) -> Result<(&str, &str), String> {
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

pub(super) fn enrich_bot_binding_payload(
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

pub(super) async fn build_bot_status_payload(
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
