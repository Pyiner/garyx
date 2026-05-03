use clap::{CommandFactory, Parser};
use garyx_models::local_paths::migrate_legacy_homes;
use garyx_router::is_thread_key;
use serde_json::json;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod channel_plugin_host;
mod cli;
mod commands;
mod config_support;
mod plugins_cli;
mod runtime_assembler;
mod service_manager;

#[cfg(test)]
mod main_tests;

use cli::{
    AgentAction, AutoResearchAction, AutomationAction, BotAction, ChannelsAction, Cli,
    CommandAction, Commands, ConfigAction, GatewayAction, LogsAction, MigrateAction, PluginsAction,
    TaskAction, TeamAction, ThreadAction, WikiAction,
};
use commands::{
    cmd_agent_create, cmd_agent_delete, cmd_agent_get, cmd_agent_list, cmd_agent_team_create,
    cmd_agent_team_delete, cmd_agent_team_get, cmd_agent_team_list, cmd_agent_team_update,
    cmd_agent_update, cmd_agent_upsert, cmd_audit, cmd_auto_research_candidates,
    cmd_auto_research_create, cmd_auto_research_feedback, cmd_auto_research_get,
    cmd_auto_research_iterations, cmd_auto_research_list, cmd_auto_research_patch,
    cmd_auto_research_reverify, cmd_auto_research_select, cmd_auto_research_stop,
    cmd_automation_activity, cmd_automation_create, cmd_automation_delete, cmd_automation_get,
    cmd_automation_list, cmd_automation_pause, cmd_automation_resume, cmd_automation_run,
    cmd_automation_update, cmd_bot_status, cmd_channels_add, cmd_channels_enable,
    cmd_channels_list, cmd_channels_login, cmd_channels_remove, cmd_command_delete,
    cmd_command_get, cmd_command_list, cmd_command_set, cmd_config_get, cmd_config_init,
    cmd_config_path, cmd_config_set, cmd_config_show, cmd_config_unset, cmd_config_validate,
    cmd_doctor, cmd_gateway_install, cmd_gateway_reload_config, cmd_gateway_restart,
    cmd_gateway_start, cmd_gateway_stop, cmd_gateway_token, cmd_gateway_uninstall, cmd_logs_clear,
    cmd_logs_path, cmd_logs_tail, cmd_migrate_thread_transcripts, cmd_onboard, cmd_send_message,
    cmd_status, cmd_task_assign, cmd_task_claim, cmd_task_create, cmd_task_get, cmd_task_history,
    cmd_task_list, cmd_task_promote, cmd_task_release, cmd_task_reopen, cmd_task_set_title,
    cmd_task_unassign, cmd_task_update, cmd_thread_create, cmd_thread_get, cmd_thread_history,
    cmd_thread_list, cmd_thread_send, cmd_thread_send_to_bot, cmd_thread_send_to_task, cmd_update,
    cmd_wiki_delete, cmd_wiki_get, cmd_wiki_init, cmd_wiki_list, cmd_wiki_status, run_gateway,
};

struct ThreadSendDestination {
    target: ThreadSendTarget,
    message_parts: Vec<String>,
}

enum ThreadSendTarget {
    Thread(String),
    Task(String),
    Bot(String),
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
            let task_ref = required_send_target("task", target)?;
            Ok(ThreadSendDestination {
                target: ThreadSendTarget::Task(task_ref),
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
) -> Result<Option<ThreadSendDestination>, Box<dyn std::error::Error>> {
    if wake.is_empty() {
        return Ok(None);
    }
    if wake.len() != 2 {
        return Err("wake target must be `thread|task|bot <target>`".into());
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
  garyx gateway restart --wake task <task_ref> --wake-message \"...\"\n\
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
                    let (kind, target) = match &destination.target {
                        ThreadSendTarget::Thread(thread_id) => ("thread", thread_id.as_str()),
                        ThreadSendTarget::Task(task_ref) => ("task", task_ref.as_str()),
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
            search_api_key,
            image_gen_api_key,
            conversation_index_api_key,
            enable_conversation_index,
            disable_conversation_index,
            conversation_index_model,
            conversation_index_base_url,
            run_gateway: onboard_run_gateway,
            json,
        }) => {
            cmd_onboard(
                config_path,
                commands::OnboardCommandOptions {
                    force,
                    json,
                    api_account,
                    search_api_key,
                    image_gen_api_key,
                    conversation_index_api_key,
                    enable_conversation_index,
                    disable_conversation_index,
                    conversation_index_model,
                    conversation_index_base_url,
                    run_gateway: onboard_run_gateway,
                    port_override: cli.port,
                    host_override: cli.host,
                    no_channels: cli.no_channels,
                },
            )
            .await
        }
        Some(Commands::Audit { json }) => cmd_audit(config_path, json).await,
        Some(Commands::Update { version, path }) => cmd_update(version, path).await,
        Some(Commands::Channels { action }) => match action {
            ChannelsAction::List { json } | ChannelsAction::Status { json } => {
                cmd_channels_list(config_path, json)
            }
            ChannelsAction::Enable {
                channel,
                account,
                enabled,
            } => cmd_channels_enable(config_path, &channel, &account, enabled).await,
            ChannelsAction::Remove { channel, account } => {
                cmd_channels_remove(config_path, &channel, &account).await
            }
            ChannelsAction::Add {
                channel,
                account,
                name,
                workspace_dir,
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
        },
        Some(Commands::AutoResearch { action }) => match action {
            AutoResearchAction::Create {
                goal,
                workspace_dir,
                max_iterations,
                time_budget_secs,
                json,
            } => {
                cmd_auto_research_create(
                    config_path,
                    goal,
                    workspace_dir,
                    max_iterations,
                    time_budget_secs,
                    json,
                )
                .await
            }
            AutoResearchAction::Get { run_id, json } => {
                cmd_auto_research_get(config_path, &run_id, json).await
            }
            AutoResearchAction::Iterations { run_id, json } => {
                cmd_auto_research_iterations(config_path, &run_id, json).await
            }
            AutoResearchAction::Stop {
                run_id,
                reason,
                json,
            } => cmd_auto_research_stop(config_path, &run_id, reason, json).await,
            AutoResearchAction::List { json } => cmd_auto_research_list(config_path, json).await,
            AutoResearchAction::Candidates { run_id, json } => {
                cmd_auto_research_candidates(config_path, &run_id, json).await
            }
            AutoResearchAction::Patch {
                run_id,
                max_iterations,
                time_budget_secs,
                json,
            } => {
                cmd_auto_research_patch(
                    config_path,
                    &run_id,
                    max_iterations,
                    time_budget_secs,
                    json,
                )
                .await
            }
            AutoResearchAction::Feedback {
                run_id,
                message,
                json,
            } => cmd_auto_research_feedback(config_path, &run_id, message, json).await,
            AutoResearchAction::Reverify {
                run_id,
                candidate_id,
                guidance,
                json,
            } => {
                cmd_auto_research_reverify(config_path, &run_id, &candidate_id, guidance, json)
                    .await
            }
            AutoResearchAction::Select {
                run_id,
                candidate_id,
                json,
            } => cmd_auto_research_select(config_path, &run_id, &candidate_id, json).await,
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
        },
        Some(Commands::Agent { action }) => match action {
            AgentAction::List {
                include_builtin,
                json,
            } => cmd_agent_list(config_path, include_builtin, json).await,
            AgentAction::Get { agent_id, json } => {
                cmd_agent_get(config_path, &agent_id, json).await
            }
            AgentAction::Create {
                agent_id,
                display_name,
                provider,
                model,
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
                    ThreadSendTarget::Task(task_ref) => {
                        cmd_thread_send_to_task(
                            config_path,
                            task_ref,
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
                agent_id,
                json,
            } => cmd_thread_create(config_path, title, workspace_dir, agent_id, json).await,
        },
        Some(Commands::Task { action }) => match action {
            TaskAction::List {
                status,
                assignee,
                include_done,
                limit,
                offset,
                json,
            } => {
                cmd_task_list(
                    config_path,
                    status.as_deref(),
                    assignee.as_deref(),
                    include_done,
                    limit,
                    offset,
                    json,
                )
                .await
            }
            TaskAction::Get { task_ref, json } => cmd_task_get(config_path, &task_ref, json).await,
            TaskAction::Create {
                title,
                body,
                assignee,
                start,
                workspace_dir,
                json,
            } => {
                cmd_task_create(
                    config_path,
                    title,
                    body,
                    assignee.as_deref(),
                    start,
                    workspace_dir,
                    json,
                )
                .await
            }
            TaskAction::Promote {
                thread_id,
                title,
                assignee,
                json,
            } => cmd_task_promote(config_path, &thread_id, title, assignee.as_deref(), json).await,
            TaskAction::Claim {
                task_ref,
                actor,
                json,
            } => cmd_task_claim(config_path, &task_ref, actor.as_deref(), json).await,
            TaskAction::Release { task_ref, json } => {
                cmd_task_release(config_path, &task_ref, json).await
            }
            TaskAction::Assign {
                task_ref,
                principal,
                json,
            } => cmd_task_assign(config_path, &task_ref, &principal, json).await,
            TaskAction::Unassign { task_ref, json } => {
                cmd_task_unassign(config_path, &task_ref, json).await
            }
            TaskAction::Update {
                task_ref,
                status,
                note,
                force,
                json,
            } => cmd_task_update(config_path, &task_ref, &status, note, force, json).await,
            TaskAction::Reopen { task_ref, json } => {
                cmd_task_reopen(config_path, &task_ref, json).await
            }
            TaskAction::SetTitle {
                task_ref,
                title,
                json,
            } => cmd_task_set_title(config_path, &task_ref, &title, json).await,
            TaskAction::History {
                task_ref,
                limit,
                json,
            } => cmd_task_history(config_path, &task_ref, limit, json).await,
        },
        Some(Commands::Migrate { action }) => match action {
            MigrateAction::ThreadTranscripts {
                data_dir,
                backup_dir,
                rewrite_records,
            } => {
                init_tracing();
                cmd_migrate_thread_transcripts(
                    config_path,
                    data_dir.as_deref(),
                    backup_dir.as_deref(),
                    rewrite_records,
                )
                .await
            }
        },
        Some(Commands::Wiki { action }) => match action {
            WikiAction::Init {
                path,
                topic,
                id,
                agent,
            } => cmd_wiki_init(config_path, path, topic, id, agent).await,
            WikiAction::List { json } => cmd_wiki_list(config_path, json).await,
            WikiAction::Get { wiki_id, json } => cmd_wiki_get(config_path, &wiki_id, json).await,
            WikiAction::Delete { wiki_id } => cmd_wiki_delete(config_path, &wiki_id).await,
            WikiAction::Status { wiki_id, json } => {
                cmd_wiki_status(config_path, &wiki_id, json).await
            }
        },
        Some(Commands::Message { bot, text }) => {
            cmd_send_message(config_path, &bot, &text.join(" ")).await
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
