use super::*;

pub(crate) async fn cmd_bot_status(
    config_path: &str,
    bot_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bot_id = bot_id.trim();
    if bot_id.is_empty() {
        return Err("bot_id cannot be empty".into());
    }

    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/bot/status?bot_id={}", urlencoding::encode(bot_id)),
    )
    .await?;

    if json {
        return print_pretty_json(&payload);
    }
    if !payload["ok"].as_bool().unwrap_or(false) {
        let message = payload["error"]
            .as_str()
            .or_else(|| payload["reason"].as_str())
            .unwrap_or("bot status failed");
        return Err(message.into());
    }

    println!("Bot: {bot_id}");
    println!(
        "Main endpoint: {}",
        payload["main_endpoint_status"]
            .as_str()
            .unwrap_or("unknown")
    );
    println!(
        "Current thread status: {}",
        payload["current_thread_status"]
            .as_str()
            .unwrap_or("unknown")
    );
    println!(
        "Current thread: {}",
        payload["current_thread_id"].as_str().unwrap_or("-")
    );
    if let Some(workspace_dir) = payload["main_endpoint"]["workspace_dir"].as_str()
        && !workspace_dir.trim().is_empty()
    {
        println!("Workspace: {workspace_dir}");
    }
    println!(
        "Workspace mode: {}",
        payload["workspace_mode"].as_str().unwrap_or("local")
    );
    if let Some(binding_key) = payload["main_endpoint"]["binding_key"].as_str()
        && !binding_key.trim().is_empty()
    {
        println!("Binding key: {binding_key}");
    }
    println!(
        "Provider: {}",
        payload["thread_runtime"]["provider_label"]
            .as_str()
            .unwrap_or("-")
    );
    let active_run = &payload["thread_runtime"]["active_run"];
    if active_run.is_null() {
        println!("Active run: -");
    } else {
        println!(
            "Active run: {}",
            active_run["run_id"].as_str().unwrap_or("-")
        );
    }
    println!("Send command: garyx thread send bot {bot_id} <message>");

    Ok(())
}

fn normalize_bot_selector_arg(bot: &str) -> Result<String, Box<dyn std::error::Error>> {
    let bot = bot.trim();
    let Some((channel, account_id)) = bot.split_once(':') else {
        return Err("bot must be `channel:account_id`, e.g. `telegram:main`".into());
    };
    if channel.trim().is_empty() || account_id.trim().is_empty() {
        return Err("bot must be `channel:account_id`, e.g. `telegram:main`".into());
    }
    Ok(format!("{}:{}", channel.trim(), account_id.trim()))
}

fn normalize_thread_id_arg(thread_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() || !is_thread_key(thread_id) {
        return Err("thread must be a canonical thread id like `thread::...`".into());
    }
    Ok(thread_id.to_owned())
}

fn normalize_endpoint_key_arg(endpoint_key: &str) -> Result<String, Box<dyn std::error::Error>> {
    let endpoint_key = endpoint_key.trim();
    let parts = endpoint_key.split("::").collect::<Vec<_>>();
    if parts.len() < 3 || parts.iter().take(3).any(|part| part.trim().is_empty()) {
        return Err(
            "endpoint must be an endpoint key like `channel::account_id::binding_key`".into(),
        );
    }
    Ok(endpoint_key.to_owned())
}

pub(crate) async fn cmd_endpoint_list(
    config_path: &str,
    bot: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bot = match bot {
        Some(bot) => Some(normalize_bot_selector_arg(bot)?),
        None => None,
    };
    let gateway = gateway_endpoint(config_path)?;
    let mut payload = fetch_gateway_json(&gateway, "/api/channel-endpoints").await?;
    if let Some(bot) = bot.as_deref()
        && let Some((channel, account_id)) = bot.split_once(':')
        && let Some(endpoints) = payload.get_mut("endpoints").and_then(Value::as_array_mut)
    {
        endpoints.retain(|endpoint| {
            endpoint["channel"].as_str() == Some(channel)
                && endpoint["account_id"].as_str() == Some(account_id)
        });
    }

    if json {
        return print_pretty_json(&payload);
    }

    let endpoints = payload
        .get("endpoints")
        .and_then(Value::as_array)
        .ok_or("gateway response missing endpoints")?;
    if endpoints.is_empty() {
        println!("No endpoints found.");
        return Ok(());
    }
    for endpoint in endpoints {
        let endpoint_key = endpoint["endpoint_key"].as_str().unwrap_or("-");
        let channel = endpoint["channel"].as_str().unwrap_or("-");
        let account_id = endpoint["account_id"].as_str().unwrap_or("-");
        let kind = endpoint["conversation_kind"].as_str().unwrap_or("unknown");
        let thread_id = endpoint["thread_id"].as_str().unwrap_or("-");
        let label = endpoint["display_label"].as_str().unwrap_or("-");
        println!("{endpoint_key}");
        println!("  Bot: {channel}:{account_id}");
        println!("  Kind: {kind}");
        println!("  Thread: {thread_id}");
        println!("  Label: {label}");
        if let Some(workspace_dir) = endpoint["workspace_dir"].as_str()
            && !workspace_dir.trim().is_empty()
        {
            println!("  Workspace: {workspace_dir}");
        }
    }
    Ok(())
}

fn print_endpoint_binding_result(
    payload: &Value,
    fallback_endpoint: &str,
    action: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        return print_pretty_json(payload);
    }
    if !payload["ok"].as_bool().unwrap_or(false) {
        let message = payload["error"]
            .as_str()
            .or_else(|| payload["reason"].as_str())
            .unwrap_or("endpoint binding failed");
        return Err(message.into());
    }
    println!(
        "Endpoint: {}",
        payload["endpoint_key"]
            .as_str()
            .unwrap_or(fallback_endpoint)
    );
    println!("Action: {action}");
    println!(
        "Current thread: {}",
        payload["thread_id"]
            .as_str()
            .or_else(|| payload["current_thread_id"].as_str())
            .unwrap_or("-")
    );
    if let Some(previous_thread_id) = payload["previous_thread_id"].as_str()
        && !previous_thread_id.trim().is_empty()
    {
        println!("Previous thread: {previous_thread_id}");
    }
    Ok(())
}

pub(crate) async fn cmd_endpoint_bind(
    config_path: &str,
    endpoint: &str,
    thread: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = normalize_endpoint_key_arg(endpoint)?;
    let thread = normalize_thread_id_arg(thread)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        "/api/channel-bindings/bind",
        &json!({
            "endpointKey": endpoint,
            "threadId": thread,
        }),
    )
    .await?;
    print_endpoint_binding_result(&payload, &endpoint, "bind", json)
}

pub(crate) async fn cmd_endpoint_detach(
    config_path: &str,
    endpoint: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = normalize_endpoint_key_arg(endpoint)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        "/api/channel-bindings/detach",
        &json!({
            "endpointKey": endpoint,
        }),
    )
    .await?;
    print_endpoint_binding_result(&payload, &endpoint, "detach", json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_bot_selector_arg_accepts_channel_account_selector() {
        let bot = normalize_bot_selector_arg(" telegram:main ").unwrap();

        assert_eq!(bot, "telegram:main");
    }

    #[test]
    fn normalize_bot_selector_arg_rejects_invalid_selector() {
        let error = normalize_bot_selector_arg("telegram")
            .unwrap_err()
            .to_string();

        assert!(error.contains("channel:account_id"));
    }

    #[test]
    fn normalize_thread_id_arg_requires_canonical_thread_id() {
        let error = normalize_thread_id_arg("not-a-thread")
            .unwrap_err()
            .to_string();

        assert!(error.contains("thread::"));
    }
}
