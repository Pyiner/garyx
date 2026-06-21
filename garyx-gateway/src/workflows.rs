use std::fs;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::SystemTime;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{SecondsFormat, Utc};
use garyx_bridge::provider_trait::BridgeError;
use garyx_models::local_paths::default_session_data_dir;
use garyx_models::{ProviderType, TaskExecutor, config::GaryxConfig};
use garyx_router::{
    ThreadEnsureOptions, WorkspaceMode, create_thread_record, tasks::task_from_record,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::garyx_db::{
    GaryxDbError, GaryxDbService, WorkflowChildRunDraft, WorkflowChildRunUsage, WorkflowEventDraft,
    WorkflowRunDraft,
};
use crate::server::AppState;

const DEFAULT_WORKFLOW_NAME: &str = "Workflow";
const DEFAULT_PHASE_TITLE: &str = "Run";
const DEFAULT_CHILD_LABEL: &str = "agent";
const DEFAULT_GLOBAL_CHILD_LIMIT: usize = 16;
const DEFAULT_PER_WORKFLOW_CHILD_LIMIT: usize = 16;
const MAX_SCHEMA_DEPTH: usize = 12;
const MAX_SCHEMA_BYTES: usize = 32 * 1024;
const MAX_RESULT_BYTES: usize = 256 * 1024;
const MAX_WORKFLOW_SOURCE_BYTES: u64 = 512 * 1024;
const WORKFLOW_MANIFEST_FILE: &str = "garyx.workflow.json";
const WORKFLOW_ENTRYPOINT_FILE: &str = "workflow.ts";

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error(transparent)]
    Db(#[from] GaryxDbError),
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("NotFound: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Bridge(#[from] BridgeError),
}

impl WorkflowError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Db(GaryxDbError::BadRequest(_)) => StatusCode::BAD_REQUEST,
            Self::Db(_) | Self::Bridge(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

fn workflow_error_response(error: WorkflowError) -> axum::response::Response {
    let status = error.status_code();
    (
        status,
        Json(json!({
            "error": match status {
                StatusCode::BAD_REQUEST => "BadRequest",
                StatusCode::NOT_FOUND => "NotFound",
                StatusCode::CONFLICT => "Conflict",
                _ => "InternalError",
            },
            "message": error.to_string(),
        })),
    )
        .into_response()
}

fn provider_supports_workflow_structured_results(provider_type: &ProviderType) -> bool {
    matches!(
        provider_type,
        ProviderType::ClaudeCode
            | ProviderType::CodexAppServer
            | ProviderType::Traex
            | ProviderType::GeminiCli
    )
}

mod child;
mod definitions;
mod entrypoint;
mod lifecycle;
mod presenter;
mod routes;
mod runtime;
mod scheduler;
mod schema;
mod store;
mod structured_result;

pub use definitions::{get_workflow_definition_package, workflow_definitions_root_for_config};
pub use entrypoint::{spawn_workflow_task_entrypoint, spawn_workflow_thread_entrypoint};
pub use lifecycle::{cancel_workflow_run, reconcile_interrupted_workflows};
pub use routes::{
    WorkflowDefinitionListQuery, WorkflowDefinitionStartRequest, WorkflowEventsQuery,
    WorkflowListQuery, WorkflowSdkAgentRequest, WorkflowSdkEventRequest, WorkflowSdkFinishRequest,
    WorkflowSdkPhaseDefinition, WorkflowSdkStartRequest, append_workflow_event, cancel_workflow,
    finish_sdk_workflow, get_workflow, get_workflow_definition, get_workflow_definition_source,
    list_thread_workflows, list_workflow_definitions, list_workflows, run_workflow_agent,
    start_sdk_workflow, start_workflow_definition, workflow_events,
};
pub use runtime::WorkflowRuntime;
pub use scheduler::{WorkflowChildPermit, WorkflowScheduler};
pub use store::{WorkflowDefinitionPackage, WorkflowDefinitionRecord, WorkflowStore};
pub use structured_result::{
    StructuredResultContext, StructuredResultSubmission, structured_result_context_for_thread,
    submit_structured_result_for_thread,
};

use child::{workflow_child_metadata, workflow_child_prompt};
use definitions::{
    list_workflow_definition_packages, workflow_definition_source, workflow_io_error,
};
use entrypoint::workflow_workspace_dir_for_entrypoint;
use lifecycle::mark_workflow_task_in_review;
use presenter::{
    parse_json_field, workflow_agent_result_json, workflow_definition_package_json,
    workflow_event_json, workflow_payload, workflow_run_json,
};
use schema::{
    normalize_submitted_payload, validate_json_size, validate_payload_against_schema,
    validate_schema_shape,
};
use structured_result::validate_result_tool_schema;

fn json_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| normalized_optional_string(Some(value)))
}

fn normalized_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required(field: &str, value: &str) -> Result<String, WorkflowError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(WorkflowError::BadRequest(format!(
            "{field} must not be empty"
        )));
    }
    Ok(value.to_owned())
}

fn summarize(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    trimmed.chars().take(max_chars).collect::<String>()
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn default_limit() -> usize {
    50
}

#[cfg(test)]
mod tests;
