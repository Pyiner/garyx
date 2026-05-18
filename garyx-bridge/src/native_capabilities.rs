use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use garyx_agent_loop::{LlmToolCall, ToolDefinition};
use garyx_models::config::{McpServerConfig, McpTransport};
use garyx_models::local_paths::default_skills_dir;
use http::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::{
    ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess,
    streamable_http_client::StreamableHttpClientTransportConfig,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::process::Command;

const MAX_SKILL_FILE_CHARS: usize = 32_000;
const MCP_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
struct NativeSkill {
    id: String,
    name: String,
    description: String,
    dir: PathBuf,
    enabled: bool,
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeMcpServerConfig {
    #[serde(default)]
    transport: McpTransport,
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    working_dir: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    bearer_token_env: Option<String>,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default = "default_mcp_enabled")]
    enabled: bool,
}

impl From<NativeMcpServerConfig> for McpServerConfig {
    fn from(config: NativeMcpServerConfig) -> Self {
        Self {
            transport: config.transport,
            command: config.command,
            args: config.args,
            env: config.env,
            working_dir: config.working_dir,
            url: config.url,
            bearer_token_env: config.bearer_token_env,
            headers: config.headers,
            enabled: config.enabled,
        }
    }
}

fn default_mcp_enabled() -> bool {
    true
}

pub(crate) fn capability_instructions(
    _workspace_dir: &Path,
    metadata: &HashMap<String, Value>,
) -> String {
    capability_instructions_for_skill_root(&default_skills_dir(), metadata)
}

fn capability_instructions_for_skill_root(
    skill_root: &Path,
    metadata: &HashMap<String, Value>,
) -> String {
    let mut sections = Vec::new();

    let skills = discover_skills_from_root(skill_root);
    let enabled_skills = skills
        .values()
        .filter(|skill| skill.enabled)
        .collect::<Vec<_>>();
    if !enabled_skills.is_empty() {
        let mut lines = vec![
            "## Garyx Skills".to_owned(),
            "Enabled Garyx skills are available as procedural instructions. When a user explicitly names a skill, or a task clearly matches a skill description, call `load_skill` before applying that skill. If the loaded skill references local files, use `read_skill_file` to read only the needed files inside that skill directory.".to_owned(),
            "Available skills:".to_owned(),
        ];
        for skill in enabled_skills {
            let description = if skill.description.trim().is_empty() {
                "No description.".to_owned()
            } else {
                skill.description.clone()
            };
            lines.push(format!(
                "- `{}` ({}): {}",
                skill.id, skill.name, description
            ));
        }
        sections.push(lines.join("\n"));
    }

    let servers = mcp_servers_from_metadata(metadata);
    if !servers.is_empty() {
        let names = servers.keys().cloned().collect::<Vec<_>>().join(", ");
        sections.push(format!(
            "## MCP Servers\nGaryx-managed MCP servers are available through `list_mcp_tools` and `call_mcp_tool`. List tools before calling an unfamiliar server tool. Available MCP servers: {names}."
        ));
    }

    sections.join("\n\n")
}

pub(crate) fn capability_tool_schemas(
    _workspace_dir: &Path,
    metadata: &HashMap<String, Value>,
) -> Vec<ToolDefinition> {
    capability_tool_schemas_for_skill_root(&default_skills_dir(), metadata)
}

fn capability_tool_schemas_for_skill_root(
    skill_root: &Path,
    metadata: &HashMap<String, Value>,
) -> Vec<ToolDefinition> {
    let mut tools = Vec::new();
    if discover_skills_from_root(skill_root)
        .values()
        .any(|skill| skill.enabled)
    {
        tools.push(ToolDefinition::function(
            "load_skill",
            "Load the SKILL.md instructions for an enabled Garyx skill.",
            json!({
                "type": "object",
                "properties": {
                    "skill_id": {
                        "type": "string",
                        "description": "The skill id from the available skills list."
                    }
                },
                "required": ["skill_id"],
                "additionalProperties": false
            }),
        ));
        tools.push(ToolDefinition::function(
            "read_skill_file",
            "Read a text file inside an enabled Garyx skill directory.",
            json!({
                "type": "object",
                "properties": {
                    "skill_id": {
                        "type": "string",
                        "description": "The skill id from the available skills list."
                    },
                    "path": {
                        "type": "string",
                        "description": "A relative file path inside the skill directory."
                    }
                },
                "required": ["skill_id", "path"],
                "additionalProperties": false
            }),
        ));
    }

    if !mcp_servers_from_metadata(metadata).is_empty() {
        tools.push(ToolDefinition::function(
            "list_mcp_tools",
            "List available tools exposed by Garyx-managed MCP servers.",
            json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "Optional MCP server name. Omit to list all configured servers."
                    }
                },
                "additionalProperties": false
            }),
        ));
        tools.push(ToolDefinition::function(
            "call_mcp_tool",
            "Call a tool on a Garyx-managed MCP server.",
            json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "The MCP server name."
                    },
                    "tool": {
                        "type": "string",
                        "description": "The MCP tool name."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments for the MCP tool.",
                        "additionalProperties": true
                    }
                },
                "required": ["server", "tool"],
                "additionalProperties": false
            }),
        ));
    }

    tools
}

pub(crate) fn is_capability_tool(name: &str) -> bool {
    matches!(
        name,
        "load_skill" | "read_skill_file" | "list_mcp_tools" | "call_mcp_tool"
    )
}

pub(crate) async fn run_capability_tool(
    call: &LlmToolCall,
    workspace_dir: &Path,
    metadata: &HashMap<String, Value>,
    runtime_env: &HashMap<String, String>,
) -> Result<Value, String> {
    match call.name.as_str() {
        "load_skill" => load_skill_tool(call).await,
        "read_skill_file" => read_skill_file_tool(call).await,
        "list_mcp_tools" => list_mcp_tools_tool(call, metadata, workspace_dir, runtime_env).await,
        "call_mcp_tool" => call_mcp_tool_tool(call, metadata, workspace_dir, runtime_env).await,
        _ => Err(format!("unknown capability tool '{}'", call.name)),
    }
}

async fn load_skill_tool(call: &LlmToolCall) -> Result<Value, String> {
    load_skill_tool_from_root(call, &default_skills_dir()).await
}

async fn load_skill_tool_from_root(call: &LlmToolCall, skill_root: &Path) -> Result<Value, String> {
    let skill = resolve_enabled_skill_from_root(call, skill_root)?;
    let skill_md = skill.dir.join("SKILL.md");
    let contents = tokio::fs::read_to_string(&skill_md)
        .await
        .map_err(|error| format!("failed to read skill '{}': {error}", skill.id))?;
    Ok(json!({
        "skill_id": skill.id,
        "name": skill.name,
        "description": skill.description,
        "content": truncate_text(&contents, MAX_SKILL_FILE_CHARS),
    }))
}

async fn read_skill_file_tool(call: &LlmToolCall) -> Result<Value, String> {
    read_skill_file_tool_from_root(call, &default_skills_dir()).await
}

async fn read_skill_file_tool_from_root(
    call: &LlmToolCall,
    skill_root: &Path,
) -> Result<Value, String> {
    let skill = resolve_enabled_skill_from_root(call, skill_root)?;
    let path = call
        .arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing path".to_owned())?;
    let relative_path = safe_relative_path(path)?;
    let target = skill.dir.join(&relative_path);
    let target = resolve_skill_file_path(&skill.dir, &target)?;
    let contents = tokio::fs::read_to_string(&target)
        .await
        .map_err(|error| format!("failed to read skill file '{}': {error}", path))?;
    Ok(json!({
        "skill_id": skill.id,
        "path": relative_path.display().to_string(),
        "content": truncate_text(&contents, MAX_SKILL_FILE_CHARS),
    }))
}

fn resolve_enabled_skill_from_root(
    call: &LlmToolCall,
    skill_root: &Path,
) -> Result<NativeSkill, String> {
    let skill_id = call
        .arguments
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing skill_id".to_owned())?;
    let skill = discover_skills_from_root(skill_root)
        .remove(skill_id)
        .ok_or_else(|| format!("skill '{}' not found", skill_id))?;
    if !skill.enabled {
        return Err(format!("skill '{}' is disabled", skill_id));
    }
    Ok(skill)
}

fn discover_skills_from_root(skill_root: &Path) -> BTreeMap<String, NativeSkill> {
    let state = load_skill_state_from_root(skill_root);
    let mut skills = BTreeMap::new();

    scan_skill_root(skill_root, &state, &mut skills);

    skills
}

fn scan_skill_root(
    root: &Path,
    state: &HashMap<String, bool>,
    skills: &mut BTreeMap<String, NativeSkill>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let id = entry.file_name().to_string_lossy().to_string();
        if !is_valid_skill_id(&id) {
            continue;
        }

        let dir = entry.path();
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let contents = std::fs::read_to_string(&skill_md).unwrap_or_default();
        let frontmatter = parse_skill_frontmatter(&contents);
        skills.insert(
            id.clone(),
            NativeSkill {
                id: id.clone(),
                name: non_empty_or(frontmatter.name.trim(), &id),
                description: frontmatter.description.trim().to_owned(),
                dir,
                enabled: state.get(&id).copied().unwrap_or(true),
            },
        );
    }
}

fn resolve_skill_file_path(skill_dir: &Path, target: &Path) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(skill_dir)
        .map_err(|error| format!("failed to resolve skill directory: {error}"))?;
    let resolved = std::fs::canonicalize(target)
        .map_err(|error| format!("failed to resolve skill file: {error}"))?;
    if !resolved.starts_with(&root) {
        return Err("skill file path cannot leave the skill directory".to_owned());
    }
    Ok(resolved)
}

fn load_skill_state_from_root(skill_root: &Path) -> HashMap<String, bool> {
    std::fs::read_to_string(skill_root.join(".state.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn parse_skill_frontmatter(contents: &str) -> SkillFrontmatter {
    let Some(yaml) = extract_frontmatter(contents) else {
        return SkillFrontmatter::default();
    };
    serde_yaml::from_str(yaml).unwrap_or_default()
}

fn extract_frontmatter(contents: &str) -> Option<&str> {
    let normalized = contents.strip_prefix("---\n")?;
    let end = normalized.find("\n---\n")?;
    Some(&normalized[..end])
}

fn is_valid_skill_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

fn safe_relative_path(path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err("skill file path must be relative".to_owned());
    }

    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("skill file path cannot leave the skill directory".to_owned());
            }
        }
    }

    if clean.as_os_str().is_empty() {
        Err("skill file path is empty".to_owned())
    } else {
        Ok(clean)
    }
}

async fn list_mcp_tools_tool(
    call: &LlmToolCall,
    metadata: &HashMap<String, Value>,
    workspace_dir: &Path,
    runtime_env: &HashMap<String, String>,
) -> Result<Value, String> {
    let servers = mcp_servers_from_metadata(metadata);
    if servers.is_empty() {
        return Err("no MCP servers are configured".to_owned());
    }

    if let Some(name) = call
        .arguments
        .get("server")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let server = servers
            .get(name)
            .ok_or_else(|| format!("MCP server '{}' not found", name))?;
        let tools = list_server_tools(server, workspace_dir, runtime_env).await?;
        return Ok(json!({ "servers": [{ "name": name, "tools": tools }] }));
    }

    let mut values = Vec::new();
    for (name, server) in servers {
        let tools = list_server_tools(&server, workspace_dir, runtime_env).await?;
        values.push(json!({ "name": name, "tools": tools }));
    }
    Ok(json!({ "servers": values }))
}

async fn call_mcp_tool_tool(
    call: &LlmToolCall,
    metadata: &HashMap<String, Value>,
    workspace_dir: &Path,
    runtime_env: &HashMap<String, String>,
) -> Result<Value, String> {
    let server_name = call
        .arguments
        .get("server")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing server".to_owned())?;
    let tool_name = call
        .arguments
        .get("tool")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing tool".to_owned())?;
    let arguments = call
        .arguments
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let servers = mcp_servers_from_metadata(metadata);
    let server = servers
        .get(server_name)
        .ok_or_else(|| format!("MCP server '{}' not found", server_name))?;
    call_server_tool(server, workspace_dir, runtime_env, tool_name, arguments).await
}

fn mcp_servers_from_metadata(
    metadata: &HashMap<String, Value>,
) -> BTreeMap<String, McpServerConfig> {
    let Some(raw_servers) = metadata
        .get("remote_mcp_servers")
        .and_then(Value::as_object)
    else {
        return BTreeMap::new();
    };

    raw_servers
        .iter()
        .filter_map(|(name, raw)| normalize_mcp_server(raw).map(|server| (name.clone(), server)))
        .filter(|(_, server)| server.enabled)
        .filter(|(_, server)| match server.transport {
            McpTransport::Stdio => !server.command.trim().is_empty(),
            McpTransport::StreamableHttp => server
                .url
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty()),
        })
        .collect()
}

fn normalize_mcp_server(raw: &Value) -> Option<McpServerConfig> {
    serde_json::from_value::<NativeMcpServerConfig>(raw.clone())
        .ok()
        .map(McpServerConfig::from)
}

async fn list_server_tools(
    server: &McpServerConfig,
    workspace_dir: &Path,
    runtime_env: &HashMap<String, String>,
) -> Result<Vec<Value>, String> {
    with_mcp_timeout(async {
        match server.transport {
            McpTransport::Stdio => {
                list_stdio_server_tools(server, workspace_dir, runtime_env).await
            }
            McpTransport::StreamableHttp => list_http_server_tools(server, runtime_env).await,
        }
    })
    .await
}

async fn call_server_tool(
    server: &McpServerConfig,
    workspace_dir: &Path,
    runtime_env: &HashMap<String, String>,
    tool_name: &str,
    arguments: Map<String, Value>,
) -> Result<Value, String> {
    with_mcp_timeout(async {
        match server.transport {
            McpTransport::Stdio => {
                call_stdio_server_tool(server, workspace_dir, runtime_env, tool_name, arguments)
                    .await
            }
            McpTransport::StreamableHttp => {
                call_http_server_tool(server, runtime_env, tool_name, arguments).await
            }
        }
    })
    .await
}

async fn with_mcp_timeout<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::time::timeout(MCP_TOOL_TIMEOUT, future)
        .await
        .map_err(|_| "MCP operation timed out".to_owned())?
}

async fn list_stdio_server_tools(
    server: &McpServerConfig,
    workspace_dir: &Path,
    runtime_env: &HashMap<String, String>,
) -> Result<Vec<Value>, String> {
    let transport = stdio_transport(server, workspace_dir, runtime_env)?;
    let client = ().serve(transport).await.map_err(format_rmcp_error)?;
    let result = client
        .list_all_tools()
        .await
        .map_err(format_rmcp_error)
        .map(to_tool_values);
    let _ = client.cancel().await;
    result
}

async fn call_stdio_server_tool(
    server: &McpServerConfig,
    workspace_dir: &Path,
    runtime_env: &HashMap<String, String>,
    tool_name: &str,
    arguments: Map<String, Value>,
) -> Result<Value, String> {
    let transport = stdio_transport(server, workspace_dir, runtime_env)?;
    let client = ().serve(transport).await.map_err(format_rmcp_error)?;
    let result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: tool_name.to_owned().into(),
            arguments: Some(arguments),
            task: None,
        })
        .await
        .map_err(format_rmcp_error)
        .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()));
    let _ = client.cancel().await;
    result
}

async fn list_http_server_tools(
    server: &McpServerConfig,
    runtime_env: &HashMap<String, String>,
) -> Result<Vec<Value>, String> {
    let transport =
        StreamableHttpClientTransport::from_config(http_transport_config(server, runtime_env)?);
    let client = ().serve(transport).await.map_err(format_rmcp_error)?;
    let result = client
        .list_all_tools()
        .await
        .map_err(format_rmcp_error)
        .map(to_tool_values);
    let _ = client.cancel().await;
    result
}

async fn call_http_server_tool(
    server: &McpServerConfig,
    runtime_env: &HashMap<String, String>,
    tool_name: &str,
    arguments: Map<String, Value>,
) -> Result<Value, String> {
    let transport =
        StreamableHttpClientTransport::from_config(http_transport_config(server, runtime_env)?);
    let client = ().serve(transport).await.map_err(format_rmcp_error)?;
    let result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: tool_name.to_owned().into(),
            arguments: Some(arguments),
            task: None,
        })
        .await
        .map_err(format_rmcp_error)
        .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()));
    let _ = client.cancel().await;
    result
}

fn stdio_transport(
    server: &McpServerConfig,
    workspace_dir: &Path,
    runtime_env: &HashMap<String, String>,
) -> Result<TokioChildProcess, String> {
    if server.command.trim().is_empty() {
        return Err("MCP stdio server is missing command".to_owned());
    }

    let transport = TokioChildProcess::new(Command::new(server.command.trim()).configure(|cmd| {
        cmd.args(&server.args)
            .envs(runtime_env)
            .envs(&server.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(working_dir) = server
            .working_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            cmd.current_dir(resolve_config_path(workspace_dir, working_dir));
        }
    }))
    .map_err(|error| format!("failed to start MCP stdio server: {error}"))?;
    Ok(transport)
}

fn http_transport_config(
    server: &McpServerConfig,
    runtime_env: &HashMap<String, String>,
) -> Result<StreamableHttpClientTransportConfig, String> {
    let url = server
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "MCP HTTP server is missing url".to_owned())?;
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    if let Some(token_env) = server
        .bearer_token_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && let Some(token) = runtime_env
            .get(token_env)
            .cloned()
            .or_else(|| std::env::var(token_env).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    {
        config = config.auth_header(token);
    }

    let mut headers = HashMap::new();
    for (name, value) in &server.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("invalid MCP HTTP header name '{}': {error}", name))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid MCP HTTP header value for '{}': {error}", name))?;
        headers.insert(header_name, header_value);
    }
    if !headers.is_empty() {
        config = config.custom_headers(headers);
    }

    Ok(config)
}

fn resolve_config_path(workspace_dir: &Path, value: &str) -> PathBuf {
    let expanded = PathBuf::from(shellexpand::tilde(value).as_ref());
    if expanded.is_absolute() {
        expanded
    } else {
        workspace_dir.join(expanded)
    }
}

fn to_tool_values(tools: Vec<rmcp::model::Tool>) -> Vec<Value> {
    tools
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "title": tool.title,
                "description": tool.description,
                "input_schema": Value::Object(tool.input_schema.as_ref().clone()),
                "output_schema": tool.output_schema.map(|schema| Value::Object(schema.as_ref().clone())),
                "annotations": tool.annotations,
            })
        })
        .collect()
}

fn format_rmcp_error(error: impl std::fmt::Display) -> String {
    format!("MCP error: {error}")
}

fn truncate_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut truncated = value.chars().take(limit).collect::<String>();
    truncated.push_str("\n...[truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    #[test]
    fn discovers_garyx_skill_and_renders_instructions() {
        let temp = tempfile::tempdir().unwrap();
        let skill_root = temp.path().join("skills");
        let skill_dir = skill_root.join("native-test-proof");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Native Test Proof\ndescription: Use proof instructions.\n---\nBody",
        )
        .unwrap();

        let instructions = capability_instructions_for_skill_root(&skill_root, &HashMap::new());
        let tools = capability_tool_schemas_for_skill_root(&skill_root, &HashMap::new());

        assert!(instructions.contains("`native-test-proof`"));
        assert!(instructions.contains("Use proof instructions."));
        assert!(tools.iter().any(|tool| tool.name == "load_skill"));
        assert!(tools.iter().any(|tool| tool.name == "read_skill_file"));
    }

    #[test]
    fn disabled_skills_are_not_exposed() {
        let temp = tempfile::tempdir().unwrap();
        let skill_root = temp.path().join("skills");
        let skill_dir = skill_root.join("native-disabled-proof");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Disabled Proof\ndescription: Hidden.\n---\nBody",
        )
        .unwrap();
        fs::write(
            skill_root.join(".state.json"),
            json!({ "native-disabled-proof": false }).to_string(),
        )
        .unwrap();

        let instructions = capability_instructions_for_skill_root(&skill_root, &HashMap::new());
        let tools = capability_tool_schemas_for_skill_root(&skill_root, &HashMap::new());

        assert!(!instructions.contains("native-disabled-proof"));
        assert!(!tools.iter().any(|tool| tool.name == "load_skill"));
    }

    #[tokio::test]
    async fn skill_tools_load_skill_and_reject_path_escape() {
        let temp = tempfile::tempdir().unwrap();
        let skill_root = temp.path().join("skills");
        let skill_dir = skill_root.join("native-test-loader");
        fs::create_dir_all(skill_dir.join("references")).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Loader\ndescription: Load files.\n---\nRead references/proof.txt",
        )
        .unwrap();
        fs::write(
            skill_dir.join("references").join("proof.txt"),
            "proof-token",
        )
        .unwrap();

        let loaded = load_skill_tool_from_root(
            &LlmToolCall {
                id: "call-load".to_owned(),
                name: "load_skill".to_owned(),
                arguments: json!({ "skill_id": "native-test-loader" }),
                metadata: HashMap::new(),
            },
            &skill_root,
        )
        .await
        .unwrap();
        assert!(
            loaded["content"]
                .as_str()
                .unwrap()
                .contains("Read references")
        );

        let file = read_skill_file_tool_from_root(
            &LlmToolCall {
                id: "call-read".to_owned(),
                name: "read_skill_file".to_owned(),
                arguments: json!({
                    "skill_id": "native-test-loader",
                    "path": "references/proof.txt"
                }),
                metadata: HashMap::new(),
            },
            &skill_root,
        )
        .await
        .unwrap();
        assert_eq!(file["content"], "proof-token");

        let escaped = read_skill_file_tool_from_root(
            &LlmToolCall {
                id: "call-escape".to_owned(),
                name: "read_skill_file".to_owned(),
                arguments: json!({
                    "skill_id": "native-test-loader",
                    "path": "../outside.txt"
                }),
                metadata: HashMap::new(),
            },
            &skill_root,
        )
        .await;
        assert!(escaped.unwrap_err().contains("cannot leave"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_skill_file_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let skill_root = temp.path().join("skills");
        let skill_dir = skill_root.join("native-test-symlink");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Symlink Proof\ndescription: Load files.\n---\nBody",
        )
        .unwrap();
        let outside = temp.path().join("outside.txt");
        fs::write(&outside, "outside-token").unwrap();
        symlink(&outside, skill_dir.join("outside-link.txt")).unwrap();

        let escaped = read_skill_file_tool_from_root(
            &LlmToolCall {
                id: "call-symlink".to_owned(),
                name: "read_skill_file".to_owned(),
                arguments: json!({
                    "skill_id": "native-test-symlink",
                    "path": "outside-link.txt"
                }),
                metadata: HashMap::new(),
            },
            &skill_root,
        )
        .await;

        assert!(escaped.unwrap_err().contains("cannot leave"));
    }

    #[test]
    fn mcp_metadata_enables_tool_schemas() {
        let metadata = HashMap::from([(
            "remote_mcp_servers".to_owned(),
            json!({
                "proof": {
                    "command": "python3",
                    "args": ["server.py"],
                    "enabled": true
                }
            }),
        )]);

        let tools = capability_tool_schemas(Path::new("."), &metadata);
        assert!(tools.iter().any(|tool| tool.name == "list_mcp_tools"));
        assert!(tools.iter().any(|tool| tool.name == "call_mcp_tool"));
    }

    #[test]
    fn mcp_metadata_rejects_downstream_alias_fields() {
        let metadata = HashMap::from([(
            "remote_mcp_servers".to_owned(),
            json!({
                "proof": {
                    "command": "python3",
                    "args": ["server.py"],
                    "cwd": ".",
                    "enabled": true
                }
            }),
        )]);

        let tools = capability_tool_schemas(Path::new("."), &metadata);

        assert!(!tools.iter().any(|tool| tool.name == "list_mcp_tools"));
        assert!(!tools.iter().any(|tool| tool.name == "call_mcp_tool"));
    }

    #[tokio::test]
    async fn mcp_tools_list_and_call_stdio_server() {
        if !python3_available().await {
            eprintln!("python3 not found, skipping stdio MCP smoke");
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let script = write_proof_mcp_server(temp.path(), "proof_tool", "synthetic-proof").unwrap();
        let metadata = HashMap::from([(
            "remote_mcp_servers".to_owned(),
            json!({
                "proof": {
                    "transport": "stdio",
                    "command": "python3",
                    "args": [script.to_string_lossy()],
                    "working_dir": temp.path().to_string_lossy(),
                    "enabled": true
                }
            }),
        )]);

        let listed = run_capability_tool(
            &LlmToolCall {
                id: "call-list".to_owned(),
                name: "list_mcp_tools".to_owned(),
                arguments: json!({ "server": "proof" }),
                metadata: HashMap::new(),
            },
            temp.path(),
            &metadata,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            listed["servers"][0]["tools"][0]["name"].as_str(),
            Some("proof_tool")
        );

        let called = run_capability_tool(
            &LlmToolCall {
                id: "call-proof".to_owned(),
                name: "call_mcp_tool".to_owned(),
                arguments: json!({
                    "server": "proof",
                    "tool": "proof_tool",
                    "arguments": {}
                }),
                metadata: HashMap::new(),
            },
            temp.path(),
            &metadata,
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(called["content"][0]["text"], "synthetic-proof");
        assert_eq!(called["isError"], false);
    }

    async fn python3_available() -> bool {
        Command::new("python3")
            .arg("--version")
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn write_proof_mcp_server(
        root: &Path,
        tool_name: &str,
        token: &str,
    ) -> std::io::Result<PathBuf> {
        let script_path = root.join("proof_mcp_server.py");
        let script = format!(
            r#"#!/usr/bin/env python3
import json
import sys

TOKEN = {token:?}
TOOL_NAME = {tool_name:?}


def read_message():
    first_line = sys.stdin.buffer.readline()
    if not first_line:
        return None, None

    stripped = first_line.strip()
    if stripped.startswith(b"{{"):
        return json.loads(stripped.decode("utf-8")), "jsonl"

    headers = {{}}
    line = first_line
    while True:
        if line in (b"\r\n", b"\n"):
            break
        key, value = line.decode("utf-8").split(":", 1)
        headers[key.strip().lower()] = value.strip()
        line = sys.stdin.buffer.readline()
        if not line:
            return None, None

    length = int(headers.get("content-length", "0"))
    if length <= 0:
        return None, None
    body = sys.stdin.buffer.read(length)
    return json.loads(body.decode("utf-8")), "lsp"


def write_message(payload, transport):
    if transport == "jsonl":
        sys.stdout.write(json.dumps(payload))
        sys.stdout.write("\n")
        sys.stdout.flush()
        return

    raw = json.dumps(payload).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {{len(raw)}}\r\n\r\n".encode("utf-8"))
    sys.stdout.buffer.write(raw)
    sys.stdout.buffer.flush()


while True:
    message, transport = read_message()
    if message is None:
        break

    method = message.get("method")
    req_id = message.get("id")

    if method == "initialize":
        write_message({{
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {{
                "protocolVersion": "2024-11-05",
                "serverInfo": {{"name": "proof", "version": "0.1.0"}},
                "capabilities": {{"tools": {{}}}},
            }},
        }}, transport)
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        write_message({{
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {{
                "tools": [{{
                    "name": TOOL_NAME,
                    "description": "Return the proof token.",
                    "inputSchema": {{
                        "type": "object",
                        "properties": {{}},
                        "additionalProperties": False,
                    }},
                }}]
            }},
        }}, transport)
    elif method == "tools/call":
        write_message({{
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {{
                "content": [{{"type": "text", "text": TOKEN}}],
                "isError": False,
            }},
        }}, transport)
    elif method == "ping":
        write_message({{"jsonrpc": "2.0", "id": req_id, "result": {{}}}}, transport)
    else:
        write_message({{
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {{"code": -32601, "message": f"unsupported method: {{method}}"}},
        }}, transport)
"#,
        );
        fs::write(&script_path, script)?;
        Ok(script_path)
    }
}
