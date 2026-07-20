use super::*;
use crate::gary_prompt::GARY_BASE_INSTRUCTIONS;
use crate::native_slash::build_native_skill_prompt;
use claude_agent_sdk::{
    AssistantMessage, MessageOrigin, ResultMessage, SystemMessage, ToolResultBlock, ToolUseBlock,
    UserContent, UserInput, UserMessage,
};
use garyx_models::provider::{ClaudeCodeConfig, QueuedUserInput};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn make_provider() -> ClaudeCliProvider {
    ClaudeCliProvider::new(ClaudeCodeConfig::default())
}

fn pending_ack_queue(pending_input_ids: &[&str]) -> PendingAckQueue {
    let mut queue = PendingAckQueue::with_root_user_message();
    for pending_input_id in pending_input_ids {
        queue.enqueue((*pending_input_id).to_owned());
    }
    queue
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
    assert_eq!(
        provider.clear_session("sess::a").await,
        ClearSessionOutcome::Cleared
    );
    assert_eq!(
        provider.clear_session("sess::a").await,
        ClearSessionOutcome::AlreadyAbsent
    );

    let new_sid = provider.get_or_create_session("sess::a").await.unwrap();
    assert!(!new_sid.is_empty());
}

#[tokio::test]
async fn test_provider_type() {
    let provider = make_provider();
    assert_eq!(provider.provider_type(), ProviderType::ClaudeCode);
}

#[test]
fn test_result_usage_tokens_accepts_claude_sdk_snake_case_usage() {
    let mut usage = HashMap::new();
    usage.insert("input_tokens".to_owned(), Value::from(12));
    usage.insert("cache_creation_input_tokens".to_owned(), Value::from(3));
    usage.insert("cache_read_input_tokens".to_owned(), Value::from(5));
    usage.insert("output_tokens".to_owned(), Value::from(34));

    assert_eq!(result_usage_tokens(Some(&usage)), (20, 34));
}

#[test]
fn test_claude_background_task_tracking_accepts_official_task_messages() {
    let mut active_background_tasks = HashSet::new();

    update_claude_background_tasks(
        &SystemMessage {
            subtype: "task_started".to_owned(),
            data: json!({
                "task_id": "task-1",
                "tool_use_id": "toolu_1",
            }),
        },
        &mut active_background_tasks,
    );
    assert!(active_background_tasks.contains("task-1"));

    update_claude_background_tasks(
        &SystemMessage {
            subtype: "task_updated".to_owned(),
            data: json!({
                "task_id": "task-1",
                "patch": {
                    "status": "running"
                }
            }),
        },
        &mut active_background_tasks,
    );
    assert!(active_background_tasks.contains("task-1"));

    update_claude_background_tasks(
        &SystemMessage {
            subtype: "task_updated".to_owned(),
            data: json!({
                "task_id": "task-1",
                "patch": {
                    "status": "killed"
                }
            }),
        },
        &mut active_background_tasks,
    );
    assert!(!active_background_tasks.contains("task-1"));

    update_claude_background_tasks(
        &SystemMessage {
            subtype: "task_notification".to_owned(),
            data: json!({
                "task_id": "task-untracked",
                "status": "completed",
            }),
        },
        &mut active_background_tasks,
    );
    assert!(
        active_background_tasks.is_empty(),
        "terminal notifications for untracked tasks should remain a no-op"
    );
}

#[test]
fn test_claude_background_task_tracking_accepts_camel_case_fallbacks() {
    let mut active_background_tasks = HashSet::new();

    update_claude_background_tasks(
        &SystemMessage {
            subtype: "task_started".to_owned(),
            data: json!({
                "taskId": "task-camel",
            }),
        },
        &mut active_background_tasks,
    );
    assert!(active_background_tasks.contains("task-camel"));

    update_claude_background_tasks(
        &SystemMessage {
            subtype: "task_notification".to_owned(),
            data: json!({
                "toolUseId": "toolu-camel",
                "status": "completed",
            }),
        },
        &mut active_background_tasks,
    );
    assert!(
        !active_background_tasks.contains("toolu-camel"),
        "terminal camelCase fallback notifications should not create active tasks"
    );
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
fn test_extract_claude_thread_title_from_session_name() {
    let data = json!({
        "type": "system",
        "subtype": "init",
        "session_id": "session-1",
        "session_name": "  Review   provider title plumbing  "
    });

    assert_eq!(
        extract_claude_thread_title(&data).as_deref(),
        Some("Review provider title plumbing")
    );
}

#[test]
fn test_extract_claude_ai_title_from_transcript_line() {
    let line =
        r#"{"type":"ai-title","aiTitle":"  Reply  with exactly ok  ","sessionId":"session-1"}"#;

    assert_eq!(
        extract_claude_ai_title_line(line, "session-1").as_deref(),
        Some("Reply with exactly ok")
    );
    assert!(extract_claude_ai_title_line(line, "other-session").is_none());
}

#[tokio::test]
async fn test_read_claude_ai_title_from_transcript_path_uses_latest_title() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("session-1.jsonl");
    fs::write(
        &transcript,
        concat!(
            r#"{"type":"ai-title","aiTitle":"Old title","sessionId":"session-1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[]}}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"New title","sessionId":"session-1"}"#,
            "\n",
        ),
    )
    .unwrap();

    assert_eq!(
        read_claude_ai_title_from_transcript_path(&transcript, "session-1")
            .await
            .as_deref(),
        Some("New title")
    );
}

#[tokio::test]
async fn test_count_claude_transcript_history_messages_at_path() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("session-1.jsonl");
    fs::write(
        &transcript,
        concat!(
            r#"{"type":"system","subtype":"init","session_id":"session-1"}"#,
            "\n",
            r#"{"type":"user","message":{"content":"hello"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
            "\n",
            r#"{"type":"result","session_id":"session-1"}"#,
            "\n",
        ),
    )
    .unwrap();

    assert_eq!(
        count_claude_transcript_history_messages_at_path(&transcript).await,
        Some(2)
    );
}

#[test]
fn test_claude_transcript_path_matches_observed_project_dir_shape() {
    let path = claude_transcript_path(
        Path::new("/Users/example/.claude"),
        Path::new("/Users/example/repos/Garyx"),
        "session-1",
    );

    assert_eq!(
        path,
        PathBuf::from("/Users/example/.claude/projects/-Users-example-repos-Garyx/session-1.jsonl")
    );
}

#[test]
fn test_claude_transcript_path_sanitizes_hidden_temp_dirs() {
    let path = claude_transcript_path(
        Path::new("/Users/test/.claude"),
        Path::new("/tmp/.garyx-tty/smoke.test"),
        "session-1",
    );

    assert_eq!(
        path,
        PathBuf::from("/Users/test/.claude/projects/-tmp--garyx-tty-smoke-test/session-1.jsonl")
    );
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
    assert!(should_retry_with_fresh_session(&BridgeError::RunFailed(
        "Control protocol error: CLI process exited before responding".to_owned()
    )));
    assert!(should_retry_with_fresh_session(&BridgeError::RunFailed(
        "no result from claude SDK".to_owned()
    )));
    assert!(should_retry_with_fresh_session(&BridgeError::RunFailed(
        "Error: session not found".to_owned()
    )));
    assert!(!should_retry_with_fresh_session(&BridgeError::RunFailed(
        "permission denied".to_owned()
    )));
    assert!(!should_retry_with_fresh_session(
        &BridgeError::SessionParseUnsupportedBlock(
            "Unknown content block type: document".to_owned()
        )
    ));
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
    assert!(
        sdk_opts
            .disallowed_tools
            .contains(&"ScheduleWakeup".to_string())
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

    // Stop-hook observer: the production wiring behind the post-result
    // stdin-close gate. Without it the provider never receives stop-hook
    // observations and the truncation protection silently degrades to
    // stream events only.
    assert!(sdk_opts.stop_hook_observer);

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
fn test_build_sdk_options_prefers_configured_claude_cli_path() {
    let dir = tempfile::tempdir().unwrap();
    let cli_path = dir.path().join("cctty");
    fs::write(&cli_path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&cli_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&cli_path, perms).unwrap();
    }

    let provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        claude_cli_mode: "cctty".to_owned(),
        claude_cli_path: Some(cli_path.to_string_lossy().into_owned()),
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
    assert_eq!(sdk_opts.cli_path.as_deref(), Some(cli_path.as_path()));
    assert!(sdk_opts.cli_prefix_args.is_empty());
}

#[test]
fn test_build_sdk_options_native_mode_uses_sdk_default_cli_discovery() {
    let provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        claude_cli_mode: "native".to_owned(),
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
    assert!(sdk_opts.cli_path.is_none());
    assert!(sdk_opts.cli_prefix_args.is_empty());
}

#[test]
fn test_build_sdk_options_cctty_mode_uses_embedded_runner() {
    let provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        claude_cli_mode: "cctty".to_owned(),
        claude_cli_path: None,
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
    assert_eq!(sdk_opts.cli_path, std::env::current_exe().ok());
    assert_eq!(sdk_opts.cli_prefix_args, vec!["__cctty"]);
}

#[test]
fn test_build_sdk_options_maps_auto_to_default_permissions() {
    let config = ClaudeCodeConfig {
        permission_mode: "auto".to_owned(),
        ..Default::default()
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

    assert_eq!(sdk_opts.permission_mode, Some(PermissionMode::Auto));
}

#[test]
fn test_build_sdk_options_maps_dont_ask_to_bypass_permissions() {
    let config = ClaudeCodeConfig {
        permission_mode: "dontAsk".to_owned(),
        ..Default::default()
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
                "X-Gary-Test-Role": "verifier"
            }),
        )]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    match &sdk_opts.mcp_servers["garyx"] {
        McpServerConfig::Http { headers, .. } => {
            assert_eq!(
                headers.get("X-Gary-Test-Role").map(String::as_str),
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
    assert!(!sdk_opts.fork_session);
}

#[test]
fn test_build_sdk_options_enables_fork_session_from_metadata() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(SDK_SESSION_FORK_METADATA_KEY.to_owned(), Value::Bool(true))]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, Some("parent-session"), "run-1");
    assert_eq!(sdk_opts.resume.as_deref(), Some("parent-session"));
    assert!(sdk_opts.fork_session);
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
fn test_build_sdk_options_custom_agent_without_prompt_uses_provider_default() {
    let provider = ClaudeCliProvider::new(ClaudeCodeConfig {
        system_prompt: Some("Provider-level override must not apply.".to_owned()),
        ..ClaudeCodeConfig::default()
    });
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
        ]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    assert!(sdk_opts.agent.is_none());
    assert!(sdk_opts.agents.is_empty());
    assert!(sdk_opts.system_prompt.is_none());
    assert!(sdk_opts.append_system_prompt.is_none());
}

#[test]
fn test_build_sdk_options_custom_agent_blank_prompt_uses_provider_default() {
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
            ("system_prompt".to_owned(), Value::String("   ".to_owned())),
        ]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");

    assert!(sdk_opts.agent.is_none());
    assert!(sdk_opts.agents.is_empty());
    assert!(sdk_opts.system_prompt.is_none());
    assert!(sdk_opts.append_system_prompt.is_none());
}

#[test]
fn test_build_sdk_options_builtin_claude_agent_uses_garyx_default_path() {
    let provider = make_provider();
    let opts = ProviderRunOptions {
        thread_id: "thread::agent".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), Value::String("claude".to_owned()))]),
    };

    let sdk_opts = provider.build_sdk_options(&opts, None, "run-1");
    let system_prompt = sdk_opts.system_prompt.unwrap_or_default();

    assert!(sdk_opts.agent.is_none());
    assert!(sdk_opts.agents.is_empty());
    assert!(sdk_opts.append_system_prompt.is_none());
    assert!(system_prompt.starts_with(GARY_BASE_INSTRUCTIONS.trim_end()));
    assert!(system_prompt.contains("Garyx runtime guidance:"));
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
        "provider_env".to_string(),
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
        "provider_env".to_string(),
        serde_json::json!({
            "CLAUDE_CODE_OAUTH_TOKEN": "from-provider",
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
        Some("from-provider")
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
    assert!(!system_prompt.contains("</garyx_memory_context>"));
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
    assert!(!system_prompt.contains("</garyx_memory_context>"));
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

    process_assistant_blocks_streaming(&blocks, &mut response_text, &mut session_messages, &cb);

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
fn test_claude_envelope_scope_predicate_covers_nested_shapes() {
    let cases = vec![
        (
            "top-level Agent use",
            None,
            vec![ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_agent".to_owned(),
                name: "Agent".to_owned(),
                input: json!({}),
            })],
            false,
        ),
        (
            "empty parent",
            Some(""),
            vec![ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_agent".to_owned(),
                name: "Agent".to_owned(),
                input: json!({}),
            })],
            false,
        ),
        (
            "whitespace parent",
            Some(" \n\t "),
            vec![ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_agent".to_owned(),
                name: "Agent".to_owned(),
                input: json!({}),
            })],
            false,
        ),
        (
            "nested use",
            Some("toolu_agent"),
            vec![ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_child".to_owned(),
                name: "Bash".to_owned(),
                input: json!({}),
            })],
            true,
        ),
        (
            "self-parent top-level result",
            Some("toolu_agent"),
            vec![ContentBlock::ToolResult(ToolResultBlock {
                tool_use_id: "toolu_agent".to_owned(),
                content: Some(json!("done")),
                is_error: Some(false),
            })],
            false,
        ),
        (
            "second-level nested result",
            Some("toolu_child_agent"),
            vec![ContentBlock::ToolResult(ToolResultBlock {
                tool_use_id: "toolu_grandchild".to_owned(),
                content: Some(json!("done")),
                is_error: Some(false),
            })],
            true,
        ),
        (
            "nested text envelope",
            Some("toolu_agent"),
            vec![ContentBlock::Text(TextBlock {
                text: "internal".to_owned(),
            })],
            true,
        ),
    ];

    for (name, parent, blocks, expected) in cases {
        assert_eq!(
            is_nested_claude_envelope(parent, &blocks),
            expected,
            "{name}"
        );
    }
}

#[test]
fn test_extract_tool_session_messages_writes_no_parent_scope_metadata() {
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
    extract_tool_session_messages(&blocks, &mut session_messages, None);

    assert_eq!(session_messages.len(), 2);
    for message in &session_messages {
        assert!(
            !message.metadata.contains_key("parent_tool_use_id"),
            "accepted top-level messages do not persist scope metadata"
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
        origin: None,
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
        origin: None,
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

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider.set_pending_inputs("run-1", 1).await;
    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-1", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

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
            // The successful result finalizes the in-flight assistant tail
            // immediately (result-time finalize, #TASK-1715).
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::AssistantSegment,
                pending_input_id: None,
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
    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-no-result", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "partial progress");
    assert!(
        result_data.is_none(),
        "Claude text/tool events alone must not be treated as a completed run"
    );
}

#[tokio::test]
async fn test_process_messages_streaming_keeps_input_queue_open_during_post_result_grace() {
    let provider = Arc::new(make_provider());
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);

    provider.set_pending_inputs("run-post-result", 1).await;

    let provider_for_task = provider.clone();
    let cb: StreamCallback = Box::new(|_| {});
    let task = tokio::spawn(async move {
        provider_for_task
            .process_messages_streaming("run-post-result", "thread::test", &mut rx, &cb)
            .await
    });

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("first user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "first".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        provider
            .run_pending_inputs
            .lock()
            .await
            .contains_key("run-post-result"),
        "first result should not close the streaming input queue immediately"
    );

    provider
        .run_pending_inputs
        .lock()
        .await
        .get_mut("run-post-result")
        .expect("pending queue should still exist")
        .enqueue("queued-1".to_owned());
    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("queued user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: None,
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
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let (response_text, result_data, _signals) = task
        .await
        .expect("processing task should not panic")
        .expect("stream should process");
    assert_eq!(response_text, "first\n\nsecond");
    assert_eq!(
        result_data.expect("expected final result").session_id,
        "sdk-session-2"
    );
}

#[tokio::test]
async fn test_process_messages_streaming_waits_for_background_task_notification_after_result() {
    let provider = Arc::new(make_provider());
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);

    provider.set_pending_inputs("run-background-task", 1).await;

    let provider_for_task = provider.clone();
    let cb: StreamCallback = Box::new(|_| {});
    let task = tokio::spawn(async move {
        provider_for_task
            .process_messages_streaming("run-background-task", "thread::test", &mut rx, &cb)
            .await
    });

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("start background task".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::System(SystemMessage {
        subtype: "task_started".to_owned(),
        data: json!({
            "type": "system",
            "subtype": "task_started",
            "task_id": "task-1",
            "tool_use_id": "toolu_1",
            "task_type": "local_bash"
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "started".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        provider
            .run_pending_inputs
            .lock()
            .await
            .contains_key("run-background-task"),
        "active Claude background tasks should keep the stream input queue open after result"
    );

    tx.send(Ok(Message::System(SystemMessage {
        subtype: "task_updated".to_owned(),
        data: json!({
            "type": "system",
            "subtype": "task_updated",
            "task_id": "task-1",
            "tool_use_id": "toolu_1",
            "patch": {
                "status": "completed"
            }
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::System(SystemMessage {
        subtype: "task_notification".to_owned(),
        data: json!({
            "type": "system",
            "subtype": "task_notification",
            "task_id": "task-1",
            "tool_use_id": "toolu_1",
            "status": "completed",
            "summary": "background task completed"
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "completed".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let (response_text, result_data, _signals) = task
        .await
        .expect("processing task should not panic")
        .expect("stream should process");
    assert_eq!(response_text, "started\n\ncompleted");
    assert_eq!(
        result_data.expect("expected final result").session_id,
        "sdk-session-2"
    );
}

/// Deadline-scripted source for deterministic post-result lifecycle tests.
/// A cancelled read leaves the pending frame in place, matching CLI stdout.
struct ScriptedMessageSource {
    items: VecDeque<(tokio::time::Instant, claude_agent_sdk::Result<Message>)>,
    end_input_calls: usize,
    /// Number of not-yet-consumed scripted items when `end_input` first ran.
    /// A premature stdin close (the truncation bug shape) leaves the
    /// follow-up script unconsumed at close time.
    remaining_at_first_end_input: Option<usize>,
}

impl ScriptedMessageSource {
    fn new(items: Vec<(u64, claude_agent_sdk::Result<Message>)>) -> Self {
        let start = tokio::time::Instant::now();
        Self {
            items: items
                .into_iter()
                .map(|(offset_ms, message)| (start + Duration::from_millis(offset_ms), message))
                .collect(),
            end_input_calls: 0,
            remaining_at_first_end_input: None,
        }
    }
}

#[async_trait::async_trait]
impl MessageSource for ScriptedMessageSource {
    async fn next_message(&mut self) -> Option<claude_agent_sdk::Result<Message>> {
        let deadline = self.items.front()?.0;
        tokio::time::sleep_until(deadline).await;
        self.items.pop_front().map(|(_, message)| message)
    }

    async fn end_input(&mut self) -> claude_agent_sdk::Result<()> {
        self.end_input_calls += 1;
        self.remaining_at_first_end_input
            .get_or_insert(self.items.len());
        Ok(())
    }
}

fn scripted_user_text(text: &str) -> claude_agent_sdk::Result<Message> {
    Ok(Message::User(UserMessage {
        content: UserContent::Text(text.to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: None,
    }))
}

fn scripted_task_notification_user(text: &str) -> claude_agent_sdk::Result<Message> {
    Ok(Message::User(UserMessage {
        content: UserContent::Text(text.to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: Some(MessageOrigin {
            kind: "task-notification".to_owned(),
            metadata: HashMap::new(),
        }),
    }))
}

fn scripted_assistant_text(text: &str) -> claude_agent_sdk::Result<Message> {
    Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: text.to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    }))
}

fn scripted_success_result(session_id: &str) -> claude_agent_sdk::Result<Message> {
    Ok(Message::Result(Box::new(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: session_id.to_owned(),
        total_cost_usd: Some(0.0),
        ..Default::default()
    })))
}

fn scripted_system(subtype: &str, data: Value) -> claude_agent_sdk::Result<Message> {
    Ok(Message::System(SystemMessage {
        subtype: subtype.to_owned(),
        data,
    }))
}

/// A level update may empty the live-task set before the completion edge and
/// its injected follow-up turn. Model latency can exceed the input drain
/// window, but the future turn must still be consumed.
#[tokio::test(start_paused = true)]
async fn test_process_messages_streaming_survives_followup_gap_after_empty_background_set() {
    let provider = make_provider();
    provider.set_pending_inputs("run-followup-gap", 1).await;

    let mut source = ScriptedMessageSource::new(vec![
        (0, scripted_user_text("start background task")),
        (
            0,
            scripted_system(
                "task_started",
                json!({
                    "type": "system",
                    "subtype": "task_started",
                    "task_id": "task-3",
                    "tool_use_id": "toolu_3",
                    "task_type": "local_agent",
                }),
            ),
        ),
        (0, scripted_assistant_text("interim summary")),
        (0, scripted_success_result("sdk-session-1")),
        (
            50,
            scripted_system(
                "background_tasks_changed",
                json!({
                    "type": "system",
                    "subtype": "background_tasks_changed",
                    "tasks": [],
                }),
            ),
        ),
        (
            4_000,
            scripted_system(
                "task_notification",
                json!({
                    "type": "system",
                    "subtype": "task_notification",
                    "task_id": "task-3",
                    "tool_use_id": "toolu_3",
                    "status": "completed",
                }),
            ),
        ),
        (
            4_010,
            scripted_task_notification_user("<task-notification>completed</task-notification>"),
        ),
        (4_020, scripted_assistant_text("follow-up summary")),
        (4_030, scripted_success_result("sdk-session-2")),
    ]);

    let cb: StreamCallback = Box::new(|_| {});
    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-followup-gap", "thread::test", &mut source, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "interim summary\n\nfollow-up summary");
    assert_eq!(
        result_data.expect("expected final result").session_id,
        "sdk-session-2"
    );
    assert_eq!(source.end_input_calls, 1, "stdin should close exactly once");
}

/// The completion edge is not ordered relative to the interim result. If it
/// arrives first, a later result must not re-arm an output-closing timer that
/// can discard the injected follow-up turn.
#[tokio::test(start_paused = true)]
async fn test_process_messages_streaming_survives_gap_when_task_edge_precedes_result() {
    let provider = make_provider();
    provider.set_pending_inputs("run-edge-first", 1).await;

    let mut source = ScriptedMessageSource::new(vec![
        (0, scripted_user_text("start background task")),
        (
            0,
            scripted_system(
                "task_started",
                json!({
                    "type": "system",
                    "subtype": "task_started",
                    "task_id": "task-3",
                    "tool_use_id": "toolu_3",
                    "task_type": "local_agent",
                }),
            ),
        ),
        (0, scripted_assistant_text("interim summary")),
        (
            0,
            scripted_system(
                "background_tasks_changed",
                json!({
                    "type": "system",
                    "subtype": "background_tasks_changed",
                    "tasks": [],
                }),
            ),
        ),
        (
            0,
            scripted_system(
                "task_notification",
                json!({
                    "type": "system",
                    "subtype": "task_notification",
                    "task_id": "task-3",
                    "tool_use_id": "toolu_3",
                    "status": "completed",
                }),
            ),
        ),
        (50, scripted_success_result("sdk-session-1")),
        (
            4_000,
            scripted_task_notification_user("<task-notification>completed</task-notification>"),
        ),
        (4_010, scripted_assistant_text("follow-up summary")),
        (4_020, scripted_success_result("sdk-session-2")),
    ]);

    let cb: StreamCallback = Box::new(|_| {});
    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-edge-first", "thread::test", &mut source, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "interim summary\n\nfollow-up summary");
    assert_eq!(
        result_data.expect("expected final result").session_id,
        "sdk-session-2"
    );
    assert_eq!(source.end_input_calls, 1, "stdin should close exactly once");
}

fn scripted_stop_hook_observation(background_tasks: Value) -> claude_agent_sdk::Result<Message> {
    scripted_system(
        STOP_HOOK_OBSERVATION_SUBTYPE,
        json!({
            "type": "system",
            "subtype": STOP_HOOK_OBSERVATION_SUBTYPE,
            "input": {
                "hook_event_name": "Stop",
                "stop_hook_active": false,
                "background_tasks": background_tasks,
            },
        }),
    )
}

/// Inert far-future row keeping the scripted stream pending so the
/// post-result input-drain timer can actually fire: an exhausted script
/// returns `None` instantly, which ends the loop before any drain and would
/// mask whether the gate held. First-end_input bookkeeping then reads
/// `Some(1)` when stdin closed with only this sentinel left unconsumed.
fn scripted_far_future_sentinel() -> (u64, claude_agent_sdk::Result<Message>) {
    (
        60_000,
        scripted_system("status", json!({"type": "system", "subtype": "status"})),
    )
}

/// The production truncation shape (#thread bd57f9d3, 2026-07-20): the CLI's
/// level/edge stream events for a live background task never made it into the
/// tracked set, so the post-result drain closed stdin and the CLI teardown
/// killed the task. The stop-hook observation is computed by the CLI at the
/// stop decision point and must hold stdin open on its own, without any
/// task_started/background_tasks_changed stream event.
#[tokio::test(start_paused = true)]
async fn test_stop_hook_observation_holds_stdin_without_stream_task_events() {
    let provider = make_provider();
    provider
        .initialize_pending_inputs("run-stop-hook-hold")
        .await;

    let mut source = ScriptedMessageSource::new(vec![
        (0, scripted_user_text("start tier2 in background")),
        (0, scripted_assistant_text("interim summary")),
        (
            10,
            scripted_stop_hook_observation(json!([
                {"id": "bg-1", "type": "shell", "status": "running"}
            ])),
        ),
        (20, scripted_success_result("sdk-session-1")),
        // Far beyond the 2s input-drain window: the wake flow after the task
        // settles. Without the stop-hook hold the drain would have closed
        // stdin long before these rows.
        (
            6_000,
            scripted_system(
                "task_notification",
                json!({
                    "type": "system",
                    "subtype": "task_notification",
                    "task_id": "bg-1",
                    "status": "completed",
                }),
            ),
        ),
        (
            6_010,
            scripted_task_notification_user("<task-notification>completed</task-notification>"),
        ),
        (6_020, scripted_assistant_text("follow-up summary")),
        (6_030, scripted_stop_hook_observation(json!([]))),
        (6_040, scripted_success_result("sdk-session-2")),
        scripted_far_future_sentinel(),
    ]);

    let cb: StreamCallback = Box::new(|_| {});
    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-stop-hook-hold", "thread::test", &mut source, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "interim summary\n\nfollow-up summary");
    assert_eq!(
        result_data.expect("expected final result").session_id,
        "sdk-session-2"
    );
    assert_eq!(source.end_input_calls, 1, "stdin should close exactly once");
    assert_eq!(
        source.remaining_at_first_end_input,
        Some(1),
        "stdin must only close after the wake turn fully drained \
         (a premature drain would close it with the wake rows still pending)"
    );
}

/// An observation whose entries are all terminal must not hold stdin open:
/// the defensive status filter treats it as "nothing in flight", and the
/// post-result drain closes stdin on schedule.
#[tokio::test(start_paused = true)]
async fn test_stop_hook_observation_with_terminal_entries_releases_stdin() {
    let provider = make_provider();
    provider
        .initialize_pending_inputs("run-stop-hook-terminal")
        .await;

    let mut source = ScriptedMessageSource::new(vec![
        (0, scripted_user_text("start background task")),
        (0, scripted_assistant_text("interim summary")),
        (
            10,
            scripted_stop_hook_observation(json!([
                {"id": "bg-1", "type": "shell", "status": "completed"},
                {"id": "bg-2", "type": "shell", "status": "killed"}
            ])),
        ),
        (20, scripted_success_result("sdk-session-1")),
        scripted_far_future_sentinel(),
    ]);

    let cb: StreamCallback = Box::new(|_| {});
    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-stop-hook-terminal", "thread::test", &mut source, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "interim summary");
    assert_eq!(
        result_data.expect("expected result").session_id,
        "sdk-session-1"
    );
    assert_eq!(source.end_input_calls, 1, "stdin should close exactly once");
    assert_eq!(
        source.remaining_at_first_end_input,
        Some(1),
        "terminal-only entries must not delay the post-result drain"
    );
}

/// A later stop with an empty in-flight list releases a hold set by an
/// earlier stop in the same run (wake-turn convergence).
#[tokio::test(start_paused = true)]
async fn test_stop_hook_observation_empty_list_releases_prior_hold() {
    let provider = make_provider();
    provider
        .initialize_pending_inputs("run-stop-hook-release")
        .await;

    let mut source = ScriptedMessageSource::new(vec![
        (0, scripted_assistant_text("interim summary")),
        (
            10,
            scripted_stop_hook_observation(json!([
                {"id": "bg-1", "type": "shell", "status": "running"}
            ])),
        ),
        (20, scripted_success_result("sdk-session-1")),
        (3_000, scripted_stop_hook_observation(json!([]))),
        scripted_far_future_sentinel(),
    ]);

    let cb: StreamCallback = Box::new(|_| {});
    let (response_text, _result_data, _signals) = provider
        .process_messages_streaming("run-stop-hook-release", "thread::test", &mut source, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "interim summary");
    assert_eq!(source.end_input_calls, 1, "stdin should close exactly once");
    assert_eq!(
        source.remaining_at_first_end_input,
        Some(1),
        "stdin must close only after the empty observation released the hold"
    );
}

/// Protocol-permitted no-wake edge: a stop-hook hold with a suppressed
/// continuation (no follow-up turn ever arrives) must NOT close stdin —
/// killing live background work is worse than holding — and the 1h stream
/// idle timeout is the deliberate backstop that eventually fails the run.
#[tokio::test(start_paused = true)]
async fn test_stop_hook_hold_without_wake_turn_hits_idle_backstop_without_closing_stdin() {
    let provider = make_provider();
    provider
        .initialize_pending_inputs("run-stop-hook-no-wake")
        .await;

    let mut source = ScriptedMessageSource::new(vec![
        (0, scripted_assistant_text("interim summary")),
        (
            10,
            scripted_stop_hook_observation(json!([
                {"id": "bg-1", "type": "shell", "status": "running"}
            ])),
        ),
        (20, scripted_success_result("sdk-session-1")),
        // The task's terminal notification arrives ...
        (
            5_000,
            scripted_system(
                "task_notification",
                json!({
                    "type": "system",
                    "subtype": "task_notification",
                    "task_id": "bg-1",
                    "status": "completed",
                }),
            ),
        ),
        // ... but the protocol suppressed the follow-up turn (real
        // `SDKInformationalMessage` shape carries `prevent_continuation`):
        // no wake rows, no fresh stop observation, ever.
        (
            5_000,
            scripted_system(
                "informational",
                json!({
                    "type": "system",
                    "subtype": "informational",
                    "content": "Stop hook denied continuation",
                    "level": "notice",
                    "prevent_continuation": true,
                }),
            ),
        ),
        // Pending far beyond the 1h idle ceiling so the timeout can fire.
        (
            4_000_000,
            scripted_system("status", json!({"type": "system", "subtype": "status"})),
        ),
    ]);

    let cb: StreamCallback = Box::new(|_| {});
    let started = tokio::time::Instant::now();
    let error = provider
        .process_messages_streaming("run-stop-hook-no-wake", "thread::test", &mut source, &cb)
        .await
        .expect_err("run should fail on the idle backstop");

    assert!(
        matches!(&error, BridgeError::RunFailed(message) if message.contains("idle")),
        "expected stream-idle failure, got: {error:?}"
    );
    // The last stream activity is the suppressed-continuation pair at t=5s;
    // the failure must land exactly one idle ceiling later, proving the 1h
    // backstop (and not some earlier teardown) is what ends the hold. The
    // 3605s literal is deliberate: deriving it from STREAM_IDLE_TIMEOUT_SECS
    // would let a shortened-timeout mutation pass unnoticed.
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(3_605),
        "the hold must end exactly at the 1h stream-idle ceiling"
    );
    assert_eq!(
        source.end_input_calls, 0,
        "stdin must never close while the stop hook reports live background work"
    );
}

#[test]
fn test_stop_hook_reports_background_work_parses_summary_shapes() {
    // Running entry (real CLI shape uses `id`, not `task_id`).
    assert_eq!(
        stop_hook_reports_background_work(&json!({
            "input": {"background_tasks": [{"id": "bg-1", "status": "running"}]}
        })),
        Some(true)
    );
    // Empty list: nothing in flight.
    assert_eq!(
        stop_hook_reports_background_work(&json!({
            "input": {"background_tasks": [], "session_crons": []}
        })),
        Some(false)
    );
    // Entry without a status stays conservative (holds).
    assert_eq!(
        stop_hook_reports_background_work(&json!({
            "input": {"background_tasks": [{"id": "bg-1"}]}
        })),
        Some(true)
    );
    // Terminal-only entries release.
    assert_eq!(
        stop_hook_reports_background_work(&json!({
            "input": {"background_tasks": [{"id": "bg-1", "status": "failed"}]}
        })),
        Some(false)
    );
    // Older CLI without the field: no signal at all.
    assert_eq!(
        stop_hook_reports_background_work(&json!({"input": {"hook_event_name": "Stop"}})),
        None
    );
    assert_eq!(stop_hook_reports_background_work(&json!({})), None);
}

struct NeverEndingMessageSource;

#[async_trait::async_trait]
impl MessageSource for NeverEndingMessageSource {
    async fn next_message(&mut self) -> Option<claude_agent_sdk::Result<Message>> {
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn test_process_messages_streaming_reports_idle_stream_as_failure() {
    let provider = make_provider();
    let mut source = NeverEndingMessageSource;
    let cb: StreamCallback = Box::new(|_| {});

    let error = provider
        .process_messages_streaming("run-idle", "thread::test", &mut source, &cb)
        .await
        .expect_err("an idle stream must use the forceful error cleanup path");

    assert!(
        matches!(&error, BridgeError::RunFailed(message) if message.contains("stream idle")),
        "unexpected idle-stream error: {error}"
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
        origin: None,
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
        origin: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    provider
        .run_pending_inputs
        .lock()
        .await
        .insert("run-queued".to_owned(), pending_ack_queue(&["queued-1"]));

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (_response_text, result_data, _signals) = provider
        .process_messages_streaming("run-queued", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

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
async fn test_process_messages_streaming_suppresses_claude_synthetic_no_response_placeholder() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::Text(TextBlock {
            text: "Continue from where you left off.".to_owned(),
        })]),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "No response requested.".to_owned(),
        })],
        model: "<synthetic>".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("real queued user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "真实回复".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-synthetic".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    provider
        .run_pending_inputs
        .lock()
        .await
        .insert("run-synthetic".to_owned(), pending_ack_queue(&["queued-1"]));

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-synthetic", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "真实回复");
    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(
        emitted,
        vec![
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: None,
            },
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: Some("queued-1".to_owned()),
            },
            StreamEvent::Delta {
                text: "真实回复".to_owned(),
            },
            // Result-time finalize of the in-flight tail (#TASK-1715).
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::AssistantSegment,
                pending_input_id: None,
            },
        ]
    );

    let result = result_data.expect("expected result message");
    assert_eq!(result.session_id, "sdk-session-synthetic");
    assert_eq!(result.session_messages.len(), 1);
    assert_eq!(result.session_messages[0].text.as_deref(), Some("真实回复"));
}

#[tokio::test]
async fn test_process_messages_streaming_preserves_non_synthetic_no_response_text() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "No response requested.".to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "success".to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error: false,
        num_turns: 1,
        session_id: "sdk-session-real-text".to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-real-text", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "No response requested.");
    assert_eq!(
        chunks.lock().expect("chunks mutex poisoned").as_slice(),
        &[
            StreamEvent::Delta {
                text: "No response requested.".to_owned(),
            },
            // Result-time finalize of the in-flight tail (#TASK-1715).
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::AssistantSegment,
                pending_input_id: None,
            },
        ]
    );
    let result = result_data.expect("expected result message");
    assert_eq!(result.session_messages.len(), 1);
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
        origin: None,
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

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, _result_data, _signals) = provider
        .process_messages_streaming("run-assistant-segment", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "让我先看看。\n\n好了，现在开始修。");
    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(emitted.len(), 5);
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
    // Result-time finalize of the in-flight tail (#TASK-1715).
    assert_eq!(
        emitted[4],
        StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        }
    );
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
        origin: None,
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

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider.set_pending_inputs("run-2", 1).await;
    let (response_text, _result_data, _signals) = provider
        .process_messages_streaming("run-2", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

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
        origin: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, _result_data, _signals) = provider
        .process_messages_streaming("run-tools", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

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
async fn test_process_messages_streaming_nested_envelopes_have_zero_main_stream_side_effects() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::ToolUse(ToolUseBlock {
            id: "toolu_parent_agent".to_owned(),
            name: "Agent".to_owned(),
            input: json!({"description": "Synthetic delegated check"}),
        })],
        model: "claude-top-level".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "toolu_parent_agent".to_owned(),
            content: Some(Value::String("Synthetic delegated result".to_owned())),
            is_error: Some(false),
        })]),
        uuid: None,
        // Claude uses this self-parent shape for a top-level tool result.
        parent_tool_use_id: Some("toolu_parent_agent".to_owned()),
        tool_use_result: None,
        origin: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "Top-level answer remains in flight.".to_owned(),
        })],
        model: "claude-top-level".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();

    // Background subagent activity arrives after top-level text. Before this
    // fix the assistant envelope inserted a segment boundary/separator and the
    // user result cleared assistant_text_in_flight, so Result failed to close
    // the real top-level tail.
    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![
            ContentBlock::Text(TextBlock {
                text: "Nested narration must be invisible.".to_owned(),
            }),
            ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_child_bash".to_owned(),
                name: "Bash".to_owned(),
                input: json!({"command": "printf synthetic"}),
            }),
        ],
        model: "claude-nested".to_owned(),
        parent_tool_use_id: Some("toolu_parent_agent".to_owned()),
        error: Some(AssistantMessageError::Overloaded),
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "toolu_child_bash".to_owned(),
            content: Some(Value::String("synthetic".to_owned())),
            is_error: Some(false),
        })]),
        uuid: None,
        parent_tool_use_id: Some("toolu_parent_agent".to_owned()),
        tool_use_result: None,
        origin: None,
    })))
    .await
    .unwrap();

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, result_data, signals) = provider
        .process_messages_streaming("run-subagent", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "Top-level answer remains in flight.");
    assert!(
        signals.last_assistant_error.is_none(),
        "nested envelopes cannot alter main-run error state"
    );

    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(emitted.len(), 4);
    assert!(matches!(&emitted[0], StreamEvent::ToolUse { message }
            if message.tool_name.as_deref() == Some("Agent")
                && message.tool_use_id.as_deref() == Some("toolu_parent_agent")));
    assert!(matches!(&emitted[1], StreamEvent::ToolResult { message }
            if message.tool_use_id.as_deref() == Some("toolu_parent_agent")));
    assert_eq!(
        emitted[2],
        StreamEvent::Delta {
            text: "Top-level answer remains in flight.".to_owned()
        }
    );
    // Result-time finalize of the in-flight tail (#TASK-1715).
    assert_eq!(
        emitted[3],
        StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
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
            .all(|entry| entry.tool_use_id.as_deref() != Some("toolu_child_bash"))
    );
    assert!(
        result
            .session_messages
            .iter()
            .all(|entry| !entry.metadata.contains_key("parent_tool_use_id"))
    );

    let persistence = crate::multi_provider::probe_provider_persistence(
        &result.session_messages,
        &response_text,
        &emitted,
    )
    .await;
    assert!(
        persistence.ledger_messages.iter().all(|message| {
            message.get("tool_use_id").and_then(Value::as_str) != Some("toolu_child_bash")
                && message.pointer("/metadata/parent_tool_use_id").is_none()
        }),
        "terminal reconcile must not reintroduce nested provider messages"
    );
    let parent_agent_rows = persistence
        .ledger_messages
        .iter()
        .filter(|message| {
            message.get("tool_use_id").and_then(Value::as_str) == Some("toolu_parent_agent")
        })
        .count();
    assert_eq!(parent_agent_rows, 2, "top-level Agent use/result persist");
    assert_eq!(
        persistence.ledger_seqs,
        (1..=persistence.ledger_seqs.len() as u64).collect::<Vec<_>>()
    );
    let committed_seqs = persistence
        .committed_events
        .iter()
        .map(|event| {
            assert_eq!(
                event.get("type").and_then(Value::as_str),
                Some("committed_message")
            );
            event.get("seq").and_then(Value::as_u64).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        committed_seqs, persistence.ledger_seqs,
        "write-then-emit committed_message seqs must match the gapless ledger"
    );
}

#[tokio::test]
async fn test_process_messages_streaming_suppresses_orphan_nested_result() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "Top-level tail".to_owned(),
        })],
        model: "claude-top-level".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();
    // No corresponding child use was observed: scope is decided entirely
    // from this result envelope, with no cross-record suppressed-id set.
    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Blocks(vec![ContentBlock::ToolResult(ToolResultBlock {
            tool_use_id: "toolu_orphan_child".to_owned(),
            content: Some(Value::String("internal".to_owned())),
            is_error: Some(false),
        })]),
        uuid: None,
        parent_tool_use_id: Some("toolu_parent_agent".to_owned()),
        tool_use_result: None,
        origin: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(result_message_with_error(
        "sdk-session-orphan-nested",
        false,
    ))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-orphan-nested", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(response_text, "Top-level tail");
    assert_eq!(
        chunks.lock().expect("chunks mutex poisoned").as_slice(),
        &[
            StreamEvent::Delta {
                text: "Top-level tail".to_owned(),
            },
            StreamEvent::Boundary {
                kind: StreamBoundaryKind::AssistantSegment,
                pending_input_id: None,
            },
        ]
    );
    let result = result_data.expect("expected result message");
    assert_eq!(result.session_messages.len(), 1);
    assert_eq!(result.session_messages[0].role_str(), "assistant");
}

fn result_message_with_error(session_id: &str, is_error: bool) -> Box<ResultMessage> {
    Box::new(ResultMessage {
        subtype: if is_error { "error" } else { "success" }.to_owned(),
        duration_ms: 1,
        duration_api_ms: 1,
        is_error,
        num_turns: 1,
        session_id: session_id.to_owned(),
        total_cost_usd: Some(0.0),
        usage: None,
        result: None,
        structured_output: None,
        ..Default::default()
    })
}

fn assistant_text_message(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: text.to_owned(),
        })],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    }
}

/// Error results must keep the old flow (no early finalize) so a
/// fresh-session retry never commits the doomed attempt's tail (#TASK-1715).
#[tokio::test]
async fn test_process_messages_streaming_error_result_skips_finalize_boundary() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::Assistant(assistant_text_message(
        "No conversation found",
    ))))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(result_message_with_error(
        "sdk-session-error",
        true,
    ))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider
        .process_messages_streaming("run-error-result", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(
        chunks.lock().expect("chunks mutex poisoned").as_slice(),
        &[StreamEvent::Delta {
            text: "No conversation found".to_owned(),
        }]
    );
}

/// A turn that ends on a tool event has no in-flight assistant tail, so the
/// result must not emit a redundant finalize boundary (#TASK-1715).
#[tokio::test]
async fn test_process_messages_streaming_tool_tail_result_skips_finalize_boundary() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![
            ContentBlock::Text(TextBlock {
                text: "运行一下。".to_owned(),
            }),
            ContentBlock::ToolUse(ToolUseBlock {
                id: "toolu_tail".to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({ "command": "true" }),
            }),
        ],
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(result_message_with_error(
        "sdk-session-tool-tail",
        false,
    ))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider
        .process_messages_streaming("run-tool-tail", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    assert_eq!(emitted.len(), 2);
    assert!(matches!(
        &emitted[0],
        StreamEvent::Delta { text } if text == "运行一下。"
    ));
    assert!(
        matches!(&emitted[1], StreamEvent::ToolUse { message } if message.role_str() == "tool_use")
    );
}

/// Every successful result finalizes its own turn's tail: a run continued by
/// queued input gets one finalize boundary per turn, in order (#TASK-1715).
#[tokio::test]
async fn test_process_messages_streaming_finalizes_each_turn_result() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(6);

    tx.send(Ok(Message::Assistant(assistant_text_message("turn one"))))
        .await
        .unwrap();
    tx.send(Ok(Message::Result(result_message_with_error(
        "sdk-session-multi",
        false,
    ))))
    .await
    .unwrap();
    tx.send(Ok(Message::Assistant(assistant_text_message("turn two"))))
        .await
        .unwrap();
    tx.send(Ok(Message::Result(result_message_with_error(
        "sdk-session-multi",
        false,
    ))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider
        .process_messages_streaming("run-multi-result", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    let finalize = StreamEvent::Boundary {
        kind: StreamBoundaryKind::AssistantSegment,
        pending_input_id: None,
    };
    assert_eq!(
        chunks.lock().expect("chunks mutex poisoned").as_slice(),
        &[
            StreamEvent::Delta {
                text: "turn one".to_owned(),
            },
            finalize.clone(),
            // Pre-existing segment-start boundary emitted before a new
            // assistant segment when prior text exists.
            finalize.clone(),
            StreamEvent::Delta {
                text: "turn two".to_owned(),
            },
            finalize,
        ]
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
        origin: None,
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

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-order", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

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
        origin: None,
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

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();

    tx.send(Ok(Message::User(UserMessage {
        content: UserContent::Text("queued user".to_owned()),
        uuid: None,
        parent_tool_use_id: None,
        tool_use_result: None,
        origin: None,
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

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
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
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });

    provider
        .run_pending_inputs
        .lock()
        .await
        .insert("run-3".to_owned(), pending_ack_queue(&["queued-1"]));
    let (response_text, result_data, _signals) = provider
        .process_messages_streaming("run-3", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

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
fn test_build_user_message_input_skips_agent_memory_for_builtin_claude() {
    let options = ProviderRunOptions {
        thread_id: "sess".to_owned(),
        message: "describe this".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), Value::String("claude".to_owned()))]),
    };

    let content = build_user_message_input(&options, true);
    match content {
        UserInput::Text(text) => assert_eq!(text, "describe this"),
        UserInput::Blocks(_) => panic!("expected text input"),
    }
}

#[test]
fn test_build_user_message_input_prepends_memory_for_custom_agents() {
    let options = ProviderRunOptions {
        thread_id: "sess".to_owned(),
        message: "describe this".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), Value::String("reviewer".to_owned()))]),
    };

    let content = build_user_message_input(&options, true);
    match content {
        UserInput::Text(text) => {
            assert!(text.starts_with("<garyx_memory_context>"));
            assert!(text.contains("<agent_memory agent_id=\"reviewer\""));
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
            thread_title: None,
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
async fn test_sdk_unsupported_content_block_error_is_not_swallowed_as_no_result() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    tx.send(Err(claude_agent_sdk::ClaudeSDKError::MessageParse {
        message: "Unknown content block type: future_block".to_owned(),
        data: Some(json!({
            "type": "future_block"
        })),
    }))
    .await
    .unwrap();
    drop(tx);

    let cb: StreamCallback = Box::new(|_| {});
    let err = provider
        .process_messages_streaming("run-document", "thread::document", &mut rx, &cb)
        .await
        .expect_err("unsupported SDK blocks should surface as parse errors");

    assert!(matches!(
        err,
        BridgeError::SessionParseUnsupportedBlock(ref message)
            if message.contains("Unknown content block type: future_block")
    ));
    assert!(!should_retry_with_fresh_session(&err));
}

#[tokio::test]
async fn test_session_parse_error_does_not_retry_with_fresh_session() {
    let mut provider = make_provider();
    provider.ready = true;
    provider
        .session_map
        .lock()
        .await
        .insert("thread::document".to_owned(), "stale-session".to_owned());
    provider
        .enqueue_test_run_attempt(Err(BridgeError::SessionParseUnsupportedBlock(
            "Unknown content block type: document".to_owned(),
        )))
        .await;

    let err = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "thread::document".to_owned(),
                message: "hello".to_owned(),
                workspace_dir: None,
                images: None,
                metadata: HashMap::new(),
            },
            Box::new(|_| {}),
        )
        .await
        .expect_err("parse errors should not fall back to a new session");

    assert!(matches!(
        err,
        BridgeError::SessionParseUnsupportedBlock(ref message)
            if message.contains("Unknown content block type: document")
    ));
    assert_eq!(
        provider.recorded_test_session_attempts().await,
        vec![Some("stale-session".to_owned())]
    );
    assert_eq!(
        provider
            .session_map
            .lock()
            .await
            .get("thread::document")
            .cloned()
            .as_deref(),
        Some("stale-session")
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
            thread_title: None,
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
            thread_title: None,
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
    provider
        .run_pending_inputs
        .lock()
        .await
        .insert("run-1".to_owned(), pending_ack_queue(&[]));
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
    provider
        .run_pending_inputs
        .lock()
        .await
        .insert("run-1".to_owned(), pending_ack_queue(&["queued-1"]));

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
    provider
        .run_pending_inputs
        .lock()
        .await
        .insert("run-stale".to_owned(), pending_ack_queue(&[]));

    assert_eq!(
        provider.clear_session("sess::x").await,
        ClearSessionOutcome::Cleared
    );

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

#[test]
fn test_reasoning_effort_maps_to_claude_effort_levels() {
    for level in ["low", "medium", "high", "xhigh", "max"] {
        assert_eq!(
            claude_effort_for_reasoning_effort(level),
            Some(level.to_owned())
        );
    }
    assert_eq!(
        claude_effort_for_reasoning_effort(" High "),
        Some("high".to_owned())
    );
    assert_eq!(claude_effort_for_reasoning_effort("off"), None);
    assert_eq!(claude_effort_for_reasoning_effort("minimal"), None);
    assert_eq!(claude_effort_for_reasoning_effort("auto"), None);
    assert_eq!(claude_effort_for_reasoning_effort(""), None);
}

#[test]
fn test_resolve_requested_effort_reads_metadata() {
    let config = ClaudeCodeConfig::default();
    let metadata = HashMap::from([(
        "model_reasoning_effort".to_owned(),
        Value::String("xhigh".to_owned()),
    )]);
    assert_eq!(
        resolve_requested_effort(&config, &metadata),
        Some("xhigh".to_owned())
    );
    assert_eq!(resolve_requested_effort(&config, &HashMap::new()), None);
}

#[test]
fn test_resolve_requested_effort_uses_config_default() {
    let config = ClaudeCodeConfig {
        model_reasoning_effort: "max".to_owned(),
        ..ClaudeCodeConfig::default()
    };

    assert_eq!(
        resolve_requested_effort(&config, &HashMap::new()),
        Some("max".to_owned())
    );

    let metadata = HashMap::from([(
        "model_reasoning_effort".to_owned(),
        Value::String("high".to_owned()),
    )]);
    assert_eq!(
        resolve_requested_effort(&config, &metadata),
        Some("high".to_owned())
    );
}

// ---------------------------------------------------------------------------
// Agent SDK protocol signals: terminal classification, rate limits, refusal
// fallback, background-task replace, context compaction
// ---------------------------------------------------------------------------

fn collecting_callback() -> (Arc<std::sync::Mutex<Vec<StreamEvent>>>, StreamCallback) {
    let chunks = Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let chunks_cb = chunks.clone();
    let cb: StreamCallback = Box::new(move |event| {
        chunks_cb.lock().expect("chunks mutex poisoned").push(event);
    });
    (chunks, cb)
}

#[tokio::test]
async fn test_result_terminal_classification_carried_into_processed_result() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "error_during_execution".to_owned(),
        is_error: true,
        session_id: "sdk-session-err".to_owned(),
        terminal_reason: Some("api_error".to_owned()),
        stop_reason: Some("max_tokens".to_owned()),
        api_error_status: Some(529),
        errors: vec!["upstream exploded".to_owned()],
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let (_chunks, cb) = collecting_callback();
    let (_text, result_data, _signals) = provider
        .process_messages_streaming("run-term", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    let result = result_data.expect("result frame should be captured");
    assert!(result.is_error);
    assert_eq!(result.subtype, "error_during_execution");
    assert_eq!(result.terminal_reason.as_deref(), Some("api_error"));
    assert_eq!(result.stop_reason.as_deref(), Some("max_tokens"));
    assert_eq!(result.api_error_status, Some(529));
    assert_eq!(result.errors, vec!["upstream exploded".to_owned()]);

    let formatted = format_claude_run_error(&result, Some(&AssistantMessageError::Overloaded));
    assert_eq!(
        formatted,
        "claude run failed (error_during_execution, terminal_reason=api_error, \
         stop_reason=max_tokens, api_error_status=529, api_error=overloaded): \
         upstream exploded"
    );
}

#[test]
fn test_format_claude_run_error_falls_back_to_generic_label() {
    let result = ProcessedResult {
        session_id: String::new(),
        cost_usd: 0.0,
        input_tokens: 0,
        output_tokens: 0,
        is_error: true,
        subtype: "success".to_owned(),
        terminal_reason: None,
        stop_reason: None,
        errors: Vec::new(),
        api_error_status: None,
        actual_model: None,
        thread_title: None,
        session_messages: Vec::new(),
    };
    assert_eq!(
        format_claude_run_error(&result, None),
        "claude SDK reported error"
    );
}

#[tokio::test]
async fn test_blocking_limit_result_stages_rate_limit_for_take() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::System(SystemMessage {
        subtype: "rate_limit_event".to_owned(),
        data: json!({
            "type": "rate_limit_event",
            "rate_limit_info": {
                "status": "rejected",
                "resetsAt": 1767225600,
                "rateLimitType": "five_hour",
                "utilization": 0.93,
            },
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "error_during_execution".to_owned(),
        is_error: true,
        session_id: "sdk-session-limit".to_owned(),
        terminal_reason: Some("blocking_limit".to_owned()),
        errors: vec!["usage limit reached".to_owned()],
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let (_chunks, cb) = collecting_callback();
    provider
        .process_messages_streaming("run-limit", "thread::limit", &mut rx, &cb)
        .await
        .expect("stream should process");

    let staged = provider
        .take_rate_limit("thread::limit")
        .await
        .expect("rate limit should be staged");
    assert_eq!(staged.provider, "claude_code");
    assert_eq!(staged.reset_at, unix_to_rfc3339(1767225600));
    assert_eq!(staged.window.as_deref(), Some("five_hour"));
    assert_eq!(staged.used_percent, Some(93));
    assert_eq!(staged.reached_type.as_deref(), Some("blocking_limit"));
    assert_eq!(staged.message.as_deref(), Some("usage limit reached"));

    // Consumed exactly once.
    assert!(provider.take_rate_limit("thread::limit").await.is_none());
}

#[tokio::test]
async fn test_rejected_rate_limit_without_result_still_stages() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);

    tx.send(Ok(Message::System(SystemMessage {
        subtype: "rate_limit_event".to_owned(),
        data: json!({
            "type": "rate_limit_event",
            "rate_limit_info": {
                "status": "rejected",
                "resetsAt": 1767225600,
                "rateLimitType": "seven_day",
            },
        }),
    })))
    .await
    .unwrap();
    // Stream dies without a result frame (CLI killed by the limit).
    drop(tx);

    let (_chunks, cb) = collecting_callback();
    provider
        .process_messages_streaming("run-dead", "thread::dead", &mut rx, &cb)
        .await
        .expect("stream should process");

    let staged = provider
        .take_rate_limit("thread::dead")
        .await
        .expect("rate limit should be staged for the dead run");
    assert_eq!(staged.window.as_deref(), Some("seven_day"));
    assert_eq!(staged.reached_type.as_deref(), Some("rate_limit_rejected"));
}

#[tokio::test]
async fn test_rate_limit_not_staged_on_success_or_warning() {
    let provider = make_provider();

    // Success run after a rejected event: no staging.
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    tx.send(Ok(Message::System(SystemMessage {
        subtype: "rate_limit_event".to_owned(),
        data: json!({
            "type": "rate_limit_event",
            "rate_limit_info": { "status": "rejected", "resetsAt": 1767225600 },
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "success".to_owned(),
        is_error: false,
        session_id: "sdk-session-ok".to_owned(),
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);
    let (_chunks, cb) = collecting_callback();
    provider
        .process_messages_streaming("run-ok", "thread::ok", &mut rx, &cb)
        .await
        .expect("stream should process");
    assert!(provider.take_rate_limit("thread::ok").await.is_none());

    // Failed run with only a warning-level event: no staging.
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    tx.send(Ok(Message::System(SystemMessage {
        subtype: "rate_limit_event".to_owned(),
        data: json!({
            "type": "rate_limit_event",
            "rate_limit_info": { "status": "allowed_warning", "utilization": 91 },
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "error_during_execution".to_owned(),
        is_error: true,
        session_id: "sdk-session-warn".to_owned(),
        terminal_reason: Some("api_error".to_owned()),
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);
    let (_chunks, cb) = collecting_callback();
    provider
        .process_messages_streaming("run-warn", "thread::warn", &mut rx, &cb)
        .await
        .expect("stream should process");
    assert!(provider.take_rate_limit("thread::warn").await.is_none());
}

#[test]
fn test_background_tasks_changed_replaces_task_set() {
    let mut active = HashSet::new();
    active.insert("stale-task".to_owned());

    update_claude_background_tasks(
        &SystemMessage {
            subtype: "background_tasks_changed".to_owned(),
            data: json!({
                "tasks": [
                    { "task_id": "live-1", "task_type": "local_agent", "description": "d1" },
                    { "task_id": "live-2", "task_type": "local_agent", "description": "d2" },
                ],
            }),
        },
        &mut active,
    );
    assert_eq!(
        active,
        HashSet::from(["live-1".to_owned(), "live-2".to_owned()])
    );

    // Empty payload clears everything, allowing stdin to close after the
    // input-drain window even when an individual terminal edge was missed.
    // Output consumption remains open until stream EOF.
    update_claude_background_tasks(
        &SystemMessage {
            subtype: "background_tasks_changed".to_owned(),
            data: json!({ "tasks": [] }),
        },
        &mut active,
    );
    assert!(active.is_empty());
}

#[tokio::test]
async fn test_model_refusal_fallback_overrides_actual_model() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text(TextBlock {
            text: "before refusal".to_owned(),
        })],
        model: "claude-original".to_owned(),
        parent_tool_use_id: None,
        error: None,
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::System(SystemMessage {
        subtype: "model_refusal_fallback".to_owned(),
        data: json!({
            "original_model": "claude-original",
            "fallback_model": "claude-fallback",
            "direction": "sticky",
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "success".to_owned(),
        is_error: false,
        session_id: "sdk-session-refusal".to_owned(),
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let (_chunks, cb) = collecting_callback();
    let (_text, result_data, _signals) = provider
        .process_messages_streaming("run-refusal", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(
        result_data
            .expect("result frame should be captured")
            .actual_model
            .as_deref(),
        Some("claude-fallback")
    );
}

#[tokio::test]
async fn test_assistant_api_error_captured_in_signals() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::Assistant(AssistantMessage {
        content: Vec::new(),
        model: "claude-test".to_owned(),
        parent_tool_use_id: None,
        error: Some(AssistantMessageError::RateLimit),
    })))
    .await
    .unwrap();
    drop(tx);

    let (_chunks, cb) = collecting_callback();
    let (_text, _result_data, signals) = provider
        .process_messages_streaming("run-apierr", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    assert_eq!(
        signals.last_assistant_error,
        Some(AssistantMessageError::RateLimit)
    );
}

#[tokio::test]
async fn test_compact_boundary_emits_paired_context_compaction_activity() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::System(SystemMessage {
        subtype: "compact_boundary".to_owned(),
        data: json!({
            "uuid": "compact-uuid-1",
            "compact_metadata": {
                "trigger": "auto",
                "pre_tokens": 50000,
                "post_tokens": 8000,
            },
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "success".to_owned(),
        is_error: false,
        session_id: "sdk-session-compact".to_owned(),
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let (chunks, cb) = collecting_callback();
    let (_text, result_data, _signals) = provider
        .process_messages_streaming("run-compact", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    let tool_use = emitted
        .iter()
        .find_map(|event| match event {
            StreamEvent::ToolUse { message } => Some(message.clone()),
            _ => None,
        })
        .expect("paired ToolUse frame should be emitted");
    let tool_result = emitted
        .iter()
        .find_map(|event| match event {
            StreamEvent::ToolResult { message } => Some(message.clone()),
            _ => None,
        })
        .expect("paired ToolResult frame should be emitted");

    assert_eq!(tool_use.tool_name.as_deref(), Some("contextCompaction"));
    assert_eq!(tool_use.tool_use_id.as_deref(), Some("compact-uuid-1"));
    // `item_type` must be present so channel placeholder policy
    // (plugin_tools::should_hide_tool_call_display) hides compaction like
    // it does for Codex.
    assert_eq!(
        tool_use.metadata.get("item_type").and_then(Value::as_str),
        Some("contextCompaction")
    );
    assert_eq!(
        tool_result
            .metadata
            .get("item_type")
            .and_then(Value::as_str),
        Some("contextCompaction")
    );
    assert_eq!(tool_result.tool_use_id.as_deref(), Some("compact-uuid-1"));
    assert_eq!(tool_result.is_error, Some(false));
    let text = tool_result.text.clone().unwrap_or_default();
    assert!(
        text.contains("50000 -> 8000"),
        "result text should carry token accounting, got: {text}"
    );

    // Both halves also land in the persisted session messages.
    let session_messages = result_data
        .expect("result frame should be captured")
        .session_messages;
    let compaction_rows = session_messages
        .iter()
        .filter(|message| message.tool_name.as_deref() == Some("contextCompaction"))
        .count();
    assert_eq!(compaction_rows, 2);
}

#[tokio::test]
async fn test_failed_compact_status_emits_error_activity() {
    let provider = make_provider();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    tx.send(Ok(Message::System(SystemMessage {
        subtype: "status".to_owned(),
        data: json!({
            "uuid": "compact-uuid-2",
            "status": null,
            "compact_result": "failed",
            "compact_error": "compaction blew up",
        }),
    })))
    .await
    .unwrap();
    tx.send(Ok(Message::Result(Box::new(ResultMessage {
        subtype: "success".to_owned(),
        is_error: false,
        session_id: "sdk-session-compact-fail".to_owned(),
        ..Default::default()
    }))))
    .await
    .unwrap();
    drop(tx);

    let (chunks, cb) = collecting_callback();
    provider
        .process_messages_streaming("run-compact-fail", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");

    let emitted = chunks.lock().expect("chunks mutex poisoned").clone();
    let tool_result = emitted
        .iter()
        .find_map(|event| match event {
            StreamEvent::ToolResult { message } => Some(message.clone()),
            _ => None,
        })
        .expect("failed compact should emit a paired ToolResult frame");
    assert_eq!(tool_result.tool_name.as_deref(), Some("contextCompaction"));
    assert_eq!(tool_result.is_error, Some(true));
    assert_eq!(tool_result.text.as_deref(), Some("compaction blew up"));

    // A plain "compacting" status frame must NOT emit activity: pairing is
    // completed-only.
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    tx.send(Ok(Message::System(SystemMessage {
        subtype: "status".to_owned(),
        data: json!({ "status": "compacting" }),
    })))
    .await
    .unwrap();
    drop(tx);
    let (chunks, cb) = collecting_callback();
    provider
        .process_messages_streaming("run-compacting", "thread::test", &mut rx, &cb)
        .await
        .expect("stream should process");
    assert!(
        chunks.lock().expect("chunks mutex poisoned").is_empty(),
        "in-progress compacting status must not emit frames"
    );
}

#[test]
fn test_claude_utilization_percent_normalizes_ratio_and_percent() {
    // Official CLI reports a 0..1 ratio.
    assert_eq!(claude_utilization_percent(&json!(0.93)), Some(93));
    assert_eq!(claude_utilization_percent(&json!(1.0)), Some(100));
    assert_eq!(claude_utilization_percent(&json!(0)), Some(0));
    // Values above 1 are treated as already-percentages for tolerance.
    assert_eq!(claude_utilization_percent(&json!(93)), Some(93));
    // Numeric strings are accepted.
    assert_eq!(claude_utilization_percent(&json!("0.5")), Some(50));
    // Garbage is rejected.
    assert_eq!(claude_utilization_percent(&json!(-0.1)), None);
    assert_eq!(claude_utilization_percent(&json!("nope")), None);
    assert_eq!(claude_utilization_percent(&json!(null)), None);
}

#[tokio::test]
async fn test_execute_sdk_run_entry_clears_stale_rate_limit_stash() {
    let provider = make_provider();

    // A previous attempt staged a rate limit for this thread.
    provider
        .pending_rate_limits
        .stage(
            "thread::stale-stash".to_owned(),
            garyx_models::provider::ProviderRateLimit {
                provider: "claude_code".to_owned(),
                ..Default::default()
            },
        )
        .await;

    // The next attempt dies at connect time (injected before the message
    // loop, mirroring a connect/send failure that never reaches it).
    provider
        .test_run_attempts
        .lock()
        .await
        .push_back(Err(BridgeError::RunFailed(
            "failed to connect to claude: boom".to_owned(),
        )));

    let options = ProviderRunOptions {
        thread_id: "thread::stale-stash".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };
    let cb: StreamCallback = Box::new(|_| {});
    let attempt = provider
        .execute_sdk_run(&options, None, "run-stale", &cb)
        .await;
    assert!(attempt.is_err(), "injected connect failure should surface");

    // The stale stash from the earlier attempt must NOT be attributed to
    // this failed attempt's terminal record.
    assert!(
        provider
            .take_rate_limit("thread::stale-stash")
            .await
            .is_none(),
        "connect-failure attempt must clear the stale rate-limit stash"
    );
}
