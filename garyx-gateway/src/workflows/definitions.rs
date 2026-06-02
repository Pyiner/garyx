use super::*;

pub fn workflow_definitions_root_for_config(config: &GaryxConfig) -> PathBuf {
    let data_dir = config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(default_session_data_dir);
    if data_dir.file_name().and_then(|name| name.to_str()) == Some("data") {
        data_dir
            .parent()
            .map(|parent| parent.join("workflows"))
            .unwrap_or_else(|| data_dir.join("workflows"))
    } else {
        data_dir.join("workflows")
    }
}

pub fn get_workflow_definition_package(
    config: &GaryxConfig,
    workflow_id: &str,
) -> Result<WorkflowDefinitionPackage, WorkflowError> {
    let workflow_id = workflow_id.trim();
    if workflow_id.is_empty() {
        return Err(WorkflowError::BadRequest(
            "workflow_id is required".to_owned(),
        ));
    }
    let root = workflow_definitions_root_for_config(config);
    if !root.exists() {
        return Err(WorkflowError::NotFound(format!(
            "workflow definition not found: {workflow_id}"
        )));
    }

    let preferred_package_dir = root.join(workflow_package_dir_name(workflow_id));
    if preferred_package_dir.join(WORKFLOW_MANIFEST_FILE).is_file() {
        let package = read_workflow_definition_package(preferred_package_dir.clone())?;
        if package.record.workflow_id == workflow_id {
            return Ok(package);
        }
    }

    for package_dir in workflow_definition_package_dirs(&root)? {
        if package_dir == preferred_package_dir {
            continue;
        }
        match read_workflow_definition_package(package_dir.clone()) {
            Ok(package) if package.record.workflow_id == workflow_id => return Ok(package),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    package_dir = %package_dir.display(),
                    error = %error,
                    "skipping invalid workflow definition package while searching"
                );
            }
        }
    }

    Err(WorkflowError::NotFound(format!(
        "workflow definition not found: {workflow_id}"
    )))
}

pub(super) fn list_workflow_definition_packages(
    config: &GaryxConfig,
) -> Result<Vec<WorkflowDefinitionPackage>, WorkflowError> {
    let root = workflow_definitions_root_for_config(config);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut packages = Vec::new();
    for package_dir in workflow_definition_package_dirs(&root)? {
        match read_workflow_definition_package(package_dir.clone()) {
            Ok(package) => packages.push(package),
            Err(error) => {
                tracing::warn!(
                    package_dir = %package_dir.display(),
                    error = %error,
                    "skipping invalid workflow definition package"
                );
            }
        }
    }
    Ok(packages)
}

fn workflow_definition_package_dirs(root: &FsPath) -> Result<Vec<PathBuf>, WorkflowError> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root).map_err(workflow_io_error)? {
        let entry = entry.map_err(workflow_io_error)?;
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let package_dir = entry.path();
        if !package_dir.join(WORKFLOW_MANIFEST_FILE).is_file() {
            continue;
        }
        dirs.push(package_dir);
    }
    Ok(dirs)
}

fn workflow_package_dir_name(workflow_id: &str) -> String {
    let mut output = workflow_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    output = output.trim_matches('-').to_owned();
    if output.is_empty() {
        "workflow".to_owned()
    } else {
        output
    }
}

fn read_workflow_definition_package(
    package_dir: PathBuf,
) -> Result<WorkflowDefinitionPackage, WorkflowError> {
    let manifest_path = package_dir.join(WORKFLOW_MANIFEST_FILE);
    let raw = fs::read_to_string(&manifest_path).map_err(workflow_io_error)?;
    let manifest: Value = serde_json::from_str(&raw).map_err(|error| {
        WorkflowError::BadRequest(format!(
            "invalid workflow manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    workflow_definition_package_from_manifest(manifest, package_dir, Some(&manifest_path))
}

pub(super) fn workflow_definition_package_from_manifest(
    manifest: Value,
    package_dir: PathBuf,
    manifest_path: Option<&FsPath>,
) -> Result<WorkflowDefinitionPackage, WorkflowError> {
    let workflow_id = json_string_field(&manifest, "workflowId")
        .or_else(|| json_string_field(&manifest, "workflow_id"))
        .ok_or_else(|| WorkflowError::BadRequest("workflowId is required".to_owned()))?;
    let version = manifest.get("version").and_then(Value::as_u64).unwrap_or(1);
    if version == 0 {
        return Err(WorkflowError::BadRequest(
            "workflow definition version must be greater than zero".to_owned(),
        ));
    }
    let name = json_string_field(&manifest, "name")
        .ok_or_else(|| WorkflowError::BadRequest("name is required".to_owned()))?;
    let description = json_string_field(&manifest, "description");
    let input = manifest.get("input").cloned().unwrap_or_else(|| json!({}));
    validate_json_size("input", &input, MAX_SCHEMA_BYTES)?;
    validate_workflow_entrypoint_file_for_package(&package_dir)?;
    let defaults = manifest
        .get("defaults")
        .cloned()
        .unwrap_or_else(|| json!({}));
    validate_json_size("defaults", &defaults, MAX_SCHEMA_BYTES)?;
    let updated_at = manifest_path
        .and_then(|path| path.metadata().ok())
        .and_then(|metadata| metadata.modified().ok())
        .map(rfc3339_from_system_time)
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
    Ok(WorkflowDefinitionPackage {
        record: WorkflowDefinitionRecord {
            workflow_id,
            version,
            name,
            description,
            input_json: input.to_string(),
            defaults_json: defaults.to_string(),
            created_at: updated_at.clone(),
            updated_at,
        },
        package_dir,
    })
}

fn rfc3339_from_system_time(time: SystemTime) -> String {
    let datetime: chrono::DateTime<Utc> = time.into();
    datetime.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(super) fn workflow_io_error(error: std::io::Error) -> WorkflowError {
    WorkflowError::BadRequest(error.to_string())
}

pub(super) fn workflow_entrypoint_path(package_dir: &FsPath) -> PathBuf {
    package_dir.join(WORKFLOW_ENTRYPOINT_FILE)
}

fn validate_workflow_entrypoint_file_for_package(
    package_dir: &FsPath,
) -> Result<(), WorkflowError> {
    let entrypoint = workflow_entrypoint_path(package_dir);
    if !entrypoint.is_file() {
        return Err(WorkflowError::BadRequest(format!(
            "{WORKFLOW_ENTRYPOINT_FILE} is required in workflow package"
        )));
    }
    Ok(())
}

pub(super) struct WorkflowSourceDocument {
    pub(super) relative_path: String,
    pub(super) content: String,
    pub(super) media_type: String,
    pub(super) language: String,
}

pub(super) fn workflow_definition_source(
    package: &WorkflowDefinitionPackage,
) -> Result<WorkflowSourceDocument, WorkflowError> {
    let resolved = workflow_entrypoint_path(&package.package_dir);
    let metadata = resolved.metadata().map_err(workflow_io_error)?;
    if !metadata.is_file() {
        return Err(WorkflowError::NotFound(format!(
            "workflow source not found: {WORKFLOW_ENTRYPOINT_FILE}"
        )));
    }
    if metadata.len() > MAX_WORKFLOW_SOURCE_BYTES {
        return Err(WorkflowError::BadRequest(format!(
            "workflow source is too large: {WORKFLOW_ENTRYPOINT_FILE}"
        )));
    }
    let content = fs::read_to_string(&resolved).map_err(workflow_io_error)?;
    Ok(WorkflowSourceDocument {
        relative_path: format!("./{WORKFLOW_ENTRYPOINT_FILE}"),
        content,
        media_type: "text/typescript".to_owned(),
        language: "typescript".to_owned(),
    })
}
