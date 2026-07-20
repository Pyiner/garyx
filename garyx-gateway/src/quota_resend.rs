//! Provider quota auto-resend reactor.
//!
//! When a run terminates because the provider's rolling usage quota was
//! exhausted, the bridge commits a `run_complete` control record with
//! `status == "rate_limited"` and a `rate_limit` block carrying the
//! authoritative reset time (sourced from Codex's own
//! `account/rateLimits/updated` snapshot). This reactor watches the committed
//! event stream and, the moment such a record appears, schedules a one-shot
//! `continue` followup for when the window recovers — the exact prompt the
//! rate-limit banner's manual Continue button sends, so the auto and manual
//! recovery paths stay behaviorally identical.
//!
//! Scheduling reuses the file-backed `InternalDispatch` cron primitive so a
//! pending resend survives a gateway restart and is retried on transient
//! dispatch failures. The job is keyed per-thread, so a fresh rate-limit
//! (including one produced by a resend that hit the limit again) replaces any
//! prior pending resend — the thread keeps retrying until it gets through.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use garyx_models::config::{
    CronAction, CronJobConfig, CronJobKind, CronSchedule, InternalDispatchJobPayload,
};
use serde_json::Value;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::server::AppState;

/// Fire the resend one minute after the reported reset so the provider window
/// has actually rolled over by the time we resubmit.
const RESEND_BUFFER_SECS: i64 = 60;

/// The literal prompt dispatched when the quota window recovers. Matches the
/// rate-limit banner's manual Continue button, which sends the same literal
/// `continue` through the regular send pipeline; the followup metadata block
/// prepended by the cron dispatch path carries the "why" (the auto-resend
/// reason), so the prompt itself stays minimal.
const RESEND_PROMPT: &str = "continue";

/// How many recent events to replay after a broadcast lag, to recover any
/// `rate_limited` record that was dropped from the subscriber buffer. Matches
/// the hub's retained-history depth.
const EVENT_HISTORY_REPLAY: usize = 256;

/// Spawn the background reactor. No-op when no cron service is configured
/// (nothing can be scheduled) or when called outside a Tokio runtime.
pub(crate) fn spawn_reactor(state: Arc<AppState>) {
    if state.ops.cron_service.is_none() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    let mut rx = state.ops.events.subscribe();
    handle.spawn(async move {
        loop {
            match rx.recv().await {
                Ok(raw_event) => {
                    if let Some(plan) = parse_resend_plan(&raw_event) {
                        // Schedule inline rather than in a detached task so
                        // events for the same thread are handled in arrival
                        // order — a newer rate-limit must replace an older
                        // pending resend, and concurrent tasks could finish out
                        // of order and let the stale one win. Scheduling is a
                        // fast history read + cron upsert.
                        schedule_resend(&state, plan).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // A burst overran the subscriber buffer and a `rate_limited`
                    // record may have been dropped. Replay recent history so the
                    // resend is still scheduled; per-thread job ids make
                    // re-processing idempotent.
                    for raw_event in state
                        .ops
                        .events
                        .history_snapshot(EVENT_HISTORY_REPLAY)
                        .await
                    {
                        if let Some(plan) = parse_resend_plan(&raw_event) {
                            schedule_resend(&state, plan).await;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResendPlan {
    thread_id: String,
    run_id: String,
    provider: String,
    window: Option<String>,
    reset_at: DateTime<Utc>,
}

/// Parse a committed-event payload into a resend plan, or `None` when the event
/// is not a rate-limited `run_complete` that opted into auto-resend.
fn parse_resend_plan(raw_event: &str) -> Option<ResendPlan> {
    // Cheap prefilter: the vast majority of committed events are not terminal
    // rate-limit records, so avoid a full JSON parse unless the marker is
    // present.
    if !raw_event.contains("rate_limited") {
        return None;
    }

    let event: Value = serde_json::from_str(raw_event).ok()?;
    if event.get("type").and_then(Value::as_str) != Some("committed_message") {
        return None;
    }
    let thread_id = non_empty(event.get("thread_id").and_then(Value::as_str))?;
    let control = event.get("message").and_then(|m| m.get("control"))?;
    if control.get("kind").and_then(Value::as_str) != Some("run_complete") {
        return None;
    }
    if control.get("status").and_then(Value::as_str) != Some("rate_limited") {
        return None;
    }

    let rate_limit = control.get("rate_limit")?;
    if !rate_limit
        .get("will_auto_resend")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let reset_at = rate_limit.get("reset_at").and_then(Value::as_str)?;
    let reset_at = DateTime::parse_from_rfc3339(reset_at)
        .ok()?
        .with_timezone(&Utc);
    let run_id = non_empty(
        control
            .get("run_id")
            .and_then(Value::as_str)
            .or_else(|| event.get("run_id").and_then(Value::as_str)),
    )?;
    let provider = rate_limit
        .get("provider")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("codex")
        .to_owned();
    let window = rate_limit
        .get("window")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned);

    Some(ResendPlan {
        thread_id,
        run_id,
        provider,
        window,
        reset_at,
    })
}

async fn schedule_resend(state: &Arc<AppState>, plan: ResendPlan) {
    let Some(cron) = state.ops.cron_service.as_ref().cloned() else {
        return;
    };

    let now = Utc::now();
    let earliest = now + Duration::seconds(RESEND_BUFFER_SECS);
    let fire_at = (plan.reset_at + Duration::seconds(RESEND_BUFFER_SECS)).max(earliest);
    let delay_seconds = (fire_at - now).num_seconds().max(0) as u64;

    let reason = match &plan.window {
        Some(window) => format!(
            "{} usage-limit auto-resend after {} quota window reset",
            plan.provider, window
        ),
        None => format!(
            "{} usage-limit auto-resend after quota reset",
            plan.provider
        ),
    };

    let cfg = CronJobConfig {
        id: resend_job_id(&plan.thread_id),
        kind: CronJobKind::InternalDispatch {
            payload: InternalDispatchJobPayload {
                prompt: RESEND_PROMPT.to_owned(),
                reason: Some(reason),
                originating_run_id: Some(plan.run_id.clone()),
                scheduled_at: now,
                delay_seconds_requested: delay_seconds,
            },
        },
        label: Some(format!("quota auto-resend ({})", plan.thread_id)),
        schedule: CronSchedule::Once {
            at: fire_at.to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(plan.thread_id.clone()),
        delete_after_run: true,
        enabled: true,
        system: true,
    };

    match cron.upsert(cfg).await {
        Ok(_) => info!(
            thread_id = %plan.thread_id,
            run_id = %plan.run_id,
            provider = %plan.provider,
            window = ?plan.window,
            fire_at = %fire_at.to_rfc3339(),
            "scheduled quota auto-resend"
        ),
        Err(error) => warn!(
            thread_id = %plan.thread_id,
            error = %error,
            "failed to schedule quota auto-resend"
        ),
    }
}

/// Deterministic, filesystem-safe, per-thread job id so a newer rate-limit
/// replaces any prior pending resend for the same thread.
fn resend_job_id(thread_id: &str) -> String {
    let sanitized: String = thread_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("quota-resend:{sanitized}")
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn committed_run_complete(rate_limit: Value) -> String {
        json!({
            "type": "committed_message",
            "thread_id": "thread::abc",
            "run_id": "run::xyz",
            "seq": 7,
            "message": {
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": {
                    "kind": "run_complete",
                    "thread_id": "thread::abc",
                    "run_id": "run::xyz",
                    "status": "rate_limited",
                    "rate_limit": rate_limit,
                }
            }
        })
        .to_string()
    }

    #[test]
    fn parses_rate_limited_run_complete_into_plan() {
        let raw = committed_run_complete(json!({
            "provider": "codex_app_server",
            "window": "primary",
            "reset_at": "2030-01-01T06:00:00+00:00",
            "will_auto_resend": true
        }));
        let plan = parse_resend_plan(&raw).expect("plan parsed");
        assert_eq!(plan.thread_id, "thread::abc");
        assert_eq!(plan.run_id, "run::xyz");
        assert_eq!(plan.provider, "codex_app_server");
        assert_eq!(plan.window.as_deref(), Some("primary"));
        assert_eq!(plan.reset_at.to_rfc3339(), "2030-01-01T06:00:00+00:00");
    }

    #[test]
    fn ignores_when_auto_resend_disabled_or_no_reset() {
        let no_resend = committed_run_complete(json!({
            "provider": "codex",
            "reset_at": "2030-01-01T06:00:00+00:00",
            "will_auto_resend": false
        }));
        assert!(parse_resend_plan(&no_resend).is_none());

        let no_reset = committed_run_complete(json!({
            "provider": "codex",
            "will_auto_resend": true
        }));
        assert!(parse_resend_plan(&no_reset).is_none());
    }

    #[test]
    fn ignores_non_rate_limited_and_non_committed_events() {
        let normal = json!({
            "type": "committed_message",
            "thread_id": "thread::abc",
            "message": { "control": { "kind": "run_complete", "status": "completed" } }
        })
        .to_string();
        assert!(parse_resend_plan(&normal).is_none());
        assert!(parse_resend_plan("not json").is_none());
    }

    #[test]
    fn resend_job_id_is_filesystem_safe_and_thread_scoped() {
        assert_eq!(
            resend_job_id("thread::abc/def"),
            "quota-resend:thread__abc_def"
        );
    }

    /// The scheduled followup dispatches the literal `continue` prompt — the
    /// same prompt the rate-limit banner's manual Continue button sends — and
    /// never a copy of the thread's prior user message.
    #[tokio::test]
    async fn schedules_literal_continue_followup() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cron = Arc::new(crate::automation::CronService::new(
            tmp.path().to_path_buf(),
        ));
        let state = crate::composition::app_bootstrap::AppStateBuilder::new(
            garyx_models::config::GaryxConfig::default(),
        )
        .with_cron_service(cron.clone())
        .build();

        schedule_resend(
            &state,
            ResendPlan {
                thread_id: "thread::abc".to_owned(),
                run_id: "run::xyz".to_owned(),
                provider: "claude".to_owned(),
                window: Some("weekly".to_owned()),
                reset_at: Utc::now() + Duration::seconds(300),
            },
        )
        .await;

        let job = cron
            .get("quota-resend:thread__abc")
            .await
            .expect("resend job scheduled");
        let CronJobKind::InternalDispatch { payload } = &job.kind else {
            panic!("expected internal-dispatch job, got {:?}", job.kind);
        };
        assert_eq!(payload.prompt, "continue");
        assert_eq!(payload.originating_run_id.as_deref(), Some("run::xyz"));
        assert!(
            payload
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("claude usage-limit auto-resend"),
            "reason should carry the auto-resend context: {:?}",
            payload.reason
        );
        assert_eq!(job.thread_id.as_deref(), Some("thread::abc"));
        assert!(job.delete_after_run);
    }
}
