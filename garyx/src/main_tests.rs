use std::sync::Arc;

use clap::{CommandFactory, Parser};

use crate::cli::{
    AgentAction, AutoResearchAction, ChannelsAction, Cli, CommandAction, Commands, ConfigAction,
    DebugAction, GatewayAction, LogsAction, MigrateAction, TaskAction, TeamAction, ThreadAction,
};
use crate::commands::{
    OnboardCommandOptions, canonical_channel_id, cmd_channels_add, cmd_channels_login, cmd_onboard,
    parse_feishu_domain, routing_rebuild_channels, which,
};
use crate::config_support::{default_config_path_string, load_config_or_default};

#[test]
fn verify_cli() {
    // clap's built-in validator that catches conflicts/missing args at test time
    Cli::command().debug_assert();
}

#[test]
fn parse_no_args_requires_explicit_subcommand() {
    let cli = Cli::parse_from(["garyx"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.config, default_config_path_string());
    assert!(cli.port.is_none());
}

#[test]
fn parse_gateway_run() {
    let cli = Cli::parse_from(["garyx", "gateway", "run", "--port", "8080"]);
    match cli.command {
        Some(Commands::Gateway {
            action: GatewayAction::Run { port, .. },
        }) => assert_eq!(port, Some(8080)),
        other => panic!("unexpected command: {:?}", other.is_some()),
    }
}

#[test]
fn parse_gateway_reload_config() {
    let cli = Cli::parse_from(["garyx", "gateway", "reload-config"]);
    match cli.command {
        Some(Commands::Gateway {
            action: GatewayAction::ReloadConfig,
        }) => {}
        _ => panic!("expected Gateway reload-config"),
    }
}

#[test]
fn parse_gateway_start() {
    let cli = Cli::parse_from(["garyx", "gateway", "start"]);
    match cli.command {
        Some(Commands::Gateway {
            action: GatewayAction::Start,
        }) => {}
        _ => panic!("expected Gateway start"),
    }
}

#[test]
fn parse_gateway_restart() {
    let cli = Cli::parse_from(["garyx", "gateway", "restart"]);
    match cli.command {
        Some(Commands::Gateway {
            action: GatewayAction::Restart,
        }) => {}
        _ => panic!("expected Gateway restart"),
    }
}

#[test]
fn parse_gateway_install() {
    let cli = Cli::parse_from(["garyx", "gateway", "install"]);
    match cli.command {
        Some(Commands::Gateway {
            action: GatewayAction::Install,
        }) => {}
        _ => panic!("expected Gateway install"),
    }
}

#[test]
fn parse_gateway_uninstall() {
    let cli = Cli::parse_from(["garyx", "gateway", "uninstall"]);
    match cli.command {
        Some(Commands::Gateway {
            action: GatewayAction::Uninstall,
        }) => {}
        _ => panic!("expected Gateway uninstall"),
    }
}

#[test]
fn parse_global_config() {
    let cli = Cli::parse_from(["garyx", "--config", "/tmp/test.json", "config", "show"]);
    assert_eq!(cli.config, "/tmp/test.json");
    assert!(matches!(
        cli.command,
        Some(Commands::Config {
            action: ConfigAction::Show
        })
    ));
}

#[test]
fn parse_config_validate() {
    let cli = Cli::parse_from(["garyx", "config", "validate"]);
    assert!(matches!(
        cli.command,
        Some(Commands::Config {
            action: ConfigAction::Validate
        })
    ));
}

#[test]
fn parse_commands_list_alias() {
    let cli = Cli::parse_from([
        "garyx",
        "shortcuts",
        "ls",
        "--surface",
        "telegram",
        "--channel",
        "telegram",
        "--account-id",
        "main",
    ]);
    match cli.command {
        Some(Commands::CommandList {
            action:
                CommandAction::List {
                    surface,
                    channel,
                    account_id,
                    ..
                },
        }) => {
            assert_eq!(surface.as_deref(), Some("telegram"));
            assert_eq!(channel.as_deref(), Some("telegram"));
            assert_eq!(account_id.as_deref(), Some("main"));
        }
        _ => panic!("expected commands list"),
    }
}

#[test]
fn parse_commands_set_alias() {
    let cli = Cli::parse_from([
        "garyx",
        "commands",
        "add",
        "/summary",
        "--prompt",
        "summarize this",
        "--description",
        "Summarize",
    ]);
    match cli.command {
        Some(Commands::CommandList {
            action:
                CommandAction::Set {
                    name,
                    prompt,
                    description,
                    ..
                },
        }) => {
            assert_eq!(name, "/summary");
            assert_eq!(prompt.as_deref(), Some("summarize this"));
            assert_eq!(description.as_deref(), Some("Summarize"));
        }
        _ => panic!("expected commands set"),
    }
}

#[test]
fn parse_status() {
    let cli = Cli::parse_from(["garyx", "status", "--json"]);
    match cli.command {
        Some(Commands::Status { json }) => assert!(json),
        _ => panic!("expected Status"),
    }
}

#[test]
fn parse_doctor() {
    let cli = Cli::parse_from(["garyx", "doctor"]);
    match cli.command {
        Some(Commands::Doctor { json }) => assert!(!json),
        _ => panic!("expected Doctor"),
    }
}

#[test]
fn parse_update_command() {
    let cli = Cli::parse_from(["garyx", "update", "--version", "0.1.7"]);
    match cli.command {
        Some(Commands::Update { version, path }) => {
            assert_eq!(version.as_deref(), Some("0.1.7"));
            assert!(path.is_none());
        }
        _ => panic!("expected Update"),
    }
}

#[test]
fn parse_onboard_flags() {
    let cli = Cli::parse_from([
        "garyx",
        "onboard",
        "--api-account",
        "gateway",
        "--search-api-key",
        "search-key",
        "--image-gen-api-key",
        "image-key",
        "--conversation-index-api-key",
        "openai-key",
        "--enable-conversation-index",
        "--conversation-index-model",
        "text-embedding-3-large",
        "--conversation-index-base-url",
        "https://example.invalid/v1",
        "--run-gateway",
    ]);
    match cli.command {
        Some(Commands::Onboard {
            api_account,
            search_api_key,
            image_gen_api_key,
            conversation_index_api_key,
            enable_conversation_index,
            disable_conversation_index,
            conversation_index_model,
            conversation_index_base_url,
            run_gateway,
            ..
        }) => {
            assert_eq!(api_account, "gateway");
            assert_eq!(search_api_key.as_deref(), Some("search-key"));
            assert_eq!(image_gen_api_key.as_deref(), Some("image-key"));
            assert_eq!(conversation_index_api_key.as_deref(), Some("openai-key"));
            assert!(enable_conversation_index);
            assert!(!disable_conversation_index);
            assert_eq!(
                conversation_index_model.as_deref(),
                Some("text-embedding-3-large")
            );
            assert_eq!(
                conversation_index_base_url.as_deref(),
                Some("https://example.invalid/v1")
            );
            assert!(run_gateway);
        }
        _ => panic!("expected Onboard"),
    }
}

#[test]
fn parse_config_set() {
    let cli = Cli::parse_from(["garyx", "config", "set", "gateway.port", "8080"]);
    match cli.command {
        Some(Commands::Config {
            action: ConfigAction::Set { path, value },
        }) => {
            assert_eq!(path, "gateway.port");
            assert_eq!(value, "8080");
        }
        _ => panic!("expected Config::Set"),
    }
}

#[test]
fn parse_channels_enable() {
    let cli = Cli::parse_from(["garyx", "channels", "enable", "telegram", "main", "true"]);
    match cli.command {
        Some(Commands::Channels {
            action:
                ChannelsAction::Enable {
                    channel,
                    account,
                    enabled,
                },
        }) => {
            assert_eq!(channel, "telegram");
            assert_eq!(account, "main");
            assert!(enabled);
        }
        _ => panic!("expected Channels::Enable"),
    }
}

#[test]
fn parse_logs_tail_follow() {
    let cli = Cli::parse_from(["garyx", "logs", "tail", "--lines", "10", "--follow"]);
    match cli.command {
        Some(Commands::Logs {
            action: LogsAction::Tail { lines, follow, .. },
        }) => {
            assert_eq!(lines, 10);
            assert!(follow);
        }
        _ => panic!("expected Logs::Tail"),
    }
}

#[test]
fn parse_debug_thread() {
    let cli = Cli::parse_from([
        "garyx",
        "debug",
        "thread",
        "thread::abc",
        "--limit",
        "5",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Debug {
            action:
                DebugAction::Thread {
                    thread_id,
                    limit,
                    json,
                },
        }) => {
            assert_eq!(thread_id, "thread::abc");
            assert_eq!(limit, 5);
            assert!(json);
        }
        _ => panic!("expected Debug::Thread"),
    }
}

#[test]
fn parse_debug_bot() {
    let cli = Cli::parse_from(["garyx", "debug", "bot", "telegram:main"]);
    match cli.command {
        Some(Commands::Debug {
            action:
                DebugAction::Bot {
                    bot_id,
                    limit,
                    json,
                },
        }) => {
            assert_eq!(bot_id, "telegram:main");
            assert_eq!(limit, 20);
            assert!(!json);
        }
        _ => panic!("expected Debug::Bot"),
    }
}

#[test]
fn parse_auto_research_create() {
    let cli = Cli::parse_from([
        "garyx",
        "auto-research",
        "create",
        "--goal",
        "Compare two options",
        "--json",
    ]);
    match cli.command {
        Some(Commands::AutoResearch {
            action: AutoResearchAction::Create { goal, json, .. },
        }) => {
            assert_eq!(goal, "Compare two options");
            assert!(json);
        }
        _ => panic!("expected AutoResearch::Create"),
    }
}

#[test]
fn parse_auto_research_get() {
    let cli = Cli::parse_from(["garyx", "autoresearch", "get", "ar_123", "--json"]);
    match cli.command {
        Some(Commands::AutoResearch {
            action: AutoResearchAction::Get { run_id, json },
        }) => {
            assert_eq!(run_id, "ar_123");
            assert!(json);
        }
        _ => panic!("expected AutoResearch::Get"),
    }
}

#[test]
fn parse_team_create() {
    let cli = Cli::parse_from([
        "garyx",
        "team",
        "create",
        "--team-id",
        "product-ship",
        "--display-name",
        "Product Ship",
        "--leader-agent-id",
        "planner",
        "--member-agent-id",
        "planner",
        "--member-agent-id",
        "generator",
        "--member-agent-id",
        "reviewer",
        "--workflow-text",
        "Leader plans, generator implements, reviewer validates.",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Team {
            action:
                TeamAction::Create {
                    team_id,
                    display_name,
                    leader_agent_id,
                    member_agent_ids,
                    workflow_text,
                    json,
                    ..
                },
        }) => {
            assert_eq!(team_id, "product-ship");
            assert_eq!(display_name, "Product Ship");
            assert_eq!(leader_agent_id, "planner");
            assert_eq!(member_agent_ids, vec!["planner", "generator", "reviewer"]);
            assert_eq!(
                workflow_text,
                "Leader plans, generator implements, reviewer validates."
            );
            assert!(json);
        }
        _ => panic!("expected Team::Create"),
    }
}

#[test]
fn parse_team_update() {
    let cli = Cli::parse_from([
        "garyx",
        "team",
        "update",
        "product-ship",
        "--new-team-id",
        "product-ship-v2",
        "--display-name",
        "Product Ship V2",
        "--leader-agent-id",
        "planner",
        "--member-agent-id",
        "planner",
        "--member-agent-id",
        "generator",
        "--workflow-text",
        "Leader plans, generator executes.",
    ]);
    match cli.command {
        Some(Commands::Team {
            action:
                TeamAction::Update {
                    team_id,
                    new_team_id,
                    display_name,
                    member_agent_ids,
                    ..
                },
        }) => {
            assert_eq!(team_id, "product-ship");
            assert_eq!(new_team_id.as_deref(), Some("product-ship-v2"));
            assert_eq!(display_name, "Product Ship V2");
            assert_eq!(member_agent_ids, vec!["planner", "generator"]);
        }
        _ => panic!("expected Team::Update"),
    }
}

#[test]
fn parse_thread_create_with_agent() {
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "create",
        "--workspace-dir",
        "/tmp/garyx",
        "--agent-id",
        "product-ship",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Create {
                    workspace_dir,
                    agent_id,
                    json,
                    ..
                },
        }) => {
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx"));
            assert_eq!(agent_id.as_deref(), Some("product-ship"));
            assert!(json);
        }
        _ => panic!("expected Thread::Create"),
    }
}

#[test]
fn parse_thread_create_with_team_id_uses_same_flag() {
    // A team id goes through the same `--agent-id` flag as a standalone agent
    // id: the CLI layer does not care whether the id resolves to a team. The
    // gateway's resolver (§4.2 of agent-team-provider.md) decides which
    // provider to dispatch to.
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "create",
        "--agent-id",
        "product-ship-team",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Create {
                    agent_id,
                    workspace_dir,
                    title,
                    json,
                },
        }) => {
            assert_eq!(agent_id.as_deref(), Some("product-ship-team"));
            assert!(workspace_dir.is_none());
            assert!(title.is_none());
            assert!(!json);
        }
        _ => panic!("expected Thread::Create"),
    }
}

#[test]
fn parse_thread_list_include_hidden() {
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "list",
        "--include-hidden",
        "--limit",
        "5",
        "--offset",
        "2",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::List {
                    include_hidden,
                    limit,
                    offset,
                    json,
                },
        }) => {
            assert!(include_hidden);
            assert_eq!(limit, 5);
            assert_eq!(offset, 2);
            assert!(!json);
        }
        _ => panic!("expected Thread::List"),
    }
}

#[test]
fn parse_thread_send_thread_target() {
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "send",
        "thread",
        "thread::abc",
        "hello world",
        "--workspace-dir",
        "/tmp/garyx",
        "--timeout",
        "42",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Send {
                    kind,
                    target,
                    message,
                    bot,
                    workspace_dir,
                    timeout,
                    json,
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("thread"));
            assert_eq!(target.as_deref(), Some("thread::abc"));
            assert_eq!(message, vec!["hello world".to_owned()]);
            assert_eq!(bot, None);
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx"));
            assert_eq!(timeout, 42);
            assert!(json);
        }
        _ => panic!("expected Thread::Send"),
    }
}

#[test]
fn parse_thread_send_bot_target() {
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "send",
        "bot",
        "telegram:codex_bot",
        "hello",
        "world",
        "--workspace-dir",
        "/tmp/garyx",
        "--timeout",
        "42",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Send {
                    kind,
                    target,
                    message,
                    bot,
                    workspace_dir,
                    timeout,
                    json,
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("bot"));
            assert_eq!(target.as_deref(), Some("telegram:codex_bot"));
            assert_eq!(message, vec!["hello".to_owned(), "world".to_owned()]);
            assert_eq!(bot, None);
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx"));
            assert_eq!(timeout, 42);
            assert!(json);
        }
        _ => panic!("expected Thread::Send"),
    }
}

#[test]
fn parse_thread_send_task_target() {
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "send",
        "task",
        "#telegram/main/1",
        "status?",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Send {
                    kind,
                    target,
                    message,
                    bot,
                    workspace_dir,
                    timeout,
                    json,
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("task"));
            assert_eq!(target.as_deref(), Some("#telegram/main/1"));
            assert_eq!(message, vec!["status?".to_owned()]);
            assert_eq!(bot, None);
            assert_eq!(workspace_dir, None);
            assert_eq!(timeout, 300);
            assert!(!json);
        }
        _ => panic!("expected Thread::Send"),
    }
}

#[test]
fn parse_thread_send_legacy_thread_id() {
    let cli = Cli::parse_from(["garyx", "thread", "send", "thread::abc", "hello"]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Send {
                    kind,
                    target,
                    message,
                    bot,
                    ..
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("thread::abc"));
            assert_eq!(target.as_deref(), Some("hello"));
            assert!(message.is_empty());
            assert_eq!(bot, None);
        }
        _ => panic!("expected Thread::Send"),
    }
}

#[test]
fn parse_task_create_runtime_options() {
    let cli = Cli::parse_from([
        "garyx",
        "task",
        "create",
        "telegram/main",
        "--title",
        "Investigate",
        "--body",
        "Check logs",
        "--assignee",
        "agent:reviewer",
        "--start",
        "--agent-id",
        "codex",
        "--workspace-dir",
        "/tmp/garyx-task",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Task {
            action:
                TaskAction::Create {
                    scope,
                    title,
                    body,
                    assignee,
                    start,
                    agent_id,
                    workspace_dir,
                    json,
                },
        }) => {
            assert_eq!(scope, "telegram/main");
            assert_eq!(title.as_deref(), Some("Investigate"));
            assert_eq!(body.as_deref(), Some("Check logs"));
            assert_eq!(assignee.as_deref(), Some("agent:reviewer"));
            assert!(start);
            assert_eq!(agent_id.as_deref(), Some("codex"));
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx-task"));
            assert!(json);
        }
        _ => panic!("expected Task::Create"),
    }
}

#[test]
fn parse_agent_create() {
    let cli = Cli::parse_from([
        "garyx",
        "agent",
        "create",
        "--agent-id",
        "spec-review",
        "--display-name",
        "Spec Review",
        "--provider",
        "codex_app_server",
        "--model",
        "gpt-5",
        "--system-prompt",
        "Review specs carefully.",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Agent {
            action:
                AgentAction::Create {
                    agent_id,
                    display_name,
                    provider,
                    model,
                    system_prompt,
                    json,
                },
        }) => {
            assert_eq!(agent_id, "spec-review");
            assert_eq!(display_name, "Spec Review");
            assert_eq!(provider, "codex_app_server");
            assert_eq!(model.as_deref(), Some("gpt-5"));
            assert_eq!(system_prompt, "Review specs carefully.");
            assert!(json);
        }
        _ => panic!("expected Agent::Create"),
    }
}

#[test]
fn parse_agent_update_without_model() {
    let cli = Cli::parse_from([
        "garyx",
        "agent",
        "update",
        "--agent-id",
        "spec-review",
        "--display-name",
        "Spec Review",
        "--provider",
        "codex_app_server",
        "--system-prompt",
        "Review specs carefully.",
    ]);
    match cli.command {
        Some(Commands::Agent {
            action:
                AgentAction::Update {
                    agent_id,
                    display_name,
                    provider,
                    model,
                    system_prompt,
                    json,
                },
        }) => {
            assert_eq!(agent_id, "spec-review");
            assert_eq!(display_name, "Spec Review");
            assert_eq!(provider, "codex_app_server");
            assert_eq!(model, None);
            assert_eq!(system_prompt, "Review specs carefully.");
            assert!(!json);
        }
        _ => panic!("expected Agent::Update"),
    }
}

#[test]
fn parse_agent_upsert() {
    let cli = Cli::parse_from([
        "garyx",
        "agent",
        "upsert",
        "--agent-id",
        "spec-review",
        "--display-name",
        "Spec Review",
        "--provider",
        "codex_app_server",
        "--model",
        "gpt-5",
        "--system-prompt",
        "Review specs carefully.",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Agent {
            action:
                AgentAction::Upsert {
                    agent_id,
                    display_name,
                    provider,
                    model,
                    system_prompt,
                    json,
                },
        }) => {
            assert_eq!(agent_id, "spec-review");
            assert_eq!(display_name, "Spec Review");
            assert_eq!(provider, "codex_app_server");
            assert_eq!(model.as_deref(), Some("gpt-5"));
            assert_eq!(system_prompt, "Review specs carefully.");
            assert!(json);
        }
        _ => panic!("expected Agent::Upsert"),
    }
}

#[test]
fn parse_migrate_thread_transcripts() {
    let cli = Cli::parse_from([
        "garyx",
        "migrate",
        "thread-transcripts",
        "--data-dir",
        "/tmp/gary-data",
        "--backup-dir",
        "/tmp/gary-backup",
        "--rewrite-records",
    ]);
    match cli.command {
        Some(Commands::Migrate {
            action:
                MigrateAction::ThreadTranscripts {
                    data_dir,
                    backup_dir,
                    rewrite_records,
                },
        }) => {
            assert_eq!(data_dir.as_deref(), Some("/tmp/gary-data"));
            assert_eq!(backup_dir.as_deref(), Some("/tmp/gary-backup"));
            assert!(rewrite_records);
        }
        _ => panic!("expected Migrate::ThreadTranscripts"),
    }
}

#[test]
fn which_finds_common_binary() {
    // `ls` should exist on any Unix system
    assert!(which("ls"));
    assert!(!which("nonexistent_binary_xyz_42"));
}

#[test]
fn load_config_or_default_missing_file() {
    let loaded = load_config_or_default(
        "/tmp/nonexistent_garyx_test.json",
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    assert_eq!(loaded.config.gateway.port, 31337);
}

#[tokio::test]
async fn onboard_writes_gateway_keys_and_api_account() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    cmd_onboard(
        &config_path.display().to_string(),
        OnboardCommandOptions {
            force: false,
            json: true,
            api_account: "main".to_owned(),
            search_api_key: Some("search-key".to_owned()),
            image_gen_api_key: Some("image-key".to_owned()),
            conversation_index_api_key: Some("openai-key".to_owned()),
            enable_conversation_index: false,
            disable_conversation_index: false,
            conversation_index_model: Some("text-embedding-3-large".to_owned()),
            conversation_index_base_url: Some("https://example.invalid/v1".to_owned()),
            run_gateway: false,
            port_override: None,
            host_override: None,
            no_channels: true,
        },
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let config = loaded.config;
    let api_account = config.channels.api.accounts.get("main").unwrap();
    assert!(api_account.enabled);
    assert_eq!(config.gateway.search.api_key, "search-key");
    assert_eq!(config.gateway.image_gen.api_key, "image-key");
    assert_eq!(config.gateway.conversation_index.api_key, "openai-key");
    assert!(config.gateway.conversation_index.enabled);
    assert_eq!(
        config.gateway.conversation_index.model,
        "text-embedding-3-large"
    );
    assert_eq!(
        config.gateway.conversation_index.base_url,
        "https://example.invalid/v1"
    );
}

#[tokio::test]
async fn onboard_updates_existing_config_without_resetting_other_fields() {
    use garyx_models::config::{ApiAccount, GaryxConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    let mut initial = GaryxConfig::default();
    initial.gateway.search.api_key = "keep-search-key".to_owned();
    initial.gateway.conversation_index.api_key = "keep-openai-key".to_owned();
    initial.gateway.conversation_index.enabled = true;
    initial.channels.api.accounts.insert(
        "custom".to_owned(),
        ApiAccount {
            enabled: false,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );
    std::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap()).unwrap();

    cmd_onboard(
        &config_path.display().to_string(),
        OnboardCommandOptions {
            force: false,
            json: true,
            api_account: "custom".to_owned(),
            search_api_key: None,
            image_gen_api_key: Some("new-image-key".to_owned()),
            conversation_index_api_key: None,
            enable_conversation_index: false,
            disable_conversation_index: true,
            conversation_index_model: None,
            conversation_index_base_url: None,
            run_gateway: false,
            port_override: None,
            host_override: None,
            no_channels: true,
        },
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let config = loaded.config;
    let api_account = config.channels.api.accounts.get("custom").unwrap();
    assert!(api_account.enabled);
    assert_eq!(config.gateway.search.api_key, "keep-search-key");
    assert_eq!(config.gateway.image_gen.api_key, "new-image-key");
    assert_eq!(config.gateway.conversation_index.api_key, "keep-openai-key");
    assert!(!config.gateway.conversation_index.enabled);
}

// -------------------------------------------------------------------------
// Non-interactive `garyx channels add` smoke tests
//
// These exercise the CLI-flag-driven path of `cmd_channels_add` to pin the
// contract we document in the README / release notes: every channel that
// doesn't fundamentally require a human (i.e. everything except Weixin QR
// scan) can be configured head-lessly in CI or scripts. cargo test runs
// with stdin detached, so `stdin_is_interactive()` reports false and the
// function skips the interactive prompt block entirely.
// -------------------------------------------------------------------------

fn write_empty_config(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("gary.json");
    let cfg = garyx_models::config::GaryxConfig::default();
    std::fs::write(&path, serde_json::to_vec_pretty(&cfg).unwrap()).unwrap();
    path
}

#[tokio::test]
async fn channels_add_non_interactive_telegram() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    cmd_channels_add(
        &config_path.display().to_string(),
        Some("telegram".to_owned()),
        Some("alice".to_owned()),
        Some("Alice Bot".to_owned()),
        None,
        None,
        Some("123:ABCDEF".to_owned()),
        None,
        None,
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let telegram = loaded.config.channels.resolved_telegram_config().unwrap();
    let account = telegram.accounts.get("alice").unwrap();
    assert!(account.enabled);
    assert_eq!(account.token, "123:ABCDEF");
    assert_eq!(account.name.as_deref(), Some("Alice Bot"));
    assert_eq!(account.agent_id, "claude");
}

#[tokio::test]
async fn channels_add_non_interactive_feishu() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    cmd_channels_add(
        &config_path.display().to_string(),
        Some("feishu".to_owned()),
        Some("myapp".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        Some("cli_abcdef".to_owned()),
        Some("s_xyz".to_owned()),
        None,
        false,
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let account = loaded
        .config
        .channels
        .plugins
        .get("feishu")
        .and_then(|plugin| plugin.accounts.get("myapp"))
        .unwrap();
    assert!(account.enabled);
    assert_eq!(account.config["app_id"], "cli_abcdef");
    assert_eq!(account.config["app_secret"], "s_xyz");
    assert_eq!(account.agent_id.as_deref(), Some("claude"));
}

#[tokio::test]
async fn channels_add_non_interactive_api_only_needs_account() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    cmd_channels_add(
        &config_path.display().to_string(),
        Some("api".to_owned()),
        Some("scripted".to_owned()),
        None,
        Some("/tmp/ws".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let account = loaded.config.channels.api.accounts.get("scripted").unwrap();
    assert!(account.enabled);
    assert_eq!(account.workspace_dir.as_deref(), Some("/tmp/ws"));
    assert_eq!(account.agent_id, "claude");
}

#[tokio::test]
async fn channels_add_non_interactive_persists_explicit_agent_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    cmd_channels_add(
        &config_path.display().to_string(),
        Some("api".to_owned()),
        Some("scripted".to_owned()),
        None,
        None,
        Some("product-ship".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let account = loaded.config.channels.api.accounts.get("scripted").unwrap();
    assert_eq!(account.agent_id, "product-ship");
}

#[tokio::test]
async fn channels_add_non_interactive_weixin_accepts_preexisting_token() {
    // Users who already hold a Weixin bot_token (e.g. scanned once on their
    // laptop) should be able to script subsequent hosts via --token, even
    // without a TTY. This pins that path so we don't accidentally make
    // Weixin QR-only.
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    cmd_channels_add(
        &config_path.display().to_string(),
        Some("weixin".to_owned()),
        Some("wxbot".to_owned()),
        None,
        None,
        None,
        Some("pre-scanned-token".to_owned()),
        None,
        Some("https://ilinkai.weixin.qq.com".to_owned()),
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let account = loaded
        .config
        .channels
        .plugins
        .get("weixin")
        .and_then(|plugin| plugin.accounts.get("wxbot"))
        .unwrap();
    assert!(account.enabled);
    assert_eq!(account.config["token"], "pre-scanned-token");
    assert_eq!(account.config["base_url"], "https://ilinkai.weixin.qq.com");
}

#[tokio::test]
async fn channels_add_non_interactive_rejects_missing_required_token() {
    // Negative test: missing --token under non-interactive mode must error
    // rather than silently inserting an unauthenticated bot.
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    let err = cmd_channels_add(
        &config_path.display().to_string(),
        Some("telegram".to_owned()),
        Some("alice".to_owned()),
        None,
        None,
        None,
        None, // no --token
        None,
        None,
        None,
        None,
        None,
        false,
    )
    .await
    .expect_err("non-interactive telegram add without --token should fail");
    assert!(
        err.to_string().contains("token"),
        "error should mention the missing token, got: {err}"
    );

    // Config should be untouched (no half-inserted account).
    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    assert!(
        loaded
            .config
            .channels
            .plugins
            .get("telegram")
            .map(|plugin| plugin.accounts.is_empty())
            .unwrap_or(true)
    );
}

#[tokio::test]
async fn channels_add_non_interactive_rejects_missing_feishu_app_secret() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    let err = cmd_channels_add(
        &config_path.display().to_string(),
        Some("feishu".to_owned()),
        Some("myapp".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        Some("cli_abcdef".to_owned()),
        None, // missing --app-secret
        None,
        false,
    )
    .await
    .expect_err("non-interactive feishu add without --app-secret should fail");
    assert!(err.to_string().contains("app_secret") || err.to_string().contains("app-secret"));
}

// -------------------------------------------------------------------------
// Feishu device-flow integration (`--auto-register`) — input handling
//
// The full device-flow HTTP round-trip is covered by wiremock-based unit
// tests in `garyx_channels::feishu::device_auth`. These tests pin the CLI
// plumbing: domain parsing, persistence into the `FeishuAccount::domain`
// field, and the guardrails that stop users from mixing `--auto-register`
// with explicit credentials.
// -------------------------------------------------------------------------

#[test]
fn parse_feishu_domain_accepts_common_synonyms() {
    use garyx_models::config::FeishuDomain;
    assert_eq!(parse_feishu_domain("feishu"), Some(FeishuDomain::Feishu));
    assert_eq!(parse_feishu_domain("FEISHU"), Some(FeishuDomain::Feishu));
    assert_eq!(parse_feishu_domain("  cn  "), Some(FeishuDomain::Feishu));
    assert_eq!(parse_feishu_domain("飞书"), Some(FeishuDomain::Feishu));
    assert_eq!(parse_feishu_domain("国内"), Some(FeishuDomain::Feishu));
    assert_eq!(parse_feishu_domain("domestic"), Some(FeishuDomain::Feishu));

    assert_eq!(parse_feishu_domain("lark"), Some(FeishuDomain::Lark));
    assert_eq!(parse_feishu_domain("LARK"), Some(FeishuDomain::Lark));
    assert_eq!(parse_feishu_domain("larksuite"), Some(FeishuDomain::Lark));
    assert_eq!(parse_feishu_domain("intl"), Some(FeishuDomain::Lark));
    assert_eq!(parse_feishu_domain("海外"), Some(FeishuDomain::Lark));

    // Unrecognized inputs fall through to None so the caller can fall back
    // to the default (Feishu) or prompt the user.
    assert_eq!(parse_feishu_domain("teams"), None);
    assert_eq!(parse_feishu_domain(""), None);
    assert_eq!(parse_feishu_domain("cn-feishu-old"), None);
}

#[test]
fn canonical_channel_id_resolves_builtin_aliases() {
    assert_eq!(canonical_channel_id("telegram"), "telegram");
    assert_eq!(canonical_channel_id("TG"), "telegram");
    assert_eq!(canonical_channel_id("lark"), "feishu");
    assert_eq!(canonical_channel_id("wechat"), "weixin");
    assert_eq!(canonical_channel_id("api"), "api");
}

#[tokio::test]
async fn channels_add_feishu_persists_lark_domain_when_flag_set() {
    // Users on a Lark tenant should be able to script `garyx channels add
    // feishu --app-id ... --app-secret ... --domain lark` and have the
    // resulting config write `domain: "lark"` so the channel picks the
    // larksuite API base at runtime.
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    cmd_channels_add(
        &config_path.display().to_string(),
        Some("feishu".to_owned()),
        Some("larkbot".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        Some("cli_lark".to_owned()),
        Some("s_lark".to_owned()),
        Some("lark".to_owned()),
        false,
    )
    .await
    .unwrap();

    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    let feishu = loaded.config.channels.resolved_feishu_config().unwrap();
    let account = feishu
        .accounts
        .get("larkbot")
        .expect("larkbot should exist");
    assert_eq!(account.app_id, "cli_lark");
    assert_eq!(account.app_secret, "s_lark");
    assert_eq!(account.domain, garyx_models::config::FeishuDomain::Lark);
}

#[tokio::test]
async fn channels_add_rejects_auto_register_with_explicit_credentials() {
    // `--auto-register` runs the device flow to *obtain* credentials, so
    // mixing it with `--app-id` / `--app-secret` is always user error.
    // The CLI should refuse before we fire an HTTP request — this test
    // pins that guard so we don't regress and start silently discarding
    // one of the inputs.
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    let err = cmd_channels_add(
        &config_path.display().to_string(),
        Some("feishu".to_owned()),
        Some("conflict".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        Some("cli_abc".to_owned()),
        Some("s_xyz".to_owned()),
        None,
        true, // --auto-register
    )
    .await
    .expect_err("auto-register combined with explicit credentials should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("auto-register") || msg.contains("auto_register"),
        "error should mention auto-register, got: {msg}"
    );

    // Config is untouched.
    let loaded = load_config_or_default(
        &config_path.display().to_string(),
        garyx_models::config_loader::ConfigRuntimeOverrides::default(),
    )
    .unwrap();
    assert!(
        loaded
            .config
            .channels
            .plugins
            .get("feishu")
            .map(|plugin| plugin.accounts.is_empty())
            .unwrap_or(true)
    );
}

#[test]
fn cli_channels_add_parses_auto_register_and_domain_flags() {
    // Pin the flag surface we just added so any future rename of
    // `--auto-register` / `--domain` needs an explicit migration.
    let cli = Cli::parse_from([
        "garyx",
        "channels",
        "add",
        "feishu",
        "mybot",
        "--domain",
        "lark",
        "--auto-register",
    ]);
    match cli.command {
        Some(Commands::Channels {
            action:
                ChannelsAction::Add {
                    channel,
                    account,
                    domain,
                    auto_register,
                    ..
                },
        }) => {
            assert_eq!(channel.as_deref(), Some("feishu"));
            assert_eq!(account.as_deref(), Some("mybot"));
            assert_eq!(domain.as_deref(), Some("lark"));
            assert!(auto_register, "--auto-register should flip the bool");
        }
        _ => panic!("unexpected parse result: expected Channels::Add"),
    }
}

#[test]
fn cli_channels_add_parses_agent_id_flag() {
    let cli = Cli::parse_from([
        "garyx",
        "channels",
        "add",
        "telegram",
        "mybot",
        "--agent-id",
        "product-ship",
    ]);
    match cli.command {
        Some(Commands::Channels {
            action: ChannelsAction::Add { agent_id, .. },
        }) => {
            assert_eq!(agent_id.as_deref(), Some("product-ship"));
        }
        _ => panic!("unexpected parse result: expected Channels::Add"),
    }
}

#[test]
fn cli_channels_login_parses_feishu_domain_flag() {
    let cli = Cli::parse_from([
        "garyx",
        "channels",
        "login",
        "feishu",
        "--account",
        "mybot",
        "--reauthorize",
        "oldbot",
        "--forget-previous",
        "--name",
        "My Bot",
        "--workspace-dir",
        "/tmp/garyx-login",
        "--agent-id",
        "product-ship",
        "--domain",
        "lark",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Channels {
            action:
                ChannelsAction::Login {
                    channel,
                    account,
                    reauthorize,
                    forget_previous,
                    name,
                    workspace_dir,
                    agent_id,
                    domain,
                    json,
                    ..
                },
        }) => {
            assert_eq!(channel, "feishu");
            assert_eq!(account.as_deref(), Some("mybot"));
            assert_eq!(reauthorize.as_deref(), Some("oldbot"));
            assert!(forget_previous);
            assert_eq!(name.as_deref(), Some("My Bot"));
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx-login"));
            assert_eq!(agent_id.as_deref(), Some("product-ship"));
            assert_eq!(domain.as_deref(), Some("lark"));
            assert!(json);
        }
        _ => panic!("unexpected parse result: expected Channels::Login"),
    }
}

#[tokio::test]
async fn channels_login_rejects_unsupported_channel() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    let err = cmd_channels_login(
        &config_path.display().to_string(),
        "telegram",
        Some("alice".to_owned()),
        None,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        30,
        false,
    )
    .await
    .expect_err("telegram login should not be supported");
    assert!(
        err.to_string().contains("feishu/weixin"),
        "unexpected error: {err}"
    );
}

#[test]
fn default_config_path_points_to_hidden_garyx_dir() {
    let path = default_config_path_string();
    assert!(path.ends_with(".garyx/garyx.json") || path.ends_with(".garyx\\garyx.json"));
}

#[test]
fn default_config_omits_legacy_thread_history_backend() {
    let config = garyx_models::config::GaryxConfig::default();
    let value = serde_json::to_value(config).expect("default config should serialize");
    assert!(value.pointer("/sessions/thread_history_backend").is_none());
}

#[test]
fn plugin_discovery_with_valid_config() {
    use garyx_bridge::MultiProviderBridge;
    use garyx_channels::{BuiltInPluginDiscoverer, ChannelPluginManager};
    use garyx_models::config::{
        FeishuAccount, GaryxConfig, TelegramAccount, feishu_account_to_plugin_entry,
        telegram_account_to_plugin_entry,
    };
    use garyx_router::{InMemoryThreadStore, MessageRouter, ThreadStore};
    use tokio::sync::Mutex;

    let mut config = GaryxConfig::default();
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(store, config.clone())));
    let bridge = Arc::new(MultiProviderBridge::new());

    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "test".to_owned(),
            telegram_account_to_plugin_entry(&TelegramAccount {
                token: "fake-token".to_owned(),
                enabled: true,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: Default::default(),
            }),
        );

    config
        .channels
        .plugin_channel_mut("feishu")
        .accounts
        .insert(
            "test".to_owned(),
            feishu_account_to_plugin_entry(&FeishuAccount {
                app_id: "cli_test".to_owned(),
                app_secret: "secret".to_owned(),
                enabled: true,
                domain: Default::default(),
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                require_mention: true,
                topic_session_mode: Default::default(),
            }),
        );

    let discoverer =
        BuiltInPluginDiscoverer::new(config.channels.clone(), router, bridge, String::new());
    let mut manager = ChannelPluginManager::new();
    manager.discover_and_register(&discoverer).unwrap();
    let statuses = manager.statuses();
    let plugin_ids = statuses
        .iter()
        .map(|status| status.metadata.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(statuses.len(), 3);
    assert!(plugin_ids.contains(&"telegram"));
    assert!(plugin_ids.contains(&"feishu"));
    assert!(plugin_ids.contains(&"weixin"));
}

#[test]
fn routing_rebuild_channels_prefers_enabled_accounts() {
    use garyx_models::config::{
        FeishuAccount, GaryxConfig, TelegramAccount, feishu_account_to_plugin_entry,
        telegram_account_to_plugin_entry,
    };

    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "tg1".to_owned(),
            telegram_account_to_plugin_entry(&TelegramAccount {
                token: "t".to_owned(),
                enabled: false,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: Default::default(),
            }),
        );
    config
        .channels
        .plugin_channel_mut("feishu")
        .accounts
        .insert(
            "fs1".to_owned(),
            feishu_account_to_plugin_entry(&FeishuAccount {
                app_id: "a".to_owned(),
                app_secret: "s".to_owned(),
                enabled: true,
                domain: Default::default(),
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                require_mention: true,
                topic_session_mode: Default::default(),
            }),
        );

    assert_eq!(routing_rebuild_channels(&config), vec!["feishu"]);
}

#[test]
fn routing_rebuild_channels_includes_all_enabled_channels() {
    use garyx_models::config::{
        ApiAccount, FeishuAccount, GaryxConfig, TelegramAccount, feishu_account_to_plugin_entry,
        telegram_account_to_plugin_entry,
    };

    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "tg1".to_owned(),
            telegram_account_to_plugin_entry(&TelegramAccount {
                token: "t".to_owned(),
                enabled: true,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: Default::default(),
            }),
        );
    config
        .channels
        .plugin_channel_mut("feishu")
        .accounts
        .insert(
            "fs1".to_owned(),
            feishu_account_to_plugin_entry(&FeishuAccount {
                app_id: "a".to_owned(),
                app_secret: "s".to_owned(),
                enabled: true,
                domain: Default::default(),
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                require_mention: true,
                topic_session_mode: Default::default(),
            }),
        );
    config.channels.api.accounts.insert(
        "api1".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );

    let mut channels = routing_rebuild_channels(&config);
    channels.sort();
    assert_eq!(channels, vec!["api", "feishu", "telegram"]);
}

#[test]
fn routing_rebuild_channels_falls_back_when_none_enabled() {
    use garyx_models::config::GaryxConfig;

    let config = GaryxConfig::default();
    assert!(routing_rebuild_channels(&config).is_empty());
}

#[tokio::test]
async fn startup_runtime_wiring_enables_operational_handlers() {
    use crate::runtime_assembler::RuntimeAssembler;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use garyx_gateway::api;
    use garyx_models::config::{CronAction, CronJobConfig, CronSchedule, GaryxConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(tmp.path().display().to_string());
    config.cron.jobs.push(CronJobConfig {
        id: "startup-job".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
    });

    // Test sets process env before runtime assembly and clears it immediately after.
    unsafe {
        std::env::set_var("GARYX_RESTART_TOKENS", "restart-token");
    }
    let state = RuntimeAssembler::new(tmp.path().join("gary.json"), config)
        .assemble()
        .await
        .unwrap()
        .state;
    unsafe {
        std::env::remove_var("GARYX_RESTART_TOKENS");
    }

    let resp = api::cron_jobs(State(state.clone())).await.into_response();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["service_available"], true);

    let resp = api::restart(State(state), HeaderMap::new())
        .await
        .into_response();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn startup_runtime_rebuilds_indexes_and_workspace_bindings_from_canonical_threads() {
    use crate::runtime_assembler::RuntimeAssembler;
    use garyx_models::config::{GaryxConfig, TelegramAccount};
    use garyx_router::{FileThreadStore, ThreadStore};
    use serde_json::json;

    let tmp = tempfile::TempDir::new().unwrap();
    let session_dir = tmp.path().join("sessions");
    let store: Arc<dyn ThreadStore> = Arc::new(FileThreadStore::new(&session_dir).await.unwrap());
    store
        .set(
            "thread::startup-alice",
            json!({
                "thread_id": "thread::startup-alice",
                "label": "Alice",
                "workspace_dir": "/tmp/runtime-assembler-ws",
                "messages": [{"role": "user", "content": "hello"}],
                "sdk_session_id": "sdk-123",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice"
                }]
            }),
        )
        .await;

    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(session_dir.display().to_string());
    let account = TelegramAccount {
        token: "token".to_owned(),
        enabled: true,
        name: None,
        agent_id: "claude".to_owned(),
        workspace_dir: None,
        owner_target: None,
        groups: Default::default(),
    };
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&account),
        );

    let assembly = RuntimeAssembler::new(tmp.path().join("gary.json"), config)
        .assemble()
        .await
        .unwrap();

    let keys = assembly.state.threads.thread_store.list_keys(None).await;
    assert!(keys.iter().any(|key| key == "thread::startup-alice"));

    let thread = assembly
        .state
        .threads
        .thread_store
        .get("thread::startup-alice")
        .await
        .unwrap();
    assert_eq!(thread["sdk_session_id"], "sdk-123");
    assert_eq!(thread["workspace_dir"], "/tmp/runtime-assembler-ws");

    let mut router = assembly.state.threads.router.lock().await;
    assert_eq!(
        router
            .resolve_endpoint_thread_id("telegram", "main", "alice")
            .await
            .as_deref(),
        Some("thread::startup-alice")
    );
    drop(router);

    let workspace_bindings = assembly.bridge.thread_workspace_bindings_snapshot().await;
    assert_eq!(
        workspace_bindings
            .get("thread::startup-alice")
            .map(String::as_str),
        Some("/tmp/runtime-assembler-ws")
    );
}
