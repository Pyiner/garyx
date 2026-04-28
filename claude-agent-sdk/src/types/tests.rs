use super::*;

#[test]
fn test_default_options_cli_args() {
    let opts = ClaudeAgentOptions::default();
    let args = opts.to_cli_args();

    assert!(args.contains(&"--output-format".to_string()));
    assert!(args.contains(&"stream-json".to_string()));
    assert!(args.contains(&"--verbose".to_string()));
    // Default: empty system prompt
    assert!(args.contains(&"--system-prompt".to_string()));
    // Default: setting-sources with empty string
    assert!(args.contains(&"--setting-sources".to_string()));
}

#[test]
fn test_options_with_model() {
    let opts = ClaudeAgentOptions {
        model: Some("claude-opus-4-6".into()),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args.iter().position(|a| a == "--model").unwrap();
    assert_eq!(args[idx + 1], "claude-opus-4-6");
}

#[test]
fn test_options_with_max_turns_and_budget() {
    let opts = ClaudeAgentOptions {
        max_turns: Some(10),
        max_budget_usd: Some(0.5),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args.iter().position(|a| a == "--max-turns").unwrap();
    assert_eq!(args[idx + 1], "10");
    let idx = args.iter().position(|a| a == "--max-budget-usd").unwrap();
    assert_eq!(args[idx + 1], "0.5");
}

#[test]
fn test_options_with_session_agent() {
    let opts = ClaudeAgentOptions {
        agent: Some("reviewer".into()),
        agents: HashMap::from([(
            "reviewer".into(),
            ClaudeAgentDefinition {
                description: "Code reviewer".into(),
                prompt: "Review code carefully.".into(),
            },
        )]),
        ..Default::default()
    };
    let args = opts.to_cli_args();

    let agent_idx = args.iter().position(|a| a == "--agent").unwrap();
    assert_eq!(args[agent_idx + 1], "reviewer");

    let agents_idx = args.iter().position(|a| a == "--agents").unwrap();
    let payload: Value = serde_json::from_str(&args[agents_idx + 1]).unwrap();
    assert_eq!(payload["reviewer"]["description"], "Code reviewer");
    assert_eq!(payload["reviewer"]["prompt"], "Review code carefully.");
}

#[test]
fn test_options_with_permission_mode() {
    let opts = ClaudeAgentOptions {
        permission_mode: Some(PermissionMode::Auto),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args.iter().position(|a| a == "--permission-mode").unwrap();
    assert_eq!(args[idx + 1], "auto");
}

#[test]
fn test_options_with_disallowed_tools() {
    let opts = ClaudeAgentOptions {
        disallowed_tools: vec!["Bash".into(), "Write".into()],
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args.iter().position(|a| a == "--disallowedTools").unwrap();
    assert_eq!(args[idx + 1], "Bash,Write");
}

#[test]
fn test_options_with_system_prompt() {
    let opts = ClaudeAgentOptions {
        system_prompt: Some("You are a helpful bot".into()),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args.iter().position(|a| a == "--system-prompt").unwrap();
    assert_eq!(args[idx + 1], "You are a helpful bot");
}

#[test]
fn test_options_with_append_system_prompt() {
    let opts = ClaudeAgentOptions {
        append_system_prompt: Some("Extra instructions".into()),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args
        .iter()
        .position(|a| a == "--append-system-prompt")
        .unwrap();
    assert_eq!(args[idx + 1], "Extra instructions");
    // Should NOT have --system-prompt
    assert!(!args.contains(&"--system-prompt".to_string()));
}

#[test]
fn test_options_with_resume() {
    let opts = ClaudeAgentOptions {
        resume: Some("session-abc".into()),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args.iter().position(|a| a == "--resume").unwrap();
    assert_eq!(args[idx + 1], "session-abc");
}

#[test]
fn test_options_with_continue() {
    let opts = ClaudeAgentOptions {
        continue_conversation: true,
        ..Default::default()
    };
    let args = opts.to_cli_args();
    assert!(args.contains(&"--continue".to_string()));
}

#[test]
fn test_options_with_extra_args() {
    let mut extra = HashMap::new();
    extra.insert("debug-to-stderr".to_string(), None);
    extra.insert("replay-user-messages".to_string(), None);
    let opts = ClaudeAgentOptions {
        extra_args: extra,
        ..Default::default()
    };
    let args = opts.to_cli_args();
    assert!(args.contains(&"--debug-to-stderr".to_string()));
    assert!(args.contains(&"--replay-user-messages".to_string()));
}

#[test]
fn test_options_with_mcp_servers() {
    let mut servers = HashMap::new();
    servers.insert(
        "my-server".to_string(),
        McpServerConfig::Stdio {
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            env: HashMap::new(),
        },
    );
    let opts = ClaudeAgentOptions {
        mcp_servers: servers,
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args.iter().position(|a| a == "--mcp-config").unwrap();
    let json: Value = serde_json::from_str(&args[idx + 1]).unwrap();
    assert!(json.get("mcpServers").unwrap().get("my-server").is_some());
}

#[test]
fn test_options_with_output_format() {
    let opts = ClaudeAgentOptions {
        output_format: Some(serde_json::json!({
            "type": "json_schema",
            "schema": { "type": "object", "properties": { "answer": { "type": "string" } } }
        })),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    assert!(args.contains(&"--json-schema".to_string()));
}

#[test]
fn test_options_with_max_thinking_tokens() {
    let opts = ClaudeAgentOptions {
        max_thinking_tokens: Some(4096),
        ..Default::default()
    };
    let args = opts.to_cli_args();
    let idx = args
        .iter()
        .position(|a| a == "--max-thinking-tokens")
        .unwrap();
    assert_eq!(args[idx + 1], "4096");
}

#[test]
fn test_permission_mode_display() {
    assert_eq!(PermissionMode::Default.to_string(), "default");
    assert_eq!(PermissionMode::AcceptEdits.to_string(), "acceptEdits");
    assert_eq!(PermissionMode::Auto.to_string(), "auto");
    assert_eq!(PermissionMode::Plan.to_string(), "plan");
    assert_eq!(
        PermissionMode::BypassPermissions.to_string(),
        "bypassPermissions"
    );
}

#[test]
fn test_content_block_serde() {
    let text = ContentBlock::Text(TextBlock {
        text: "hello".into(),
    });
    let json = serde_json::to_value(&text).unwrap();
    assert_eq!(json["type"], "text");
    assert_eq!(json["text"], "hello");

    let round_trip: ContentBlock = serde_json::from_value(json).unwrap();
    assert_eq!(round_trip, text);
}

#[test]
fn test_tool_use_block_serde() {
    let block = ContentBlock::ToolUse(ToolUseBlock {
        id: "tu-1".into(),
        name: "Bash".into(),
        input: serde_json::json!({ "command": "ls" }),
    });
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "tool_use");
    assert_eq!(json["name"], "Bash");

    let round_trip: ContentBlock = serde_json::from_value(json).unwrap();
    assert_eq!(round_trip, block);
}
