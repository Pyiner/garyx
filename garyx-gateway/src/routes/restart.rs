//! Gateway restart handler and restart cooldown tracker.

use crate::server::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
// ---------------------------------------------------------------------------

/// Tracks the last restart timestamp for cooldown enforcement.
#[derive(Default)]
pub struct RestartTracker {
    pub(super) last_restart: Option<Instant>,
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
pub(super) const RESTART_COOLDOWN_SECS: u64 = 30;

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

    if let Err(e) = (state.ops.restart_requester)("api".to_owned()).await {
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
