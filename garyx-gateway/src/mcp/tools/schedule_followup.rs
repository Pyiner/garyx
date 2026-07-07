//! MCP tool `mcp__garyx__schedule_followup`.
//!
//! Long-running tasks often leave the assistant with nothing to say until
//! some external work completes. Closing the turn at that point silently
//! ends the conversation — no system primitive currently re-wakes the agent
//! when the background work is ready. `schedule_followup` closes that loop:
//! the agent schedules a one-shot cron job in [`crate::cron`] and, when the
//! delay elapses, the scheduler injects a synthetic user turn back into the
//! originating thread via [`crate::internal_inbound::dispatch_internal_message_to_thread`].
//!
//! Dedupe semantics: per `(thread_id, originating_run_id)`. A second call
//! from the same agent run replaces the first; the response always reports
//! `replaced_previous` so the agent can see whether it just bumped its own
//! earlier schedule.

use chrono::{Local, Utc};
use garyx_models::config::{
    CronAction, CronJobConfig, CronJobKind, CronSchedule, InternalDispatchJobPayload,
};
use serde_json::json;

use super::super::*;

/// Inclusive lower bound on `delay_seconds`. Anything tighter belongs in
/// the in-session loop, not a persisted cron job — fits the AnyClaw heart-
/// beat tick which is multi-second already.
pub(crate) const MIN_DELAY_SECONDS: u64 = 60;

/// Inclusive upper bound on `delay_seconds`. 24h matches the longest
/// reasonable "park this thread for the night" use case the issue's grill
/// session converged on; longer durations should go through a regular
/// automation, not an ad-hoc followup.
pub(crate) const MAX_DELAY_SECONDS: u64 = 86_400;

/// Stable FNV-1a 64-bit hash. Kept local so followup dedupe does not pull in
/// unrelated tool dependencies.
fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Derive the cron job id used for `(thread_id, originating_run_id)` dedupe.
/// Two calls with the same `(thread_id, run_id)` produce the same id and
/// therefore replace each other; two calls with different `run_id`s do not.
pub(crate) fn followup_job_id(thread_id: &str, run_id: &str) -> String {
    // NUL byte separator avoids the (admittedly synthetic) collision where
    // a "thread_id::run_id" pair could match a different split of the same
    // bytes. Cron job ids land in filenames (`<id>.json`) so we keep the
    // visible form ASCII-hex.
    let mut buf = Vec::with_capacity(thread_id.len() + run_id.len() + 1);
    buf.extend_from_slice(thread_id.as_bytes());
    buf.push(0);
    buf.extend_from_slice(run_id.as_bytes());
    format!("followup_{:016x}", stable_hash64(&buf))
}

/// Render the public-facing summary of a replaced previous schedule.
/// Format an agent-facing timestamp: gateway-machine local wall-clock time
/// (`YYYY-MM-DD HH:MM:SS`, timezone implicit). Machine-facing `unix_ts`
/// fields stay timezone-neutral alongside.
fn format_local_wall_clock(instant: chrono::DateTime<chrono::Utc>) -> String {
    instant
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn previous_payload_json(previous: &crate::cron::CronJob) -> serde_json::Value {
    let scheduled_for = format_local_wall_clock(previous.next_run);
    let payload = match &previous.kind {
        CronJobKind::InternalDispatch { payload } => Some(payload),
        CronJobKind::AutomationPrompt => None,
    };
    json!({
        "schedule_id": previous.id,
        "was_scheduled_for_local": scheduled_for,
        "was_scheduled_for_unix_ts": previous.next_run.timestamp(),
        "delay_seconds_requested": payload.map(|p| p.delay_seconds_requested),
        "reason": payload.and_then(|p| p.reason.clone()),
        "originating_run_id": payload.and_then(|p| p.originating_run_id.clone()),
        "scheduled_at": payload.map(|p| format_local_wall_clock(p.scheduled_at)),
    })
}

pub(crate) async fn run(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: ScheduleFollowupParams,
) -> Result<String, String> {
    let started = Instant::now();
    let run_ctx = RunContext::from_request_context(&ctx);
    let result = run_inner(server, run_ctx, params).await;
    server.record_tool_metric(
        "schedule_followup",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result.map(|value| serde_json::to_string(&value).unwrap_or_default())
}

async fn run_inner(
    server: &GaryMcpServer,
    run_ctx: RunContext,
    params: ScheduleFollowupParams,
) -> Result<serde_json::Value, String> {
    let thread_id = run_ctx
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "schedule_followup requires a thread_id in the MCP request context".to_owned()
        })?
        .to_owned();

    let originating_run_id = run_ctx
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            "schedule_followup requires a run_id in the MCP request context".to_owned()
        })?;

    let prompt = params.prompt.trim();
    if prompt.is_empty() {
        return Err("missing required parameter: prompt".to_owned());
    }

    let delay_seconds = params.delay_seconds;
    if !(MIN_DELAY_SECONDS..=MAX_DELAY_SECONDS).contains(&delay_seconds) {
        // Reject explicitly rather than clamp — the issue spec calls this
        // out as a hard requirement so the caller's intent isn't silently
        // rewritten. The structured error code helps clients branch.
        return Err(format!(
            "delay_seconds out_of_range: must be in {MIN_DELAY_SECONDS}..={MAX_DELAY_SECONDS}, got {delay_seconds}"
        ));
    }

    let reason = params
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let cron = server
        .app_state
        .ops
        .cron_service
        .as_ref()
        .ok_or_else(|| "schedule_followup unavailable: cron service is not running".to_owned())?
        .clone();

    let now = Utc::now();
    let scheduled_for = now + chrono::Duration::seconds(delay_seconds as i64);
    let job_id = followup_job_id(&thread_id, &originating_run_id);

    let payload = InternalDispatchJobPayload {
        prompt: prompt.to_owned(),
        reason: reason.clone(),
        originating_run_id: Some(originating_run_id.clone()),
        scheduled_at: now,
        delay_seconds_requested: delay_seconds,
    };

    let cfg = CronJobConfig {
        id: job_id.clone(),
        kind: CronJobKind::InternalDispatch {
            payload: payload.clone(),
        },
        label: Some(format!("schedule_followup({})", &thread_id)),
        schedule: CronSchedule::Once {
            at: scheduled_for.to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(thread_id.clone()),
        delete_after_run: true,
        enabled: true,
        system: true,
    };

    let (new_job, previous) = cron
        .upsert(cfg)
        .await
        .map_err(|error| format!("failed to schedule followup: {error}"))?;

    Ok(json!({
        "tool": "schedule_followup",
        "status": "ok",
        "schedule_id": new_job.id,
        "scheduled_for_local": format_local_wall_clock(new_job.next_run),
        "scheduled_for_unix_ts": new_job.next_run.timestamp(),
        "thread_id": thread_id,
        "originating_run_id": originating_run_id,
        "delay_seconds_requested": delay_seconds,
        "reason": reason,
        "replaced_previous": previous
            .as_ref()
            .map(previous_payload_json)
            .unwrap_or(serde_json::Value::Null),
    }))
}
