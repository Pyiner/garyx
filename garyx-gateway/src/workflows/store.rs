use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde_json::json;

use crate::garyx_db::{
    GaryxDbService, WorkflowChildRunRecord, WorkflowEventDraft, WorkflowEventRecord,
    WorkflowRunDraft, WorkflowRunDrilldownSnapshot, WorkflowRunRecord,
};

use super::WorkflowError;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowDefinitionRecord {
    pub workflow_id: String,
    pub version: u64,
    pub name: String,
    pub description: Option<String>,
    pub input_json: String,
    pub defaults_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowDefinitionPackage {
    pub record: WorkflowDefinitionRecord,
    pub package_dir: PathBuf,
}

#[derive(Clone)]
pub struct WorkflowStore {
    db: Arc<GaryxDbService>,
}

/// Async facade over the workflow tables. Every method routes through
/// `run_blocking` so workflow HTTP/SDK traffic never runs SQLite work on a
/// runtime worker (#TASK-1829 batch 3, review #TASK-1936 finding).
impl WorkflowStore {
    pub fn new(db: Arc<GaryxDbService>) -> Self {
        Self { db }
    }

    pub async fn create_run(
        &self,
        draft: WorkflowRunDraft,
    ) -> Result<WorkflowRunRecord, WorkflowError> {
        Ok(self
            .db
            .run_blocking(move |db| db.create_workflow_run(draft))
            .await?)
    }

    pub async fn get_run(&self, workflow_run_id: &str) -> Result<WorkflowRunRecord, WorkflowError> {
        let id = workflow_run_id.to_owned();
        self.db
            .run_blocking(move |db| db.get_workflow_run(&id))
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow run not found: {workflow_run_id}"))
            })
    }

    pub async fn drilldown_snapshot(
        &self,
        workflow_run_id: &str,
        after_event_seq: u64,
        events_limit: usize,
    ) -> Result<WorkflowRunDrilldownSnapshot, WorkflowError> {
        let id = workflow_run_id.to_owned();
        self.db
            .run_blocking(move |db| {
                db.get_workflow_run_drilldown_snapshot(&id, after_event_seq, events_limit)
            })
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow run not found: {workflow_run_id}"))
            })
    }

    pub async fn list_runs(
        &self,
        parent_thread_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WorkflowRunRecord>, WorkflowError> {
        let parent_thread_id = parent_thread_id.map(ToOwned::to_owned);
        Ok(self
            .db
            .run_blocking(move |db| {
                db.list_workflow_runs(parent_thread_id.as_deref(), limit, offset)
            })
            .await?)
    }

    pub async fn children(
        &self,
        workflow_run_id: &str,
    ) -> Result<Vec<WorkflowChildRunRecord>, WorkflowError> {
        let id = workflow_run_id.to_owned();
        Ok(self
            .db
            .run_blocking(move |db| db.list_workflow_child_runs(&id))
            .await?)
    }

    pub async fn events_after(
        &self,
        workflow_run_id: &str,
        after_event_seq: u64,
        limit: usize,
    ) -> Result<Vec<WorkflowEventRecord>, WorkflowError> {
        let id = workflow_run_id.to_owned();
        Ok(self
            .db
            .run_blocking(move |db| db.list_workflow_events_after(&id, after_event_seq, limit))
            .await?)
    }

    pub async fn append_event(
        &self,
        draft: WorkflowEventDraft,
    ) -> Result<WorkflowEventRecord, WorkflowError> {
        Ok(self
            .db
            .run_blocking(move |db| db.append_workflow_event(draft))
            .await?)
    }

    pub async fn cancel_run(&self, workflow_run_id: &str) -> Result<bool, WorkflowError> {
        // The whole read-check-update flow stays inside one blocking hop on
        // the writer path, preserving the previous ordering semantics.
        let id = workflow_run_id.to_owned();
        let updated = self
            .db
            .run_blocking(move |db| {
                let Some(existing) = db.get_workflow_run(&id)? else {
                    return Ok(None);
                };
                if matches!(
                    existing.status.as_str(),
                    "succeeded" | "failed" | "cancelled"
                ) {
                    return Ok(Some(Err(existing.status)));
                }
                let updated = db.update_workflow_run_status(
                    &id,
                    "cancelled",
                    None,
                    None,
                    Some("cancelled by user"),
                )?;
                if updated {
                    let _ = db.cancel_workflow_child_runs(&id, "cancelled by user")?;
                    db.append_workflow_event(WorkflowEventDraft {
                        event_id: None,
                        workflow_id: id.clone(),
                        workflow_child_run_id: None,
                        thread_id: None,
                        event_type: "workflow.cancelled".to_owned(),
                        payload_json: json!({"reason": "user"}).to_string(),
                    })?;
                }
                Ok(Some(Ok(updated)))
            })
            .await?;
        match updated {
            None => Ok(false),
            Some(Err(status)) => Err(WorkflowError::Conflict(format!(
                "workflow is already terminal: {status}"
            ))),
            Some(Ok(updated)) => Ok(updated),
        }
    }
}
