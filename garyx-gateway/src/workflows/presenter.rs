use serde_json::{Value, json};

use crate::garyx_db::{WorkflowChildRunRecord, WorkflowEventRecord, WorkflowRunRecord};

use super::WorkflowError;
use super::runtime::WorkflowAgentExecutionResult;
use super::store::{WorkflowDefinitionPackage, WorkflowDefinitionRecord, WorkflowStore};

pub(super) fn workflow_payload(
    store: &WorkflowStore,
    workflow_run_id: &str,
) -> Result<Value, WorkflowError> {
    let workflow = store.get_run(workflow_run_id)?;
    workflow_run_drilldown_json(store, &workflow)
}

pub(super) fn workflow_run_drilldown_json(
    store: &WorkflowStore,
    workflow: &WorkflowRunRecord,
) -> Result<Value, WorkflowError> {
    let workflow_run_id = &workflow.workflow_id;
    let children = store.children(workflow_run_id)?;
    let events = store.events_after(workflow_run_id, 0, 200)?;
    Ok(json!({
        "workflow": workflow_run_json(workflow),
        "children": children.iter().map(workflow_child_json).collect::<Vec<_>>(),
        "events": events.iter().map(workflow_event_json).collect::<Vec<_>>(),
    }))
}

fn workflow_definition_json(record: &WorkflowDefinitionRecord) -> Value {
    json!({
        "workflowId": record.workflow_id,
        "version": record.version,
        "name": record.name,
        "description": record.description,
        "input": parse_json_field(&record.input_json),
        "defaults": parse_json_field(&record.defaults_json),
        "createdAt": record.created_at,
        "updatedAt": record.updated_at,
    })
}

pub(super) fn workflow_definition_package_json(package: &WorkflowDefinitionPackage) -> Value {
    let mut value = workflow_definition_json(&package.record);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "packageDir".to_owned(),
            Value::String(package.package_dir.to_string_lossy().to_string()),
        );
    }
    value
}

pub(super) fn workflow_run_json(record: &WorkflowRunRecord) -> Value {
    json!({
        "workflowRunId": record.workflow_id,
        "threadId": record.workflow_id,
        // Compatibility alias for callers created before WorkflowRun became the
        // public identity name.
        "workflowId": record.workflow_id,
        "taskId": record.task_id,
        "taskThreadId": record.task_thread_id,
        "workflowDefinitionId": record.workflow_definition_id,
        "workflowDefinitionVersion": record.workflow_definition_version,
        "workflowDefinitionSnapshot": record.workflow_definition_snapshot_json.as_deref().map(parse_json_field),
        "input": record.input_json.as_deref().map(parse_json_field),
        "parentThreadId": record.parent_thread_id,
        "parentRunId": record.parent_run_id,
        "name": record.name,
        "description": record.description,
        "status": record.status,
        "currentPhaseIndex": record.current_phase_index,
        "scriptText": record.script_text,
        "meta": parse_json_field(&record.meta_json),
        "result": record.result_json.as_deref().map(parse_json_field),
        "outputText": record.output_text,
        "error": record.error,
        "workspaceDir": record.workspace_dir,
        "createdBy": record.created_by,
        "totalChildren": record.total_children,
        "completedChildren": record.completed_children,
        "failedChildren": record.failed_children,
        "totalInputTokens": record.total_input_tokens,
        "totalOutputTokens": record.total_output_tokens,
        "totalToolCalls": record.total_tool_calls,
        "totalCostUsd": record.total_cost_usd,
        "createdAt": record.created_at,
        "startedAt": record.started_at,
        "finishedAt": record.finished_at,
        "updatedAt": record.updated_at,
    })
}

pub(super) fn workflow_child_json(record: &WorkflowChildRunRecord) -> Value {
    json!({
        "workflowRunId": record.workflow_id,
        "workflowId": record.workflow_id,
        "workflowChildRunId": record.workflow_child_run_id,
        "threadId": record.thread_id,
        "phaseIndex": record.phase_index,
        "phaseTitle": record.phase_title,
        "label": record.label,
        "agentId": record.agent_id,
        "status": record.status,
        "prompt": record.prompt,
        "resultMode": record.result_mode,
        "schema": record.schema_json.as_deref().map(parse_json_field),
        "resultText": record.result_text,
        "result": record.result_json.as_deref().map(parse_json_field),
        "resultPreview": record.result_preview,
        "error": record.error,
        "inputTokens": record.input_tokens,
        "outputTokens": record.output_tokens,
        "toolCalls": record.tool_calls,
        "costUsd": record.cost_usd,
        "queuedAt": record.queued_at,
        "startedAt": record.started_at,
        "finishedAt": record.finished_at,
        "updatedAt": record.updated_at,
    })
}

pub(super) fn workflow_event_json(record: &WorkflowEventRecord) -> Value {
    json!({
        "eventSeq": record.event_seq,
        "eventId": record.event_id,
        "workflowRunId": record.workflow_id,
        "workflowId": record.workflow_id,
        "workflowChildRunId": record.workflow_child_run_id,
        "threadId": record.thread_id,
        "eventType": record.event_type,
        "payload": parse_json_field(&record.payload_json),
        "createdAt": record.created_at,
    })
}

pub(super) fn workflow_agent_result_json(result: &WorkflowAgentExecutionResult) -> Value {
    let binding = result.binding.as_deref().map(normalize_result_binding_name);
    json!({
        "workflowChildRunId": result.workflow_child_run_id,
        "threadId": result.thread_id,
        "label": result.label,
        "binding": binding,
        "orderIndex": result.order_index,
        "phaseTitle": result.phase_title,
        "result": result.result,
        "resultPreview": result.preview,
        "failed": result.failed,
        "optional": result.optional,
        "error": result.error,
    })
}

pub(super) fn binding_collection_name(binding: &str) -> Option<&str> {
    binding
        .strip_suffix("[]")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(super) fn normalize_result_binding_name(binding: &str) -> &str {
    binding_collection_name(binding).unwrap_or(binding)
}

pub(super) fn parse_json_field(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}
