use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use garyx_bridge::claude_provider::ClaudeCliProvider;
use garyx_bridge::codex_provider::CodexAgentProvider;
use garyx_bridge::provider_trait::AgentLoopProvider;
use garyx_models::config::{GaryxConfig, McpServerConfig};
use garyx_models::provider::{
    ClaudeCodeConfig, CodexAppServerConfig, ProviderRunOptions, ProviderRunResult, StreamEvent,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::timeout;

use crate::managed_mcp_metadata::inject_managed_mcp_servers;

async fn binary_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn noop_stream_callback() -> Box<dyn Fn(StreamEvent) + Send + Sync> {
    Box::new(|_| {})
}

fn unique_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn real_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn copy_tree(source: &Path, target: &Path, skip_names: &[&str]) -> io::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if skip_names.contains(&file_name) {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()), skip_names)?;
        }
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target)?;
    Ok(())
}

fn seed_temp_home(temp_home: &Path) -> io::Result<()> {
    let source_home = real_home();
    copy_tree(
        &source_home.join(".claude"),
        &temp_home.join(".claude"),
        &[],
    )?;
    copy_tree(
        &source_home.join(".codex"),
        &temp_home.join(".codex"),
        &["sessions", "history", "logs"],
    )?;
    Ok(())
}

fn make_isolated_home() -> io::Result<(TempDir, PathBuf, PathBuf)> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&workspace)?;
    seed_temp_home(&home)?;
    Ok((temp, home, workspace))
}

fn write_proof_mcp_server(root: &Path, tool_name: &str, token: &str) -> io::Result<PathBuf> {
    let script_path = root.join(format!("{tool_name}.py"));
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
    elif method == "resources/list":
        write_message({{"jsonrpc": "2.0", "id": req_id, "result": {{"resources": []}}}}, transport)
    elif method == "prompts/list":
        write_message({{"jsonrpc": "2.0", "id": req_id, "result": {{"prompts": []}}}}, transport)
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

fn proof_server_config(script_path: &Path) -> McpServerConfig {
    McpServerConfig {
        command: "python3".to_owned(),
        args: vec![script_path.to_string_lossy().to_string()],
        env: HashMap::new(),
        working_dir: script_path
            .parent()
            .map(|path| path.to_string_lossy().to_string()),
        ..Default::default()
    }
}

fn with_claude_home(metadata: &mut HashMap<String, Value>, temp_home: &Path) {
    metadata.insert(
        "desktop_claude_env".to_owned(),
        json!({"HOME": temp_home.to_string_lossy().to_string()}),
    );
}

fn managed_mcp_metadata(config: &GaryxConfig, temp_home: Option<&Path>) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    if let Some(home) = temp_home {
        with_claude_home(&mut metadata, home);
    }
    inject_managed_mcp_servers(&config.mcp_servers, &mut metadata);
    metadata
}

fn tool_use_contains(result: &ProviderRunResult, needle: &str) -> bool {
    result.session_messages.iter().any(|message| {
        message.role_str() == "tool_use"
            && message
                .tool_name
                .as_deref()
                .is_some_and(|tool_name| tool_name.contains(needle))
    })
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn claude_downstream_managed_mcp_metadata_is_usable() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let tool_name = format!("get_proof_token_{suffix}");
    let proof_token = format!("CLAUDE_MANAGED_MCP_TOKEN_{suffix}");
    let proof_server =
        write_proof_mcp_server(&workspace, &tool_name, &proof_token).expect("write proof server");

    let mut gateway_config = GaryxConfig::default();
    gateway_config.mcp_servers.insert(
        "managed-proof".to_owned(),
        proof_server_config(&proof_server),
    );

    let mut provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(2),
        max_retries: 1,
        mcp_base_url: String::new(),
        setting_sources: vec![],
        ..Default::default()
    });
    provider
        .initialize()
        .await
        .expect("initialize claude provider");

    let options = ProviderRunOptions {
        thread_id: format!("real::claude::managed-mcp::{suffix}"),
        message: format!(
            "You must call the MCP tool {tool_name} exactly once and reply with exactly the token it returns."
        ),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: managed_mcp_metadata(&gateway_config, Some(&temp_home)),
    };
    let result = timeout(
        Duration::from_secs(120),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("claude managed MCP run timed out")
    .expect("claude managed MCP run failed");
    assert!(result.success, "managed MCP run failed: {:?}", result.error);
    assert_eq!(result.response.trim(), proof_token);
    assert!(
        tool_use_contains(&result, &tool_name),
        "expected managed MCP tool use, got {:?}",
        result.session_messages
    );
}

#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn codex_downstream_managed_mcp_metadata_is_usable() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let tool_name = format!("get_proof_token_{suffix}");
    let proof_token = format!("CODEX_MANAGED_MCP_TOKEN_{suffix}");
    let proof_server =
        write_proof_mcp_server(&workspace, &tool_name, &proof_token).expect("write proof server");

    let mut gateway_config = GaryxConfig::default();
    gateway_config.mcp_servers.insert(
        "managed-proof".to_owned(),
        proof_server_config(&proof_server),
    );

    let mut provider = CodexAgentProvider::new(CodexAppServerConfig {
        model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        env: HashMap::from([("HOME".to_owned(), temp_home.to_string_lossy().to_string())]),
        ..Default::default()
    });
    provider
        .initialize()
        .await
        .expect("initialize codex provider");

    let options = ProviderRunOptions {
        thread_id: format!("real::codex::managed-mcp::{suffix}"),
        message: format!(
            "You must call the MCP tool {tool_name} exactly once and reply with exactly the token it returns."
        ),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: managed_mcp_metadata(&gateway_config, None),
    };
    let result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("codex managed MCP run timed out")
    .expect("codex managed MCP run failed");
    assert!(result.success, "managed MCP run failed: {:?}", result.error);
    assert_eq!(result.response.trim(), proof_token);
    assert!(
        tool_use_contains(&result, &tool_name),
        "expected managed MCP tool use, got {:?}",
        result.session_messages
    );
}

#[tokio::test]
#[ignore = "requires live codex CLI + auth + local gateway MCP"]
async fn codex_downstream_garyx_status_mcp_is_usable() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let response_token = format!("CODEX_GARYX_STATUS_TOKEN_{suffix}");

    let mut provider = CodexAgentProvider::new(CodexAppServerConfig {
        model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        env: HashMap::from([("HOME".to_owned(), temp_home.to_string_lossy().to_string())]),
        ..Default::default()
    });
    provider
        .initialize()
        .await
        .expect("initialize codex provider");

    let options = ProviderRunOptions {
        thread_id: format!("real::codex::garyx-status::{suffix}"),
        message: format!(
            "You must call the MCP tool `status` from the `garyx` MCP server exactly once. After it returns, reply with exactly {response_token}."
        ),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: HashMap::new(),
    };
    let result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("codex garyx status MCP run timed out")
    .expect("codex garyx status MCP run failed");
    assert!(
        result.success,
        "garyx status MCP run failed: {:?}",
        result.error
    );
    assert!(
        result.response.contains(&response_token),
        "expected response to contain token, got {:?}",
        result.response
    );
    assert!(
        tool_use_contains(&result, "mcp:garyx:status"),
        "expected garyx status MCP tool use, got {:?}",
        result.session_messages
    );
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth + local gateway MCP"]
async fn claude_downstream_garyx_status_mcp_is_usable() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let (_temp, _temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let response_token = format!("CLAUDE_GARYX_STATUS_TOKEN_{suffix}");

    let mut provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(2),
        max_retries: 1,
        setting_sources: vec![],
        ..Default::default()
    });
    provider
        .initialize()
        .await
        .expect("initialize claude provider");

    let options = ProviderRunOptions {
        thread_id: format!("real::claude::garyx-status::{suffix}"),
        message: format!(
            "You must call the MCP tool `status` from the `garyx` MCP server exactly once. After it returns, reply with exactly {response_token}."
        ),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: HashMap::new(),
    };
    let result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("claude garyx status MCP run timed out")
    .expect("claude garyx status MCP run failed");
    assert!(
        result.success,
        "garyx status MCP run failed: response={:?} error={:?} session_messages={:?}",
        result.response, result.error, result.session_messages
    );
    assert!(
        result.response.contains(&response_token),
        "expected response to contain token, got {:?}",
        result.response
    );
    assert!(
        tool_use_contains(&result, "status"),
        "expected garyx status MCP tool use, got {:?}",
        result.session_messages
    );
}
