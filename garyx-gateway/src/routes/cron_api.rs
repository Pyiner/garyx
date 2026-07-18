//! Cron/automation data and debug-cron handlers.

use crate::server::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
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
                "validation_state": if j.validation_error.is_some() { "invalid" } else { "valid" },
                "validation_error": j.validation_error,
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

pub(super) fn default_cron_runs_limit() -> usize {
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
pub(super) fn default_debug_runs_limit() -> usize {
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
pub(super) fn parse_since(raw: &str) -> Option<chrono::DateTime<Utc>> {
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
pub(super) fn debug_job_json(job: &crate::cron::CronJob, recent_runs: Vec<Value>) -> Value {
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
pub(super) fn debug_run_json(r: &crate::cron::RunRecord) -> Value {
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
