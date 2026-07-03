use super::*;

fn parse_command_surface(value: &str) -> Result<CommandSurface, Box<dyn std::error::Error>> {
    let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
    match normalized.as_str() {
        "router" => Ok(CommandSurface::Router),
        "gateway_api" | "gateway" | "api" => Ok(CommandSurface::GatewayApi),
        "desktop_composer" | "desktop" | "composer" => Ok(CommandSurface::DesktopComposer),
        "telegram" => Ok(CommandSurface::Telegram),
        "api_chat" | "chat" => Ok(CommandSurface::ApiChat),
        "plugin" | "plugins" => Ok(CommandSurface::Plugin),
        _ => Err(format!(
            "unknown command surface '{value}'. Expected router, gateway_api, desktop_composer, telegram, api_chat, or plugin"
        )
        .into()),
    }
}

pub(super) fn command_prompt_preview(prompt: &str, max_chars: usize) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let mut preview = compact
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        preview.push('…');
        preview
    }
}

pub(super) fn read_shortcut_prompt(
    prompt: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(prompt) = prompt {
        let trimmed = prompt.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_owned());
        }
    }

    if std::io::stdin().is_terminal() {
        return Err("provide --prompt or pipe prompt text on stdin".into());
    }

    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    let prompt = buffer.trim();
    if prompt.is_empty() {
        return Err("prompt cannot be empty".into());
    }
    Ok(prompt.to_owned())
}

fn normalize_cli_shortcut_name(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let normalized = normalize_shortcut_command_name(name);
    if !is_valid_shortcut_command_name(&normalized) {
        return Err(
            "command name must be 1-32 chars using only lowercase a-z, 0-9, and _"
                .to_owned()
                .into(),
        );
    }
    Ok(normalized)
}

pub(crate) fn cmd_command_list(
    config_path: &str,
    json_output: bool,
    surface: Option<String>,
    channel: Option<String>,
    account_id: Option<String>,
    include_hidden: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let surface = surface.as_deref().map(parse_command_surface).transpose()?;
    let catalog = command_catalog_for_config(
        &loaded.config,
        CommandCatalogOptions {
            surface,
            channel,
            account_id,
            include_hidden,
        },
    );

    if json_output {
        println!("{}", serde_json::to_string_pretty(&catalog)?);
        return Ok(());
    }

    for warning in &catalog.warnings {
        eprintln!("warning: {}: {}", warning.code, warning.message);
    }

    if catalog.commands.is_empty() {
        println!("No commands found");
        return Ok(());
    }

    for command in &catalog.commands {
        println!(
            "{:<18} {:<14} {}",
            command.slash,
            serde_json::to_string(&command.kind)?.trim_matches('"'),
            command.description
        );
    }
    Ok(())
}

pub(crate) fn cmd_command_get(
    config_path: &str,
    name: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let normalized = normalize_cli_shortcut_name(name)?;
    let Some(command) = loaded
        .config
        .commands
        .iter()
        .find(|command| normalize_shortcut_command_name(&command.name) == normalized)
    else {
        return Err(format!("shortcut '/{normalized}' not found").into());
    };

    if json_output {
        let slash = format!("/{normalized}");
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": normalized.clone(),
                "slash": slash,
                "description": command.description.clone(),
                "prompt": command.prompt.as_deref().unwrap_or_default(),
            }))?
        );
    } else {
        println!("/{normalized}");
        if !command.description.trim().is_empty() {
            println!("Description: {}", command.description.trim());
        }
        println!("Prompt:");
        println!("{}", command.prompt.as_deref().unwrap_or_default());
    }
    Ok(())
}

pub(crate) async fn cmd_command_set(
    config_path: &str,
    name: String,
    prompt: Option<String>,
    description: Option<String>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let config_path = loaded.path;
    let mut config = loaded.config;
    let normalized = normalize_cli_shortcut_name(&name)?;
    if reserved_command_names().contains(normalized.as_str()) {
        return Err(
            format!("shortcut '/{normalized}' collides with a built-in channel command").into(),
        );
    }
    let prompt = read_shortcut_prompt(prompt)?;
    let description = description
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| command_prompt_preview(&prompt, 80));

    let mut created = true;
    if let Some(existing) = config
        .commands
        .iter_mut()
        .find(|command| normalize_shortcut_command_name(&command.name) == normalized)
    {
        existing.name = normalized.clone();
        existing.description = description.clone();
        existing.prompt = Some(prompt.clone());
        existing.skill_id = None;
        created = false;
    } else {
        config.commands.push(SlashCommand {
            name: normalized.clone(),
            description: description.clone(),
            prompt: Some(prompt.clone()),
            skill_id: None,
        });
    }

    save_config_struct(&config_path, &config)?;
    if json_output {
        notify_gateway_reload_quiet(&config_path).await;
        let slash = format!("/{normalized}");
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "created": created,
                "name": normalized.clone(),
                "slash": slash,
                "description": description,
                "prompt": prompt,
            }))?
        );
    } else {
        notify_gateway_reload(&config_path).await;
        println!(
            "{} /{}",
            if created { "Created" } else { "Updated" },
            normalized
        );
    }
    Ok(())
}

pub(crate) async fn cmd_command_delete(
    config_path: &str,
    name: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let config_path = loaded.path;
    let mut config = loaded.config;
    let normalized = normalize_cli_shortcut_name(name)?;
    let before = config.commands.len();
    config
        .commands
        .retain(|command| normalize_shortcut_command_name(&command.name) != normalized);
    if config.commands.len() == before {
        return Err(format!("shortcut '/{normalized}' not found").into());
    }

    save_config_struct(&config_path, &config)?;
    if json_output {
        notify_gateway_reload_quiet(&config_path).await;
        let slash = format!("/{normalized}");
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "deleted": true,
                "name": normalized.clone(),
                "slash": slash,
            }))?
        );
    } else {
        notify_gateway_reload(&config_path).await;
        println!("Deleted /{normalized}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn cmd_command_set_get_and_delete_persist_shortcut() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("gary.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "gateway": {
                    "host": "127.0.0.1",
                    "port": 9
                },
                "commands": []
            }))
            .expect("config json"),
        )
        .expect("write config");

        cmd_command_set(
            config_path.to_str().expect("config path"),
            "/summary".to_owned(),
            Some("Summarize the current thread".to_owned()),
            Some("Summarize thread".to_owned()),
            true,
        )
        .await
        .expect("set shortcut");
        cmd_command_get(config_path.to_str().expect("config path"), "summary", true)
            .expect("get shortcut");

        let loaded = load_config_or_default(
            config_path.to_str().expect("config path"),
            ConfigRuntimeOverrides::default(),
        )
        .expect("load config");
        assert_eq!(loaded.config.commands.len(), 1);
        assert_eq!(loaded.config.commands[0].name, "summary");
        assert_eq!(
            loaded.config.commands[0].prompt.as_deref(),
            Some("Summarize the current thread")
        );

        cmd_command_delete(config_path.to_str().expect("config path"), "/summary", true)
            .await
            .expect("delete shortcut");
        let loaded = load_config_or_default(
            config_path.to_str().expect("config path"),
            ConfigRuntimeOverrides::default(),
        )
        .expect("reload config");
        assert!(loaded.config.commands.is_empty());
    }

    #[tokio::test]
    async fn cmd_command_set_rejects_builtin_collision() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("gary.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "gateway": {
                    "host": "127.0.0.1",
                    "port": 9
                }
            }))
            .expect("config json"),
        )
        .expect("write config");

        let err = cmd_command_set(
            config_path.to_str().expect("config path"),
            "threads".to_owned(),
            Some("custom thread list".to_owned()),
            None,
            true,
        )
        .await
        .expect_err("reserved command must fail");
        assert!(err.to_string().contains("collides"));
    }
}
