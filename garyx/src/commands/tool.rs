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

const TOOL_SEARCH_GEMINI_MODEL: &str = "gemini-3-flash-preview";

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SearchSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SearchToolMetadata {
    tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sources: Vec<SearchSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SearchStreamState {
    pub(crate) answer: String,
    #[cfg(test)]
    pub(crate) thread_id: Option<String>,
    #[cfg(test)]
    pub(crate) run_id: Option<String>,
    pub(crate) searched: bool,
    pub(crate) sources: Vec<SearchSource>,
    pub(crate) tool_metadata: Vec<SearchToolMetadata>,
}

#[derive(Debug, Clone, Serialize)]
struct SearchCommandOutput {
    ok: bool,
    query: String,
    answer: String,
    sources: Vec<SearchSource>,
    runtime_thread_id: String,
    run_id: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    searched: bool,
    tool_metadata: Vec<SearchToolMetadata>,
}

#[derive(Debug, Default)]
struct GeminiCliSearchSummary {
    session_id: Option<String>,
    model: Option<String>,
    status: Option<String>,
    duration_ms: Option<u64>,
}

fn build_gemini_search_prompt(query: &str) -> String {
    format!(
        "You are handling `garyx tool search`.\n\n\
You must use Gemini CLI's provider-native `google_web_search` tool for this request. \
Do not use Garyx MCP `search`, do not call any Garyx MCP web search helper, \
and do not answer only from memory. The tool call is mandatory even if you already know the answer.\n\n\
After the search tool returns, write a concise answer followed by source citations.\n\n\
<user_query_verbatim>\n{query}\n</user_query_verbatim>"
    )
}

fn gemini_search_policy_text() -> &'static str {
    r#"[[rule]]
toolName = "*"
decision = "deny"
priority = 900
interactive = false

[[rule]]
toolName = "google_web_search"
decision = "allow"
priority = 999
interactive = false
"#
}

#[derive(Debug)]
struct TemporaryGeminiSearchPolicy {
    path: PathBuf,
    dir: PathBuf,
}

impl TemporaryGeminiSearchPolicy {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryGeminiSearchPolicy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

async fn write_gemini_search_policy() -> io::Result<TemporaryGeminiSearchPolicy> {
    let dir = std::env::temp_dir().join(format!("garyx-gemini-search-policy-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("search-tool-only-policy.toml");
    tokio::fs::write(&path, gemini_search_policy_text()).await?;
    Ok(TemporaryGeminiSearchPolicy { path, dir })
}

fn strip_source_url_punctuation(url: &str) -> &str {
    url.trim_matches(|ch: char| matches!(ch, ')' | ']' | '}' | '>' | '.' | ',' | ';' | ':'))
}

fn push_source_unique(sources: &mut Vec<SearchSource>, source: SearchSource) {
    if source.url.trim().is_empty() || sources.iter().any(|item| item.url == source.url) {
        return;
    }
    sources.push(source);
}

pub(crate) fn extract_search_sources_from_text(text: &str) -> Vec<SearchSource> {
    let mut sources = Vec::new();
    let mut remainder = text;
    while let Some(open) = remainder.find('[') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find("](") else {
            remainder = after_open;
            continue;
        };
        let title = after_open[..close].trim();
        let after_url = &after_open[close + 2..];
        let Some(end) = after_url.find(')') else {
            break;
        };
        let url = strip_source_url_punctuation(after_url[..end].trim());
        if url.starts_with("http://") || url.starts_with("https://") {
            push_source_unique(
                &mut sources,
                SearchSource {
                    title: (!title.is_empty()).then(|| title.to_owned()),
                    url: url.to_owned(),
                },
            );
        }
        remainder = &after_url[end + 1..];
    }

    for raw in text.split_whitespace() {
        let Some(start) = raw.find("http://").or_else(|| raw.find("https://")) else {
            continue;
        };
        let url = strip_source_url_punctuation(&raw[start..]);
        if url.starts_with("http://") || url.starts_with("https://") {
            push_source_unique(
                &mut sources,
                SearchSource {
                    title: None,
                    url: url.to_owned(),
                },
            );
        }
    }
    sources
}

#[cfg(test)]
fn value_search_sources(value: &Value) -> Vec<SearchSource> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item
                        .get("url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    Some(SearchSource {
                        title: item
                            .get("title")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned),
                        url: url.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_search_like_tool_name(tool_name: &str) -> bool {
    let lower = tool_name.to_ascii_lowercase();
    lower.contains("google_web_search")
        || lower.contains("web_search")
        || lower.contains("google search")
        || lower.contains("search")
}

#[cfg(test)]
pub(crate) fn apply_search_stream_event(state: &mut SearchStreamState, event: &Value) {
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    if let Some(thread_id) = event
        .get("threadId")
        .or_else(|| event.get("thread_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        state.thread_id = Some(thread_id.to_owned());
    }
    if let Some(run_id) = event
        .get("runId")
        .or_else(|| event.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        state.run_id = Some(run_id.to_owned());
    }

    match event_type {
        "committed_message" => {
            let Some(message) = event.get("message") else {
                return;
            };
            match message.get("role").and_then(Value::as_str).unwrap_or("") {
                "assistant" => {
                    if let Some(text) = message
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| message.get("content").and_then(Value::as_str))
                    {
                        state.answer.push_str(text);
                    }
                }
                "tool_use" | "tool_result" => apply_search_tool_message(state, message),
                _ => {}
            }
        }
        "tool_use" | "tool_result" => {
            let Some(message) = event.get("message") else {
                return;
            };
            apply_search_tool_message(state, message);
        }
        _ => {}
    }
}

#[cfg(test)]
fn apply_search_tool_message(state: &mut SearchStreamState, message: &Value) {
    let tool_name = message
        .get("tool_name")
        .and_then(Value::as_str)
        .or_else(|| message.get("toolName").and_then(Value::as_str))
        .or_else(|| {
            message
                .get("content")
                .and_then(|content| content.get("rawInput"))
                .and_then(|raw| raw.get("name"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            message
                .get("content")
                .and_then(|content| content.get("title"))
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    let search_metadata = message
        .get("metadata")
        .and_then(|metadata| metadata.get("gemini_search"));
    if is_search_like_tool_name(tool_name) || search_metadata.is_some() {
        state.searched = true;
    }
    let Some(search_metadata) = search_metadata else {
        return;
    };
    let mut sources = value_search_sources(&search_metadata["sources"]);
    if sources.is_empty()
        && let Some(output) = search_metadata.get("output").and_then(Value::as_str)
    {
        sources = extract_search_sources_from_text(output);
    }
    for source in &sources {
        push_source_unique(&mut state.sources, source.clone());
    }
    state.tool_metadata.push(SearchToolMetadata {
        tool_name: tool_name.to_owned(),
        tool_use_id: message
            .get("tool_use_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        sources,
        output: search_metadata
            .get("output")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    });
}

fn apply_gemini_cli_search_event(
    state: &mut SearchStreamState,
    summary: &mut GeminiCliSearchSummary,
    event: &Value,
) {
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "init" => {
            summary.session_id = event
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            summary.model = event
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
        }
        "tool_use" => {
            let tool_name = event.get("tool_name").and_then(Value::as_str).unwrap_or("");
            if is_search_like_tool_name(tool_name) {
                state.searched = true;
                state.tool_metadata.push(SearchToolMetadata {
                    tool_name: tool_name.to_owned(),
                    tool_use_id: event
                        .get("tool_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    sources: Vec::new(),
                    output: None,
                });
            }
        }
        "tool_result" => {
            let tool_id = event
                .get("tool_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let output = event
                .get("output")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if let (Some(existing), Some(output)) = (
                tool_id.as_deref().and_then(|id| {
                    state
                        .tool_metadata
                        .iter_mut()
                        .find(|item| item.tool_use_id.as_deref() == Some(id))
                }),
                output,
            ) {
                existing.output = Some(output);
            }
        }
        "message" => {
            if event.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(content) = event.get("content").and_then(Value::as_str)
            {
                state.answer.push_str(content);
            }
        }
        "result" => {
            summary.status = event
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let stats = event.get("stats").and_then(Value::as_object);
            summary.duration_ms = stats
                .and_then(|stats| stats.get("duration_ms"))
                .and_then(Value::as_u64);
            if let Some(tool_calls) = stats
                .and_then(|stats| stats.get("tool_calls"))
                .and_then(Value::as_u64)
                && tool_calls > 0
            {
                state.searched = true;
            }
        }
        _ => {}
    }
}

fn sanitize_gemini_cli_stderr(stderr: &str) -> String {
    stderr
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("authorization")
                || lower.contains("access_token")
                || lower.contains("refresh_token")
                || lower.contains("credential")
                || lower.contains("api key")
                || lower.contains("apikey")
            {
                "[redacted sensitive stderr line]".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn run_gemini_cli_search(
    query: &str,
    timeout_secs: u64,
) -> Result<SearchCommandOutput, Box<dyn std::error::Error>> {
    let workspace_dir = tool_workspace_dir("search")?;
    let policy = write_gemini_search_policy().await?;
    let run_id = format!("tool-run-{}", Uuid::new_v4());
    let mut command = Command::new("gemini");
    command
        .current_dir(&workspace_dir)
        .kill_on_drop(true)
        .arg("--approval-mode")
        .arg("yolo")
        .arg("--model")
        .arg(TOOL_SEARCH_GEMINI_MODEL)
        .arg("--policy")
        .arg(policy.path())
        .arg("--output-format")
        .arg("stream-json")
        .arg("-p")
        .arg(build_gemini_search_prompt(query))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match tokio::time::timeout(Duration::from_secs(timeout_secs), command.output())
        .await
    {
        Ok(output) => output?,
        Err(_) => {
            return Err(
                format!("timed out after {timeout_secs}s waiting for Gemini CLI search").into(),
            );
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let sanitized = sanitize_gemini_cli_stderr(&stderr);
        return Err(format!(
            "Gemini CLI search failed with status {}{}",
            output.status,
            if sanitized.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", sanitized.trim())
            }
        )
        .into());
    }

    let mut state = SearchStreamState::default();
    let mut summary = GeminiCliSearchSummary::default();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let event: Value = serde_json::from_str(line)
            .map_err(|error| format!("Gemini CLI emitted malformed stream JSON: {error}"))?;
        apply_gemini_cli_search_event(&mut state, &mut summary, &event);
    }

    let answer = state.answer.trim().to_owned();
    if summary
        .status
        .as_deref()
        .is_some_and(|status| status != "success")
    {
        return Err(format!(
            "Gemini CLI search finished with status {}",
            summary.status.as_deref().unwrap_or("unknown")
        )
        .into());
    }
    if !state.searched {
        return Err("Gemini completed without using provider-native search".into());
    }
    if answer.is_empty() {
        return Err("Gemini returned no answer".into());
    }
    if state.sources.is_empty() {
        state.sources = extract_search_sources_from_text(&answer);
    }

    let runtime_thread_id = summary
        .session_id
        .as_deref()
        .map(|session_id| format!("gemini-cli::{session_id}"))
        .unwrap_or_else(|| format!("tool::search::{}", Uuid::new_v4()));
    Ok(SearchCommandOutput {
        ok: true,
        query: query.to_owned(),
        answer,
        sources: state.sources,
        runtime_thread_id,
        run_id,
        model: summary
            .model
            .unwrap_or_else(|| TOOL_SEARCH_GEMINI_MODEL.to_owned()),
        duration_ms: summary.duration_ms,
        searched: state.searched,
        tool_metadata: state.tool_metadata,
    })
}

pub(crate) async fn cmd_tool_search(
    _config_path: &str,
    query_parts: Vec<String>,
    json_output: bool,
    timeout_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let query = query_parts.join(" ").trim().to_owned();
    if query.is_empty() {
        return Err("query cannot be empty".into());
    }
    if timeout_secs == 0 {
        return Err("timeout must be greater than zero".into());
    }

    let output = run_gemini_cli_search(&query, timeout_secs).await?;

    if json_output {
        return print_pretty_json(&serde_json::to_value(output)?);
    }

    println!("{}", output.answer);
    if output.sources.is_empty() {
        println!("\nSources: (none returned by provider; no URLs found in final answer)");
    } else {
        println!("\nSources:");
        for source in output.sources {
            match source.title.as_deref() {
                Some(title) => println!("- {title}: {}", source.url),
                None => println!("- {}", source.url),
            }
        }
    }
    Ok(())
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

        let workspace = tool_workspace_dir("search").expect("workspace");

        assert_eq!(
            workspace,
            home.path()
                .join(".garyx")
                .join("tool-workspaces")
                .join("search")
        );
        assert!(workspace.is_dir());
    }

    #[test]
    fn tool_search_uses_fixed_gemini_flash_model() {
        assert_eq!(TOOL_SEARCH_GEMINI_MODEL, "gemini-3-flash-preview");
    }

    #[test]
    fn gemini_search_policy_allows_only_google_web_search() {
        let policy = gemini_search_policy_text();
        assert!(policy.contains("toolName = \"*\""));
        assert!(policy.contains("decision = \"deny\""));
        assert!(policy.contains("toolName = \"google_web_search\""));
        assert!(policy.contains("decision = \"allow\""));
    }

    #[tokio::test]
    async fn gemini_search_policy_is_temporary_and_removed_on_drop() {
        let policy = write_gemini_search_policy()
            .await
            .expect("temporary policy");
        let path = policy.path().to_owned();
        let dir = path.parent().expect("policy dir").to_owned();

        assert!(path.starts_with(std::env::temp_dir()));
        assert!(path.ends_with("search-tool-only-policy.toml"));
        assert_eq!(
            tokio::fs::read_to_string(&path).await.expect("policy text"),
            gemini_search_policy_text()
        );

        drop(policy);

        assert!(!path.exists(), "temporary policy file should be removed");
        assert!(
            !dir.exists(),
            "temporary policy directory should be removed"
        );
    }

    #[test]
    fn gemini_cli_search_event_parser_requires_tool_use_not_direct_answer() {
        let mut state = SearchStreamState::default();
        let mut summary = GeminiCliSearchSummary::default();

        apply_gemini_cli_search_event(
            &mut state,
            &mut summary,
            &json!({
                "type": "init",
                "session_id": "session-1",
                "model": "gemini-3-flash-preview"
            }),
        );
        apply_gemini_cli_search_event(
            &mut state,
            &mut summary,
            &json!({
                "type": "message",
                "role": "assistant",
                "content": "Direct answer without a tool."
            }),
        );

        assert!(!state.searched);
        assert_eq!(state.answer, "Direct answer without a tool.");
        assert_eq!(summary.session_id.as_deref(), Some("session-1"));
        assert_eq!(summary.model.as_deref(), Some("gemini-3-flash-preview"));
    }

    #[test]
    fn gemini_cli_search_event_parser_collects_tool_answer_and_stats() {
        let mut state = SearchStreamState::default();
        let mut summary = GeminiCliSearchSummary::default();

        apply_gemini_cli_search_event(
            &mut state,
            &mut summary,
            &json!({
                "type": "tool_use",
                "tool_name": "google_web_search",
                "tool_id": "google_web_search_1",
                "parameters": { "query": "example" }
            }),
        );
        apply_gemini_cli_search_event(
            &mut state,
            &mut summary,
            &json!({
                "type": "tool_result",
                "tool_id": "google_web_search_1",
                "status": "success",
                "output": "Search results returned."
            }),
        );
        apply_gemini_cli_search_event(
            &mut state,
            &mut summary,
            &json!({
                "type": "message",
                "role": "assistant",
                "content": "Answer with [Source](https://example.test/source)."
            }),
        );
        apply_gemini_cli_search_event(
            &mut state,
            &mut summary,
            &json!({
                "type": "result",
                "status": "success",
                "stats": {
                    "duration_ms": 1234,
                    "tool_calls": 1
                }
            }),
        );

        assert!(state.searched);
        assert_eq!(
            state.answer,
            "Answer with [Source](https://example.test/source)."
        );
        assert_eq!(summary.status.as_deref(), Some("success"));
        assert_eq!(summary.duration_ms, Some(1234));
        assert_eq!(state.tool_metadata.len(), 1);
        assert_eq!(state.tool_metadata[0].tool_name, "google_web_search");
        assert_eq!(
            state.tool_metadata[0].tool_use_id.as_deref(),
            Some("google_web_search_1")
        );
        assert_eq!(
            state.tool_metadata[0].output.as_deref(),
            Some("Search results returned.")
        );
    }

    #[test]
    fn gemini_cli_stderr_sanitizer_redacts_sensitive_lines() {
        let stderr = "safe warning\nAuthorization: Bearer secret\nrefresh_token=secret";
        let sanitized = sanitize_gemini_cli_stderr(stderr);
        assert!(sanitized.contains("safe warning"));
        assert!(!sanitized.contains("Bearer secret"));
        assert!(!sanitized.contains("refresh_token=secret"));
        assert!(sanitized.contains("[redacted sensitive stderr line]"));
    }

    #[test]
    fn search_stream_event_does_not_count_direct_answer_as_search() {
        let mut state = SearchStreamState::default();

        apply_search_stream_event(
            &mut state,
            &json!({
                "type": "committed_message",
                "thread_id": "thread::search",
                "run_id": "run-search",
                "seq": 1,
                "message": {
                    "role": "assistant",
                    "text": "I can answer this from memory without searching."
                }
            }),
        );

        assert!(!state.searched);
        assert_eq!(
            state.answer,
            "I can answer this from memory without searching."
        );
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
