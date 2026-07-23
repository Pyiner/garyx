use super::*;

const THREAD_LIFECYCLE_RETRY_BACKOFFS: [std::time::Duration; 5] = [
    std::time::Duration::from_secs(1),
    std::time::Duration::from_secs(2),
    std::time::Duration::from_secs(4),
    std::time::Duration::from_secs(8),
    std::time::Duration::from_secs(8),
];

enum ThreadLifecycleAttempt {
    Applied(Value),
    Retry(String),
    Terminal(GatewayCliError),
}

fn print_thread_summary(value: &Value) {
    let thread_id = value["thread_id"].as_str().unwrap_or("-");
    let label = value["label"].as_str().unwrap_or("-");
    let workspace_dir = value["workspace_dir"].as_str().unwrap_or("(none)");
    println!("Thread: {thread_id}");
    println!("Label: {label}");
    println!("Workspace: {workspace_dir}");
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

pub(crate) async fn cmd_thread_archive(
    config_path: &str,
    thread_id: &str,
    endpoint_keys: Vec<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let thread_id = require_lifecycle_thread_id(thread_id)?;
    let mut endpoint_keys = endpoint_keys
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    endpoint_keys.sort();
    endpoint_keys.dedup();
    let payload = execute_thread_lifecycle(
        config_path,
        "thread_archive",
        &thread_id,
        reqwest::Method::POST,
        &format!("/api/threads/{}/archive", urlencoding::encode(&thread_id)),
        endpoint_keys,
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    let changed = payload["changed"].as_bool().unwrap_or(false);
    println!(
        "{}: {thread_id}",
        if changed {
            "Archived thread"
        } else {
            "Thread already archived"
        }
    );
    Ok(())
}

pub(crate) async fn cmd_thread_delete(
    config_path: &str,
    thread_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let thread_id = require_lifecycle_thread_id(thread_id)?;
    let payload = execute_thread_lifecycle(
        config_path,
        "thread_delete",
        &thread_id,
        reqwest::Method::DELETE,
        &format!("/api/threads/{}", urlencoding::encode(&thread_id)),
        Vec::new(),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    let changed = payload["changed"].as_bool().unwrap_or(false);
    println!(
        "{}: {thread_id}",
        if changed {
            "Deleted thread"
        } else {
            "Thread already deleted"
        }
    );
    Ok(())
}

fn require_lifecycle_thread_id(thread_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let thread_id = thread_id.trim();
    if !thread_id.starts_with("thread::") {
        return Err("thread_id must be a canonical id like `thread::...`".into());
    }
    Ok(thread_id.to_owned())
}

async fn execute_thread_lifecycle(
    config_path: &str,
    operation: &str,
    thread_id: &str,
    method: reqwest::Method,
    path: &str,
    endpoint_keys: Vec<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let identity = fetch_gateway_json(&gateway, "/api/thread-summaries?limit=1").await?;
    let expected_store_incarnation = identity["store_incarnation_id"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("gateway thread summaries omitted store_incarnation_id")?
        .to_owned();
    let operation_id = uuid::Uuid::new_v4().to_string();
    let payload =
        thread_lifecycle_payload(&operation_id, &expected_store_incarnation, endpoint_keys);
    let mut last_ambiguous = "thread lifecycle result was unavailable".to_owned();

    for attempt_index in 0..=THREAD_LIFECYCLE_RETRY_BACKOFFS.len() {
        let disposition =
            match thread_lifecycle_gateway_raw_once(&gateway, method.clone(), path, &payload).await
            {
                Ok(response) => classify_thread_lifecycle_response(
                    response,
                    operation,
                    &operation_id,
                    thread_id,
                ),
                Err(error) => ThreadLifecycleAttempt::Retry(error.to_string()),
            };
        match disposition {
            ThreadLifecycleAttempt::Applied(value) => return Ok(value),
            ThreadLifecycleAttempt::Terminal(error) => return Err(error.into()),
            ThreadLifecycleAttempt::Retry(message) => last_ambiguous = message,
        }
        let Some(delay) = THREAD_LIFECYCLE_RETRY_BACKOFFS.get(attempt_index) else {
            break;
        };
        tokio::time::sleep(*delay).await;
    }

    Err(GatewayCliError {
        kind: GatewayErrorKind::Unreachable,
        message: format!(
            "thread lifecycle result remained ambiguous after {} attempts: {last_ambiguous}",
            THREAD_LIFECYCLE_RETRY_BACKOFFS.len() + 1
        ),
    }
    .into())
}

fn thread_lifecycle_payload(
    operation_id: &str,
    expected_store_incarnation: &str,
    endpoint_keys: Vec<String>,
) -> Value {
    json!({
        "operationId": operation_id,
        "expectedStoreIncarnation": expected_store_incarnation,
        "endpointKeys": endpoint_keys,
    })
}

fn classify_thread_lifecycle_response(
    response: RawGatewayResponse,
    expected_operation: &str,
    expected_operation_id: &str,
    expected_thread_id: &str,
) -> ThreadLifecycleAttempt {
    let status = match reqwest::StatusCode::from_u16(response.status) {
        Ok(status) => status,
        Err(error) => return ThreadLifecycleAttempt::Retry(error.to_string()),
    };
    let body = String::from_utf8_lossy(&response.raw_body);
    let payload = match serde_json::from_slice::<Value>(&response.raw_body) {
        Ok(payload) => payload,
        Err(error) => {
            return ThreadLifecycleAttempt::Retry(if body.trim().is_empty() {
                error.to_string()
            } else {
                body.trim().to_owned()
            });
        }
    };
    if status.is_success() {
        let outcome = payload["outcome"].as_str().unwrap_or_default();
        let changed = payload["changed"].as_bool();
        let applied = matches!(outcome, "applied_changed" | "applied_noop")
            && changed == Some(outcome == "applied_changed")
            && payload["operation_id"] == expected_operation_id
            && payload["thread_id"] == expected_thread_id
            && payload["deleted"] == true
            && payload["detached_endpoint_keys"].is_array()
            && (expected_operation != "thread_archive" || payload["archived"] == true);
        if !applied {
            return ThreadLifecycleAttempt::Retry(
                "gateway returned an invalid thread lifecycle success payload".to_owned(),
            );
        }
        return ThreadLifecycleAttempt::Applied(payload);
    }

    let operation = payload["operation"].as_str().unwrap_or_default();
    let code = payload["code"].as_str().unwrap_or_default();
    let endpoint_match =
        payload["kind"] == "garyx_api_error" && operation == expected_operation && !code.is_empty();
    let auth_match = payload["kind"] == "garyx_api_error"
        && operation == "gateway_auth"
        && matches!(status.as_u16(), 401 | 403)
        && matches!(code, "unauthorized" | "forbidden");
    let message = payload["message"]
        .as_str()
        .or_else(|| payload["error"].as_str())
        .unwrap_or_else(|| body.trim())
        .to_owned();
    if !endpoint_match && !auth_match {
        return ThreadLifecycleAttempt::Retry(if message.is_empty() {
            format!("gateway returned untagged {status}")
        } else {
            message
        });
    }
    if matches!(code, "operation_in_progress" | "unavailable") {
        return ThreadLifecycleAttempt::Retry(message);
    }

    let kind = if status == reqwest::StatusCode::NOT_FOUND {
        GatewayErrorKind::NotFound
    } else if status == reqwest::StatusCode::CONFLICT {
        GatewayErrorKind::Conflict
    } else {
        GatewayErrorKind::Rejected
    };
    let message = if code == "operation_id_conflict" {
        format!("client bug: lifecycle operation_id conflict: {message}")
    } else if message.is_empty() {
        format!("gateway rejected thread lifecycle request: {status} {code}")
    } else {
        format!("gateway rejected thread lifecycle request: {status}: {message}")
    };
    ThreadLifecycleAttempt::Terminal(GatewayCliError { kind, message })
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
        let updated_at = format_local_timestamp(active_run["updated_at"].as_str());
        println!(
            "Active run: {run_id}  provider={active_provider_label} ({})  pending_inputs={pending_user_input_count}  updated={updated_at}",
            active_provider_type.unwrap_or("-"),
        );
    }
    println!("Ledger:");
    if ledger_records.is_empty() {
        println!("  (no records)");
    } else {
        for record in ledger_records.iter().rev().take(10).rev() {
            let status = record["status"].as_str().unwrap_or("unknown");
            let reason = record["terminal_reason"].as_str().unwrap_or("-");
            let updated_at = format_local_timestamp(record["updated_at"].as_str());
            let excerpt = record["text_excerpt"].as_str().unwrap_or("");
            println!("  - {updated_at}  {status}  reason={reason}  {excerpt}");
        }
    }
    Ok(())
}

fn provider_type_display(value: Option<&str>) -> &'static str {
    match value.unwrap_or("").trim() {
        "codex_app_server" => "Codex",
        "claude_code" => "Claude",
        "traex" => "Traex",
        "antigravity" => "Antigravity",
        "grok_build" => "Grok",
        _ => "-",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use axum::{Json, Router, http::StatusCode, routing::get, routing::post};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc as StdArc, Mutex};
    use tempfile::tempdir;

    fn lifecycle_response(status: u16, payload: Value) -> RawGatewayResponse {
        let raw_body = serde_json::to_vec(&payload).expect("serialize response");
        RawGatewayResponse {
            status,
            body_len: raw_body.len(),
            raw_body,
        }
    }

    #[test]
    fn lifecycle_retry_budget_matches_client_contract() {
        assert_eq!(GATEWAY_LIFECYCLE_TIMEOUT, std::time::Duration::from_secs(8));
        assert_eq!(
            THREAD_LIFECYCLE_RETRY_BACKOFFS,
            [
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(4),
                std::time::Duration::from_secs(8),
                std::time::Duration::from_secs(8),
            ]
        );
    }

    #[test]
    fn provider_type_display_includes_grok_build() {
        assert_eq!(provider_type_display(Some("grok_build")), "Grok");
    }

    #[test]
    fn lifecycle_payload_keeps_one_operation_and_incarnation_identity() {
        let payload = thread_lifecycle_payload(
            "10000000-0000-4000-8000-000000000001",
            "20000000-0000-4000-8000-000000000002",
            vec!["api::main::loop".to_owned()],
        );
        assert_eq!(
            payload["operationId"],
            "10000000-0000-4000-8000-000000000001"
        );
        assert_eq!(
            payload["expectedStoreIncarnation"],
            "20000000-0000-4000-8000-000000000002"
        );
        assert_eq!(payload["endpointKeys"], json!(["api::main::loop"]));
    }

    #[test]
    fn lifecycle_classifier_retries_only_ambiguous_equivalents() {
        for (status, code) in [(409, "operation_in_progress"), (503, "unavailable")] {
            let disposition = classify_thread_lifecycle_response(
                lifecycle_response(
                    status,
                    json!({
                        "kind": "garyx_api_error",
                        "operation": "thread_archive",
                        "code": code,
                        "message": code,
                    }),
                ),
                "thread_archive",
                "10000000-0000-4000-8000-000000000001",
                "thread::lifecycle",
            );
            assert!(matches!(disposition, ThreadLifecycleAttempt::Retry(_)));
        }
        assert!(matches!(
            classify_thread_lifecycle_response(
                lifecycle_response(500, json!({"error": "untagged"})),
                "thread_archive",
                "10000000-0000-4000-8000-000000000001",
                "thread::lifecycle"
            ),
            ThreadLifecycleAttempt::Retry(_)
        ));
    }

    #[test]
    fn lifecycle_classifier_has_applied_rejected_and_conflict_terminal_paths() {
        assert!(matches!(
            classify_thread_lifecycle_response(
                lifecycle_response(
                    200,
                    json!({
                        "operation_id": "10000000-0000-4000-8000-000000000001",
                        "outcome": "applied_changed",
                        "thread_id": "thread::lifecycle",
                        "changed": true,
                        "archived": true,
                        "deleted": true,
                        "detached_endpoint_keys": [],
                    }),
                ),
                "thread_archive",
                "10000000-0000-4000-8000-000000000001",
                "thread::lifecycle"
            ),
            ThreadLifecycleAttempt::Applied(_)
        ));
        assert!(matches!(
            classify_thread_lifecycle_response(
                lifecycle_response(
                    200,
                    json!({
                        "operation_id": "10000000-0000-4000-8000-000000000099",
                        "outcome": "applied_changed",
                        "thread_id": "thread::lifecycle",
                        "changed": true,
                        "archived": true,
                        "deleted": true,
                        "detached_endpoint_keys": [],
                    }),
                ),
                "thread_archive",
                "10000000-0000-4000-8000-000000000001",
                "thread::lifecycle"
            ),
            ThreadLifecycleAttempt::Retry(_)
        ));

        let rejected = classify_thread_lifecycle_response(
            lifecycle_response(
                404,
                json!({
                    "kind": "garyx_api_error",
                    "operation": "thread_archive",
                    "code": "rejected_not_found",
                    "message": "missing",
                }),
            ),
            "thread_archive",
            "10000000-0000-4000-8000-000000000001",
            "thread::lifecycle",
        );
        assert!(matches!(
            rejected,
            ThreadLifecycleAttempt::Terminal(GatewayCliError {
                kind: GatewayErrorKind::NotFound,
                ..
            })
        ));

        let conflict = classify_thread_lifecycle_response(
            lifecycle_response(
                409,
                json!({
                    "kind": "garyx_api_error",
                    "operation": "thread_archive",
                    "code": "operation_id_conflict",
                    "message": "bad reuse",
                }),
            ),
            "thread_archive",
            "10000000-0000-4000-8000-000000000001",
            "thread::lifecycle",
        );
        match conflict {
            ThreadLifecycleAttempt::Terminal(error) => {
                assert_eq!(error.kind, GatewayErrorKind::Conflict);
                assert!(error.message.contains("client bug"));
            }
            _ => panic!("operation_id_conflict must terminate"),
        }
    }

    #[tokio::test]
    async fn cli_lifecycle_in_progress_resends_the_same_operation_identity() {
        let requests = StdArc::new(Mutex::new(Vec::<Value>::new()));
        let attempts = StdArc::new(AtomicUsize::new(0));
        let archive_requests = requests.clone();
        let archive_attempts = attempts.clone();
        let app = Router::new()
            .route(
                "/api/thread-summaries",
                get(|| async {
                    Json(json!({
                        "store_incarnation_id": "20000000-0000-4000-8000-000000000002",
                        "server_boot_id": "30000000-0000-4000-8000-000000000003",
                        "threads": [],
                        "count": 0,
                        "limit": 1,
                        "total": 0,
                        "has_more": false,
                        "next_cursor": null,
                    }))
                }),
            )
            .route(
                "/api/threads/{thread_id}/archive",
                post(move |Json(payload): Json<Value>| {
                    let requests = archive_requests.clone();
                    let attempts = archive_attempts.clone();
                    async move {
                        requests.lock().expect("request lock").push(payload.clone());
                        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            (
                                StatusCode::CONFLICT,
                                Json(json!({
                                    "kind": "garyx_api_error",
                                    "operation": "thread_archive",
                                    "code": "operation_in_progress",
                                    "message": "still working",
                                })),
                            )
                        } else {
                            (
                                StatusCode::OK,
                                Json(json!({
                                    "operation_id": payload["operationId"],
                                    "outcome": "applied_changed",
                                    "changed": true,
                                    "archived": true,
                                    "deleted": true,
                                    "thread_id": "thread::cli-lifecycle",
                                    "detached_endpoint_keys": [],
                                })),
                            )
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve lifecycle test");
        });
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &format!("http://{addr}"));

        cmd_thread_archive(
            config_path.to_str().expect("config path"),
            "thread::cli-lifecycle",
            vec!["api::main::loop".to_owned()],
            true,
        )
        .await
        .expect("archive should converge");
        handle.abort();

        let requests = requests.lock().expect("request lock");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["operationId"], requests[1]["operationId"]);
        assert_eq!(
            requests[0]["expectedStoreIncarnation"],
            "20000000-0000-4000-8000-000000000002"
        );
        assert_eq!(requests[0]["endpointKeys"], json!(["api::main::loop"]));
    }

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

    #[tokio::test]
    async fn cmd_thread_create_preserves_disabled_agent_gateway_error() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_disabled_agent_rejection_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        let error = cmd_thread_create(
            config_path.to_str().expect("config path"),
            Some("Rejected thread".to_owned()),
            None,
            Some("codex".to_owned()),
            false,
            true,
        )
        .await
        .expect_err("disabled explicit agent must be rejected");

        handle.abort();
        let gateway_error = error
            .downcast_ref::<GatewayCliError>()
            .expect("gateway rejection must remain typed");
        assert_eq!(gateway_error.kind, GatewayErrorKind::Rejected);
        assert_eq!(
            gateway_error.message,
            "gateway request failed: 400 Bad Request: agent is disabled: codex"
        );
        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].body["agentId"], "codex");
    }
}
