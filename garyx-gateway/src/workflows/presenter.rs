use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::garyx_db::{
    WorkflowChildRunRecord, WorkflowEventRecord, WorkflowRunDrilldownSnapshot, WorkflowRunRecord,
};

use super::WorkflowError;
use super::runtime::WorkflowAgentExecutionResult;
use super::store::{WorkflowDefinitionPackage, WorkflowDefinitionRecord, WorkflowStore};

pub(super) fn workflow_payload(
    store: &WorkflowStore,
    workflow_run_id: &str,
) -> Result<Value, WorkflowError> {
    let snapshot = store.drilldown_snapshot(workflow_run_id, 0, WORKFLOW_INLINE_EVENTS_LIMIT)?;
    workflow_run_drilldown_snapshot_json(&snapshot)
}

fn workflow_run_drilldown_snapshot_json(
    snapshot: &WorkflowRunDrilldownSnapshot,
) -> Result<Value, WorkflowError> {
    let workflow = &snapshot.workflow;
    Ok(json!({
        "workflow": workflow_run_json(workflow),
        "children": snapshot.children.iter().map(workflow_child_json).collect::<Vec<_>>(),
        "events": snapshot.events.iter().map(workflow_event_json).collect::<Vec<_>>(),
        "presentation": workflow_presentation_json(snapshot),
    }))
}

const WORKFLOW_INLINE_EVENTS_LIMIT: usize = 200;
const WORKFLOW_PRESENTATION_VERSION: u64 = 1;

#[derive(Debug, Clone)]
struct WorkflowPhaseProjection {
    phase_id: String,
    index: Option<i64>,
    title: String,
    detail: Option<String>,
    status: String,
    active: bool,
    children: Vec<WorkflowChildRunRecord>,
}

fn workflow_presentation_json(snapshot: &WorkflowRunDrilldownSnapshot) -> Value {
    let workflow = &snapshot.workflow;
    let phases = workflow_phase_projection(snapshot);
    let child_cards = sorted_child_cards(&snapshot.children);
    let terminal_child_count = snapshot
        .children
        .iter()
        .filter(|child| is_terminal_child_status(&child.status))
        .count() as u64;
    let failed_counter_count = snapshot
        .children
        .iter()
        .filter(|child| matches!(child.status.as_str(), "failed" | "cancelled"))
        .count() as u64;
    let running_child_count = snapshot
        .children
        .iter()
        .filter(|child| child.status == "running")
        .count() as u64;
    let queued_child_count = snapshot
        .children
        .iter()
        .filter(|child| child.status == "queued")
        .count() as u64;
    let skipped_child_count = snapshot
        .children
        .iter()
        .filter(|child| child.status == "skipped")
        .count() as u64;
    let completed_phase_count = phases
        .iter()
        .filter(|phase| is_terminal_phase_status(&phase.status))
        .count() as u64;
    let active_phase = phases.iter().find(|phase| phase.active);
    let last_inline_event_seq = snapshot
        .events
        .last()
        .map(|event| event.event_seq)
        .unwrap_or(0);
    let inline_events_truncated = last_inline_event_seq < snapshot.latest_event_seq;
    let terminal_complete = terminal_complete(workflow, &snapshot.children);
    let stale = is_terminal_workflow_status(&workflow.status) && !terminal_complete;
    let snapshot_version = snapshot_version(snapshot);

    json!({
        "version": WORKFLOW_PRESENTATION_VERSION,
        "workflowRunId": workflow.workflow_id,
        "threadId": workflow.workflow_id,
        "workflowDefinitionId": workflow.workflow_definition_id,
        "taskId": workflow.task_id,
        "taskThreadId": workflow.task_thread_id,
        "title": workflow.name,
        "description": workflow.description,
        "status": workflow.status,
        "counts": {
            "total": snapshot.children.len() as u64,
            "completed": terminal_child_count,
            "failedChildren": failed_counter_count,
            "runningChildren": running_child_count,
            "queuedChildren": queued_child_count,
            "skippedChildren": skipped_child_count,
            "totalPhases": phases.len() as u64,
            "completedPhases": completed_phase_count,
            "totalInputTokens": workflow.total_input_tokens,
            "totalOutputTokens": workflow.total_output_tokens,
            "totalToolCalls": workflow.total_tool_calls,
            "costUsd": workflow.total_cost_usd,
        },
        "activePhase": active_phase.map(phase_identity_json),
        "phaseStatus": phases.iter().map(phase_status_json).collect::<Vec<_>>(),
        "phases": phases.iter().map(phase_json).collect::<Vec<_>>(),
        "childCards": child_cards.iter().map(workflow_child_card_json).collect::<Vec<_>>(),
        "outcome": workflow_outcome_json(workflow),
        "outputText": workflow.output_text,
        "result": workflow.result_json.as_deref().map(parse_json_field),
        "error": workflow.error,
        "terminalComplete": terminal_complete,
        "stale": stale,
        "staleReason": if stale {
            Some("terminal_children_unconverged")
        } else {
            None
        },
        "snapshotVersion": snapshot_version,
        "latestEventSeq": snapshot.latest_event_seq,
        "eventsSeed": {
            "count": snapshot.events.len(),
            "latestSeedEventSeq": last_inline_event_seq,
            "truncated": inline_events_truncated,
        },
    })
}

fn workflow_phase_projection(
    snapshot: &WorkflowRunDrilldownSnapshot,
) -> Vec<WorkflowPhaseProjection> {
    let workflow = &snapshot.workflow;
    let current_phase_index = workflow.current_phase_index;
    let mut phases = BTreeMap::<(i64, String), WorkflowPhaseProjection>::new();
    let mut title_only_indexes = BTreeMap::<String, (i64, String)>::new();

    for plan in workflow_phase_plan(workflow) {
        let key = (plan.index.unwrap_or(i64::MAX), plan.title.clone());
        title_only_indexes.insert(plan.title.clone(), key.clone());
        phases
            .entry(key)
            .or_insert_with(|| WorkflowPhaseProjection {
                phase_id: plan.phase_id,
                index: plan.index,
                title: plan.title,
                detail: plan.detail,
                status: "queued".to_owned(),
                active: false,
                children: Vec::new(),
            });
    }

    for event in &snapshot.events {
        if event.event_type != "workflow.phase_started" {
            continue;
        }
        let payload = parse_json_field(&event.payload_json);
        let Some(title) = payload
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        let index = payload
            .get("phaseIndex")
            .or_else(|| payload.get("phase_index"))
            .and_then(Value::as_i64);
        let detail = payload
            .get("detail")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let key = (index.unwrap_or(i64::MAX), title.clone());
        title_only_indexes.insert(title.clone(), key.clone());
        phases
            .entry(key)
            .or_insert_with(|| WorkflowPhaseProjection {
                phase_id: phase_id(index, &title),
                index,
                title,
                detail,
                status: "queued".to_owned(),
                active: false,
                children: Vec::new(),
            });
    }

    for child in &snapshot.children {
        let key = child_phase_key(child, &title_only_indexes);
        title_only_indexes.insert(child.phase_title.clone(), key.clone());
        let phase = phases
            .entry(key)
            .or_insert_with(|| WorkflowPhaseProjection {
                phase_id: phase_id(Some(child.phase_index), &child.phase_title),
                index: Some(child.phase_index),
                title: child.phase_title.clone(),
                detail: None,
                status: "queued".to_owned(),
                active: false,
                children: Vec::new(),
            });
        phase.children.push(child.clone());
    }

    let mut projected = phases.into_values().collect::<Vec<_>>();
    projected.sort_by(|left, right| {
        left.index
            .unwrap_or(i64::MAX)
            .cmp(&right.index.unwrap_or(i64::MAX))
            .then_with(|| left.title.cmp(&right.title))
    });

    for phase in &mut projected {
        phase.children = sorted_child_cards(&phase.children);
        phase.active = !is_terminal_workflow_status(&workflow.status)
            && current_phase_index.is_some()
            && phase.index == current_phase_index;
        phase.status = derive_phase_status(phase, workflow);
    }

    projected
}

#[derive(Debug, Clone)]
struct WorkflowPhasePlan {
    phase_id: String,
    index: Option<i64>,
    title: String,
    detail: Option<String>,
}

fn workflow_phase_plan(workflow: &WorkflowRunRecord) -> Vec<WorkflowPhasePlan> {
    let meta = parse_json_field(&workflow.meta_json);
    let Some(entries) = meta.get("phases").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::<(Option<i64>, String)>::new();
    entries
        .iter()
        .enumerate()
        .filter_map(|(fallback_index, entry)| {
            let record = entry.as_object()?;
            let title = record
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_owned();
            let index = record
                .get("index")
                .and_then(Value::as_i64)
                .or(Some(fallback_index as i64));
            if !seen.insert((index, title.clone())) {
                return None;
            }
            let phase_id = record
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| phase_id(index, &title));
            let detail = record
                .get("detail")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            Some(WorkflowPhasePlan {
                phase_id,
                index,
                title,
                detail,
            })
        })
        .collect()
}

fn child_phase_key(
    child: &WorkflowChildRunRecord,
    title_only_indexes: &BTreeMap<String, (i64, String)>,
) -> (i64, String) {
    title_only_indexes
        .get(&child.phase_title)
        .cloned()
        .unwrap_or_else(|| (child.phase_index, child.phase_title.clone()))
}

fn derive_phase_status(phase: &WorkflowPhaseProjection, workflow: &WorkflowRunRecord) -> String {
    if phase.active {
        return "running".to_owned();
    }
    if phase.children.iter().any(|child| child.status == "running") {
        return "running".to_owned();
    }
    if phase.children.iter().any(|child| child.status == "failed") {
        return "failed".to_owned();
    }
    if phase
        .children
        .iter()
        .any(|child| child.status == "cancelled")
    {
        return "cancelled".to_owned();
    }
    if !phase.children.is_empty() && phase.children.iter().all(|child| child.status == "skipped") {
        return "skipped".to_owned();
    }
    if !phase.children.is_empty()
        && phase
            .children
            .iter()
            .all(|child| is_terminal_child_status(&child.status))
    {
        return "succeeded".to_owned();
    }
    if is_terminal_workflow_status(&workflow.status)
        && workflow.current_phase_index.is_some()
        && phase.index > workflow.current_phase_index
    {
        return "skipped".to_owned();
    }
    "queued".to_owned()
}

fn phase_identity_json(phase: &WorkflowPhaseProjection) -> Value {
    json!({
        "phaseId": phase.phase_id,
        "index": phase.index,
        "title": phase.title,
        "detail": phase.detail,
    })
}

fn phase_status_json(phase: &WorkflowPhaseProjection) -> Value {
    json!({
        "phaseId": phase.phase_id,
        "index": phase.index,
        "title": phase.title,
        "status": phase.status,
        "active": phase.active,
        "completedChildren": phase.children.iter().filter(|child| is_terminal_child_status(&child.status)).count(),
        "totalChildren": phase.children.len(),
        "failedChildren": phase.children.iter().filter(|child| matches!(child.status.as_str(), "failed" | "cancelled")).count(),
    })
}

fn phase_json(phase: &WorkflowPhaseProjection) -> Value {
    json!({
        "phaseId": phase.phase_id,
        "index": phase.index,
        "title": phase.title,
        "detail": phase.detail,
        "status": phase.status,
        "active": phase.active,
        "counts": {
            "completed": phase.children.iter().filter(|child| is_terminal_child_status(&child.status)).count(),
            "total": phase.children.len(),
            "failedChildren": phase.children.iter().filter(|child| matches!(child.status.as_str(), "failed" | "cancelled")).count(),
        },
        "children": phase.children.iter().map(workflow_child_card_json).collect::<Vec<_>>(),
    })
}

fn workflow_child_card_json(record: &WorkflowChildRunRecord) -> Value {
    let total_tokens = record.input_tokens.saturating_add(record.output_tokens);
    json!({
        "workflowChildRunId": record.workflow_child_run_id,
        "threadId": record.thread_id,
        "phaseIndex": record.phase_index,
        "phaseTitle": record.phase_title,
        "label": record.label,
        "agentId": record.agent_id,
        "status": record.status,
        "prompt": record.prompt,
        "resultMode": record.result_mode,
        "resultText": record.result_text,
        "result": record.result_json.as_deref().map(parse_json_field),
        "resultPreview": record.result_preview,
        "error": record.error,
        "inputTokens": record.input_tokens,
        "outputTokens": record.output_tokens,
        "tokens": total_tokens,
        "toolCalls": record.tool_calls,
        "costUsd": record.cost_usd,
        "queuedAt": record.queued_at,
        "startedAt": record.started_at,
        "finishedAt": record.finished_at,
        "updatedAt": record.updated_at,
    })
}

fn workflow_outcome_json(workflow: &WorkflowRunRecord) -> Value {
    let output_text = workflow
        .output_text
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let result = workflow
        .result_json
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty() && value != "null");
    let kind = match workflow.status.as_str() {
        "succeeded" if output_text => "finalText",
        "succeeded" if result => "structuredOnly",
        "succeeded" => "completedNoOutput",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "running",
    };
    json!({
        "kind": kind,
        "status": workflow.status,
        "hasOutputText": output_text,
        "hasResult": result,
        "error": workflow.error,
    })
}

fn terminal_complete(workflow: &WorkflowRunRecord, children: &[WorkflowChildRunRecord]) -> bool {
    if !is_terminal_workflow_status(&workflow.status) {
        return false;
    }
    let total = children.len() as u32;
    let completed = children
        .iter()
        .filter(|child| is_terminal_child_status(&child.status))
        .count() as u32;
    let failed = children
        .iter()
        .filter(|child| matches!(child.status.as_str(), "failed" | "cancelled"))
        .count() as u32;
    completed == total
        && workflow.total_children == total
        && workflow.completed_children == completed
        && workflow.failed_children == failed
}

fn snapshot_version(snapshot: &WorkflowRunDrilldownSnapshot) -> u64 {
    // A monotonic presentation version across both ledger appends and record
    // updates; latestEventSeq remains exposed separately for event de-duping.
    let mut version = snapshot.latest_event_seq;
    for timestamp in std::iter::once(snapshot.workflow.updated_at.as_str())
        .chain(snapshot.workflow.finished_at.as_deref())
        .chain(
            snapshot
                .children
                .iter()
                .map(|child| child.updated_at.as_str()),
        )
        .chain(
            snapshot
                .events
                .iter()
                .map(|event| event.created_at.as_str()),
        )
    {
        version = version.max(timestamp_millis(timestamp));
    }
    version
}

fn timestamp_millis(timestamp: &str) -> u64 {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Utc).timestamp_millis())
        .ok()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0)
}

fn sorted_child_cards(children: &[WorkflowChildRunRecord]) -> Vec<WorkflowChildRunRecord> {
    let mut sorted = children.to_vec();
    sorted.sort_by(|left, right| {
        child_status_rank(&left.status)
            .cmp(&child_status_rank(&right.status))
            .then_with(|| left.phase_index.cmp(&right.phase_index))
            .then_with(|| child_sort_time(left).cmp(&child_sort_time(right)))
            .then_with(|| left.workflow_child_run_id.cmp(&right.workflow_child_run_id))
    });
    sorted
}

fn child_status_rank(status: &str) -> u8 {
    match status {
        "failed" | "cancelled" => 0,
        "running" => 1,
        "queued" => 2,
        "skipped" => 3,
        "succeeded" => 4,
        _ => 5,
    }
}

fn child_sort_time(child: &WorkflowChildRunRecord) -> String {
    child
        .started_at
        .as_deref()
        .or(Some(child.queued_at.as_str()))
        .unwrap_or_default()
        .to_owned()
}

fn phase_id(index: Option<i64>, title: &str) -> String {
    let index = index
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_owned());
    let normalized = title
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        format!("phase-{index}")
    } else {
        format!("phase-{index}-{slug}")
    }
}

fn is_terminal_workflow_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

fn is_terminal_child_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled" | "skipped")
}

fn is_terminal_phase_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled" | "skipped")
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
