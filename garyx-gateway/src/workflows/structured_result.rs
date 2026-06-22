use super::*;
use garyx_router::thread_metadata_from_value;

pub(super) const STRUCTURED_RESULT_SCHEMA_METADATA_KEY: &str = "structured_result_schema";

#[derive(Debug, Clone, PartialEq)]
pub struct StructuredResultSubmission {
    pub workflow_id: String,
    pub workflow_child_run_id: String,
    pub thread_id: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct StructuredResultContext {
    pub workflow_id: String,
    pub workflow_child_run_id: String,
    pub thread_id: String,
    pub schema_json: Value,
}

pub async fn structured_result_context_for_thread(
    state: &Arc<AppState>,
    thread_id: &str,
) -> Result<Option<StructuredResultContext>, WorkflowError> {
    let thread_id = required("thread_id", thread_id)?;
    let Some(thread_data) = state.threads.thread_store.get(&thread_id).await else {
        return Ok(None);
    };
    let metadata = thread_metadata_from_value(&thread_data);
    let Some(schema_json) = metadata.get(STRUCTURED_RESULT_SCHEMA_METADATA_KEY).cloned() else {
        return Ok(None);
    };
    validate_result_tool_schema(&schema_json)?;
    let workflow_id = metadata
        .get("workflow_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            WorkflowError::BadRequest("structured result thread is missing workflow_id".to_owned())
        })
        .and_then(|value| required("workflow_id", value))?;
    let workflow_child_run_id = metadata
        .get("workflow_child_run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            WorkflowError::BadRequest(
                "structured result thread is missing workflow_child_run_id".to_owned(),
            )
        })
        .and_then(|value| required("workflow_child_run_id", value))?;
    Ok(Some(StructuredResultContext {
        workflow_id,
        workflow_child_run_id,
        thread_id,
        schema_json,
    }))
}

pub async fn submit_structured_result_for_thread(
    state: &Arc<AppState>,
    thread_id: &str,
    payload: Value,
) -> Result<StructuredResultSubmission, WorkflowError> {
    let context = structured_result_context_for_thread(state, thread_id)
        .await?
        .ok_or_else(|| {
            WorkflowError::BadRequest(
                "current thread does not accept structured results".to_owned(),
            )
        })?;
    validate_json_size("result", &payload, MAX_RESULT_BYTES)?;
    let payload = normalize_submitted_payload(&context.schema_json, payload);
    validate_payload_against_schema(&context.schema_json, &payload, "$")?;
    let preview = summarize(payload.as_str().unwrap_or(""), 240);
    let preview = if preview.is_empty() {
        Some(summarize(&payload.to_string(), 240))
    } else {
        Some(preview)
    };
    let updated = state.ops.garyx_db.submit_workflow_child_result(
        &context.workflow_id,
        &context.workflow_child_run_id,
        &context.thread_id,
        &payload.to_string(),
        preview.as_deref(),
    )?;
    if !updated {
        return Err(WorkflowError::Conflict(
            "structured result target is already terminal or already has a submitted result"
                .to_owned(),
        ));
    }
    Ok(StructuredResultSubmission {
        workflow_id: context.workflow_id,
        workflow_child_run_id: context.workflow_child_run_id,
        thread_id: context.thread_id,
        payload,
    })
}

pub(super) fn validate_result_tool_schema(schema: &Value) -> Result<(), WorkflowError> {
    validate_schema_shape(schema, 0)?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(WorkflowError::BadRequest(
            "structured result schema must be an object so it can be exposed as a tool input schema"
                .to_owned(),
        ));
    }
    Ok(())
}
