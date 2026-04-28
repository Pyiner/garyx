use clap::{CommandFactory, Parser};
use garyx_models::local_paths::migrate_legacy_homes;
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
    AgentAction, AutoResearchAction, ChannelsAction, Cli, CommandAction, Commands, ConfigAction,
    DebugAction, GatewayAction, LogsAction, MigrateAction, PluginsAction, TeamAction, ThreadAction,
    WikiAction,
};
use commands::{
    cmd_agent_create, cmd_agent_delete, cmd_agent_get, cmd_agent_list, cmd_agent_team_create,
    cmd_agent_team_delete, cmd_agent_team_get, cmd_agent_team_list, cmd_agent_team_update,
    cmd_agent_update, cmd_agent_upsert, cmd_audit, cmd_auto_research_candidates,
    cmd_auto_research_create, cmd_auto_research_feedback, cmd_auto_research_get,
    cmd_auto_research_iterations, cmd_auto_research_list, cmd_auto_research_patch,
    cmd_auto_research_reverify, cmd_auto_research_select, cmd_auto_research_stop, cmd_channels_add,
    cmd_channels_enable, cmd_channels_list, cmd_channels_login, cmd_channels_remove,
    cmd_command_delete, cmd_command_get, cmd_command_list, cmd_command_set, cmd_config_get,
    cmd_config_init, cmd_config_path, cmd_config_set, cmd_config_show, cmd_config_unset,
    cmd_config_validate, cmd_debug_bot, cmd_debug_thread, cmd_doctor, cmd_gateway_install,
    cmd_gateway_reload_config, cmd_gateway_restart, cmd_gateway_start, cmd_gateway_stop,
    cmd_gateway_token, cmd_gateway_uninstall, cmd_logs_clear, cmd_logs_path, cmd_logs_tail,
    cmd_migrate_thread_transcripts, cmd_onboard, cmd_send_message, cmd_status, cmd_thread_create,
    cmd_thread_get, cmd_thread_list, cmd_thread_send, cmd_update, cmd_wiki_delete, cmd_wiki_get,
    cmd_wiki_init, cmd_wiki_list, cmd_wiki_status, run_gateway,
};

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
            GatewayAction::Restart => cmd_gateway_restart(config_path).await,
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
                agent_id,
                base_url,
                domain,
                timeout_seconds,
            } => {
                cmd_channels_login(
                    config_path,
                    &channel,
                    account,
                    agent_id,
                    base_url,
                    domain,
                    timeout_seconds,
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
        Some(Commands::Debug { action }) => match action {
            DebugAction::Thread {
                thread_id,
                limit,
                json,
            } => cmd_debug_thread(config_path, &thread_id, limit, json).await,
            DebugAction::Bot {
                bot_id,
                limit,
                json,
            } => cmd_debug_bot(config_path, &bot_id, limit, json).await,
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
                system_prompt,
                json,
            } => {
                cmd_agent_create(
                    config_path,
                    agent_id,
                    display_name,
                    provider,
                    model,
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
                system_prompt,
                json,
            } => {
                cmd_agent_update(
                    config_path,
                    agent_id,
                    display_name,
                    provider,
                    model,
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
                system_prompt,
                json,
            } => {
                cmd_agent_upsert(
                    config_path,
                    agent_id,
                    display_name,
                    provider,
                    model,
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
            ThreadAction::Send {
                thread_id,
                message,
                workspace_dir,
                timeout,
                json,
            } => {
                let text = match message {
                    Some(m) => m,
                    None => {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin().read_to_string(&mut buf)?;
                        buf.trim().to_owned()
                    }
                };
                cmd_thread_send(config_path, thread_id, text, workspace_dir, timeout, json).await
            }
            ThreadAction::Create {
                title,
                workspace_dir,
                agent_id,
                json,
            } => cmd_thread_create(config_path, title, workspace_dir, agent_id, json).await,
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
