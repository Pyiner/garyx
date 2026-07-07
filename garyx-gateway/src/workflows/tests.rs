use super::*;
use crate::garyx_db::WorkflowChildRunDraft;
use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, StreamCallback};
use garyx_models::config::GaryxConfig;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType};
use garyx_models::{Principal, TaskExecutor, TaskNotificationTarget, TaskStatus};
use garyx_router::tasks::{canonical_task_id, task_from_record};
use garyx_router::{CreateTaskInput, FileTaskCounterStore, TaskService};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;
use tower::ServiceExt;

struct WorkflowRecordingProvider {
    db: Arc<GaryxDbService>,
    provider_type: ProviderType,
    submit_structured_results: bool,
    run_count: Arc<AtomicUsize>,
    bridge_error: Option<String>,
}

#[async_trait]
impl AgentLoopProvider for WorkflowRecordingProvider {
    fn provider_type(&self) -> ProviderType {
        self.provider_type.clone()
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        self.run_count.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = &self.bridge_error {
            return Err(BridgeError::RunFailed(error.clone()));
        }
        let structured = options
            .metadata
            .contains_key(structured_result::STRUCTURED_RESULT_SCHEMA_METADATA_KEY);
        let response = if structured {
            let payload = json!({"summary": format!("structured: {}", options.message.lines().next().unwrap_or_default())});
            if self.submit_structured_results
                && let (Some(workflow_id), Some(child_id)) = (
                    options.metadata.get("workflow_id").and_then(Value::as_str),
                    options
                        .metadata
                        .get("workflow_child_run_id")
                        .and_then(Value::as_str),
                )
            {
                self.db
                    .submit_workflow_child_result(
                        workflow_id,
                        child_id,
                        &options.thread_id,
                        &payload.to_string(),
                        Some("structured fake result"),
                    )
                    .expect("submit workflow result");
            }
            payload.to_string()
        } else {
            format!(
                "text: {}",
                options.message.lines().next().unwrap_or_default()
            )
        };
        on_chunk(garyx_models::provider::StreamEvent::Delta {
            text: response.clone(),
        });
        on_chunk(garyx_models::provider::StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "workflow-provider-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response,
            session_messages: Vec::new(),
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{thread_id}"))
    }
}

async fn workflow_test_state_with_recording_provider_error(
    provider_type: ProviderType,
    submit_structured_results: bool,
    bridge_error: Option<String>,
) -> (Arc<AppState>, Arc<AtomicUsize>) {
    let bridge = Arc::new(MultiProviderBridge::new());
    let state = crate::server::AppStateBuilder::new(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
    .with_bridge(bridge.clone())
    .build();
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_thread_history(state.threads.history.clone());
    bridge.set_event_tx(state.ops.events.sender()).await;
    let run_count = Arc::new(AtomicUsize::new(0));
    bridge
        .register_provider(
            "workflow-test-provider",
            Arc::new(WorkflowRecordingProvider {
                db: state.ops.garyx_db.clone(),
                provider_type,
                submit_structured_results,
                run_count: run_count.clone(),
                bridge_error,
            }),
        )
        .await;
    bridge
        .set_default_provider_key("workflow-test-provider")
        .await;
    (state, run_count)
}

async fn workflow_test_state() -> Arc<AppState> {
    workflow_test_state_with_recording_provider(ProviderType::CodexAppServer, true)
        .await
        .0
}

async fn workflow_test_state_with_recording_provider(
    provider_type: ProviderType,
    submit_structured_results: bool,
) -> (Arc<AppState>, Arc<AtomicUsize>) {
    workflow_test_state_with_recording_provider_error(
        provider_type,
        submit_structured_results,
        None,
    )
    .await
}

#[test]
fn workflow_entrypoint_workspace_uses_task_input_then_defaults() {
    let input = json!({
        "workspaceDir": "/Users/test/input-workspace"
    });
    let defaults = json!({
        "workspaceDir": "/Users/test/default-workspace"
    });
    assert_eq!(
        workflow_workspace_dir_for_entrypoint(
            Some(" /Users/test/task-workspace "),
            &input,
            &defaults
        )
        .as_deref(),
        Some("/Users/test/task-workspace")
    );
    assert_eq!(
        workflow_workspace_dir_for_entrypoint(None, &input, &defaults).as_deref(),
        Some("/Users/test/input-workspace")
    );
    assert_eq!(
        workflow_workspace_dir_for_entrypoint(
            None,
            &json!({"workspace_dir": "/Users/test/input-snake"}),
            &defaults
        )
        .as_deref(),
        Some("/Users/test/input-snake")
    );
    assert_eq!(
        workflow_workspace_dir_for_entrypoint(None, &json!({}), &defaults).as_deref(),
        Some("/Users/test/default-workspace")
    );
}

#[test]
fn workflow_bun_command_uses_overrides_or_bundled_sibling() {
    let temp = tempdir().expect("runtime root");
    let exe = temp.path().join("garyx");
    let bundled_bun = temp.path().join("garyx-bun");
    std::fs::write(&exe, "").expect("fake garyx");
    std::fs::write(&bundled_bun, "#!/bin/sh\n").expect("fake bun");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&bundled_bun)
            .expect("fake bun metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&bundled_bun, permissions).expect("fake bun executable");
    }

    assert_eq!(
        entrypoint::workflow_bun_command_from_values(
            Some(exe.clone()),
            Some("/Users/test/custom-bun"),
            Some("/Users/test/generic-bun"),
            None,
        )
        .expect("workflow override"),
        PathBuf::from("/Users/test/custom-bun")
    );
    assert_eq!(
        entrypoint::workflow_bun_command_from_values(
            Some(exe.clone()),
            Some("  "),
            Some("/Users/test/generic-bun"),
            None,
        )
        .expect("generic override"),
        PathBuf::from("/Users/test/generic-bun")
    );
    assert_eq!(
        entrypoint::workflow_bun_command_from_values(Some(exe), None, None, None)
            .expect("bundled sibling"),
        bundled_bun
    );
}

#[test]
fn workflow_bun_command_resolves_system_path_then_errors_with_install_hint() {
    // No override, no embedded runtime, no bundled sibling: the gateway resolves
    // `bun` from PATH (the release binary no longer ships Bun), and otherwise
    // tells the user to install it.
    let temp = tempdir().expect("runtime root");
    let exe = temp.path().join("garyx"); // dir has no `garyx-bun` sibling
    std::fs::write(&exe, "").expect("fake garyx");

    let bun_dir = temp.path().join("pathdir");
    std::fs::create_dir_all(&bun_dir).expect("path dir");
    let path_bun = bun_dir.join("bun");
    std::fs::write(&path_bun, "#!/bin/sh\n").expect("fake bun");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path_bun)
            .expect("fake bun metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path_bun, permissions).expect("fake bun executable");
    }

    // Found on PATH.
    assert_eq!(
        entrypoint::workflow_bun_command_from_values(
            Some(exe.clone()),
            None,
            None,
            Some(bun_dir.as_os_str()),
        )
        .expect("system path bun"),
        path_bun.canonicalize().expect("canonicalize fake bun")
    );

    // Nothing anywhere -> a clear install error.
    let empty_path = temp.path().join("empty");
    std::fs::create_dir_all(&empty_path).expect("empty path dir");
    let error = entrypoint::workflow_bun_command_from_values(
        Some(exe),
        None,
        None,
        Some(empty_path.as_os_str()),
    )
    .expect_err("missing bun must error");
    let message = format!("{error:?}");
    assert!(
        message.contains("Bun is required") && message.contains("bun.sh"),
        "error should tell the user to install Bun: {message}"
    );
}

#[test]
fn workflow_definition_packages_read_manifest_input_and_ts_entrypoint() {
    let temp = tempdir().expect("workflow root");
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(temp.path().join("data").to_string_lossy().to_string());
    let package = temp.path().join("workflows").join("deep-research");
    std::fs::create_dir_all(&package).expect("package dirs");
    std::fs::write(package.join("workflow.ts"), "export {};\n").expect("entrypoint");
    std::fs::write(
        package.join(WORKFLOW_MANIFEST_FILE),
        r#"{
          "workflowId": "deep-research",
          "version": 2,
          "name": "Deep Research",
          "description": "File-backed workflow",
          "input": {"placeholder": "What should this workflow research?"},
          "defaults": {"workspaceDir": "/Users/test/project"}
        }"#,
    )
    .expect("manifest");
    let broken_package = temp.path().join("workflows").join("broken");
    std::fs::create_dir_all(&broken_package).expect("broken package dirs");
    std::fs::write(
        broken_package.join(WORKFLOW_MANIFEST_FILE),
        r#"{"workflowId":"broken","name":"Broken"}"#,
    )
    .expect("broken manifest");

    let packages = list_workflow_definition_packages(&config).expect("packages");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].record.workflow_id, "deep-research");
    assert_eq!(packages[0].record.version, 2);
    assert_eq!(
        parse_json_field(&packages[0].record.input_json)["placeholder"],
        "What should this workflow research?"
    );
    let package = get_workflow_definition_package(&config, "deep-research").expect("valid package");
    assert_eq!(package.record.workflow_id, "deep-research");
    assert!(package.package_dir.join("workflow.ts").is_file());
}

#[test]
fn workflow_definition_get_reports_target_package_error() {
    let temp = tempdir().expect("workflow root");
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(temp.path().join("data").to_string_lossy().to_string());
    let package = temp.path().join("workflows").join("broken");
    std::fs::create_dir_all(&package).expect("package dirs");
    std::fs::write(
        package.join(WORKFLOW_MANIFEST_FILE),
        r#"{"workflowId":"broken","name":"Broken"}"#,
    )
    .expect("manifest");

    let error = get_workflow_definition_package(&config, "broken").expect_err("error");
    assert!(
        error.to_string().contains("workflow.ts is required"),
        "{error}"
    );
}

async fn create_workflow_task_for_test(state: &Arc<AppState>) -> (String, String) {
    let data_dir = tempdir().expect("data dir");
    let task_service = TaskService::new(
        state.threads.thread_store.clone(),
        Arc::new(FileTaskCounterStore::new(data_dir.path())),
    );
    let (task_thread_id, task) = task_service
        .create_task(CreateTaskInput {
            title: Some("Run workflow".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: Some(TaskExecutor::Workflow {
                workflow_id: "unit".to_owned(),
                workflow_version: Some(1),
            }),
            start: true,
            actor: Some(Principal::Agent {
                agent_id: "workflow".to_owned(),
            }),
            agent_id: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .expect("task");
    (task_thread_id, canonical_task_id(&task))
}

async fn start_sdk_workflow_for_test(state: &Arc<AppState>) -> String {
    let (task_thread_id, task_id) = create_workflow_task_for_test(state).await;
    let payload = WorkflowRuntime::new(state.clone())
        .start_sdk(WorkflowSdkStartRequest {
            workflow_run_id: None,
            workflow_id: None,
            task_id: Some(task_id),
            task_thread_id: Some(task_thread_id.clone()),
            workflow_definition_id: Some("unit".to_owned()),
            workflow_definition_version: Some(1),
            workflow_definition_snapshot: Some(json!({
                "workflowId": "unit",
                "version": 1
            })),
            input: Some(json!("test workflow input")),
            parent_thread_id: Some(task_thread_id.clone()),
            parent_run_id: None,
            name: None,
            description: None,
            phases: Vec::new(),
            workspace_dir: None,
            created_by: Some("test".to_owned()),
        })
        .await
        .expect("start workflow");
    let workflow_id = payload["workflow"]["workflowId"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    assert_eq!(workflow_id, task_thread_id);
    workflow_id
}

#[tokio::test]
async fn sdk_start_persists_workflow_input() {
    let state = workflow_test_state().await;
    let workflow_run_id = start_sdk_workflow_for_test(&state).await;
    let payload = workflow_payload(
        &WorkflowStore::new(state.ops.garyx_db.clone()),
        &workflow_run_id,
    )
    .expect("workflow payload");
    assert_eq!(payload["workflow"]["input"], "test workflow input");

    let stored = state
        .ops
        .garyx_db
        .get_workflow_run(&workflow_run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(
        stored.input_json.as_deref(),
        Some("\"test workflow input\"")
    );
}

#[tokio::test]
async fn sdk_start_persists_phase_plan_in_meta() {
    let state = workflow_test_state().await;
    let (task_thread_id, task_id) = create_workflow_task_for_test(&state).await;
    let payload = WorkflowRuntime::new(state.clone())
        .start_sdk(WorkflowSdkStartRequest {
            workflow_run_id: None,
            workflow_id: None,
            task_id: Some(task_id),
            task_thread_id: Some(task_thread_id.clone()),
            workflow_definition_id: Some("unit".to_owned()),
            workflow_definition_version: Some(1),
            workflow_definition_snapshot: Some(json!({
                "workflowId": "unit",
                "version": 1
            })),
            input: Some(json!("phase plan test")),
            parent_thread_id: Some(task_thread_id.clone()),
            parent_run_id: None,
            name: Some("Phase plan workflow".to_owned()),
            description: Some("records planned phases".to_owned()),
            phases: vec![
                WorkflowSdkPhaseDefinition {
                    id: Some("scope".to_owned()),
                    title: "Scope".to_owned(),
                    detail: Some("Define the work".to_owned()),
                    index: Some(0),
                },
                WorkflowSdkPhaseDefinition {
                    id: Some("review".to_owned()),
                    title: "Review".to_owned(),
                    detail: None,
                    index: Some(1),
                },
            ],
            workspace_dir: None,
            created_by: Some("test".to_owned()),
        })
        .await
        .expect("start workflow");

    assert_eq!(
        payload["workflow"]["meta"]["phases"][0],
        json!({
            "id": "scope",
            "title": "Scope",
            "detail": "Define the work",
            "index": 0
        })
    );
    assert_eq!(payload["workflow"]["meta"]["phases"][1]["title"], "Review");
}

fn summary_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary"],
        "properties": { "summary": { "type": "string" } },
    })
}

#[test]
fn structured_result_schema_requires_object_tool_arguments() {
    assert!(validate_result_tool_schema(&summary_schema()).is_ok());
    assert!(matches!(
        validate_result_tool_schema(&json!({"type": "array", "items": {"type": "string"}})),
        Err(WorkflowError::BadRequest(_))
    ));
}

#[test]
fn structured_result_payload_validation_checks_enum_membership() {
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["quality"],
        "properties": {
            "quality": {
                "type": "string",
                "enum": ["high", "medium", "low"]
            }
        }
    });
    assert!(matches!(
        validate_payload_against_schema(&schema, &json!({"quality": "unknown"}), "$"),
        Err(WorkflowError::BadRequest(_))
    ));
    assert!(validate_payload_against_schema(&schema, &json!({"quality": "high"}), "$").is_ok());
}

#[test]
fn structured_result_accepts_stringified_json_when_it_matches_schema() {
    let schema = json!({
        "type": "object",
        "required": ["label", "ok"],
        "properties": {
            "label": { "type": "string" },
            "ok": { "type": "boolean" }
        }
    });
    let result = normalize_submitted_payload(
        &schema,
        Value::String(r#"{"label":"structured-smoke","ok":true}"#.to_owned()),
    );
    assert_eq!(result["label"], "structured-smoke");
    assert_eq!(result["ok"], true);
}

#[tokio::test]
async fn structured_result_submission_uses_thread_metadata_schema() {
    let state = workflow_test_state().await;
    let workflow_id = start_sdk_workflow_for_test(&state).await;
    let child_id = "workflow-child::structured-thread".to_owned();
    let schema = summary_schema();
    let metadata = workflow_child_metadata(
        &workflow_id,
        &child_id,
        "parent-thread",
        "inspect",
        0,
        Some(&schema),
    );
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("structured child".to_owned()),
            metadata,
            ..ThreadEnsureOptions::default()
        },
    )
    .await
    .expect("child thread");
    state
        .ops
        .garyx_db
        .upsert_workflow_child_run(WorkflowChildRunDraft {
            workflow_id: workflow_id.clone(),
            workflow_child_run_id: Some(child_id.clone()),
            thread_id: thread_id.clone(),
            phase_index: 0,
            phase_title: "Inspect".to_owned(),
            label: "inspect".to_owned(),
            agent_id: None,
            status: "running".to_owned(),
            prompt: "Inspect".to_owned(),
            result_mode: "structured".to_owned(),
            schema_json: Some(schema.to_string()),
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
        })
        .expect("child row");

    let submitted = submit_structured_result_for_thread(
        &state,
        &thread_id,
        json!({"summary": "thread schema result"}),
    )
    .await
    .expect("submit structured result");
    assert_eq!(submitted.workflow_id, workflow_id);
    assert_eq!(submitted.workflow_child_run_id, child_id);

    let second =
        submit_structured_result_for_thread(&state, &thread_id, json!({"summary": "second"}))
            .await
            .expect_err("second submit should not overwrite first result");
    assert!(matches!(second, WorkflowError::Conflict(_)));

    let child = state
        .ops
        .garyx_db
        .get_workflow_child_run(&workflow_id, &child_id)
        .expect("get child")
        .expect("child");
    assert_eq!(
        child.result_json.as_deref(),
        Some(r#"{"summary":"thread schema result"}"#)
    );
}

#[tokio::test]
async fn structured_result_submission_requires_matching_child_thread_row() {
    let state = workflow_test_state().await;
    let workflow_id = start_sdk_workflow_for_test(&state).await;
    let child_id = "workflow-child::structured-thread-mismatch".to_owned();
    let schema = summary_schema();
    let metadata = workflow_child_metadata(
        &workflow_id,
        &child_id,
        "parent-thread",
        "inspect",
        0,
        Some(&schema),
    );
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("structured child metadata".to_owned()),
            metadata,
            ..ThreadEnsureOptions::default()
        },
    )
    .await
    .expect("child thread");
    state
        .ops
        .garyx_db
        .upsert_workflow_child_run(WorkflowChildRunDraft {
            workflow_id: workflow_id.clone(),
            workflow_child_run_id: Some(child_id.clone()),
            thread_id: "thread::different-child-row".to_owned(),
            phase_index: 0,
            phase_title: "Inspect".to_owned(),
            label: "inspect".to_owned(),
            agent_id: None,
            status: "running".to_owned(),
            prompt: "Inspect".to_owned(),
            result_mode: "structured".to_owned(),
            schema_json: Some(schema.to_string()),
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
        })
        .expect("child row");

    let error =
        submit_structured_result_for_thread(&state, &thread_id, json!({"summary": "wrong thread"}))
            .await
            .expect_err("thread metadata must match child row thread id");
    assert!(matches!(error, WorkflowError::Conflict(_)));

    let child = state
        .ops
        .garyx_db
        .get_workflow_child_run(&workflow_id, &child_id)
        .expect("get child")
        .expect("child");
    assert!(child.result_json.is_none());
}

#[tokio::test]
async fn scheduler_queues_when_per_workflow_or_global_limit_is_full() {
    let scheduler = WorkflowScheduler::new(1, 1);
    let first = scheduler.acquire_child_permit("one").await.expect("first");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            scheduler.acquire_child_permit("one")
        )
        .await
        .is_err()
    );
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            scheduler.acquire_child_permit("two")
        )
        .await
        .is_err()
    );
    drop(first);
    let second = scheduler.acquire_child_permit("one").await.expect("second");
    assert_eq!(scheduler.available_global_child_slots(), 0);
    drop(second);
    assert_eq!(scheduler.available_global_child_slots(), 1);
}

#[tokio::test]
async fn sdk_workflow_start_without_task_context_creates_workflow_thread() {
    let state = workflow_test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Parent".to_owned()),
            ..ThreadEnsureOptions::default()
        },
    )
    .await
    .expect("create parent thread");
    let router = crate::route_graph::build_router(state.clone());
    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "Route Workflow",
                        "createdBy": "test",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("json");
    let workflow_thread_id = payload["workflow"]["workflowRunId"]
        .as_str()
        .expect("workflow thread id");
    assert!(workflow_thread_id.starts_with("thread::"));
    assert_eq!(payload["workflow"]["threadId"], workflow_thread_id);
    assert!(payload["workflow"]["taskId"].is_null());
    let workflow_thread = state
        .threads
        .thread_store
        .get(workflow_thread_id)
        .await
        .expect("workflow thread");
    assert_eq!(workflow_thread["thread_kind"], "workflow_run");
    assert_eq!(workflow_thread["workflow_run_id"], workflow_thread_id);

    let response = router
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "parentThreadId": thread_id,
                        "taskId": "#TASK-123",
                        "taskThreadId": thread_id,
                        "name": "Non-task thread",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("non-task response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sdk_workflow_start_without_task_context_reuses_requested_workflow_thread() {
    let state = workflow_test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Product workflow".to_owned()),
            thread_kind: Some("workflow_run".to_owned()),
            ..ThreadEnsureOptions::default()
        },
    )
    .await
    .expect("create workflow thread");

    let payload = WorkflowRuntime::new(state.clone())
        .start_sdk(WorkflowSdkStartRequest {
            workflow_run_id: Some(thread_id.clone()),
            workflow_id: None,
            task_id: None,
            task_thread_id: None,
            workflow_definition_id: Some("unit".to_owned()),
            workflow_definition_version: Some(1),
            workflow_definition_snapshot: None,
            input: Some(json!("direct product input")),
            parent_thread_id: None,
            parent_run_id: None,
            name: Some("Product workflow".to_owned()),
            description: None,
            phases: Vec::new(),
            workspace_dir: None,
            created_by: Some("test".to_owned()),
        })
        .await
        .expect("start workflow");

    assert_eq!(payload["workflow"]["workflowRunId"], thread_id);
    assert!(payload["workflow"]["taskId"].is_null());
    let workflow_thread = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("workflow thread");
    assert_eq!(workflow_thread["thread_kind"], "workflow_run");
    assert_eq!(workflow_thread["workflow_run_id"], thread_id);
    assert_eq!(workflow_thread["workflow_status"], "running");
}

#[tokio::test]
async fn start_workflow_definition_route_creates_workflow_thread() {
    let data_dir = tempdir().expect("data dir");
    let workspace_dir = tempdir().expect("workspace dir");
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    config.gateway.auth_token = crate::test_support::TEST_GATEWAY_TOKEN.to_owned();
    let workflow_root = workflow_definitions_root_for_config(&config);
    let workflow_package = workflow_root.join("unit");
    fs::create_dir_all(&workflow_package).expect("workflow package");
    fs::write(
        workflow_package.join("garyx.workflow.json"),
        r#"{
          "workflowId": "unit",
          "version": 1,
          "name": "Unit Workflow",
          "description": "Unit route workflow",
          "defaults": {}
        }"#,
    )
    .expect("workflow manifest");
    fs::write(workflow_package.join("workflow.ts"), "export {};\n").expect("workflow source");
    let state = crate::server::AppStateBuilder::new(config).build();
    let router = crate::route_graph::build_router(state.clone());
    let old_bun = std::env::var_os("GARYX_WORKFLOW_BUN_BIN");
    unsafe {
        std::env::set_var("GARYX_WORKFLOW_BUN_BIN", "/usr/bin/true");
    }
    let response = router
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflow-definitions/unit/runs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "input": "ship the product",
                        "workspaceDir": workspace_dir.path().to_string_lossy(),
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    unsafe {
        if let Some(value) = old_bun {
            std::env::set_var("GARYX_WORKFLOW_BUN_BIN", value);
        } else {
            std::env::remove_var("GARYX_WORKFLOW_BUN_BIN");
        }
    }

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("json");
    let thread_id = payload["workflowRunId"].as_str().expect("workflow run id");
    assert_eq!(payload["thread"]["thread_id"], thread_id);
    assert_eq!(payload["thread"]["thread_type"], "workflow_run");
    assert_eq!(payload["thread"]["label"], "ship the product");
    assert_eq!(payload["dispatch"]["workflowRunId"], thread_id);

    let workflow_thread = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("workflow thread");
    assert_eq!(workflow_thread["thread_kind"], "workflow_run");
    assert_eq!(workflow_thread["label"], "ship the product");
    assert_eq!(workflow_thread["workflow_run_id"], thread_id);
    assert_eq!(workflow_thread["workflow_definition_id"], "unit");
    assert_eq!(
        workflow_thread["workspace_dir"],
        workspace_dir.path().to_string_lossy().as_ref(),
    );
}

#[tokio::test]
async fn sdk_workflow_start_rejects_fabricated_or_mismatched_task_context() {
    let state = workflow_test_state().await;
    let (thread_id, task_id) = create_workflow_task_for_test(&state).await;
    let router = crate::route_graph::build_router(state.clone());

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "parentThreadId": thread_id,
                        "taskId": "#TASK-999",
                        "taskThreadId": thread_id,
                        "name": "Forged task id",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("forged response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = router
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "parentThreadId": thread_id,
                        "taskId": task_id,
                        "taskThreadId": thread_id,
                        "workflowDefinitionId": "other",
                        "name": "Mismatched workflow definition",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("mismatch response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workflow_routes_create_list_and_page_events_for_sdk_run() {
    let state = workflow_test_state().await;
    let (thread_id, task_id) = create_workflow_task_for_test(&state).await;
    let router = crate::route_graph::build_router(state);
    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "parentThreadId": thread_id,
                        "taskId": task_id,
                        "taskThreadId": thread_id,
                        "name": "Route Workflow",
                        "createdBy": "test",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("json");
    let workflow_id = payload["workflow"]["workflowId"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    assert_eq!(workflow_id, thread_id);
    assert_eq!(payload["workflow"]["threadId"], thread_id);
    assert_eq!(payload["workflow"]["status"], "running");
    assert_eq!(payload["workflow"]["meta"]["source"], "sdk");

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("GET")
                .uri(format!("/api/workflows/{workflow_id}/events?after=0"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("events response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let events: Value = serde_json::from_slice(&body).expect("events json");
    assert_eq!(events["events"].as_array().expect("events").len(), 1);
    assert_eq!(events["events"][0]["eventSeq"], 1);

    let response = router
        .oneshot(
            crate::test_support::authed_request()
                .method("GET")
                .uri(format!("/api/threads/{}/workflows", thread_id))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("thread workflows response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let list: Value = serde_json::from_slice(&body).expect("list json");
    assert_eq!(list["workflows"].as_array().expect("workflows").len(), 1);
    assert_eq!(list["workflows"][0]["workflowId"], workflow_id);
}

#[tokio::test]
async fn workflow_get_returns_server_presentation_projection() {
    let state = workflow_test_state().await;
    let (thread_id, task_id) = create_workflow_task_for_test(&state).await;
    let db = state.ops.garyx_db.clone();
    let router = crate::route_graph::build_router(state);
    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "parentThreadId": thread_id,
                        "taskId": task_id,
                        "taskThreadId": thread_id,
                        "name": "Presentation Workflow",
                        "phases": [
                            { "id": "plan", "title": "Plan", "index": 0 },
                            { "id": "review", "title": "Review", "detail": "Architecture gate", "index": 1 },
                            { "id": "finalize", "title": "Finalize", "index": 2 }
                        ],
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("json");
    let workflow_id = payload["workflow"]["workflowId"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    assert_eq!(payload["presentation"]["counts"]["totalPhases"], 3);
    assert_eq!(payload["presentation"]["terminalComplete"], false);

    db.upsert_workflow_child_run(WorkflowChildRunDraft {
        workflow_id: workflow_id.clone(),
        workflow_child_run_id: Some("child::lint".to_owned()),
        thread_id: "thread::child-lint".to_owned(),
        phase_index: 1,
        phase_title: "Review".to_owned(),
        label: "Lint check".to_owned(),
        agent_id: Some("codex-test".to_owned()),
        status: "running".to_owned(),
        prompt: "Run lint.".to_owned(),
        result_mode: "structured".to_owned(),
        schema_json: None,
        result_text: None,
        result_json: None,
        result_preview: None,
        error: None,
        input_tokens: 700,
        output_tokens: 200,
        tool_calls: 3,
        cost_usd: 0.22,
        started_at: Some("2026-06-21T08:01:02Z".to_owned()),
        finished_at: None,
    })
    .expect("insert running child");
    db.upsert_workflow_child_run(WorkflowChildRunDraft {
        workflow_id: workflow_id.clone(),
        workflow_child_run_id: Some("child::risk".to_owned()),
        thread_id: "thread::child-risk".to_owned(),
        phase_index: 1,
        phase_title: "Review".to_owned(),
        label: "Risk review".to_owned(),
        agent_id: Some("claude-test".to_owned()),
        status: "failed".to_owned(),
        prompt: "Review high-risk paths.".to_owned(),
        result_mode: "text".to_owned(),
        schema_json: None,
        result_text: None,
        result_json: None,
        result_preview: None,
        error: Some("Missing fixture coverage.".to_owned()),
        input_tokens: 200,
        output_tokens: 40,
        tool_calls: 1,
        cost_usd: 0.09,
        started_at: Some("2026-06-21T08:01:12Z".to_owned()),
        finished_at: Some("2026-06-21T08:01:50Z".to_owned()),
    })
    .expect("insert failed child");

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri(format!("/api/workflows/{workflow_id}/events"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "eventType": "workflow.phase_started",
                        "payload": {
                            "title": "Review",
                            "detail": "Architecture gate",
                            "phaseIndex": 1
                        },
                    })
                    .to_string(),
                ))
                .expect("event request"),
        )
        .await
        .expect("event response");
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = router
        .oneshot(
            crate::test_support::authed_request()
                .method("GET")
                .uri(format!("/api/workflows/{workflow_id}"))
                .body(Body::empty())
                .expect("get request"),
        )
        .await
        .expect("get response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("get body");
    let payload: Value = serde_json::from_slice(&body).expect("workflow json");
    let presentation = &payload["presentation"];
    assert_eq!(payload["workflow"]["currentPhaseIndex"], 1);
    assert_eq!(presentation["workflowRunId"], workflow_id);
    assert_eq!(presentation["activePhase"]["phaseId"], "review");
    assert_eq!(presentation["activePhase"]["index"], 1);
    assert_eq!(presentation["phases"][1]["status"], "running");
    assert_eq!(presentation["phases"][1]["active"], true);
    assert_eq!(
        presentation["phases"][1]["children"]
            .as_array()
            .expect("review children")
            .iter()
            .map(|child| child["workflowChildRunId"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["child::risk", "child::lint"]
    );
    assert_eq!(
        presentation["childCards"]
            .as_array()
            .expect("child cards")
            .iter()
            .map(|child| child["workflowChildRunId"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["child::risk", "child::lint"]
    );
    assert_eq!(presentation["phaseStatus"][1]["status"], "running");
    assert_eq!(presentation["counts"]["completedPhases"], 0);
    assert_eq!(presentation["stale"], false);
    assert_eq!(presentation["latestEventSeq"], 2);
    assert!(
        presentation["snapshotVersion"]
            .as_u64()
            .expect("snapshot version")
            >= 2
    );
}

#[tokio::test]
async fn workflow_definition_routes_get_and_list_file_packages() {
    let temp = tempdir().expect("workflow root");
    let mut config = crate::test_support::with_gateway_auth(GaryxConfig::default());
    config.sessions.data_dir = Some(temp.path().join("data").to_string_lossy().to_string());
    let package = temp.path().join("workflows").join("deep-research");
    std::fs::create_dir_all(&package).expect("package dir");
    std::fs::write(package.join("workflow.ts"), "export {};\n").expect("entrypoint");
    std::fs::write(
        package.join(WORKFLOW_MANIFEST_FILE),
        r#"{
          "workflowId": "deep-research",
          "version": 3,
          "name": "Deep Research",
          "description": "Research with verification",
          "input": {"placeholder": "Research topic"},
          "defaults": {"agentId": "claude"}
        }"#,
    )
    .expect("manifest");
    let broken_package = temp.path().join("workflows").join("broken");
    std::fs::create_dir_all(&broken_package).expect("broken package dir");
    std::fs::write(
        broken_package.join(WORKFLOW_MANIFEST_FILE),
        r#"{"workflowId":"broken","name":"Broken"}"#,
    )
    .expect("broken manifest");
    let state = crate::server::AppStateBuilder::new(config).build();
    let router = crate::route_graph::build_router(state);

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("GET")
                .uri("/api/workflow-definitions/deep-research")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("get response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("GET")
                .uri("/api/workflow-definitions/deep-research/source")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("source response");
    assert_eq!(response.status(), StatusCode::OK);
    let source: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
            .expect("source json");
    assert_eq!(source["workflowId"], "deep-research");
    assert_eq!(source["path"], "./workflow.ts");
    assert_eq!(source["language"], "typescript");
    assert_eq!(source["content"], "export {};\n");

    let response = router
        .oneshot(
            crate::test_support::authed_request()
                .method("GET")
                .uri("/api/workflow-definitions")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
            .expect("json");
    assert_eq!(
        payload["workflowDefinitions"][0]["workflowId"],
        "deep-research"
    );
}

async fn start_linked_workflow_task_for_test(state: &Arc<AppState>) -> (String, String, String) {
    let (task_thread_id, task_id) = create_workflow_task_for_test(state).await;
    let payload = WorkflowRuntime::new(state.clone())
        .start_sdk(WorkflowSdkStartRequest {
            workflow_run_id: None,
            workflow_id: None,
            task_id: Some(task_id.clone()),
            task_thread_id: Some(task_thread_id.clone()),
            workflow_definition_id: Some("unit".to_owned()),
            workflow_definition_version: Some(1),
            workflow_definition_snapshot: Some(json!({
                "workflowId": "unit",
                "version": 1
            })),
            input: Some(json!("unit test input")),
            parent_thread_id: Some(task_thread_id.clone()),
            parent_run_id: None,
            name: Some("Unit workflow".to_owned()),
            description: None,
            phases: Vec::new(),
            workspace_dir: None,
            created_by: Some("test".to_owned()),
        })
        .await
        .expect("start workflow");
    let workflow_id = payload["workflow"]["workflowId"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    (task_thread_id, task_id, workflow_id)
}

async fn start_linked_workflow_task_with_notification_target_for_test(
    state: &Arc<AppState>,
    notification_target: TaskNotificationTarget,
) -> (String, String, String) {
    let data_dir = tempdir().expect("data dir");
    let task_service = TaskService::new(
        state.threads.thread_store.clone(),
        Arc::new(FileTaskCounterStore::new(data_dir.path())),
    );
    let (task_thread_id, task) = task_service
        .create_task(CreateTaskInput {
            title: Some("Run workflow with handoff".to_owned()),
            body: None,
            assignee: None,
            notification_target: Some(notification_target),
            source: None,
            executor: Some(TaskExecutor::Workflow {
                workflow_id: "unit".to_owned(),
                workflow_version: Some(1),
            }),
            start: true,
            actor: Some(Principal::Agent {
                agent_id: "workflow".to_owned(),
            }),
            agent_id: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .expect("task");
    let task_id = canonical_task_id(&task);
    let payload = WorkflowRuntime::new(state.clone())
        .start_sdk(WorkflowSdkStartRequest {
            workflow_run_id: None,
            workflow_id: None,
            task_id: Some(task_id.clone()),
            task_thread_id: Some(task_thread_id.clone()),
            workflow_definition_id: Some("unit".to_owned()),
            workflow_definition_version: Some(1),
            workflow_definition_snapshot: Some(json!({
                "workflowId": "unit",
                "version": 1
            })),
            input: Some(json!("unit test input")),
            parent_thread_id: Some(task_thread_id.clone()),
            parent_run_id: None,
            name: Some("Unit workflow".to_owned()),
            description: None,
            phases: Vec::new(),
            workspace_dir: None,
            created_by: Some("test".to_owned()),
        })
        .await
        .expect("start workflow");
    let workflow_id = payload["workflow"]["workflowId"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    (task_thread_id, task_id, workflow_id)
}

#[tokio::test]
async fn sdk_finish_moves_linked_workflow_task_to_review() {
    let state = workflow_test_state().await;
    let (task_thread_id, task_id, workflow_id) = start_linked_workflow_task_for_test(&state).await;
    WorkflowRuntime::new(state.clone())
        .finish_sdk(
            &workflow_id,
            WorkflowSdkFinishRequest {
                status: Some("succeeded".to_owned()),
                result: Some(json!({"ok": true})),
                output_text: None,
                error: None,
            },
        )
        .await
        .expect("finish workflow");

    let stored = state
        .threads
        .thread_store
        .get(&task_thread_id)
        .await
        .expect("task thread");
    let task = task_from_record(&stored)
        .expect("task parse")
        .expect("task record");
    assert_eq!(canonical_task_id(&task), task_id);
    assert_eq!(task.status, TaskStatus::InReview);
}

#[tokio::test]
async fn sdk_finish_delivers_output_text_as_task_handoff() {
    let state = workflow_test_state().await;
    let target_thread_id = "thread::workflow-review-target";
    state
        .threads
        .thread_store
        .set(
            target_thread_id,
            json!({
                "thread_id": target_thread_id,
                "channel": "api",
                "account_id": "main",
                "from_id": "loop",
            }),
        )
        .await;
    let (_task_thread_id, _task_id, workflow_id) =
        start_linked_workflow_task_with_notification_target_for_test(
            &state,
            TaskNotificationTarget::Thread {
                thread_id: target_thread_id.to_owned(),
            },
        )
        .await;

    WorkflowRuntime::new(state.clone())
        .finish_sdk(
            &workflow_id,
            WorkflowSdkFinishRequest {
                status: Some("succeeded".to_owned()),
                result: Some(json!({"ok": true})),
                output_text: Some("Workflow output handoff.".to_owned()),
                error: None,
            },
        )
        .await
        .expect("finish workflow");

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let snapshot = state
                .threads
                .history
                .thread_snapshot(target_thread_id, 20)
                .await
                .expect("target snapshot");
            if snapshot.combined_messages().iter().any(|message| {
                message.get("role").and_then(Value::as_str) == Some("user")
                    && message
                        .get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| content.contains("Workflow output handoff."))
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("workflow outputText handoff should be delivered to target thread");
}

#[tokio::test]
async fn sdk_finish_with_output_text_without_notification_target_still_succeeds() {
    let state = workflow_test_state().await;
    let (task_thread_id, task_id, workflow_id) = start_linked_workflow_task_for_test(&state).await;

    WorkflowRuntime::new(state.clone())
        .finish_sdk(
            &workflow_id,
            WorkflowSdkFinishRequest {
                status: Some("succeeded".to_owned()),
                result: Some(json!({"ok": true})),
                output_text: Some("Workflow output handoff.".to_owned()),
                error: None,
            },
        )
        .await
        .expect("finish workflow should not fail when no notification target exists");

    let stored = state
        .threads
        .thread_store
        .get(&task_thread_id)
        .await
        .expect("task thread");
    let task = task_from_record(&stored)
        .expect("task parse")
        .expect("task record");
    assert_eq!(canonical_task_id(&task), task_id);
    assert_eq!(task.status, TaskStatus::InReview);
}

#[tokio::test]
async fn workflow_cancel_moves_linked_task_to_review() {
    let state = workflow_test_state().await;
    let (task_thread_id, task_id, workflow_id) = start_linked_workflow_task_for_test(&state).await;

    assert!(
        cancel_workflow_run(&state, &workflow_id)
            .await
            .expect("cancel workflow")
    );

    let stored = state
        .threads
        .thread_store
        .get(&task_thread_id)
        .await
        .expect("task thread");
    let task = task_from_record(&stored)
        .expect("task parse")
        .expect("task record");
    assert_eq!(canonical_task_id(&task), task_id);
    assert_eq!(task.status, TaskStatus::InReview);
}

#[tokio::test]
async fn reconcile_interrupted_workflows_moves_linked_task_to_review() {
    let state = workflow_test_state().await;
    let (task_thread_id, task_id, workflow_id) = start_linked_workflow_task_for_test(&state).await;

    assert_eq!(
        reconcile_interrupted_workflows(&state, "9999-12-31T23:59:59.999Z").await,
        1
    );

    let workflow = state
        .ops
        .garyx_db
        .get_workflow_run(&workflow_id)
        .expect("get workflow")
        .expect("workflow exists");
    assert_eq!(workflow.status, "failed");
    assert_eq!(workflow.error.as_deref(), Some("gateway restarted"));
    let stored = state
        .threads
        .thread_store
        .get(&task_thread_id)
        .await
        .expect("task thread");
    let task = task_from_record(&stored)
        .expect("task parse")
        .expect("task record");
    assert_eq!(canonical_task_id(&task), task_id);
    assert_eq!(task.status, TaskStatus::InReview);
}

#[tokio::test]
async fn reconcile_interrupted_workflows_ignores_runs_after_startup_cutoff() {
    let state = workflow_test_state().await;
    let (task_thread_id, task_id, workflow_id) = start_linked_workflow_task_for_test(&state).await;

    assert_eq!(
        reconcile_interrupted_workflows(&state, "0000-01-01T00:00:00.000Z").await,
        0
    );

    let workflow = state
        .ops
        .garyx_db
        .get_workflow_run(&workflow_id)
        .expect("get workflow")
        .expect("workflow exists");
    assert_eq!(workflow.status, "running");
    let stored = state
        .threads
        .thread_store
        .get(&task_thread_id)
        .await
        .expect("task thread");
    let task = task_from_record(&stored)
        .expect("task parse")
        .expect("task record");
    assert_eq!(canonical_task_id(&task), task_id);
    assert_eq!(task.status, TaskStatus::InProgress);
}

#[tokio::test]
async fn workflow_entrypoint_success_without_run_moves_task_to_review() {
    let data_dir = tempdir().expect("data dir");
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    let workflow_root = workflow_definitions_root_for_config(&config);
    let workflow_package = workflow_root.join("unit");
    fs::create_dir_all(&workflow_package).expect("workflow package");
    fs::write(
        workflow_package.join("garyx.workflow.json"),
        r#"{
          "workflowId": "unit",
          "version": 1,
          "name": "Unit Workflow",
          "input": {"placeholder": "Unit request"},
          "defaults": {}
        }"#,
    )
    .expect("workflow manifest");
    fs::write(workflow_package.join("workflow.ts"), "export {};\n").expect("workflow source");
    let state = crate::server::AppStateBuilder::new(config).build();
    let (task_thread_id, task_id) = create_workflow_task_for_test(&state).await;
    let old_bun = std::env::var_os("GARYX_WORKFLOW_BUN_BIN");
    unsafe {
        std::env::set_var("GARYX_WORKFLOW_BUN_BIN", "/usr/bin/true");
    }
    let spawn_result = spawn_workflow_task_entrypoint(
        state.clone(),
        task_id.clone(),
        task_thread_id.clone(),
        "unit".to_owned(),
        json!({}),
        None,
    );
    unsafe {
        if let Some(value) = old_bun {
            std::env::set_var("GARYX_WORKFLOW_BUN_BIN", value);
        } else {
            std::env::remove_var("GARYX_WORKFLOW_BUN_BIN");
        }
    }
    spawn_result.expect("spawn workflow entrypoint");

    let mut observed = None;
    for _ in 0..20 {
        if let Some(record) = state.threads.thread_store.get(&task_thread_id).await
            && let Some(task) = task_from_record(&record).expect("task parse")
            && task.status == TaskStatus::InReview
        {
            observed = Some(task);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let task = observed.expect("task moved to review");
    assert_eq!(canonical_task_id(&task), task_id);
}

#[tokio::test]
async fn sdk_agent_executes_hidden_child_and_structured_schema() {
    let state = workflow_test_state().await;
    let (thread_id, task_id) = create_workflow_task_for_test(&state).await;
    let router = crate::route_graph::build_router(state.clone());
    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "parentThreadId": thread_id,
                        "taskId": task_id,
                        "taskThreadId": thread_id,
                        "name": "Structured flow",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("json");
    let workflow_id = payload["workflow"]["workflowId"]
        .as_str()
        .expect("workflow id")
        .to_owned();

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri(format!("/api/workflows/{workflow_id}/agents"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "prompt": "Inspect this",
                        "label": "inspect",
                        "binding": "finding",
                        "schema": summary_schema(),
                    })
                    .to_string(),
                ))
                .expect("agent request"),
        )
        .await
        .expect("agent response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("agent body");
    let agent_payload: Value = serde_json::from_slice(&body).expect("agent json");
    assert_eq!(agent_payload["failed"], false);
    assert!(agent_payload["result"]["summary"].as_str().is_some());

    let final_payload = workflow_payload(
        &WorkflowStore::new(state.ops.garyx_db.clone()),
        &workflow_id,
    )
    .expect("workflow payload");
    assert_eq!(final_payload["workflow"]["totalChildren"], 1);
    assert_eq!(final_payload["workflow"]["completedChildren"], 1);
    assert_eq!(final_payload["presentation"]["counts"]["total"], 1);
    assert_eq!(final_payload["presentation"]["counts"]["completed"], 1);
    assert_eq!(final_payload["presentation"]["terminalComplete"], false);
    let children = final_payload["children"].as_array().expect("children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["resultMode"], "structured");
    assert!(children[0]["result"]["summary"].as_str().is_some());
    assert!(
        children[0]["resultPreview"]
            .as_str()
            .expect("structured preview")
            .starts_with("{\"summary\":")
    );
    let child_thread_id = children[0]["threadId"].as_str().unwrap();
    let thread_data = state
        .threads
        .thread_store
        .get(child_thread_id)
        .await
        .expect("child thread");
    assert_eq!(thread_data["source"], "workflow");
    assert_eq!(thread_data["exclude_from_recent"], true);
}

#[tokio::test]
async fn optional_structured_child_missing_submit_result_fails_child_not_workflow() {
    let (state, _) =
        workflow_test_state_with_recording_provider(ProviderType::CodexAppServer, false).await;
    let workflow_id = start_sdk_workflow_for_test(&state).await;
    let agent_payload = WorkflowRuntime::new(state.clone())
        .run_sdk_agent(
            workflow_id.clone(),
            WorkflowSdkAgentRequest {
                prompt: "Inspect without submitting".to_owned(),
                label: Some("optional-inspect".to_owned()),
                binding: Some("finding".to_owned()),
                order_index: Some(0),
                phase_index: Some(0),
                phase_title: Some("Inspect".to_owned()),
                agent_id: None,
                workspace_dir: None,
                schema: Some(summary_schema()),
                optional: Some(true),
            },
        )
        .await
        .expect("run optional child");
    assert_eq!(agent_payload["failed"], true);
    assert_eq!(agent_payload["optional"], true);

    let payload = workflow_payload(
        &WorkflowStore::new(state.ops.garyx_db.clone()),
        &workflow_id,
    )
    .expect("workflow payload");
    assert_eq!(payload["workflow"]["status"], "running");
    let children = payload["children"].as_array().expect("children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["status"], "failed");
    assert!(
        children[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("without submit_result")
    );
    assert!(children[0]["finishedAt"].as_str().is_some());
}

#[tokio::test]
async fn optional_child_bridge_error_fails_child_not_workflow() {
    let (state, run_count) = workflow_test_state_with_recording_provider_error(
        ProviderType::CodexAppServer,
        true,
        Some("provider unavailable".to_owned()),
    )
    .await;
    let workflow_id = start_sdk_workflow_for_test(&state).await;
    let agent_payload = WorkflowRuntime::new(state.clone())
        .run_sdk_agent(
            workflow_id.clone(),
            WorkflowSdkAgentRequest {
                prompt: "Inspect through failing provider".to_owned(),
                label: Some("optional-provider".to_owned()),
                binding: Some("finding".to_owned()),
                order_index: Some(0),
                phase_index: Some(0),
                phase_title: Some("Inspect".to_owned()),
                agent_id: None,
                workspace_dir: None,
                schema: None,
                optional: Some(true),
            },
        )
        .await
        .expect("run optional child");

    assert_eq!(run_count.load(Ordering::SeqCst), 1);
    assert_eq!(agent_payload["failed"], true);
    let payload = workflow_payload(
        &WorkflowStore::new(state.ops.garyx_db.clone()),
        &workflow_id,
    )
    .expect("workflow payload");
    assert_eq!(payload["workflow"]["totalChildren"], 1);
    assert_eq!(payload["workflow"]["completedChildren"], 1);
    assert_eq!(payload["workflow"]["failedChildren"], 1);
    assert_eq!(payload["workflow"]["status"], "running");
    let children = payload["children"].as_array().expect("children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["status"], "failed");
    assert!(
        children[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("provider unavailable")
    );
    assert!(children[0]["finishedAt"].as_str().is_some());
}

#[tokio::test]
async fn structured_child_with_native_provider_fails_before_launch() {
    let (state, run_count) =
        workflow_test_state_with_recording_provider(ProviderType::CodexAppServer, true).await;
    state
        .ops
        .custom_agents
        .upsert_agent_for_test(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "native-structured".to_owned(),
            display_name: "Native Structured".to_owned(),
            provider_type: ProviderType::Gpt,
            model: Some(String::new()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: None,
            avatar_data_url: None,
            system_prompt: Some("Native structured test agent".to_owned()),
        })
        .await
        .expect("custom native agent");
    let workflow_id = start_sdk_workflow_for_test(&state).await;
    let agent_payload = WorkflowRuntime::new(state.clone())
        .run_sdk_agent(
            workflow_id.clone(),
            WorkflowSdkAgentRequest {
                prompt: "Inspect with native provider".to_owned(),
                label: Some("native-inspect".to_owned()),
                binding: Some("finding".to_owned()),
                order_index: Some(0),
                phase_index: Some(0),
                phase_title: Some("Inspect".to_owned()),
                agent_id: Some("native-structured".to_owned()),
                workspace_dir: None,
                schema: Some(summary_schema()),
                optional: Some(false),
            },
        )
        .await
        .expect("run native structured child");

    assert_eq!(run_count.load(Ordering::SeqCst), 0);
    assert_eq!(agent_payload["failed"], true);
    let payload = workflow_payload(
        &WorkflowStore::new(state.ops.garyx_db.clone()),
        &workflow_id,
    )
    .expect("workflow payload");
    assert_eq!(payload["workflow"]["status"], "running");
    let children = payload["children"].as_array().expect("children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["status"], "failed");
    assert!(
        children[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("does not support submit_result")
    );
    assert!(children[0]["finishedAt"].as_str().is_some());
}

#[tokio::test]
async fn sdk_workflow_routes_start_log_run_agent_and_finish() {
    let state = workflow_test_state().await;
    let (thread_id, task_id) = create_workflow_task_for_test(&state).await;
    let router = crate::route_graph::build_router(state.clone());

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri("/api/workflows/sdk")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "parentThreadId": thread_id,
                        "taskId": task_id,
                        "taskThreadId": thread_id,
                        "name": "SDK Flow",
                        "workspaceDir": "/Users/test/project",
                    })
                    .to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("start sdk response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: Value = serde_json::from_slice(&body).expect("json");
    let workflow_id = payload["workflow"]["workflowId"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    assert_eq!(payload["workflow"]["meta"]["source"], "sdk");

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri(format!("/api/workflows/{workflow_id}/events"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "eventType": "workflow.sdk_log",
                        "payload": { "message": "phase started" },
                    })
                    .to_string(),
                ))
                .expect("event request"),
        )
        .await
        .expect("event response");
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri(format!("/api/workflows/{workflow_id}/agents"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "prompt": "Inspect SDK flow",
                        "label": "inspect",
                        "binding": "finding",
                        "orderIndex": 0,
                    })
                    .to_string(),
                ))
                .expect("agent request"),
        )
        .await
        .expect("agent response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("agent body");
    let agent_payload: Value = serde_json::from_slice(&body).expect("agent json");
    assert_eq!(agent_payload["label"], "inspect");
    assert_eq!(agent_payload["binding"], "finding");
    assert_eq!(agent_payload["failed"], false);

    let response = router
        .clone()
        .oneshot(
            crate::test_support::authed_request()
                .method("POST")
                .uri(format!("/api/workflows/{workflow_id}/finish"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "result": { "finding": agent_payload["result"].clone() },
                        "outputText": "SDK done",
                    })
                    .to_string(),
                ))
                .expect("finish request"),
        )
        .await
        .expect("finish response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("finish body");
    let final_payload: Value = serde_json::from_slice(&body).expect("finish json");
    assert_eq!(final_payload["workflow"]["status"], "succeeded");
    assert_eq!(final_payload["workflow"]["totalChildren"], 1);
    assert_eq!(final_payload["presentation"]["terminalComplete"], true);
    assert_eq!(final_payload["presentation"]["stale"], false);
    assert_eq!(final_payload["presentation"]["counts"]["completed"], 1);
    assert_eq!(
        final_payload["presentation"]["outcome"]["kind"],
        "finalText"
    );
    assert_eq!(
        final_payload["workflow"]["result"]["finding"],
        agent_payload["result"]
    );
    assert_eq!(final_payload["workflow"]["outputText"], "SDK done");

    let events = WorkflowStore::new(state.ops.garyx_db.clone())
        .events_after(&workflow_id, 0, 20)
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "workflow.sdk_log")
    );
}
