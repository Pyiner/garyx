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

impl WorkflowStore {
    pub fn new(db: Arc<GaryxDbService>) -> Self {
        Self { db }
    }

    pub fn create_run(&self, draft: WorkflowRunDraft) -> Result<WorkflowRunRecord, WorkflowError> {
        Ok(self.db.create_workflow_run(draft)?)
    }

    pub fn get_run(&self, workflow_run_id: &str) -> Result<WorkflowRunRecord, WorkflowError> {
        self.db.get_workflow_run(workflow_run_id)?.ok_or_else(|| {
            WorkflowError::NotFound(format!("workflow run not found: {workflow_run_id}"))
        })
    }

    pub fn drilldown_snapshot(
        &self,
        workflow_run_id: &str,
        after_event_seq: u64,
        events_limit: usize,
    ) -> Result<WorkflowRunDrilldownSnapshot, WorkflowError> {
        self.db
            .get_workflow_run_drilldown_snapshot(workflow_run_id, after_event_seq, events_limit)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow run not found: {workflow_run_id}"))
            })
    }

    pub fn list_runs(
        &self,
        parent_thread_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WorkflowRunRecord>, WorkflowError> {
        Ok(self
            .db
            .list_workflow_runs(parent_thread_id, limit, offset)?)
    }

    pub fn children(
        &self,
        workflow_run_id: &str,
    ) -> Result<Vec<WorkflowChildRunRecord>, WorkflowError> {
        Ok(self.db.list_workflow_child_runs(workflow_run_id)?)
    }

    pub fn events_after(
        &self,
        workflow_run_id: &str,
        after_event_seq: u64,
        limit: usize,
    ) -> Result<Vec<WorkflowEventRecord>, WorkflowError> {
        Ok(self
            .db
            .list_workflow_events_after(workflow_run_id, after_event_seq, limit)?)
    }

    pub fn append_event(
        &self,
        draft: WorkflowEventDraft,
    ) -> Result<WorkflowEventRecord, WorkflowError> {
        Ok(self.db.append_workflow_event(draft)?)
    }

    pub fn cancel_run(&self, workflow_run_id: &str) -> Result<bool, WorkflowError> {
        let Some(existing) = self.db.get_workflow_run(workflow_run_id)? else {
            return Ok(false);
        };
        if matches!(
            existing.status.as_str(),
            "succeeded" | "failed" | "cancelled"
        ) {
            return Err(WorkflowError::Conflict(format!(
                "workflow is already terminal: {}",
                existing.status
            )));
        }
        let updated = self.db.update_workflow_run_status(
            workflow_run_id,
            "cancelled",
            None,
            None,
            Some("cancelled by user"),
        )?;
        if updated {
            let _ = self
                .db
                .cancel_workflow_child_runs(workflow_run_id, "cancelled by user")?;
            self.append_event(WorkflowEventDraft {
                event_id: None,
                workflow_id: workflow_run_id.to_owned(),
                workflow_child_run_id: None,
                thread_id: None,
                event_type: "workflow.cancelled".to_owned(),
                payload_json: json!({"reason": "user"}).to_string(),
            })?;
        }
        Ok(updated)
    }
}
