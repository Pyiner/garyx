use std::sync::Arc;

use crate::{ThreadSendTarget, resolve_thread_send_destination};

use clap::{CommandFactory, Parser};

use crate::cli::{
    AgentAction, AutomationAction, BotAction, BotEndpointAction, ChannelsAction, Cli,
    CommandAction, Commands, ConfigAction, GatewayAction, LogsAction, ProviderAction, TaskAction,
    ThreadAction, ToolAction,
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
fn root_version_token_matches_only_first_arg() {
    // B0: `-V` / `--version` as argv[1] are the side-effect-free
    // version query that gets short-circuited before home migration.
    assert!(crate::is_root_version_token(Some("--version")));
    assert!(crate::is_root_version_token(Some("-V")));
}

#[test]
fn root_version_token_ignores_update_version_flag() {
    // `garyx update --version <ver>` puts `--version` at argv[2], not
    // argv[1]; a full-argv scan would wrongly short-circuit the update.
    // The pre-scan only ever inspects argv[1], so it must NOT match the
    // `update` subcommand token here.
    assert!(!crate::is_root_version_token(Some("update")));
    assert!(!crate::is_root_version_token(Some("gateway")));
    assert!(!crate::is_root_version_token(Some("-v")));
    assert!(!crate::is_root_version_token(None));
}

#[test]
fn parse_no_args_requires_explicit_subcommand() {
    let cli = Cli::parse_from(["garyx"]);
    assert!(cli.command.is_none());
    assert!(
        cli.config.ends_with(".garyx/garyx.json") || cli.config.ends_with(".garyx\\garyx.json")
    );
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
fn parse_gateway_rotate_store_incarnation() {
    let cli = Cli::parse_from(["garyx", "gateway", "rotate-store-incarnation"]);
    assert!(matches!(
        cli.command,
        Some(Commands::Gateway {
            action: GatewayAction::RotateStoreIncarnation
        })
    ));
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
fn parse_gateway_restart_rejects_retired_wake_flags() {
    for flags in [
        vec!["--no-wake"],
        vec!["--wake", "all"],
        vec!["--wake", "thread", "thread::abc"],
        vec!["--wake-message", "continue"],
        vec!["--wake-json"],
    ] {
        let mut args = vec!["garyx", "gateway", "restart"];
        args.extend(flags.iter());
        assert!(
            Cli::try_parse_from(&args).is_err(),
            "retired restart flag should be rejected: {flags:?}"
        );
    }
}

#[test]
fn thread_send_destination_resolves_canonical_and_legacy_positional_forms() {
    let destination = resolve_thread_send_destination(
        Some("thread".to_owned()),
        Some("thread::abc".to_owned()),
        vec!["hello".to_owned()],
    )
    .expect("thread form");
    assert_eq!(
        destination.target,
        ThreadSendTarget::Thread("thread::abc".to_owned())
    );
    assert_eq!(destination.message_parts, vec!["hello".to_owned()]);

    let destination = resolve_thread_send_destination(
        Some("bot".to_owned()),
        Some("telegram:main".to_owned()),
        vec!["continue".to_owned()],
    )
    .expect("bot form");
    assert_eq!(
        destination.target,
        ThreadSendTarget::Bot("telegram:main".to_owned())
    );

    // Documented compatibility shorthand: a bare canonical thread id.
    let destination = resolve_thread_send_destination(
        Some("thread::abc".to_owned()),
        Some("hello".to_owned()),
        vec!["there".to_owned()],
    )
    .expect("legacy positional form");
    assert_eq!(
        destination.target,
        ThreadSendTarget::Thread("thread::abc".to_owned())
    );
    assert_eq!(
        destination.message_parts,
        vec!["hello".to_owned(), "there".to_owned()]
    );

    let error = resolve_thread_send_destination(Some("bogus".to_owned()), None, Vec::new())
        .expect_err("unknown kind");
    assert!(error.to_string().contains("thread"));
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
        "--run-gateway",
    ]);
    match cli.command {
        Some(Commands::Onboard {
            api_account,
            run_gateway,
            ..
        }) => {
            assert_eq!(api_account, "gateway");
            assert!(run_gateway);
        }
        _ => panic!("expected Onboard"),
    }
}

#[test]
fn parse_onboard_rejects_search_api_key_flag() {
    let result = Cli::try_parse_from(["garyx", "onboard", "--search-api-key", "search-key"]);
    assert!(
        result.is_err(),
        "onboard should not accept unsupported search API key setup"
    );
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
fn parse_config_claude_cli() {
    let cli = Cli::parse_from([
        "garyx",
        "config",
        "claude-cli",
        "--mode",
        "native",
        "--clear-path",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Config {
            action:
                ConfigAction::ClaudeCli {
                    mode,
                    path,
                    clear_path,
                    json,
                },
        }) => {
            assert_eq!(mode.as_deref(), Some("native"));
            assert_eq!(path, None);
            assert!(clear_path);
            assert!(json);
        }
        _ => panic!("expected Config::ClaudeCli"),
    }
}

#[test]
fn parse_config_provider_model() {
    let cli = Cli::parse_from([
        "garyx",
        "config",
        "provider-model",
        "claude_code",
        "--model",
        "claude-opus-4-8",
        "--model-reasoning-effort",
        "max",
        "--claude-cli-mode",
        "cctty",
        "--clear-claude-cli-path",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Config {
            action:
                ConfigAction::ProviderModel {
                    provider,
                    model,
                    clear_model,
                    model_reasoning_effort,
                    clear_model_reasoning_effort,
                    claude_cli_mode,
                    clear_claude_cli_mode,
                    claude_cli_path,
                    clear_claude_cli_path,
                    json,
                },
        }) => {
            assert_eq!(provider, "claude_code");
            assert_eq!(model.as_deref(), Some("claude-opus-4-8"));
            assert!(!clear_model);
            assert_eq!(model_reasoning_effort.as_deref(), Some("max"));
            assert!(!clear_model_reasoning_effort);
            assert_eq!(claude_cli_mode.as_deref(), Some("cctty"));
            assert!(!clear_claude_cli_mode);
            assert_eq!(claude_cli_path, None);
            assert!(clear_claude_cli_path);
            assert!(json);
        }
        _ => panic!("expected Config::ProviderModel"),
    }
}

#[test]
fn parse_provider_set() {
    let cli = Cli::parse_from([
        "garyx",
        "provider",
        "set",
        "codex_app_server",
        "--model",
        "gpt-5.5",
        "--reasoning",
        "high",
        "--service-tier",
        "priority",
        "--env",
        "CODEX_HOME=/tmp/test-codex-home",
        "--clear-env",
        "OLD_KEY",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Provider {
            action:
                ProviderAction::Set {
                    provider,
                    model,
                    clear_model,
                    reasoning,
                    clear_reasoning,
                    service_tier,
                    claude_cli_mode,
                    claude_cli_path,
                    env,
                    clear_env,
                    json,
                },
        }) => {
            assert_eq!(provider, "codex_app_server");
            assert_eq!(model.as_deref(), Some("gpt-5.5"));
            assert!(!clear_model);
            assert_eq!(reasoning.as_deref(), Some("high"));
            assert!(!clear_reasoning);
            assert_eq!(service_tier.as_deref(), Some("priority"));
            assert_eq!(claude_cli_mode, None);
            assert_eq!(claude_cli_path, None);
            assert_eq!(env, vec!["CODEX_HOME=/tmp/test-codex-home"]);
            assert_eq!(clear_env, vec!["OLD_KEY"]);
            assert!(json);
        }
        _ => panic!("expected Provider::Set"),
    }
}

#[test]
fn parse_usage_provider_json() {
    let cli = Cli::parse_from(["garyx", "usage", "claude_code", "--json"]);
    match cli.command {
        Some(Commands::Usage { provider, json }) => {
            assert_eq!(provider.as_deref(), Some("claude_code"));
            assert!(json);
        }
        _ => panic!("expected Usage"),
    }
}

#[test]
fn parse_channels_enable() {
    let cli = Cli::parse_from(["garyx", "channels", "enable", "telegram", "main"]);
    match cli.command {
        Some(Commands::Channels {
            action: ChannelsAction::Enable { channel, account },
        }) => {
            assert_eq!(channel, "telegram");
            assert_eq!(account, "main");
        }
        _ => panic!("expected Channels::Enable"),
    }
}

#[test]
fn parse_channels_disable() {
    let cli = Cli::parse_from(["garyx", "channels", "disable", "telegram", "main"]);
    match cli.command {
        Some(Commands::Channels {
            action: ChannelsAction::Disable { channel, account },
        }) => {
            assert_eq!(channel, "telegram");
            assert_eq!(account, "main");
        }
        _ => panic!("expected Channels::Disable"),
    }
}

#[test]
fn parse_channels_enable_rejects_legacy_bool_argument() {
    assert!(
        Cli::try_parse_from(["garyx", "channels", "enable", "telegram", "main", "false"]).is_err()
    );
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
fn parse_thread_history() {
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "history",
        "thread::abc",
        "--limit",
        "5",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::History {
                    thread_id,
                    limit,
                    json,
                },
        }) => {
            assert_eq!(thread_id, "thread::abc");
            assert_eq!(limit, 5);
            assert!(json);
        }
        _ => panic!("expected Thread::History"),
    }
}

#[test]
fn parse_debug_thread_is_not_a_cli_entrypoint() {
    assert!(Cli::try_parse_from(["garyx", "debug", "thread", "thread::abc"]).is_err());
}

#[test]
fn parse_bot_status() {
    let cli = Cli::parse_from(["garyx", "bot", "status", "telegram:main"]);
    match cli.command {
        Some(Commands::Bot {
            action: BotAction::Status { bot_id, json },
        }) => {
            assert_eq!(bot_id, "telegram:main");
            assert!(!json);
        }
        _ => panic!("expected Bot::Status"),
    }
}

#[test]
fn parse_bot_status_keeps_old_bot_entrypoints_removed() {
    for args in [
        vec!["garyx", "debug", "bot", "telegram:main"],
        vec!["garyx", "bot", "current", "telegram:main"],
        vec!["garyx", "bot", "resolve", "telegram:main"],
        vec!["garyx", "bots", "status", "telegram:main"],
        vec!["garyx", "bot", "bind", "--bot", "telegram:main"],
        vec!["garyx", "bot", "unbind", "--bot", "telegram:main"],
    ] {
        let error = match Cli::try_parse_from(args) {
            Ok(_) => panic!("old bot entrypoint should not parse"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}

#[test]
fn parse_endpoint_list() {
    let cli = Cli::parse_from([
        "garyx",
        "bot",
        "endpoint",
        "list",
        "--bot",
        "telegram:main",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Bot {
            action:
                BotAction::Endpoint {
                    action: BotEndpointAction::List { bot, json },
                },
        }) => {
            assert_eq!(bot.as_deref(), Some("telegram:main"));
            assert!(json);
        }
        _ => panic!("expected Bot::Endpoint::List"),
    }
}

#[test]
fn parse_endpoint_bind() {
    let cli = Cli::parse_from([
        "garyx",
        "bot",
        "endpoint",
        "bind",
        "--endpoint",
        "telegram::main::chat42",
        "--thread",
        "thread::abc",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Bot {
            action:
                BotAction::Endpoint {
                    action:
                        BotEndpointAction::Bind {
                            endpoint,
                            thread,
                            json,
                        },
                },
        }) => {
            assert_eq!(endpoint, "telegram::main::chat42");
            assert_eq!(thread, "thread::abc");
            assert!(json);
        }
        _ => panic!("expected Bot::Endpoint::Bind"),
    }
}

#[test]
fn parse_endpoint_detach_alias() {
    let cli = Cli::parse_from([
        "garyx",
        "bot",
        "endpoint",
        "unbind",
        "--endpoint",
        "telegram::main::chat42",
    ]);
    match cli.command {
        Some(Commands::Bot {
            action:
                BotAction::Endpoint {
                    action: BotEndpointAction::Detach { endpoint, json },
                },
        }) => {
            assert_eq!(endpoint, "telegram::main::chat42");
            assert!(!json);
        }
        _ => panic!("expected Bot::Endpoint::Detach"),
    }
}

#[test]
fn parse_automation_create_interval() {
    let cli = Cli::parse_from([
        "garyx",
        "automation",
        "create",
        "--label",
        "Daily triage",
        "--prompt",
        "Summarize repo state",
        "--workspace-dir",
        "/tmp/repo",
        "--every-hours",
        "6",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Automation {
            action:
                AutomationAction::Create {
                    label,
                    prompt,
                    workspace_dir,
                    schedule,
                    json,
                    ..
                },
        }) => {
            assert_eq!(label, "Daily triage");
            assert_eq!(prompt.as_deref(), Some("Summarize repo state"));
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/repo"));
            assert_eq!(schedule.every_hours, Some(6));
            assert!(json);
        }
        _ => panic!("expected Automation::Create"),
    }
}

#[test]
fn parse_automation_has_no_legacy_aliases() {
    for args in [
        ["garyx", "cron", "list"],
        ["garyx", "schedule", "list"],
        ["garyx", "automations", "list"],
    ] {
        let error = match Cli::try_parse_from(args) {
            Ok(_) => panic!("legacy automation alias should not parse"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}

#[test]
fn json_failure_envelope_intent_follows_parsed_matches_not_argv() {
    // A real --json output flag anywhere on the matched command path.
    let matches = <Cli as CommandFactory>::command()
        .try_get_matches_from(["garyx", "agent", "get", "spec-review", "--json"])
        .expect("parse");
    assert!(crate::arg_matches_request_json(&matches));

    // A positional message that happens to be the literal `--json` (after --)
    // must NOT flip the failure format.
    let matches = <Cli as CommandFactory>::command()
        .try_get_matches_from([
            "garyx",
            "thread",
            "send",
            "thread",
            "thread::abc",
            "--",
            "--json",
        ])
        .expect("parse");
    assert!(!crate::arg_matches_request_json(&matches));

    // No --json at all.
    let matches = <Cli as CommandFactory>::command()
        .try_get_matches_from(["garyx", "status"])
        .expect("parse");
    assert!(!crate::arg_matches_request_json(&matches));
}

#[test]
fn parse_thread_create_with_agent() {
    let cli = Cli::parse_from([
        "garyx",
        "thread",
        "create",
        "--workspace-dir",
        "/tmp/garyx",
        "--worktree",
        "--agent-id",
        "product-ship",
        "--json",
    ]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Create {
                    workspace_dir,
                    worktree,
                    agent_id,
                    json,
                    ..
                },
        }) => {
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx"));
            assert!(worktree);
            assert_eq!(agent_id.as_deref(), Some("product-ship"));
            assert!(json);
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
                    workspace_dir,
                    timeout,
                    json,
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("thread"));
            assert_eq!(target.as_deref(), Some("thread::abc"));
            assert_eq!(message, vec!["hello world".to_owned()]);
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
                    workspace_dir,
                    timeout,
                    json,
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("bot"));
            assert_eq!(target.as_deref(), Some("telegram:codex_bot"));
            assert_eq!(message, vec!["hello".to_owned(), "world".to_owned()]);
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx"));
            assert_eq!(timeout, 42);
            assert!(json);
        }
        _ => panic!("expected Thread::Send"),
    }
}

#[test]
fn parse_thread_send_task_target() {
    let cli = Cli::parse_from(["garyx", "thread", "send", "task", "#TASK-1", "status?"]);
    match cli.command {
        Some(Commands::Thread {
            action:
                ThreadAction::Send {
                    kind,
                    target,
                    message,
                    workspace_dir,
                    timeout,
                    json,
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("task"));
            assert_eq!(target.as_deref(), Some("#TASK-1"));
            assert_eq!(message, vec!["status?".to_owned()]);
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
                    ..
                },
        }) => {
            assert_eq!(kind.as_deref(), Some("thread::abc"));
            assert_eq!(target.as_deref(), Some("hello"));
            assert!(message.is_empty());
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
        "--title",
        "Investigate",
        "--body",
        "Check logs",
        "--workspace-dir",
        "/tmp/garyx-task",
        "--worktree",
        "--notify",
        "bot",
        "telegram:owner",
    ]);
    match cli.command {
        Some(Commands::Task {
            action:
                TaskAction::Create {
                    title,
                    body,
                    workspace_dir,
                    worktree,
                    agent,
                    notify,
                },
        }) => {
            assert_eq!(title.as_deref(), Some("Investigate"));
            assert_eq!(body.as_deref(), Some("Check logs"));
            assert_eq!(workspace_dir.as_deref(), Some("/tmp/garyx-task"));
            assert!(worktree);
            assert!(agent.is_none());
            assert_eq!(notify, vec!["bot", "telegram:owner"]);
        }
        _ => panic!("expected Task::Create"),
    }
}

#[test]
fn parse_task_create_agent_executor_options() {
    let cli = Cli::parse_from([
        "garyx", "task", "create", "--title", "Review", "--agent", "claude", "--notify", "none",
    ]);
    match cli.command {
        Some(Commands::Task {
            action: TaskAction::Create { agent, .. },
        }) => {
            assert_eq!(agent.as_deref(), Some("claude"));
        }
        _ => panic!("expected task create"),
    }
}

#[test]
fn parse_task_stop_and_delete() {
    let cli = Cli::parse_from(["garyx", "task", "stop", "#TASK-42", "--json"]);
    match cli.command {
        Some(Commands::Task {
            action: TaskAction::Stop { task_id, json },
        }) => {
            assert_eq!(task_id, "#TASK-42");
            assert!(json);
        }
        _ => panic!("expected Task::Stop"),
    }

    let cli = Cli::parse_from(["garyx", "task", "delete", "#TASK-42"]);
    match cli.command {
        Some(Commands::Task {
            action: TaskAction::Delete { task_id, json },
        }) => {
            assert_eq!(task_id, "#TASK-42");
            assert!(!json);
        }
        _ => panic!("expected Task::Delete"),
    }
}

#[test]
fn parse_agent_availability_commands() {
    let cli = Cli::parse_from(["garyx", "agent", "enable", "codex", "--json"]);
    match cli.command {
        Some(Commands::Agent {
            action: AgentAction::Enable { agent_id, json },
        }) => {
            assert_eq!(agent_id, "codex");
            assert!(json);
        }
        _ => panic!("expected Agent::Enable"),
    }

    let cli = Cli::parse_from(["garyx", "agent", "disable", "reviewer"]);
    match cli.command {
        Some(Commands::Agent {
            action: AgentAction::Disable { agent_id, json },
        }) => {
            assert_eq!(agent_id, "reviewer");
            assert!(!json);
        }
        _ => panic!("expected Agent::Disable"),
    }

    let cli = Cli::parse_from(["garyx", "agent", "default"]);
    match cli.command {
        Some(Commands::Agent {
            action: AgentAction::Default { agent_id, json },
        }) => {
            assert_eq!(agent_id, None);
            assert!(!json);
        }
        _ => panic!("expected Agent::Default query"),
    }

    let cli = Cli::parse_from(["garyx", "agent", "default", "codex", "--json"]);
    match cli.command {
        Some(Commands::Agent {
            action: AgentAction::Default { agent_id, json },
        }) => {
            assert_eq!(agent_id.as_deref(), Some("codex"));
            assert!(json);
        }
        _ => panic!("expected Agent::Default mutation"),
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
        "--model-reasoning-effort",
        "high",
        "--model-service-tier",
        "priority",
        "--default-workspace-dir",
        "/tmp/spec-review",
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
                    model_reasoning_effort,
                    model_service_tier,
                    env: _,
                    unset_env: _,
                    env_clear: _,
                    default_workspace_dir,
                    system_prompt,
                    json,
                },
        }) => {
            assert_eq!(agent_id, "spec-review");
            assert_eq!(display_name, "Spec Review");
            assert_eq!(provider, "codex_app_server");
            assert_eq!(model.as_deref(), Some("gpt-5"));
            assert_eq!(model_reasoning_effort.as_deref(), Some("high"));
            assert_eq!(model_service_tier.as_deref(), Some("priority"));
            assert_eq!(default_workspace_dir.as_deref(), Some("/tmp/spec-review"));
            assert_eq!(system_prompt.as_deref(), Some("Review specs carefully."));
            assert!(json);
        }
        _ => panic!("expected Agent::Create"),
    }
}

#[test]
fn parse_agent_create_without_system_prompt() {
    let cli = Cli::parse_from([
        "garyx",
        "agent",
        "create",
        "--agent-id",
        "plain-claude",
        "--display-name",
        "Plain Claude",
        "--provider",
        "claude_code",
    ]);
    match cli.command {
        Some(Commands::Agent {
            action:
                AgentAction::Create {
                    agent_id,
                    display_name,
                    provider,
                    system_prompt,
                    ..
                },
        }) => {
            assert_eq!(agent_id, "plain-claude");
            assert_eq!(display_name, "Plain Claude");
            assert_eq!(provider, "claude_code");
            assert_eq!(system_prompt, None);
        }
        _ => panic!("expected Agent::Create"),
    }
}

#[test]
fn parse_agent_create_without_args_prints_full_help() {
    let error = match Cli::try_parse_from(["garyx", "agent", "create"]) {
        Ok(_) => panic!("expected help output"),
        Err(error) => error,
    };

    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains("--model <MODEL>"),
        "expected full help with model flag, got:\n{rendered}"
    );
    assert!(
        rendered.contains("--model-service-tier <MODEL_SERVICE_TIER>"),
        "expected full help with service tier flag, got:\n{rendered}"
    );
}

#[test]
fn parse_tool_image() {
    let cli = Cli::parse_from([
        "garyx",
        "tool",
        "image",
        "a precise product render",
        "--output",
        "/tmp/garyx-image.png",
        "--json",
        "--timeout",
        "42",
    ]);
    match cli.command {
        Some(Commands::Tool {
            action:
                ToolAction::Image {
                    prompt,
                    output,
                    json,
                    timeout,
                },
        }) => {
            assert_eq!(prompt, "a precise product render");
            assert_eq!(output, std::path::PathBuf::from("/tmp/garyx-image.png"));
            assert!(json);
            assert_eq!(timeout, 42);
        }
        _ => panic!("expected Tool::Image"),
    }
}

#[test]
fn parse_tools_image_alias() {
    let cli = Cli::parse_from([
        "garyx",
        "tools",
        "image",
        "a precise product render",
        "--output",
        "/tmp/garyx-image",
    ]);
    match cli.command {
        Some(Commands::Tool {
            action:
                ToolAction::Image {
                    prompt,
                    output,
                    json,
                    timeout,
                },
        }) => {
            assert_eq!(prompt, "a precise product render");
            assert_eq!(output, std::path::PathBuf::from("/tmp/garyx-image"));
            assert!(!json);
            assert_eq!(timeout, 600);
        }
        _ => panic!("expected Tool::Image via alias"),
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
                    clear_model,
                    model_reasoning_effort,
                    model_service_tier,
                    env: _,
                    unset_env: _,
                    env_clear: _,
                    default_workspace_dir,
                    system_prompt,
                    json,
                },
        }) => {
            assert_eq!(agent_id, "spec-review");
            assert_eq!(display_name.as_deref(), Some("Spec Review"));
            assert_eq!(provider.as_deref(), Some("codex_app_server"));
            assert_eq!(model, None);
            assert!(!clear_model);
            assert_eq!(model_reasoning_effort, None);
            assert_eq!(model_service_tier, None);
            assert_eq!(default_workspace_dir, None);
            assert_eq!(system_prompt.as_deref(), Some("Review specs carefully."));
            assert!(!json);
        }
        _ => panic!("expected Agent::Update"),
    }
}

#[test]
fn parse_agent_update_clear_model() {
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
        "--clear-model",
        "--system-prompt",
        "Review specs carefully.",
    ]);
    match cli.command {
        Some(Commands::Agent {
            action: AgentAction::Update {
                model, clear_model, ..
            },
        }) => {
            assert_eq!(model, None);
            assert!(clear_model);
        }
        _ => panic!("expected Agent::Update"),
    }
}

#[test]
fn parse_agent_update_env_flags() {
    let cli = Cli::parse_from([
        "garyx",
        "agent",
        "update",
        "--agent-id",
        "spec-review",
        "--display-name",
        "Spec Review",
        "--provider",
        "claude_code",
        "--env",
        "FOO=bar",
        "--env",
        "BAZ=qux",
        "--unset-env",
        "OLD",
        "--env-clear",
        "--system-prompt",
        "Prompt.",
    ]);
    match cli.command {
        Some(Commands::Agent {
            action:
                AgentAction::Update {
                    env,
                    unset_env,
                    env_clear,
                    ..
                },
        }) => {
            assert_eq!(env, vec!["FOO=bar".to_owned(), "BAZ=qux".to_owned()]);
            assert_eq!(unset_env, vec!["OLD".to_owned()]);
            assert!(env_clear);
        }
        _ => panic!("expected Agent::Update"),
    }
}

#[test]
fn parse_agent_update_rejects_model_with_clear_model() {
    let error = match Cli::try_parse_from([
        "garyx",
        "agent",
        "update",
        "--agent-id",
        "spec-review",
        "--display-name",
        "Spec Review",
        "--provider",
        "codex_app_server",
        "--model",
        "gpt-5",
        "--clear-model",
        "--system-prompt",
        "Review specs carefully.",
    ]) {
        Ok(_) => panic!("model and clear-model should conflict"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
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
        "--model-reasoning-effort",
        "xhigh",
        "--default-workspace-dir",
        "/tmp/spec-review",
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
                    clear_model,
                    model_reasoning_effort,
                    model_service_tier,
                    env: _,
                    unset_env: _,
                    env_clear: _,
                    default_workspace_dir,
                    system_prompt,
                    json,
                },
        }) => {
            assert_eq!(agent_id, "spec-review");
            assert_eq!(display_name.as_deref(), Some("Spec Review"));
            assert_eq!(provider.as_deref(), Some("codex_app_server"));
            assert_eq!(model.as_deref(), Some("gpt-5"));
            assert!(!clear_model);
            assert_eq!(model_reasoning_effort.as_deref(), Some("xhigh"));
            assert_eq!(model_service_tier, None);
            assert_eq!(default_workspace_dir.as_deref(), Some("/tmp/spec-review"));
            assert_eq!(system_prompt.as_deref(), Some("Review specs carefully."));
            assert!(json);
        }
        _ => panic!("expected Agent::Upsert"),
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
async fn onboard_writes_api_account_without_search_key_setup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    cmd_onboard(
        &config_path.display().to_string(),
        OnboardCommandOptions {
            force: false,
            json: true,
            api_account: "main".to_owned(),
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
    assert!(config.gateway.search.api_key.is_empty());
}

#[tokio::test]
async fn onboard_updates_existing_config_without_resetting_other_fields() {
    use garyx_models::config::{ApiAccount, GaryxConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    let mut initial = GaryxConfig::default();
    initial.gateway.search.api_key = "keep-search-key".to_owned();
    initial.channels.api.accounts.insert(
        "custom".to_owned(),
        ApiAccount {
            enabled: false,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
        },
    );
    std::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap()).unwrap();

    cmd_onboard(
        &config_path.display().to_string(),
        OnboardCommandOptions {
            force: false,
            json: true,
            api_account: "custom".to_owned(),
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
    assert_eq!(account.agent_id, None);
}

#[tokio::test]
async fn channels_add_non_interactive_discord() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = write_empty_config(&tmp);

    cmd_channels_add(
        &config_path.display().to_string(),
        Some("discord".to_owned()),
        Some("guildbot".to_owned()),
        Some("Discord Bot".to_owned()),
        None,
        None,
        None,
        Some("discord-token".to_owned()),
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
    let discord = loaded.config.channels.resolved_discord_config().unwrap();
    let account = discord.accounts.get("guildbot").unwrap();
    assert!(account.enabled);
    assert_eq!(account.token, "discord-token");
    assert_eq!(account.name.as_deref(), Some("Discord Bot"));
    assert!(account.require_mention);
    assert_eq!(account.agent_id, None);
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
    assert_eq!(account.agent_id, None);
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
    assert_eq!(account.agent_id, None);
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
    assert_eq!(account.agent_id.as_deref(), Some("product-ship"));
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
    assert_eq!(parse_feishu_domain("invalid"), None);
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
        "--workspace-mode",
        "worktree",
        "--agent-id",
        "product-ship",
    ]);
    match cli.command {
        Some(Commands::Channels {
            action:
                ChannelsAction::Add {
                    workspace_mode,
                    agent_id,
                    ..
                },
        }) => {
            assert_eq!(workspace_mode.as_deref(), Some("worktree"));
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
        "--workspace-mode",
        "local",
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
                    workspace_mode,
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
            assert_eq!(workspace_mode.as_deref(), Some("local"));
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
                agent_id: Some("claude".to_owned()),
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
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                owner_target: None,
                require_mention: true,
                topic_session_mode: Default::default(),
                meeting_entities: true,
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
    assert_eq!(statuses.len(), 4);
    assert!(plugin_ids.contains(&"telegram"));
    assert!(plugin_ids.contains(&"discord"));
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
                agent_id: Some("claude".to_owned()),
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
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                owner_target: None,
                require_mention: true,
                topic_session_mode: Default::default(),
                meeting_entities: true,
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
                agent_id: Some("claude".to_owned()),
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
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                owner_target: None,
                require_mention: true,
                topic_session_mode: Default::default(),
                meeting_entities: true,
            }),
        );
    config.channels.api.accounts.insert(
        "api1".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            workspace_mode: None,
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
        system: false,
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

    assert!(
        state
            .integration
            .bridge
            .default_provider_key()
            .await
            .is_none(),
        "RuntimeAssembler must leave provider reconciliation to deferred startup"
    );
    let provider_keys = state.integration.bridge.provider_keys().await;
    assert!(
        !provider_keys.iter().any(|key| key == "claude_code"),
        "default Claude provider should not be registered before HTTP bind"
    );
    assert!(
        !state.provider_runtime_ready(),
        "provider-backed dispatch must stay gated until deferred startup reconciles providers"
    );

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
async fn runtime_assembler_clears_stale_runs_before_returning_for_listener_bind() {
    use crate::runtime_assembler::RuntimeAssembler;
    use garyx_gateway::garyx_db::{GaryxDbService, RecentThreadDraft};
    use garyx_models::config::GaryxConfig;
    use garyx_models::local_paths::garyx_database_path_for_data_dir;

    let tmp = tempfile::TempDir::new().unwrap();
    let session_dir = tmp.path().join("custom-data");
    let database_path = garyx_database_path_for_data_dir(&session_dir);
    let database = GaryxDbService::open(&database_path).expect("seed database");
    database
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::startup-orphan".to_owned(),
            title: "Startup orphan".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 1,
            last_message_preview: "stale".to_owned(),
            recent_run_id: Some("run::startup-orphan".to_owned()),
            active_run_id: Some("run::startup-orphan".to_owned()),
            run_state: "running".to_owned(),
            updated_at: None,
            last_active_at: "2026-07-16T00:00:00Z".to_owned(),
        })
        .unwrap();
    drop(database);

    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(session_dir.display().to_string());
    let assembly = RuntimeAssembler::new(tmp.path().join("garyx.json"), config)
        .assemble()
        .await
        .expect("runtime assembly");
    let row = assembly
        .state
        .ops
        .garyx_db
        .list_recent_threads(10, 0)
        .unwrap()
        .into_iter()
        .find(|row| row.thread_id == "thread::startup-orphan")
        .expect("orphan row");
    assert_eq!(row.active_run_id, None);
    assert_eq!(row.run_state, "completed");
    assert!(session_dir.join("garyx.lock").exists());
}

#[tokio::test]
async fn startup_runtime_assembles_without_rebuilding_thread_indexes_from_canonical_threads() {
    use crate::runtime_assembler::RuntimeAssembler;
    use garyx_models::config::{GaryxConfig, TelegramAccount};
    use serde_json::json;

    let tmp = tempfile::TempDir::new().unwrap();
    let session_dir = tmp.path().join("sessions");
    let legacy_threads_dir = session_dir.join("threads");
    tokio::fs::create_dir_all(&legacy_threads_dir)
        .await
        .unwrap();
    let legacy_record = json!({
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
    });
    let legacy_record_path = legacy_threads_dir.join(
        garyx_router::file_store::thread_storage_file_name("thread::startup-alice", "json"),
    );
    tokio::fs::write(
        legacy_record_path,
        serde_json::to_vec_pretty(&legacy_record).unwrap(),
    )
    .await
    .unwrap();

    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(session_dir.display().to_string());
    let account = TelegramAccount {
        token: "token".to_owned(),
        enabled: true,
        name: None,
        agent_id: Some("claude".to_owned()),
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

    let keys = assembly
        .state
        .threads
        .thread_store
        .list_keys(None)
        .await
        .unwrap();
    assert!(keys.iter().any(|key| key == "thread::startup-alice"));

    let thread = assembly
        .state
        .threads
        .thread_store
        .get("thread::startup-alice")
        .await
        .unwrap()
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

    assert!(
        assembly
            .bridge
            .thread_workspace_bindings_snapshot()
            .await
            .is_empty()
    );
}

// The `plugins update` subcommand and its tests were retired together
// with the host-driven update loop (Architecture C). Plugins now own
// their own upgrade timer + advertised-version source and trigger the
// swap via the `request_self_replace` host RPC; nothing on the CLI
// surface to assert anymore.
