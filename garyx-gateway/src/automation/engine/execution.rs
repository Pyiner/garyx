//! Job execution: the scheduler tick, claim/prepare/execute/settle flow,
//! agent-turn dispatch through the injected execution environment, and the
//! `schedule_followup` retry driver.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use garyx_models::config::{CronAction, CronJobKind, InternalDispatchJobPayload};
use garyx_models::thread_logs::ThreadLogEvent;
use garyx_router::ThreadStoreExt;
use garyx_router::{ThreadEnsureOptions, delete_thread_record};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::super::dispatch::{AutomationDispatchError, AutomationExecEnv};
use super::model::{
    CronJob, JobRunStatus, RunRecord, is_automation_prompt_job, uses_generated_automation_thread_job,
};
use super::store::{MAX_RUN_HISTORY, delete_job_file, persist_job, persist_runs};
use super::{CronService, validate_cron_job};
use crate::agent_identity::create_thread_for_agent_reference;
use crate::delivery_target::resolve_delivery_target_with_recovery;
use crate::garyx_db::{AutomationThreadRunDraft, GaryxDbService};
use crate::skills::sync_default_external_user_skills;

#[cfg(test)]
use garyx_channels::ChannelDispatcher;
#[cfg(test)]
use garyx_channels::{OutboundMessage, SendMessageResult};
#[cfg(test)]
use garyx_models::ChannelOutboundContent;
#[cfg(test)]
use garyx_models::provider::{StreamBoundaryKind, StreamEvent};
#[cfg(test)]
use garyx_models::thread_logs::ThreadLogSink;
#[cfg(test)]
use garyx_router::MessageRouter;

/// Maximum number of *retries* (i.e. attempts after the first) for a transient
/// internal-dispatch failure before the followup is dropped. Total
/// attempts are `FOLLOWUP_MAX_RETRIES + 1`.
pub(super) const FOLLOWUP_MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff between internal-dispatch retries. The
/// nth retry waits `FOLLOWUP_RETRY_BASE_BACKOFF * 2^n` (≈200ms, 400ms, 800ms).
pub(super) const FOLLOWUP_RETRY_BASE_BACKOFF: Duration = Duration::from_millis(200);

/// Classification of a single internal-dispatch attempt outcome.
///
/// `Dropped` is non-retryable — the target thread is gone (deleted) or the job
/// is structurally unable to dispatch, so retrying cannot help. `Transient` is
/// a network/internal failure worth retrying with backoff.
#[derive(Debug)]
pub(super) enum FollowupAttemptError {
    /// Non-retryable: drop the followup immediately with this reason.
    Dropped(String),
    /// Retryable transient failure carrying the underlying error text.
    Transient(String),
}

/// Render the synthetic user-turn body for a fired `schedule_followup` job.
///
/// The body has two sections: a `<garyx_followup_metadata>` block so the
/// resumed agent (and telemetry) can identify the turn as a followup, and
/// then the verbatim prompt the caller passed to `schedule_followup`.
///
/// `scheduled_for` is the wall-clock time the cron tick actually fired at —
/// equal to `payload.scheduled_at + payload.delay_seconds_requested` unless a
/// later `schedule_followup` call replaced the job. The metadata exposes
/// both so the resumed agent can reason about the actual delay it
/// experienced.
pub(crate) fn build_followup_body(
    schedule_id: &str,
    payload: &InternalDispatchJobPayload,
    scheduled_for: DateTime<Utc>,
) -> String {
    let mut lines = Vec::with_capacity(8);
    lines.push("<garyx_followup_metadata>".to_owned());
    lines.push(format!("schedule_id: {schedule_id}"));
    // Agent-facing timestamps: gateway-machine local wall-clock time
    // (`YYYY-MM-DD HH:MM:SS`, timezone implicit) so the resumed agent
    // reasons about the delay in the user's wall-clock time.
    lines.push(format!(
        "scheduled_at: {}",
        payload
            .scheduled_at
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    ));
    lines.push(format!(
        "scheduled_for: {}",
        scheduled_for
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
    ));
    lines.push(format!(
        "delay_seconds_requested: {}",
        payload.delay_seconds_requested
    ));
    if let Some(reason) = payload.reason.as_deref() {
        lines.push(format!("reason: {reason}"));
    }
    if let Some(originating) = payload.originating_run_id.as_deref() {
        lines.push(format!("originating_run_id: {originating}"));
    }
    lines.push("</garyx_followup_metadata>".to_owned());
    lines.push(String::new());
    lines.push(payload.prompt.clone());
    lines.join("\n")
}

impl CronService {
    /// Execute a specific job immediately.
    pub(crate) async fn run_now(&self, id: &str, env: &AutomationExecEnv) -> Option<RunRecord> {
        let invalid = {
            let mut jobs = self.jobs.write().await;
            let job = jobs.get_mut(id)?;
            job.normalize_agent_contract();
            job.revalidate();
            job.validation_error
                .clone()
                .map(|error| (job.clone(), error))
        };
        if let Some((job, error)) = invalid {
            let run_id = Uuid::new_v4().to_string();
            let record = Self::failed_run_record(&job, &run_id, error);
            if let Some(stored) = self.jobs.write().await.get_mut(id) {
                stored.settle_after_run(&record.status, record.started_at);
                let _ = persist_job(&self.data_dir, stored).await;
            }
            let _ = Self::append_run_record(&self.data_dir, &self.runs, record.clone()).await;
            return Some(record);
        }
        if !Self::provider_runtime_ready_for_job(&self.jobs, env, id).await {
            tracing::info!(
                job_id = %id,
                "cron run_now skipped: provider runtime is still starting"
            );
            return None;
        }

        let job = match Self::claim_job_for_execution(
            &self.data_dir,
            &self.jobs,
            &self.active_agent_runs,
            env,
            id,
        )
        .await
        {
            Some(job) => job,
            None => {
                tracing::info!(job_id = %id, "cron run_now skipped: job missing, disabled, or already running");
                return None;
            }
        };
        if !job.enabled {
            return None;
        }
        let run_id = Uuid::new_v4().to_string();
        let should_cleanup_prepared_thread = uses_generated_automation_thread_job(&job);
        let (record, prepared_thread_id) =
            match Self::prepare_job_for_execution(&self.jobs, id, &run_id, env).await {
                Ok(prepared_job) => {
                    let prepared_thread_id = if should_cleanup_prepared_thread {
                        prepared_job.thread_id.clone()
                    } else {
                        None
                    };
                    (
                        Self::execute_job(&prepared_job, &self.active_agent_runs, env, &run_id)
                            .await,
                        prepared_thread_id,
                    )
                }
                Err(error) => {
                    tracing::warn!(job_id = %id, error = %error, "cron job preparation failed");
                    (Self::failed_run_record(&job, &run_id, error), None)
                }
            };

        // Update runtime job state.
        let mut should_delete = false;
        {
            let mut jobs = self.jobs.write().await;
            if let Some(j) = jobs.get_mut(id) {
                should_delete = j.settle_after_run(&record.status, record.started_at);
                if !should_delete {
                    let _ = persist_job(&self.data_dir, j).await;
                }
            }
            if should_delete {
                jobs.remove(id);
            }
        }
        if record.status != JobRunStatus::Success {
            Self::cleanup_rejected_automation_thread(env, prepared_thread_id.as_deref()).await;
        }
        Self::finish_recorded_automation_thread_run(env.garyx_db.as_ref(), &record).await;
        if should_delete {
            let _ = delete_job_file(&self.data_dir, id).await;
        }

        let _ = Self::append_run_record(&self.data_dir, &self.runs, record.clone()).await;
        Some(record)
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    pub(super) async fn append_run_record(
        data_dir: &Path,
        runs: &Arc<RwLock<VecDeque<RunRecord>>>,
        record: RunRecord,
    ) -> std::io::Result<()> {
        let mut guard = runs.write().await;
        guard.push_back(record);
        while guard.len() > MAX_RUN_HISTORY {
            guard.pop_front();
        }
        persist_runs(data_dir, &guard).await
    }

    pub(super) async fn provider_runtime_ready_for_job(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        env: &AutomationExecEnv,
        id: &str,
    ) -> bool {
        let requires_provider_runtime = {
            let map = jobs.read().await;
            let Some(job) = map.get(id) else {
                return true;
            };
            match &job.kind {
                CronJobKind::InternalDispatch { .. } => true,
                CronJobKind::AutomationPrompt => {
                    matches!(job.action, CronAction::SystemEvent | CronAction::AgentTurn)
                }
            }
        };
        if !requires_provider_runtime {
            return true;
        }
        env.port.provider_runtime_ready()
    }

    pub(super) async fn claim_job_for_execution(
        data_dir: &Path,
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        env: &AutomationExecEnv,
        id: &str,
    ) -> Option<CronJob> {
        Self::clear_inactive_agent_run(active_agent_runs, env, id).await;
        let has_active_agent_run = active_agent_runs.read().await.contains_key(id);
        let claimed = {
            let mut map = jobs.write().await;
            let job = map.get_mut(id)?;
            job.normalize_agent_contract();
            job.revalidate();
            if job.validation_error.is_some() {
                return None;
            }
            if !job.enabled || job.last_status == JobRunStatus::Running || has_active_agent_run {
                return None;
            }
            job.last_status = JobRunStatus::Running;
            job.clone()
        };
        let _ = persist_job(data_dir, &claimed).await;
        Some(claimed)
    }

    pub(super) async fn clear_inactive_agent_run(
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        env: &AutomationExecEnv,
        job_id: &str,
    ) {
        let run_id = active_agent_runs.read().await.get(job_id).cloned();
        let Some(run_id) = run_id else {
            return;
        };

        if !env.bridge.is_run_active(&run_id).await {
            active_agent_runs.write().await.remove(job_id);
        }
    }

    pub(super) fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    pub(super) fn automation_label(job: &CronJob) -> String {
        Self::trimmed_non_empty(job.label.as_deref()).unwrap_or_else(|| job.id.clone())
    }

    pub(super) fn automation_thread_run_status(status: &JobRunStatus) -> &'static str {
        match status {
            JobRunStatus::Success => "success",
            JobRunStatus::Failed => "failed",
            JobRunStatus::FailedDropped => "dropped",
            JobRunStatus::Running => "running",
            JobRunStatus::NeverRun => "unknown",
        }
    }

    pub(super) fn automation_thread_options(
        automation_id: &str,
        label: &str,
        workspace_dir: &str,
        agent_id: Option<&str>,
    ) -> ThreadEnsureOptions {
        let mut metadata = HashMap::new();
        metadata.insert(
            "source".to_owned(),
            serde_json::Value::String("automation".to_owned()),
        );
        metadata.insert(
            "automation_id".to_owned(),
            serde_json::Value::String(automation_id.to_owned()),
        );
        metadata.insert(
            "automation_thread_mode".to_owned(),
            serde_json::Value::String("generated_thread".to_owned()),
        );
        ThreadEnsureOptions {
            label: Some(label.to_owned()),
            workspace_dir: Some(workspace_dir.to_owned()),
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: Some(
                Self::trimmed_non_empty(agent_id).unwrap_or_else(|| "claude".to_owned()),
            ),
            metadata,
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        }
    }

    pub(super) async fn prepare_job_for_execution(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        id: &str,
        run_id: &str,
        env: &AutomationExecEnv,
    ) -> Result<CronJob, String> {
        let current = {
            let map = jobs.read().await;
            map.get(id)
                .cloned()
                .ok_or_else(|| format!("cron job not found: {id}"))?
        };

        if !uses_generated_automation_thread_job(&current) {
            return Ok(current);
        }

        let workspace_dir = Self::trimmed_non_empty(current.workspace_dir.as_deref())
            .ok_or_else(|| format!("automation {} is missing workspace_dir", current.id))?;
        let label = Self::automation_label(&current);
        let agent_id = Self::trimmed_non_empty(current.agent_id.as_deref());
        let (thread_id, _, _) = create_thread_for_agent_reference(
            env.thread_store.clone(),
            env.bridge.clone(),
            env.custom_agents.clone(),
            Self::automation_thread_options(
                &current.id,
                &label,
                &workspace_dir,
                agent_id.as_deref(),
            ),
            crate::agent_identity::AgentBindingIntent::Fresh,
        )
        .await
        .map_err(|error| format!("failed to create automation thread: {error}"))?;

        if let Some(garyx_db) = env.garyx_db.as_ref() {
            let draft = AutomationThreadRunDraft {
                automation_id: current.id.clone(),
                run_id: run_id.to_owned(),
                thread_id: thread_id.clone(),
                workspace_dir: Some(workspace_dir.clone()),
                agent_id: agent_id.clone(),
                automation_label_snapshot: Some(label.clone()),
                mode: "generated_thread".to_owned(),
                status: "running".to_owned(),
                started_at: Utc::now().to_rfc3339(),
                finished_at: None,
            };
            if let Err(error) = garyx_db
                .run_blocking(move |db| db.upsert_automation_thread_run(draft))
                .await
            {
                let _ = delete_thread_record(&env.thread_store, &thread_id).await;
                return Err(format!(
                    "failed to record automation thread association: {error}"
                ));
            }
        }

        let mut updated = current;
        updated.thread_id = Some(thread_id.clone());
        env.port.invalidate_gateway_sync_caches().await;

        Ok(updated)
    }

    pub(super) fn failed_run_record(job: &CronJob, run_id: &str, error: String) -> RunRecord {
        let started_at = Utc::now();
        RunRecord {
            run_id: run_id.to_owned(),
            job_id: job.id.clone(),
            started_at,
            finished_at: Some(started_at),
            duration_ms: Some(0),
            status: JobRunStatus::Failed,
            thread_id: Self::trimmed_non_empty(job.thread_id.as_deref()),
            error: Some(error),
        }
    }

    pub(super) async fn finish_recorded_automation_thread_run(
        garyx_db: Option<&Arc<GaryxDbService>>,
        record: &RunRecord,
    ) {
        let Some(garyx_db) = garyx_db else {
            return;
        };
        let Some(finished_at) = record.finished_at else {
            return;
        };
        let status = Self::automation_thread_run_status(&record.status);
        let job_id = record.job_id.clone();
        let run_id = record.run_id.clone();
        let finished_at = finished_at.to_rfc3339();
        if let Err(error) = garyx_db
            .run_blocking(move |db| {
                db.finish_automation_thread_run(&job_id, &run_id, status, &finished_at)
            })
            .await
        {
            tracing::warn!(
                job_id = %record.job_id,
                run_id = %record.run_id,
                error = %error,
                "failed to finish recorded automation thread association"
            );
        }
    }

    pub(super) async fn cleanup_rejected_automation_thread(
        env: &AutomationExecEnv,
        prepared_thread_id: Option<&str>,
    ) {
        let Some(prepared_thread_id) = prepared_thread_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            return;
        };

        if let Err(error) = delete_thread_record(&env.thread_store, prepared_thread_id).await {
            tracing::warn!(
                thread_id = prepared_thread_id,
                error = %error,
                "failed to delete rejected cron automation thread"
            );
        }
    }

    /// Called every tick to find and execute due jobs.
    pub(super) async fn tick(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        runs: &Arc<RwLock<VecDeque<RunRecord>>>,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        data_dir: &Path,
        env: &AutomationExecEnv,
    ) {
        // Collect due job IDs under a read lock.
        let due_ids: Vec<String> = {
            let map = jobs.read().await;
            map.values()
                .filter(|j| j.is_due() && validate_cron_job(j).is_none())
                .map(|j| j.id.clone())
                .collect()
        };

        for id in due_ids {
            if !Self::provider_runtime_ready_for_job(jobs, env, &id).await {
                tracing::debug!(
                    job_id = %id,
                    "cron tick skipped due job while provider runtime is starting"
                );
                continue;
            }

            let Some(job) =
                Self::claim_job_for_execution(data_dir, jobs, active_agent_runs, env, &id).await
            else {
                continue;
            };
            let run_id = Uuid::new_v4().to_string();
            let should_cleanup_prepared_thread = uses_generated_automation_thread_job(&job);
            let (record, prepared_thread_id) =
                match Self::prepare_job_for_execution(jobs, &id, &run_id, env).await {
                    Ok(prepared_job) => {
                        let prepared_thread_id = if should_cleanup_prepared_thread {
                            prepared_job.thread_id.clone()
                        } else {
                            None
                        };
                        (
                            Self::execute_job(&prepared_job, active_agent_runs, env, &run_id)
                                .await,
                            prepared_thread_id,
                        )
                    }
                    Err(error) => {
                        tracing::warn!(job_id = %id, error = %error, "cron job preparation failed");
                        (Self::failed_run_record(&job, &run_id, error), None)
                    }
                };

            // Update state under write lock.
            let mut should_delete = false;
            {
                let mut map = jobs.write().await;
                if let Some(j) = map.get_mut(&id) {
                    should_delete = j.settle_after_run(&record.status, record.started_at);
                    if !should_delete {
                        let _ = persist_job(data_dir, j).await;
                    }
                }
                if should_delete {
                    map.remove(&id);
                }
            }
            if record.status != JobRunStatus::Success {
                Self::cleanup_rejected_automation_thread(env, prepared_thread_id.as_deref()).await;
            }
            Self::finish_recorded_automation_thread_run(env.garyx_db.as_ref(), &record).await;
            if should_delete {
                let _ = delete_job_file(data_dir, &id).await;
            }

            let _ = Self::append_run_record(data_dir, runs, record).await;
        }
    }

    /// Execute a single job's action. Returns a `RunRecord`.
    pub(super) async fn execute_job(
        job: &CronJob,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        env: &AutomationExecEnv,
        run_id: &str,
    ) -> RunRecord {
        let run_id = run_id.to_owned();
        let started_at = Utc::now();

        tracing::info!(job_id = %job.id, run_id = %run_id, action = ?job.action, "cron job executing");

        // The run id recorded on the RunRecord. A scheduled turn queued into
        // a thread's already-active run is owned by that run — automation
        // activity resolves the transcript through this id, so it must be the
        // effective one; the requested id stays in logs as the dispatch
        // correlation id.
        let mut record_run_id = run_id.clone();
        let (status, error) = match &job.kind {
            CronJobKind::InternalDispatch { payload } => {
                // Boundary fallback: classify drop-vs-transient and
                // retry transient dispatch failures with exponential backoff.
                // Any terminal failure (thread gone, or retry budget exhausted)
                // becomes `FailedDropped` with the reason recorded in the run
                // record — never a silent drop.
                match Self::dispatch_internal_followup_with_retry(
                    job,
                    &run_id,
                    payload,
                    env,
                    FOLLOWUP_RETRY_BASE_BACKOFF,
                )
                .await
                {
                    Ok(()) => (JobRunStatus::Success, None),
                    Err(reason) => (JobRunStatus::FailedDropped, Some(reason)),
                }
            }
            CronJobKind::AutomationPrompt => match &job.action {
                CronAction::Log => {
                    tracing::info!(job_id = %job.id, "cron log action fired");
                    (JobRunStatus::Success, None)
                }
                CronAction::SystemEvent | CronAction::AgentTurn => {
                    let message = job
                        .message
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or_default()
                        .to_owned();
                    if message.is_empty() {
                        (
                            JobRunStatus::Failed,
                            Some("cron message payload is empty".to_owned()),
                        )
                    } else {
                        match Self::dispatch_agent_turn(
                            job,
                            &run_id,
                            &message,
                            active_agent_runs,
                            env,
                        )
                        .await
                        {
                            Ok(effective_run_id) => {
                                record_run_id = effective_run_id;
                                (JobRunStatus::Success, None)
                            }
                            Err(e) => (JobRunStatus::Failed, Some(e)),
                        }
                    }
                }
            },
        };

        let finished_at = Utc::now();
        let duration_ms = (finished_at - started_at).num_milliseconds().max(0) as u64;

        tracing::info!(
            job_id = %job.id,
            run_id = %run_id,
            status = ?status,
            duration_ms,
            "cron job completed"
        );

        RunRecord {
            run_id: record_run_id,
            job_id: job.id.clone(),
            started_at,
            finished_at: Some(finished_at),
            duration_ms: Some(duration_ms),
            status,
            thread_id: Self::trimmed_non_empty(job.thread_id.as_deref()),
            error,
        }
    }

    /// Build a synthetic user-turn body from a `schedule_followup` payload and
    /// inject it into the originating thread via
    /// [`dispatch_internal_message_to_thread`].
    ///
    /// The synthetic body is prefixed with a `<garyx_followup_metadata>` block
    /// so the resumed agent can correlate the followup with its own earlier
    /// `schedule_followup` call (and so telemetry can distinguish followups
    /// from organic user input).
    /// Drive [`Self::dispatch_internal_followup_once`] with bounded
    /// exponential-backoff retry.
    ///
    /// Returns `Ok(())` on success (possibly after retries) or `Err(reason)`
    /// when the followup is dropped — either non-retryably (thread gone) or
    /// because the retry budget was exhausted. The reason string is recorded in
    /// the run record so a drop is never silent.
    pub(super) async fn dispatch_internal_followup_with_retry(
        job: &CronJob,
        run_id: &str,
        payload: &InternalDispatchJobPayload,
        env: &AutomationExecEnv,
        base_backoff: Duration,
    ) -> Result<(), String> {
        Self::run_followup_with_retry(
            FOLLOWUP_MAX_RETRIES,
            base_backoff,
            &job.id,
            run_id,
            |_attempt| Self::dispatch_internal_followup_once(job, run_id, payload, env),
        )
        .await
    }

    /// Generic retry driver shared by production and tests.
    ///
    /// Calls `attempt` (receiving the zero-based attempt index) until it
    /// succeeds, hits a non-retryable `Dropped` outcome, or exhausts
    /// `max_retries` transient failures. Every drop path emits a `tracing::warn`
    /// so drops are observable; the nth retry sleeps `base_backoff * 2^n`.
    pub(super) async fn run_followup_with_retry<F, Fut>(
        max_retries: u32,
        base_backoff: Duration,
        job_id: &str,
        run_id: &str,
        mut attempt: F,
    ) -> Result<(), String>
    where
        F: FnMut(u32) -> Fut,
        Fut: std::future::Future<Output = Result<(), FollowupAttemptError>>,
    {
        let mut last_error = String::new();
        for n in 0..=max_retries {
            match attempt(n).await {
                Ok(()) => return Ok(()),
                Err(FollowupAttemptError::Dropped(reason)) => {
                    tracing::warn!(
                        job_id = %job_id,
                        run_id = %run_id,
                        reason = %reason,
                        "schedule_followup dropped (non-retryable)"
                    );
                    return Err(reason);
                }
                Err(FollowupAttemptError::Transient(error)) => {
                    last_error = error;
                    if n < max_retries {
                        let backoff = base_backoff * 2u32.pow(n);
                        tracing::warn!(
                            job_id = %job_id,
                            run_id = %run_id,
                            attempt = n + 1,
                            max_attempts = max_retries + 1,
                            backoff_ms = backoff.as_millis() as u64,
                            error = %last_error,
                            "schedule_followup dispatch failed; retrying after backoff"
                        );
                        if !backoff.is_zero() {
                            tokio::time::sleep(backoff).await;
                        }
                    }
                }
            }
        }

        let reason = format!(
            "dispatch failed after {} retries: {}",
            max_retries, last_error
        );
        tracing::warn!(
            job_id = %job_id,
            run_id = %run_id,
            reason = %reason,
            "schedule_followup dropped (retry budget exhausted)"
        );
        Err(reason)
    }

    /// Perform a single internal-dispatch attempt, classifying the outcome into
    /// retryable vs non-retryable.
    ///
    /// Builds a synthetic user-turn body from a `schedule_followup` payload and
    /// injects it into the originating thread via
    /// [`dispatch_internal_message_to_thread`]. The body is prefixed with a
    /// `<garyx_followup_metadata>` block so the resumed agent can correlate the
    /// followup with its own earlier `schedule_followup` call (and so telemetry
    /// can distinguish followups from organic user input).
    ///
    /// A missing thread_id / unavailable gateway state, or a thread that is no
    /// longer present in the thread store, yields `Dropped` (retrying cannot
    /// help). Any other dispatch error yields `Transient`.
    pub(super) async fn dispatch_internal_followup_once(
        job: &CronJob,
        run_id: &str,
        payload: &InternalDispatchJobPayload,
        env: &AutomationExecEnv,
    ) -> Result<(), FollowupAttemptError> {
        let thread_id = Self::trimmed_non_empty(job.thread_id.as_deref()).ok_or_else(|| {
            FollowupAttemptError::Dropped(format!(
                "cron internal-dispatch job {} is missing thread_id",
                job.id
            ))
        })?;

        // Explicit pre-check: if the originating thread was deleted before the
        // followup fired, drop it now rather than relying on string-matching the
        // dispatch error.
        if env.thread_store.get_logged(&thread_id).await.is_none() {
            return Err(FollowupAttemptError::Dropped(format!(
                "thread not found: {thread_id}"
            )));
        }

        let scheduled_for = job.next_run;
        let body = build_followup_body(&job.id, payload, scheduled_for);

        let mut extra_metadata = HashMap::new();
        extra_metadata.insert(
            "schedule_followup".to_owned(),
            serde_json::Value::Bool(true),
        );
        extra_metadata.insert(
            "schedule_followup_job_id".to_owned(),
            serde_json::Value::String(job.id.clone()),
        );
        extra_metadata.insert(
            "schedule_followup_scheduled_at".to_owned(),
            serde_json::Value::String(payload.scheduled_at.to_rfc3339()),
        );
        extra_metadata.insert(
            "schedule_followup_scheduled_for".to_owned(),
            serde_json::Value::String(scheduled_for.to_rfc3339()),
        );
        if let Some(reason) = payload.reason.as_deref() {
            extra_metadata.insert(
                "schedule_followup_reason".to_owned(),
                serde_json::Value::String(reason.to_owned()),
            );
        }
        if let Some(originating) = payload.originating_run_id.as_deref() {
            extra_metadata.insert(
                "schedule_followup_originating_run_id".to_owned(),
                serde_json::Value::String(originating.to_owned()),
            );
        }

        env.port
            .dispatch_internal_message(&thread_id, run_id, &body, extra_metadata)
            .await
            .map(|_outcome| ())
            .map_err(|error| match error {
                AutomationDispatchError::StateUnavailable => FollowupAttemptError::Dropped(
                    "gateway app state is unavailable".to_owned(),
                ),
                // A thread deleted between the pre-check and dispatch surfaces
                // here as the dispatch sentinel — still a non-retryable drop.
                AutomationDispatchError::Dispatch(error) => {
                    if error.starts_with("thread not found") {
                        FollowupAttemptError::Dropped(error)
                    } else {
                        FollowupAttemptError::Transient(error)
                    }
                }
            })
    }

    /// Dispatch a scheduled prompt into an existing thread through the
    /// internal-inbound front door: the message is injected exactly like a
    /// user message (router inbound semantics, transcript user turn, busy
    /// queueing, channel echo), sharing the pipeline with `schedule_followup`
    /// and the quota auto-resend instead of starting a bridge run directly.
    ///
    /// Returns the run id that owns the reply — the requested one for a fresh
    /// run, or the already-active run's id when the prompt was queued into it
    /// — so run records and automation activity resolve the real transcript.
    pub(super) async fn dispatch_agent_turn_via_thread(
        job: &CronJob,
        run_id: &str,
        message: &str,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        env: &AutomationExecEnv,
        thread_key: &str,
    ) -> Result<String, String> {
        let automation_job = is_automation_prompt_job(job);
        let source = if automation_job { "automation" } else { "cron" };

        env.thread_logs
            .record_event(
                ThreadLogEvent::info(thread_key, "automation", "scheduled dispatch started")
                    .with_run_id(run_id.to_owned())
                    .with_field("job_id", serde_json::json!(job.id))
                    .with_field("job_kind", serde_json::json!(format!("{:?}", job.kind)))
                    .with_field("source", serde_json::json!(source))
                    .with_field("dispatch", serde_json::json!("internal_inbound"))
                    .with_field("thread_id", serde_json::json!(thread_key)),
            )
            .await;

        if let Err(error) = sync_default_external_user_skills() {
            tracing::warn!(
                error = %error,
                thread_id = %thread_key,
                "failed to sync external user skills before scheduled dispatch"
            );
        }

        let mut extra_metadata = HashMap::new();
        extra_metadata.insert("source".to_owned(), serde_json::json!(source));
        if automation_job {
            extra_metadata.insert("automation_id".to_owned(), serde_json::json!(job.id));
        } else {
            extra_metadata.insert("cron_job_id".to_owned(), serde_json::json!(job.id));
        }
        extra_metadata.insert(
            "cron_action".to_owned(),
            serde_json::json!(format!("{:?}", job.action)),
        );

        let result = env
            .port
            .dispatch_internal_message(thread_key, run_id, message, extra_metadata)
            .await
            .map_err(|error| match error {
                AutomationDispatchError::StateUnavailable => {
                    "gateway app state is unavailable".to_owned()
                }
                AutomationDispatchError::Dispatch(error) => error,
            });

        match result {
            Ok(outcome) => {
                // When the thread was busy the prompt was queued into the
                // already-active run — every downstream consumer (run
                // bookkeeping, automation activity) must attribute the reply
                // to that run, not the requested one.
                let effective_run_id = outcome.effective_run_id().unwrap_or(run_id).to_owned();
                active_agent_runs
                    .write()
                    .await
                    .insert(job.id.clone(), effective_run_id.clone());
                env.thread_logs
                    .record_event(
                        ThreadLogEvent::info(
                            thread_key,
                            "automation",
                            "scheduled dispatch accepted",
                        )
                        .with_run_id(run_id.to_owned())
                        .with_field("job_id", serde_json::json!(job.id))
                        .with_field("effective_run_id", serde_json::json!(effective_run_id))
                        .with_field(
                            "queued_into_active_run",
                            serde_json::json!(outcome.effective_run_id().is_some()),
                        )
                        .with_field("thread_id", serde_json::json!(thread_key)),
                    )
                    .await;
                Ok(effective_run_id)
            }
            Err(error) => {
                env.thread_logs
                    .record_event(
                        ThreadLogEvent::error(
                            thread_key,
                            "automation",
                            "scheduled dispatch failed",
                        )
                        .with_run_id(run_id.to_owned())
                        .with_field("job_id", serde_json::json!(job.id))
                        .with_field("error", serde_json::json!(error)),
                    )
                    .await;
                Err(format!("cron dispatch failed: {error}"))
            }
        }
    }

    pub(super) async fn dispatch_agent_turn(
        job: &CronJob,
        run_id: &str,
        message: &str,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        env: &AutomationExecEnv,
    ) -> Result<String, String> {
        let configured_target = job
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let (thread_key, thread_record) = if let Some(thread_id) = job
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let thread_record = env
                .thread_store
                .get(thread_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("cron target thread not found: {thread_id}"))?;
            (thread_id.to_owned(), Some(thread_record))
        } else if let Some(target) = configured_target {
            // An explicit target must resolve to an existing thread record;
            // silently starting a bare run against a missing thread would
            // both bypass the front door and fake a Success.
            if target.starts_with("thread:") || target.contains("::") {
                let key = if target.starts_with("thread::") {
                    target.to_owned()
                } else {
                    target.strip_prefix("thread:").unwrap_or(target).to_owned()
                };
                let thread_record = env
                    .thread_store
                    .get(&key)
                    .await
                    .map_err(|error| format!("cron target thread read failed: {error}"))?
                    .ok_or_else(|| format!("cron target thread not found: {key}"))?;
                (key, Some(thread_record))
            } else {
                let resolved = resolve_delivery_target_with_recovery(&env.router, target)
                    .await
                    .ok_or_else(|| format!("unable to resolve cron delivery target: {target}"))?;
                let thread_record = env
                    .thread_store
                    .get(&resolved.0)
                    .await
                    .map_err(|error| format!("cron delivery target read failed: {error}"))?
                    .ok_or_else(|| {
                        format!(
                            "cron delivery target {target} resolved to missing thread {}",
                            resolved.0
                        )
                    })?;
                (resolved.0, Some(thread_record))
            }
        } else {
            (format!("cron::{}", job.id), None)
        };

        // Front door: any scheduled turn that resolved to a real, existing
        // thread dispatches through the same internal-inbound pipeline as
        // `schedule_followup` and the quota auto-resend — the prompt behaves
        // exactly like a user message (router inbound semantics, transcript
        // user turn, busy queueing, channel echo). Thread-less pseudo-targets
        // are invalid and never reach the bridge.
        if thread_record.is_some() {
            return Self::dispatch_agent_turn_via_thread(
                job,
                run_id,
                message,
                active_agent_runs,
                env,
                &thread_key,
            )
            .await;
        }

        Err(format!(
            "cron job {} is missing a canonical thread target",
            job.id
        ))
    }
}

#[cfg(test)]
pub(super) fn format_scheduled_message(text: &str, thread_id: &str) -> String {
    if text.is_empty() || !MessageRouter::is_scheduled_thread(thread_id) {
        return text.to_owned();
    }

    let header = format!("#{thread_id}");
    if text.trim_start().starts_with(&header) {
        return text.to_owned();
    }

    format!("{header}\n{text}")
}

#[cfg(test)]
pub(super) struct ScheduledResponseContext {
    pub(super) thread_id: String,
    pub(super) channel: String,
    pub(super) account_id: String,
    pub(super) chat_id: String,
    pub(super) delivery_target_type: String,
    pub(super) delivery_target_id: String,
    pub(super) delivery_thread_id: Option<String>,
    pub(super) thread_log_id: Option<String>,
}

#[cfg(test)]
pub(super) fn build_scheduled_response_callback(
    dispatcher: Arc<dyn ChannelDispatcher>,
    thread_logs: Arc<dyn ThreadLogSink>,
    context: ScheduledResponseContext,
) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let pending = Arc::new(std::sync::Mutex::new(ScheduledStreamState::default()));
    let ScheduledResponseContext {
        thread_id,
        channel,
        account_id,
        chat_id,
        delivery_target_type,
        delivery_target_id,
        delivery_thread_id,
        thread_log_id,
    } = context;

    Arc::new(move |event: StreamEvent| {
        let maybe_message = {
            let mut buf = match pending.lock() {
                Ok(buf) => buf,
                Err(_) => {
                    tracing::warn!("scheduled response callback buffer lock poisoned");
                    return;
                }
            };
            match event {
                StreamEvent::Delta { text } => {
                    if !buf.closed_after_user_ack && !text.is_empty() {
                        buf.text.push_str(&text);
                    }
                    None
                }
                StreamEvent::Boundary {
                    kind: StreamBoundaryKind::AssistantSegment,
                    ..
                } => {
                    if !buf.closed_after_user_ack {
                        append_inline_assistant_separator(&mut buf.text);
                    }
                    None
                }
                StreamEvent::Boundary {
                    kind: StreamBoundaryKind::UserAck,
                    ..
                } => {
                    buf.closed_after_user_ack = true;
                    None
                }
                StreamEvent::Done => {
                    let merged = std::mem::take(&mut buf.text);
                    if merged.trim().is_empty() {
                        None
                    } else {
                        Some(merged)
                    }
                }
                StreamEvent::SessionBound { .. }
                | StreamEvent::ToolUse { .. }
                | StreamEvent::ToolResult { .. }
                | StreamEvent::ThreadTitleUpdated { .. } => None,
            }
        };

        let Some(merged) = maybe_message else {
            return;
        };

        let outbound_text = format_scheduled_message(&merged, &thread_id);
        let dispatcher = dispatcher.clone();
        let thread_logs = thread_logs.clone();
        let request = OutboundMessage {
            channel: channel.clone(),
            account_id: account_id.clone(),
            chat_id: chat_id.clone(),
            delivery_target_type: delivery_target_type.clone(),
            delivery_target_id: delivery_target_id.clone(),
            content: ChannelOutboundContent::text(outbound_text),
            reply_to: None,
            thread_id: delivery_thread_id.clone(),
        };
        let channel_name = channel.clone();
        let account_name = account_id.clone();
        let chat_id_value = chat_id.clone();
        let thread_log_id_value = thread_log_id.clone();

        tokio::spawn(async move {
            match dispatcher.send_message(request).await {
                Ok(SendMessageResult { message_ids }) => {
                    if message_ids.is_empty() {
                        return;
                    }
                    if let Some(thread_log_id) = thread_log_id_value.as_deref() {
                        for message_id in message_ids {
                            thread_logs
                                .record_event(
                                    ThreadLogEvent::info(
                                        thread_log_id,
                                        "delivery",
                                        "outbound message delivered",
                                    )
                                    .with_field("channel", serde_json::json!(channel_name))
                                    .with_field("account_id", serde_json::json!(account_name))
                                    .with_field("chat_id", serde_json::json!(chat_id_value))
                                    .with_field("message_id", serde_json::json!(message_id))
                                    .with_field("thread_id", serde_json::json!(thread_log_id)),
                                )
                                .await;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to send scheduled cron response");
                }
            }
        });
    })
}

#[cfg(test)]
pub(super) fn append_inline_assistant_separator(buffer: &mut String) {
    if buffer.trim().is_empty() || buffer.ends_with("\n\n") {
        return;
    }
    if buffer.ends_with('\n') {
        buffer.push('\n');
    } else {
        buffer.push_str("\n\n");
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct ScheduledStreamState {
    text: String,
    closed_after_user_ack: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
