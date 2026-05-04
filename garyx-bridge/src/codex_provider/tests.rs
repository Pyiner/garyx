use super::*;
use serde_json::json;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

// -- Helper function tests --

#[test]
fn test_matches_turn_exact() {
    let params = json!({"threadId": "t1", "turnId": "u1"});
    assert!(matches_turn(&params, "t1", "u1"));
}

#[test]
fn test_matches_turn_wrong_thread() {
    let params = json!({"threadId": "t2", "turnId": "u1"});
    assert!(!matches_turn(&params, "t1", "u1"));
}

#[test]
fn test_matches_turn_wrong_turn() {
    let params = json!({"threadId": "t1", "turnId": "u2"});
    assert!(!matches_turn(&params, "t1", "u1"));
}

#[test]
fn test_matches_turn_no_ids_matches() {
    let params = json!({"data": 42});
    assert!(matches_turn(&params, "t1", "u1"));
}

#[test]
fn test_matches_turn_via_turn_object() {
    let params = json!({"turn": {"id": "u1"}});
    assert!(matches_turn(&params, "t1", "u1"));
    assert!(!matches_turn(&params, "t1", "u2"));
}

#[test]
fn test_extract_usage_full() {
    let turn = json!({
        "usage": {
            "inputTokens": 100,
            "outputTokens": 50,
            "totalCostUsd": 0.005,
        }
    });
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 100);
    assert_eq!(output, 50);
    assert!((cost - 0.005).abs() < f64::EPSILON);
}

#[test]
fn test_extract_usage_snake_case() {
    let turn = json!({
        "usage": {
            "input_tokens": 200,
            "output_tokens": 80,
            "cost": 0.01,
        }
    });
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 200);
    assert_eq!(output, 80);
    assert!((cost - 0.01).abs() < f64::EPSILON);
}

#[test]
fn test_extract_usage_missing() {
    let turn = json!({"status": "completed"});
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 0);
    assert_eq!(output, 0);
    assert!((cost - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_extract_usage_string_values() {
    let turn = json!({
        "usage": {
            "inputTokens": "150",
            "outputTokens": "75",
            "totalCostUsd": "0.003",
        }
    });
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 150);
    assert_eq!(output, 75);
    assert!((cost - 0.003).abs() < f64::EPSILON);
}

#[test]
fn test_resolve_runtime_codex_env_merges_desktop_auth_env() {
    let config = CodexAppServerConfig {
        env: HashMap::from([
            ("OPENAI_API_KEY".to_owned(), "from-config".to_owned()),
            (
                "OPENAI_BASE_URL".to_owned(),
                "https://example.test".to_owned(),
            ),
        ]),
        ..Default::default()
    };
    let metadata = HashMap::from([(
        "desktop_codex_env".to_owned(),
        json!({
            "OPENAI_API_KEY": "from-desktop",
            "OPENAI_ORG_ID": "org_123",
        }),
    )]);

    let env = resolve_runtime_codex_env(&config, &metadata);
    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("from-desktop")
    );
    assert_eq!(
        env.get("OPENAI_BASE_URL").map(String::as_str),
        Some("https://example.test")
    );
    assert_eq!(
        env.get("OPENAI_ORG_ID").map(String::as_str),
        Some("org_123")
    );
}

#[test]
fn test_resolve_runtime_codex_env_keeps_blank_desktop_api_key_override() {
    let config = CodexAppServerConfig {
        env: HashMap::from([("OPENAI_API_KEY".to_owned(), "from-config".to_owned())]),
        ..Default::default()
    };
    let metadata = HashMap::from([(
        "desktop_codex_env".to_owned(),
        json!({
            "OPENAI_API_KEY": "",
        }),
    )]);

    let env = resolve_runtime_codex_env(&config, &metadata);
    assert_eq!(env.get("OPENAI_API_KEY").map(String::as_str), Some(""));
}

#[test]
fn test_resolve_runtime_codex_env_exports_task_cli_env() {
    let config = CodexAppServerConfig::default();
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("codex")),
        (
            "runtime_context".to_owned(),
            json!({
                "thread_id": "thread::task",
                "task": {
                    "task_id": "#TASK-4",
                    "status": "todo",
                    "scope": "telegram/codex_bot"
                }
            }),
        ),
    ]);

    let env = resolve_runtime_codex_env(&config, &metadata);

    assert_eq!(
        env.get("GARYX_THREAD_ID").map(String::as_str),
        Some("thread::task")
    );
    assert_eq!(
        env.get("GARYX_ACTOR").map(String::as_str),
        Some("agent:codex")
    );
    assert_eq!(
        env.get("GARYX_TASK_ID").map(String::as_str),
        Some("#TASK-4")
    );
}

#[test]
fn test_codex_client_reuse_keeps_active_client_when_env_changes() {
    let existing = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::old".to_owned())]);
    let desired = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::new".to_owned())]);

    assert_eq!(
        decide_codex_client_reuse(&existing, &desired, 1),
        CodexClientReuseDecision::Reuse
    );
}

#[test]
fn test_codex_client_reuse_replaces_idle_client_when_env_changes() {
    let existing = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::old".to_owned())]);
    let desired = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::new".to_owned())]);

    assert_eq!(
        decide_codex_client_reuse(&existing, &desired, 0),
        CodexClientReuseDecision::ReplaceIdle
    );
}

#[test]
fn test_codex_client_idle_ttl_is_three_minutes() {
    assert_eq!(CODEX_CLIENT_IDLE_TTL, Duration::from_secs(180));
}

#[test]
fn test_build_input_items_text_only() {
    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "hello world".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };
    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0], InputItem::Text { text } if text == "hello world"));
}

#[test]
fn test_build_input_items_prepends_memory_on_first_turn() {
    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "hello world".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), json!("codex"))]),
    };
    let items = build_input_items(&options, true);
    assert_eq!(items.len(), 1);
    assert!(
        matches!(&items[0], InputItem::Text { text } if text.starts_with("<garyx_memory_context>") && text.contains("<agent_memory agent_id=\"codex\"") && text.ends_with("hello world"))
    );
}

#[test]
fn test_build_input_items_does_not_append_task_status_suffix() {
    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "继续".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "runtime_context".to_owned(),
            json!({
                "task": {
                    "task_id": "#TASK-4",
                    "status": "in_progress",
                    "assignee": { "kind": "agent", "agent_id": "codex" }
                }
            }),
        )]),
    };
    let items = build_input_items(&options, false);

    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0], InputItem::Text { text } if text == "继续"));

    let items = build_input_items(&options, true);
    assert_eq!(items.len(), 1);
    match &items[0] {
        InputItem::Text { text } => {
            assert!(text.starts_with("<garyx_thread_metadata>"));
            assert!(text.contains("task_id: #TASK-4"));
            assert!(!text.contains("status=in_progress"));
            assert!(!text.contains("assignee=agent:codex"));
            assert!(text.ends_with("继续"));
        }
        _ => panic!("expected text input"),
    }
}

#[test]
fn test_build_input_items_with_images() {
    let img = ImagePayload {
        name: "sample.png".to_owned(),
        media_type: "image/png".to_owned(),
        data: "abc123==".to_owned(),
    };

    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "analyze this".to_owned(),
        workspace_dir: None,
        images: Some(vec![img]),
        metadata: HashMap::new(),
    };
    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 2);
    assert!(
        matches!(&items[1], InputItem::Image { url } if url == "data:image/png;base64,abc123==")
    );
}

#[test]
fn test_build_input_items_empty_image_data_skipped() {
    let img = ImagePayload {
        name: "empty.png".to_owned(),
        media_type: "image/png".to_owned(),
        data: String::new(),
    };

    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "msg".to_owned(),
        workspace_dir: None,
        images: Some(vec![img]),
        metadata: HashMap::new(),
    };
    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 1); // image skipped
}

#[test]
fn test_build_tool_session_message_command() {
    let item = json!({
        "type": "commandExecution",
        "id": "cmd_1",
        "status": "completed",
        "command": "ls -la"
    });
    let msg = build_tool_session_message(&item, true).unwrap();
    assert_eq!(msg.role_str(), "tool_result");
    assert_eq!(msg.tool_name.as_deref(), Some("commandExecution"));
    assert_eq!(msg.tool_use_id.as_deref(), Some("cmd_1"));
    assert_eq!(msg.is_error, Some(false));
}

#[test]
fn test_build_tool_session_message_failed() {
    let item = json!({
        "type": "commandExecution",
        "id": "cmd_2",
        "status": "failed",
    });
    let msg = build_tool_session_message(&item, true).unwrap();
    assert_eq!(msg.is_error, Some(true));
}

#[test]
fn test_build_tool_session_message_mcp() {
    let item = json!({
        "type": "mcpToolCall",
        "id": "mcp_1",
        "server": "filesystem",
        "tool": "read_file",
    });
    let msg = build_tool_session_message(&item, false).unwrap();
    assert_eq!(msg.role_str(), "tool_use");
    assert_eq!(msg.tool_name.as_deref(), Some("mcp:filesystem:read_file"));
    assert_eq!(msg.is_error, None);
}

#[test]
fn test_build_tool_session_message_codex_schema_tool_types() {
    let cases = [
        (
            json!({
                "type": "hookPrompt",
                "id": "hook_1",
                "fragments": [],
            }),
            "hookPrompt",
        ),
        (
            json!({
                "type": "plan",
                "id": "plan_1",
                "text": "1. inspect\n2. patch",
            }),
            "plan",
        ),
        (
            json!({
                "type": "reasoning",
                "id": "reason_1",
                "summary": ["checking state"],
                "content": [],
            }),
            "reasoning",
        ),
        (
            json!({
                "type": "dynamicToolCall",
                "id": "dyn_1",
                "namespace": "image_gen",
                "tool": "generate",
                "status": "inProgress",
            }),
            "image_gen:generate",
        ),
        (
            json!({
                "type": "collabAgentToolCall",
                "id": "agent_1",
                "tool": "spawnAgent",
                "status": "inProgress",
            }),
            "spawnAgent",
        ),
        (
            json!({
                "type": "webSearch",
                "id": "web_1",
                "query": "codex app server schema",
            }),
            "webSearch",
        ),
        (
            json!({
                "type": "imageView",
                "id": "view_1",
                "path": "/tmp/probe.png",
            }),
            "imageView",
        ),
        (
            json!({
                "type": "imageGeneration",
                "id": "img_1",
                "status": "in_progress",
                "revisedPrompt": null,
                "result": "",
            }),
            "imageGeneration",
        ),
        (
            json!({
                "type": "enteredReviewMode",
                "id": "review_1",
                "review": "code review",
            }),
            "enteredReviewMode",
        ),
        (
            json!({
                "type": "exitedReviewMode",
                "id": "review_2",
                "review": "code review",
            }),
            "exitedReviewMode",
        ),
        (
            json!({
                "type": "contextCompaction",
                "id": "compact_1",
            }),
            "contextCompaction",
        ),
    ];

    for (item, expected_name) in cases {
        let msg = build_tool_session_message(&item, false).unwrap();
        assert_eq!(msg.role_str(), "tool_use");
        assert_eq!(msg.tool_name.as_deref(), Some(expected_name));
        assert_eq!(
            msg.metadata.get("source").and_then(Value::as_str),
            Some("codex_app_server")
        );
        assert_eq!(
            msg.metadata.get("item_type").and_then(Value::as_str),
            item.get("type").and_then(Value::as_str)
        );
    }
}

#[test]
fn test_build_tool_session_message_error_statuses() {
    for status in ["failed", "declined", "error", "canceled", "cancelled"] {
        let item = json!({
            "type": "dynamicToolCall",
            "id": format!("dyn_{status}"),
            "tool": "run",
            "status": status,
        });
        let msg = build_tool_session_message(&item, true).unwrap();
        assert_eq!(msg.role_str(), "tool_result");
        assert_eq!(msg.is_error, Some(true), "status {status} should be error");
    }

    let item = json!({
        "type": "dynamicToolCall",
        "id": "dyn_success_false",
        "tool": "run",
        "status": "completed",
        "success": false,
    });
    let msg = build_tool_session_message(&item, true).unwrap();
    assert_eq!(msg.is_error, Some(true));
}

#[test]
fn test_build_tool_session_message_irrelevant_type() {
    let item = json!({"type": "text", "text": "hello"});
    assert!(build_tool_session_message(&item, false).is_none());
}

#[test]
fn test_is_agent_message_item_matches_legacy_and_v2_shapes() {
    assert!(is_agent_message_item(&json!({
        "type": "agentMessage",
        "id": "msg-1",
        "text": "commentary"
    })));
    assert!(is_agent_message_item(&json!({
        "type": "AgentMessage",
        "id": "msg-2",
        "content": []
    })));
    assert!(!is_agent_message_item(&json!({
        "type": "commandExecution",
        "id": "cmd-1"
    })));
}

#[test]
fn test_is_user_message_item_matches_v2_shape() {
    assert!(is_user_message_item(&json!({
        "type": "userMessage",
        "id": "user-1",
        "content": [{"type": "text", "text": "hello"}]
    })));
    assert!(is_user_message_item(&json!({
        "type": "UserMessage",
        "id": "user-2",
        "content": []
    })));
    assert!(!is_user_message_item(&json!({
        "type": "agentMessage",
        "id": "msg-1"
    })));
}

#[test]
fn test_is_tool_activity_item_matches_supported_types() {
    assert!(is_tool_activity_item(&json!({
        "type": "hookPrompt",
        "id": "hook-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "plan",
        "id": "plan-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "reasoning",
        "id": "reasoning-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "commandExecution",
        "id": "cmd-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "fileChange",
        "id": "file-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "mcpToolCall",
        "id": "mcp-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "dynamicToolCall",
        "id": "dyn-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "collabAgentToolCall",
        "id": "agent-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "webSearch",
        "id": "web-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "imageView",
        "id": "view-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "imageGeneration",
        "id": "img-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "enteredReviewMode",
        "id": "review-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "exitedReviewMode",
        "id": "review-2"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "contextCompaction",
        "id": "compact-1"
    })));
    assert!(!is_tool_activity_item(&json!({
        "type": "agentMessage",
        "id": "msg-1"
    })));
    assert!(!is_tool_activity_item(&json!({
        "type": "userMessage",
        "id": "user-1"
    })));
    assert!(!is_tool_activity_item(&json!({
        "type": "text",
        "text": "hello"
    })));
}

#[test]
fn test_agent_message_item_switch_with_tool_activity_inserts_separator() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let mut current_item_id = None;
    let mut current_item_has_text = false;
    let mut response_parts = Vec::new();

    maybe_emit_agent_message_separator(
        Some("commentary-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );
    assert_eq!(current_item_id.as_deref(), Some("commentary-1"));
    current_item_has_text = true;

    maybe_emit_agent_message_separator(
        Some("final-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );

    assert_eq!(current_item_id.as_deref(), Some("final-1"));
    assert!(!current_item_has_text);
    assert_eq!(response_parts, vec!["\n\n".to_owned()]);
    assert_eq!(
        emitted.lock().expect("events mutex poisoned").as_slice(),
        &[StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        }]
    );
}

#[test]
fn test_agent_message_item_switch_without_tool_activity_inserts_separator() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let mut current_item_id = Some("commentary-1".to_owned());
    let mut current_item_has_text = true;
    let mut response_parts = Vec::new();

    maybe_emit_agent_message_separator(
        Some("final-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );

    assert_eq!(current_item_id.as_deref(), Some("final-1"));
    assert!(!current_item_has_text);
    assert_eq!(response_parts, vec!["\n\n".to_owned()]);
    assert_eq!(
        emitted.lock().expect("events mutex poisoned").as_slice(),
        &[StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        }]
    );
}

#[test]
fn test_agent_message_item_switch_without_prior_text_does_not_insert_separator() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let mut current_item_id = Some("commentary-1".to_owned());
    let mut current_item_has_text = false;
    let mut response_parts = Vec::new();

    maybe_emit_agent_message_separator(
        Some("final-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );

    assert_eq!(current_item_id.as_deref(), Some("final-1"));
    assert!(!current_item_has_text);
    assert!(response_parts.is_empty());
    assert!(emitted.lock().expect("events mutex poisoned").is_empty());
}

#[test]
fn test_append_codex_assistant_session_message_groups_by_item_id() {
    let mut session_messages = Vec::new();

    append_codex_assistant_session_message(&mut session_messages, Some("item-1"), "在。");
    append_codex_assistant_session_message(&mut session_messages, Some("item-1"), "先执行 ls。");
    append_codex_assistant_session_message(&mut session_messages, Some("item-2"), "结果如下。");

    assert_eq!(session_messages.len(), 2);
    assert_eq!(session_messages[0].role_str(), "assistant");
    assert_eq!(session_messages[0].text.as_deref(), Some("在。先执行 ls。"));
    assert_eq!(
        session_messages[0]
            .metadata
            .get("item_id")
            .and_then(Value::as_str),
        Some("item-1")
    );
    assert_eq!(session_messages[1].text.as_deref(), Some("结果如下。"));
    assert_eq!(
        session_messages[1]
            .metadata
            .get("item_id")
            .and_then(Value::as_str),
        Some("item-2")
    );
}

#[test]
fn test_emit_tool_stream_event_maps_roles() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let tool_use = ProviderMessage::tool_use(
        json!({"type": "commandExecution"}),
        Some("cmd-1".to_owned()),
        Some("commandExecution".to_owned()),
    );
    let tool_result = ProviderMessage::tool_result(
        json!({"type": "commandExecution"}),
        Some("cmd-1".to_owned()),
        Some("commandExecution".to_owned()),
        Some(false),
    );

    emit_tool_stream_event(&tool_use, callback.as_ref());
    emit_tool_stream_event(&tool_result, callback.as_ref());

    let events = emitted.lock().expect("events mutex poisoned").clone();
    assert!(
        matches!(&events[0], StreamEvent::ToolUse { message } if message.role_str() == "tool_use")
    );
    assert!(
        matches!(&events[1], StreamEvent::ToolResult { message } if message.role_str() == "tool_result")
    );
}

#[test]
fn test_build_thread_start_params_full() {
    let config = CodexAppServerConfig {
        workspace_dir: Some("/tmp/work".to_owned()),
        model: "o3-mini".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    assert_eq!(params.cwd.as_deref(), Some("/tmp/work"));
    assert_eq!(params.model.as_deref(), Some("o3-mini"));
    assert_eq!(params.model_reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(params.approval_policy.as_deref(), Some("never"));
    assert_eq!(params.sandbox.as_deref(), Some("danger-full-access"));
    let config = params.config.expect("thread config should exist");
    assert!(
        config
            .get("developer_instructions")
            .and_then(Value::as_str)
            .is_some()
    );
}

#[test]
fn test_build_thread_start_params_fallback_model() {
    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: "gpt-4o".to_owned(),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    assert_eq!(params.model.as_deref(), Some("gpt-4o"));
    assert!(params.model_reasoning_effort.is_none());
}

#[test]
fn test_build_thread_start_params_workspace_override_wins() {
    let config = CodexAppServerConfig {
        workspace_dir: Some("/tmp/from-config".to_owned()),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        Some("/tmp/from-request"),
        "thread::test",
        "run-1",
        &HashMap::new(),
    );
    assert_eq!(params.cwd.as_deref(), Some("/tmp/from-request"));
}

#[test]
fn test_build_thread_start_params_no_model() {
    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: String::new(),
        approval_policy: String::new(),
        sandbox_mode: String::new(),
        workspace_dir: None,
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    assert!(params.model.is_none());
    assert!(params.model_reasoning_effort.is_none());
    assert!(params.cwd.is_none());
    assert!(params.approval_policy.is_none());
    assert!(params.sandbox.is_none());
}

#[test]
fn test_build_thread_start_params_prefers_metadata_model_override() {
    let config = CodexAppServerConfig {
        model: "gpt-5".to_owned(),
        default_model: "gpt-5-codex".to_owned(),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let metadata = HashMap::from([("model".to_owned(), json!("o3"))]);
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &metadata);
    assert_eq!(params.model.as_deref(), Some("o3"));
}

#[test]
fn test_resolve_codex_actual_model_prefers_explicit_sources() {
    let config = CodexAppServerConfig {
        model: "gpt-5".to_owned(),
        default_model: "gpt-4.1".to_owned(),
        ..Default::default()
    };
    let metadata = HashMap::from([("model".to_owned(), json!("o3"))]);
    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &metadata, None).as_deref(),
        Some("o3")
    );

    let metadata = HashMap::new();
    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &metadata, None).as_deref(),
        Some("gpt-5")
    );

    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: "gpt-4.1".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &metadata, None).as_deref(),
        Some("gpt-4.1")
    );
}

#[test]
fn test_resolve_codex_actual_model_reads_cli_default_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "model = \"gpt-5.4\"\nmodel_reasoning_effort = \"xhigh\"\n",
    )
    .expect("write config");

    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: String::new(),
        ..Default::default()
    };

    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &HashMap::new(), Some(&config_path))
            .as_deref(),
        Some("gpt-5.4")
    );
}

#[test]
fn test_build_thread_start_params_injects_remote_mcp_servers() {
    let mut metadata = HashMap::new();
    metadata.insert(
        "remote_mcp_servers".to_owned(),
        json!({
            "proof": {
                "command": "python3",
                "args": ["proof_server.py"],
                "env": {"PROOF_TOKEN": "abc"},
                "working_dir": "/tmp/proof"
            },
            "garyx": {
                "type": "http",
                "url": "http://127.0.0.1:31337/mcp",
                "headers": {"X-Run-Id": "run-1"}
            }
        }),
    );

    let params = build_thread_start_params(
        &CodexAppServerConfig {
            mcp_base_url: String::new(),
            ..Default::default()
        },
        None,
        "thread::test",
        "run-1",
        &metadata,
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["proof"]["command"].as_str(),
        Some("python3")
    );
    assert_eq!(
        config["mcp_servers"]["proof"]["cwd"].as_str(),
        Some("/tmp/proof")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["url"].as_str(),
        Some("http://127.0.0.1:31337/mcp")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Run-Id"].as_str(),
        Some("run-1")
    );
}

#[test]
fn test_build_thread_start_params_injects_gary_developer_instructions() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    let config = params.config.expect("thread config should exist");
    let developer_instructions = config
        .get("developer_instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(developer_instructions.contains("Garyx runtime guidance:"));
    assert!(developer_instructions.contains("Self-evolution:"));
    assert!(developer_instructions.contains("~/.garyx/skills/<skill-id>/SKILL.md"));
    assert!(developer_instructions.contains("garyx task create"));
    assert!(developer_instructions.contains("garyx automation create"));
    assert!(!developer_instructions.contains("Global Memory"));
    assert!(!developer_instructions.contains("<garyx_memory_context>"));
    assert!(!developer_instructions.contains("Current runtime context:"));
    assert!(!developer_instructions.contains("thread_id: thread::test"));
}

#[test]
fn test_build_thread_start_params_merges_runtime_system_prompt() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        None,
        "thread::test",
        "run-1",
        &HashMap::from([(
            "system_prompt".to_owned(),
            Value::String("Use concise bullets.".to_owned()),
        )]),
    );
    let config = params.config.expect("thread config should exist");
    let developer_instructions = config
        .get("developer_instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(developer_instructions.contains("Garyx runtime guidance:"));
    assert!(developer_instructions.contains("Use concise bullets."));
    assert!(!developer_instructions.contains("Global Memory"));
    assert!(!developer_instructions.contains("Current runtime context:"));
}

#[test]
fn test_build_thread_start_params_keeps_runtime_context_out_of_developer_instructions() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        Some("/tmp/ws"),
        "thread::ctx",
        "run-1",
        &HashMap::from([(
            "runtime_context".to_owned(),
            json!({
                "channel": "macapp",
                "account_id": "main",
                "bot_id": "macapp:main",
                "task": {
                    "task_id": "#TASK-9",
                    "status": "todo"
                }
            }),
        )]),
    );
    let config = params.config.expect("thread config should exist");
    let developer_instructions = config
        .get("developer_instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(developer_instructions.contains("System capabilities:"));
    assert!(!developer_instructions.contains("channel: macapp"));
    assert!(!developer_instructions.contains("task_id: #TASK-9"));
    assert!(!developer_instructions.contains("thread_id: thread::ctx"));
    assert!(!developer_instructions.contains("workspace_dir: /tmp/ws"));
}

#[test]
fn test_build_thread_start_params_injects_default_garyx_mcp_server() {
    let params = build_thread_start_params(
        &CodexAppServerConfig::default(),
        None,
        "thread::stop-loop",
        "run-stop",
        &HashMap::new(),
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["garyx"]["url"].as_str(),
        Some("http://127.0.0.1:31337/mcp/thread%3A%3Astop-loop/run-stop")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Run-Id"].as_str(),
        Some("run-stop")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Thread-Id"].as_str(),
        Some("thread::stop-loop")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Session-Key"].as_str(),
        Some("thread::stop-loop")
    );
}

#[test]
fn test_build_thread_start_params_merges_garyx_mcp_headers_from_metadata() {
    let params = build_thread_start_params(
        &CodexAppServerConfig::default(),
        None,
        "thread::verify",
        "run-verify",
        &HashMap::from([(
            "garyx_mcp_headers".to_owned(),
            json!({
                "X-Gary-AutoResearch-Role": "verifier"
            }),
        )]),
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Gary-AutoResearch-Role"].as_str(),
        Some("verifier")
    );
}

#[test]
fn test_build_thread_start_params_builtin_garyx_overrides_runtime_entry() {
    let mut metadata = HashMap::new();
    metadata.insert(
        "remote_mcp_servers".to_owned(),
        json!({
            "proof": {
                "command": "python3",
                "args": ["proof_server.py"]
            },
            "garyx": {
                "type": "http",
                "url": "http://127.0.0.1:31337",
                "headers": {"X-Run-Id": "stale-run"}
            }
        }),
    );

    let params = build_thread_start_params(
        &CodexAppServerConfig::default(),
        None,
        "thread::test",
        "run-1",
        &metadata,
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["proof"]["command"].as_str(),
        Some("python3")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["url"].as_str(),
        Some("http://127.0.0.1:31337/mcp/thread%3A%3Atest/run-1")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Run-Id"].as_str(),
        Some("run-1")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Thread-Id"].as_str(),
        Some("thread::test")
    );
}

#[test]
fn test_build_input_items_uses_native_skill_invocation() {
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

    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 1);
    match &items[0] {
        InputItem::Text { text } => assert_eq!(text, "/proof-skill\n\nUse the skill."),
        InputItem::Image { .. } => panic!("expected text item"),
    }
}

#[test]
fn test_provider_type() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    assert_eq!(provider.provider_type(), ProviderType::CodexAppServer);
}

#[test]
fn test_is_ready_before_init() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    assert!(!provider.is_ready());
}

#[tokio::test]
async fn test_run_returns_not_ready() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
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
async fn test_abort_no_active_run() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    assert!(!provider.abort("nonexistent").await);
}

#[tokio::test]
async fn test_abort_cleans_session_tracking_when_client_missing() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    provider.active_runs.lock().await.insert(
        "run_1".to_owned(),
        ActiveCodexRun {
            garyx_thread_id: "sess::1".to_owned(),
            codex_thread_id: "thread_1".to_owned(),
            turn_id: "turn_1".to_owned(),
        },
    );
    provider.active_session_turns.lock().await.insert(
        "sess::1".to_owned(),
        (
            "thread_1".to_owned(),
            "turn_1".to_owned(),
            "run_1".to_owned(),
        ),
    );
    provider
        .active_session_callbacks
        .lock()
        .await
        .insert("sess::1".to_owned(), ("run_1".to_owned(), Arc::new(|_| {})));

    assert!(!provider.abort("run_1").await);
    assert!(provider.active_runs.lock().await.get("run_1").is_none());
    assert!(
        provider
            .active_session_turns
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
    assert!(
        provider
            .active_session_callbacks
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
}

#[tokio::test]
async fn test_clear_session() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    provider
        .session_map
        .lock()
        .await
        .insert("sess::1".to_owned(), "thread_x".to_owned());
    provider.active_session_turns.lock().await.insert(
        "sess::1".to_owned(),
        (
            "thread_x".to_owned(),
            "turn_1".to_owned(),
            "run_1".to_owned(),
        ),
    );
    provider
        .active_session_callbacks
        .lock()
        .await
        .insert("sess::1".to_owned(), ("run_1".to_owned(), Arc::new(|_| {})));

    assert!(provider.clear_session("sess::1").await);
    assert!(provider.session_map.lock().await.get("sess::1").is_none());
    assert!(
        provider
            .active_session_turns
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
    assert!(
        provider
            .active_session_callbacks
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
}

#[tokio::test]
async fn test_streaming_input_ack_waits_for_codex_user_message_item() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    let events = Arc::new(StdMutex::new(Vec::<StreamEvent>::new()));
    let captured_events = events.clone();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        captured_events.lock().unwrap().push(event);
    });

    provider
        .active_session_callbacks
        .lock()
        .await
        .insert("thread::garyx".to_owned(), ("run_1".to_owned(), callback));
    provider.active_session_pending_acks.lock().await.insert(
        "thread::garyx".to_owned(),
        (
            "run_1".to_owned(),
            VecDeque::from([PendingCodexAckMarker::RootUserMessage]),
        ),
    );

    assert!(
        provider
            .enqueue_streaming_input_ack(
                "thread::garyx",
                "run_1",
                Some("queued_input:1".to_owned())
            )
            .await
    );

    assert!(
        events.lock().unwrap().is_empty(),
        "turn/steer acceptance should only enqueue; ACK is emitted by a later userMessage item"
    );
    assert!(
        !provider
            .acknowledge_next_codex_user_message("codex-thread-1", "run_1")
            .await,
        "callbacks are keyed by Garyx thread id, not Codex thread id"
    );
    assert!(
        !provider
            .acknowledge_next_codex_user_message("thread::garyx", "run_2")
            .await,
        "stale callbacks from another run must not receive acks"
    );
    assert!(
        !provider
            .acknowledge_next_codex_user_message("thread::garyx", "run_1")
            .await,
        "the first Codex userMessage item is the root prompt and must not ACK a queued follow-up"
    );
    assert!(events.lock().unwrap().is_empty());
    assert!(
        provider
            .acknowledge_next_codex_user_message("thread::garyx", "run_1")
            .await
    );

    let events = events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: Some("queued_input:1".to_owned()),
        }
    );
}

#[tokio::test]
async fn test_get_or_create_session_existing() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    provider
        .session_map
        .lock()
        .await
        .insert("sess::a".to_owned(), "thread_abc".to_owned());

    let result = provider.get_or_create_session("sess::a").await.unwrap();
    assert_eq!(result, "thread_abc");
}

#[tokio::test]
async fn test_get_or_create_session_new() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    let result = provider.get_or_create_session("sess::new").await.unwrap();
    // Returns empty string as placeholder for new sessions
    assert!(result.is_empty());
}

#[test]
fn test_resolve_existing_thread_id_prefers_session_map() {
    let session_map = HashMap::from([("thread::one".to_owned(), "thread-from-memory".to_owned())]);

    let resolved =
        resolve_existing_thread_id(&session_map, "thread::one", Some("thread-from-persistence"));

    assert_eq!(resolved.as_deref(), Some("thread-from-memory"));
}

#[tokio::test]
async fn test_resume_or_start_thread_falls_back_to_start_after_resume_error() {
    let resume_calls = Arc::new(AtomicUsize::new(0));
    let start_calls = Arc::new(AtomicUsize::new(0));
    let thread_params = ThreadStartParams {
        cwd: Some("/tmp/workspace".to_owned()),
        config: None,
        model: Some("gpt-5".to_owned()),
        model_reasoning_effort: Some("xhigh".to_owned()),
        approval_policy: Some("never".to_owned()),
        sandbox: Some("danger-full-access".to_owned()),
    };

    let thread_id = resume_or_start_thread(
        Some("stale-thread".to_owned()),
        thread_params.clone(),
        {
            let resume_calls = resume_calls.clone();
            move |params| {
                let resume_calls = resume_calls.clone();
                async move {
                    resume_calls.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(params.thread_id, "stale-thread");
                    assert_eq!(params.cwd.as_deref(), Some("/tmp/workspace"));
                    Err(CodexError::RpcError {
                        code: -32600,
                        message: "no rollout found for thread id stale-thread".to_owned(),
                        data: None,
                    })
                }
            }
        },
        {
            let start_calls = start_calls.clone();
            move |params| {
                let start_calls = start_calls.clone();
                async move {
                    start_calls.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(params.cwd.as_deref(), Some("/tmp/workspace"));
                    assert_eq!(params.model.as_deref(), Some("gpt-5"));
                    Ok("fresh-thread".to_owned())
                }
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(thread_id, "fresh-thread");
    assert_eq!(resume_calls.load(Ordering::Relaxed), 1);
    assert_eq!(start_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn test_resume_or_start_thread_uses_resumed_thread_without_starting_new_one() {
    let resume_calls = Arc::new(AtomicUsize::new(0));
    let start_calls = Arc::new(AtomicUsize::new(0));

    let thread_id = resume_or_start_thread(
        Some("existing-thread".to_owned()),
        ThreadStartParams::default(),
        {
            let resume_calls = resume_calls.clone();
            move |params| {
                let resume_calls = resume_calls.clone();
                async move {
                    resume_calls.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(params.thread_id, "existing-thread");
                    Ok("existing-thread".to_owned())
                }
            }
        },
        {
            let start_calls = start_calls.clone();
            move |_params| {
                let start_calls = start_calls.clone();
                async move {
                    start_calls.fetch_add(1, Ordering::Relaxed);
                    Ok("unexpected-new-thread".to_owned())
                }
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(thread_id, "existing-thread");
    assert_eq!(resume_calls.load(Ordering::Relaxed), 1);
    assert_eq!(start_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn test_map_codex_error() {
    let err = map_codex_error("thread/start failed", CodexError::Fatal("boom".to_owned()));
    assert!(matches!(err, BridgeError::RunFailed(_)));
    assert!(err.to_string().contains("thread/start failed"));
    assert!(err.to_string().contains("boom"));
}

#[test]
fn test_cwd_canonicalization_resolves_dotdot() {
    // /tmp/../tmp should resolve; on macOS /tmp -> /private/tmp
    let input = "/tmp/../tmp";
    let canonical = std::fs::canonicalize(input)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| input.to_owned());
    let expected = std::fs::canonicalize("/tmp")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(canonical, expected);
}

#[test]
fn test_cwd_canonicalization_fallback_for_nonexistent_path() {
    let bogus = "/nonexistent_path_abc123_xyz";
    let result = std::fs::canonicalize(bogus)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| bogus.to_owned());
    assert_eq!(result, bogus);
}

#[test]
fn test_normalize_codex_mcp_servers_canonicalizes_cwd() {
    let mut servers = serde_json::Map::new();
    servers.insert(
        "test-server".to_owned(),
        json!({
            "command": "node",
            "args": ["server.js"],
            "cwd": "/tmp/../tmp"
        }),
    );
    let mut metadata = HashMap::new();
    metadata.insert("remote_mcp_servers".to_owned(), Value::Object(servers));

    let result = normalize_codex_mcp_servers(&metadata).unwrap();
    let server = result.as_object().unwrap().get("test-server").unwrap();
    let cwd = server.get("cwd").unwrap().as_str().unwrap();
    let expected = std::fs::canonicalize("/tmp")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(cwd, expected);
}
