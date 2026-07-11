use super::*;

#[derive(Debug, Clone)]
struct ImageGenerationCliResult {
    path: PathBuf,
    bytes: usize,
    media_type: Option<String>,
    runtime_thread_id: String,
    run_id: String,
    extra_images_seen: bool,
}

fn tool_workspace_dir(tool_name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = gary_home_dir().join("tool-workspaces").join(tool_name);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImageGenerationEventError {
    MalformedPayload(String),
}

impl std::fmt::Display for ImageGenerationEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedPayload(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ImageGenerationEventError {}

#[derive(Debug)]
struct ToolProviderRun {
    runtime_thread_id: String,
    run_id: String,
    events: Vec<StreamEvent>,
}

async fn run_provider_tool(
    config_path: &str,
    provider_type: ProviderType,
    tool_name: &str,
    message: String,
    timeout_secs: u64,
    metadata: HashMap<String, Value>,
) -> Result<ToolProviderRun, Box<dyn std::error::Error>> {
    if timeout_secs == 0 {
        return Err("timeout must be greater than 0 seconds".into());
    }

    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let bridge = MultiProviderBridge::new();
    bridge.initialize_from_config(&loaded.config).await?;

    let workspace_dir = tool_workspace_dir(tool_name)?;
    let runtime_thread_id = format!("tool::{tool_name}::{}", Uuid::new_v4());
    let run_id = format!("tool-run-{}", Uuid::new_v4());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        let _ = tx.send(event);
    });

    let request = AgentRunRequest::new(
        runtime_thread_id.clone(),
        message,
        run_id.clone(),
        "tool",
        tool_name,
        metadata,
    )
    .with_workspace_dir(Some(workspace_dir.to_string_lossy().into_owned()))
    .with_requested_provider(Some(provider_type));

    if let Err(error) = bridge.start_agent_run(request, Some(callback)).await {
        bridge.shutdown().await;
        return Err(error.into());
    }

    let deadline = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(deadline);
    let mut events = Vec::new();

    loop {
        tokio::select! {
            _ = &mut deadline => {
                let _ = bridge.abort_run(&run_id).await;
                bridge.shutdown().await;
                return Err(format!("timed out after {timeout_secs}s waiting for provider tool `{tool_name}`").into());
            }
            event = rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                let done = matches!(event, StreamEvent::Done);
                events.push(event);
                if done {
                    break;
                }
            }
        }
    }

    bridge.shutdown().await;
    Ok(ToolProviderRun {
        runtime_thread_id,
        run_id,
        events,
    })
}

pub(crate) async fn cmd_tool_image(
    config_path: &str,
    prompt: String,
    output: PathBuf,
    timeout_secs: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = run_tool_image(config_path, &prompt, output, timeout_secs).await?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "path": result.path.display().to_string(),
                "bytes": result.bytes,
                "media_type": result.media_type,
                "runtime_thread_id": result.runtime_thread_id,
                "run_id": result.run_id,
                "extra_images_seen": result.extra_images_seen,
            }))?
        );
        return Ok(());
    }

    println!("Saved image: {}", result.path.display());
    println!("Bytes: {}", result.bytes);
    if let Some(media_type) = result.media_type.as_deref() {
        println!("Media type: {media_type}");
    }
    println!("Runtime thread: {}", result.runtime_thread_id);
    println!("Run: {}", result.run_id);
    if result.extra_images_seen {
        println!("Extra images were generated and ignored.");
    }
    Ok(())
}

async fn run_tool_image(
    config_path: &str,
    prompt: &str,
    output: PathBuf,
    timeout_secs: u64,
) -> Result<ImageGenerationCliResult, Box<dyn std::error::Error>> {
    let provider_run = run_provider_tool(
        config_path,
        ProviderType::CodexAppServer,
        "image",
        build_image_generation_prompt(prompt),
        timeout_secs,
        HashMap::from([("source".to_owned(), json!("garyx_tool_image"))]),
    )
    .await?;
    let mut first_image: Option<GeneratedImageResult> = None;
    let mut extra_images_seen = false;

    for event in &provider_run.events {
        if let Some(image) = extract_image_from_stream_event(event)? {
            if first_image.is_some() {
                extra_images_seen = true;
            } else {
                first_image = Some(image);
            }
        }
    }

    let image = first_image.ok_or("CodeX completed without generating an image")?;
    let output = resolve_image_output_path(output, image.extension);
    write_generated_image_output(&output, &image.bytes).await?;
    Ok(ImageGenerationCliResult {
        path: output,
        bytes: image.bytes.len(),
        media_type: image.media_type,
        runtime_thread_id: provider_run.runtime_thread_id,
        run_id: provider_run.run_id,
        extra_images_seen,
    })
}

fn extract_image_from_tool_result_message(
    message: &ProviderMessage,
) -> Result<Option<GeneratedImageResult>, ImageGenerationEventError> {
    if provider_message_item_type(message) != Some("imageGeneration") {
        return Ok(None);
    }
    let result = message
        .content
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if result.is_empty() {
        return Ok(None);
    }
    extract_image_generation_result(message)
        .map(Some)
        .ok_or_else(|| {
            ImageGenerationEventError::MalformedPayload(
                "generated image payload was malformed or not valid base64".to_owned(),
            )
        })
}

fn extract_image_from_stream_event(
    event: &StreamEvent,
) -> Result<Option<GeneratedImageResult>, ImageGenerationEventError> {
    match event {
        StreamEvent::ToolResult { message } => extract_image_from_tool_result_message(message),
        _ => Ok(None),
    }
}

fn resolve_image_output_path(output: PathBuf, extension: &str) -> PathBuf {
    if output.extension().is_some() {
        output
    } else {
        output.with_extension(extension)
    }
}

async fn write_generated_image_output(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use tempfile::tempdir;

    #[test]
    fn image_generation_prompt_preserves_user_prompt() {
        let user_prompt = "first line\nsecond line with [brackets]";
        let framed = build_image_generation_prompt(user_prompt);
        assert!(framed.contains("Generate exactly one image"));
        assert!(framed.contains("Do not merely describe an image"));
        assert!(framed.contains(user_prompt));
    }

    #[test]
    fn tool_workspace_dir_uses_hidden_garyx_home_and_creates_directory() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let home = tempdir().expect("home");
        let _home = ScopedEnvVar::set_path("HOME", home.path());

        let workspace = tool_workspace_dir("image").expect("workspace");

        assert_eq!(
            workspace,
            home.path()
                .join(".garyx")
                .join("tool-workspaces")
                .join("image")
        );
        assert!(workspace.is_dir());
    }

    #[test]
    fn extract_image_from_synthetic_tool_result_event() {
        let event = StreamEvent::ToolResult {
            message: ProviderMessage::tool_result(
                json!({
                    "type": "imageGeneration",
                    "id": "img_one",
                    "media_type": "image/png",
                    "result": "aGVsbG8="
                }),
                Some("img_one".to_owned()),
                Some("imageGeneration".to_owned()),
                Some(false),
            )
            .with_metadata_value("item_type", json!("imageGeneration")),
        };

        let image = extract_image_from_stream_event(&event)
            .expect("event parse")
            .expect("image");
        assert_eq!(image.bytes, b"hello");
        assert_eq!(image.extension, "png");
        assert_eq!(image.media_type.as_deref(), Some("image/png"));
    }

    #[test]
    fn extract_image_from_synthetic_tool_result_event_rejects_malformed_base64() {
        let event = StreamEvent::ToolResult {
            message: ProviderMessage::tool_result(
                json!({
                    "type": "imageGeneration",
                    "id": "img_bad",
                    "result": "not valid base64"
                }),
                Some("img_bad".to_owned()),
                Some("imageGeneration".to_owned()),
                Some(false),
            )
            .with_metadata_value("item_type", json!("imageGeneration")),
        };

        let error = extract_image_from_stream_event(&event).expect_err("malformed image");
        assert!(error.to_string().contains("malformed"));
    }

    #[test]
    fn resolve_image_output_path_adds_extension_when_missing() {
        assert_eq!(
            resolve_image_output_path(PathBuf::from("/tmp/generated-image"), "webp"),
            PathBuf::from("/tmp/generated-image.webp")
        );
        assert_eq!(
            resolve_image_output_path(PathBuf::from("/tmp/generated-image.png"), "webp"),
            PathBuf::from("/tmp/generated-image.png")
        );
    }
}
