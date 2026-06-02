use std::sync::Arc;

use garyx_models::Principal;
use garyx_router::mark_thread_task_in_review_if_in_progress;

use crate::garyx_db::WorkflowRunRecord;
use crate::server::AppState;

use super::{WorkflowError, WorkflowStore};

pub async fn cancel_workflow_run(
    state: &Arc<AppState>,
    workflow_run_id: &str,
) -> Result<bool, WorkflowError> {
    let store = WorkflowStore::new(state.ops.garyx_db.clone());
    match store.cancel_run(workflow_run_id) {
        Ok(true) => {
            let run = store.get_run(workflow_run_id)?;
            mark_workflow_run_task_in_review(
                state,
                &run,
                format!("workflow cancelled: {workflow_run_id}"),
            )
            .await?;
            Ok(true)
        }
        Ok(false) => Ok(false),
        Err(error) => Err(error),
    }
}

pub async fn reconcile_interrupted_workflows(
    state: &Arc<AppState>,
    created_before_or_at: &str,
) -> usize {
    let interrupted_tasks = match state
        .ops
        .garyx_db
        .list_interrupted_workflow_task_references(created_before_or_at)
    {
        Ok(records) => records,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to list interrupted workflow task references"
            );
            Vec::new()
        }
    };
    match state
        .ops
        .garyx_db
        .reconcile_interrupted_workflows("gateway restarted", created_before_or_at)
    {
        Ok(count) => {
            for reference in interrupted_tasks {
                if let Err(error) = mark_workflow_task_in_review(
                    state,
                    &reference.task_thread_id,
                    format!(
                        "workflow failed after gateway restart: {}",
                        reference.workflow_id
                    ),
                )
                .await
                {
                    tracing::warn!(
                        error = %error,
                        workflow_id = %reference.workflow_id,
                        task_thread_id = %reference.task_thread_id,
                        "failed to mark interrupted workflow task in review"
                    );
                }
            }
            count
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to reconcile interrupted workflows");
            0
        }
    }
}

pub(super) async fn mark_workflow_run_task_in_review(
    state: &Arc<AppState>,
    run: &WorkflowRunRecord,
    note: String,
) -> Result<(), WorkflowError> {
    if let Some(task_thread_id) = run.task_thread_id.as_deref() {
        mark_workflow_task_in_review(state, task_thread_id, note).await?;
    }
    Ok(())
}

pub(super) async fn mark_workflow_task_in_review(
    state: &Arc<AppState>,
    task_thread_id: &str,
    note: String,
) -> Result<(), WorkflowError> {
    mark_thread_task_in_review_if_in_progress(
        &state.threads.thread_store,
        task_thread_id,
        Principal::Agent {
            agent_id: "workflow".to_owned(),
        },
        Some(note),
    )
    .await
    .map(|_| ())
    .map_err(|error| WorkflowError::BadRequest(error.to_string()))
}
