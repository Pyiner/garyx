use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use chrono::{DateTime, Local, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use garyx_bridge::MultiProviderBridge;
use garyx_channels::{ChannelDispatcher, OutboundMessage, SendMessageResult};
use garyx_models::ChannelOutboundContent;
use garyx_models::config::{
    CronAction, CronConfig, CronJobConfig, CronJobKind, CronSchedule, InternalDispatchJobPayload,
    McpServerConfig,
};
use garyx_models::provider::{AgentRunRequest, StreamBoundaryKind, StreamEvent};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink, is_canonical_thread_id};
use garyx_router::{
    MessageRouter, ThreadEnsureOptions, ThreadStore, delete_thread_record,
    thread_metadata_from_value, workspace_dir_from_value,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent_identity::create_thread_for_agent_reference;
use crate::agent_teams::AgentTeamStore;
use crate::custom_agents::CustomAgentStore;
use crate::delivery_target::resolve_delivery_target_with_recovery;
use crate::garyx_db::{AutomationThreadRunDraft, GaryxDbService};
use crate::internal_inbound::{InternalDispatchOptions, dispatch_internal_message_to_thread};
use crate::managed_mcp_metadata::inject_managed_mcp_servers;
use crate::server::AppState;
use crate::skills::sync_default_external_user_skills;

const MAX_INTERVAL_SECS: u64 = i64::MAX as u64;

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

/// Maximum number of *retries* (i.e. attempts after the first) for a transient
/// internal-dispatch failure before the followup is dropped. Total
/// attempts are `FOLLOWUP_MAX_RETRIES + 1`.
const FOLLOWUP_MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff between internal-dispatch retries. The
/// nth retry waits `FOLLOWUP_RETRY_BASE_BACKOFF * 2^n` (≈200ms, 400ms, 800ms).
const FOLLOWUP_RETRY_BASE_BACKOFF: Duration = Duration::from_millis(200);

/// Classification of a single internal-dispatch attempt outcome.
///
/// `Dropped` is non-retryable — the target thread is gone (deleted) or the job
/// is structurally unable to dispatch, so retrying cannot help. `Transient` is
/// a network/internal failure worth retrying with backoff.
#[derive(Debug)]
enum FollowupAttemptError {
    /// Non-retryable: drop the followup immediately with this reason.
    Dropped(String),
    /// Retryable transient failure carrying the underlying error text.
    Transient(String),
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
}

impl CronJob {
    /// Create a new job from config, computing the initial next_run.
    pub fn from_config(cfg: &CronJobConfig) -> Self {
        let now = Utc::now();
        let next_run = Self::compute_next_run(&cfg.schedule, now);
        Self {
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
        }
    }

    /// Compute the next run time from a schedule relative to `after`.
    fn compute_next_run(schedule: &CronSchedule, after: DateTime<Utc>) -> DateTime<Utc> {
        match schedule {
            CronSchedule::Interval { interval_secs } => match i64::try_from(*interval_secs) {
                Ok(secs) => after + chrono::Duration::seconds(secs),
                Err(_) => {
                    tracing::warn!(
                        interval_secs = *interval_secs,
                        "interval schedule exceeds i64 range, using 1h fallback"
                    );
                    after + chrono::Duration::hours(1)
                }
            },
            CronSchedule::Once { at } => parse_once_timestamp(at).unwrap_or(after),
            CronSchedule::Cron { expr, timezone } => {
                if let Some(schedule) = parse_cron_schedule(expr) {
                    let start = after + chrono::Duration::seconds(1);

                    if let Some(tz_name) =
                        timezone.as_deref().map(str::trim).filter(|s| !s.is_empty())
                    {
                        if let Ok(tz) = tz_name.parse::<Tz>() {
                            let start_local = start.with_timezone(&tz);
                            if let Some(next_local) = schedule.after(&start_local).next() {
                                return next_local.with_timezone(&Utc);
                            }
                        } else {
                            tracing::warn!(timezone = tz_name, "invalid cron timezone, using UTC");
                        }
                    }

                    if let Some(next) = schedule.after(&start).next() {
                        return next;
                    }
                }
                // Fallback: avoid hot-looping invalid cron expressions.
                after + chrono::Duration::hours(1)
            }
        }
    }

    /// Advance next_run after a successful tick.
    fn advance(&mut self) {
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
    fn settle_after_run(&mut self, status: &JobRunStatus, started_at: DateTime<Utc>) -> bool {
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
    fn is_due(&self) -> bool {
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

pub(crate) fn parse_once_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(timestamp) = trimmed.parse::<DateTime<Utc>>() {
        return Some(timestamp);
    }

    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(timestamp.with_timezone(&Utc));
    }

    let naive = trimmed
        .strip_prefix("ONCE:")
        .map(str::trim)
        .and_then(parse_local_once_naive)
        .or_else(|| parse_local_once_naive(trimmed))?;

    match Local.from_local_datetime(&naive) {
        LocalResult::Single(timestamp) => Some(timestamp.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

fn parse_local_once_naive(raw: &str) -> Option<NaiveDateTime> {
    for format in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(timestamp) = NaiveDateTime::parse_from_str(raw, format) {
            return Some(timestamp);
        }
    }

    None
}

fn parse_cron_schedule(expr: &str) -> Option<Schedule> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Primary format in Rust runtime: second-precision cron expression.
    if let Ok(schedule) = Schedule::from_str(trimmed) {
        return Some(schedule);
    }

    // Python parity: accept 5-field crontab expressions used by croniter.
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() == 5 {
        let normalized = format!("0 {trimmed}");
        if let Ok(schedule) = Schedule::from_str(&normalized) {
            return Some(schedule);
        }
    }

    None
}

fn has_non_empty_cron_text(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .is_some_and(|candidate| !candidate.is_empty())
}

fn is_automation_prompt_job(job: &CronJob) -> bool {
    job.kind == CronJobKind::AutomationPrompt
        && job.action == CronAction::AgentTurn
        && has_non_empty_cron_text(job.message.as_deref())
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
    lines.push(format!(
        "scheduled_at: {}",
        payload.scheduled_at.to_rfc3339()
    ));
    lines.push(format!("scheduled_for: {}", scheduled_for.to_rfc3339()));
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

fn uses_generated_automation_thread_job(job: &CronJob) -> bool {
    is_automation_prompt_job(job)
        && has_non_empty_cron_text(job.workspace_dir.as_deref())
        && !has_non_empty_cron_text(job.thread_id.as_deref())
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

/// Directory layout: `<data_dir>/cron/jobs/<id>.json`
fn jobs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cron").join("jobs")
}

fn runs_file(data_dir: &Path) -> PathBuf {
    data_dir.join("cron").join("runs.json")
}

async fn ensure_dirs(data_dir: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(jobs_dir(data_dir)).await
}

async fn persist_job(data_dir: &Path, job: &CronJob) -> std::io::Result<()> {
    let path = jobs_dir(data_dir).join(format!("{}.json", job.id));
    let tmp = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(job).map_err(std::io::Error::other)?;
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

async fn delete_job_file(data_dir: &Path, id: &str) -> std::io::Result<()> {
    let path = jobs_dir(data_dir).join(format!("{id}.json"));
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

async fn load_jobs(data_dir: &Path) -> std::io::Result<Vec<CronJob>> {
    let dir = jobs_dir(data_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut jobs = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice::<CronJob>(&bytes) {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping corrupt cron job file");
                    let _ = tokio::fs::remove_file(&path).await;
                }
            },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to read cron job file");
            }
        }
    }
    Ok(jobs)
}

async fn load_runs(data_dir: &Path) -> std::io::Result<VecDeque<RunRecord>> {
    let path = runs_file(data_dir);
    if !path.exists() {
        return Ok(VecDeque::new());
    }

    let bytes = tokio::fs::read(&path).await?;
    let records: Vec<RunRecord> = match serde_json::from_slice(&bytes) {
        Ok(records) => records,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "skipping corrupt cron runs file"
            );
            let _ = tokio::fs::remove_file(&path).await;
            return Ok(VecDeque::new());
        }
    };

    let mut deque = VecDeque::from(records);
    while deque.len() > MAX_RUN_HISTORY {
        deque.pop_front();
    }
    Ok(deque)
}

async fn persist_runs(data_dir: &Path, runs: &VecDeque<RunRecord>) -> std::io::Result<()> {
    let path = runs_file(data_dir);
    let tmp = path.with_extension("tmp");
    let list: Vec<RunRecord> = runs.iter().cloned().collect();
    let bytes = serde_json::to_vec_pretty(&list).map_err(std::io::Error::other)?;
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CronService
// ---------------------------------------------------------------------------

const MAX_RUN_HISTORY: usize = 200;

#[derive(Clone)]
struct CronDispatchRuntime {
    thread_store: Arc<dyn ThreadStore>,
    router: Arc<tokio::sync::Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    channel_dispatcher: Arc<dyn ChannelDispatcher>,
    thread_logs: Arc<dyn ThreadLogSink>,
    managed_mcp_servers: HashMap<String, McpServerConfig>,
    custom_agents: Arc<CustomAgentStore>,
    agent_teams: Arc<AgentTeamStore>,
}

/// Cron scheduler service.
///
/// Lifecycle: `start()` spawns a background tokio task that ticks every second,
/// checking whether any job is due. `stop()` sends a signal to terminate the loop.
pub struct CronService {
    data_dir: PathBuf,
    jobs: Arc<RwLock<HashMap<String, CronJob>>>,
    runs: Arc<RwLock<VecDeque<RunRecord>>>,
    active_agent_runs: Arc<RwLock<HashMap<String, String>>>,
    /// Send () to stop the scheduler loop.
    stop_tx: Option<mpsc::Sender<()>>,
    scheduler_task: Option<JoinHandle<()>>,
    /// Optional bridge+router runtime for agent-turn/system-event actions.
    dispatch_runtime: Arc<RwLock<Option<CronDispatchRuntime>>>,
    /// Weak handle to the owning [`AppState`]. Populated by
    /// `AppStateBuilder::build` *after* the `Arc<AppState>` exists, so the
    /// scheduler can call back into [`dispatch_internal_message_to_thread`]
    /// without forming an `Arc<AppState>` ↔ `Arc<CronService>` cycle.
    ///
    /// Uses [`OnceLock`] because the weak ref is established exactly once at
    /// startup and never replaced.
    app_state_weak: Arc<OnceLock<Weak<AppState>>>,
    /// Gateway SQLite store used for user-facing automation thread associations.
    garyx_db: Arc<OnceLock<Arc<GaryxDbService>>>,
}

impl CronService {
    /// Create a new CronService.
    ///
    /// Does NOT start the scheduler loop. Call `start()` after creation.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(RwLock::new(VecDeque::new())),
            active_agent_runs: Arc::new(RwLock::new(HashMap::new())),
            stop_tx: None,
            scheduler_task: None,
            dispatch_runtime: Arc::new(RwLock::new(None)),
            app_state_weak: Arc::new(OnceLock::new()),
            garyx_db: Arc::new(OnceLock::new()),
        }
    }

    /// Install the back-reference to the owning [`AppState`].
    ///
    /// Must be called once after `Arc::new(AppState)` so internal-dispatch
    /// cron jobs (e.g. those produced by `mcp__garyx__schedule_followup`)
    /// can synthesize a user turn into the target thread when they fire.
    /// Subsequent calls are no-ops: the `Arc<AppState>` identity is stable
    /// for the gateway's lifetime.
    pub fn set_app_state(&self, weak: Weak<AppState>) {
        let _ = self.app_state_weak.set(weak);
    }

    pub fn set_garyx_db(&self, garyx_db: Arc<GaryxDbService>) {
        let _ = self.garyx_db.set(garyx_db);
    }

    /// Attach bridge+router runtime for agent-turn/system-event dispatch.
    #[allow(clippy::too_many_arguments)]
    pub async fn set_dispatch_runtime(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        router: Arc<tokio::sync::Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        channel_dispatcher: Arc<dyn ChannelDispatcher>,
        thread_logs: Arc<dyn ThreadLogSink>,
        managed_mcp_servers: HashMap<String, McpServerConfig>,
        custom_agents: Arc<CustomAgentStore>,
        agent_teams: Arc<AgentTeamStore>,
    ) {
        *self.dispatch_runtime.write().await = Some(CronDispatchRuntime {
            thread_store,
            router,
            bridge,
            channel_dispatcher,
            thread_logs,
            managed_mcp_servers,
            custom_agents,
            agent_teams,
        });
    }

    /// Load persisted jobs from disk, then merge config-defined jobs
    /// (config jobs take precedence for schedule/action, but persisted
    /// runtime state like run_count is preserved).
    pub async fn load(&self, config: &CronConfig) -> std::io::Result<()> {
        ensure_dirs(&self.data_dir).await?;

        // Load from disk first.
        let disk_jobs = load_jobs(&self.data_dir).await?;
        let mut map = HashMap::new();
        for job in disk_jobs {
            if let Err(error) = validate_cron_schedule(&job.schedule) {
                tracing::warn!(
                    job_id = %job.id,
                    error = %error,
                    "skipping persisted cron job with invalid schedule"
                );
                let _ = delete_job_file(&self.data_dir, &job.id).await;
                continue;
            }
            map.insert(job.id.clone(), job);
        }

        // Merge config-defined jobs.
        for cfg_job in &config.jobs {
            if let Err(error) = validate_cron_schedule(&cfg_job.schedule) {
                tracing::warn!(
                    job_id = %cfg_job.id,
                    error = %error,
                    "skipping config cron job with invalid schedule"
                );
                map.remove(&cfg_job.id);
                continue;
            }
            if let Some(existing) = map.get_mut(&cfg_job.id) {
                // Update schedule/action/enabled from config, keep runtime state.
                let schedule_changed = existing.schedule != cfg_job.schedule;
                existing.kind = cfg_job.kind.clone();
                existing.label = cfg_job.label.clone();
                existing.schedule = cfg_job.schedule.clone();
                existing.ui_schedule = cfg_job.ui_schedule.clone();
                existing.action = cfg_job.action.clone();
                existing.target = cfg_job.target.clone();
                existing.message = cfg_job.message.clone();
                existing.workspace_dir = cfg_job.workspace_dir.clone();
                existing.agent_id = cfg_job.agent_id.clone();
                existing.thread_id = cfg_job.thread_id.clone();
                existing.delete_after_run = cfg_job.delete_after_run;
                existing.enabled = cfg_job.enabled;
                existing.system = cfg_job.system;
                if schedule_changed {
                    existing.next_run = CronJob::compute_next_run(&existing.schedule, Utc::now());
                }
            } else {
                let job = CronJob::from_config(cfg_job);
                map.insert(job.id.clone(), job);
            }
        }

        // Persist merged state.
        for job in map.values() {
            persist_job(&self.data_dir, job).await?;
        }

        *self.jobs.write().await = map;

        let runs = load_runs(&self.data_dir).await?;
        *self.runs.write().await = runs;

        tracing::info!(count = self.jobs.read().await.len(), "cron jobs loaded");
        Ok(())
    }

    /// Start the scheduler loop as a background task.
    pub fn start(&mut self) {
        if self.stop_tx.is_some() {
            tracing::warn!("cron scheduler already running; duplicate start ignored");
            return;
        }

        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        self.stop_tx = Some(stop_tx);

        let jobs = self.jobs.clone();
        let runs = self.runs.clone();
        let active_agent_runs = self.active_agent_runs.clone();
        let data_dir = self.data_dir.clone();
        let dispatch_runtime = self.dispatch_runtime.clone();
        let app_state_weak = self.app_state_weak.clone();
        let garyx_db = self.garyx_db.clone();

        let task = tokio::spawn(async move {
            tracing::info!("cron scheduler started");
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = stop_rx.recv() => {
                        tracing::info!("cron scheduler stopping");
                        break;
                    }
                    _ = interval.tick() => {
                        Self::tick(
                            &jobs,
                            &runs,
                            &active_agent_runs,
                            &data_dir,
                            &dispatch_runtime,
                            &app_state_weak,
                            &garyx_db,
                        ).await;
                    }
                }
            }
            tracing::info!("cron scheduler stopped");
        });
        self.scheduler_task = Some(task);
    }

    /// Stop the scheduler loop.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(task) = self.scheduler_task.take() {
            let _ = task.await;
        }
    }

    /// List jobs visible to user-facing surfaces (default).
    ///
    /// System-managed jobs (e.g. those scheduled by the
    /// `schedule_followup` MCP tool) are filtered out so they don't pollute
    /// the user's automation list. Use [`Self::list_all`] when the caller
    /// genuinely needs every job — including the system-managed ones — such
    /// as the scheduler's own internal accounting or tests.
    pub async fn list(&self) -> Vec<CronJob> {
        self.jobs
            .read()
            .await
            .values()
            .filter(|job| !job.system)
            .cloned()
            .collect()
    }

    /// List every job, including system-managed ones.
    pub async fn list_all(&self) -> Vec<CronJob> {
        self.jobs.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: &str) -> Option<CronJob> {
        self.jobs.read().await.get(id).cloned()
    }

    /// List recent runs in reverse chronological order.
    pub async fn list_runs(&self, limit: usize, offset: usize) -> Vec<RunRecord> {
        let runs = self.runs.read().await;
        runs.iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Total number of persisted run records.
    pub async fn total_runs(&self) -> usize {
        self.runs.read().await.len()
    }

    pub async fn list_runs_for_job(
        &self,
        job_id: &str,
        limit: usize,
        offset: usize,
    ) -> Vec<RunRecord> {
        let runs = self.runs.read().await;
        runs.iter()
            .rev()
            .filter(|record| record.job_id == job_id)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Add a new job dynamically.
    pub async fn add(&self, cfg: CronJobConfig) -> std::io::Result<CronJob> {
        validate_cron_schedule(&cfg.schedule)?;
        ensure_dirs(&self.data_dir).await?;
        let job = CronJob::from_config(&cfg);
        persist_job(&self.data_dir, &job).await?;
        self.jobs.write().await.insert(job.id.clone(), job.clone());
        tracing::info!(job_id = %cfg.id, "cron job added");
        Ok(job)
    }

    /// Insert-or-replace a job by id, atomically capturing any prior job.
    ///
    /// Used by `schedule_followup` to dedupe per `(thread_id, run_id)` —
    /// callers derive a deterministic id and call `upsert`; the returned
    /// `previous` slot tells them whether they replaced an existing schedule
    /// (and, if so, what its terms were).
    pub async fn upsert(&self, cfg: CronJobConfig) -> std::io::Result<(CronJob, Option<CronJob>)> {
        validate_cron_schedule(&cfg.schedule)?;
        ensure_dirs(&self.data_dir).await?;
        let new_job = CronJob::from_config(&cfg);
        let previous = self
            .jobs
            .write()
            .await
            .insert(new_job.id.clone(), new_job.clone());
        persist_job(&self.data_dir, &new_job).await?;
        if previous.is_some() {
            tracing::info!(job_id = %cfg.id, "cron job replaced via upsert");
        } else {
            tracing::info!(job_id = %cfg.id, "cron job added via upsert");
        }
        Ok((new_job, previous))
    }

    /// Update an existing job in-place, preserving runtime counters/state.
    pub async fn update(&self, id: &str, cfg: CronJobConfig) -> std::io::Result<Option<CronJob>> {
        validate_cron_schedule(&cfg.schedule)?;
        ensure_dirs(&self.data_dir).await?;
        let updated = {
            let mut jobs = self.jobs.write().await;
            let Some(job) = jobs.get_mut(id) else {
                return Ok(None);
            };
            job.schedule = cfg.schedule;
            job.kind = cfg.kind;
            job.label = cfg.label;
            job.ui_schedule = cfg.ui_schedule;
            job.action = cfg.action;
            job.target = cfg.target;
            job.message = cfg.message;
            job.workspace_dir = cfg.workspace_dir;
            job.agent_id = cfg.agent_id;
            job.thread_id = cfg.thread_id;
            job.delete_after_run = cfg.delete_after_run;
            job.enabled = cfg.enabled;
            job.system = cfg.system;
            job.next_run = CronJob::compute_next_run(&job.schedule, Utc::now());

            job.clone()
        };

        persist_job(&self.data_dir, &updated).await?;
        tracing::info!(job_id = %id, "cron job updated");
        Ok(Some(updated))
    }

    /// Delete a job by ID.
    pub async fn delete(&self, id: &str) -> std::io::Result<bool> {
        let removed = self.jobs.write().await.remove(id).is_some();
        if removed {
            self.active_agent_runs.write().await.remove(id);
            delete_job_file(&self.data_dir, id).await?;
            tracing::info!(job_id = %id, "cron job deleted");
        }
        Ok(removed)
    }

    /// Execute a specific job immediately.
    pub async fn run_now(&self, id: &str) -> Option<RunRecord> {
        let job = match Self::claim_job_for_execution(
            &self.data_dir,
            &self.jobs,
            &self.active_agent_runs,
            &self.dispatch_runtime,
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
        let garyx_db = self.garyx_db.get().cloned();
        let (record, prepared_thread_id) = match Self::prepare_job_for_execution(
            &self.jobs,
            id,
            &run_id,
            &self.dispatch_runtime,
            &self.app_state_weak,
            garyx_db.clone(),
        )
        .await
        {
            Ok(prepared_job) => {
                let prepared_thread_id = if should_cleanup_prepared_thread {
                    prepared_job.thread_id.clone()
                } else {
                    None
                };
                (
                    Self::execute_job(
                        &prepared_job,
                        &self.active_agent_runs,
                        &self.dispatch_runtime,
                        &self.app_state_weak,
                        &run_id,
                    )
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
            Self::cleanup_rejected_automation_thread(
                &self.dispatch_runtime,
                prepared_thread_id.as_deref(),
            )
            .await;
        }
        Self::finish_recorded_automation_thread_run(garyx_db.as_deref(), &record);
        if should_delete {
            let _ = delete_job_file(&self.data_dir, id).await;
        }

        let _ = Self::append_run_record(&self.data_dir, &self.runs, record.clone()).await;
        Some(record)
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    async fn append_run_record(
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

    async fn claim_job_for_execution(
        data_dir: &Path,
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        dispatch_runtime: &Arc<RwLock<Option<CronDispatchRuntime>>>,
        id: &str,
    ) -> Option<CronJob> {
        Self::clear_inactive_agent_run(active_agent_runs, dispatch_runtime, id).await;
        let has_active_agent_run = active_agent_runs.read().await.contains_key(id);
        let claimed = {
            let mut map = jobs.write().await;
            let job = map.get_mut(id)?;
            if !job.enabled || job.last_status == JobRunStatus::Running || has_active_agent_run {
                return None;
            }
            job.last_status = JobRunStatus::Running;
            job.clone()
        };
        let _ = persist_job(data_dir, &claimed).await;
        Some(claimed)
    }

    async fn clear_inactive_agent_run(
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        dispatch_runtime: &Arc<RwLock<Option<CronDispatchRuntime>>>,
        job_id: &str,
    ) {
        let run_id = active_agent_runs.read().await.get(job_id).cloned();
        let Some(run_id) = run_id else {
            return;
        };

        let is_active = if let Some(runtime) = dispatch_runtime.read().await.clone() {
            runtime.bridge.is_run_active(&run_id).await
        } else {
            false
        };

        if !is_active {
            active_agent_runs.write().await.remove(job_id);
        }
    }

    fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn automation_label(job: &CronJob) -> String {
        Self::trimmed_non_empty(job.label.as_deref()).unwrap_or_else(|| job.id.clone())
    }

    fn automation_thread_run_status(status: &JobRunStatus) -> &'static str {
        match status {
            JobRunStatus::Success => "success",
            JobRunStatus::Failed => "failed",
            JobRunStatus::FailedDropped => "dropped",
            JobRunStatus::Running => "running",
            JobRunStatus::NeverRun => "unknown",
        }
    }

    fn automation_thread_options(
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
        metadata.insert(
            "exclude_from_recent".to_owned(),
            serde_json::Value::Bool(true),
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

    async fn prepare_job_for_execution(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        id: &str,
        run_id: &str,
        dispatch_runtime: &Arc<RwLock<Option<CronDispatchRuntime>>>,
        app_state_weak: &Arc<OnceLock<Weak<AppState>>>,
        garyx_db: Option<Arc<GaryxDbService>>,
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
        let runtime = dispatch_runtime
            .read()
            .await
            .clone()
            .ok_or_else(|| "cron dispatch runtime unavailable".to_owned())?;
        let (thread_id, _, _) = create_thread_for_agent_reference(
            runtime.thread_store.clone(),
            runtime.bridge.clone(),
            runtime.custom_agents.clone(),
            runtime.agent_teams.clone(),
            Self::automation_thread_options(
                &current.id,
                &label,
                &workspace_dir,
                agent_id.as_deref(),
            ),
        )
        .await
        .map_err(|error| format!("failed to create automation thread: {error}"))?;

        if let Some(garyx_db) = garyx_db {
            if let Err(error) = garyx_db.upsert_automation_thread_run(AutomationThreadRunDraft {
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
            }) {
                let _ = delete_thread_record(&runtime.thread_store, &thread_id).await;
                return Err(format!(
                    "failed to record automation thread association: {error}"
                ));
            }
        }

        let mut updated = current;
        updated.thread_id = Some(thread_id.clone());
        if let Some(app_state) = app_state_weak.get().and_then(Weak::upgrade) {
            app_state.invalidate_gateway_sync_caches().await;
        }

        Ok(updated)
    }

    fn failed_run_record(job: &CronJob, run_id: &str, error: String) -> RunRecord {
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

    fn finish_recorded_automation_thread_run(
        garyx_db: Option<&GaryxDbService>,
        record: &RunRecord,
    ) {
        let Some(garyx_db) = garyx_db else {
            return;
        };
        let Some(finished_at) = record.finished_at else {
            return;
        };
        let status = Self::automation_thread_run_status(&record.status);
        if let Err(error) = garyx_db.finish_automation_thread_run(
            &record.job_id,
            &record.run_id,
            status,
            &finished_at.to_rfc3339(),
        ) {
            tracing::warn!(
                job_id = %record.job_id,
                run_id = %record.run_id,
                error = %error,
                "failed to finish recorded automation thread association"
            );
        }
    }

    async fn cleanup_rejected_automation_thread(
        dispatch_runtime: &Arc<RwLock<Option<CronDispatchRuntime>>>,
        prepared_thread_id: Option<&str>,
    ) {
        let Some(prepared_thread_id) = prepared_thread_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            return;
        };

        let Some(runtime) = dispatch_runtime.read().await.clone() else {
            return;
        };

        if let Err(error) = delete_thread_record(&runtime.thread_store, prepared_thread_id).await {
            tracing::warn!(
                thread_id = prepared_thread_id,
                error = %error,
                "failed to delete rejected cron automation thread"
            );
        }
    }

    /// Called every tick to find and execute due jobs.
    #[allow(clippy::too_many_arguments)]
    async fn tick(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        runs: &Arc<RwLock<VecDeque<RunRecord>>>,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        data_dir: &Path,
        dispatch_runtime: &Arc<RwLock<Option<CronDispatchRuntime>>>,
        app_state_weak: &Arc<OnceLock<Weak<AppState>>>,
        garyx_db: &Arc<OnceLock<Arc<GaryxDbService>>>,
    ) {
        // Collect due job IDs under a read lock.
        let due_ids: Vec<String> = {
            let map = jobs.read().await;
            map.values()
                .filter(|j| j.is_due())
                .map(|j| j.id.clone())
                .collect()
        };

        for id in due_ids {
            let Some(job) = Self::claim_job_for_execution(
                data_dir,
                jobs,
                active_agent_runs,
                dispatch_runtime,
                &id,
            )
            .await
            else {
                continue;
            };
            let run_id = Uuid::new_v4().to_string();
            let should_cleanup_prepared_thread = uses_generated_automation_thread_job(&job);
            let garyx_db_handle = garyx_db.get().cloned();
            let (record, prepared_thread_id) = match Self::prepare_job_for_execution(
                jobs,
                &id,
                &run_id,
                dispatch_runtime,
                app_state_weak,
                garyx_db_handle.clone(),
            )
            .await
            {
                Ok(prepared_job) => {
                    let prepared_thread_id = if should_cleanup_prepared_thread {
                        prepared_job.thread_id.clone()
                    } else {
                        None
                    };
                    (
                        Self::execute_job(
                            &prepared_job,
                            active_agent_runs,
                            dispatch_runtime,
                            app_state_weak,
                            &run_id,
                        )
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
                Self::cleanup_rejected_automation_thread(
                    dispatch_runtime,
                    prepared_thread_id.as_deref(),
                )
                .await;
            }
            Self::finish_recorded_automation_thread_run(garyx_db_handle.as_deref(), &record);
            if should_delete {
                let _ = delete_job_file(data_dir, &id).await;
            }

            let _ = Self::append_run_record(data_dir, runs, record).await;
        }
    }

    /// Execute a single job's action. Returns a `RunRecord`.
    async fn execute_job(
        job: &CronJob,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        dispatch_runtime: &Arc<RwLock<Option<CronDispatchRuntime>>>,
        app_state_weak: &Arc<OnceLock<Weak<AppState>>>,
        run_id: &str,
    ) -> RunRecord {
        let run_id = run_id.to_owned();
        let started_at = Utc::now();

        tracing::info!(job_id = %job.id, run_id = %run_id, action = ?job.action, "cron job executing");

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
                    app_state_weak,
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
                            dispatch_runtime,
                        )
                        .await
                        {
                            Ok(()) => (JobRunStatus::Success, None),
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
            run_id,
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
    async fn dispatch_internal_followup_with_retry(
        job: &CronJob,
        run_id: &str,
        payload: &InternalDispatchJobPayload,
        app_state_weak: &Arc<OnceLock<Weak<AppState>>>,
        base_backoff: Duration,
    ) -> Result<(), String> {
        Self::run_followup_with_retry(
            FOLLOWUP_MAX_RETRIES,
            base_backoff,
            &job.id,
            run_id,
            |_attempt| Self::dispatch_internal_followup_once(job, run_id, payload, app_state_weak),
        )
        .await
    }

    /// Generic retry driver shared by production and tests.
    ///
    /// Calls `attempt` (receiving the zero-based attempt index) until it
    /// succeeds, hits a non-retryable `Dropped` outcome, or exhausts
    /// `max_retries` transient failures. Every drop path emits a `tracing::warn`
    /// so drops are observable; the nth retry sleeps `base_backoff * 2^n`.
    async fn run_followup_with_retry<F, Fut>(
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
    /// A missing thread_id / app_state back-reference, or a thread that is no
    /// longer present in the thread store, yields `Dropped` (retrying cannot
    /// help). Any other dispatch error yields `Transient`.
    async fn dispatch_internal_followup_once(
        job: &CronJob,
        run_id: &str,
        payload: &InternalDispatchJobPayload,
        app_state_weak: &Arc<OnceLock<Weak<AppState>>>,
    ) -> Result<(), FollowupAttemptError> {
        let thread_id = Self::trimmed_non_empty(job.thread_id.as_deref()).ok_or_else(|| {
            FollowupAttemptError::Dropped(format!(
                "cron internal-dispatch job {} is missing thread_id",
                job.id
            ))
        })?;

        let app_state = app_state_weak
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| {
                FollowupAttemptError::Dropped(
                    "cron app_state back-reference is not installed".to_owned(),
                )
            })?;

        // Explicit pre-check: if the originating thread was deleted before the
        // followup fired, drop it now rather than relying on string-matching the
        // dispatch error.
        if app_state
            .threads
            .thread_store
            .get(&thread_id)
            .await
            .is_none()
        {
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

        dispatch_internal_message_to_thread(
            &app_state,
            &thread_id,
            run_id,
            &body,
            InternalDispatchOptions {
                extra_metadata,
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            // A thread deleted between the pre-check and dispatch surfaces here
            // as the dispatch sentinel — still a non-retryable drop.
            if error.starts_with("thread not found") {
                FollowupAttemptError::Dropped(error)
            } else {
                FollowupAttemptError::Transient(error)
            }
        })
    }

    async fn dispatch_agent_turn(
        job: &CronJob,
        run_id: &str,
        message: &str,
        active_agent_runs: &Arc<RwLock<HashMap<String, String>>>,
        dispatch_runtime: &Arc<RwLock<Option<CronDispatchRuntime>>>,
    ) -> Result<(), String> {
        let runtime = dispatch_runtime
            .read()
            .await
            .clone()
            .ok_or_else(|| "cron dispatch runtime unavailable".to_owned())?;

        let configured_target = job
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let (thread_key, delivery_ctx, thread_record) = if let Some(thread_id) = job
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let thread_record = runtime
                .thread_store
                .get(thread_id)
                .await
                .ok_or_else(|| format!("cron target thread not found: {thread_id}"))?;
            (thread_id.to_owned(), None, Some(thread_record))
        } else if let Some(target) = configured_target {
            if target.starts_with("thread:") || target.contains("::") {
                let key = if target.starts_with("thread::") {
                    target.to_owned()
                } else {
                    target.strip_prefix("thread:").unwrap_or(target).to_owned()
                };
                let thread_target = format!("thread:{key}");
                let delivery =
                    resolve_delivery_target_with_recovery(&runtime.router, &thread_target)
                        .await
                        .map(|(_, ctx)| ctx);
                let thread_record = runtime.thread_store.get(&key).await;
                (key, delivery, thread_record)
            } else {
                let resolved = resolve_delivery_target_with_recovery(&runtime.router, target)
                    .await
                    .ok_or_else(|| format!("unable to resolve cron delivery target: {target}"))?;
                let thread_record = runtime.thread_store.get(&resolved.0).await;
                (resolved.0, Some(resolved.1), thread_record)
            }
        } else {
            let delivery = resolve_delivery_target_with_recovery(&runtime.router, "last")
                .await
                .map(|(_, ctx)| ctx);
            (format!("cron::{}", job.id), delivery, None)
        };

        let automation_job = is_automation_prompt_job(job);
        let thread_bound_automation =
            automation_job && Self::trimmed_non_empty(job.thread_id.as_deref()).is_some();
        let thread_workspace_dir = thread_record.as_ref().and_then(workspace_dir_from_value);
        let job_workspace_dir = Self::trimmed_non_empty(job.workspace_dir.as_deref());
        let effective_workspace_dir = if thread_bound_automation {
            thread_workspace_dir
                .clone()
                .or_else(|| job_workspace_dir.clone())
        } else {
            job_workspace_dir
                .clone()
                .or_else(|| thread_workspace_dir.clone())
        };
        let mut metadata = HashMap::new();
        if let Some(thread_record) = thread_record.as_ref() {
            for (key, value) in thread_metadata_from_value(thread_record) {
                metadata.entry(key).or_insert(value);
            }
        }
        metadata.insert(
            "source".to_owned(),
            serde_json::json!(if automation_job { "automation" } else { "cron" }),
        );
        if automation_job {
            metadata.insert("automation_id".to_owned(), serde_json::json!(job.id));
        } else {
            metadata.insert("cron_job_id".to_owned(), serde_json::json!(job.id));
        }
        metadata.insert("run_id".to_owned(), serde_json::json!(run_id));
        metadata.insert(
            "cron_action".to_owned(),
            serde_json::json!(format!("{:?}", job.action)),
        );
        metadata.insert(
            "target".to_owned(),
            serde_json::json!(
                configured_target
                    .or_else(|| job.thread_id.as_deref())
                    .unwrap_or("last")
            ),
        );
        metadata.insert(
            "resolved_thread_id".to_owned(),
            serde_json::json!(thread_key.clone()),
        );
        if let Some(workspace_dir) = thread_workspace_dir.as_ref() {
            metadata.insert(
                "workspace_dir".to_owned(),
                serde_json::json!(workspace_dir.clone()),
            );
        }

        let (channel, account_id, chat_id, thread_id, workspace_dir) =
            if let Some(delivery) = &delivery_ctx {
                metadata.insert(
                    "delivery_target".to_owned(),
                    serde_json::json!({
                        "channel": delivery.channel,
                        "chat_id": delivery.chat_id,
                        "account_id": delivery.account_id,
                    }),
                );
                if let Some(thread_id) = &delivery.thread_id {
                    metadata.insert("thread_id".to_owned(), serde_json::json!(thread_id));
                }
                (
                    delivery.channel.clone(),
                    delivery.account_id.clone(),
                    Some(delivery.chat_id.clone()),
                    delivery.thread_id.clone(),
                    effective_workspace_dir.clone(),
                )
            } else {
                let default_channel = if automation_job { "api" } else { "cron" };
                let default_account = if automation_job { "main" } else { "cron" };
                (
                    default_channel.to_owned(),
                    default_account.to_owned(),
                    None,
                    job.thread_id.clone(),
                    effective_workspace_dir,
                )
            };

        let response_callback = if automation_job {
            None
        } else {
            let delivery_target_type = delivery_ctx
                .as_ref()
                .map(|delivery| delivery.delivery_target_type.clone())
                .unwrap_or_else(|| "chat_id".to_owned());
            let delivery_target_id = delivery_ctx
                .as_ref()
                .map(|delivery| delivery.delivery_target_id.clone())
                .or_else(|| chat_id.clone())
                .unwrap_or_else(|| "last".to_owned());
            chat_id.map(|chat_id| {
                let thread_log_id = scheduled_thread_log_id(&thread_key, thread_id.as_deref());
                build_scheduled_response_callback(
                    runtime.channel_dispatcher.clone(),
                    runtime.router.clone(),
                    ScheduledResponseContext {
                        thread_id: thread_key.clone(),
                        channel: channel.clone(),
                        account_id: account_id.clone(),
                        chat_id,
                        delivery_target_type: delivery_target_type.clone(),
                        delivery_target_id: delivery_target_id.clone(),
                        delivery_thread_id: thread_id.clone(),
                        thread_log_id,
                    },
                )
            })
        };
        inject_managed_mcp_servers(&runtime.managed_mcp_servers, &mut metadata);
        let thread_log_id = scheduled_thread_log_id(&thread_key, thread_id.as_deref());
        if let Some(thread_id) = &thread_log_id {
            runtime
                .thread_logs
                .record_event(
                    ThreadLogEvent::info(thread_id, "automation", "scheduled dispatch started")
                        .with_run_id(run_id.to_owned())
                        .with_field("job_id", serde_json::json!(job.id))
                        .with_field("job_kind", serde_json::json!(format!("{:?}", job.kind)))
                        .with_field(
                            "source",
                            serde_json::json!(if automation_job { "automation" } else { "cron" }),
                        )
                        .with_field("channel", serde_json::json!(channel))
                        .with_field("account_id", serde_json::json!(account_id))
                        .with_field("thread_id", serde_json::json!(thread_key)),
                )
                .await;
        }
        if let Err(error) = sync_default_external_user_skills() {
            tracing::warn!(
                error = %error,
                thread_id = %thread_key,
                "failed to sync external user skills before scheduled dispatch"
            );
        }
        if let Err(error) = runtime
            .bridge
            .start_agent_run(
                AgentRunRequest::new(
                    &thread_key,
                    message,
                    run_id,
                    &channel,
                    &account_id,
                    metadata,
                )
                .with_workspace_dir(workspace_dir),
                response_callback,
            )
            .await
        {
            if let Some(thread_id) = &thread_log_id {
                runtime
                    .thread_logs
                    .record_event(
                        ThreadLogEvent::error(thread_id, "automation", "scheduled dispatch failed")
                            .with_run_id(run_id.to_owned())
                            .with_field("job_id", serde_json::json!(job.id))
                            .with_field("error", serde_json::json!(error.to_string())),
                    )
                    .await;
            }
            return Err(format!("cron dispatch failed: {error}"));
        }
        active_agent_runs
            .write()
            .await
            .insert(job.id.clone(), run_id.to_owned());
        if let Some(thread_id) = &thread_log_id {
            runtime
                .thread_logs
                .record_event(
                    ThreadLogEvent::info(thread_id, "automation", "scheduled dispatch accepted")
                        .with_run_id(run_id.to_owned())
                        .with_field("job_id", serde_json::json!(job.id))
                        .with_field("thread_id", serde_json::json!(thread_key)),
                )
                .await;
        }

        Ok(())
    }
}

fn validate_cron_schedule(schedule: &CronSchedule) -> std::io::Result<()> {
    match schedule {
        CronSchedule::Interval { interval_secs } => {
            if *interval_secs > MAX_INTERVAL_SECS {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("interval schedule exceeds max interval_secs={MAX_INTERVAL_SECS}"),
                ));
            }
        }
        CronSchedule::Once { at } => {
            if parse_once_timestamp(at).is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid once timestamp: {at}"),
                ));
            }
        }
        CronSchedule::Cron { expr, timezone } => {
            if parse_cron_schedule(expr).is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid cron expression: {expr}"),
                ));
            }

            if let Some(tz_name) = timezone.as_deref().map(str::trim).filter(|s| !s.is_empty())
                && tz_name.parse::<Tz>().is_err()
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid cron timezone: {tz_name}"),
                ));
            }
        }
    }

    Ok(())
}

fn scheduled_thread_log_id(thread_key: &str, thread_id: Option<&str>) -> Option<String> {
    thread_id
        .map(str::trim)
        .filter(|value| is_canonical_thread_id(value))
        .map(ToOwned::to_owned)
        .or_else(|| {
            let trimmed = thread_key.trim();
            is_canonical_thread_id(trimmed).then(|| trimmed.to_owned())
        })
}

fn format_scheduled_message(text: &str, thread_id: &str) -> String {
    if text.is_empty() || !MessageRouter::is_scheduled_thread(thread_id) {
        return text.to_owned();
    }

    let header = format!("#{thread_id}");
    if text.trim_start().starts_with(&header) {
        return text.to_owned();
    }

    format!("{header}\n{text}")
}

struct ScheduledResponseContext {
    thread_id: String,
    channel: String,
    account_id: String,
    chat_id: String,
    delivery_target_type: String,
    delivery_target_id: String,
    delivery_thread_id: Option<String>,
    thread_log_id: Option<String>,
}

fn build_scheduled_response_callback(
    dispatcher: Arc<dyn ChannelDispatcher>,
    router: Arc<tokio::sync::Mutex<MessageRouter>>,
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
        let router = router.clone();
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
        let thread_key_value = thread_id.clone();
        let delivery_thread_id_value = delivery_thread_id.clone();
        let thread_log_id_value = thread_log_id.clone();

        tokio::spawn(async move {
            match dispatcher.send_message(request).await {
                Ok(SendMessageResult { message_ids }) => {
                    if message_ids.is_empty() {
                        return;
                    }
                    let mut router_guard = router.lock().await;
                    for message_id in message_ids {
                        router_guard
                            .record_outbound_message_with_thread_log(
                                &thread_key_value,
                                &channel_name,
                                &account_name,
                                &chat_id_value,
                                delivery_thread_id_value.as_deref(),
                                &message_id,
                                thread_log_id_value.as_deref(),
                            )
                            .await;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to send scheduled cron response");
                }
            }
        });
    })
}

fn append_inline_assistant_separator(buffer: &mut String) {
    if buffer.trim().is_empty() || buffer.ends_with("\n\n") {
        return;
    }
    if buffer.ends_with('\n') {
        buffer.push('\n');
    } else {
        buffer.push_str("\n\n");
    }
}

#[derive(Default)]
struct ScheduledStreamState {
    text: String,
    closed_after_user_ack: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
