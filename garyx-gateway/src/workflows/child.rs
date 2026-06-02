use std::collections::HashMap;

use serde_json::Value;

use super::runtime::WorkflowAgentCall;
use super::structured_result::STRUCTURED_RESULT_SCHEMA_METADATA_KEY;

pub(super) fn workflow_child_metadata(
    workflow_id: &str,
    workflow_child_run_id: &str,
    parent_thread_id: &str,
    label: &str,
    phase_index: i64,
    schema_json: Option<&Value>,
) -> HashMap<String, Value> {
    let mut metadata = HashMap::from([
        ("source".to_owned(), Value::String("workflow".to_owned())),
        (
            "workflow_id".to_owned(),
            Value::String(workflow_id.to_owned()),
        ),
        (
            "workflow_child_run_id".to_owned(),
            Value::String(workflow_child_run_id.to_owned()),
        ),
        (
            "workflow_parent_thread_id".to_owned(),
            Value::String(parent_thread_id.to_owned()),
        ),
        (
            "workflow_phase_index".to_owned(),
            Value::Number(serde_json::Number::from(phase_index)),
        ),
        ("workflow_label".to_owned(), Value::String(label.to_owned())),
        ("exclude_from_recent".to_owned(), Value::Bool(true)),
    ]);
    if let Some(schema_json) = schema_json {
        metadata.insert(
            STRUCTURED_RESULT_SCHEMA_METADATA_KEY.to_owned(),
            schema_json.clone(),
        );
    }
    metadata
}

pub(super) fn workflow_child_prompt(call: &WorkflowAgentCall) -> String {
    if let Some(schema) = &call.schema_json {
        format!(
            "{}\n\nThis run requires a structured result. Call `submit_result` exactly once when complete. Pass the fields directly as the tool arguments; do not wrap them in `payload` and do not return a JSON string. The arguments must match this schema. After the tool call, do not continue working.\n{}",
            call.prompt, schema
        )
    } else {
        call.prompt.clone()
    }
}
