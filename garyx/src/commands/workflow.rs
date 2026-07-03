use super::*;

pub(crate) async fn cmd_workflow_definition_list(
    config_path: &str,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/workflow-definitions?limit={limit}&offset={offset}"),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    let definitions = payload["workflowDefinitions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if definitions.is_empty() {
        println!("No workflow definitions.");
        return Ok(());
    }
    println!("{:<34}  {:<7}  NAME", "WORKFLOW ID", "VERSION");
    println!("{}", "-".repeat(90));
    for definition in definitions {
        println!(
            "{:<34}  {:<7}  {}",
            definition["workflowId"].as_str().unwrap_or("-"),
            definition["version"].as_u64().unwrap_or_default(),
            definition["name"].as_str().unwrap_or("-")
        );
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_definition_get(
    config_path: &str,
    workflow_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_id, "workflow_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/workflow-definitions/{}",
            urlencoding::encode(&workflow_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_definition_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_workflow_definition_upsert(
    config_path: &str,
    file: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = PathBuf::from(file);
    let (package_dir, manifest_path) = workflow_package_source(&manifest_path)?;
    let raw = std::fs::read_to_string(&manifest_path)?;
    let body: Value = serde_json::from_str(&raw)?;
    let workflow_id = body
        .get("workflowId")
        .or_else(|| body.get("workflow_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "workflowId is required"))?
        .to_owned();
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let root = workflow_definitions_root_for_config(&config);
    let destination = root.join(workflow_package_dir_name(&workflow_id));
    install_workflow_package(&package_dir, &destination)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/workflow-definitions/{}",
            urlencoding::encode(&workflow_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_definition_summary(&payload);
    Ok(())
}

const WORKFLOW_MANIFEST_FILE: &str = "garyx.workflow.json";

fn workflow_package_source(path: &Path) -> io::Result<(PathBuf, PathBuf)> {
    if path.is_dir() {
        let manifest = path.join(WORKFLOW_MANIFEST_FILE);
        if !manifest.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("workflow package is missing {WORKFLOW_MANIFEST_FILE}"),
            ));
        }
        return Ok((path.to_path_buf(), manifest));
    }
    let package_dir = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "workflow manifest must have a parent directory",
        )
    })?;
    Ok((package_dir.to_path_buf(), path.to_path_buf()))
}

fn workflow_definitions_root_for_config(config: &GaryxConfig) -> PathBuf {
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

fn install_workflow_package(source: &Path, destination: &Path) -> io::Result<()> {
    let source = source.canonicalize()?;
    let destination_parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "workflow package destination must have a parent",
        )
    })?;
    fs::create_dir_all(destination_parent)?;
    let destination_canonical = destination.canonicalize().ok();
    if destination_canonical.as_deref() == Some(source.as_path()) {
        return Ok(());
    }
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    copy_dir_all(&source, destination)
}

fn copy_dir_all(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_list(
    config_path: &str,
    parent_thread_id: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let path = if let Some(thread_id) = trim_optional_cli(parent_thread_id) {
        format!(
            "/api/workflows?parentThreadId={}",
            urlencoding::encode(&thread_id)
        )
    } else {
        "/api/workflows".to_owned()
    };
    let payload = fetch_gateway_json(&gateway, &path).await?;
    if json {
        return print_pretty_json(&payload);
    }
    let workflows = payload["workflows"].as_array().cloned().unwrap_or_default();
    if workflows.is_empty() {
        println!("Workflows: (none)");
        return Ok(());
    }
    println!("{:<42}  {:<10}  {:<20}  NAME", "RUN ID", "STATUS", "PARENT");
    println!("{}", "-".repeat(100));
    for workflow in workflows {
        let workflow_id = workflow["workflowRunId"]
            .as_str()
            .or_else(|| workflow["workflowId"].as_str())
            .unwrap_or("-");
        let status = workflow["status"].as_str().unwrap_or("-");
        let parent = workflow["parentThreadId"].as_str().unwrap_or("-");
        let name = workflow["name"].as_str().unwrap_or("-");
        println!("{workflow_id:<42}  {status:<10}  {parent:<20}  {name}");
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_get(
    config_path: &str,
    workflow_run_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_run_id, "workflow_run_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/workflows/{}", urlencoding::encode(&workflow_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_workflow_events(
    config_path: &str,
    workflow_run_id: &str,
    after: u64,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_run_id, "workflow_run_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/workflows/{}/events?after={after}",
            urlencoding::encode(&workflow_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    for event in payload["events"].as_array().cloned().unwrap_or_default() {
        let seq = event["eventSeq"].as_u64().unwrap_or_default();
        let typ = event["eventType"].as_str().unwrap_or("-");
        let child = event["workflowChildRunId"].as_str().unwrap_or("-");
        println!("{seq:<8}  {typ:<28}  {child}");
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_cancel(
    config_path: &str,
    workflow_run_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_run_id, "workflow_run_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        &format!(
            "/api/workflows/{}/cancel",
            urlencoding::encode(&workflow_id)
        ),
        &json!({}),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_summary(&payload);
    Ok(())
}

fn print_workflow_summary(payload: &Value) {
    let workflow = payload.get("workflow").unwrap_or(payload);
    println!(
        "Workflow Run: {}",
        workflow["workflowRunId"]
            .as_str()
            .or_else(|| workflow["workflowId"].as_str())
            .unwrap_or("-")
    );
    println!("Name: {}", workflow["name"].as_str().unwrap_or("-"));
    println!("Status: {}", workflow["status"].as_str().unwrap_or("-"));
    println!(
        "Parent: {}",
        workflow["parentThreadId"].as_str().unwrap_or("-")
    );
    if let Some(output_text) = workflow["outputText"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        println!("Output: {output_text}");
    }
    if let Some(error) = workflow["error"].as_str().filter(|value| !value.is_empty()) {
        println!("Error: {error}");
    }
    let children = payload["children"].as_array().cloned().unwrap_or_default();
    if !children.is_empty() {
        println!("Children:");
        for child in children {
            println!(
                "- {} [{}] thread {}",
                child["label"].as_str().unwrap_or("-"),
                child["status"].as_str().unwrap_or("-"),
                child["threadId"].as_str().unwrap_or("-"),
            );
        }
    }
}

fn print_workflow_definition_summary(payload: &Value) {
    let definition = payload.get("workflowDefinition").unwrap_or(payload);
    println!(
        "Workflow Definition: {}",
        definition["workflowId"].as_str().unwrap_or("-")
    );
    println!("Name: {}", definition["name"].as_str().unwrap_or("-"));
    println!(
        "Version: {}",
        definition["version"].as_u64().unwrap_or_default()
    );
    if let Some(description) = definition["description"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
    {
        println!("Description: {description}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn workflow_package_install_copies_manifest_and_code_into_config_root() {
        let temp = tempdir().expect("tempdir");
        let package = temp.path().join("source").join("smoke");
        std::fs::create_dir_all(&package).expect("source dirs");
        std::fs::write(package.join("workflow.ts"), "export {};\n").expect("entrypoint");
        std::fs::write(
            package.join(WORKFLOW_MANIFEST_FILE),
            r#"{
          "workflowId": "smoke",
          "version": 1,
          "name": "Smoke",
          "input": {"placeholder": "Smoke request"}
        }"#,
        )
        .expect("manifest");
        let (source, manifest) = workflow_package_source(&package).expect("source");
        assert_eq!(source, package);
        assert_eq!(manifest, package.join(WORKFLOW_MANIFEST_FILE));

        let mut config = GaryxConfig::default();
        config.sessions.data_dir = Some(temp.path().join("data").to_string_lossy().to_string());
        let destination = workflow_definitions_root_for_config(&config).join("smoke");
        install_workflow_package(&source, &destination).expect("install");
        assert!(destination.join(WORKFLOW_MANIFEST_FILE).is_file());
        assert!(destination.join("workflow.ts").is_file());
    }
}
