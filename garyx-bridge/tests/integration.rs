//! Integration tests for garyx-bridge providers.
//!
//! These tests exercise the real Claude CLI and Codex app-server through
//! the bridge's `AgentLoopProvider` trait.
//!
//! Run with: `cargo test -p garyx-bridge --test integration -- --ignored`
//!
//! Requirements:
//! - `claude` CLI installed and in PATH (for Claude tests)
//! - `codex` CLI installed and in PATH (for Codex tests)
//! - Valid authentication configured for both

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use garyx_bridge::claude_provider::ClaudeCliProvider;
use garyx_bridge::codex_provider::CodexAgentProvider;
use garyx_bridge::gemini_provider::GeminiCliProvider;
use garyx_bridge::provider_trait::AgentLoopProvider;
use garyx_models::local_paths::{
    agent_memory_dir_for_gary_home, agent_memory_root_file_for_gary_home, gary_home_dir,
};
use garyx_models::provider::{
    ClaudeCodeConfig, CodexAppServerConfig, GeminiCliConfig, PromptAttachment,
    PromptAttachmentKind, ProviderRunOptions, ProviderType, QueuedUserInput, StreamEvent,
    attachments_to_metadata_value,
};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn binary_available(name: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn telegram_logo_attachment_metadata() -> HashMap<String, serde_json::Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../garyx-gateway/assets/channel-icons/telegram.png");
    let attachment = PromptAttachment {
        kind: PromptAttachmentKind::Image,
        path: path.to_string_lossy().into_owned(),
        name: "telegram.png".to_owned(),
        media_type: "image/png".to_owned(),
    };
    let mut metadata = HashMap::new();
    metadata.insert(
        "attachments".to_owned(),
        attachments_to_metadata_value(&[attachment]),
    );
    metadata
}

fn temp_text_attachment_metadata() -> (
    tempfile::TempDir,
    HashMap<String, serde_json::Value>,
    String,
) {
    let dir = tempdir().expect("create tempdir");
    let file_name = "garyx-attachment-smoke.txt".to_owned();
    let secret = "GARYX_FILE_ATTACHMENT_SECRET_20260423".to_owned();
    let path = dir.path().join(&file_name);
    fs::write(
        &path,
        format!("Garyx file attachment smoke test.\nReturn exactly this secret token: {secret}\n"),
    )
    .expect("write temp attachment");
    let attachment = PromptAttachment {
        kind: PromptAttachmentKind::File,
        path: path.to_string_lossy().into_owned(),
        name: file_name,
        media_type: "text/plain".to_owned(),
    };
    let mut metadata = HashMap::new();
    metadata.insert(
        "attachments".to_owned(),
        attachments_to_metadata_value(&[attachment]),
    );
    (dir, metadata, secret)
}

fn noop_stream_callback() -> Box<dyn Fn(StreamEvent) + Send + Sync> {
    Box::new(|_| {})
}

fn write_agent_memory_marker(agent_id: &str, marker: &str) -> std::io::Result<()> {
    let memory_file = agent_memory_root_file_for_gary_home(&gary_home_dir(), agent_id);
    if let Some(parent) = memory_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        memory_file,
        format!(
            "# Agent Memory\n\n## Durable Notes\n- Agent Memory marker: {marker}\n- When asked for the Agent Memory marker, reply with exactly `{marker}`.\n"
        ),
    )
}

fn cleanup_agent_memory(agent_id: &str) {
    let _ = fs::remove_dir_all(agent_memory_dir_for_gary_home(&gary_home_dir(), agent_id));
}

// ---------------------------------------------------------------------------
// Session management (fast — no CLI needed)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_claude_session_management() {
    let config = ClaudeCodeConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let provider = ClaudeCliProvider::new(config);

    // Create a session
    let sid = provider
        .get_or_create_session("test::sess")
        .await
        .expect("get_or_create_session failed");
    assert!(!sid.is_empty());

    // Same key returns same session
    let sid2 = provider.get_or_create_session("test::sess").await.unwrap();
    assert_eq!(sid, sid2);

    // Clear
    assert!(provider.clear_session("test::sess").await);
    assert!(!provider.clear_session("test::sess").await);
}

#[tokio::test]
async fn test_codex_session_management() {
    let config = CodexAppServerConfig::default();
    let provider = CodexAgentProvider::new(config);

    let sid = provider
        .get_or_create_session("test::codex::sess")
        .await
        .expect("get_or_create_session failed");
    assert!(sid.is_empty(), "Expected empty placeholder for new session");

    assert!(provider.clear_session("test::codex::sess").await);
}

#[tokio::test]
async fn test_gemini_session_management() {
    let config = GeminiCliConfig::default();
    let provider = GeminiCliProvider::new(config);

    let sid = provider
        .get_or_create_session("test::gemini::sess")
        .await
        .expect("get_or_create_session failed");
    assert!(sid.is_empty(), "Expected empty placeholder for new session");

    assert!(!provider.clear_session("test::gemini::sess").await);
}

// ---------------------------------------------------------------------------
// Claude Provider — single comprehensive test
// ---------------------------------------------------------------------------

/// Initialize, run (sync + streaming), verify results — all in one test to
/// avoid spawning multiple CLI processes.
#[tokio::test]
#[ignore]
async fn test_claude_provider_full() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let config = ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(1),
        max_retries: 1,
        mcp_base_url: String::new(),
        setting_sources: vec![],
        ..Default::default()
    };

    // --- Initialize ---
    let mut provider = ClaudeCliProvider::new(config);
    assert_eq!(provider.provider_type(), ProviderType::ClaudeCode);
    assert!(!provider.is_ready());
    provider.initialize().await.expect("initialize failed");
    assert!(provider.is_ready());

    // --- Sync run ---
    eprintln!("[test] starting sync run...");
    let options = ProviderRunOptions {
        thread_id: "test::claude::full".to_owned(),
        message: "What is 7 + 8? Reply with just the number.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("sync run timed out after 30s")
    .expect("run_streaming failed");
    eprintln!("[test] sync run done: {}", result.response);
    assert!(result.success, "Run was not successful: {:?}", result.error);
    assert!(
        result.response.contains("15"),
        "Expected '15' in response, got: {}",
        result.response
    );
    assert!(result.sdk_session_id.is_some(), "Expected sdk_session_id");

    // --- Streaming run ---
    eprintln!("[test] starting streaming run...");
    let stream_opts = ProviderRunOptions {
        thread_id: "test::claude::full::stream".to_owned(),
        message: "What is 10 * 10? Reply with just the number.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_clone = chunks.clone();
    let got_final = Arc::new(Mutex::new(false));
    let got_final_clone = got_final.clone();

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(move |event| match event {
        StreamEvent::Delta { text } => {
            if !text.is_empty() {
                chunks_clone.lock().unwrap().push(text);
            }
        }
        StreamEvent::ToolUse { .. } | StreamEvent::ToolResult { .. } => {}
        StreamEvent::Done => {
            *got_final_clone.lock().unwrap() = true;
        }
        StreamEvent::Boundary { .. } | StreamEvent::ThreadTitleUpdated { .. } => {}
    });

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        provider.run_streaming(&stream_opts, callback),
    )
    .await
    .expect("streaming run timed out after 30s")
    .expect("run_streaming failed");
    eprintln!("[test] streaming run done: {}", result.response);

    assert!(result.success, "Streaming run failed: {:?}", result.error);
    assert!(
        result.response.contains("100"),
        "Expected '100' in streaming response, got: {}",
        result.response
    );
    assert!(
        *got_final.lock().unwrap(),
        "Never received final chunk callback"
    );
}

/// Verify MCP server is loaded through Claude provider -> claude-agent-sdk ->
/// Claude CLI, by asserting an MCP tool_use/tool_result appears in session
/// traces.
///
/// Requirements (in addition to Claude auth):
/// - Gateway MCP endpoint is reachable (defaults to `http://127.0.0.1:31337/mcp`)
/// - Override via `GARYX_MCP_BASE_URL` when needed
#[tokio::test]
#[ignore]
async fn test_claude_provider_mcp_status_tool_loaded() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let mcp_base_url =
        std::env::var("GARYX_MCP_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:31337".to_owned());

    let config = ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(2),
        max_retries: 1,
        mcp_base_url,
        setting_sources: vec![],
        ..Default::default()
    };

    let mut provider = ClaudeCliProvider::new(config);
    provider.initialize().await.expect("initialize failed");

    let options = ProviderRunOptions {
        thread_id: "test::claude::mcp::status".to_owned(),
        message: "必须调用 mcp__garyx__status 工具一次，并只输出 status 字段值。".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("mcp run timed out after 60s")
    .expect("mcp run_streaming failed");

    assert!(result.success, "Run was not successful: {:?}", result.error);

    let used_mcp_status_tool = result.session_messages.iter().any(|msg| {
        msg.role_str() == "tool_use" && msg.tool_name.as_deref() == Some("mcp__garyx__status")
    });
    assert!(
        used_mcp_status_tool,
        "Expected tool_use for mcp__garyx__status, got session_messages={:?}",
        result.session_messages
    );

    let got_mcp_tool_result = result
        .session_messages
        .iter()
        .any(|msg| msg.role_str() == "tool_result");
    assert!(
        got_mcp_tool_result,
        "Expected at least one tool_result in session_messages={:?}",
        result.session_messages
    );
}

// ---------------------------------------------------------------------------
// Codex Provider — single comprehensive test
// ---------------------------------------------------------------------------

/// Initialize, run (sync + streaming), shutdown — all in one test.
#[tokio::test]
#[ignore]
async fn test_codex_provider_full() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let config = CodexAppServerConfig {
        model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        workspace_dir: Some("/tmp".to_owned()),
        ..Default::default()
    };

    // --- Initialize ---
    let mut provider = CodexAgentProvider::new(config);
    assert_eq!(provider.provider_type(), ProviderType::CodexAppServer);
    assert!(!provider.is_ready());
    provider.initialize().await.expect("initialize failed");
    assert!(provider.is_ready());

    // --- Sync run ---
    let options = ProviderRunOptions {
        thread_id: "test::codex::full".to_owned(),
        message: "What is 3 + 4? Reply with just the number.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let result = provider
        .run_streaming(&options, noop_stream_callback())
        .await
        .expect("run_streaming failed");
    assert!(result.success, "Run was not successful: {:?}", result.error);
    assert!(
        result.response.contains('7'),
        "Expected '7' in response, got: {}",
        result.response
    );

    // --- Streaming run ---
    let stream_opts = ProviderRunOptions {
        thread_id: "test::codex::full::stream".to_owned(),
        message: "What is 8 * 8? Reply with just the number.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let chunks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_clone = chunks.clone();
    let got_final = Arc::new(Mutex::new(false));
    let got_final_clone = got_final.clone();

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(move |event| match event {
        StreamEvent::Delta { text } => {
            if !text.is_empty() {
                chunks_clone.lock().unwrap().push(text);
            }
        }
        StreamEvent::ToolUse { .. } | StreamEvent::ToolResult { .. } => {}
        StreamEvent::Done => {
            *got_final_clone.lock().unwrap() = true;
        }
        StreamEvent::Boundary { .. } | StreamEvent::ThreadTitleUpdated { .. } => {}
    });

    let result = provider
        .run_streaming(&stream_opts, callback)
        .await
        .expect("run_streaming failed");

    assert!(result.success, "Streaming run failed: {:?}", result.error);
    assert!(
        result.response.contains("64"),
        "Expected '64' in streaming response, got: {}",
        result.response
    );
    assert!(
        *got_final.lock().unwrap(),
        "Never received final chunk callback"
    );

    // --- Shutdown ---
    provider.shutdown().await.expect("shutdown failed");
    assert!(!provider.is_ready());
}

// ---------------------------------------------------------------------------
// Gemini Provider — single comprehensive test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn test_gemini_provider_full() {
    if !binary_available("gemini").await {
        eprintln!("gemini not found, skipping");
        return;
    }

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        approval_mode: "yolo".to_owned(),
        ..Default::default()
    });
    assert_eq!(provider.provider_type(), ProviderType::GeminiCli);
    assert!(!provider.is_ready());
    provider.initialize().await.expect("initialize failed");
    assert!(provider.is_ready());

    let options = ProviderRunOptions {
        thread_id: "test::gemini::full".to_owned(),
        message: "Reply with the single word pong.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("gemini run timed out after 180s")
    .expect("gemini run_streaming failed");

    assert!(result.success, "Run was not successful: {:?}", result.error);
    assert!(
        result.response.to_ascii_lowercase().contains("pong"),
        "Expected pong in response, got: {}",
        result.response
    );
    assert!(result.sdk_session_id.is_some(), "Expected sdk_session_id");

    provider.shutdown().await.expect("shutdown failed");
    assert!(!provider.is_ready());
}

#[tokio::test]
#[ignore]
async fn test_gemini_provider_mcp_status_tool_loaded() {
    if !binary_available("gemini").await {
        eprintln!("gemini not found, skipping");
        return;
    }

    let mcp_base_url =
        std::env::var("GARYX_MCP_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:31337".to_owned());

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        mcp_base_url,
        approval_mode: "yolo".to_owned(),
        ..Default::default()
    });
    provider.initialize().await.expect("initialize failed");

    let options = ProviderRunOptions {
        thread_id: "test::gemini::mcp::status".to_owned(),
        message: "必须调用 mcp__garyx__status 工具一次，并只输出 status 字段值。".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("gemini MCP run timed out after 180s")
    .expect("gemini MCP run_streaming failed");

    assert!(result.success, "Run was not successful: {:?}", result.error);
    let used_mcp_status_tool = result.session_messages.iter().any(|msg| {
        msg.role_str() == "tool_use"
            && msg
                .tool_use_id
                .as_deref()
                .is_some_and(|id| id.starts_with("mcp_garyx_status-"))
    });
    assert!(
        used_mcp_status_tool,
        "Expected tool_use for mcp__garyx__status, got session_messages={:?}",
        result.session_messages
    );
    assert!(
        result
            .session_messages
            .iter()
            .any(|msg| msg.role_str() == "tool_result"),
        "Expected at least one tool_result in session_messages={:?}",
        result.session_messages
    );
}

#[tokio::test]
#[ignore]
async fn test_codex_provider_agent_memory_visible() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let workspace = tempdir().unwrap();
    let workspace_dir = workspace.path().to_path_buf();
    let test_agent_id = "codex-memory-test";
    let marker = "codex-agent-memory-marker-3a2b";
    write_agent_memory_marker(test_agent_id, marker).unwrap();

    let config = CodexAppServerConfig {
        model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        workspace_dir: Some(workspace_dir.display().to_string()),
        ..Default::default()
    };

    let mut provider = CodexAgentProvider::new(config);
    provider.initialize().await.expect("initialize failed");

    let options = ProviderRunOptions {
        thread_id: "test::codex::agent_memory".to_owned(),
        message: "Reply with the Agent Memory marker only.".to_owned(),
        workspace_dir: Some(workspace_dir.display().to_string()),
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), serde_json::json!(test_agent_id))]),
    };

    let result = provider
        .run_streaming(&options, noop_stream_callback())
        .await
        .expect("codex auto memory run failed");

    provider.shutdown().await.expect("shutdown failed");
    cleanup_agent_memory(test_agent_id);

    assert!(result.success, "Run was not successful: {:?}", result.error);
    assert!(
        result.response.contains(marker),
        "Expected marker in response, got: {}",
        result.response
    );
}

// ---------------------------------------------------------------------------
// Claude Provider — resume failure fallback
// ---------------------------------------------------------------------------

/// Verify that when a stale/invalid session ID is used for resume,
/// the provider falls back to creating a new session instead of failing.
#[tokio::test]
#[ignore]
async fn test_claude_resume_failure_fallback() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let config = ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(1),
        max_retries: 1,
        mcp_base_url: String::new(),
        setting_sources: vec![],
        ..Default::default()
    };

    let mut provider = ClaudeCliProvider::new(config);
    provider.initialize().await.expect("initialize failed");

    // Inject a fake/stale session ID via metadata — simulates loading a
    // persisted sdk_session_id that has since expired or been invalidated.
    let mut metadata = HashMap::new();
    metadata.insert(
        "sdk_session_id".to_owned(),
        serde_json::Value::String("550e8400-e29b-41d4-a716-446655440000".to_owned()),
    );

    let options = ProviderRunOptions {
        thread_id: "test::claude::resume_fallback".to_owned(),
        message: "What is 2 + 2? Reply with just the number.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata,
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        provider.run_streaming(&options, noop_stream_callback()),
    )
    .await
    .expect("resume fallback run timed out after 60s")
    .expect("run_streaming should succeed after fallback to new session");

    eprintln!(
        "[test] resume fallback done: success={} response={}",
        result.success, result.response
    );
    assert!(
        result.success,
        "Expected successful run after resume fallback: {:?}",
        result.error
    );
    assert!(
        result.response.contains('4'),
        "Expected '4' in response, got: {}",
        result.response
    );
    assert!(
        result.sdk_session_id.is_some(),
        "Expected new sdk_session_id after fallback"
    );
}

/// Verify that a live Claude streaming session accepts follow-up input through
/// `add_streaming_input` and incorporates that input into the final result.
#[tokio::test]
#[ignore]
async fn test_claude_streaming_input_follow_up_live() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let config = ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(3),
        max_retries: 1,
        mcp_base_url: String::new(),
        setting_sources: vec![],
        ..Default::default()
    };

    let mut provider = ClaudeCliProvider::new(config);
    provider.initialize().await.expect("initialize failed");

    let thread_id = "test::claude::streaming_follow_up".to_owned();
    let options = ProviderRunOptions {
        thread_id: thread_id.clone(),
        message: "Start by replying with READY on its own line, then continue with a long numbered list from 1 to 200, one item per line, and do not stop early unless I send a follow-up instruction.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(|_| {});

    let run_future = provider.run_streaming(&options, callback);
    tokio::pin!(run_future);

    let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let mut accepted = false;
        let mut attempts = 0usize;

        while !accepted {
            tokio::select! {
                result = &mut run_future => {
                    panic!(
                        "Claude run completed before add_streaming_input was accepted: {:?}",
                        result
                    );
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                    attempts += 1;
                    accepted = provider
                        .add_streaming_input(
                            &thread_id,
                            QueuedUserInput::text(
                                "Stop the numbered list now and reply with exactly FOLLOW_UP_OK",
                            )
                            .with_pending_input_id("live-follow-up-1"),
                        )
                        .await;
                    if attempts >= 30 {
                        panic!("add_streaming_input was not accepted within 30s");
                    }
                }
            }
        }

        run_future.await
    })
    .await
    .expect("streaming follow-up run timed out after 120s")
    .expect("run_streaming failed");

    eprintln!(
        "[test] streaming follow-up done: success={} response={}",
        result.success, result.response
    );
    assert!(
        result.success,
        "Expected successful streaming run with follow-up: {:?}",
        result.error
    );
    assert!(
        result.response.contains("FOLLOW_UP_OK"),
        "Expected follow-up response marker in final response, got: {}",
        result.response
    );
    assert!(
        result.sdk_session_id.is_some(),
        "Expected sdk_session_id for live streaming follow-up run"
    );
}

#[tokio::test]
#[ignore]
async fn test_claude_provider_image_attachment_live() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let mut provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(1),
        max_retries: 1,
        mcp_base_url: String::new(),
        setting_sources: vec![],
        ..Default::default()
    });
    provider.initialize().await.expect("initialize failed");

    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "test::claude::attachment::image".to_owned(),
                message: "Inspect the attached image file. Reply with exactly one uppercase word: TELEGRAM if it is the Telegram logo, otherwise UNKNOWN.".to_owned(),
                workspace_dir: None,
                images: None,
                metadata: telegram_logo_attachment_metadata(),
            },
            noop_stream_callback(),
        )
        .await
        .expect("claude attachment run failed");

    assert!(result.success, "Claude run failed: {:?}", result.error);
    assert!(
        result.response.to_ascii_uppercase().contains("TELEGRAM"),
        "Expected TELEGRAM, got: {}",
        result.response
    );
}

#[tokio::test]
#[ignore]
async fn test_codex_provider_image_attachment_live() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let mut provider = CodexAgentProvider::new(CodexAppServerConfig {
        model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        workspace_dir: Some(env!("CARGO_MANIFEST_DIR").to_owned()),
        ..Default::default()
    });
    provider.initialize().await.expect("initialize failed");

    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "test::codex::attachment::image".to_owned(),
                message: "Inspect the attached image file. Reply with exactly one uppercase word: TELEGRAM if it is the Telegram logo, otherwise UNKNOWN.".to_owned(),
                workspace_dir: None,
                images: None,
                metadata: telegram_logo_attachment_metadata(),
            },
            noop_stream_callback(),
        )
        .await
        .expect("codex attachment run failed");

    assert!(result.success, "Codex run failed: {:?}", result.error);
    assert!(
        result.response.to_ascii_uppercase().contains("TELEGRAM"),
        "Expected TELEGRAM, got: {}",
        result.response
    );
}

#[tokio::test]
#[ignore]
async fn test_gemini_provider_image_attachment_live() {
    if !binary_available("gemini").await {
        eprintln!("gemini not found, skipping");
        return;
    }

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        approval_mode: "yolo".to_owned(),
        workspace_dir: Some(env!("CARGO_MANIFEST_DIR").to_owned()),
        ..Default::default()
    });
    provider.initialize().await.expect("initialize failed");

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        provider.run_streaming(
            &ProviderRunOptions {
                thread_id: "test::gemini::attachment::image".to_owned(),
                message: "Inspect the attached image file. Reply with exactly one uppercase word: TELEGRAM if it is the Telegram logo, otherwise UNKNOWN.".to_owned(),
                workspace_dir: None,
                images: None,
                metadata: telegram_logo_attachment_metadata(),
            },
            noop_stream_callback(),
        ),
    )
    .await
    .expect("gemini attachment run timed out after 180s")
    .expect("gemini attachment run failed");

    assert!(result.success, "Gemini run failed: {:?}", result.error);
    assert!(
        result.response.to_ascii_uppercase().contains("TELEGRAM"),
        "Expected TELEGRAM, got: {}",
        result.response
    );
}

#[tokio::test]
#[ignore]
async fn test_claude_provider_file_attachment_live() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let (_dir, metadata, secret) = temp_text_attachment_metadata();
    let mut provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        max_turns: Some(1),
        max_retries: 1,
        mcp_base_url: String::new(),
        setting_sources: vec![],
        ..Default::default()
    });
    provider.initialize().await.expect("initialize failed");

    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "test::claude::attachment::file".to_owned(),
                message: "Read the attached file and reply with exactly the secret token contained inside it. Do not add any other words.".to_owned(),
                workspace_dir: None,
                images: None,
                metadata,
            },
            noop_stream_callback(),
        )
        .await
        .expect("claude file attachment run failed");

    assert!(result.success, "Claude run failed: {:?}", result.error);
    assert!(
        result.response.contains(&secret),
        "Expected secret token {}, got: {}",
        secret,
        result.response
    );
}

#[tokio::test]
#[ignore]
async fn test_codex_provider_file_attachment_live() {
    if !binary_available("codex").await {
        eprintln!("codex not found, skipping");
        return;
    }

    let (_dir, metadata, secret) = temp_text_attachment_metadata();
    let mut provider = CodexAgentProvider::new(CodexAppServerConfig {
        model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        workspace_dir: Some(env!("CARGO_MANIFEST_DIR").to_owned()),
        ..Default::default()
    });
    provider.initialize().await.expect("initialize failed");

    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "test::codex::attachment::file".to_owned(),
                message: "Read the attached file and reply with exactly the secret token contained inside it. Do not add any other words.".to_owned(),
                workspace_dir: None,
                images: None,
                metadata,
            },
            noop_stream_callback(),
        )
        .await
        .expect("codex file attachment run failed");

    assert!(result.success, "Codex run failed: {:?}", result.error);
    assert!(
        result.response.contains(&secret),
        "Expected secret token {}, got: {}",
        secret,
        result.response
    );
}

#[tokio::test]
#[ignore]
async fn test_gemini_provider_file_attachment_live() {
    if !binary_available("gemini").await {
        eprintln!("gemini not found, skipping");
        return;
    }

    let (_dir, metadata, secret) = temp_text_attachment_metadata();
    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        approval_mode: "yolo".to_owned(),
        workspace_dir: Some(env!("CARGO_MANIFEST_DIR").to_owned()),
        ..Default::default()
    });
    provider.initialize().await.expect("initialize failed");

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        provider.run_streaming(
            &ProviderRunOptions {
                thread_id: "test::gemini::attachment::file".to_owned(),
                message: "Read the attached file and reply with exactly the secret token contained inside it. Do not add any other words.".to_owned(),
                workspace_dir: None,
                images: None,
                metadata,
            },
            noop_stream_callback(),
        ),
    )
    .await
    .expect("gemini file attachment run timed out after 180s")
    .expect("gemini file attachment run failed");

    assert!(result.success, "Gemini run failed: {:?}", result.error);
    assert!(
        result.response.contains(&secret),
        "Expected secret token {}, got: {}",
        secret,
        result.response
    );
}
