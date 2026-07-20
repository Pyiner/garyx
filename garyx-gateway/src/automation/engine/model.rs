//! Persisted job state: `CronJob`, run records, and structural validation.

use chrono::{DateTime, Local, Utc};
use chrono_tz::Tz;
use garyx_models::config::{CronAction, CronJobConfig, CronJobKind, CronSchedule};
use serde::{Deserialize, Serialize};

use garyx_models::thread_logs::is_canonical_thread_id;

use super::schedule::{
    has_non_empty_cron_text, machine_cron_timezone, next_cron_run_in_timezone, parse_cron_schedule,
    parse_once_timestamp,
};

// ---------------------------------------------------------------------------
// Persisted job state
// ---------------------------------------------------------------------------

/// Status of the last run of a cron job.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobRunStatus {
    Success,
    Failed,
    /// Terminal failure where the run was intentionally dropped rather than
    /// retried further: the target thread is gone, or a transient dispatch
    /// failure exhausted its retry budget. Distinct from `Failed` so a dropped
    /// followup is treated as terminal (one-shot jobs are disabled, see
    /// `CronJob::settle_after_run`) and never silently re-fires.
    FailedDropped,
    Running,
    NeverRun,
}

/// Run metadata produced by each cron execution (FR-4).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunRecord {
    pub run_id: String,
    pub job_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub status: JobRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Persisted state for a single cron job.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CronJob {
    pub id: String,
    pub kind: CronJobKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub schedule: CronSchedule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_schedule: Option<garyx_models::config::AutomationScheduleView>,
    pub action: CronAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub delete_after_run: bool,
    pub enabled: bool,
    pub next_run: DateTime<Utc>,
    pub last_status: JobRunStatus,
    #[serde(default)]
    pub run_count: u64,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<DateTime<Utc>>,
    /// System-managed marker. Mirrors `CronJobConfig.system` and is hidden
    /// from the default user-facing list. `#[serde(default)]` keeps old
    /// persisted jobs (written before this field existed) deserializable
    /// as `system = false`.
    #[serde(default)]
    pub system: bool,
    /// Derived structural validation. Never persisted; every load/mutation
    /// and both execution paths recompute it from the current job fields.
    #[serde(skip)]
    pub validation_error: Option<String>,
}

impl CronJob {
    /// Create a new job from config, computing the initial next_run.
    pub fn from_config(cfg: &CronJobConfig) -> Self {
        let now = Utc::now();
        let next_run = Self::compute_next_run(&cfg.schedule, now);
        let mut job = Self {
            id: cfg.id.clone(),
            kind: cfg.kind.clone(),
            label: cfg.label.clone(),
            schedule: cfg.schedule.clone(),
            ui_schedule: cfg.ui_schedule.clone(),
            action: cfg.action.clone(),
            target: cfg.target.clone(),
            message: cfg.message.clone(),
            workspace_dir: cfg.workspace_dir.clone(),
            agent_id: cfg.agent_id.clone(),
            thread_id: cfg.thread_id.clone(),
            delete_after_run: cfg.delete_after_run,
            enabled: cfg.enabled,
            next_run,
            last_status: JobRunStatus::NeverRun,
            run_count: 0,
            created_at: now,
            last_run_at: None,
            system: cfg.system,
            validation_error: None,
        };
        job.normalize_agent_contract();
        job.revalidate();
        job
    }

    pub(super) fn normalize_agent_contract(&mut self) {
        if is_automation_prompt_job(self)
            && has_non_empty_cron_text(self.workspace_dir.as_deref())
            && !has_non_empty_cron_text(self.thread_id.as_deref())
        {
            self.agent_id = Some(
                self.agent_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("claude")
                    .to_owned(),
            );
        }
    }

    pub(super) fn revalidate(&mut self) {
        self.validation_error = validate_cron_job(self);
    }

    /// Compute the next run time from a schedule relative to `after`.
    pub(super) fn compute_next_run(schedule: &CronSchedule, after: DateTime<Utc>) -> DateTime<Utc> {
        match schedule {
            CronSchedule::Interval { interval_secs } => i64::try_from(*interval_secs)
                .ok()
                .and_then(chrono::Duration::try_seconds)
                .and_then(|delta| after.checked_add_signed(delta))
                .unwrap_or_else(|| {
                    // The interval is so large that representing `after + interval`
                    // would overflow chrono's timeline (or the Duration itself).
                    // Park the run far in the future rather than panicking -- a
                    // panic here would crash the create request and, via
                    // `advance`, the whole scheduler task. Legitimate intervals
                    // are bounded well below this by `MAX_INTERVAL_SECS`.
                    tracing::warn!(target: "garyx_gateway::cron", 
                        interval_secs = *interval_secs,
                        "interval schedule overflows the representable timeline; parking next_run far in the future"
                    );
                    after
                        .checked_add_signed(chrono::Duration::days(365 * 100))
                        .unwrap_or(after)
                }),
            CronSchedule::Once { at } => parse_once_timestamp(at).unwrap_or(after),
            CronSchedule::Cron { expr, timezone } => {
                if let Some(schedule) = parse_cron_schedule(expr) {
                    let start = after + chrono::Duration::seconds(1);

                    if let Some(tz_name) =
                        timezone.as_deref().map(str::trim).filter(|s| !s.is_empty())
                    {
                        if let Ok(tz) = tz_name.parse::<Tz>() {
                            if let Some(next) = next_cron_run_in_timezone(&schedule, start, &tz) {
                                return next;
                            }
                        } else {
                            tracing::warn!(target: "garyx_gateway::cron", 
                                timezone = tz_name,
                                "invalid cron timezone, using machine local timezone"
                            );
                        }
                    }

                    // No (valid) explicit timezone: interpret the cron
                    // expression in the gateway machine's timezone rather
                    // than UTC, so a bare "0 9 * * *" means 9am local.
                    // Prefer resolving the machine zone to an IANA `Tz`
                    // (TZ env first, then the system setting) so DST
                    // transitions get chrono-tz's well-defined ambiguity
                    // semantics; `chrono::Local`'s platform resolver is a
                    // last-resort fallback because its fall-back handling
                    // is platform-dependent.
                    if let Some(tz) = machine_cron_timezone() {
                        if let Some(next) = next_cron_run_in_timezone(&schedule, start, &tz) {
                            return next;
                        }
                    } else if let Some(next) = next_cron_run_in_timezone(&schedule, start, &Local)
                    {
                        return next;
                    }
                }
                // Fallback: avoid hot-looping invalid cron expressions.
                after + chrono::Duration::hours(1)
            }
        }
    }

    /// Advance next_run after a successful tick.
    pub(super) fn advance(&mut self) {
        let now = Utc::now();
        self.last_run_at = Some(now);
        self.run_count += 1;
        match &self.schedule {
            CronSchedule::Interval { .. } | CronSchedule::Cron { .. } => {
                self.next_run = Self::compute_next_run(&self.schedule, now);
            }
            CronSchedule::Once { .. } => {
                // One-shot jobs disable themselves after firing.
                self.enabled = false;
                // Set next_run far in the future so it won't fire again.
                self.next_run = now + chrono::Duration::days(365 * 100);
            }
        }
    }

    /// Apply post-run bookkeeping after a run produced `status`, returning
    /// whether the job file should be deleted (`delete_after_run` on a terminal
    /// outcome). Single source of truth for both the `run_now` and `tick` paths.
    ///
    /// `Success` advances/disables the schedule as before. `FailedDropped` is a
    /// *terminal* failure: one-shot jobs are disabled so a dropped followup is
    /// not re-claimed every tick (`is_due` only exempts past-at-registration
    /// jobs, so a fired-but-not-advanced `Once` job would otherwise re-fire
    /// indefinitely), and `delete_after_run` is honored just like `Success`.
    /// All other statuses keep the prior behavior (bump counters, leave the
    /// schedule untouched).
    pub(super) fn settle_after_run(
        &mut self,
        status: &JobRunStatus,
        started_at: DateTime<Utc>,
    ) -> bool {
        self.last_status = status.clone();
        match status {
            JobRunStatus::Success => self.advance(),
            JobRunStatus::FailedDropped => {
                self.last_run_at = Some(started_at);
                self.run_count += 1;
                if matches!(self.schedule, CronSchedule::Once { .. }) {
                    self.enabled = false;
                }
            }
            _ => {
                self.last_run_at = Some(started_at);
                self.run_count += 1;
            }
        }
        self.delete_after_run
            && matches!(status, JobRunStatus::Success | JobRunStatus::FailedDropped)
    }

    /// Is this job due to run?
    pub(super) fn is_due(&self) -> bool {
        if !self.enabled {
            return false;
        }

        if matches!(self.schedule, CronSchedule::Once { .. }) && self.created_at > self.next_run {
            // Python parity: one-shot jobs already in the past at registration/startup
            // are considered exhausted and should not auto-fire.
            return false;
        }

        Utc::now() >= self.next_run
    }
}

pub(super) fn is_automation_prompt_job(job: &CronJob) -> bool {
    job.kind == CronJobKind::AutomationPrompt
        && job.action == CronAction::AgentTurn
        && has_non_empty_cron_text(job.message.as_deref())
}

pub(super) fn uses_generated_automation_thread_job(job: &CronJob) -> bool {
    is_automation_prompt_job(job)
        && has_non_empty_cron_text(job.workspace_dir.as_deref())
        && !has_non_empty_cron_text(job.thread_id.as_deref())
}

/// Structural validator shared by list state and dispatch admission. It does
/// not query mutable stores: target existence is rechecked by the execution
/// path, while this closes the historical thread-less pseudo-run bypass.
pub(crate) fn validate_cron_job(job: &CronJob) -> Option<String> {
    if matches!(job.kind, CronJobKind::InternalDispatch { .. }) {
        return None;
    }
    match job.action {
        CronAction::AgentTurn => {
            let thread_id = job
                .thread_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if thread_id.is_some_and(|value| !is_canonical_thread_id(value)) {
                return Some("invalid canonical thread_id for agent turn".to_owned());
            }
            let has_thread = thread_id.is_some();
            let has_target = has_non_empty_cron_text(job.target.as_deref());
            let generated = thread_id.is_none()
                && is_automation_prompt_job(job)
                && has_non_empty_cron_text(job.workspace_dir.as_deref());
            if !has_thread && !has_target && !generated {
                Some("missing canonical target for agent turn".to_owned())
            } else {
                None
            }
        }
        CronAction::SystemEvent => {
            let thread_id = job
                .thread_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if thread_id.is_some_and(|value| !is_canonical_thread_id(value)) {
                return Some("invalid canonical thread_id for system event".to_owned());
            }
            let has_thread = thread_id.is_some();
            let has_target = has_non_empty_cron_text(job.target.as_deref());
            if !has_thread && !has_target {
                Some("missing canonical target for system event".to_owned())
            } else {
                None
            }
        }
        CronAction::Log => None,
    }
}
