use super::*;

#[derive(Debug, Serialize)]
struct OnboardSummary {
    ok: bool,
    config_path: String,
    created_config: bool,
    api_account: String,
    api_account_created: bool,
    gateway_run_requested: bool,
    /// `channel.account` identifiers bound during this onboarding session.
    channels_bound: Vec<String>,
    /// Total account count across all user-facing channels (excludes `api`).
    total_user_channel_accounts: usize,
    next_steps: Vec<String>,
}

fn stdin_is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn trim_to_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn prompt_line(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_owned())
}

fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        let value = prompt_line(&format!("{prompt} {suffix} "))?;
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Ok(default);
        }
        match normalized.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                println!("Please answer y or n.");
            }
        }
    }
}

fn ensure_onboard_api_account(config: &mut GaryxConfig, account_id: &str) -> bool {
    let account_id = account_id.trim();
    if let Some(account) = config.channels.api.accounts.get_mut(account_id) {
        account.enabled = true;
        return false;
    }
    config.channels.api.accounts.insert(
        account_id.to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: None,
            workspace_dir: None,
            workspace_mode: None,
        },
    );
    true
}

fn user_channel_account_count(cfg: &GaryxConfig) -> usize {
    cfg.channels
        .plugins
        .values()
        .map(|plugin_cfg| plugin_cfg.accounts.len())
        .sum::<usize>()
}

fn next_onboard_steps(cfg: &GaryxConfig) -> Vec<String> {
    let mut steps = Vec::new();
    // If the user skipped channel binding entirely, nudge them back to
    // `garyx channels add` so they don't end up with a gateway that has
    // nothing to talk to.
    if user_channel_account_count(cfg) == 0 {
        steps.push("garyx channels add  # 绑定至少一个聊天渠道".to_owned());
    }
    steps.push("garyx status".to_owned());
    steps.push("garyx config show".to_owned());
    steps.push("garyx doctor".to_owned());
    steps
}

fn print_onboard_summary(summary: &OnboardSummary) {
    println!("Onboarding complete.");
    println!("Config: {}", summary.config_path);
    if summary.api_account_created {
        println!("API account: {} (created)", summary.api_account);
    } else {
        println!("API account: {} (enabled)", summary.api_account);
    }
    if summary.channels_bound.is_empty() {
        println!(
            "User-facing channels: {} configured (none bound this session)",
            summary.total_user_channel_accounts
        );
    } else {
        println!(
            "User-facing channels: {} configured (bound this session: {})",
            summary.total_user_channel_accounts,
            summary.channels_bound.join(", "),
        );
    }
}

pub(crate) async fn cmd_onboard(
    config_path: &str,
    options: OnboardCommandOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let config_path = prepared.active_path;
    let existed_before = config_path.exists();
    let created_config = options.force || !existed_before;
    if created_config {
        let default_value = serde_json::to_value(GaryxConfig::default())?;
        write_config_value_atomic(&config_path, &default_value, &ConfigWriteOptions::default())?;
    }

    let loaded = load_config_or_default(
        &config_path.to_string_lossy(),
        ConfigRuntimeOverrides::default(),
    )?;
    print_diagnostics(&loaded.diagnostics);
    let mut cfg = loaded.config;

    let api_account =
        trim_to_option(Some(options.api_account.as_str())).unwrap_or_else(|| "main".to_owned());
    let api_account_created = ensure_onboard_api_account(&mut cfg, &api_account);

    let interactive = !options.json && stdin_is_interactive();
    let mut channels_bound: Vec<String> = Vec::new();
    if interactive {
        println!("Garyx onboarding");
        if created_config {
            println!("Initialized config at {}", config_path.display());
        } else {
            println!("Using existing config at {}", config_path.display());
        }
        println!("Gateway/API account `{api_account}` will be available after setup.");

        // ---- Channel binding ----
        // The api.* account auto-created above lets programs talk to gateway,
        // but a human needs at least one user-facing channel (telegram /
        // discord / feishu / weixin / subprocess plugin) to actually chat with gary.
        // Push the user through that now rather than leaving it as a footnote.
        let pre_bound = user_channel_account_count(&cfg);
        if pre_bound == 0 {
            println!();
            println!("还需要绑定至少一个聊天渠道，garyx 才能在 IM 里使用。");
        }
        let mut want_bind = prompt_yes_no(
            if pre_bound == 0 {
                "现在绑定渠道？"
            } else {
                "再绑定一个渠道？"
            },
            pre_bound == 0,
        )?;
        while want_bind {
            let channel = prompt_channel_choice()?;
            match interactive_bind_channel(&mut cfg, &channel).await {
                Ok(account_id) => {
                    // Persist immediately so a ctrl-c while deciding about
                    // the next channel doesn't throw away a successful QR
                    // scan or a freshly typed token.
                    if let Err(err) = save_config_struct(&config_path, &cfg) {
                        eprintln!("[warn] 保存渠道配置失败：{err}");
                    } else {
                        println!("✓ 已绑定 {channel}.{account_id}");
                    }
                    channels_bound.push(format!("{channel}.{account_id}"));
                }
                Err(err) => {
                    eprintln!("绑定 {channel} 失败：{err}");
                }
            }
            want_bind = prompt_yes_no("继续绑定下一个渠道？", false)?;
        }
    }

    save_config_struct(&config_path, &cfg)?;
    notify_gateway_reload_quiet(&config_path).await;
    let gateway_running = gateway_is_reachable(&config_path).await;

    let summary = OnboardSummary {
        ok: true,
        config_path: config_path.display().to_string(),
        created_config,
        api_account: api_account.clone(),
        api_account_created,
        gateway_run_requested: options.run_gateway,
        channels_bound,
        total_user_channel_accounts: user_channel_account_count(&cfg),
        next_steps: next_onboard_steps(&cfg),
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print_onboard_summary(&summary);
    }

    let should_run_gateway = if gateway_running {
        if options.run_gateway && !options.json {
            println!("Gateway is already running.");
        }
        false
    } else if options.run_gateway {
        true
    } else if interactive {
        prompt_yes_no("Start gateway now?", true)?
    } else {
        false
    };

    if should_run_gateway {
        if !options.json {
            println!("Starting gateway...");
        }
        return run_gateway(
            &config_path.to_string_lossy(),
            options.port_override,
            options.host_override,
            options.no_channels,
        )
        .await;
    }

    if !options.json {
        println!("Next steps:");
        for (index, step) in summary.next_steps.iter().enumerate() {
            println!("{}. {}", index + 1, step);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_onboard_steps_suggests_channel_bind_when_empty() {
        let cfg = GaryxConfig::default();
        let steps = next_onboard_steps(&cfg);
        assert!(
            steps.iter().any(|s| s.contains("garyx channels add")),
            "expected channel-add hint in fresh config, got {steps:?}"
        );
        assert!(
            steps.iter().any(|s| s == "garyx status"),
            "expected status check after onboarding, got {steps:?}"
        );
        assert!(
            !steps.iter().any(|s| s.contains("gateway install")),
            "onboarding should assume the gateway service is already installed, got {steps:?}"
        );
    }

    #[test]
    fn next_onboard_steps_omits_channel_bind_when_user_channel_exists() {
        let mut cfg = GaryxConfig::default();
        cfg.channels.plugin_channel_mut("telegram").accounts.insert(
            "alice".to_owned(),
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
        let steps = next_onboard_steps(&cfg);
        assert!(
            !steps.iter().any(|s| s.contains("garyx channels add")),
            "should not nag about binding when a channel already exists, got {steps:?}"
        );
    }

    #[test]
    fn user_channel_account_count_ignores_api_accounts() {
        let mut cfg = GaryxConfig::default();
        cfg.channels.api.accounts.insert(
            "main".to_owned(),
            ApiAccount {
                enabled: true,
                name: None,
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                workspace_mode: None,
            },
        );
        // api-only should still count as zero user-facing channels
        assert_eq!(user_channel_account_count(&cfg), 0);
    }
}
