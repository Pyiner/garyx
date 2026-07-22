//! Durable provider-quota recovery.
//!
//! A committed `run_complete(status = rate_limited)` is historical transcript
//! truth. This module projects that terminal generation into SQLite and drives
//! one synthetic `continue` through the ordinary durable chat-admission path.
//! Account switch, manual retry, and the quota deadline only make the same SQL
//! row due; an atomic claim and deterministic dispatch intent keep those
//! triggers from producing duplicate turns.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    Json,
    extract::{Path as AxumPath, State},
    response::IntoResponse,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use futures_util::stream::{self, StreamExt};
use garyx_models::config::CronJobKind;
use garyx_router::ThreadTranscriptRecord;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tracing::{info, warn};
use uuid::Uuid;

use crate::application::chat::contracts::{ChatRequest, IdempotencyScope};
use crate::garyx_db::{NewQuotaRecoveryJob, QuotaRecoveryClaimWitness, QuotaRecoveryJob};
use crate::server::AppState;

const RESEND_BUFFER_SECS: i64 = 60;
const CLAIM_LEASE_SECS: i64 = 120;
const MAX_RETRY_BACKOFF_SECS: i64 = 300;
const RESEND_PROMPT: &str = "continue";
const EVENT_HISTORY_REPLAY: usize = 256;
const QUOTA_ADMISSION_SCOPE: &str = "__quota_recovery__";
const QUOTA_ADMISSION_EPOCH: i64 = 1;
const LEGACY_JOB_PREFIX: &str = "quota-resend:";
const NO_AUTOMATIC_DUE_AT: &str = "9999-12-31T23:59:59.999Z";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecoveryPlan {
    thread_id: String,
    run_id: String,
    blocked_seq: u64,
    provider: String,
    window: Option<String>,
    /// `None` means the provider supplied no trustworthy reset deadline. The
    /// generation remains parked for account-switch or manual recovery, but
    /// must never wake from the timer by itself.
    reset_at: Option<DateTime<Utc>>,
}

/// Start the event projection and SQL recovery worker. Both are process-local
/// drivers over durable state; spawning them more than once for one AppState is
/// unsupported, matching the rest of gateway lifecycle startup.
pub(crate) fn spawn_reactor(state: Arc<AppState>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    let event_state = state.clone();
    handle.spawn(async move {
        run_event_projection(event_state).await;
    });
    handle.spawn(async move {
        run_recovery_worker(state).await;
    });
}

async fn run_event_projection(state: Arc<AppState>) {
    let mut rx = state.ops.events.subscribe();
    loop {
        match rx.recv().await {
            Ok(raw_event) => {
                if let Some(plan) = parse_recovery_plan(&raw_event) {
                    register_plan(&state, plan).await;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                for raw_event in state
                    .ops
                    .events
                    .history_snapshot(EVENT_HISTORY_REPLAY)
                    .await
                {
                    if let Some(plan) = parse_recovery_plan(&raw_event) {
                        register_plan(&state, plan).await;
                    }
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

fn parse_recovery_plan(raw_event: &str) -> Option<RecoveryPlan> {
    if !raw_event.contains("rate_limited") {
        return None;
    }
    let event: Value = serde_json::from_str(raw_event).ok()?;
    if event.get("type").and_then(Value::as_str) != Some("committed_message") {
        return None;
    }
    let thread_id = non_empty(event.get("thread_id").and_then(Value::as_str))?;
    let blocked_seq = event.get("seq").and_then(Value::as_u64)?;
    let control = event
        .get("message")
        .and_then(|message| message.get("control"))?;
    recovery_plan_from_control(
        thread_id,
        blocked_seq,
        event.get("run_id").and_then(Value::as_str),
        control,
    )
}

fn recovery_plan_from_control(
    thread_id: String,
    blocked_seq: u64,
    record_run_id: Option<&str>,
    control: &Value,
) -> Option<RecoveryPlan> {
    if control.get("kind").and_then(Value::as_str) != Some("run_complete")
        || control.get("status").and_then(Value::as_str) != Some("rate_limited")
    {
        return None;
    }
    let rate_limit = control.get("rate_limit")?;
    let will_auto_resend = rate_limit
        .get("will_auto_resend")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let parsed_reset_at = match rate_limit.get("reset_at").and_then(Value::as_str) {
        Some(value) => Some(
            DateTime::parse_from_rfc3339(value)
                .ok()?
                .with_timezone(&Utc),
        ),
        None => None,
    };
    if will_auto_resend && parsed_reset_at.is_none() {
        return None;
    }
    let reset_at = if will_auto_resend {
        parsed_reset_at
    } else {
        None
    };
    let run_id = non_empty(
        control
            .get("run_id")
            .and_then(Value::as_str)
            .or(record_run_id),
    )?;
    let provider = canonical_quota_provider(
        rate_limit
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("codex"),
    );
    let window = rate_limit
        .get("window")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Some(RecoveryPlan {
        thread_id,
        run_id,
        blocked_seq,
        provider,
        window,
        reset_at,
    })
}

pub(crate) fn canonical_quota_provider(provider: &str) -> String {
    let normalized = provider.trim().to_ascii_lowercase().replace('-', "_");
    if normalized.contains("claude") {
        "claude_code".to_owned()
    } else if normalized.contains("codex") || normalized == "openai" {
        "codex_app_server".to_owned()
    } else if normalized.contains("antigravity") || normalized.contains("gemini") {
        "antigravity".to_owned()
    } else {
        normalized
    }
}

fn due_at_for_reset(reset_at: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    (reset_at + Duration::seconds(RESEND_BUFFER_SECS))
        .max(now + Duration::seconds(RESEND_BUFFER_SECS))
}

async fn register_plan(state: &Arc<AppState>, plan: RecoveryPlan) {
    let due_at = plan
        .reset_at
        .map(|reset_at| {
            due_at_for_reset(reset_at, Utc::now()).to_rfc3339_opts(SecondsFormat::Millis, true)
        })
        .unwrap_or_else(|| NO_AUTOMATIC_DUE_AT.to_owned());
    let db = state.ops.garyx_db.clone();
    let log_thread_id = plan.thread_id.clone();
    let log_run_id = plan.run_id.clone();
    let log_provider = plan.provider.clone();
    let input_thread_id = plan.thread_id;
    let input_provider = plan.provider;
    let input_run_id = plan.run_id;
    let input_blocked_seq = plan.blocked_seq;
    let input_window = plan.window;
    let input_reset_at = plan
        .reset_at
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true));
    let result = db
        .run_blocking(move |db| {
            db.register_quota_recovery_job(NewQuotaRecoveryJob {
                thread_id: &input_thread_id,
                provider: &input_provider,
                blocked_run_id: &input_run_id,
                blocked_seq: input_blocked_seq,
                quota_window: input_window.as_deref(),
                reset_at: input_reset_at.as_deref(),
                due_at: &due_at,
            })
        })
        .await;
    match result {
        Ok(job) => {
            state.ops.quota_recovery_notify.notify_one();
            info!(
                thread_id = %log_thread_id,
                run_id = %log_run_id,
                provider = %log_provider,
                due_at = %job.due_at,
                "registered quota recovery"
            );
        }
        Err(error) => warn!(
            thread_id = %log_thread_id,
            run_id = %log_run_id,
            error = %error,
            "failed to register quota recovery"
        ),
    }
}

async fn run_recovery_worker(state: Arc<AppState>) {
    loop {
        let mut dispatched_any = false;
        loop {
            let now = Utc::now();
            let now_string = now.to_rfc3339_opts(SecondsFormat::Millis, true);
            let claim_token = Uuid::new_v4().to_string();
            let claim_expires_at = (now + Duration::seconds(CLAIM_LEASE_SECS))
                .to_rfc3339_opts(SecondsFormat::Millis, true);
            let db = state.ops.garyx_db.clone();
            let claim_token_for_db = claim_token.clone();
            let job = match db
                .run_blocking(move |db| {
                    db.claim_next_due_quota_recovery(
                        &now_string,
                        &claim_token_for_db,
                        &claim_expires_at,
                    )
                })
                .await
            {
                Ok(job) => job,
                Err(error) => {
                    warn!(error = %error, "failed to claim quota recovery");
                    break;
                }
            };
            let Some(job) = job else {
                break;
            };
            dispatched_any = true;
            process_claimed_job(&state, job, claim_token).await;
        }

        if dispatched_any {
            continue;
        }

        let db = state.ops.garyx_db.clone();
        let next_due = match db.run_blocking(|db| db.next_quota_recovery_due_at()).await {
            Ok(value) => value,
            Err(error) => {
                warn!(error = %error, "failed to read next quota recovery deadline");
                None
            }
        };
        let notified = state.ops.quota_recovery_notify.notified();
        match next_due.and_then(|value| DateTime::parse_from_rfc3339(&value).ok()) {
            Some(next_due) => {
                let delay = (next_due.with_timezone(&Utc) - Utc::now())
                    .to_std()
                    .unwrap_or_default();
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = notified => {}
                }
            }
            None => notified.await,
        }
    }
}

async fn process_claimed_job(state: &Arc<AppState>, job: QuotaRecoveryJob, claim_token: String) {
    match latest_rate_limited_generation(state, &job.thread_id).await {
        Ok(Some(run_id)) if run_id == job.blocked_run_id => {}
        Ok(_) => {
            settle_claim_as_superseded(state, &job, &claim_token).await;
            return;
        }
        Err(error) if error.starts_with("thread not found") => {
            settle_claim_as_cancelled(state, &job, &claim_token).await;
            return;
        }
        Err(error) => {
            retry_claim(state, &job, &claim_token, error).await;
            return;
        }
    }

    let mut metadata = HashMap::new();
    metadata.insert("internal_dispatch".to_owned(), Value::Bool(true));
    metadata.insert("quota_recovery".to_owned(), Value::Bool(true));
    let request = ChatRequest {
        message: RESEND_PROMPT.to_owned(),
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        thread_id: Some(job.thread_id.clone()),
        client_intent_id: Some(job.dispatch_intent_id.clone()),
        idempotency_scope: Some(IdempotencyScope {
            identity: QUOTA_ADMISSION_SCOPE.to_owned(),
            epoch: QUOTA_ADMISSION_EPOCH,
        }),
        bot: None,
        from_id: "garyx-quota-recovery".to_owned(),
        account_id: "main".to_owned(),
        wait_for_response: false,
        workspace_path: None,
        provider_type: None,
        metadata,
    };

    let claim = QuotaRecoveryClaimWitness {
        job_id: job.job_id.clone(),
        claim_token: claim_token.clone(),
    };
    match crate::chat::start_chat_run_with_quota_recovery(state, request, claim).await {
        Ok(_) => {
            let db = state.ops.garyx_db.clone();
            let job_id = job.job_id.clone();
            let token = claim_token.clone();
            match db
                .run_blocking(move |db| db.deliver_claimed_quota_recovery(&job_id, &token))
                .await
            {
                Ok(true) => info!(
                    thread_id = %job.thread_id,
                    blocked_run_id = %job.blocked_run_id,
                    wake_reason = %job.wake_reason.as_str(),
                    "quota recovery dispatched"
                ),
                Ok(false) => info!(
                    thread_id = %job.thread_id,
                    blocked_run_id = %job.blocked_run_id,
                    "quota recovery was already settled while dispatching"
                ),
                Err(error) => warn!(
                    thread_id = %job.thread_id,
                    error = %error,
                    "quota recovery dispatch succeeded but settlement failed"
                ),
            }
        }
        Err((status, payload)) => {
            let body = payload.0;
            let code = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("quota_recovery_dispatch_failed");
            let message = body
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(code)
                .to_owned();
            if status == StatusCode::NOT_FOUND {
                settle_claim_as_cancelled(state, &job, &claim_token).await;
            } else if status == StatusCode::CONFLICT
                && matches!(code, "dispatch_ambiguous" | "idempotency_conflict")
            {
                // Reissuing an ambiguous durable admission could duplicate a
                // user turn. Settle the recovery generation; the durable
                // admission remains the diagnostic truth for that ambiguity.
                settle_claim_as_superseded(state, &job, &claim_token).await;
            } else {
                retry_claim(state, &job, &claim_token, message).await;
            }
        }
    }
}

async fn latest_rate_limited_generation(
    state: &Arc<AppState>,
    thread_id: &str,
) -> Result<Option<String>, String> {
    if state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err(format!("thread not found: {thread_id}"));
    }
    let records = state
        .threads
        .history
        .transcript_store()
        .records(thread_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(latest_rate_limited_plan(&records).map(|plan| plan.run_id))
}

fn latest_rate_limited_plan(records: &[ThreadTranscriptRecord]) -> Option<RecoveryPlan> {
    for record in records.iter().rev() {
        let Some(control) = record.message.get("control") else {
            continue;
        };
        match control.get("kind").and_then(Value::as_str) {
            Some("run_complete") => {
                return recovery_plan_from_control(
                    record.thread_id.clone(),
                    record.seq,
                    record.run_id.as_deref(),
                    control,
                );
            }
            Some("run_start" | "run_error") => return None,
            _ => {}
        }
    }
    None
}

async fn retry_claim(
    state: &Arc<AppState>,
    job: &QuotaRecoveryJob,
    claim_token: &str,
    error: String,
) {
    let exponent = u32::try_from(job.attempt_count.saturating_sub(1).min(6)).unwrap_or(6);
    let backoff_secs =
        (5_i64.saturating_mul(2_i64.saturating_pow(exponent))).min(MAX_RETRY_BACKOFF_SECS);
    let due_at =
        (Utc::now() + Duration::seconds(backoff_secs)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let db = state.ops.garyx_db.clone();
    let job_id = job.job_id.clone();
    let claim_token = claim_token.to_owned();
    let error_for_db = error.clone();
    match db
        .run_blocking(move |db| {
            db.retry_claimed_quota_recovery(&job_id, &claim_token, &due_at, &error_for_db)
        })
        .await
    {
        Ok(true) => {
            state.ops.quota_recovery_notify.notify_one();
            warn!(
                thread_id = %job.thread_id,
                blocked_run_id = %job.blocked_run_id,
                backoff_secs,
                error = %error,
                "quota recovery dispatch failed; retry scheduled"
            );
        }
        Ok(false) => {}
        Err(settle_error) => warn!(
            thread_id = %job.thread_id,
            error = %settle_error,
            "failed to release quota recovery claim"
        ),
    }
}

async fn settle_claim_as_superseded(
    state: &Arc<AppState>,
    job: &QuotaRecoveryJob,
    claim_token: &str,
) {
    let db = state.ops.garyx_db.clone();
    let job_id = job.job_id.clone();
    let claim_token = claim_token.to_owned();
    if let Err(error) = db
        .run_blocking(move |db| db.supersede_claimed_quota_recovery(&job_id, &claim_token))
        .await
    {
        warn!(thread_id = %job.thread_id, error = %error, "failed to supersede stale quota recovery");
    }
}

async fn settle_claim_as_cancelled(
    state: &Arc<AppState>,
    job: &QuotaRecoveryJob,
    claim_token: &str,
) {
    let db = state.ops.garyx_db.clone();
    let job_id = job.job_id.clone();
    let claim_token = claim_token.to_owned();
    if let Err(error) = db
        .run_blocking(move |db| db.cancel_claimed_quota_recovery(&job_id, &claim_token))
        .await
    {
        warn!(thread_id = %job.thread_id, error = %error, "failed to cancel quota recovery");
    }
}

/// Import quota jobs created by the pre-SQL implementation. This runs after
/// cron files are loaded and before the cron scheduler starts, so a legacy job
/// cannot fire while it is being fenced by its SQL generation.
pub async fn migrate_legacy_cron_jobs(state: &Arc<AppState>) {
    let Some(cron) = state.ops.cron_service.as_ref() else {
        return;
    };
    let jobs = cron.list_all().await;
    for job in jobs {
        if !job.system || !job.id.starts_with(LEGACY_JOB_PREFIX) {
            continue;
        }
        let Some(thread_id) = job.thread_id.as_deref() else {
            continue;
        };
        let CronJobKind::InternalDispatch { payload } = &job.kind else {
            continue;
        };
        let Some(originating_run_id) = payload.originating_run_id.as_deref() else {
            continue;
        };
        let records = match state
            .threads
            .history
            .transcript_store()
            .records(thread_id)
            .await
        {
            Ok(records) => records,
            Err(error) => {
                warn!(job_id = %job.id, error = %error, "failed to inspect legacy quota job");
                continue;
            }
        };
        let plan = latest_rate_limited_plan(&records);
        let stale = plan
            .as_ref()
            .is_none_or(|plan| plan.run_id != originating_run_id);
        if stale {
            if let Err(error) = cron.delete(&job.id).await {
                warn!(job_id = %job.id, error = %error, "failed to delete stale legacy quota job");
            }
            continue;
        }
        let plan = plan.expect("non-stale legacy quota job has a plan");
        let db = state.ops.garyx_db.clone();
        let due_at = job.next_run.to_rfc3339_opts(SecondsFormat::Millis, true);
        let reset_at = plan
            .reset_at
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Millis, true));
        let result = db
            .run_blocking(move |db| {
                db.register_quota_recovery_job(NewQuotaRecoveryJob {
                    thread_id: &plan.thread_id,
                    provider: &plan.provider,
                    blocked_run_id: &plan.run_id,
                    blocked_seq: plan.blocked_seq,
                    quota_window: plan.window.as_deref(),
                    reset_at: reset_at.as_deref(),
                    due_at: &due_at,
                })
            })
            .await;
        match result {
            Ok(_) => {
                if let Err(error) = cron.delete(&job.id).await {
                    warn!(job_id = %job.id, error = %error, "legacy quota job imported but file deletion failed");
                }
            }
            Err(error) => {
                warn!(job_id = %job.id, error = %error, "failed to import legacy quota job")
            }
        }
    }
}

/// Repair the narrow crash window between a committed terminal transcript
/// record and its broadcast projection reaching SQLite. Settled generations
/// replay idempotently, so this scan only creates work when the SQL row was
/// genuinely missing.
pub async fn reconcile_transcript_recovery_jobs(state: &Arc<AppState>) {
    let db = state.ops.garyx_db.clone();
    let thread_ids = match db
        .run_blocking(|db| db.list_thread_record_keys(Some("thread::")))
        .await
    {
        Ok(thread_ids) => thread_ids,
        Err(error) => {
            warn!(error = %error, "failed to enumerate threads for quota recovery repair");
            return;
        }
    };
    stream::iter(thread_ids)
        .for_each_concurrent(8, |thread_id| {
            let state = Arc::clone(state);
            async move {
                let records = match state
                    .threads
                    .history
                    .transcript_store()
                    .records(&thread_id)
                    .await
                {
                    Ok(records) => records,
                    Err(error) => {
                        warn!(thread_id, error = %error, "failed to inspect transcript for quota recovery repair");
                        return;
                    }
                };
                if let Some(plan) = latest_rate_limited_plan(&records) {
                    register_plan(&state, plan).await;
                }
            }
        })
        .await;
}

pub(crate) async fn expedite_provider_after_account_switch(
    state: &Arc<AppState>,
    provider: &str,
) -> Result<crate::garyx_db::QuotaRecoveryExpediteSummary, String> {
    // The transcript terminal is committed before its SQL projection is
    // broadcast. Close that narrow race so an account switch made from the
    // freshly rendered card cannot miss the generation it is meant to wake.
    reconcile_transcript_recovery_jobs(state).await;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let provider = provider.to_owned();
    let db = state.ops.garyx_db.clone();
    let summary = db
        .run_blocking(move |db| db.expedite_quota_recovery_provider(&provider, &now))
        .await
        .map_err(|error| error.to_string())?;
    if summary.expedited_threads > 0 {
        state.ops.quota_recovery_notify.notify_one();
    }
    Ok(summary)
}

pub(crate) async fn expedite_thread_manual(
    state: &Arc<AppState>,
    thread_id: &str,
) -> Result<bool, String> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let thread_id_for_db = thread_id.to_owned();
    let db = state.ops.garyx_db.clone();
    let mut changed = db
        .run_blocking(move |db| db.expedite_quota_recovery_thread(&thread_id_for_db, &now))
        .await
        .map_err(|error| error.to_string())?;
    if !changed {
        let records = state
            .threads
            .history
            .transcript_store()
            .records(thread_id)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(plan) = latest_rate_limited_plan(&records) {
            register_plan(state, plan).await;
            let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let thread_id_for_db = thread_id.to_owned();
            let db = state.ops.garyx_db.clone();
            changed = db
                .run_blocking(move |db| db.expedite_quota_recovery_thread(&thread_id_for_db, &now))
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    if changed {
        state.ops.quota_recovery_notify.notify_one();
    }
    Ok(changed)
}

pub async fn retry_thread_quota_recovery(
    State(state): State<Arc<AppState>>,
    AxumPath(thread_id): AxumPath<String>,
) -> impl IntoResponse {
    if !garyx_models::thread_logs::is_canonical_thread_id(thread_id.trim()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "thread_id must be canonical" })),
        )
            .into_response();
    }
    match expedite_thread_manual(&state, &thread_id).await {
        Ok(true) => (
            StatusCode::ACCEPTED,
            Json(json!({ "status": "accepted", "thread_id": thread_id })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "quota_recovery_not_found",
                "thread_id": thread_id,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "quota_recovery_wake_failed", "message": error })),
        )
            .into_response(),
    }
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

    fn raw_rate_limit_event(run_id: &str, reset_at: &str) -> String {
        json!({
            "type": "committed_message",
            "thread_id": "thread::quota",
            "run_id": run_id,
            "seq": 7,
            "message": {
                "role": "system",
                "control": {
                    "kind": "run_complete",
                    "run_id": run_id,
                    "status": "rate_limited",
                    "rate_limit": {
                        "provider": "claude",
                        "window": "primary",
                        "reset_at": reset_at,
                        "will_auto_resend": true
                    }
                }
            }
        })
        .to_string()
    }

    #[test]
    fn parses_committed_rate_limit_generation() {
        let plan =
            parse_recovery_plan(&raw_rate_limit_event("run::one", "2026-07-23T00:00:00Z")).unwrap();
        assert_eq!(plan.thread_id, "thread::quota");
        assert_eq!(plan.run_id, "run::one");
        assert_eq!(plan.blocked_seq, 7);
        assert_eq!(plan.provider, "claude_code");
        assert_eq!(plan.window.as_deref(), Some("primary"));
    }

    #[test]
    fn ignores_rate_limit_with_an_invalid_reset_time() {
        let raw = raw_rate_limit_event("run::one", "bad-time");
        assert!(parse_recovery_plan(&raw).is_none());
    }

    #[test]
    fn parks_rate_limit_without_an_automatic_reset() {
        let mut raw: Value = serde_json::from_str(&raw_rate_limit_event(
            "run::manual-only",
            "2099-01-01T00:00:00Z",
        ))
        .unwrap();
        let rate_limit = raw["message"]["control"]["rate_limit"]
            .as_object_mut()
            .unwrap();
        rate_limit.remove("reset_at");
        rate_limit.insert("will_auto_resend".to_owned(), Value::Bool(false));

        let plan = parse_recovery_plan(&raw.to_string()).unwrap();
        assert_eq!(plan.run_id, "run::manual-only");
        assert!(plan.reset_at.is_none());
    }

    #[test]
    fn latest_generation_is_invalidated_by_new_run_start() {
        let rate_limit: Value =
            serde_json::from_str(&raw_rate_limit_event("run::one", "2026-07-23T00:00:00Z"))
                .unwrap();
        let records = vec![
            ThreadTranscriptRecord {
                seq: 1,
                thread_id: "thread::quota".to_owned(),
                run_id: Some("run::one".to_owned()),
                timestamp: "2026-07-22T00:00:00Z".to_owned(),
                message: rate_limit["message"].clone(),
            },
            ThreadTranscriptRecord {
                seq: 2,
                thread_id: "thread::quota".to_owned(),
                run_id: Some("run::two".to_owned()),
                timestamp: "2026-07-22T00:01:00Z".to_owned(),
                message: json!({
                    "role": "system",
                    "control": { "kind": "run_start", "run_id": "run::two" }
                }),
            },
        ];
        assert!(latest_rate_limited_plan(&records).is_none());
    }

    #[tokio::test]
    async fn startup_repair_projects_a_committed_terminal_generation() {
        let state = crate::server::AppStateBuilder::new(Default::default()).build();
        state
            .ops
            .garyx_db
            .run_thread_data_startup_migrations()
            .unwrap();
        state
            .ops
            .garyx_db
            .write_thread_record_with_projections("thread::quota", "{}", None, None)
            .unwrap();
        let raw: Value =
            serde_json::from_str(&raw_rate_limit_event("run::repair", "2099-01-01T00:00:00Z"))
                .unwrap();
        state
            .threads
            .history
            .transcript_store()
            .append_committed_messages(
                "thread::quota",
                Some("run::repair"),
                &[raw["message"].clone()],
            )
            .await
            .unwrap();

        reconcile_transcript_recovery_jobs(&state).await;

        let job = state
            .ops
            .garyx_db
            .active_quota_recovery_job("thread::quota")
            .unwrap()
            .unwrap();
        assert_eq!(job.blocked_run_id, "run::repair");
        assert_eq!(job.provider, "claude_code");
    }

    #[tokio::test]
    async fn manual_retry_projects_then_wakes_a_fresh_terminal() {
        let state = crate::server::AppStateBuilder::new(Default::default()).build();
        state
            .ops
            .garyx_db
            .run_thread_data_startup_migrations()
            .unwrap();
        state
            .ops
            .garyx_db
            .write_thread_record_with_projections("thread::quota-manual", "{}", None, None)
            .unwrap();
        let raw: Value =
            serde_json::from_str(&raw_rate_limit_event("run::manual", "2099-01-01T00:00:00Z"))
                .unwrap();
        state
            .threads
            .history
            .transcript_store()
            .append_committed_messages(
                "thread::quota-manual",
                Some("run::manual"),
                &[raw["message"].clone()],
            )
            .await
            .unwrap();
        assert!(
            state
                .ops
                .garyx_db
                .active_quota_recovery_job("thread::quota-manual")
                .unwrap()
                .is_none()
        );

        assert!(
            expedite_thread_manual(&state, "thread::quota-manual")
                .await
                .unwrap()
        );

        let job = state
            .ops
            .garyx_db
            .active_quota_recovery_job("thread::quota-manual")
            .unwrap()
            .unwrap();
        assert_eq!(job.blocked_run_id, "run::manual");
        assert_eq!(
            job.wake_reason,
            crate::garyx_db::QuotaRecoveryWakeReason::Manual
        );
    }

    #[test]
    fn reset_deadline_keeps_one_minute_safety_buffer() {
        let now = DateTime::parse_from_rfc3339("2026-07-22T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            due_at_for_reset(now + Duration::hours(1), now),
            now + Duration::hours(1) + Duration::seconds(60)
        );
        assert_eq!(
            due_at_for_reset(now - Duration::hours(1), now),
            now + Duration::seconds(60)
        );
    }
}
