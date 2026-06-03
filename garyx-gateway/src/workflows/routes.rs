use super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSdkStartRequest {
    #[serde(default, alias = "workflow_run_id")]
    pub workflow_run_id: Option<String>,
    #[serde(default, alias = "workflow_id")]
    pub workflow_id: Option<String>,
    #[serde(default, alias = "task_id")]
    pub task_id: Option<String>,
    #[serde(default, alias = "task_thread_id")]
    pub task_thread_id: Option<String>,
    #[serde(default, alias = "workflow_definition_id")]
    pub workflow_definition_id: Option<String>,
    #[serde(default, alias = "workflow_definition_version")]
    pub workflow_definition_version: Option<u64>,
    #[serde(default, alias = "workflow_definition_snapshot")]
    pub workflow_definition_snapshot: Option<Value>,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default, alias = "parent_thread_id")]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub parent_run_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub phases: Vec<WorkflowSdkPhaseDefinition>,
    #[serde(default, alias = "workspace_dir")]
    pub workspace_dir: Option<String>,
    #[serde(default, alias = "created_by")]
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSdkPhaseDefinition {
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub index: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowDefinitionListQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinitionStartRequest {
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default, alias = "workspace_dir")]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "created_by")]
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSdkAgentRequest {
    pub prompt: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub binding: Option<String>,
    #[serde(default)]
    pub order_index: Option<usize>,
    #[serde(default)]
    pub phase_index: Option<i64>,
    #[serde(default)]
    pub phase_title: Option<String>,
    #[serde(default, alias = "agent_id")]
    pub agent_id: Option<String>,
    #[serde(default, alias = "workspace_dir")]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub schema: Option<Value>,
    #[serde(default)]
    pub optional: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSdkEventRequest {
    #[serde(default, alias = "event_type")]
    pub event_type: Option<String>,
    #[serde(default, alias = "workflow_child_run_id")]
    pub workflow_child_run_id: Option<String>,
    #[serde(default, alias = "thread_id")]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSdkFinishRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowListQuery {
    #[serde(default, alias = "thread")]
    pub parent_thread_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEventsQuery {
    #[serde(default, alias = "after_event_seq")]
    pub after: u64,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

pub async fn list_workflow_definitions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WorkflowDefinitionListQuery>,
) -> impl IntoResponse {
    match list_workflow_definition_packages(&state.config_snapshot()) {
        Ok(mut packages) => {
            packages.sort_by(|left, right| {
                right
                    .record
                    .updated_at
                    .cmp(&left.record.updated_at)
                    .then_with(|| left.record.workflow_id.cmp(&right.record.workflow_id))
            });
            let offset = query.offset.unwrap_or(0);
            let limit = query.limit.unwrap_or(100).min(200);
            let records = packages
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            (
            StatusCode::OK,
            Json(json!({
                "workflowDefinitions": records.iter().map(workflow_definition_package_json).collect::<Vec<_>>(),
            })),
        )
                .into_response()
        }
        Err(error) => workflow_error_response(error),
    }
}

pub async fn get_workflow_definition(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<String>,
) -> impl IntoResponse {
    match get_workflow_definition_package(&state.config_snapshot(), &workflow_id) {
        Ok(package) => (
            StatusCode::OK,
            Json(json!({
                "workflowDefinition": workflow_definition_package_json(&package),
            })),
        )
            .into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn get_workflow_definition_source(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<String>,
) -> impl IntoResponse {
    match get_workflow_definition_package(&state.config_snapshot(), &workflow_id)
        .and_then(|package| workflow_definition_source(&package).map(|source| (package, source)))
    {
        Ok((package, source)) => (
            StatusCode::OK,
            Json(json!({
                "workflowId": package.record.workflow_id,
                "path": source.relative_path,
                "content": source.content,
                "mediaType": source.media_type,
                "language": source.language,
            })),
        )
            .into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn start_sdk_workflow(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<WorkflowSdkStartRequest>,
) -> impl IntoResponse {
    match WorkflowRuntime::new(state).start_sdk(payload).await {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn start_workflow_definition(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<String>,
    Json(payload): Json<WorkflowDefinitionStartRequest>,
) -> impl IntoResponse {
    match WorkflowRuntime::new(state)
        .start_definition(&workflow_id, payload)
        .await
    {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn get_workflow(
    State(state): State<Arc<AppState>>,
    Path(workflow_run_id): Path<String>,
) -> impl IntoResponse {
    let store = WorkflowStore::new(state.ops.garyx_db.clone());
    match workflow_payload(&store, &workflow_run_id) {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn list_workflows(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WorkflowListQuery>,
) -> impl IntoResponse {
    let store = WorkflowStore::new(state.ops.garyx_db.clone());
    match store.list_runs(
        query.parent_thread_id.as_deref(),
        query.limit.min(200),
        query.offset,
    ) {
        Ok(records) => (
            StatusCode::OK,
            Json(json!({
                "workflows": records.iter().map(workflow_run_json).collect::<Vec<_>>(),
                "count": records.len(),
            })),
        )
            .into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn list_thread_workflows(
    State(state): State<Arc<AppState>>,
    Path(thread_id): Path<String>,
    Query(query): Query<WorkflowListQuery>,
) -> impl IntoResponse {
    let store = WorkflowStore::new(state.ops.garyx_db.clone());
    match store.list_runs(Some(&thread_id), query.limit.min(200), query.offset) {
        Ok(records) => (
            StatusCode::OK,
            Json(json!({
                "threadId": thread_id,
                "workflows": records.iter().map(workflow_run_json).collect::<Vec<_>>(),
                "count": records.len(),
            })),
        )
            .into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn list_task_workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    Query(query): Query<WorkflowListQuery>,
) -> impl IntoResponse {
    let limit = query.limit.min(200);
    let store = WorkflowStore::new(state.ops.garyx_db.clone());
    match store.list_runs_for_task(&task_id, limit.saturating_add(1), query.offset) {
        Ok(mut records) => {
            let has_more = records.len() > limit;
            records.truncate(limit);
            let mut workflow_runs = Vec::with_capacity(records.len());
            for record in records {
                match workflow_run_drilldown_json(&store, &record) {
                    Ok(value) => workflow_runs.push(value),
                    Err(error) => return workflow_error_response(error),
                }
            }
            (
                StatusCode::OK,
                Json(json!({
                    "taskId": task_id,
                    "workflowRuns": workflow_runs,
                    "count": workflow_runs.len(),
                    "hasMore": has_more,
                })),
            )
                .into_response()
        }
        Err(error) => workflow_error_response(error),
    }
}

pub async fn workflow_events(
    State(state): State<Arc<AppState>>,
    Path(workflow_run_id): Path<String>,
    Query(query): Query<WorkflowEventsQuery>,
) -> impl IntoResponse {
    let store = WorkflowStore::new(state.ops.garyx_db.clone());
    match store.events_after(&workflow_run_id, query.after, query.limit.min(500)) {
        Ok(events) => (
            StatusCode::OK,
            Json(json!({
                "workflowRunId": workflow_run_id,
                "workflowId": workflow_run_id,
                "events": events.iter().map(workflow_event_json).collect::<Vec<_>>(),
                "count": events.len(),
                "nextAfter": events.last().map(|event| event.event_seq).unwrap_or(query.after),
            })),
        )
            .into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn append_workflow_event(
    State(state): State<Arc<AppState>>,
    Path(workflow_run_id): Path<String>,
    Json(payload): Json<WorkflowSdkEventRequest>,
) -> impl IntoResponse {
    match WorkflowRuntime::new(state).append_sdk_event(&workflow_run_id, payload) {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn run_workflow_agent(
    State(state): State<Arc<AppState>>,
    Path(workflow_run_id): Path<String>,
    Json(payload): Json<WorkflowSdkAgentRequest>,
) -> impl IntoResponse {
    match WorkflowRuntime::new(state)
        .run_sdk_agent(workflow_run_id, payload)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn finish_sdk_workflow(
    State(state): State<Arc<AppState>>,
    Path(workflow_run_id): Path<String>,
    Json(payload): Json<WorkflowSdkFinishRequest>,
) -> impl IntoResponse {
    match WorkflowRuntime::new(state)
        .finish_sdk(&workflow_run_id, payload)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => workflow_error_response(error),
    }
}

pub async fn cancel_workflow(
    State(state): State<Arc<AppState>>,
    Path(workflow_run_id): Path<String>,
) -> impl IntoResponse {
    let store = WorkflowStore::new(state.ops.garyx_db.clone());
    match cancel_workflow_run(&state, &workflow_run_id).await {
        Ok(true) => match workflow_payload(&store, &workflow_run_id) {
            Ok(value) => (StatusCode::OK, Json(value)).into_response(),
            Err(error) => workflow_error_response(error),
        },
        Ok(false) => workflow_error_response(WorkflowError::NotFound(format!(
            "workflow not found: {workflow_run_id}"
        ))),
        Err(error) => workflow_error_response(error),
    }
}
