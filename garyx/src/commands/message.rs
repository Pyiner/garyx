use super::*;

// ---------------------------------------------------------------------------
// garyx message
// ---------------------------------------------------------------------------
pub(crate) async fn cmd_send_message(
    config_path: &str,
    bot: Option<&str>,
    image: Option<&Path>,
    file: Option<&Path>,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bot = resolve_cli_message_bot(bot)?;
    if image.is_some() && file.is_some() {
        return Err("message supports at most one attachment: choose --image or --file".into());
    }
    let image_path = image.map(|path| {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string()
    });
    let file_path = file.map(|path| {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string()
    });
    if text.trim().is_empty() && image_path.is_none() && file_path.is_none() {
        return Err("message text, --image, or --file is required".into());
    }

    let gateway = gateway_endpoint(config_path)?;
    let url = format!("{}/api/send", gateway.base_url);

    let mut body = serde_json::json!({
        "bot": bot,
        "text": text,
    });
    if let Some(image_path) = image_path {
        body["image"] = serde_json::Value::String(image_path);
    }
    if let Some(file_path) = file_path {
        body["file"] = serde_json::Value::String(file_path);
    }

    let client = reqwest::Client::new();
    let resp = gateway_request(client.post(&url), &gateway)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let status = resp.status();
    let payload: serde_json::Value = resp.json().await?;

    if status.is_success() && payload.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        let ids = payload
            .get("message_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!("✅ 已发送 (message_ids: {ids})");
    } else {
        let error = payload
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        eprintln!("❌ 发送失败: {error}");
        std::process::exit(1);
    }

    Ok(())
}

fn resolve_cli_message_bot(bot: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(bot) = bot.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(bot.to_owned());
    }
    if let Ok(bot) = std::env::var("GARYX_BOT") {
        let bot = bot.trim();
        if !bot.is_empty() {
            return Ok(bot.to_owned());
        }
    }
    let channel = std::env::var("GARYX_CHANNEL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let account_id = std::env::var("GARYX_ACCOUNT_ID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let (Some(channel), Some(account_id)) = (channel, account_id) {
        return Ok(format!("{channel}:{account_id}"));
    }
    Err("bot is required: pass --bot channel:account_id or set GARYX_BOT/GARYX_CHANNEL+GARYX_ACCOUNT_ID".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::*;

    #[test]
    fn resolve_cli_message_bot_prefers_explicit_bot() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _bot = ScopedEnvVar::set_string("GARYX_BOT", "telegram:env");

        let bot = resolve_cli_message_bot(Some("telegram:explicit")).unwrap();

        assert_eq!(bot, "telegram:explicit");
    }

    #[test]
    fn resolve_cli_message_bot_uses_env_bot() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _bot = ScopedEnvVar::set_string("GARYX_BOT", "telegram:main");
        let _channel = ScopedEnvVar::remove("GARYX_CHANNEL");
        let _account = ScopedEnvVar::remove("GARYX_ACCOUNT_ID");

        let bot = resolve_cli_message_bot(None).unwrap();

        assert_eq!(bot, "telegram:main");
    }

    #[test]
    fn resolve_cli_message_bot_uses_channel_account_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _bot = ScopedEnvVar::remove("GARYX_BOT");
        let _channel = ScopedEnvVar::set_string("GARYX_CHANNEL", "telegram");
        let _account = ScopedEnvVar::set_string("GARYX_ACCOUNT_ID", "main");

        let bot = resolve_cli_message_bot(None).unwrap();

        assert_eq!(bot, "telegram:main");
    }

    #[test]
    fn resolve_cli_message_bot_requires_bot_context() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _bot = ScopedEnvVar::remove("GARYX_BOT");
        let _channel = ScopedEnvVar::remove("GARYX_CHANNEL");
        let _account = ScopedEnvVar::remove("GARYX_ACCOUNT_ID");

        let error = resolve_cli_message_bot(None).unwrap_err().to_string();

        assert!(error.contains("bot is required"));
    }
}
