use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{DEFAULT_GLOBAL_CHILD_LIMIT, DEFAULT_PER_WORKFLOW_CHILD_LIMIT, WorkflowError};

#[derive(Debug, Clone)]
pub struct WorkflowScheduler {
    global_child_permits: Arc<Semaphore>,
    per_workflow_limit: usize,
    per_workflow: Arc<tokio::sync::Mutex<HashMap<String, Arc<Semaphore>>>>,
}

impl Default for WorkflowScheduler {
    fn default() -> Self {
        Self::new(DEFAULT_GLOBAL_CHILD_LIMIT, DEFAULT_PER_WORKFLOW_CHILD_LIMIT)
    }
}

impl WorkflowScheduler {
    pub fn new(global_limit: usize, per_workflow_limit: usize) -> Self {
        Self {
            global_child_permits: Arc::new(Semaphore::new(global_limit.max(1))),
            per_workflow_limit: per_workflow_limit.max(1),
            per_workflow: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn acquire_child_permit(
        &self,
        workflow_run_id: &str,
    ) -> Result<WorkflowChildPermit, WorkflowError> {
        let workflow_run_id = workflow_run_id.trim();
        if workflow_run_id.is_empty() {
            return Err(WorkflowError::BadRequest(
                "workflowRunId must not be empty".to_owned(),
            ));
        }
        let per_workflow = {
            let mut entries = self.per_workflow.lock().await;
            entries
                .entry(workflow_run_id.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(self.per_workflow_limit)))
                .clone()
        };
        let workflow_permit = per_workflow
            .acquire_owned()
            .await
            .map_err(|_| WorkflowError::Conflict("workflow child limiter closed".to_owned()))?;
        let global_permit = self
            .global_child_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                WorkflowError::Conflict("global workflow child limiter closed".to_owned())
            })?;
        Ok(WorkflowChildPermit {
            _workflow: workflow_permit,
            _global: global_permit,
        })
    }

    pub fn available_global_child_slots(&self) -> usize {
        self.global_child_permits.available_permits()
    }
}

pub struct WorkflowChildPermit {
    _workflow: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}
