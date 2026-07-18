//! Health, runtime info, store identity, and fallback handlers.

use super::*;

/// GET /health - basic health check
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.runtime.start_time.elapsed().as_secs();
    Json(json!({
        "status": "ok",
        "uptime_seconds": uptime,
    }))
}

/// GET /health/detailed - comprehensive health report
pub async fn health_detailed(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let report = state.runtime.health_checker.run_checks().await;
    Json(serde_json::to_value(report).unwrap_or_default())
}

/// GET /runtime - service runtime information
pub async fn runtime_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.config_snapshot();
    let uptime = state.runtime.start_time.elapsed().as_secs();
    Json(json!({
        "runtime": {
            "uptime_seconds": uptime,
            "version": env!("CARGO_PKG_VERSION"),
        },
        "gateway": {
            "host": cfg.gateway.host,
            "port": cfg.gateway.port,
        },
    }))
}

/// GET /api/store-identity - bootstrap identity for the favorites CAS domain.
pub async fn store_identity(State(state): State<Arc<AppState>>) -> axum::response::Response {
    match state.ops.garyx_db.store_incarnation_id() {
        Ok(store_incarnation_id) => Json(json!({
            "store_incarnation_id": store_incarnation_id,
            "server_boot_id": state.server_boot_id(),
        }))
        .into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/threads - list threads with pagination/filtering
// ---------------------------------------------------------------------------

/// GET /api/status - detailed system status
pub async fn system_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.runtime.start_time.elapsed().as_secs();
    let thread_count = match state.threads.thread_store.count_keys(None).await {
        Ok(count) => count,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response();
        }
    };
    let stream_drops = state.ops.events.dropped_count();
    let stream_history_size = state.ops.events.history_len().await;

    Json(json!({
        "status": "running",
        "uptime_seconds": uptime,
        "threads": {
            "count": thread_count,
        },
        "stream": {
            "drops": stream_drops,
            "history_size": stream_history_size,
        },
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

/// Fallback handler for unknown routes
pub async fn fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json::<Value>(json!({"error": "not found"})),
    )
}
