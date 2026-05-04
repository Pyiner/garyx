use super::*;
use crate::gary_prompt::GARY_BASE_INSTRUCTIONS;
use crate::native_slash::build_native_skill_prompt;
use claude_agent_sdk::{
    AssistantMessage, ResultMessage, ToolResultBlock, ToolUseBlock, UserContent, UserInput,
    UserMessage,
};
use garyx_models::provider::{ClaudeCodeConfig, QueuedUserInput};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;

fn make_provider() -> ClaudeCliProvider {
    ClaudeCliProvider::new(ClaudeCodeConfig::default())
}

#[tokio::test]
async fn test_session_map_create() {
    let provider = make_provider();

    let sid1 = provider
        .get_or_create_session("sess::tg::123")
        .await
        .unwrap();
    assert!(!sid1.is_empty());

    let sid2 = provider
        .get_or_create_session("sess::tg::123")
        .await
        .unwrap();
    assert_eq!(sid1, sid2);

    let sid3 = provider
        .get_or_create_session("sess::tg::456")
        .await
        .unwrap();
    assert_ne!(sid1, sid3);
}

#[tokio::test]
async fn test_session_map_clear() {
    let provider = make_provider();

    let _ = provider.get_or_create_session("sess::a").await.unwrap();
    assert!(provider.clear_session("sess::a").await);
    assert!(!provider.clear_session("sess::a").await);

    let new_sid = provider.get_or_create_session("sess::a").await.unwrap();
    assert!(!new_sid.is_empty());
}

#[tokio::test]
async fn test_provider_type() {
    let provider = make_provider();
    assert_eq!(provider.provider_type(), ProviderType::ClaudeCode);
}

#[tokio::test]
async fn test_is_ready_before_init() {
    let provider = make_provider();
    assert!(!provider.is_ready());
}

#[tokio::test]
async fn test_run_returns_not_ready_before_init() {
    let provider = make_provider();
    let options = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };
    let noop: StreamCallback = Box::new(|_| {});
    let err = provider.run_streaming(&options, noop).await.unwrap_err();
    assert!(matches!(err, BridgeError::ProviderNotReady));
}

#[tokio::test]
async fn test_abort_nonexistent_run() {
    let provider = make_provider();
    assert!(!provider.abort("nonexistent").await);
}

#[test]
fn test_resolve_run_id_uses_metadata_override() {
    let mut metadata = HashMap::new();
    metadata.insert(
        "client_run_id".to_owned(),
        Value::String("run_from_client".to_owned()),
    );

    assert_eq!(resolve_run_id(&metadata), "run_from_client");
}

#[test]
fn test_resolve_run_id_generates_unique_default_ids() {
    let first = resolve_run_id(&HashMap::new());
    let second = resolve_run_id(&HashMap::new());

    assert!(first.starts_with("run_"));
    assert!(second.starts_with("run_"));
    assert_ne!(first, second);
}

#[tokio::test]
async fn test_multiple_sessions_isolated() {
    let provider = make_provider();

    let s1 = provider.get_or_create_session("a::1").await.unwrap();
    let s2 = provider.get_or_create_session("b::2").await.unwrap();

    provider.clear_session("a::1").await;

    let s2_again = provider.get_or_create_session("b::2").await.unwrap();
    assert_eq!(s2, s2_again);

    let s1_new = provider.get_or_create_session("a::1").await.unwrap();
    assert_ne!(s1, s1_new);
}

#[test]
fn test_retryable_error_detection() {
    assert!(is_retryable_error("Error: overloaded_error"));
    assert!(is_retryable_error("HTTP 529 Too Many Requests"));
    assert!(is_retryable_error("connection refused"));
    assert!(!is_retryable_error("invalid argument"));
    assert!(!is_retryable_error("permission denied"));
}

#[test]
fn test_session_corruption_detection() {
    assert!(is_session_corrupted_error("Error: session not found"));
    assert!(is_session_corrupted_error("invalid session id"));
    assert!(!is_session_corrupted_error("some other error"));
}

#[test]
fn test_fresh_session_retry_detection() {
    assert!(should_retry_with_fresh_session(
        "Control protocol error: CLI process exited before responding"
    ));
    assert!(should_retry_with_fresh_session(
        "run failed: no result from claude SDK"
    ));
    assert!(should_retry_with_fresh_session("Error: session not found"));
    assert!(!should_retry_with_fresh_session("permission denied"));
}

#[test]
fn test_build_sdk_options_defaults() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    // Permission mode
    assert_eq!(
        sdk_opts.permission_mode,
        Some(PermissionMode::BypassPermissions)
    );

    // Disallowed tools
    assert!(
        sdk_opts
            .disallowed_tools
            .contains(&"EnterPlanMode".to_string())
    );
    assert!(
        sdk_opts
            .disallowed_tools
            .contains(&"ExitPlanMode".to_string())
    );
    assert!(
        sdk_opts
            .disallowed_tools
            .contains(&"AskUserQuestion".to_string())
    );

    // MCP servers
    assert!(sdk_opts.mcp_servers.contains_key("garyx"));
    match &sdk_opts.mcp_servers["garyx"] {
        McpServerConfig::Http { url, headers } => {
            assert!(url.contains("/mcp"));
            assert_eq!(headers.get("X-Run-Id").unwrap(), "run-1");
        }
        _ => panic!("expected Http MCP config"),
    }

    // Setting sources
    assert_eq!(
        sdk_opts.setting_sources,
        Some(vec![
            "user".to_string(),
            "project".to_string(),
            "local".to_string()
        ])
    );

    // Extra args
    assert!(sdk_opts.extra_args.contains_key("replay-user-messages"));

    // Max buffer size
    assert_eq!(sdk_opts.max_buffer_size, Some(10 * 1024 * 1024));

    // Max turns intentionally unset (no --max-turns injection).
    assert!(sdk_opts.max_turns.is_none());

    // No resume for new session
    assert!(sdk_opts.resume.is_none());
}

#[test]
fn test_build_sdk_options_maps_auto_to_default_permissions() {
    let mut config = ClaudeCodeConfig::default();
    config.permission_mode = "auto".to_owned();
    let provider = ClaudeCliProvider::new(config);
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    assert_eq!(sdk_opts.permission_mode, Some(PermissionMode::Auto));
}

#[test]
fn test_build_sdk_options_maps_dont_ask_to_bypass_permissions() {
    let mut config = ClaudeCodeConfig::default();
    config.permission_mode = "dontAsk".to_owned();
    let provider = ClaudeCliProvider::new(config);
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    assert_eq!(
        sdk_opts.permission_mode,
        Some(PermissionMode::BypassPermissions)
    );
}

#[test]
fn test_build_sdk_options_merges_garyx_mcp_headers_from_metadata() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "garyx_mcp_headers".to_owned(),
            serde_json::json!({
                "X-Gary-AutoResearch-Role": "verifier"
            }),
        )]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    match &sdk_opts.mcp_servers["garyx"] {
        McpServerConfig::Http { headers, .. } => {
            assert_eq!(
                headers.get("X-Gary-AutoResearch-Role").map(String::as_str),
                Some("verifier")
            );
        }
        _ => panic!("expected Http MCP config"),
    }
}

#[test]
fn test_build_sdk_options_with_session() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, Some("session-abc"), "run-1");
    assert_eq!(sdk_opts.resume, Some("session-abc".to_string()));
}

#[test]
fn test_build_sdk_options_uses_claude_session_agent_for_custom_agent() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "thread::agent".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([
            (
                "agent_id".to_owned(),
                Value::String("spec-review".to_owned()),
            ),
            (
                "agent_display_name".to_owned(),
                Value::String("Spec Review".to_owned()),
            ),
            (
                "system_prompt".to_owned(),
                Value::String("Review specs carefully.".to_owned()),
            ),
            (
                "runtime_context".to_owned(),
                serde_json::json!({
                    "channel": "api",
                    "account_id": "main",
                    "bot_id": "api:main",
                    "task": {
                        "task_id": "#TASK-12",
                        "status": "todo"
                    }
                }),
            ),
        ]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    assert_eq!(sdk_opts.agent.as_deref(), Some("spec-review"));
    assert!(sdk_opts.system_prompt.is_none());
    assert!(sdk_opts.append_system_prompt.is_none());
    assert_eq!(
        sdk_opts.env.get("GARYX_AGENT_ID").map(String::as_str),
        Some("spec-review")
    );
    assert_eq!(
        sdk_opts.env.get("GARYX_ACTOR").map(String::as_str),
        Some("agent:spec-review")
    );
    assert_eq!(
        sdk_opts.env.get("GARYX_TASK_ID").map(String::as_str),
        Some("#TASK-12")
    );
    let definition = sdk_opts.agents.get("spec-review").expect("session agent");
    assert_eq!(definition.description, "Garyx custom agent: Spec Review");
    assert!(definition.prompt.contains("Review specs carefully."));
    // Regression guard for 81b81a5 ("custom agent uses own system_prompt
    // directly, no Garyx base injection"): when a custom agent supplies its
    // own system_prompt, we must NOT prepend a Garyx-branded preamble,
    // otherwise the agent would introduce itself as "Garyx" in user-facing
    // output. The previous assertion here checked the opposite and was
    // left over from before that fix.
    assert!(
        !definition.prompt.contains("You are Garyx"),
        "custom agent prompt must not carry Garyx-branded preamble: {}",
        definition.prompt
    );
}

#[test]
fn test_build_sdk_options_builtin_garyx_overrides_runtime_entry() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "test-thread".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "remote_mcp_servers".to_owned(),
            serde_json::json!({
                "garyx": {
                    "url": "http://127.0.0.1:31337",
                    "headers": {"X-Run-Id": "stale-run"}
                },
                "proof": {
                    "command": "python3",
                    "args": ["proof.py"]
                }
            }),
        )]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    assert!(sdk_opts.mcp_servers.contains_key("proof"));
    match &sdk_opts.mcp_servers["garyx"] {
        McpServerConfig::Http { url, headers } => {
            assert_eq!(url, "http://127.0.0.1:31337/mcp/test-thread/run-1");
            assert_eq!(headers.get("X-Run-Id").map(String::as_str), Some("run-1"));
            assert_eq!(
                headers.get("X-Thread-Id").map(String::as_str),
                Some("test-thread")
            );
        }
        _ => panic!("expected Http MCP config"),
    }
}

#[test]
fn test_build_sdk_options_workspace_override_wins() {
    let config = ClaudeCodeConfig {
        workspace_dir: Some("/tmp/from-config".to_string()),
        ..ClaudeCodeConfig::default()
    };
    let provider = ClaudeCliProvider::new(config);
    let workspace_override = std::env::temp_dir().to_string_lossy().to_string();

    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: Some(workspace_override.clone()),
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    assert_eq!(
        sdk_opts.cwd,
        Some(std::path::PathBuf::from(workspace_override))
    );
}

#[test]
fn test_build_sdk_options_with_model() {
    let config = ClaudeCodeConfig {
        default_model: "claude-opus-4-6".to_string(),
        ..ClaudeCodeConfig::default()
    };
    let provider = ClaudeCliProvider::new(config);

    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    assert_eq!(sdk_opts.model, Some("claude-opus-4-6".to_string()));
}

#[test]
fn test_build_sdk_options_metadata_model_override() {
    let config = ClaudeCodeConfig {
        default_model: "claude-haiku-4-5".to_string(),
        ..ClaudeCodeConfig::default()
    };
    let provider = ClaudeCliProvider::new(config);

    let mut metadata = HashMap::new();
    metadata.insert(
        "model".to_string(),
        Value::String("claude-opus-4-6".to_string()),
    );
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata,
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    assert_eq!(sdk_opts.model, Some("claude-opus-4-6".to_string()));
}

#[test]
fn test_build_sdk_options_metadata_env_override() {
    let provider = make_provider();

    let mut metadata = HashMap::new();
    metadata.insert(
        "desktop_claude_env".to_string(),
        serde_json::json!({
            "CLAUDE_CODE_OAUTH_TOKEN": "token-123",
            "ANTHROPIC_API_KEY": "api-key-456",
            "NON_STRING": 42
        }),
    );
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata,
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    assert_eq!(
        sdk_opts.env.get("CLAUDE_CODE_OAUTH_TOKEN"),
        Some(&"token-123".to_string())
    );
    assert_eq!(
        sdk_opts.env.get("ANTHROPIC_API_KEY"),
        Some(&"api-key-456".to_string())
    );
    assert!(!sdk_opts.env.contains_key("NON_STRING"));
}

#[test]
fn test_build_sdk_options_merges_config_env_and_metadata_env() {
    let provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        env: HashMap::from([
            ("HTTPS_PROXY".to_owned(), "http://127.0.0.1:6152".to_owned()),
            ("NO_PROXY".to_owned(), "127.0.0.1,localhost".to_owned()),
            (
                "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
                "from-config".to_owned(),
            ),
        ]),
        ..Default::default()
    });

    let mut metadata = HashMap::new();
    metadata.insert(
        "desktop_claude_env".to_string(),
        serde_json::json!({
            "CLAUDE_CODE_OAUTH_TOKEN": "from-desktop",
            "ALL_PROXY": "socks5://127.0.0.1:6153"
        }),
    );
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata,
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    assert_eq!(
        sdk_opts.env.get("HTTPS_PROXY").map(String::as_str),
        Some("http://127.0.0.1:6152")
    );
    assert_eq!(
        sdk_opts.env.get("NO_PROXY").map(String::as_str),
        Some("127.0.0.1,localhost")
    );
    assert_eq!(
        sdk_opts.env.get("ALL_PROXY").map(String::as_str),
        Some("socks5://127.0.0.1:6153")
    );
    assert_eq!(
        sdk_opts
            .env
            .get("CLAUDE_CODE_OAUTH_TOKEN")
            .map(String::as_str),
        Some("from-desktop")
    );
}

#[test]
fn test_build_sdk_options_with_system_prompt() {
    let config = ClaudeCodeConfig {
        system_prompt: Some("You are a helpful bot.".to_string()),
        ..ClaudeCodeConfig::default()
    };
    let provider = ClaudeCliProvider::new(config);

    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    let system_prompt = sdk_opts.system_prompt.unwrap_or_default();
    assert!(system_prompt.starts_with(GARY_BASE_INSTRUCTIONS.trim_end()));
    assert!(system_prompt.contains("Garyx runtime guidance:"));
    assert!(system_prompt.contains("Additional runtime instructions:"));
    assert!(system_prompt.contains("You are a helpful bot."));
    assert!(!system_prompt.contains("Global Memory"));
    assert!(!system_prompt.contains("<garyx_memory_context>"));
    assert!(!system_prompt.contains("Current runtime context:"));
    assert!(!system_prompt.contains("thread_id: test"));
}

#[test]
fn test_build_sdk_options_injects_gary_prompt_by_default() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    let system_prompt = sdk_opts.system_prompt.unwrap_or_default();
    assert!(system_prompt.starts_with(GARY_BASE_INSTRUCTIONS.trim_end()));
    assert!(system_prompt.contains("Self-evolution:"));
    assert!(system_prompt.contains("System capabilities:"));
    assert!(system_prompt.contains("garyx task create"));
    assert!(system_prompt.contains("garyx automation create"));
    assert!(!system_prompt.contains("Global Memory"));
    assert!(!system_prompt.contains("<garyx_memory_context>"));
    assert!(!system_prompt.contains("Current runtime context:"));
}

#[test]
fn test_build_sdk_options_exports_task_cli_env_from_metadata() {
    let provider = make_provider();
    let workspace_dir = std::env::temp_dir().to_string_lossy().to_string();
    let opts = ProviderRunOptions {
        thread_id: "thread::ctx".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: Some(workspace_dir.clone()),
        images: None,
        metadata: HashMap::from([(
            "runtime_context".to_owned(),
            serde_json::json!({
                "channel": "weixin",
                "account_id": "main",
                "bot_id": "weixin:main",
                "thread_id": "thread::ctx",
                "thread": {
                    "agent_id": "codex"
                },
                "task": {
                    "task_id": "#TASK-5",
                    "status": "in_review"
                }
            }),
        )]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    let system_prompt = sdk_opts.system_prompt.unwrap_or_default();
    assert!(system_prompt.contains("System capabilities:"));
    assert!(!system_prompt.contains("channel: weixin"));
    assert!(!system_prompt.contains("task_id: #TASK-5"));
    assert_eq!(
        sdk_opts.env.get("GARYX_THREAD_ID").map(String::as_str),
        Some("thread::ctx")
    );
    assert_eq!(
        sdk_opts.env.get("GARYX_AGENT_ID").map(String::as_str),
        Some("codex")
    );
    assert_eq!(
        sdk_opts.env.get("GARYX_TASK_ID").map(String::as_str),
        Some("#TASK-5")
    );
    assert!(!sdk_opts.env.contains_key("GARYX_TASK_SCOPE"));
    assert_eq!(opts.workspace_dir.as_deref(), Some(workspace_dir.as_str()));
}

#[test]
fn test_build_sdk_options_omits_empty_setting_sources_override() {
    let provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        setting_sources: vec![],
        ..ClaudeCodeConfig::default()
    });
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    assert!(sdk_opts.setting_sources.is_none());
}

#[test]
fn test_build_sdk_options_does_not_inject_skill_instruction_into_system_prompt() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "Use the skill.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "slash_command_skill_id".to_owned(),
            Value::String("proof-skill".to_owned()),
        )]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    let system_prompt = sdk_opts.system_prompt.unwrap_or_default();
    assert!(system_prompt.starts_with(GARY_BASE_INSTRUCTIONS.trim_end()));
    assert!(system_prompt.contains("Garyx runtime guidance:"));
    assert!(!system_prompt.contains("/proof-skill"));
}

#[test]
fn test_build_user_message_input_uses_native_skill_invocation() {
    let options = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "Use the skill.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "slash_command_skill_id".to_owned(),
            Value::String("proof-skill".to_owned()),
        )]),
    };

    assert_eq!(
        build_native_skill_prompt(&options.message, &options.metadata).as_deref(),
        Some("/proof-skill\n\nUse the skill.")
    );

    match build_user_message_input(&options, false) {
        UserInput::Text(text) => assert_eq!(text, "/proof-skill\n\nUse the skill."),
        UserInput::Blocks(_) => panic!("expected text input"),
    }
}

#[test]
fn test_build_user_message_input_does_not_append_task_status_suffix() {
    let options = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "继续".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "runtime_context".to_owned(),
            serde_json::json!({
                "task": {
                    "task_id": "#TASK-8",
                    "status": "todo",
                    "assignee": { "kind": "agent", "agent_id": "codex" }
                }
            }),
        )]),
    };

    match build_user_message_input(&options, false) {
        UserInput::Text(text) => assert_eq!(text, "继续"),
        UserInput::Blocks(_) => panic!("expected text input"),
    }

    match build_user_message_input(&options, true) {
        UserInput::Text(text) => {
            assert!(text.starts_with("<garyx_thread_metadata>"));
            assert!(text.contains("task_id: #TASK-8"));
            assert!(!text.contains("status=todo"));
            assert!(!text.contains("assignee=agent:codex"));
            assert!(text.ends_with("继续"));
        }
        UserInput::Blocks(_) => panic!("expected text input"),
    }
}

#[test]
fn test_process_assistant_blocks_streaming_preserves_block_order() {
    let blocks = vec![
        ContentBlock::Text(TextBlock {
            text: "Hello ".to_string(),
        }),
        ContentBlock::ToolUse(ToolUseBlock {
            id: "tu-1".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({}),
        }),
        ContentBlock::Text(TextBlock {
            text: "world!".to_string(),
        }),
    ];

    let mut response_text = String::new();
    let mut session_messages = Vec::new();
    let emitted = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let cb: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("emitted mutex poisoned")
            .push(event);
    });

    process_assistant_blocks_streaming(
        &blocks,
        &mut response_text,
        &mut session_messages,
        &cb,
        None,
    );

    assert_eq!(response_text, "Hello world!");
    let roles: Vec<&str> = session_messages
        .iter()
        .map(|entry| entry.role_str())
        .collect();
    assert_eq!(roles, vec!["assistant", "tool_use", "assistant"]);
    let events = emitted.lock().expect("emitted mutex poisoned").clone();
    assert!(matches!(
        &events[0],
        StreamEvent::Delta { text } if text == "Hello "
    ));
    assert!(matches!(
        &events[1],
        StreamEvent::ToolUse { message } if message.role_str() == "tool_use"
    ));
    assert!(matches!(
        &events[2],
        StreamEvent::Delta { text } if text == "world!"
    ));
}

#[test]
fn test_append_assistant_segment_separator_uses_blank_line() {
    let mut response_text = "第一段".to_owned();
    append_assistant_segment_separator(&mut response_text);
    assert_eq!(response_text, "第一段\n\n");
}

#[test]
fn test_assistant_blocks_starting_with_newline_skip_boundary_separator() {
    let blocks = vec![ContentBlock::Text(TextBlock {
        text: "\n结果如下。".to_owned(),
    })];
    assert!(assistant_blocks_have_visible_text(&blocks));
    assert!(assistant_blocks_start_with_newline(&blocks));
}

#[test]
fn test_has_tool_result_blocks_true() {
    let blocks = vec![ContentBlock::ToolResult(ToolResultBlock {
        tool_use_id: "tu-1".to_string(),
        content: Some(Value::String("ok".to_string())),
        is_error: None,
    })];
    assert!(has_tool_result_blocks(&blocks));
}

#[test]
fn test_has_tool_result_blocks_false() {
    let blocks = vec![ContentBlock::Text(TextBlock {
        text: "hello".to_string(),
    })];
    assert!(!has_tool_result_blocks(&blocks));
}

#[test]
fn test_extract_tool_session_messages_preserves_parent_tool_use_id_metadata() {
    let blocks = vec![
        ContentBlock::ToolUse(ToolUseBlock {
            id: "toolu_child".to_owned(),
            name: "Bash".to_owned(),
            input: serde_json::json!({
                "command": "pwd"
            }),
        }),
        ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "toolu_child".to_owned(),
            content: Some(Value::String("/tmp".to_owned())),
            is_error: Some(false),
        }),
    ];

    let mut session_messages = Vec::new();
    extract_tool_session_messages(&blocks, &mut session_messages, None, Some("toolu_parent"));

    assert_eq!(session_messages.len(), 2);
    for message in &session_messages {
        assert_eq!(
            message.metadata.get("parent_tool_use_id"),
            Some(&Value::String("toolu_parent".to_owned()))
        );
    }
}

#[tokio::test]
async fn test_process_messages_streaming_emits_user_ack_boundaries() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("first user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "第一段".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("second user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "第二段".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    let mut usage = HashMap::new();
    usage.insert("input".to_owned(), Value::from(12));
    usage.insert("output".to_owned(), Value::from(34));

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 2,
        session_id: "sdk-session-1".to_owned(),
        total_cost_usd: Some(0.01),
        usage: Some(usage),
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider.set_pending_inputs("run-1", 1).await;
    let (response_text, result_data) = provider
        .process_messages_streaming("run-1", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "第一段\n\n第二段");
    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(
        emitted,
        vec![
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: None,
            },
            StreamEvent::Delta {
                text: "第一段".to_owned(),
            },
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: None,
            },
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::AssistantSegment,
                pending_input_id: None,
            },
            StreamEvent::Delta {
                text: "第二段".to_owned(),
            },
        ]
    );

    let result = result_data.expect("expected result message");
    assert_eq!(result.session_id, "sdk-session-1");
    assert_eq!(result.input_tokens, 12);
    assert_eq!(result.output_tokens, 34);
    assert!(!result.is_error);
    assert_eq!(result.actual_model.as_deref(), Some("claude-test"));
}

#[tokio::test]
async fn test_process_messages_streaming_requires_result_message_for_completion() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "partial progress".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let cb: StreamCallback = Box::new(|_| {});
    let (response_text, result_data) = provider
        .process_messages_streaming("run-no-result", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "partial progress");
    assert!(
        result_data.is_none(),
        "Claude text/tool events alone must not be treated as a completed run"
    );
}

#[tokio::test]
async fn test_process_messages_streaming_emits_queued_input_ack_id_after_root_ack() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("first user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "working".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("queued user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 2,
        session_id: "sdk-session-queued".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    provider.run_pending_inputs.lock().await.insert(
        "run-queued".to_owned(),
        VecDeque::from([
            PendingAckMarker::RootUserMessage,
            PendingAckMarker::QueuedInput("queued-1".to_owned()),
        ]),
    );

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (_response_text, result_data) = provider
        .process_messages_streaming("run-queued", "thread::test", &mut rx, &cb)
        .await;

    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(
        emitted,
        vec![
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: None,
            },
            StreamEvent::Delta {
                text: "working".to_owned(),
            },
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: Some("queued-1".to_owned()),
            },
        ]
    );

    let result = result_data.expect("expected result message");
    assert_eq!(result.session_id, "sdk-session-queued");
}

#[tokio::test]
async fn test_process_messages_streaming_emits_assistant_segment_boundaries() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(6);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "让我先看看。".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "tu-1".to_owned(),
            content: Some(Value::String("ok".to_owned())),
            is_error: Some(false),
        })]),
        uuid: None,
        parent_tool_use_id: Some("tu-1".to_owned()),
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "好了，现在开始修。".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-segment".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, _result_data) = provider
        .process_messages_streaming("run-assistant-segment", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "让我先看看。\n\n好了，现在开始修。");
    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(emitted.len(), 4);
    assert!(matches!(
        &emitted[0],
        StreamEvent::Delta { text } if text == "让我先看看。"
    ));
    assert!(matches!(
        &emitted[1],
        StreamEvent::ToolResult { message } if message.role_str() == "tool_result"
    ));
    assert_eq!(
        emitted[2],
        StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        }
    );
    assert!(matches!(
        &emitted[3],
        StreamEvent::Delta { text } if text == "好了，现在开始修。"
    ));
}

#[tokio::test]
async fn test_process_messages_streaming_emits_tool_result_user_echo_without_boundary() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "tu-1".to_owned(),
            content: Some(Value::String("ok".to_owned())),
            is_error: None,
        })]),
        uuid: None,
        parent_tool_use_id: Some("tu-1".to_owned()),
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "after tool".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-2".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider.set_pending_inputs("run-2", 1).await;
    let (response_text, _result_data) = provider
        .process_messages_streaming("run-2", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "after tool");
    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert!(matches!(
        &emitted[0],
        StreamEvent::ToolResult { message } if message.role_str() == "tool_result"
    ));
    assert_eq!(
        emitted[1],
        StreamEvent::Delta {
            text: "after tool".to_owned()
        }
    );
}

#[tokio::test]
async fn test_process_messages_streaming_emits_live_tool_events() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(6);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::ToolUse(ToolUseBlock {
            id: "toolu_1".to_owned(),
            name: "Bash".to_owned(),
            input: serde_json::json!({
                "command": "pwd"
            }),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "toolu_1".to_owned(),
            content: Some(Value::String("/tmp/workspace".to_owned())),
            is_error: Some(false),
        })]),
        uuid: None,
        parent_tool_use_id: Some("toolu_1".to_owned()),
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-tools".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, _result_data) = provider
        .process_messages_streaming("run-tools", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "");
    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert!(
        matches!(&emitted[0], StreamEvent::ToolUse { message } if message.role_str() == "tool_use")
    );
    assert!(
        matches!(&emitted[1], StreamEvent::ToolResult { message } if message.role_str() == "tool_result")
    );
}

#[tokio::test]
async fn test_process_messages_streaming_suppresses_subagent_text_but_keeps_tool_trace() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![
            ContentBlock::Text(TextBlock {
                text: "子 Agent 内部文本，不应外发".to_owned(),
            }),
            ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_child".to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({
                    "command": "pwd"
                }),
            }),
        ],
        model: "claude-test".to_owned(),
        parent_tool_use_id: Some("toolu_parent".to_owned()),
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "toolu_child".to_owned(),
            content: Some(Value::String("/tmp/workspace".to_owned())),
            is_error: Some(false),
        })]),
        uuid: None,
        parent_tool_use_id: Some("toolu_parent".to_owned()),
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "最终只保留顶层回复".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-subagent".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, result_data) = provider
        .process_messages_streaming("run-subagent", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "最终只保留顶层回复");

    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(emitted.len(), 3);
    assert!(
        matches!(&emitted[0], StreamEvent::ToolUse { message } if message.role_str() == "tool_use")
    );
    assert!(
        matches!(&emitted[1], StreamEvent::ToolResult { message } if message.role_str() == "tool_result")
    );
    assert_eq!(
        emitted[2],
        StreamEvent::Delta {
            text: "最终只保留顶层回复".to_owned()
        }
    );

    let result = result_data.expect("expected result message");
    let roles: Vec<&str> = result
        .session_messages
        .iter()
        .map(|entry| entry.role_str())
        .collect();
    assert_eq!(roles, vec!["tool_use", "tool_result", "assistant"]);
    assert!(
        result
            .session_messages
            .iter()
            .all(|entry| entry.text.as_deref() != Some("子 Agent 内部文本，不应外发"))
    );
}

#[tokio::test]
async fn test_process_messages_streaming_preserves_assistant_block_order() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(6);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![
            ContentBlock::Text(TextBlock {
                text: "在。先执行 ls。".to_owned(),
            }),
            ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_order".to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({
                    "command": "ls"
                }),
            }),
        ],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "toolu_order".to_owned(),
            content: Some(Value::String("a\nb\n".to_owned())),
            is_error: Some(false),
        })]),
        uuid: None,
        parent_tool_use_id: Some("toolu_order".to_owned()),
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "\n结果如下。".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-order".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, result_data) = provider
        .process_messages_streaming("run-order", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "在。先执行 ls。\n结果如下。");
    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert!(matches!(
        &emitted[0],
        StreamEvent::Delta { text } if text == "在。先执行 ls。"
    ));
    assert!(matches!(
        &emitted[1],
        StreamEvent::ToolUse { message } if message.role_str() == "tool_use"
    ));
    assert!(matches!(
        &emitted[2],
        StreamEvent::ToolResult { message } if message.role_str() == "tool_result"
    ));
    assert!(matches!(
        &emitted[3],
        StreamEvent::Delta { text } if text == "\n结果如下。"
    ));

    let result = result_data.expect("expected result message");
    let roles: Vec<&str> = result
        .session_messages
        .iter()
        .map(|entry| entry.role_str())
        .collect();
    assert_eq!(
        roles,
        vec!["assistant", "tool_use", "tool_result", "assistant"]
    );
}

#[tokio::test]
async fn test_process_messages_streaming_waits_for_all_pending_results() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("first user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "first ".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-1".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("queued user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "second".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-2".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
    })))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider.run_pending_inputs.lock().await.insert(
        "run-3".to_owned(),
        VecDeque::from([
            PendingAckMarker::RootUserMessage,
            PendingAckMarker::QueuedInput("queued-1".to_owned()),
        ]),
    );
    let (response_text, result_data) = provider
        .process_messages_streaming("run-3", "thread::test", &mut rx, &cb)
        .await;

    assert_eq!(response_text, "first \n\nsecond");
    let result = result_data.expect("expected final result");
    assert_eq!(result.session_id, "sdk-session-2");
    assert_eq!(result.actual_model.as_deref(), Some("claude-test"));
}

#[test]
fn test_build_user_message_input_plain_text_without_images() {
    let options = ProviderRunOptions {
        thread_id: "sess".to_owned(),
        message: "describe this".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };

    let content = build_user_message_input(&options, false);
    assert_eq!(content, UserInput::Text("describe this".to_owned()));
}

#[test]
fn test_build_user_message_input_prepends_memory_on_first_turn() {
    let options = ProviderRunOptions {
        thread_id: "sess".to_owned(),
        message: "describe this".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), Value::String("claude".to_owned()))]),
    };

    let content = build_user_message_input(&options, true);
    match content {
        UserInput::Text(text) => {
            assert!(text.starts_with("<garyx_memory_context>"));
            assert!(text.contains("<agent_memory agent_id=\"claude\""));
            assert!(text.ends_with("describe this"));
        }
        UserInput::Blocks(_) => panic!("expected text input"),
    }
}

#[test]
fn test_build_user_message_input_with_valid_and_invalid_images() {
    use garyx_models::ImagePayload;

    let valid = ImagePayload {
        name: "valid.png".to_owned(),
        media_type: "image/png".to_owned(),
        data: "abc123==".to_owned(),
    };
    let invalid_type = ImagePayload {
        name: "invalid.pdf".to_owned(),
        media_type: "application/pdf".to_owned(),
        data: "ignored".to_owned(),
    };
    let empty_data = ImagePayload {
        name: "empty.jpg".to_owned(),
        media_type: "image/jpeg".to_owned(),
        data: String::new(),
    };

    let options = ProviderRunOptions {
        thread_id: "sess".to_owned(),
        message: "describe this".to_owned(),
        workspace_dir: None,
        images: Some(vec![valid, invalid_type, empty_data]),
        metadata: HashMap::new(),
    };

    let content = build_user_message_input(&options, false);
    let arr = match content {
        UserInput::Blocks(arr) => arr,
        other => panic!("expected blocks input, got {other:?}"),
    };
    assert_eq!(arr.len(), 2);

    assert_eq!(arr[0]["type"], "text");
    assert_eq!(arr[0]["text"], "describe this");

    assert_eq!(arr[1]["type"], "image");
    assert_eq!(arr[1]["source"]["type"], "base64");
    assert_eq!(arr[1]["source"]["media_type"], "image/png");
    assert_eq!(arr[1]["source"]["data"], "abc123==");
}

#[tokio::test]
async fn test_failure_tracking() {
    let provider = make_provider();

    // First two failures should not trigger clear
    assert!(!provider.record_failure("test-session").await);
    assert!(!provider.record_failure("test-session").await);

    // Third failure triggers clear
    assert!(provider.record_failure("test-session").await);

    // Reset
    provider.reset_failure_count("test-session").await;
    assert!(!provider.record_failure("test-session").await);
}

#[tokio::test]
async fn test_run_streaming_retries_with_fresh_session_after_connect_failure() {
    let mut provider = make_provider();
    provider.ready = true;
    provider
        .session_map
        .lock()
        .await
        .insert("sess::retry".to_owned(), "stale-session".to_owned());
    provider
        .enqueue_test_run_attempt(Err(BridgeError::RunFailed(
            "failed to connect to claude: Control protocol error: CLI process exited before responding"
                .to_owned(),
        )))
        .await;
    provider
        .enqueue_test_run_attempt(Ok(Some(SdkRunOutcome {
            session_id: "fresh-session".to_owned(),
            response_text: "ok".to_owned(),
            session_messages: Vec::new(),
            is_error: false,
            error_message: None,
            input_tokens: 1,
            output_tokens: 1,
            cost_usd: 0.0,
            actual_model: Some("claude-3-7-sonnet".to_owned()),
        })))
        .await;

    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "sess::retry".to_owned(),
                message: "hello".to_owned(),
                workspace_dir: None,
                images: None,
                metadata: HashMap::new(),
            },
            Box::new(|_| {}),
        )
        .await
        .expect("retry should fall back to a fresh session");

    assert!(result.success);
    assert_eq!(result.sdk_session_id.as_deref(), Some("fresh-session"));
    assert_eq!(result.actual_model.as_deref(), Some("claude-3-7-sonnet"));
    assert_eq!(
        provider.recorded_test_session_attempts().await,
        vec![Some("stale-session".to_owned()), None]
    );
    assert_eq!(
        provider
            .session_map
            .lock()
            .await
            .get("sess::retry")
            .cloned()
            .as_deref(),
        Some("fresh-session")
    );
}

#[tokio::test]
async fn test_run_streaming_keeps_stable_session_id_when_sdk_reports_different_id() {
    let mut provider = make_provider();
    provider.ready = true;
    provider
        .session_map
        .lock()
        .await
        .insert("sess::stable".to_owned(), "session-a".to_owned());
    provider
        .enqueue_test_run_attempt(Ok(Some(SdkRunOutcome {
            session_id: "session-b".to_owned(),
            response_text: "ok".to_owned(),
            session_messages: Vec::new(),
            is_error: false,
            error_message: None,
            input_tokens: 1,
            output_tokens: 1,
            cost_usd: 0.0,
            actual_model: Some("claude-3-7-sonnet".to_owned()),
        })))
        .await;

    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "sess::stable".to_owned(),
                message: "hello".to_owned(),
                workspace_dir: None,
                images: None,
                metadata: HashMap::new(),
            },
            Box::new(|_| {}),
        )
        .await
        .expect("run should succeed");

    assert!(result.success);
    assert_eq!(result.sdk_session_id.as_deref(), Some("session-a"));
    assert_eq!(result.actual_model.as_deref(), Some("claude-3-7-sonnet"));
    assert_eq!(
        provider.recorded_test_session_attempts().await,
        vec![Some("session-a".to_owned())]
    );
    assert_eq!(
        provider
            .session_map
            .lock()
            .await
            .get("sess::stable")
            .cloned()
            .as_deref(),
        Some("session-a")
    );
}

#[tokio::test]
async fn test_run_streaming_reports_incomplete_stream_as_unsuccessful() {
    let mut provider = make_provider();
    provider.ready = true;
    provider
        .session_map
        .lock()
        .await
        .insert("sess::incomplete".to_owned(), "session-a".to_owned());
    provider
        .enqueue_test_run_attempt(Ok(Some(SdkRunOutcome {
            session_id: "session-a".to_owned(),
            response_text: "I will continue from here.".to_owned(),
            session_messages: Vec::new(),
            is_error: true,
            error_message: Some(CLAUDE_MISSING_RESULT_ERROR.to_owned()),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            actual_model: Some("claude-opus-4-7".to_owned()),
        })))
        .await;

    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "sess::incomplete".to_owned(),
                message: "hello".to_owned(),
                workspace_dir: None,
                images: None,
                metadata: HashMap::new(),
            },
            Box::new(|_| {}),
        )
        .await
        .expect("incomplete streams should preserve partial response");

    assert!(!result.success);
    assert_eq!(result.response, "I will continue from here.");
    assert_eq!(result.error.as_deref(), Some(CLAUDE_MISSING_RESULT_ERROR));
    assert_eq!(result.sdk_session_id.as_deref(), Some("session-a"));
}

#[tokio::test]
async fn test_auto_recover_clears_session_and_failures() {
    let provider = make_provider();

    // Set up a session with some state
    provider
        .session_map
        .lock()
        .await
        .insert("sess::a".to_owned(), "sdk-123".to_owned());
    provider
        .session_failure_counts
        .lock()
        .await
        .insert("sess::a".to_owned(), 2);
    provider
        .last_messages
        .lock()
        .await
        .insert("sess::a".to_owned(), "hello world".to_owned());

    let replay = provider.auto_recover_session("sess::a").await;

    // Session should be cleared
    assert!(provider.session_map.lock().await.get("sess::a").is_none());
    // Failure count should be cleared
    assert!(
        provider
            .session_failure_counts
            .lock()
            .await
            .get("sess::a")
            .is_none()
    );
    // Should return the last message for replay
    assert_eq!(replay, Some("hello world".to_owned()));
}

#[tokio::test]
async fn test_auto_recover_no_last_message() {
    let provider = make_provider();

    provider
        .session_map
        .lock()
        .await
        .insert("sess::b".to_owned(), "sdk-456".to_owned());

    let replay = provider.auto_recover_session("sess::b").await;
    assert!(replay.is_none());
}

#[tokio::test]
async fn test_add_streaming_input_no_session() {
    let provider = make_provider();
    // Should return false when no streaming session exists
    assert!(
        !provider
            .add_streaming_input("nonexistent", QueuedUserInput::text("hello"))
            .await
    );
}

#[tokio::test]
async fn test_add_streaming_input_with_run_mapping_but_no_handle() {
    let provider = make_provider();
    provider
        .run_session_map
        .lock()
        .await
        .insert("run-1".to_owned(), "sess::a".to_owned());
    provider.run_pending_inputs.lock().await.insert(
        "run-1".to_owned(),
        VecDeque::from([PendingAckMarker::RootUserMessage]),
    );
    assert!(
        !provider
            .add_streaming_input(
                "sess::a",
                QueuedUserInput::text("follow-up message").with_pending_input_id("queued-1"),
            )
            .await
    );
    assert!(provider.run_session_map.lock().await.get("run-1").is_none());
    assert!(
        provider
            .run_pending_inputs
            .lock()
            .await
            .get("run-1")
            .is_none()
    );
}

#[tokio::test]
async fn test_interrupt_streaming_session_no_session() {
    let provider = make_provider();
    assert!(!provider.interrupt_streaming_session("nonexistent").await);
}

#[tokio::test]
async fn test_interrupt_streaming_session_cleans_stale_run_mapping() {
    let provider = make_provider();
    provider
        .run_session_map
        .lock()
        .await
        .insert("run-1".to_owned(), "sess::stale".to_owned());
    provider.run_pending_inputs.lock().await.insert(
        "run-1".to_owned(),
        VecDeque::from([
            PendingAckMarker::RootUserMessage,
            PendingAckMarker::QueuedInput("queued-1".to_owned()),
        ]),
    );

    assert!(!provider.interrupt_streaming_session("sess::stale").await);
    assert!(provider.run_session_map.lock().await.get("run-1").is_none());
    assert!(
        provider
            .run_pending_inputs
            .lock()
            .await
            .get("run-1")
            .is_none()
    );
}

#[tokio::test]
async fn test_clear_session_cleans_all_state() {
    let provider = make_provider();

    // Set up all the state
    provider
        .session_map
        .lock()
        .await
        .insert("sess::x".to_owned(), "sdk-789".to_owned());
    provider
        .session_failure_counts
        .lock()
        .await
        .insert("sess::x".to_owned(), 1);
    provider
        .last_messages
        .lock()
        .await
        .insert("sess::x".to_owned(), "last msg".to_owned());
    provider
        .run_session_map
        .lock()
        .await
        .insert("run-stale".to_owned(), "sess::x".to_owned());
    provider.run_pending_inputs.lock().await.insert(
        "run-stale".to_owned(),
        VecDeque::from([PendingAckMarker::RootUserMessage]),
    );

    assert!(provider.clear_session("sess::x").await);

    // All state should be cleaned
    assert!(provider.session_map.lock().await.get("sess::x").is_none());
    assert!(
        provider
            .session_failure_counts
            .lock()
            .await
            .get("sess::x")
            .is_none()
    );
    assert!(provider.last_messages.lock().await.get("sess::x").is_none());
    assert!(
        provider
            .run_session_map
            .lock()
            .await
            .get("run-stale")
            .is_none()
    );
    assert!(
        provider
            .run_pending_inputs
            .lock()
            .await
            .get("run-stale")
            .is_none()
    );
}

#[tokio::test]
async fn test_run_session_map_tracking() {
    let provider = make_provider();

    provider
        .run_session_map
        .lock()
        .await
        .insert("run-1".to_owned(), "sess::a".to_owned());

    let map = provider.run_session_map.lock().await;
    assert_eq!(map.get("run-1"), Some(&"sess::a".to_owned()));
    drop(map);

    provider.run_session_map.lock().await.remove("run-1");
    let map = provider.run_session_map.lock().await;
    assert!(map.get("run-1").is_none());
}

// -----------------------------------------------------------------------
// try_close_pending_inputs – atomic check-and-close
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_try_close_pending_inputs_closes_when_empty() {
    let provider = make_provider();
    // Initialize with only the root marker (no queued inputs).
    provider.initialize_pending_inputs("run-1").await;

    // Root marker alone → should close (root is not a QueuedInput).
    assert!(provider.try_close_pending_inputs("run-1").await);

    // After close, enqueue must fail because the entry was removed.
    assert!(!provider.enqueue_pending_input("run-1", "qi-1".into()).await);
}

#[tokio::test]
async fn test_try_close_pending_inputs_blocks_when_queued_input_present() {
    let provider = make_provider();
    provider.initialize_pending_inputs("run-2").await;
    assert!(provider.enqueue_pending_input("run-2", "qi-1".into()).await);

    // There is a queued input → should NOT close.
    assert!(!provider.try_close_pending_inputs("run-2").await);

    // Enqueue still works — the entry was not removed.
    assert!(provider.enqueue_pending_input("run-2", "qi-2".into()).await);
}

#[tokio::test]
async fn test_try_close_pending_inputs_idempotent_on_absent_run() {
    let provider = make_provider();
    // No pending inputs entry at all → trivially closed.
    assert!(provider.try_close_pending_inputs("nonexistent").await);
}

#[tokio::test]
async fn test_enqueue_after_close_fails() {
    let provider = make_provider();
    provider.initialize_pending_inputs("run-3").await;

    // Close the run.
    assert!(provider.try_close_pending_inputs("run-3").await);

    // Subsequent enqueue must fail (this is the fix for the race).
    let enqueued = provider
        .enqueue_pending_input("run-3", "qi-late".into())
        .await;
    assert!(
        !enqueued,
        "enqueue must fail after try_close_pending_inputs removed the entry"
    );

    // unregister_run should not panic even though entry is already gone.
    let (run_handle, thread_id) = provider.unregister_run("run-3").await;
    assert!(run_handle.is_none());
    assert!(thread_id.is_none());
}
