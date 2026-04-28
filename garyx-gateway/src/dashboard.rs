//! Dashboard / observability endpoints.
//!
//! Provides system overview, agent view, log tailing, settings, and SSE
//! event streaming for the Garyx gateway.

use std::convert::Infallible;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use garyx_channels::feishu::policy_block_counters_snapshot;
use garyx_models::local_paths::default_log_file_path;
use serde::Deserialize;
#[cfg(test)]
use serde_json::Value;
use serde_json::json;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::agent_team_provider::AGENT_TEAM_PROVIDER_KEY;
use crate::delivery_target::metrics_snapshot as delivery_target_metrics_snapshot;
use crate::server::AppState;

// ---------------------------------------------------------------------------
// GET /api/overview
// ---------------------------------------------------------------------------

/// System overview snapshot.
pub async fn overview(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.config_snapshot();
    let uptime_secs = state.runtime.start_time.elapsed().as_secs();
    let thread_count = state.threads.thread_store.list_keys(None).await.len();
    let stream_drops = state.ops.events.dropped_count();
    let stream_history_size = state.ops.events.history_len().await;
    let feishu_policy_blocks = policy_block_counters_snapshot();
    let mcp_metrics = state.ops.mcp_tool_metrics.snapshot();
    let delivery_target_metrics = delivery_target_metrics_snapshot();

    let active_runs = state.integration.bridge.get_active_runs().await.len();
    let provider_count = state.integration.bridge.provider_keys().await.len();

    Json(json!({
        "status": "running",
        "uptime_seconds": uptime_secs,
        "version": env!("CARGO_PKG_VERSION"),
        "gateway": {
            "host": cfg.gateway.host,
            "port": cfg.gateway.port,
            "public_url": cfg.gateway.public_url,
        },
        "threads": {
            "active": thread_count,
        },
        "providers": {
            "count": provider_count,
        },
        "channels": {
            "feishu_policy_blocks": feishu_policy_blocks,
        },
        "active_runs": active_runs,
        "stream": {
            "drops": stream_drops,
            "history_size": stream_history_size,
        },
        "mcp_metrics": mcp_metrics,
        "delivery_target_metrics": delivery_target_metrics,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/agent-view
// ---------------------------------------------------------------------------

/// Agent / provider execution view.
pub async fn agent_view(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bridge = &state.integration.bridge;

    let keys = bridge.provider_keys().await;
    let active_runs = bridge.get_active_runs().await;

    // The AgentTeam meta-provider is always registered at boot and is an
    // internal dispatch target rather than a user-configurable provider;
    // hide it from the admin view. See `agent_team_provider` module docs.
    let visible_keys: Vec<&String> = keys
        .iter()
        .filter(|key| key.as_str() != AGENT_TEAM_PROVIDER_KEY)
        .collect();

    let mut providers = Vec::new();
    for key in &visible_keys {
        let provider = bridge.get_provider(key).await;
        if let Some(p) = provider {
            let run_count = active_runs.len(); // approximate; per-provider not tracked
            providers.push(json!({
                "key": key,
                "type": format!("{:?}", p.provider_type()),
                "ready": p.is_ready(),
                "active_runs": run_count,
            }));
        }
    }

    Json(json!({
        "providers": providers,
        "bridge_ready": !visible_keys.is_empty(),
        "total_active_runs": active_runs.len(),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/logs/tail
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LogTailParams {
    /// Number of lines to return (default 100).
    #[serde(default = "default_lines")]
    pub lines: usize,
    /// Optional regex pattern to filter log lines.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Optional explicit log file path (overrides GARYX_LOG_FILE env var).
    #[serde(default)]
    pub path: Option<String>,
}

fn default_lines() -> usize {
    100
}

pub(crate) fn default_log_path() -> PathBuf {
    if let Ok(path) = std::env::var("GARYX_LOG_FILE") {
        return PathBuf::from(path);
    }
    default_log_file_path()
}

fn normalize_absolute_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }
    Some(normalized)
}

fn allowed_log_dir(base_log_path: &Path) -> PathBuf {
    base_log_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Return the last N lines from the log file, optionally filtered by a regex
/// pattern.
pub async fn logs_tail(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<LogTailParams>,
) -> impl IntoResponse {
    let base_log_path = default_log_path();
    let log_dir = allowed_log_dir(&base_log_path);
    let log_path = if let Some(path) = params.path.clone() {
        let requested = PathBuf::from(path.clone());
        let Some(normalized_requested) = normalize_absolute_path(&requested) else {
            return Json(json!({
                "error": "log path must be an absolute path",
                "path": path,
                "lines": [],
            }));
        };
        let Some(normalized_dir) = normalize_absolute_path(&log_dir) else {
            return Json(json!({
                "error": "internal log directory is not absolute",
                "path": log_dir.display().to_string(),
                "lines": [],
            }));
        };
        if !normalized_requested.starts_with(&normalized_dir) {
            return Json(json!({
                "error": "log path outside allowed log directory",
                "path": normalized_requested.display().to_string(),
                "allowed_dir": normalized_dir.display().to_string(),
                "lines": [],
            }));
        }
        normalized_requested
    } else {
        base_log_path
    };

    let content = match tokio::fs::read_to_string(&log_path).await {
        Ok(c) => c,
        Err(e) => {
            return Json(json!({
                "error": format!("failed to read log file: {e}"),
                "path": log_path.display().to_string(),
                "lines": [],
            }));
        }
    };

    let all_lines: Vec<&str> = content.lines().collect();

    // Apply regex filter if provided
    let filtered: Vec<&str> = if let Some(ref pat) = params.pattern {
        match regex::Regex::new(pat) {
            Ok(re) => all_lines.into_iter().filter(|l| re.is_match(l)).collect(),
            Err(e) => {
                return Json(json!({
                    "error": format!("invalid regex pattern: {e}"),
                    "lines": [],
                }));
            }
        }
    } else {
        all_lines
    };

    // Take last N lines
    let start = filtered.len().saturating_sub(params.lines);
    let tail: Vec<&str> = filtered[start..].to_vec();

    Json(json!({
        "path": log_path.display().to_string(),
        "total_lines": tail.len(),
        "lines": tail,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/settings
// ---------------------------------------------------------------------------

/// Return the current runtime configuration.
pub async fn settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let live_config = (*state.config_snapshot()).clone();
    Json(serde_json::to_value(&live_config).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// GET /api/stream  (Server-Sent Events)
// ---------------------------------------------------------------------------

/// SSE endpoint for real-time gateway events.
///
/// Protocol: snapshot envelope -> history replay -> live events.
/// Uses bounded broadcast channel with drop strategy for slow consumers.
/// Drop count is surfaced via `state.ops.events`.
#[derive(Deserialize)]
pub struct EventStreamParams {
    /// Number of buffered events replayed before live stream.
    #[serde(default = "default_stream_history_limit")]
    pub history_limit: usize,
}

fn default_stream_history_limit() -> usize {
    50
}

const MAX_STREAM_HISTORY_LIMIT: usize = 200;

pub async fn event_stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<EventStreamParams>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    // Build the snapshot envelope with current system state.
    let uptime_secs = state.runtime.start_time.elapsed().as_secs();
    let thread_count = state.threads.thread_store.list_keys(None).await.len();
    let snapshot = json!({
        "type": "snapshot",
        "status": "running",
        "uptime_seconds": uptime_secs,
        "threads": { "active": thread_count },
        "version": env!("CARGO_PKG_VERSION"),
    });

    // Emit snapshot as the first event.
    let snapshot_event = Event::default()
        .event("snapshot")
        .data(snapshot.to_string());

    // Replay a bounded in-memory backlog before switching to live stream.
    let history_limit = params.history_limit.min(MAX_STREAM_HISTORY_LIMIT);
    let history_events = state.ops.events.history_snapshot(history_limit).await;
    let history_event = Event::default().event("history").data(
        json!({
            "type": "history",
            "count": history_events.len(),
            "events": history_events,
        })
        .to_string(),
    );

    let rx = state.ops.events.subscribe();
    let stream = BroadcastStream::new(rx);

    // Clone the Arc so the closure owns a reference to AppState for drop counting.
    let state_for_drops = state.clone();
    let mapped = stream.filter_map(move |item| match item {
        Ok(event_data) => Some(Ok(Event::default().data(event_data))),
        Err(_) => {
            // Lagged: slow consumer. Count the drop.
            state_for_drops.ops.events.record_drop();
            None
        }
    });

    // Chain: snapshot first, then replayed history, then live events.
    let snapshot_stream = tokio_stream::once(Ok::<Event, Infallible>(snapshot_event));
    let history_stream = tokio_stream::once(Ok::<Event, Infallible>(history_event));
    let combined = snapshot_stream.chain(history_stream).chain(mapped);

    Sse::new(combined).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
