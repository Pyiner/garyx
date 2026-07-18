//! Outbound message send handler.

use crate::server::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use garyx_models::ChannelOutboundContent;
use serde::Deserialize;
use serde_json::json;
use std::path::Path as FsPath;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
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
