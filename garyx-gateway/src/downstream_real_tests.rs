use std::collections::HashMap;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;
#[cfg(windows)]
use std::os::windows::fs as windows_fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use garyx_bridge::claude_provider::ClaudeCliProvider;
use garyx_bridge::codex_provider::CodexAgentProvider;
use garyx_bridge::provider_trait::AgentLoopProvider;
use garyx_channels::{ChannelDispatcher, ChannelInfo, OutboundMessage, SendMessageResult};
use garyx_models::config::{GaryxConfig, TelegramAccount};
use garyx_models::provider::{
    ClaudeCodeConfig, CodexAppServerConfig, ProviderRunOptions, StreamEvent,
};
use garyx_models::routing::DeliveryContext;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::skills::SkillsService;
use crate::{build_router, server::create_app_state};

fn insert_telegram_plugin_account(
    config: &mut GaryxConfig,
    account_id: &str,
    account: TelegramAccount,
) {
    config
        .channels
        .plugins
        .entry("telegram".to_owned())
        .or_default()
        .accounts
        .insert(
            account_id.to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&account),
        );
}

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

fn create_synced_skill(
    temp_home: &Path,
    workspace: &Path,
    skill_id: &str,
    token: &str,
) -> io::Result<()> {
    let user_skills_dir = temp_home.join(".gary").join("skills");
    let service = SkillsService::new(user_skills_dir.clone(), None);
    service
        .create_skill(
            skill_id,
            "Proof Skill",
            "Reply with a fixed proof token.",
            &format!(
                "# Proof Skill\n\nWhen this skill is used, reply with exactly {token}.\nDo not add punctuation or explanation.\n"
            ),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;

    copy_tree(
        &user_skills_dir.join(skill_id),
        &workspace.join(".claude").join("skills").join(skill_id),
        &[],
    )?;
    copy_tree(
        &user_skills_dir.join(skill_id),
        &workspace.join(".codex").join("skills").join(skill_id),
        &[],
    )?;
    Ok(())
}

fn create_managed_skill(temp_home: &Path, skill_id: &str, token: &str) -> io::Result<()> {
    let user_skills_dir = temp_home.join(".gary").join("skills");
    let service = SkillsService::new(user_skills_dir, None);
    service
        .create_skill(
            skill_id,
            "Proof Skill",
            "Reply with a fixed proof token.",
            &format!(
                "# Proof Skill\n\nWhen this skill is used, reply with exactly {token}.\nDo not add punctuation or explanation.\n"
            ),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(())
}

fn symlink_dir(source: &Path, target: &Path) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if target.exists() {
        fs::remove_dir_all(target)?;
    }

    #[cfg(unix)]
    {
        unix_fs::symlink(source, target)
    }

    #[cfg(windows)]
    {
        windows_fs::symlink_dir(source, target)
    }
}

fn create_home_linked_skill_roots(temp_home: &Path, skill_id: &str, token: &str) -> io::Result<()> {
    let user_skills_dir = temp_home.join(".gary").join("skills");
    let service = SkillsService::new(user_skills_dir.clone(), None);
    service
        .create_skill(
            skill_id,
            "Proof Skill",
            "Reply with a fixed proof token.",
            &format!(
                "# Proof Skill\n\nWhen this skill is used, reply with exactly {token}.\nDo not add punctuation or explanation.\n"
            ),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;

    for target_root in [
        temp_home.join(".claude").join("skills"),
        temp_home.join(".codex").join("skills"),
    ] {
        if target_root.exists() {
            fs::remove_dir_all(&target_root)?;
        }
        symlink_dir(&user_skills_dir, &target_root)?;
    }

    Ok(())
}

fn write_proof_mcp_server(root: &Path, token: &str) -> io::Result<PathBuf> {
    let script_path = root.join("proof_mcp_server.py");
    let script = format!(
        r#"#!/usr/bin/env python3
import json
import os
import sys

TOKEN = {token:?}


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
                    "name": "get_proof_token",
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

fn proof_mcp_metadata(script_path: &Path, token: &str) -> HashMap<String, Value> {
    HashMap::from([(
        "remote_mcp_servers".to_owned(),
        json!({
            "proof": {
                "command": "python3",
                "args": [script_path.to_string_lossy().to_string()],
                "env": {"PROOF_TOKEN": token},
                "enabled": true,
                "working_dir": script_path
                    .parent()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_default(),
            }
        }),
    )])
}

fn garyx_search_metadata(base_url: &str) -> HashMap<String, Value> {
    HashMap::from([(
        "remote_mcp_servers".to_owned(),
        json!({
            "garyx": {
                "type": "http",
                "url": format!("{base_url}/mcp"),
                "headers": {},
            }
        }),
    )])
}

fn with_home_metadata(
    mut metadata: HashMap<String, Value>,
    temp_home: &Path,
) -> HashMap<String, Value> {
    metadata.insert(
        "desktop_claude_env".to_owned(),
        json!({"HOME": temp_home.to_string_lossy().to_string()}),
    );
    metadata
}

fn slash_skill_metadata(skill_id: &str) -> HashMap<String, Value> {
    HashMap::from([
        (
            "slash_command_skill_id".to_owned(),
            Value::String(skill_id.to_owned()),
        ),
        (
            "slash_command_name".to_owned(),
            Value::String(skill_id.to_owned()),
        ),
    ])
}

fn tool_use_contains(result: &garyx_models::provider::ProviderRunResult, needle: &str) -> bool {
    result.session_messages.iter().any(|message| {
        message.role_str() == "tool_use"
            && message
                .tool_name
                .as_deref()
                .is_some_and(|tool_name| tool_name.contains(needle))
    })
}

fn tool_message_contains(result: &garyx_models::provider::ProviderRunResult, needle: &str) -> bool {
    result.session_messages.iter().any(|message| {
        matches!(message.role_str(), "tool_use" | "tool_result")
            && message.content.to_string().contains(needle)
    })
}

struct LocalMcpServer {
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl LocalMcpServer {
    async fn shutdown(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        let _ = self.handle.await;
    }
}

#[derive(Default)]
struct RecordingDispatcher {
    calls: Mutex<Vec<OutboundMessage>>,
    available_channels: Vec<ChannelInfo>,
    message_ids: Vec<String>,
}

impl RecordingDispatcher {
    fn new(available_channels: Vec<ChannelInfo>, message_ids: &[&str]) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            available_channels,
            message_ids: message_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ChannelDispatcher for RecordingDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, garyx_channels::ChannelError> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .push(request);
        Ok(SendMessageResult {
            message_ids: self.message_ids.clone(),
        })
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        self.available_channels.clone()
    }
}

async fn spawn_local_search_mcp_server() -> Option<LocalMcpServer> {
    let api_key = std::env::var("GEMINI_API_KEY")
        .ok()
        .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())?;

    let mut config = GaryxConfig::default();
    config.gateway.search.api_key = api_key;

    let state = create_app_state(config);
    let router = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local MCP listener");
    let addr = listener.local_addr().expect("local MCP addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve local MCP router");
    });

    Some(LocalMcpServer {
        base_url: format!("http://{}", addr),
        shutdown_tx: Some(shutdown_tx),
        handle,
    })
}

async fn spawn_local_bot_route_mcp_server(
    thread_id: &str,
) -> (LocalMcpServer, Arc<RecordingDispatcher>) {
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        TelegramAccount {
            token: "main-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    insert_telegram_plugin_account(
        &mut config,
        "ops",
        TelegramAccount {
            token: "ops-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );

    let state = create_app_state(config);
    let dispatcher = Arc::new(RecordingDispatcher::new(
        vec![
            ChannelInfo {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                is_running: true,
            },
            ChannelInfo {
                channel: "telegram".to_owned(),
                account_id: "ops".to_owned(),
                is_running: true,
            },
        ],
        &["msg-live-route-1"],
    ));
    state.replace_channel_dispatcher(dispatcher.clone());
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "channel_bindings": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "peer_id": "42",
                        "chat_id": "42",
                        "display_label": "Main Bot"
                    },
                    {
                        "channel": "telegram",
                        "account_id": "ops",
                        "peer_id": "84",
                        "chat_id": "84",
                        "display_label": "Ops Bot"
                    }
                ]
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        router.set_last_delivery(
            thread_id,
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        );
    }

    let router = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local bot-route MCP listener");
    let addr = listener.local_addr().expect("local bot-route MCP addr");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve local bot-route MCP router");
    });

    (
        LocalMcpServer {
            base_url: format!("http://{}", addr),
            shutdown_tx: Some(shutdown_tx),
            handle,
        },
        dispatcher,
    )
}

fn garyx_bot_route_prompt(thread_id: &str, response_token: &str) -> String {
    format!(
        "You must call the `garyx` MCP server `status` tool exactly once and confirm its `bots.thread_bound` list includes `telegram:ops`. Then call the `garyx` MCP server `message` tool exactly once with `target` set to `thread:{thread_id}`, `bot` set to `telegram:ops`, and `text` set to `{response_token}`. After the message tool succeeds, reply with exactly `{response_token}`."
    )
}

fn assert_bot_route_result(
    result: &garyx_models::provider::ProviderRunResult,
    dispatcher: &RecordingDispatcher,
    response_token: &str,
) {
    assert!(
        result.success,
        "bot-route run failed: response={:?} error={:?} session_messages={:?}",
        result.response, result.error, result.session_messages
    );
    assert!(
        result.response.contains(response_token),
        "expected response token, got response={:?} error={:?} session_messages={:?}",
        result.response,
        result.error,
        result.session_messages
    );
    assert!(
        tool_use_contains(result, "status"),
        "expected garyx status tool use, got {:?}",
        result.session_messages
    );
    assert!(
        tool_use_contains(result, "message"),
        "expected garyx message tool use, got {:?}",
        result.session_messages
    );
    assert!(
        tool_message_contains(result, "thread_bound"),
        "expected status payload with bots.thread_bound, got {:?}",
        result.session_messages
    );
    assert!(
        tool_message_contains(result, "telegram:ops"),
        "expected tool payload to mention telegram:ops, got {:?}",
        result.session_messages
    );

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1, "unexpected outbound calls: {:?}", calls);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "ops");
    assert_eq!(calls[0].chat_id, "84");
    assert_eq!(calls[0].delivery_target_type, "chat_id");
    assert_eq!(calls[0].delivery_target_id, "84");
    assert_eq!(calls[0].text_content(), Some(response_token.as_str()));
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth + gateway for search"]
async fn claude_downstream_skill_and_mcp_are_usable() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let skill_id = format!("proof-skill-{suffix}");
    let skill_token = format!("CLAUDE_SKILL_TOKEN_{suffix}");
    let proof_token = format!("CLAUDE_MCP_TOKEN_{suffix}");
    create_synced_skill(&temp_home, &workspace, &skill_id, &skill_token)
        .expect("create synced skill");
    let proof_server =
        write_proof_mcp_server(&workspace, &proof_token).expect("write proof MCP server");

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

    let skill_options = ProviderRunOptions {
        thread_id: format!("real::claude::skill::{suffix}"),
        message: "Please follow the selected skill exactly.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: with_home_metadata(slash_skill_metadata(&skill_id), &temp_home),
    };
    let skill_result = timeout(
        Duration::from_secs(120),
        provider.run_streaming(&skill_options, noop_stream_callback()),
    )
    .await
    .expect("claude skill run timed out")
    .expect("claude skill run failed");
    assert!(
        skill_result.success,
        "skill run failed: {:?}",
        skill_result.error
    );
    assert_eq!(
        skill_result.response.trim(),
        skill_token,
        "unexpected skill result: response={:?} error={:?} session_messages={:?}",
        skill_result.response,
        skill_result.error,
        skill_result.session_messages
    );

    let mcp_options = ProviderRunOptions {
        thread_id: format!("real::claude::mcp::{suffix}"),
        message: "You must call the MCP tool get_proof_token exactly once and reply with exactly the token it returns.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: with_home_metadata(proof_mcp_metadata(&proof_server, &proof_token), &temp_home),
    };
    let mcp_result = timeout(
        Duration::from_secs(120),
        provider.run_streaming(&mcp_options, noop_stream_callback()),
    )
    .await
    .expect("claude MCP run timed out")
    .expect("claude MCP run failed");
    assert!(mcp_result.success, "MCP run failed: {:?}", mcp_result.error);
    assert_eq!(mcp_result.response.trim(), proof_token);
    assert!(
        tool_use_contains(&mcp_result, "get_proof_token"),
        "expected proof MCP tool use, got {:?}",
        mcp_result.session_messages
    );

    if let Some(search_server) = spawn_local_search_mcp_server().await {
        let search_token = format!("CLAUDE_SEARCH_OK_{suffix}");
        let search_options = ProviderRunOptions {
            thread_id: format!("real::claude::search::{suffix}"),
            message: format!(
                "必须调用 garyx MCP server 上的 search 工具一次，查询 `OpenAI GPT-5`。完成后只回复 {search_token}。"
            ),
            workspace_dir: Some(workspace.to_string_lossy().to_string()),
            images: None,
            metadata: with_home_metadata(
                garyx_search_metadata(&search_server.base_url),
                &temp_home,
            ),
        };
        let search_result = timeout(
            Duration::from_secs(180),
            provider.run_streaming(&search_options, noop_stream_callback()),
        )
        .await
        .expect("claude search run timed out")
        .expect("claude search run failed");
        assert!(
            tool_use_contains(&search_result, "search"),
            "expected garyx search tool use, got {:?}",
            search_result.session_messages
        );
        assert!(
            search_result.response.contains(&search_token),
            "expected search token in response, got {}",
            search_result.response
        );

        search_server.shutdown().await;
    }
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth + local gateway MCP"]
async fn claude_downstream_garyx_status_and_message_bot_are_usable() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let (_temp, _temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let thread_id = format!("real::claude::garyx-bot-route::{suffix}");
    let response_token = format!("CLAUDE_BOT_ROUTE_OK_{suffix}");
    let (server, dispatcher) = spawn_local_bot_route_mcp_server(&thread_id).await;

    let mut provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(2),
        max_retries: 1,
        mcp_base_url: server.base_url.clone(),
        setting_sources: vec![],
        ..Default::default()
    });
    provider
        .initialize()
        .await
        .expect("initialize claude provider");

    let options = ProviderRunOptions {
        thread_id: thread_id.clone(),
        message: garyx_bot_route_prompt(&thread_id, &response_token),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: HashMap::new(),
    };
    let result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("claude garyx bot-route run timed out")
    .expect("claude garyx bot-route run failed");

    assert_bot_route_result(&result, dispatcher.as_ref(), &response_token);
    server.shutdown().await;
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn claude_downstream_home_skill_symlink_root_is_usable() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let skill_id = format!("proof-skill-link-{suffix}");
    let skill_token = format!("CLAUDE_LINKED_SKILL_TOKEN_{suffix}");
    create_home_linked_skill_roots(&temp_home, &skill_id, &skill_token)
        .expect("create linked home skill roots");

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

    let skill_options = ProviderRunOptions {
        thread_id: format!("real::claude::link-skill::{suffix}"),
        message: "Please follow the selected skill exactly.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: with_home_metadata(slash_skill_metadata(&skill_id), &temp_home),
    };
    let skill_result = timeout(
        Duration::from_secs(120),
        provider.run_streaming(&skill_options, noop_stream_callback()),
    )
    .await
    .expect("claude linked skill run timed out")
    .expect("claude linked skill run failed");
    assert!(
        skill_result.success,
        "skill run failed: {:?}",
        skill_result.error
    );
    assert_eq!(
        skill_result.response.trim(),
        skill_token,
        "unexpected linked skill result: response={:?} error={:?} session_messages={:?}",
        skill_result.response,
        skill_result.error,
        skill_result.session_messages
    );
}

#[tokio::test]
#[ignore = "requires live claude CLI + auth"]
async fn claude_downstream_managed_skill_sync_is_usable() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let skill_id = format!("proof-skill-managed-{suffix}");
    let skill_token = format!("CLAUDE_MANAGED_SKILL_TOKEN_{suffix}");
    create_managed_skill(&temp_home, &skill_id, &skill_token).expect("create managed skill");
    assert!(
        fs::symlink_metadata(temp_home.join(".claude").join("skills").join(&skill_id))
            .expect("claude managed skill metadata")
            .file_type()
            .is_symlink()
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

    let skill_options = ProviderRunOptions {
        thread_id: format!("real::claude::managed-skill::{suffix}"),
        message: "Please follow the selected skill exactly.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: with_home_metadata(slash_skill_metadata(&skill_id), &temp_home),
    };
    let skill_result = timeout(
        Duration::from_secs(120),
        provider.run_streaming(&skill_options, noop_stream_callback()),
    )
    .await
    .expect("claude managed skill run timed out")
    .expect("claude managed skill run failed");
    assert!(
        skill_result.success,
        "skill run failed: {:?}",
        skill_result.error
    );
    assert_eq!(
        skill_result.response.trim(),
        skill_token,
        "unexpected managed skill result: response={:?} error={:?} session_messages={:?}",
        skill_result.response,
        skill_result.error,
        skill_result.session_messages
    );
}

#[tokio::test]
#[ignore = "requires live codex CLI + auth + gateway for search"]
async fn codex_downstream_skill_and_mcp_are_usable() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let skill_id = format!("proof-skill-{suffix}");
    let skill_token = format!("CODEX_SKILL_TOKEN_{suffix}");
    let proof_token = format!("CODEX_MCP_TOKEN_{suffix}");
    create_synced_skill(&temp_home, &workspace, &skill_id, &skill_token)
        .expect("create synced skill");
    let proof_server =
        write_proof_mcp_server(&workspace, &proof_token).expect("write proof MCP server");

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

    let skill_options = ProviderRunOptions {
        thread_id: format!("real::codex::skill::{suffix}"),
        message: "Please follow the selected skill exactly.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: slash_skill_metadata(&skill_id),
    };
    let skill_result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&skill_options, noop_stream_callback()),
    )
    .await
    .expect("codex skill run timed out")
    .expect("codex skill run failed");
    assert!(
        skill_result.success,
        "skill run failed: {:?}",
        skill_result.error
    );
    assert!(
        skill_result.response.contains(&skill_token),
        "unexpected skill result: response={:?} error={:?} session_messages={:?}",
        skill_result.response,
        skill_result.error,
        skill_result.session_messages
    );
    assert!(
        tool_message_contains(
            &skill_result,
            &format!("/.codex/skills/{skill_id}/SKILL.md")
        ),
        "expected codex to read skill file, got {:?}",
        skill_result.session_messages
    );

    let mcp_options = ProviderRunOptions {
        thread_id: format!("real::codex::mcp::{suffix}"),
        message: "You must call the MCP tool get_proof_token exactly once and reply with exactly the token it returns.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: proof_mcp_metadata(&proof_server, &proof_token),
    };
    let mcp_result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&mcp_options, noop_stream_callback()),
    )
    .await
    .expect("codex MCP run timed out")
    .expect("codex MCP run failed");
    assert!(mcp_result.success, "MCP run failed: {:?}", mcp_result.error);
    assert_eq!(mcp_result.response.trim(), proof_token);
    assert!(
        tool_use_contains(&mcp_result, "mcp:proof:get_proof_token"),
        "expected proof MCP tool use, got {:?}",
        mcp_result.session_messages
    );

    if let Some(search_server) = spawn_local_search_mcp_server().await {
        let search_token = format!("CODEX_SEARCH_OK_{suffix}");
        let search_options = ProviderRunOptions {
            thread_id: format!("real::codex::search::{suffix}"),
            message: format!(
                "必须调用 garyx MCP server 上的 search 工具一次，查询 `OpenAI GPT-5`。完成后只回复 {search_token}。"
            ),
            workspace_dir: Some(workspace.to_string_lossy().to_string()),
            images: None,
            metadata: garyx_search_metadata(&search_server.base_url),
        };
        let search_result = timeout(
            Duration::from_secs(240),
            provider.run_streaming(&search_options, noop_stream_callback()),
        )
        .await
        .expect("codex search run timed out")
        .expect("codex search run failed");
        assert!(
            tool_use_contains(&search_result, "mcp:garyx:search"),
            "expected garyx search tool use, got {:?}",
            search_result.session_messages
        );
        assert!(
            search_result.response.contains(&search_token),
            "expected search token in response, got {}",
            search_result.response
        );

        search_server.shutdown().await;
    }

    provider.shutdown().await.expect("shutdown codex provider");
}

#[tokio::test]
#[ignore = "requires live codex CLI + auth + local gateway MCP"]
async fn codex_downstream_garyx_status_and_message_bot_are_usable() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let thread_id = format!("real::codex::garyx-bot-route::{suffix}");
    let response_token = format!("CODEX_BOT_ROUTE_OK_{suffix}");
    let (server, dispatcher) = spawn_local_bot_route_mcp_server(&thread_id).await;

    let mut provider = CodexAgentProvider::new(CodexAppServerConfig {
        model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        mcp_base_url: server.base_url.clone(),
        env: HashMap::from([("HOME".to_owned(), temp_home.to_string_lossy().to_string())]),
        ..Default::default()
    });
    provider
        .initialize()
        .await
        .expect("initialize codex provider");

    let options = ProviderRunOptions {
        thread_id: thread_id.clone(),
        message: garyx_bot_route_prompt(&thread_id, &response_token),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: HashMap::new(),
    };
    let result = timeout(
        Duration::from_secs(240),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("codex garyx bot-route run timed out")
    .expect("codex garyx bot-route run failed");

    assert_bot_route_result(&result, dispatcher.as_ref(), &response_token);
    provider.shutdown().await.expect("shutdown codex provider");
    server.shutdown().await;
}

#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn codex_downstream_home_skill_symlink_root_is_usable() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let skill_id = format!("proof-skill-link-{suffix}");
    let skill_token = format!("CODEX_LINKED_SKILL_TOKEN_{suffix}");
    create_home_linked_skill_roots(&temp_home, &skill_id, &skill_token)
        .expect("create linked home skill roots");

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

    let skill_options = ProviderRunOptions {
        thread_id: format!("real::codex::link-skill::{suffix}"),
        message: "Please follow the selected skill exactly.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: slash_skill_metadata(&skill_id),
    };
    let skill_result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&skill_options, noop_stream_callback()),
    )
    .await
    .expect("codex linked skill run timed out")
    .expect("codex linked skill run failed");
    assert!(
        skill_result.success,
        "skill run failed: {:?}",
        skill_result.error
    );
    assert!(
        skill_result.response.contains(&skill_token),
        "unexpected linked skill result: response={:?} error={:?} session_messages={:?}",
        skill_result.response,
        skill_result.error,
        skill_result.session_messages
    );
}

#[tokio::test]
#[ignore = "requires live codex CLI + auth"]
async fn codex_downstream_managed_skill_sync_is_usable() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let (_temp, temp_home, workspace) = make_isolated_home().expect("isolated home");
    let suffix = unique_suffix();
    let skill_id = format!("proof-skill-managed-{suffix}");
    let skill_token = format!("CODEX_MANAGED_SKILL_TOKEN_{suffix}");
    create_managed_skill(&temp_home, &skill_id, &skill_token).expect("create managed skill");
    assert!(
        fs::symlink_metadata(temp_home.join(".codex").join("skills").join(&skill_id))
            .expect("codex managed skill metadata")
            .file_type()
            .is_symlink()
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

    let skill_options = ProviderRunOptions {
        thread_id: format!("real::codex::managed-skill::{suffix}"),
        message: "Please follow the selected skill exactly.".to_owned(),
        workspace_dir: Some(workspace.to_string_lossy().to_string()),
        images: None,
        metadata: slash_skill_metadata(&skill_id),
    };
    let skill_result = timeout(
        Duration::from_secs(180),
        provider.run_streaming(&skill_options, noop_stream_callback()),
    )
    .await
    .expect("codex managed skill run timed out")
    .expect("codex managed skill run failed");
    assert!(
        skill_result.success,
        "skill run failed: {:?}",
        skill_result.error
    );
    assert!(
        skill_result.response.contains(&skill_token),
        "unexpected managed skill result: response={:?} error={:?} session_messages={:?}",
        skill_result.response,
        skill_result.error,
        skill_result.session_messages
    );
}
