//! The cron engine: job model, schedule math, persistence, and the
//! scheduler service that drives job execution.
//!
//! Layout: [`model`] (persisted job state), [`schedule`] (cron/timezone
//! math), [`store`] (on-disk jobs + run records), [`execution`] (tick,
//! claim/prepare/execute/settle, dispatch). This hub owns [`CronService`]
//! itself: lifecycle (`new`/`load`/`start`/`stop`) and the CRUD surface.

mod execution;
mod model;
mod schedule;
mod store;

#[cfg(test)]
pub(crate) use execution::build_followup_body;
pub(crate) use model::validate_cron_job;
pub use model::{CronJob, JobRunStatus, RunRecord};
pub(crate) use schedule::parse_once_timestamp;

#[allow(unused_imports)]
use execution::*;
#[allow(unused_imports)]
use model::*;
#[allow(unused_imports)]
use schedule::*;
#[allow(unused_imports)]
use store::*;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use chrono::Utc;
use garyx_models::config::{CronConfig, CronJobConfig};
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use super::dispatch::AutomationExecEnv;

// ---------------------------------------------------------------------------
// CronService
// ---------------------------------------------------------------------------

/// Cron scheduler service.
///
/// Lifecycle: `start(env)` spawns a background tokio task that ticks every
/// second, checking whether any job is due. `stop()` sends a signal to
/// terminate the loop. The execution environment is injected exactly once at
/// start time; there are no post-construction mutation channels.
pub struct CronService {
    data_dir: PathBuf,
    jobs: Arc<RwLock<HashMap<String, CronJob>>>,
    runs: Arc<RwLock<VecDeque<RunRecord>>>,
    active_agent_runs: Arc<RwLock<HashMap<String, String>>>,
    /// Send () to stop the scheduler loop.
    stop_tx: StdMutex<Option<mpsc::Sender<()>>>,
    scheduler_task: StdMutex<Option<JoinHandle<()>>>,
}

impl CronService {
    /// Create a new CronService.
    ///
    /// Does NOT start the scheduler loop. Call `start(env)` after creation.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(RwLock::new(VecDeque::new())),
            active_agent_runs: Arc::new(RwLock::new(HashMap::new())),
            stop_tx: StdMutex::new(None),
            scheduler_task: StdMutex::new(None),
        }
    }

    /// Attach bridge+router runtime for agent-turn/system-event dispatch.
    /// Load persisted jobs from disk, then merge config-defined jobs
    /// (config jobs take precedence for schedule/action, but persisted
    /// runtime state like run_count is preserved).
    pub async fn load(&self, config: &CronConfig) -> std::io::Result<()> {
        ensure_dirs(&self.data_dir).await?;

        // Load from disk first.
        let disk_jobs = load_jobs(&self.data_dir).await?;
        let mut map = HashMap::new();
        for mut job in disk_jobs {
            if let Err(error) = validate_cron_schedule(&job.schedule) {
                tracing::warn!(target: "garyx_gateway::cron",
                    job_id = %job.id,
                    error = %error,
                    "skipping persisted cron job with invalid schedule"
                );
                let _ = delete_job_file(&self.data_dir, &job.id).await;
                continue;
            }
            // A `Running` status persisted across a restart is a stale claim:
            // the run that set it was killed with the previous process (Garyx
            // restarts are non-graceful / SIGKILL and never settle the in-flight
            // tick). No run survives a restart, so treat it as an interrupted
            // failure and make the job claimable again -- otherwise
            // `claim_job_for_execution` skips it forever and the schedule
            // silently stops firing with no recovery via the UI. This mirrors
            // the startup reconciliation that repairs interrupted threads
            // and tasks.
            if job.last_status == JobRunStatus::Running {
                tracing::warn!(target: "garyx_gateway::cron",
                    job_id = %job.id,
                    "resetting stale `Running` cron job left by an interrupted run/restart"
                );
                job.last_status = JobRunStatus::Failed;
            }
            job.normalize_agent_contract();
            job.revalidate();
            map.insert(job.id.clone(), job);
        }

        // Merge config-defined jobs.
        for cfg_job in &config.jobs {
            if let Err(error) = validate_cron_schedule(&cfg_job.schedule) {
                tracing::warn!(target: "garyx_gateway::cron",
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
                existing.normalize_agent_contract();
                existing.revalidate();
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

        tracing::info!(target: "garyx_gateway::cron", count = self.jobs.read().await.len(), "cron jobs loaded");
        Ok(())
    }

    /// Start the scheduler loop as a background task.
    /// Start the scheduler loop with its execution environment.
    ///
    /// Called once after the application state is assembled (see
    /// `composition::automation_wiring::start_automation_scheduler`); the env
    /// is owned by the loop for its whole lifetime.
    pub(crate) fn start(&self, env: AutomationExecEnv) {
        let mut stop_slot = self.stop_tx.lock().expect("cron stop_tx lock poisoned");
        if stop_slot.is_some() {
            tracing::warn!(target: "garyx_gateway::cron", "cron scheduler already running; duplicate start ignored");
            return;
        }

        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        *stop_slot = Some(stop_tx);
        drop(stop_slot);

        let jobs = self.jobs.clone();
        let runs = self.runs.clone();
        let active_agent_runs = self.active_agent_runs.clone();
        let data_dir = self.data_dir.clone();

        let task = tokio::spawn(async move {
            tracing::info!(target: "garyx_gateway::cron", "cron scheduler started");
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = stop_rx.recv() => {
                        tracing::info!(target: "garyx_gateway::cron", "cron scheduler stopping");
                        break;
                    }
                    _ = interval.tick() => {
                        Self::tick(
                            &jobs,
                            &runs,
                            &active_agent_runs,
                            &data_dir,
                            &env,
                        ).await;
                    }
                }
            }
            tracing::info!(target: "garyx_gateway::cron", "cron scheduler stopped");
        });
        *self
            .scheduler_task
            .lock()
            .expect("cron scheduler_task lock poisoned") = Some(task);
    }

    /// Stop the scheduler loop.
    pub async fn stop(&self) {
        let stop_tx = self
            .stop_tx
            .lock()
            .expect("cron stop_tx lock poisoned")
            .take();
        if let Some(tx) = stop_tx {
            let _ = tx.send(()).await;
        }
        let task = self
            .scheduler_task
            .lock()
            .expect("cron scheduler_task lock poisoned")
            .take();
        if let Some(task) = task {
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
        tracing::info!(target: "garyx_gateway::cron", job_id = %cfg.id, "cron job added");
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
            tracing::info!(target: "garyx_gateway::cron", job_id = %cfg.id, "cron job replaced via upsert");
        } else {
            tracing::info!(target: "garyx_gateway::cron", job_id = %cfg.id, "cron job added via upsert");
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
            job.normalize_agent_contract();
            job.revalidate();

            job.clone()
        };

        persist_job(&self.data_dir, &updated).await?;
        tracing::info!(target: "garyx_gateway::cron", job_id = %id, "cron job updated");
        Ok(Some(updated))
    }

    /// Delete a job by ID.
    pub async fn delete(&self, id: &str) -> std::io::Result<bool> {
        let removed = self.jobs.write().await.remove(id).is_some();
        if removed {
            self.active_agent_runs.write().await.remove(id);
            delete_job_file(&self.data_dir, id).await?;
            tracing::info!(target: "garyx_gateway::cron", job_id = %id, "cron job deleted");
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
