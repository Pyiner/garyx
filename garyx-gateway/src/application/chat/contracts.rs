use std::collections::HashMap;

use axum::Json;
use axum::http::StatusCode;
use garyx_models::provider::{FilePayload, ImagePayload, PromptAttachment, ProviderType};
use garyx_router::is_thread_key;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// WebSocket `start` payload fields.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
    #[serde(default)]
    pub images: Vec<ImagePayload>,
    #[serde(default)]
    pub files: Vec<FilePayload>,
    #[serde(default, alias = "threadId", alias = "thread_id")]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub bot: Option<String>,
    #[serde(default = "default_from_id")]
    pub from_id: String,
    #[serde(default = "default_account_id")]
    pub account_id: String,
    #[serde(default = "default_true")]
    pub wait_for_response: bool,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub provider_type: Option<ProviderType>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
    #[serde(default)]
    pub provider_metadata: HashMap<String, Value>,
}

fn default_from_id() -> String {
    "api-user".to_owned()
}

fn default_account_id() -> String {
    "main".to_owned()
}

fn default_true() -> bool {
    garyx_models::config::default_true()
}

pub fn resolve_existing_thread_key(
    thread_id: Option<String>,
) -> Result<String, (StatusCode, Json<Value>)> {
    let Some(key) = thread_id else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "threadId is required",
            })),
        ));
    };
    let trimmed = key.trim();
    if trimmed.is_empty() || !is_thread_key(trimmed) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "threadId": trimmed,
                "error": "threadId must be a canonical thread id",
            })),
        ));
    }
    Ok(trimmed.to_owned())
}

/// POST /api/chat/stream-input request body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInputRequest {
    #[serde(default, alias = "threadId", alias = "thread_id")]
    pub thread_id: Option<String>,
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
    #[serde(default)]
    pub images: Vec<ImagePayload>,
    #[serde(default)]
    pub files: Vec<FilePayload>,
}

/// POST /api/chat/stream-input response body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInputResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_input_id: Option<String>,
    pub thread_id: String,
}

/// POST /api/chat/interrupt request body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptRequest {
    #[serde(default, alias = "threadId", alias = "thread_id")]
    pub thread_id: Option<String>,
}

/// POST /api/chat/interrupt response body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptResponse {
    pub status: String,
    pub thread_id: String,
    #[serde(default)]
    pub aborted_runs: Vec<String>,
}
