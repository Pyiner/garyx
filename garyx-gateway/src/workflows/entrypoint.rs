use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const GARYX_WORKFLOW_SDK_SOURCE: &str =
    include_str!("../../../packages/garyx-workflow/src/index.ts");
const GARYX_WORKFLOW_SDK_PACKAGE_JSON: &str = r#"{
  "name": "@garyx/workflow",
  "version": "0.0.0",
  "type": "module",
  "exports": {
    ".": {
      "import": "./index.ts",
      "default": "./index.ts"
    }
  }
}
"#;

pub fn spawn_workflow_task_entrypoint(
    state: Arc<AppState>,
    task_id: String,
    task_thread_id: String,
    workflow_id: String,
    input: Value,
    task_workspace_dir: Option<String>,
) -> Result<Value, WorkflowError> {
    let config = state.config_snapshot();
    let definition = get_workflow_definition_package(&config, &workflow_id)?;
    let node_path = prepare_workflow_sdk_node_path(&config)?;
    let snapshot = workflow_definition_package_json(&definition);
    let gateway_url = config
        .gateway
        .public_url
        .trim()
        .trim_end_matches('/')
        .to_owned();
    let gateway_url = if gateway_url.is_empty() {
        format!("http://127.0.0.1:{}", config.gateway.port)
    } else {
        gateway_url
    };
    let input_json = input.to_string();
    let workflow_args = workflow_argument_string(&input);
    let defaults = parse_json_field(&definition.record.defaults_json);
    let workspace_dir_for_spawn =
        workflow_workspace_dir_for_entrypoint(task_workspace_dir.as_deref(), &input, &defaults);
    let command_name = workflow_bun_command()?;
    let command_args = vec![WORKFLOW_ENTRYPOINT_FILE.to_owned()];
    let task_id_for_spawn = task_id.clone();
    let task_thread_for_spawn = task_thread_id.clone();
    let workflow_id_for_spawn = definition.record.workflow_id.clone();
    let version_for_spawn = definition.record.version.to_string();
    let snapshot_for_spawn = snapshot.to_string();
    let token_for_spawn = config.gateway.auth_token.trim().to_owned();
    let package_dir_for_spawn = definition.package_dir.clone();
    let node_path_for_spawn = node_path_env_value(node_path);
    tokio::spawn(async move {
        let mut command = tokio::process::Command::new(&command_name);
        command.args(&command_args);
        command.current_dir(&package_dir_for_spawn);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("GARYX_TASK_ID", &task_id_for_spawn)
            .env("GARYX_TASK_THREAD_ID", &task_thread_for_spawn)
            .env("GARYX_PARENT_THREAD_ID", &task_thread_for_spawn)
            .env("GARYX_WORKFLOW_THREAD_ID", &task_thread_for_spawn)
            .env("GARYX_WORKFLOW_RUN_ID", &task_thread_for_spawn)
            .env("GARYX_WORKFLOW_DEFINITION_ID", &workflow_id_for_spawn)
            .env("GARYX_WORKFLOW_DEFINITION_VERSION", &version_for_spawn)
            .env("GARYX_WORKFLOW_DEFINITION_SNAPSHOT", &snapshot_for_spawn)
            .env("GARYX_WORKFLOW_INPUT_JSON", &input_json)
            .env("GARYX_WORKFLOW_ARGS", &workflow_args)
            .env("GARYX_GATEWAY_URL", &gateway_url);
        if let Some(workspace_dir) = workspace_dir_for_spawn.as_deref() {
            command.env("GARYX_WORKSPACE_DIR", workspace_dir);
        }
        command.env("GARYX_WORKFLOW_DIR", &package_dir_for_spawn);
        command.env("NODE_PATH", &node_path_for_spawn);
        if !token_for_spawn.is_empty() {
            command.env("GARYX_GATEWAY_TOKEN", token_for_spawn);
        }
        match command.spawn() {
            Ok(mut child) => match child.wait().await {
                Ok(status) if status.success() => {
                    match WorkflowStore::new(state.ops.garyx_db.clone())
                        .get_run(&task_thread_for_spawn)
                    {
                        Ok(_) => {}
                        Err(WorkflowError::NotFound(_)) => {
                            mark_workflow_task_entrypoint_failed(
                                &state,
                                &task_thread_for_spawn,
                                "workflow entrypoint exited without creating a workflow run"
                                    .to_owned(),
                            )
                            .await;
                        }
                        Err(error) => {
                            mark_workflow_task_entrypoint_failed(
                                &state,
                                &task_thread_for_spawn,
                                format!("workflow entrypoint run check failed: {error}"),
                            )
                            .await;
                        }
                    }
                }
                Ok(status) => {
                    mark_workflow_task_entrypoint_failed(
                        &state,
                        &task_thread_for_spawn,
                        format!("workflow entrypoint exited with status {status}"),
                    )
                    .await;
                }
                Err(error) => {
                    mark_workflow_task_entrypoint_failed(
                        &state,
                        &task_thread_for_spawn,
                        format!("workflow entrypoint wait failed: {error}"),
                    )
                    .await;
                }
            },
            Err(error) => {
                mark_workflow_task_entrypoint_failed(
                    &state,
                    &task_thread_for_spawn,
                    format!("workflow entrypoint failed to start: {error}"),
                )
                .await;
            }
        }
    });
    Ok(json!({
        "kind": "workflow_entrypoint",
        "workflowDefinitionId": definition.record.workflow_id,
        "workflowId": definition.record.workflow_id,
        "workflowVersion": definition.record.version,
        "workflowRunId": task_thread_id.clone(),
        "threadId": task_thread_id.clone(),
        "taskId": task_id,
        "taskThreadId": task_thread_id,
    }))
}

pub fn spawn_workflow_thread_entrypoint(
    state: Arc<AppState>,
    workflow_thread_id: String,
    workflow_id: String,
    input: Value,
    workspace_dir: Option<String>,
) -> Result<Value, WorkflowError> {
    let config = state.config_snapshot();
    let definition = get_workflow_definition_package(&config, &workflow_id)?;
    let node_path = prepare_workflow_sdk_node_path(&config)?;
    let snapshot = workflow_definition_package_json(&definition);
    let gateway_url = config
        .gateway
        .public_url
        .trim()
        .trim_end_matches('/')
        .to_owned();
    let gateway_url = if gateway_url.is_empty() {
        format!("http://127.0.0.1:{}", config.gateway.port)
    } else {
        gateway_url
    };
    let input_json = input.to_string();
    let workflow_args = workflow_argument_string(&input);
    let defaults = parse_json_field(&definition.record.defaults_json);
    let workspace_dir_for_spawn =
        workflow_workspace_dir_for_entrypoint(workspace_dir.as_deref(), &input, &defaults);
    let command_name = workflow_bun_command()?;
    let command_args = vec![WORKFLOW_ENTRYPOINT_FILE.to_owned()];
    let workflow_thread_for_spawn = workflow_thread_id.clone();
    let workflow_id_for_spawn = definition.record.workflow_id.clone();
    let version_for_spawn = definition.record.version.to_string();
    let snapshot_for_spawn = snapshot.to_string();
    let token_for_spawn = config.gateway.auth_token.trim().to_owned();
    let package_dir_for_spawn = definition.package_dir.clone();
    let node_path_for_spawn = node_path_env_value(node_path);
    tokio::spawn(async move {
        let mut command = tokio::process::Command::new(&command_name);
        command.args(&command_args);
        command.current_dir(&package_dir_for_spawn);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("GARYX_PARENT_THREAD_ID", &workflow_thread_for_spawn)
            .env("GARYX_WORKFLOW_THREAD_ID", &workflow_thread_for_spawn)
            .env("GARYX_WORKFLOW_RUN_ID", &workflow_thread_for_spawn)
            .env("GARYX_WORKFLOW_DEFINITION_ID", &workflow_id_for_spawn)
            .env("GARYX_WORKFLOW_DEFINITION_VERSION", &version_for_spawn)
            .env("GARYX_WORKFLOW_DEFINITION_SNAPSHOT", &snapshot_for_spawn)
            .env("GARYX_WORKFLOW_INPUT_JSON", &input_json)
            .env("GARYX_WORKFLOW_ARGS", &workflow_args)
            .env("GARYX_GATEWAY_URL", &gateway_url);
        if let Some(workspace_dir) = workspace_dir_for_spawn.as_deref() {
            command.env("GARYX_WORKSPACE_DIR", workspace_dir);
        }
        command.env("GARYX_WORKFLOW_DIR", &package_dir_for_spawn);
        command.env("NODE_PATH", &node_path_for_spawn);
        if !token_for_spawn.is_empty() {
            command.env("GARYX_GATEWAY_TOKEN", token_for_spawn);
        }
        match command.spawn() {
            Ok(mut child) => match child.wait().await {
                Ok(status) if status.success() => {
                    match WorkflowStore::new(state.ops.garyx_db.clone())
                        .get_run(&workflow_thread_for_spawn)
                    {
                        Ok(_) => {}
                        Err(WorkflowError::NotFound(_)) => {
                            mark_workflow_thread_entrypoint_failed(
                                &state,
                                &workflow_thread_for_spawn,
                                "workflow entrypoint exited without creating a workflow run"
                                    .to_owned(),
                            )
                            .await;
                        }
                        Err(error) => {
                            mark_workflow_thread_entrypoint_failed(
                                &state,
                                &workflow_thread_for_spawn,
                                format!("workflow entrypoint run check failed: {error}"),
                            )
                            .await;
                        }
                    }
                }
                Ok(status) => {
                    mark_workflow_thread_entrypoint_failed(
                        &state,
                        &workflow_thread_for_spawn,
                        format!("workflow entrypoint exited with status {status}"),
                    )
                    .await;
                }
                Err(error) => {
                    mark_workflow_thread_entrypoint_failed(
                        &state,
                        &workflow_thread_for_spawn,
                        format!("workflow entrypoint wait failed: {error}"),
                    )
                    .await;
                }
            },
            Err(error) => {
                mark_workflow_thread_entrypoint_failed(
                    &state,
                    &workflow_thread_for_spawn,
                    format!("workflow entrypoint failed to start: {error}"),
                )
                .await;
            }
        }
    });
    Ok(json!({
        "kind": "workflow_entrypoint",
        "workflowDefinitionId": definition.record.workflow_id,
        "workflowId": definition.record.workflow_id,
        "workflowVersion": definition.record.version,
        "workflowRunId": workflow_thread_id.clone(),
        "threadId": workflow_thread_id,
    }))
}

pub(super) fn workflow_bun_command() -> Result<PathBuf, WorkflowError> {
    workflow_bun_command_from_values(
        std::env::current_exe().ok(),
        std::env::var("GARYX_WORKFLOW_BUN_BIN").ok().as_deref(),
        std::env::var("GARYX_BUN_BIN").ok().as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
}

pub(super) fn workflow_bun_command_from_values(
    current_exe: Option<PathBuf>,
    workflow_bin_override: Option<&str>,
    bun_bin_override: Option<&str>,
    path_var: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, WorkflowError> {
    if let Some(path) = normalized_optional_string(workflow_bin_override)
        .or_else(|| normalized_optional_string(bun_bin_override))
        .map(PathBuf::from)
    {
        return Ok(path);
    }
    if let Some(path) = bundled_workflow_bun_path(current_exe.as_deref()) {
        return Ok(path);
    }
    if let Some(path) = path_var.and_then(bun_on_path) {
        return Ok(path);
    }
    Err(WorkflowError::Conflict(
        "Bun is required to run Garyx workflows but was not found. Install Bun \
         (https://bun.sh — e.g. `brew install bun` or `curl -fsSL https://bun.sh/install | bash`) \
         so that `bun` is on your PATH, or set GARYX_WORKFLOW_BUN_BIN to a Bun binary."
            .to_owned(),
    ))
}

fn bundled_workflow_bun_path(current_exe: Option<&FsPath>) -> Option<PathBuf> {
    let exe_dir = current_exe?.parent()?;
    let sibling = exe_dir.join("garyx-bun");
    executable_file_exists(&sibling).then_some(sibling)
}

/// Find an executable `bun` on a `PATH`-formatted value. The release binary no
/// longer bundles a Bun runtime, so workflows run on the user's installed Bun;
/// when it is absent `workflow_bun_command_from_values` returns install
/// instructions. Takes the PATH value so it can be unit-tested without mutating
/// the process environment.
fn bun_on_path(path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .map(|dir| dir.join("bun"))
        .find(|candidate| executable_file_exists(candidate))
        // Canonicalize so a relative PATH entry resolves to an absolute path: the
        // workflow process is spawned with its cwd changed to the package dir, and
        // a relative `bun` would otherwise resolve against the wrong directory.
        .and_then(|candidate| candidate.canonicalize().ok())
}

fn executable_file_exists(path: &FsPath) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn prepare_workflow_sdk_node_path(config: &GaryxConfig) -> Result<PathBuf, WorkflowError> {
    let node_path = workflow_definitions_root_for_config(config)
        .join(".runtime")
        .join("node_path");
    let sdk_dir = node_path.join("@garyx").join("workflow");
    fs::create_dir_all(&sdk_dir).map_err(workflow_io_error)?;
    write_if_changed(
        &sdk_dir.join("package.json"),
        GARYX_WORKFLOW_SDK_PACKAGE_JSON,
    )?;
    write_if_changed(&sdk_dir.join("index.ts"), GARYX_WORKFLOW_SDK_SOURCE)?;
    Ok(node_path)
}

fn write_if_changed(path: &FsPath, content: &str) -> Result<(), WorkflowError> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    fs::write(path, content).map_err(workflow_io_error)
}

fn node_path_env_value(node_path: PathBuf) -> std::ffi::OsString {
    let mut paths = vec![node_path.clone()];
    if let Some(existing) = std::env::var_os("NODE_PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap_or_else(|_| node_path.into_os_string())
}

pub(super) fn workflow_workspace_dir_for_entrypoint(
    task_workspace_dir: Option<&str>,
    input: &Value,
    defaults: &Value,
) -> Option<String> {
    normalized_optional_string(task_workspace_dir)
        .or_else(|| json_string_field(input, "workspaceDir"))
        .or_else(|| json_string_field(input, "workspace_dir"))
        .or_else(|| json_string_field(defaults, "workspaceDir"))
        .or_else(|| json_string_field(defaults, "workspace_dir"))
}

pub(super) async fn mark_workflow_task_entrypoint_failed(
    state: &Arc<AppState>,
    task_thread_id: &str,
    note: String,
) {
    let _ = mark_workflow_task_in_review(state, task_thread_id, note, None).await;
}

pub(super) async fn mark_workflow_thread_entrypoint_failed(
    state: &Arc<AppState>,
    workflow_thread_id: &str,
    note: String,
) {
    let _ = state.ops.garyx_db.update_workflow_run_status(
        workflow_thread_id,
        "failed",
        None,
        None,
        Some(&note),
    );
    let Some(mut record) = state.threads.thread_store.get(workflow_thread_id).await else {
        return;
    };
    if let Some(obj) = record.as_object_mut() {
        obj.insert(
            "thread_kind".to_owned(),
            Value::String("workflow_run".to_owned()),
        );
        obj.insert(
            "workflow_run_id".to_owned(),
            Value::String(workflow_thread_id.to_owned()),
        );
        obj.insert(
            "workflow_status".to_owned(),
            Value::String("failed".to_owned()),
        );
        let metadata_value = obj
            .entry("metadata".to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !metadata_value.is_object() {
            *metadata_value = Value::Object(serde_json::Map::new());
        }
        if let Some(metadata) = metadata_value.as_object_mut() {
            metadata.insert("workflow_thread".to_owned(), Value::Bool(true));
            metadata.insert(
                "workflow_run_id".to_owned(),
                Value::String(workflow_thread_id.to_owned()),
            );
            metadata.insert(
                "workflow_status".to_owned(),
                Value::String("failed".to_owned()),
            );
        }
        obj.insert("updated_at".to_owned(), Value::String(now_string()));
    }
    state
        .threads
        .thread_store
        .set(workflow_thread_id, record)
        .await;
    state.invalidate_gateway_sync_caches().await;
}

pub(super) fn workflow_argument_string(input: &Value) -> String {
    input
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| {
            input
                .get("question")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| input.to_string())
}
