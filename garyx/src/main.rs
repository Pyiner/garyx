use clap::{CommandFactory, Parser};
use garyx_models::local_paths::migrate_legacy_homes;
use garyx_router::is_thread_key;
use serde_json::json;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod auto_update_common;
mod channel_plugin_host;
mod cli;
mod commands;
mod config_support;
mod gateway_auto_update;
mod plugin_self_replace;
mod plugins_cli;
mod runtime_assembler;
mod service_manager;

const EMBEDDED_CCTTY_ARG: &str = "__cctty";
const EMBEDDED_CCTTY_MCP_PROXY_ARG: &str = "__cctty-mcp-proxy";

#[cfg(test)]
mod main_tests;

use cli::{
    AgentAction, AutoUpdateAction, AutomationAction, AutomationDataTriggerAction,
    AutomationTriggerAction, BotAction, BotEndpointAction, ChannelsAction, Cli, CommandAction,
    Commands, ConfigAction, DbAction, DbFieldAction, DbRecordAction, DbTableAction, DreamAction,
    GatewayAction, LogsAction, PluginsAction, TaskAction, TeamAction, ThreadAction, ToolAction,
    WorkflowAction, WorkflowDefinitionAction,
};
use commands::{
    cmd_agent_create, cmd_agent_delete, cmd_agent_get, cmd_agent_list, cmd_agent_team_create,
    cmd_agent_team_delete, cmd_agent_team_get, cmd_agent_team_list, cmd_agent_team_update,
    cmd_agent_update, cmd_agent_upsert, cmd_automation_activity, cmd_automation_create,
    cmd_automation_data_trigger_create, cmd_automation_data_trigger_delete,
    cmd_automation_data_trigger_list, cmd_automation_data_trigger_set_enabled,
    cmd_automation_delete, cmd_automation_get, cmd_automation_list, cmd_automation_pause,
    cmd_automation_resume, cmd_automation_run, cmd_automation_update, cmd_bot_status,
    cmd_channels_add, cmd_channels_enable, cmd_channels_list, cmd_channels_login,
    cmd_channels_remove, cmd_command_delete, cmd_command_get, cmd_command_list, cmd_command_set,
    cmd_config_claude_cli, cmd_config_get, cmd_config_init, cmd_config_path,
    cmd_config_provider_model, cmd_config_set, cmd_config_show, cmd_config_unset,
    cmd_config_validate, cmd_db_events, cmd_db_field_add, cmd_db_field_drop, cmd_db_record_delete,
    cmd_db_record_get, cmd_db_record_insert, cmd_db_record_update, cmd_db_sql, cmd_db_table_create,
    cmd_db_table_drop, cmd_db_table_list, cmd_db_table_schema, cmd_doctor, cmd_dream_auto,
    cmd_dream_list, cmd_dream_scan, cmd_dream_show, cmd_endpoint_bind, cmd_endpoint_detach,
    cmd_endpoint_list, cmd_gateway_install, cmd_gateway_reload_config, cmd_gateway_restart,
    cmd_gateway_start, cmd_gateway_stop, cmd_gateway_token, cmd_gateway_uninstall, cmd_logs_clear,
    cmd_logs_path, cmd_logs_tail, cmd_onboard, cmd_send_message, cmd_status, cmd_task_assign,
    cmd_task_create, cmd_task_delete, cmd_task_get, cmd_task_history,
    cmd_task_list, cmd_task_reopen, cmd_task_set_title, cmd_task_stop,
    cmd_task_update, cmd_thread_create, cmd_thread_get, cmd_thread_history,
    cmd_thread_list, cmd_thread_send, cmd_thread_send_to_bot, cmd_thread_send_to_task,
    cmd_tool_image, cmd_tool_search, cmd_update, cmd_workflow_cancel, cmd_workflow_definition_get,
    cmd_workflow_definition_list, cmd_workflow_definition_upsert, cmd_workflow_events,
    cmd_workflow_get, cmd_workflow_list, run_gateway,
};

#[derive(Debug)]
struct ThreadSendDestination {
    target: ThreadSendTarget,
    message_parts: Vec<String>,
}

#[derive(Debug)]
enum ThreadSendTarget {
    Thread(String),
    Task(String),
    Bot(String),
}

#[derive(Debug)]
enum GatewayRestartWakeDecision {
    Single(ThreadSendDestination),
    All { message: String },
}

fn resolve_thread_send_destination(
    kind: Option<String>,
    target: Option<String>,
    message: Vec<String>,
    bot_flag: Option<String>,
) -> Result<ThreadSendDestination, Box<dyn std::error::Error>> {
    if let Some(bot) = trim_optional(bot_flag) {
        let mut message_parts = Vec::new();
        if let Some(kind) = kind {
            message_parts.push(kind);
        }
        if let Some(target) = target {
            message_parts.push(target);
        }
        message_parts.extend(message);
        validate_bot_selector(&bot)?;
        return Ok(ThreadSendDestination {
            target: ThreadSendTarget::Bot(bot),
            message_parts,
        });
    }

    let Some(kind) = trim_optional(kind) else {
        return Err(
            "destination is required: use `garyx thread send thread|task|bot <target> [message]...`"
                .into(),
        );
    };

    match kind.to_ascii_lowercase().as_str() {
        "thread" | "threads" => {
            let thread_id = required_send_target("thread", target)?;
            validate_thread_id(&thread_id)?;
            Ok(ThreadSendDestination {
                target: ThreadSendTarget::Thread(thread_id),
                message_parts: message,
            })
        }
        "task" | "tasks" => {
            let task_id = required_send_target("task", target)?;
            Ok(ThreadSendDestination {
                target: ThreadSendTarget::Task(task_id),
                message_parts: message,
            })
        }
        "bot" | "bots" => {
            let bot = required_send_target("bot", target)?;
            validate_bot_selector(&bot)?;
            Ok(ThreadSendDestination {
                target: ThreadSendTarget::Bot(bot),
                message_parts: message,
            })
        }
        _ if is_thread_key(&kind) => {
            let mut message_parts = Vec::new();
            if let Some(target) = target {
                message_parts.push(target);
            }
            message_parts.extend(message);
            Ok(ThreadSendDestination {
                target: ThreadSendTarget::Thread(kind),
                message_parts,
            })
        }
        _ => Err(
            "destination kind must be `thread`, `task`, or `bot` (or a legacy canonical thread id)"
                .into(),
        ),
    }
}

fn resolve_gateway_restart_wake_destination(
    wake: Vec<String>,
    wake_message: Option<String>,
) -> Result<Option<GatewayRestartWakeDecision>, Box<dyn std::error::Error>> {
    if wake.is_empty() {
        return Ok(None);
    }
    if wake.len() == 1 {
        if wake[0].trim() != "all" {
            return Err("single-token wake target must be `all`".into());
        }
        let message = trim_optional(wake_message).unwrap_or_else(|| "continue".to_owned());
        return Ok(Some(GatewayRestartWakeDecision::All { message }));
    }
    if wake.len() != 2 {
        return Err("wake target must be `all` or `thread|task|bot <target>`".into());
    }
    let message = trim_optional(wake_message).ok_or_else(|| {
        "wake message is required: use `--wake-message \"...\"` with `--wake`".to_owned()
    })?;
    resolve_thread_send_destination(
        Some(wake[0].clone()),
        Some(wake[1].clone()),
        vec![message],
        None,
    )
    .map(GatewayRestartWakeDecision::Single)
    .map(Some)
}

fn validate_gateway_restart_wake_decision(has_wake: bool, no_wake: bool) -> Result<(), String> {
    if has_wake || no_wake {
        return Ok(());
    }
    Err("gateway restart requires an explicit wake decision.\n\
Agent safety: when you restart the gateway from an agent thread, queue a wake so the new gateway resumes the same thread after restart. Do not run a bare restart from agent work.\n\
Use one of:\n\
  garyx gateway restart --wake thread <thread_id> --wake-message \"...\"\n\
  garyx gateway restart --wake task <task_id> --wake-message \"...\"\n\
  garyx gateway restart --wake bot <channel:account_id> --wake-message \"...\"\n\
If you intentionally want no continuation, run:\n\
  garyx gateway restart --no-wake"
        .to_owned())
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn required_send_target(
    kind: &str,
    target: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    trim_optional(target)
        .ok_or_else(|| format!("target is required for `garyx thread send {kind}`").into())
}

fn validate_thread_id(thread_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    if is_thread_key(thread_id) {
        Ok(())
    } else {
        Err("thread target must be a canonical thread id like `thread::...`".into())
    }
}

fn validate_bot_selector(bot: &str) -> Result<(), Box<dyn std::error::Error>> {
    match bot.split_once(':') {
        Some((channel, account_id))
            if !channel.trim().is_empty() && !account_id.trim().is_empty() =>
        {
            Ok(())
        }
        _ => Err("bot target must be `channel:account_id`, e.g. `telegram:main`".into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if run_embedded_cctty_if_requested().await? {
        return Ok(());
    }

    if let Err(error) = migrate_legacy_homes() {
        eprintln!("failed to migrate legacy state into ~/.garyx: {error}");
    }

    let cli = Cli::parse();

    // Initialize tracing (only for gateway, keep quiet for utility commands).
    let init_tracing = || {
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(false);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .init();
    };

    let config_path = &cli.config;

    match cli.command {
        Some(Commands::Gateway { action }) => match action {
            GatewayAction::Run {
                port,
                host,
                no_channels,
            } => {
                init_tracing();
                run_gateway(config_path, port, host, no_channels).await
            }
            GatewayAction::Install => cmd_gateway_install(config_path).await,
            GatewayAction::Uninstall => cmd_gateway_uninstall().await,
            GatewayAction::Start => cmd_gateway_start(config_path).await,
            GatewayAction::Restart {
                wake,
                wake_message,
                no_wake,
                wake_json,
            } => {
                let wake_destination =
                    resolve_gateway_restart_wake_destination(wake, wake_message)?;
                if let Err(message) =
                    validate_gateway_restart_wake_decision(wake_destination.is_some(), no_wake)
                {
                    eprintln!("{message}");
                    std::process::exit(2);
                }
                if let Some(destination) = wake_destination.as_ref() {
                    match destination {
                        GatewayRestartWakeDecision::Single(destination) => {
                            let (kind, target) = match &destination.target {
                                ThreadSendTarget::Thread(thread_id) => {
                                    ("thread", thread_id.as_str())
                                }
                                ThreadSendTarget::Task(task_id) => ("task", task_id.as_str()),
                                ThreadSendTarget::Bot(bot) => ("bot", bot.as_str()),
                            };
                            let message = destination.message_parts.join(" ");
                            let path = garyx_gateway::restart_wake::queue_pending_restart_wake(
                                kind, target, &message,
                            )?;
                            if wake_json {
                                println!(
                                    "{}",
                                    serde_json::to_string(&json!({
                                        "type": "restart_wake_queued",
                                        "kind": kind,
                                        "target": target,
                                        "path": path.display().to_string(),
                                    }))?
                                );
                            } else {
                                println!("Queued restart wake: {}", path.display());
                            }
                        }
                        GatewayRestartWakeDecision::All { message } => {
                            let report =
                                garyx_gateway::restart_wake::queue_pending_restart_wake_all(
                                    message,
                                )?;
                            if wake_json {
                                println!(
                                    "{}",
                                    serde_json::to_string(&json!({
                                        "type": "restart_wake_queued",
                                        "kind": "all",
                                        "target_count": report.targets.len(),
                                        "truncated_count": report.truncated_count,
                                        "path": report.path.display().to_string(),
                                    }))?
                                );
                            } else {
                                println!(
                                    "Queued restart wake-all: {} target(s) at {}",
                                    report.targets.len(),
                                    report.path.display()
                                );
                                if report.truncated_count > 0 {
                                    println!(
                                        "  warning: {} additional running thread(s) omitted by wake-all cap",
                                        report.truncated_count
                                    );
                                }
                            }
                        }
                    }
                }
                cmd_gateway_restart(config_path).await?;
                Ok(())
            }
            GatewayAction::Stop => cmd_gateway_stop().await,
            GatewayAction::ReloadConfig => cmd_gateway_reload_config(config_path).await,
            GatewayAction::Token { rotate, json } => {
                cmd_gateway_token(config_path, rotate, json).await
            }
        },
        Some(Commands::Config { action }) => match action {
            ConfigAction::Path => cmd_config_path(config_path),
            ConfigAction::Get { path } => cmd_config_get(config_path, &path),
            ConfigAction::Set { path, value } => cmd_config_set(config_path, &path, &value),
            ConfigAction::Unset { path } => cmd_config_unset(config_path, &path),
            ConfigAction::ClaudeCli {
                mode,
                path,
                clear_path,
                json,
            } => cmd_config_claude_cli(config_path, mode, path, clear_path, json).await,
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
            } => {
                cmd_config_provider_model(
                    config_path,
                    &provider,
                    model,
                    clear_model,
                    model_reasoning_effort,
                    clear_model_reasoning_effort,
                    claude_cli_mode,
                    clear_claude_cli_mode,
                    claude_cli_path,
                    clear_claude_cli_path,
                    json,
                )
                .await
            }
            ConfigAction::Init { force } => cmd_config_init(config_path, force),
            ConfigAction::Show => cmd_config_show(config_path),
            ConfigAction::Validate => cmd_config_validate(config_path),
        },
        Some(Commands::CommandList { action }) => match action {
            CommandAction::List {
                json,
                surface,
                channel,
                account_id,
                include_hidden,
            } => cmd_command_list(
                config_path,
                json,
                surface,
                channel,
                account_id,
                include_hidden,
            ),
            CommandAction::Get { name, json } => cmd_command_get(config_path, &name, json),
            CommandAction::Set {
                name,
                prompt,
                description,
                json,
            } => cmd_command_set(config_path, name, prompt, description, json).await,
            CommandAction::Delete { name, json } => {
                cmd_command_delete(config_path, &name, json).await
            }
        },
        Some(Commands::Status { json }) => cmd_status(config_path, json).await,
        Some(Commands::Doctor { json }) => cmd_doctor(config_path, json).await,
        Some(Commands::Onboard {
            force,
            api_account,
            run_gateway: onboard_run_gateway,
            json,
        }) => {
            cmd_onboard(
                config_path,
                commands::OnboardCommandOptions {
                    force,
                    json,
                    api_account,
                    run_gateway: onboard_run_gateway,
                    port_override: cli.port,
                    host_override: cli.host,
                    no_channels: cli.no_channels,
                },
            )
            .await
        }
        Some(Commands::Update { version, path }) => cmd_update(version, path).await,
        Some(Commands::AutoUpdate { action }) => match action {
            AutoUpdateAction::Status { json } => {
                commands::cmd_auto_update_status(config_path, json).await
            }
            AutoUpdateAction::Disable { gateway, plugin } => {
                commands::cmd_auto_update_disable(config_path, gateway, plugin).await
            }
            AutoUpdateAction::Enable { gateway, plugin } => {
                commands::cmd_auto_update_enable(config_path, gateway, plugin).await
            }
        },
        Some(Commands::Channels { action }) => match action {
            ChannelsAction::List { json } | ChannelsAction::Status { json } => {
                cmd_channels_list(config_path, json)
            }
            ChannelsAction::Enable { channel, account } => {
                cmd_channels_enable(config_path, &channel, &account, true).await
            }
            ChannelsAction::Disable { channel, account } => {
                cmd_channels_enable(config_path, &channel, &account, false).await
            }
            ChannelsAction::Remove { channel, account } => {
                cmd_channels_remove(config_path, &channel, &account).await
            }
            ChannelsAction::Add {
                channel,
                account,
                name,
                workspace_dir,
                workspace_mode,
                agent_id,
                token,
                uin,
                base_url,
                app_id,
                app_secret,
                domain,
                auto_register,
            } => {
                cmd_channels_add(
                    config_path,
                    channel,
                    account,
                    name,
                    workspace_dir,
                    workspace_mode,
                    agent_id,
                    token,
                    uin,
                    base_url,
                    app_id,
                    app_secret,
                    domain,
                    auto_register,
                )
                .await
            }
            ChannelsAction::Login {
                channel,
                account,
                reauthorize,
                forget_previous,
                name,
                workspace_dir,
                workspace_mode,
                agent_id,
                uin,
                base_url,
                domain,
                timeout_seconds,
                json,
            } => {
                cmd_channels_login(
                    config_path,
                    &channel,
                    account,
                    reauthorize,
                    forget_previous,
                    name,
                    workspace_dir,
                    workspace_mode,
                    agent_id,
                    uin,
                    base_url,
                    domain,
                    timeout_seconds,
                    json,
                )
                .await
            }
        },
        Some(Commands::Plugins { action }) => match action {
            PluginsAction::Install {
                path,
                target,
                force,
            } => plugins_cli::install(&path, target, force)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string().into()),
            PluginsAction::List { target, json } => {
                plugins_cli::list(target, json).map_err(|e| e.to_string().into())
            }
            PluginsAction::Uninstall { id, target } => {
                plugins_cli::uninstall(&id, target).map_err(|e| e.to_string().into())
            }
        },
        Some(Commands::Logs { action }) => match action {
            LogsAction::Path { path } => {
                cmd_logs_path(path);
                Ok(())
            }
            LogsAction::Tail {
                path,
                lines,
                pattern,
                follow,
            } => cmd_logs_tail(path, lines, pattern, follow).await,
            LogsAction::Clear { path } => cmd_logs_clear(path),
        },
        Some(Commands::Bot { action }) => match action {
            BotAction::Status { bot_id, json } => cmd_bot_status(config_path, &bot_id, json).await,
            BotAction::Endpoint { action } => match action {
                BotEndpointAction::List { bot, json } => {
                    cmd_endpoint_list(config_path, bot.as_deref(), json).await
                }
                BotEndpointAction::Bind {
                    endpoint,
                    thread,
                    json,
                } => cmd_endpoint_bind(config_path, &endpoint, &thread, json).await,
                BotEndpointAction::Detach { endpoint, json } => {
                    cmd_endpoint_detach(config_path, &endpoint, json).await
                }
            },
        },
        Some(Commands::Automation { action }) => match action {
            AutomationAction::List { json } => cmd_automation_list(config_path, json).await,
            AutomationAction::Get {
                automation_id,
                json,
            } => cmd_automation_get(config_path, &automation_id, json).await,
            AutomationAction::Create {
                label,
                prompt,
                agent_id,
                workspace_dir,
                thread_id,
                schedule,
                disabled,
                json,
            } => {
                cmd_automation_create(
                    config_path,
                    label,
                    prompt,
                    agent_id,
                    workspace_dir,
                    thread_id,
                    schedule,
                    disabled,
                    json,
                )
                .await
            }
            AutomationAction::Update {
                automation_id,
                label,
                prompt,
                agent_id,
                workspace_dir,
                thread_id,
                schedule,
                enable,
                disable,
                json,
            } => {
                cmd_automation_update(
                    config_path,
                    &automation_id,
                    label,
                    prompt,
                    agent_id,
                    workspace_dir,
                    thread_id,
                    schedule,
                    enable,
                    disable,
                    json,
                )
                .await
            }
            AutomationAction::Delete {
                automation_id,
                json,
            } => cmd_automation_delete(config_path, &automation_id, json).await,
            AutomationAction::Run {
                automation_id,
                json,
            } => cmd_automation_run(config_path, &automation_id, json).await,
            AutomationAction::Pause {
                automation_id,
                json,
            } => cmd_automation_pause(config_path, &automation_id, json).await,
            AutomationAction::Resume {
                automation_id,
                json,
            } => cmd_automation_resume(config_path, &automation_id, json).await,
            AutomationAction::Activity {
                automation_id,
                limit,
                offset,
                json,
            } => cmd_automation_activity(config_path, &automation_id, limit, offset, json).await,
            AutomationAction::Trigger { action } => match action {
                AutomationTriggerAction::Data { action } => match action {
                    AutomationDataTriggerAction::List {
                        table,
                        event_type,
                        json,
                    } => {
                        cmd_automation_data_trigger_list(config_path, table, event_type, json).await
                    }
                    AutomationDataTriggerAction::Create {
                        table,
                        event_type,
                        label,
                        title,
                        body,
                        agent_id,
                        workspace_dir,
                        disabled,
                        json,
                    } => {
                        cmd_automation_data_trigger_create(
                            config_path,
                            &table,
                            &event_type,
                            &label,
                            &title,
                            &body,
                            agent_id,
                            workspace_dir,
                            disabled,
                            json,
                        )
                        .await
                    }
                    AutomationDataTriggerAction::Enable { trigger_id, json } => {
                        cmd_automation_data_trigger_set_enabled(
                            config_path,
                            &trigger_id,
                            true,
                            json,
                        )
                        .await
                    }
                    AutomationDataTriggerAction::Disable { trigger_id, json } => {
                        cmd_automation_data_trigger_set_enabled(
                            config_path,
                            &trigger_id,
                            false,
                            json,
                        )
                        .await
                    }
                    AutomationDataTriggerAction::Delete { trigger_id, json } => {
                        cmd_automation_data_trigger_delete(config_path, &trigger_id, json).await
                    }
                },
            },
        },
        Some(Commands::Workflow { action }) => match action {
            WorkflowAction::Definition { action } => match action {
                WorkflowDefinitionAction::List {
                    limit,
                    offset,
                    json,
                } => cmd_workflow_definition_list(config_path, limit, offset, json).await,
                WorkflowDefinitionAction::Get { workflow_id, json } => {
                    cmd_workflow_definition_get(config_path, &workflow_id, json).await
                }
                WorkflowDefinitionAction::Upsert { file, json } => {
                    cmd_workflow_definition_upsert(config_path, &file, json).await
                }
            },
            WorkflowAction::List {
                parent_thread_id,
                json,
            } => cmd_workflow_list(config_path, parent_thread_id, json).await,
            WorkflowAction::Get {
                workflow_run_id,
                json,
            } => cmd_workflow_get(config_path, &workflow_run_id, json).await,
            WorkflowAction::Events {
                workflow_run_id,
                after,
                json,
            } => cmd_workflow_events(config_path, &workflow_run_id, after, json).await,
            WorkflowAction::Cancel {
                workflow_run_id,
                json,
            } => cmd_workflow_cancel(config_path, &workflow_run_id, json).await,
        },
        Some(Commands::Db { action }) => match action {
            DbAction::Table { action } => match action {
                DbTableAction::List { json } => cmd_db_table_list(config_path, json).await,
                DbTableAction::Create {
                    table,
                    display_name,
                    fields,
                    json,
                } => cmd_db_table_create(config_path, &table, display_name, fields, json).await,
                DbTableAction::Schema { table, json } => {
                    cmd_db_table_schema(config_path, &table, json).await
                }
                DbTableAction::Drop { table, json } => {
                    cmd_db_table_drop(config_path, &table, json).await
                }
            },
            DbAction::Field { action } => match action {
                DbFieldAction::Add {
                    table,
                    field,
                    field_type,
                    not_null,
                    unique,
                    index,
                    display_name,
                    default_value,
                    json,
                } => {
                    cmd_db_field_add(
                        config_path,
                        &table,
                        &field,
                        &field_type,
                        not_null,
                        unique,
                        index,
                        display_name,
                        default_value,
                        json,
                    )
                    .await
                }
                DbFieldAction::Drop { table, field, json } => {
                    cmd_db_field_drop(config_path, &table, &field, json).await
                }
            },
            DbAction::Record { action } => match action {
                DbRecordAction::Insert { table, data, json } => {
                    cmd_db_record_insert(config_path, &table, &data, json).await
                }
                DbRecordAction::Get { table, id, json } => {
                    cmd_db_record_get(config_path, &table, &id, json).await
                }
                DbRecordAction::Update {
                    table,
                    id,
                    data,
                    json,
                } => cmd_db_record_update(config_path, &table, &id, &data, json).await,
                DbRecordAction::Delete { table, id, json } => {
                    cmd_db_record_delete(config_path, &table, &id, json).await
                }
            },
            DbAction::Sql { sql, limit, json } => cmd_db_sql(config_path, sql, limit, json).await,
            DbAction::Events {
                table,
                event_type,
                limit,
                offset,
                json,
            } => cmd_db_events(config_path, table, event_type, limit, offset, json).await,
        },
        Some(Commands::Agent { action }) => match action {
            AgentAction::List { json } => cmd_agent_list(config_path, json).await,
            AgentAction::Get { agent_id, json } => {
                cmd_agent_get(config_path, &agent_id, json).await
            }
            AgentAction::Create {
                agent_id,
                display_name,
                provider,
                model,
                model_reasoning_effort,
                model_service_tier,
                provider_auth_source,
                provider_api_key,
                default_workspace_dir,
                system_prompt,
                json,
            } => {
                cmd_agent_create(
                    config_path,
                    agent_id,
                    display_name,
                    provider,
                    model,
                    model_reasoning_effort,
                    model_service_tier,
                    provider_auth_source,
                    provider_api_key,
                    default_workspace_dir,
                    system_prompt,
                    json,
                )
                .await
            }
            AgentAction::Update {
                agent_id,
                display_name,
                provider,
                model,
                clear_model,
                model_reasoning_effort,
                model_service_tier,
                provider_auth_source,
                provider_api_key,
                default_workspace_dir,
                system_prompt,
                json,
            } => {
                cmd_agent_update(
                    config_path,
                    agent_id,
                    display_name,
                    provider,
                    model,
                    clear_model,
                    model_reasoning_effort,
                    model_service_tier,
                    provider_auth_source,
                    provider_api_key,
                    default_workspace_dir,
                    system_prompt,
                    json,
                )
                .await
            }
            AgentAction::Upsert {
                agent_id,
                display_name,
                provider,
                model,
                clear_model,
                model_reasoning_effort,
                model_service_tier,
                provider_auth_source,
                provider_api_key,
                default_workspace_dir,
                system_prompt,
                json,
            } => {
                cmd_agent_upsert(
                    config_path,
                    agent_id,
                    display_name,
                    provider,
                    model,
                    clear_model,
                    model_reasoning_effort,
                    model_service_tier,
                    provider_auth_source,
                    provider_api_key,
                    default_workspace_dir,
                    system_prompt,
                    json,
                )
                .await
            }
            AgentAction::Delete { agent_id, json } => {
                cmd_agent_delete(config_path, &agent_id, json).await
            }
        },
        Some(Commands::Team { action }) => match action {
            TeamAction::List { json } => cmd_agent_team_list(config_path, json).await,
            TeamAction::Get { team_id, json } => {
                cmd_agent_team_get(config_path, &team_id, json).await
            }
            TeamAction::Create {
                team_id,
                display_name,
                leader_agent_id,
                member_agent_ids,
                workflow_text,
                json,
            } => {
                cmd_agent_team_create(
                    config_path,
                    team_id,
                    display_name,
                    leader_agent_id,
                    member_agent_ids,
                    workflow_text,
                    json,
                )
                .await
            }
            TeamAction::Update {
                team_id,
                new_team_id,
                display_name,
                leader_agent_id,
                member_agent_ids,
                workflow_text,
                json,
            } => {
                cmd_agent_team_update(
                    config_path,
                    team_id,
                    new_team_id,
                    display_name,
                    leader_agent_id,
                    member_agent_ids,
                    workflow_text,
                    json,
                )
                .await
            }
            TeamAction::Delete { team_id, json } => {
                cmd_agent_team_delete(config_path, &team_id, json).await
            }
        },
        Some(Commands::Tool { action }) => match action {
            ToolAction::Image {
                prompt,
                output,
                json,
                timeout,
            } => cmd_tool_image(config_path, prompt, output, timeout, json).await,
            ToolAction::Search {
                query,
                json,
                timeout,
            } => cmd_tool_search(config_path, query, json, timeout).await,
        },
        Some(Commands::Thread { action }) => match action {
            ThreadAction::List {
                include_hidden,
                limit,
                offset,
                json,
            } => cmd_thread_list(config_path, include_hidden, limit, offset, json).await,
            ThreadAction::Get { thread_id, json } => {
                cmd_thread_get(config_path, &thread_id, json).await
            }
            ThreadAction::History {
                thread_id,
                limit,
                json,
            } => cmd_thread_history(config_path, &thread_id, limit, json).await,
            ThreadAction::Send {
                kind,
                target,
                message,
                bot,
                workspace_dir,
                timeout,
                json,
            } => {
                let destination = resolve_thread_send_destination(kind, target, message, bot)?;
                let message_parts = destination.message_parts;
                let text = if message_parts.is_empty() {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf.trim().to_owned()
                } else {
                    message_parts.join(" ")
                };
                match destination.target {
                    ThreadSendTarget::Thread(thread_id) => {
                        cmd_thread_send(config_path, thread_id, text, workspace_dir, timeout, json)
                            .await
                    }
                    ThreadSendTarget::Task(task_id) => {
                        cmd_thread_send_to_task(
                            config_path,
                            task_id,
                            text,
                            workspace_dir,
                            timeout,
                            json,
                        )
                        .await
                    }
                    ThreadSendTarget::Bot(bot) => {
                        cmd_thread_send_to_bot(config_path, bot, text, workspace_dir, timeout, json)
                            .await
                    }
                }
            }
            ThreadAction::Create {
                title,
                workspace_dir,
                worktree,
                agent_id,
                json,
            } => {
                cmd_thread_create(config_path, title, workspace_dir, agent_id, worktree, json).await
            }
        },
        Some(Commands::Dream { action }) => match action {
            DreamAction::List {
                from,
                to,
                since_hours,
                limit,
                json,
            } => {
                cmd_dream_list(
                    config_path,
                    from.as_deref(),
                    to.as_deref(),
                    since_hours,
                    limit,
                    json,
                )
                .await
            }
            DreamAction::Scan {
                from,
                to,
                since_hours,
                mode,
                limit,
                json,
            } => {
                cmd_dream_scan(
                    config_path,
                    from.as_deref(),
                    to.as_deref(),
                    since_hours,
                    &mode,
                    limit,
                    json,
                )
                .await
            }
            DreamAction::Show { dream_id, json } => {
                cmd_dream_show(config_path, &dream_id, json).await
            }
            DreamAction::Auto { state, json } => cmd_dream_auto(config_path, &state, json).await,
        },
        Some(Commands::Task { action }) => match action {
            TaskAction::List {
                status,
                assignee,
                source_thread,
                source_task,
                source_bot,
                limit,
                offset,
                json,
            } => {
                cmd_task_list(
                    config_path,
                    status.as_deref(),
                    assignee.as_deref(),
                    source_thread.as_deref(),
                    source_task.as_deref(),
                    source_bot.as_deref(),
                    limit,
                    offset,
                    json,
                )
                .await
            }
            TaskAction::Get { task_id, json } => cmd_task_get(config_path, &task_id, json).await,
            TaskAction::Create {
                title,
                body,
                workspace_dir,
                worktree,
                agent,
                team,
                workflow,
                input,
                notify,
                json,
            } => {
                cmd_task_create(
                    config_path,
                    title,
                    body,
                    workspace_dir,
                    worktree,
                    agent,
                    team,
                    workflow,
                    input,
                    notify,
                    json,
                )
                .await
            }
            TaskAction::Stop { task_id, json } => cmd_task_stop(config_path, &task_id, json).await,
            TaskAction::Delete { task_id, json } => {
                cmd_task_delete(config_path, &task_id, json).await
            }
            TaskAction::Assign {
                task_id,
                principal,
                json,
            } => cmd_task_assign(config_path, &task_id, &principal, json).await,
            TaskAction::Update {
                task_id,
                status,
                note,
                force,
                json,
            } => cmd_task_update(config_path, &task_id, &status, note, force, json).await,
            TaskAction::Reopen { task_id, json } => {
                cmd_task_reopen(config_path, &task_id, json).await
            }
            TaskAction::SetTitle {
                task_id,
                title,
                json,
            } => cmd_task_set_title(config_path, &task_id, &title, json).await,
            TaskAction::History {
                task_id,
                limit,
                json,
            } => cmd_task_history(config_path, &task_id, limit, json).await,
        },
        Some(Commands::Message {
            bot,
            image,
            file,
            text,
        }) => {
            cmd_send_message(
                config_path,
                bot.as_deref(),
                image.as_deref(),
                file.as_deref(),
                &text.join(" "),
            )
            .await
        }

        None => {
            eprintln!(
                "No subcommand provided. Try `garyx status`, `garyx setup`, or `garyx gateway run`."
            );
            let mut command = Cli::command();
            command.print_help()?;
            eprintln!();
            std::process::exit(1);
        }
    }
}

async fn run_embedded_cctty_if_requested() -> Result<bool, Box<dyn std::error::Error>> {
    let mut argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some(EMBEDDED_CCTTY_ARG) => {
            argv.remove(1);
        }
        Some(EMBEDDED_CCTTY_MCP_PROXY_ARG) => {}
        _ => return Ok(false),
    }

    let exit_code = match cctty::run_cli(argv).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("cctty: {error}");
            error.exit_code()
        }
    };
    std::process::exit(exit_code);
}
