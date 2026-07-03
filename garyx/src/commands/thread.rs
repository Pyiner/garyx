use super::*;

fn print_thread_summary(value: &Value) {
    let thread_id = value["thread_id"].as_str().unwrap_or("-");
    let label = value["label"].as_str().unwrap_or("-");
    let team_id = value["team_id"].as_str();
    let workspace_dir = value["workspace_dir"].as_str().unwrap_or("(none)");
    println!("Thread: {thread_id}");
    println!("Label: {label}");
    println!("Workspace: {workspace_dir}");
    if let Some(team_id) = team_id {
        println!("Team: {team_id}");
    }
}

pub(crate) async fn cmd_thread_list(
    config_path: &str,
    include_hidden: bool,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/threads?include_hidden={include_hidden}&limit={}&offset={}",
            limit.max(1),
            offset
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    let items = payload["threads"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        println!("Threads: (none)");
        return Ok(());
    }
    for item in items {
        print_thread_summary(&item);
        println!();
    }
    Ok(())
}

pub(crate) async fn cmd_thread_get(
    config_path: &str,
    thread_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/threads/{}", urlencoding::encode(thread_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_thread_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_thread_create(
    config_path: &str,
    title: Option<String>,
    workspace_dir: Option<String>,
    agent_id: Option<String>,
    worktree: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace_dir = workspace_dir
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    // agent_id flows through unchanged; team ids and standalone agent ids share
    // one namespace and the gateway's resolver decides which provider to pick.
    let agent_id = agent_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        "/api/threads",
        &json!({
            "label": title.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()),
            "workspaceDir": workspace_dir,
            "workspaceMode": if worktree { "worktree" } else { "local" },
            "agentId": agent_id,
        }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_thread_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_thread_send(
    config_path: &str,
    thread_id: String,
    message: String,
    workspace_dir: Option<String>,
    timeout_secs: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    cmd_thread_send_start(
        config_path,
        Some(thread_id),
        None,
        message,
        workspace_dir,
        timeout_secs,
        json_output,
    )
    .await
}

pub(crate) async fn cmd_thread_send_to_bot(
    config_path: &str,
    bot: String,
    message: String,
    workspace_dir: Option<String>,
    timeout_secs: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    cmd_thread_send_start(
        config_path,
        None,
        Some(bot),
        message,
        workspace_dir,
        timeout_secs,
        json_output,
    )
    .await
}

pub(crate) async fn cmd_thread_send_to_task(
    config_path: &str,
    task_id: String,
    message: String,
    workspace_dir: Option<String>,
    timeout_secs: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/tasks/{}", encode_task_id(&task_id)?),
    )
    .await?;
    let thread_id = payload
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("task '{task_id}' did not resolve to a thread"))?
        .to_owned();
    cmd_thread_send_start(
        config_path,
        Some(thread_id),
        None,
        message,
        workspace_dir,
        timeout_secs,
        json_output,
    )
    .await
}

async fn cmd_thread_send_start(
    config_path: &str,
    thread_id: Option<String>,
    bot: Option<String>,
    message: String,
    workspace_dir: Option<String>,
    timeout_secs: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    };

    let gateway = gateway_endpoint(config_path)?;
    // Build WebSocket URL from HTTP base URL
    let ws_url = gateway
        .base_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let ws_url = format!("{ws_url}/api/chat/ws");

    let mut request = ws_url.into_client_request()?;
    if let Some(token) = gateway.auth_token.as_deref() {
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse()?);
    }

    let (ws_stream, _) = connect_async(request)
        .await
        .map_err(|e| format!("WebSocket connect failed: {e}"))?;
    let (mut write, mut read) = ws_stream.split();

    let mut start_payload = json!({
        "op": "start",
        "message": message,
        "accountId": "cli",
        "fromId": "cli",
        "waitForResponse": false,
        "workspacePath": workspace_dir,
    });
    if let Some(thread_id) = thread_id {
        start_payload["threadId"] = Value::String(thread_id);
    }
    if let Some(bot) = bot {
        start_payload["bot"] = Value::String(bot);
    }

    // Send start message
    let start = serde_json::to_string(&start_payload)?;
    write.send(Message::Text(start.into())).await?;

    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    let mut response_started = false;
    let mut printed_committed_seqs = HashSet::new();

    loop {
        tokio::select! {
            _ = &mut deadline => {
                eprintln!("\n[timeout after {timeout_secs}s]");
                break;
            }
            msg = read.next() => {
                match msg {
                    None => break,
                    Some(Err(e)) => {
                        eprintln!("\n[WebSocket error: {e}]");
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        let event: Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let event_type = event["type"].as_str().unwrap_or("");
                        if json_output {
                            println!("{}", serde_json::to_string(&event)?);
                            if matches!(event_type, "complete" | "error")
                                || committed_control_kind(&event).is_some_and(|kind| {
                                    matches!(kind, "run_complete" | "run_error")
                                })
                            {
                                break;
                            }
                            continue;
                        }
                        match event_type {
                            "committed_message" => {
                                if let Some(kind) = committed_control_kind(&event)
                                    && matches!(kind, "run_complete" | "run_error")
                                {
                                    if response_started {
                                        println!();
                                    }
                                    break;
                                }
                                let seq = event.get("seq").and_then(Value::as_u64).unwrap_or(0);
                                if seq != 0
                                    && printed_committed_seqs.insert(seq)
                                    && let Some(text) = committed_assistant_text(&event)
                                {
                                    if !response_started {
                                        response_started = true;
                                    }
                                    print!("{text}");
                                    let _ = io::stdout().flush();
                                }
                            }
                            "done" | "complete" => {
                                if response_started {
                                    println!();
                                }
                                break;
                            }
                            "error" => {
                                let msg = event["message"].as_str()
                                    .or_else(|| event["error"].as_str())
                                    .unwrap_or("unknown error");
                                eprintln!("\n[error: {msg}]");
                                break;
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn cmd_thread_history(
    config_path: &str,
    thread_id: &str,
    limit: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id cannot be empty".into());
    }

    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/threads/diagnostics?thread_id={}&limit={}",
            urlencoding::encode(thread_id),
            limit.clamp(1, 500)
        ),
    )
    .await?;

    if json {
        return print_pretty_json(&payload);
    }

    let binding_count = payload["bindings"].as_array().map(Vec::len).unwrap_or(0);
    let ledger_records = payload["message_ledger"]["records"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let local_timezone = Local::now().format("%Z").to_string();

    println!("Thread: {thread_id}");
    println!("Bindings: {binding_count}");
    if let Some(path) = payload["transcript_path"].as_str() {
        println!("Transcript: {path}");
    }
    let runtime = &payload["thread_runtime"];
    let provider_type = runtime["provider_type"].as_str();
    let provider_label = provider_type_display(provider_type);
    if provider_type.is_some() {
        println!(
            "Provider: {provider_label} ({})",
            provider_type.unwrap_or("-")
        );
    }
    if let Some(sdk_session_id) = runtime["sdk_session_id"].as_str() {
        println!("SDK session: {sdk_session_id}");
    }
    if runtime["active_run"].is_object() {
        let active_run = &runtime["active_run"];
        let run_id = active_run["run_id"].as_str().unwrap_or("-");
        let active_provider_type = active_run["provider_type"].as_str();
        let active_provider_label = provider_type_display(active_provider_type);
        let pending_user_input_count = active_run["pending_user_input_count"].as_u64().unwrap_or(0);
        let updated_at = format_local_thread_timestamp(active_run["updated_at"].as_str());
        println!(
            "Active run: {run_id}  provider={active_provider_label} ({})  pending_inputs={pending_user_input_count}  updated={updated_at}",
            active_provider_type.unwrap_or("-"),
        );
    }
    println!("Ledger ({local_timezone}):");
    if ledger_records.is_empty() {
        println!("  (no records)");
    } else {
        for record in ledger_records.iter().rev().take(10).rev() {
            let status = record["status"].as_str().unwrap_or("unknown");
            let reason = record["terminal_reason"].as_str().unwrap_or("-");
            let updated_at = format_local_thread_timestamp(record["updated_at"].as_str());
            let excerpt = record["text_excerpt"].as_str().unwrap_or("");
            println!("  - {updated_at}  {status}  reason={reason}  {excerpt}");
        }
    }
    Ok(())
}

fn format_local_thread_timestamp(value: Option<&str>) -> String {
    let raw = value.unwrap_or("-").trim();
    if raw.is_empty() || raw == "-" {
        return "-".to_owned();
    }

    match DateTime::parse_from_rfc3339(raw) {
        Ok(parsed) => parsed
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S %Z")
            .to_string(),
        Err(_) => raw.to_owned(),
    }
}

fn provider_type_display(value: Option<&str>) -> &'static str {
    match value.unwrap_or("").trim() {
        "codex_app_server" => "Codex",
        "gemini_cli" => "Gemini",
        "gpt" => "GPT",
        "garyx_native" => "GPT",
        "claude_code" => "Claude",
        _ => "-",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use std::sync::{Arc as StdArc, Mutex};
    use tempfile::tempdir;

    #[tokio::test]
    async fn cmd_thread_create_posts_worktree_mode() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_thread_create(
            config_path.to_str().expect("config path"),
            Some("Worktree thread".to_owned()),
            Some("/tmp/garyx-repo".to_owned()),
            Some("claude".to_owned()),
            true,
            true,
        )
        .await
        .expect("thread create should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/api/threads");
        assert_eq!(records[0].body["label"], "Worktree thread");
        assert_eq!(records[0].body["workspaceDir"], "/tmp/garyx-repo");
        assert_eq!(records[0].body["agentId"], "claude");
        assert_eq!(records[0].body["workspaceMode"], "worktree");
    }
}
