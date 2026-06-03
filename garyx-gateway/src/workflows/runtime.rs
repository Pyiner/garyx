use super::*;

#[derive(Debug, Clone)]
pub(super) struct WorkflowAgentCall {
    pub(super) prompt: String,
    pub(super) label: String,
    pub(super) binding: Option<String>,
    pub(super) order_index: usize,
    pub(super) phase_index: i64,
    pub(super) phase_title: String,
    pub(super) agent_id: Option<String>,
    pub(super) workspace_dir: Option<String>,
    pub(super) schema_json: Option<Value>,
    pub(super) optional: bool,
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowAgentExecutionResult {
    pub(super) workflow_child_run_id: String,
    pub(super) thread_id: String,
    pub(super) label: String,
    pub(super) binding: Option<String>,
    pub(super) order_index: usize,
    pub(super) phase_title: String,
    pub(super) result: Option<Value>,
    pub(super) preview: Option<String>,
    pub(super) failed: bool,
    pub(super) optional: bool,
    pub(super) error: Option<String>,
}

#[derive(Clone)]
pub struct WorkflowRuntime {
    state: Arc<AppState>,
}

struct WorkflowTaskContext {
    task_id: String,
    task_thread_id: String,
    workflow_definition_id: Option<String>,
    workflow_definition_version: Option<u64>,
}

impl WorkflowRuntime {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    pub async fn start_sdk(
        &self,
        request: WorkflowSdkStartRequest,
    ) -> Result<Value, WorkflowError> {
        let name = request
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_WORKFLOW_NAME)
            .to_owned();
        let phases = normalize_phase_plan(&request.phases)?;
        let task_context = match (
            request
                .task_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            request
                .task_thread_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        ) {
            (Some(task_id), Some(task_thread_id)) => Some(
                self.workflow_task_context(
                    task_thread_id,
                    task_id,
                    request.workflow_definition_id.as_deref(),
                )
                .await?,
            ),
            (None, None) => None,
            _ => {
                return Err(WorkflowError::BadRequest(
                    "taskId and taskThreadId must be provided together".to_owned(),
                ));
            }
        };
        let (
            workflow_thread_id,
            task_id,
            task_thread_id,
            workflow_definition_id,
            workflow_definition_version,
        ) = if let Some(context) = task_context {
            let workflow_thread_id = context.task_thread_id.clone();
            if let Some(requested_run_id) = request
                .workflow_run_id
                .as_deref()
                .or(request.workflow_id.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                && requested_run_id != workflow_thread_id
            {
                return Err(WorkflowError::BadRequest(format!(
                    "workflowRunId must match the workflow thread id {workflow_thread_id}"
                )));
            }
            self.mark_existing_workflow_thread(
                &workflow_thread_id,
                &name,
                context.workflow_definition_id.as_deref(),
                context.workflow_definition_version,
                request.workspace_dir.as_deref(),
                "running",
            )
            .await?;
            (
                workflow_thread_id,
                Some(context.task_id),
                Some(context.task_thread_id),
                context.workflow_definition_id,
                context.workflow_definition_version,
            )
        } else {
            let workflow_thread_id = self
                .create_workflow_thread(
                    &name,
                    request.workflow_definition_id.as_deref(),
                    request.workflow_definition_version,
                    request.workspace_dir.as_deref(),
                )
                .await?;
            (
                workflow_thread_id,
                None,
                None,
                request.workflow_definition_id.clone(),
                request.workflow_definition_version,
            )
        };
        let parent_thread_id = request
            .parent_thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&workflow_thread_id)
            .to_owned();
        let mut meta = json!({
            "name": name,
            "description": request.description.clone(),
            "source": "sdk",
            "sdk": true,
        });
        if !phases.is_empty() {
            meta["phases"] = Value::Array(phases);
        }
        let workflow_definition_snapshot_json = request
            .workflow_definition_snapshot
            .as_ref()
            .map(Value::to_string);
        let input_json = request.input.as_ref().map(Value::to_string);
        let store = WorkflowStore::new(self.state.ops.garyx_db.clone());
        let workflow = store.create_run(WorkflowRunDraft {
            workflow_id: Some(workflow_thread_id.clone()),
            task_id,
            task_thread_id,
            workflow_definition_id,
            workflow_definition_version,
            workflow_definition_snapshot_json,
            input_json,
            parent_thread_id,
            parent_run_id: request.parent_run_id,
            name,
            description: request.description.clone(),
            status: "running".to_owned(),
            current_phase_index: None,
            script_text: "sdk".to_owned(),
            meta_json: meta.to_string(),
            result_json: None,
            summary: None,
            error: None,
            workspace_dir: request.workspace_dir,
            created_by: request.created_by.or_else(|| Some("sdk".to_owned())),
            started_at: Some(now_string()),
            finished_at: None,
        })?;
        let workflow_run_id = workflow.workflow_id.clone();
        store.append_event(WorkflowEventDraft {
            event_id: None,
            workflow_id: workflow_run_id.clone(),
            workflow_child_run_id: None,
            thread_id: None,
            event_type: "workflow.created".to_owned(),
            payload_json: json!({
                "status": workflow.status,
                "source": "sdk",
                "input": workflow.input_json.as_deref().map(parse_json_field),
            })
            .to_string(),
        })?;
        workflow_payload(&store, &workflow_run_id)
    }

    async fn workflow_task_context(
        &self,
        task_thread_id: &str,
        task_id: &str,
        requested_workflow_id: Option<&str>,
    ) -> Result<WorkflowTaskContext, WorkflowError> {
        let record = self
            .state
            .threads
            .thread_store
            .get(task_thread_id)
            .await
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("task thread not found: {task_thread_id}"))
            })?;
        let task = task_from_record(&record)
            .map_err(|error| WorkflowError::BadRequest(error.to_string()))?
            .ok_or_else(|| {
                WorkflowError::BadRequest("taskThreadId must reference a Task thread".to_owned())
            })?;
        let canonical = garyx_router::tasks::canonical_task_id(&task);
        if canonical != task_id {
            return Err(WorkflowError::BadRequest(format!(
                "taskId {task_id} does not match taskThreadId {task_thread_id}"
            )));
        }
        let Some(TaskExecutor::Workflow {
            workflow_id,
            workflow_version,
        }) = task.executor
        else {
            return Err(WorkflowError::BadRequest(
                "taskThreadId must reference a workflow-backed Task".to_owned(),
            ));
        };
        if let Some(requested) = requested_workflow_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && requested != workflow_id
        {
            return Err(WorkflowError::BadRequest(format!(
                "workflowDefinitionId {requested} does not match Task executor {workflow_id}"
            )));
        }
        Ok(WorkflowTaskContext {
            task_id: canonical,
            task_thread_id: task_thread_id.to_owned(),
            workflow_definition_id: Some(workflow_id),
            workflow_definition_version: workflow_version,
        })
    }

    async fn create_workflow_thread(
        &self,
        name: &str,
        workflow_definition_id: Option<&str>,
        workflow_definition_version: Option<u64>,
        workspace_dir: Option<&str>,
    ) -> Result<String, WorkflowError> {
        let metadata = workflow_thread_metadata(
            None,
            workflow_definition_id,
            workflow_definition_version,
            "running",
        );
        let (thread_id, _) = create_thread_record(
            &self.state.threads.thread_store,
            ThreadEnsureOptions {
                label: Some(name.to_owned()),
                workspace_dir: workspace_dir.map(ToOwned::to_owned),
                workspace_mode: WorkspaceMode::default(),
                metadata,
                thread_kind: Some("workflow_run".to_owned()),
                ..ThreadEnsureOptions::default()
            },
        )
        .await
        .map_err(WorkflowError::BadRequest)?;
        self.mark_existing_workflow_thread(
            &thread_id,
            name,
            workflow_definition_id,
            workflow_definition_version,
            workspace_dir,
            "running",
        )
        .await?;
        Ok(thread_id)
    }

    async fn mark_existing_workflow_thread(
        &self,
        thread_id: &str,
        name: &str,
        workflow_definition_id: Option<&str>,
        workflow_definition_version: Option<u64>,
        workspace_dir: Option<&str>,
        status: &str,
    ) -> Result<(), WorkflowError> {
        let mut record = self
            .state
            .threads
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("workflow thread not found: {thread_id}"))
            })?;
        let metadata = workflow_thread_metadata(
            Some(thread_id),
            workflow_definition_id,
            workflow_definition_version,
            status,
        );
        {
            let obj = record.as_object_mut().ok_or_else(|| {
                WorkflowError::BadRequest("workflow thread record is not an object".to_owned())
            })?;
            obj.insert(
                "thread_kind".to_owned(),
                Value::String("workflow_run".to_owned()),
            );
            obj.insert(
                "workflow_run_id".to_owned(),
                Value::String(thread_id.to_owned()),
            );
            obj.insert(
                "workflow_status".to_owned(),
                Value::String(status.to_owned()),
            );
            if let Some(workflow_definition_id) = workflow_definition_id {
                obj.insert(
                    "workflow_definition_id".to_owned(),
                    Value::String(workflow_definition_id.to_owned()),
                );
            }
            if let Some(workflow_definition_version) = workflow_definition_version {
                obj.insert(
                    "workflow_definition_version".to_owned(),
                    Value::Number(serde_json::Number::from(workflow_definition_version)),
                );
            }
            if let Some(workspace_dir) = normalized_optional_string(workspace_dir) {
                obj.insert("workspace_dir".to_owned(), Value::String(workspace_dir));
            }
            if obj
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty)
            {
                obj.insert("label".to_owned(), Value::String(name.to_owned()));
            }
            let metadata_value = obj
                .entry("metadata".to_owned())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !metadata_value.is_object() {
                *metadata_value = Value::Object(serde_json::Map::new());
            }
            if let Some(metadata_obj) = metadata_value.as_object_mut() {
                for (key, value) in metadata {
                    metadata_obj.insert(key, value);
                }
            }
            obj.insert("updated_at".to_owned(), Value::String(now_string()));
        }
        self.state.threads.thread_store.set(thread_id, record).await;
        self.state.invalidate_gateway_sync_caches().await;
        Ok(())
    }

    pub async fn run_sdk_agent(
        &self,
        workflow_run_id: String,
        request: WorkflowSdkAgentRequest,
    ) -> Result<Value, WorkflowError> {
        let store = WorkflowStore::new(self.state.ops.garyx_db.clone());
        let workflow = store.get_run(&workflow_run_id)?;
        if matches!(
            workflow.status.as_str(),
            "succeeded" | "failed" | "cancelled"
        ) {
            return Err(WorkflowError::Conflict(format!(
                "workflow is already terminal: {}",
                workflow.status
            )));
        }
        let prompt = required("prompt", &request.prompt)?;
        let label = request
            .label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_CHILD_LABEL)
            .to_owned();
        let phase_title = request
            .phase_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_PHASE_TITLE)
            .to_owned();
        let call = WorkflowAgentCall {
            prompt,
            label,
            binding: request.binding,
            order_index: request.order_index.unwrap_or(0),
            phase_index: request.phase_index.unwrap_or(0),
            phase_title,
            agent_id: request.agent_id,
            workspace_dir: request.workspace_dir.or(workflow.workspace_dir),
            schema_json: request.schema,
            optional: request.optional.unwrap_or(false),
        };
        let result = self.run_agent_call(workflow_run_id, call).await?;
        Ok(workflow_agent_result_json(&result))
    }

    pub async fn finish_sdk(
        &self,
        workflow_run_id: &str,
        request: WorkflowSdkFinishRequest,
    ) -> Result<Value, WorkflowError> {
        let status = request.status.as_deref().unwrap_or("succeeded");
        if !matches!(status, "succeeded" | "failed" | "cancelled") {
            return Err(WorkflowError::BadRequest(
                "status must be succeeded, failed, or cancelled".to_owned(),
            ));
        }
        let store = WorkflowStore::new(self.state.ops.garyx_db.clone());
        let existing = store.get_run(workflow_run_id)?;
        let result_json = request.result.as_ref().map(Value::to_string);
        let summary = request.summary;
        let updated = self.state.ops.garyx_db.update_workflow_run_status(
            workflow_run_id,
            status,
            result_json.as_deref(),
            summary.as_deref(),
            request.error.as_deref(),
        )?;
        if !updated {
            return Err(WorkflowError::Conflict(
                "workflow is already terminal".to_owned(),
            ));
        }
        if self
            .state
            .threads
            .thread_store
            .get(workflow_run_id)
            .await
            .is_some()
        {
            self.mark_existing_workflow_thread(
                workflow_run_id,
                &existing.name,
                existing.workflow_definition_id.as_deref(),
                existing.workflow_definition_version,
                existing.workspace_dir.as_deref(),
                status,
            )
            .await?;
        }
        let event_type = match status {
            "succeeded" => "workflow.completed",
            "failed" => "workflow.failed",
            _ => "workflow.cancelled",
        };
        store.append_event(WorkflowEventDraft {
            event_id: None,
            workflow_id: workflow_run_id.to_owned(),
            workflow_child_run_id: None,
            thread_id: None,
            event_type: event_type.to_owned(),
            payload_json: json!({
                "summary": summary,
                "error": request.error,
                "source": "sdk",
            })
            .to_string(),
        })?;
        if let Some(task_thread_id) = existing.task_thread_id.as_deref() {
            mark_workflow_task_in_review(
                &self.state,
                task_thread_id,
                format!("workflow {status}: {workflow_run_id}"),
            )
            .await?;
        }
        workflow_payload(&store, workflow_run_id)
    }

    pub fn append_sdk_event(
        &self,
        workflow_run_id: &str,
        request: WorkflowSdkEventRequest,
    ) -> Result<Value, WorkflowError> {
        let store = WorkflowStore::new(self.state.ops.garyx_db.clone());
        store.get_run(workflow_run_id)?;
        let event_type = request
            .event_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("workflow.log")
            .to_owned();
        let payload = request.payload.unwrap_or(Value::Null);
        let event = store.append_event(WorkflowEventDraft {
            event_id: None,
            workflow_id: workflow_run_id.to_owned(),
            workflow_child_run_id: request.workflow_child_run_id,
            thread_id: request.thread_id,
            event_type,
            payload_json: payload.to_string(),
        })?;
        Ok(workflow_event_json(&event))
    }

    async fn run_agent_call(
        &self,
        workflow_run_id: String,
        call: WorkflowAgentCall,
    ) -> Result<WorkflowAgentExecutionResult, WorkflowError> {
        if self.is_workflow_cancelled(&workflow_run_id)? {
            return Ok(WorkflowAgentExecutionResult {
                workflow_child_run_id: String::new(),
                thread_id: String::new(),
                label: call.label,
                binding: call.binding,
                order_index: call.order_index,
                phase_title: call.phase_title,
                result: None,
                preview: None,
                failed: true,
                optional: call.optional,
                error: Some("workflow cancelled".to_owned()),
            });
        }
        let _permit = self
            .state
            .ops
            .workflow_scheduler
            .acquire_child_permit(&workflow_run_id)
            .await?;
        if self.is_workflow_cancelled(&workflow_run_id)? {
            return Ok(WorkflowAgentExecutionResult {
                workflow_child_run_id: String::new(),
                thread_id: String::new(),
                label: call.label,
                binding: call.binding,
                order_index: call.order_index,
                phase_title: call.phase_title,
                result: None,
                preview: None,
                failed: true,
                optional: call.optional,
                error: Some("workflow cancelled".to_owned()),
            });
        }
        let child_run_id = format!("workflow-child::{}", Uuid::new_v4());
        let thread_id = self
            .create_child_thread(
                &workflow_run_id,
                &child_run_id,
                &call.label,
                call.agent_id.as_deref(),
                call.workspace_dir.as_deref(),
                call.phase_index,
                call.schema_json.as_ref(),
            )
            .await?;
        let db = &self.state.ops.garyx_db;
        let result_mode = if call.schema_json.is_some() {
            "structured"
        } else {
            "text"
        };
        db.upsert_workflow_child_run(WorkflowChildRunDraft {
            workflow_id: workflow_run_id.clone(),
            workflow_child_run_id: Some(child_run_id.clone()),
            thread_id: thread_id.clone(),
            phase_index: call.phase_index,
            phase_title: call.phase_title.clone(),
            label: call.label.clone(),
            agent_id: call.agent_id.clone(),
            status: "running".to_owned(),
            prompt: call.prompt.clone(),
            result_mode: result_mode.to_owned(),
            schema_json: call.schema_json.as_ref().map(Value::to_string),
            result_text: None,
            result_json: None,
            result_preview: None,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: 0,
            cost_usd: 0.0,
            started_at: Some(now_string()),
            finished_at: None,
        })?;
        if let Some(schema) = &call.schema_json {
            if let Some(provider_type) = self
                .workflow_child_provider_type(call.agent_id.as_deref())
                .await?
                && !provider_supports_workflow_structured_results(&provider_type)
            {
                let error = format!(
                    "structured workflow child {} uses provider {}, which does not support submit_result",
                    call.label,
                    provider_type.as_slug()
                );
                return finish_failed_workflow_child(
                    db,
                    &workflow_run_id,
                    &child_run_id,
                    &thread_id,
                    call,
                    error,
                    None,
                );
            }
            validate_result_tool_schema(schema)?;
        }
        db.append_workflow_event(WorkflowEventDraft {
            event_id: None,
            workflow_id: workflow_run_id.clone(),
            workflow_child_run_id: Some(child_run_id.clone()),
            thread_id: Some(thread_id.clone()),
            event_type: "workflow.child_started".to_owned(),
            payload_json: json!({
                "label": call.label,
                "phaseIndex": call.phase_index,
                "phaseTitle": call.phase_title,
                "resultMode": result_mode,
            })
            .to_string(),
        })?;

        let parent_thread_id = db
            .get_workflow_run(&workflow_run_id)?
            .map(|workflow| workflow.parent_thread_id)
            .unwrap_or_else(|| workflow_run_id.clone());
        let mut metadata = workflow_child_metadata(
            &workflow_run_id,
            &child_run_id,
            &parent_thread_id,
            &call.label,
            call.phase_index,
            call.schema_json.as_ref(),
        );
        let gateway_token = self
            .state
            .config_snapshot()
            .gateway
            .auth_token
            .trim()
            .to_owned();
        if !gateway_token.is_empty() {
            metadata.insert(
                "garyx_mcp_auth_token".to_owned(),
                Value::String(gateway_token),
            );
        }
        let child_prompt = workflow_child_prompt(&call);
        let result = match self
            .state
            .integration
            .bridge
            .run_subagent_streaming(
                &thread_id,
                &child_prompt,
                metadata,
                None,
                call.workspace_dir.clone(),
                None,
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                return finish_failed_workflow_child(
                    db,
                    &workflow_run_id,
                    &child_run_id,
                    &thread_id,
                    call,
                    error.to_string(),
                    None,
                );
            }
        };

        let usage = provider_result_usage(&result);
        if self.is_workflow_cancelled(&workflow_run_id)? {
            db.finish_workflow_child_run(
                &workflow_run_id,
                &child_run_id,
                "cancelled",
                None,
                None,
                None,
                Some("workflow cancelled"),
                Some(usage),
            )?;
            return Ok(WorkflowAgentExecutionResult {
                workflow_child_run_id: child_run_id,
                thread_id,
                label: call.label,
                binding: call.binding,
                order_index: call.order_index,
                phase_title: call.phase_title,
                result: None,
                preview: None,
                failed: true,
                optional: call.optional,
                error: Some("workflow cancelled".to_owned()),
            });
        }

        if result.success {
            let child_result = if let Some(schema) = &call.schema_json {
                let payload_result = db
                    .get_workflow_child_run(&workflow_run_id, &child_run_id)?
                    .and_then(|child| child.result_json)
                    .ok_or_else(|| {
                        WorkflowError::BadRequest(format!(
                            "structured workflow child {} finished without submit_result",
                            call.label
                        ))
                    })
                    .and_then(|raw| {
                        let payload = serde_json::from_str::<Value>(&raw).map_err(|error| {
                            WorkflowError::BadRequest(format!(
                                "structured workflow child {} submitted invalid JSON: {error}",
                                call.label
                            ))
                        })?;
                        validate_payload_against_schema(schema, &payload, "$")?;
                        Ok(payload)
                    });
                match payload_result {
                    Ok(payload) => payload,
                    Err(error) => {
                        return finish_failed_workflow_child(
                            db,
                            &workflow_run_id,
                            &child_run_id,
                            &thread_id,
                            call,
                            error.to_string(),
                            Some(usage),
                        );
                    }
                }
            } else {
                Value::String(result.response.clone())
            };
            let preview_source = child_result
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| child_result.to_string());
            let preview = summarize(preview_source.trim(), 240);
            let structured_result_json =
                call.schema_json.is_some().then(|| child_result.to_string());
            db.finish_workflow_child_run(
                &workflow_run_id,
                &child_run_id,
                "succeeded",
                if call.schema_json.is_some() {
                    None
                } else {
                    Some(&result.response)
                },
                structured_result_json.as_deref(),
                Some(&preview),
                None,
                Some(usage),
            )?;
            db.append_workflow_event(WorkflowEventDraft {
                event_id: None,
                workflow_id: workflow_run_id.clone(),
                workflow_child_run_id: Some(child_run_id.clone()),
                thread_id: Some(thread_id.clone()),
                event_type: "workflow.child_succeeded".to_owned(),
                payload_json: json!({"preview": preview, "resultMode": result_mode}).to_string(),
            })?;
            Ok(WorkflowAgentExecutionResult {
                workflow_child_run_id: child_run_id,
                thread_id,
                label: call.label,
                binding: call.binding,
                order_index: call.order_index,
                phase_title: call.phase_title,
                result: Some(child_result),
                preview: Some(preview),
                failed: false,
                optional: call.optional,
                error: None,
            })
        } else {
            let usage = provider_result_usage(&result);
            let error = result
                .error
                .unwrap_or_else(|| "child run failed".to_owned());
            db.finish_workflow_child_run(
                &workflow_run_id,
                &child_run_id,
                "failed",
                None,
                None,
                None,
                Some(&error),
                Some(usage),
            )?;
            db.append_workflow_event(WorkflowEventDraft {
                event_id: None,
                workflow_id: workflow_run_id,
                workflow_child_run_id: Some(child_run_id.clone()),
                thread_id: Some(thread_id.clone()),
                event_type: "workflow.child_failed".to_owned(),
                payload_json: json!({"error": error}).to_string(),
            })?;
            Ok(WorkflowAgentExecutionResult {
                workflow_child_run_id: child_run_id,
                thread_id,
                label: call.label,
                binding: call.binding,
                order_index: call.order_index,
                phase_title: call.phase_title,
                result: None,
                preview: None,
                failed: true,
                optional: call.optional,
                error: Some(error),
            })
        }
    }

    pub async fn create_child_thread(
        &self,
        workflow_run_id: &str,
        workflow_child_run_id: &str,
        label: &str,
        agent_id: Option<&str>,
        workspace_dir: Option<&str>,
        phase_index: i64,
        schema_json: Option<&Value>,
    ) -> Result<String, WorkflowError> {
        let provider_type = match agent_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(agent_id) => {
                let profile = self
                    .state
                    .ops
                    .custom_agents
                    .get_agent(agent_id)
                    .await
                    .ok_or_else(|| {
                        WorkflowError::BadRequest(format!("unknown agent_id: {agent_id}"))
                    })?;
                Some(profile.provider_type)
            }
            None => None,
        };
        let options = ThreadEnsureOptions {
            label: Some(format!("Workflow: {label}")),
            workspace_dir: workspace_dir.map(ToOwned::to_owned),
            workspace_mode: WorkspaceMode::default(),
            agent_id: agent_id.map(ToOwned::to_owned),
            provider_type,
            metadata: workflow_child_metadata(
                workflow_run_id,
                workflow_child_run_id,
                &self
                    .state
                    .ops
                    .garyx_db
                    .get_workflow_run(workflow_run_id)?
                    .map(|workflow| workflow.parent_thread_id)
                    .unwrap_or_else(|| workflow_run_id.to_owned()),
                label,
                phase_index,
                schema_json,
            ),
            ..ThreadEnsureOptions::default()
        };
        let (thread_id, _) = create_thread_record(&self.state.threads.thread_store, options)
            .await
            .map_err(WorkflowError::BadRequest)?;
        self.state.invalidate_thread_list_cache().await;
        Ok(thread_id)
    }

    fn is_workflow_cancelled(&self, workflow_run_id: &str) -> Result<bool, WorkflowError> {
        Ok(self
            .state
            .ops
            .garyx_db
            .get_workflow_run(workflow_run_id)?
            .is_some_and(|record| record.status == "cancelled"))
    }

    async fn workflow_child_provider_type(
        &self,
        agent_id: Option<&str>,
    ) -> Result<Option<ProviderType>, WorkflowError> {
        let Some(agent_id) = agent_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let profile = self
            .state
            .ops
            .custom_agents
            .get_agent(agent_id)
            .await
            .ok_or_else(|| WorkflowError::BadRequest(format!("unknown agent_id: {agent_id}")))?;
        Ok(Some(profile.provider_type))
    }
}

fn provider_result_usage(
    result: &garyx_models::provider::ProviderRunResult,
) -> WorkflowChildRunUsage {
    WorkflowChildRunUsage {
        input_tokens: result.input_tokens.max(0) as u64,
        output_tokens: result.output_tokens.max(0) as u64,
        tool_calls: 0,
        cost_usd: result.cost.max(0.0),
    }
}

fn normalize_phase_plan(
    phases: &[WorkflowSdkPhaseDefinition],
) -> Result<Vec<Value>, WorkflowError> {
    let mut normalized = Vec::with_capacity(phases.len());
    let mut seen_titles = std::collections::HashSet::new();
    for (fallback_index, phase) in phases.iter().enumerate() {
        let title = phase.title.trim();
        if title.is_empty() {
            return Err(WorkflowError::BadRequest(
                "workflow phase title is required".to_owned(),
            ));
        }
        if !seen_titles.insert(title.to_owned()) {
            return Err(WorkflowError::BadRequest(format!(
                "duplicate workflow phase title: {title}"
            )));
        }
        let index = phase.index.unwrap_or(fallback_index as i64);
        if index < 0 {
            return Err(WorkflowError::BadRequest(
                "workflow phase index must be non-negative".to_owned(),
            ));
        }

        let mut value = json!({
            "title": title,
            "index": index,
        });
        if let Some(id) = phase
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            value["id"] = Value::String(id.to_owned());
        }
        if let Some(detail) = phase
            .detail
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            value["detail"] = Value::String(detail.to_owned());
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn finish_failed_workflow_child(
    db: &GaryxDbService,
    workflow_run_id: &str,
    child_run_id: &str,
    thread_id: &str,
    call: WorkflowAgentCall,
    error: String,
    usage: Option<WorkflowChildRunUsage>,
) -> Result<WorkflowAgentExecutionResult, WorkflowError> {
    db.finish_workflow_child_run(
        workflow_run_id,
        child_run_id,
        "failed",
        None,
        None,
        None,
        Some(&error),
        usage,
    )?;
    db.append_workflow_event(WorkflowEventDraft {
        event_id: None,
        workflow_id: workflow_run_id.to_owned(),
        workflow_child_run_id: Some(child_run_id.to_owned()),
        thread_id: Some(thread_id.to_owned()),
        event_type: "workflow.child_failed".to_owned(),
        payload_json: json!({"error": error}).to_string(),
    })?;
    Ok(WorkflowAgentExecutionResult {
        workflow_child_run_id: child_run_id.to_owned(),
        thread_id: thread_id.to_owned(),
        label: call.label,
        binding: call.binding,
        order_index: call.order_index,
        phase_title: call.phase_title,
        result: None,
        preview: None,
        failed: true,
        optional: call.optional,
        error: Some(error),
    })
}

fn workflow_thread_metadata(
    workflow_thread_id: Option<&str>,
    workflow_definition_id: Option<&str>,
    workflow_definition_version: Option<u64>,
    status: &str,
) -> std::collections::HashMap<String, Value> {
    let mut metadata = std::collections::HashMap::from([
        ("workflow_thread".to_owned(), Value::Bool(true)),
        (
            "workflow_status".to_owned(),
            Value::String(status.to_owned()),
        ),
    ]);
    if let Some(workflow_thread_id) = workflow_thread_id {
        metadata.insert(
            "workflow_run_id".to_owned(),
            Value::String(workflow_thread_id.to_owned()),
        );
    }
    if let Some(workflow_definition_id) = workflow_definition_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        metadata.insert(
            "workflow_definition_id".to_owned(),
            Value::String(workflow_definition_id.to_owned()),
        );
    }
    if let Some(workflow_definition_version) = workflow_definition_version {
        metadata.insert(
            "workflow_definition_version".to_owned(),
            Value::Number(serde_json::Number::from(workflow_definition_version)),
        );
    }
    metadata
}
