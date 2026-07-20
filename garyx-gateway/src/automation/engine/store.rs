//! On-disk persistence for jobs and run records
//! (`<data_dir>/cron/jobs/*.json`, `<data_dir>/cron/runs.json`).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::model::{CronJob, RunRecord};

pub(super) const MAX_RUN_HISTORY: usize = 200;

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

/// Directory layout: `<data_dir>/cron/jobs/<id>.json`
pub(super) fn jobs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cron").join("jobs")
}

pub(super) fn runs_file(data_dir: &Path) -> PathBuf {
    data_dir.join("cron").join("runs.json")
}

pub(super) async fn ensure_dirs(data_dir: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(jobs_dir(data_dir)).await
}

pub(super) async fn persist_job(data_dir: &Path, job: &CronJob) -> std::io::Result<()> {
    let path = jobs_dir(data_dir).join(format!("{}.json", job.id));
    let tmp = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(job).map_err(std::io::Error::other)?;
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

pub(super) async fn delete_job_file(data_dir: &Path, id: &str) -> std::io::Result<()> {
    let path = jobs_dir(data_dir).join(format!("{id}.json"));
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub(super) async fn load_jobs(data_dir: &Path) -> std::io::Result<Vec<CronJob>> {
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
                    tracing::warn!(target: "garyx_gateway::cron", path = %path.display(), error = %e, "skipping corrupt cron job file");
                    let _ = tokio::fs::remove_file(&path).await;
                }
            },
            Err(e) => {
                tracing::warn!(target: "garyx_gateway::cron", path = %path.display(), error = %e, "failed to read cron job file");
            }
        }
    }
    Ok(jobs)
}

pub(super) async fn load_runs(data_dir: &Path) -> std::io::Result<VecDeque<RunRecord>> {
    let path = runs_file(data_dir);
    if !path.exists() {
        return Ok(VecDeque::new());
    }

    let bytes = tokio::fs::read(&path).await?;
    let records: Vec<RunRecord> = match serde_json::from_slice(&bytes) {
        Ok(records) => records,
        Err(error) => {
            tracing::warn!(target: "garyx_gateway::cron",
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

pub(super) async fn persist_runs(
    data_dir: &Path,
    runs: &VecDeque<RunRecord>,
) -> std::io::Result<()> {
    let path = runs_file(data_dir);
    let tmp = path.with_extension("tmp");
    let list: Vec<RunRecord> = runs.iter().cloned().collect();
    let bytes = serde_json::to_vec_pretty(&list).map_err(std::io::Error::other)?;
    tokio::fs::write(&tmp, &bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}
