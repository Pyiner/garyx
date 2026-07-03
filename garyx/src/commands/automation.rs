use super::*;

// ---------------------------------------------------------------------------
// Scheduled automation commands
// ---------------------------------------------------------------------------
fn resolve_automation_workspace_dir(
    value: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = trim_optional_cli(value)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let resolved = path.canonicalize().unwrap_or(path);
    Ok(resolved.display().to_string())
}

fn automation_schedule_from_cli_args(
    args: &AutomationScheduleArgs,
    required: bool,
) -> Result<Option<AutomationScheduleView>, Box<dyn std::error::Error>> {
    let selected_count = [
        args.schedule_json.is_some(),
        args.every_hours.is_some(),
        args.daily_time.is_some(),
        args.once_at.is_some(),
    ]
    .into_iter()
    .filter(|selected| *selected)
    .count();

    if selected_count > 1 {
        return Err(
            "choose exactly one schedule shape: --every-hours, --daily-time, --once-at, or --schedule-json"
                .into(),
        );
    }

    if selected_count == 0 {
        if !args.weekdays.is_empty() || args.timezone.is_some() {
            return Err("--weekday and --timezone require --daily-time".into());
        }
        if required {
            return Err(
                "schedule is required: use --every-hours, --daily-time, --once-at, or --schedule-json"
                    .into(),
            );
        }
        return Ok(None);
    }

    if let Some(raw) = args.schedule_json.as_deref() {
        let schedule = serde_json::from_str::<AutomationScheduleView>(raw)
            .map_err(|error| format!("invalid --schedule-json: {error}"))?;
        return Ok(Some(schedule));
    }

    if let Some(hours) = args.every_hours {
        if hours == 0 {
            return Err("--every-hours must be greater than 0".into());
        }
        return Ok(Some(AutomationScheduleView::Interval { hours }));
    }

    if let Some(time) = args.daily_time.as_deref() {
        let time = trim_required_cli(time, "--daily-time")?;
        let timezone =
            trim_optional_cli(args.timezone.clone()).unwrap_or_else(|| "Asia/Shanghai".to_owned());
        let weekdays = args
            .weekdays
            .iter()
            .filter_map(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            })
            .collect::<Vec<_>>();
        return Ok(Some(AutomationScheduleView::Daily {
            time,
            weekdays,
            timezone,
        }));
    }

    if let Some(at) = args.once_at.as_deref() {
        let at = trim_required_cli(at, "--once-at")?;
        if !args.weekdays.is_empty() || args.timezone.is_some() {
            return Err("--weekday and --timezone are only valid with --daily-time".into());
        }
        return Ok(Some(AutomationScheduleView::Once { at }));
    }

    unreachable!("selected_count guarded all schedule variants")
}

fn format_automation_schedule(schedule: &Value) -> String {
    match schedule["kind"].as_str().unwrap_or_default() {
        "interval" => format!(
            "every {}h",
            schedule["hours"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_owned())
        ),
        "daily" => {
            let time = schedule["time"].as_str().unwrap_or("?");
            let timezone = schedule["timezone"].as_str().unwrap_or("");
            let weekdays = schedule["weekdays"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "daily".to_owned());
            if timezone.is_empty() {
                format!("{weekdays} {time}")
            } else {
                format!("{weekdays} {time} {timezone}")
            }
        }
        "once" => format!("once {}", schedule["at"].as_str().unwrap_or("?")),
        _ => "-".to_owned(),
    }
}

fn print_automation_summary(value: &Value) {
    println!("Automation: {}", value["id"].as_str().unwrap_or("-"));
    println!("Name: {}", value["label"].as_str().unwrap_or("-"));
    println!(
        "Enabled: {}",
        value["enabled"]
            .as_bool()
            .map(|enabled| enabled.to_string())
            .unwrap_or_else(|| "-".to_owned())
    );
    println!("Agent: {}", value["agentId"].as_str().unwrap_or("-"));
    println!(
        "Workspace: {}",
        value["workspaceDir"].as_str().unwrap_or("-")
    );
    if let Some(target_thread_id) = value["targetThreadId"].as_str() {
        println!("Target thread: {target_thread_id}");
    }
    println!(
        "Schedule: {}",
        format_automation_schedule(&value["schedule"])
    );
    println!("Next run: {}", value["nextRun"].as_str().unwrap_or("-"));
    if let Some(thread_id) = value["threadId"].as_str() {
        println!("Thread: {thread_id}");
    }
    if let Some(last_run_at) = value["lastRunAt"].as_str() {
        println!("Last run: {last_run_at}");
    }
    let prompt = value["prompt"].as_str().unwrap_or_default();
    if !prompt.trim().is_empty() {
        println!("Prompt: {}", command_prompt_preview(prompt, 160));
    }
}

fn print_automation_activity_entry(value: &Value) {
    let run_id = value["runId"].as_str().unwrap_or("-");
    let status = value["status"].as_str().unwrap_or("-");
    let started_at = value["startedAt"].as_str().unwrap_or("-");
    let thread_id = value["threadId"].as_str().unwrap_or("-");
    println!("Run: {run_id}");
    println!("Status: {status}");
    println!("Started: {started_at}");
    if let Some(finished_at) = value["finishedAt"].as_str() {
        println!("Finished: {finished_at}");
    }
    if let Some(duration_ms) = value["durationMs"].as_u64() {
        println!("Duration: {duration_ms}ms");
    }
    println!("Thread: {thread_id}");
    if let Some(excerpt) = value["excerpt"].as_str() {
        println!("Excerpt: {}", command_prompt_preview(excerpt, 160));
    }
}

pub(crate) async fn cmd_automation_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/automations").await?;
    if json {
        return print_pretty_json(&payload);
    }
    let items = payload["automations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        println!("Automations: (none)");
        return Ok(());
    }
    println!(
        "{:<42}  {:<7}  {:<28}  {:<25}  NAME",
        "ID", "ENABLED", "SCHEDULE", "NEXT RUN"
    );
    println!("{}", "-".repeat(120));
    for item in &items {
        let id = item["id"].as_str().unwrap_or("-");
        let enabled = if item["enabled"].as_bool().unwrap_or(false) {
            "yes"
        } else {
            "no"
        };
        let schedule = format_automation_schedule(&item["schedule"]);
        let next_run = item["nextRun"].as_str().unwrap_or("-");
        let label = item["label"].as_str().unwrap_or("-");
        println!("{id:<42}  {enabled:<7}  {schedule:<28}  {next_run:<25}  {label}");
    }
    Ok(())
}

pub(crate) async fn cmd_automation_get(
    config_path: &str,
    automation_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let automation_id = trim_required_cli(automation_id, "automation_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/automations/{}", urlencoding::encode(&automation_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_automation_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_automation_create(
    config_path: &str,
    label: String,
    prompt: Option<String>,
    agent_id: Option<String>,
    workspace_dir: Option<String>,
    thread_id: Option<String>,
    schedule: AutomationScheduleArgs,
    disabled: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let label = trim_required_cli(&label, "label")?;
    let prompt = read_shortcut_prompt(prompt)?;
    let thread_id = trim_optional_cli(thread_id);
    let workspace_dir = if thread_id.is_some() {
        workspace_dir
            .map(|value| resolve_automation_workspace_dir(Some(value)))
            .transpose()?
    } else {
        Some(resolve_automation_workspace_dir(workspace_dir)?)
    };
    let schedule = automation_schedule_from_cli_args(&schedule, true)?
        .expect("required automation schedule should be present");
    let gateway = gateway_endpoint(config_path)?;

    let mut body = json!({
        "label": label,
        "prompt": prompt,
        "schedule": schedule,
        "enabled": !disabled,
    });
    if let Some(workspace_dir) = workspace_dir {
        body["workspaceDir"] = json!(workspace_dir);
    }
    if let Some(thread_id) = thread_id {
        body["targetThreadId"] = json!(thread_id);
    }
    if let Some(agent_id) = trim_optional_cli(agent_id) {
        body["agentId"] = json!(agent_id);
    }

    let payload = post_gateway_json(&gateway, "/api/automations", &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_automation_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_automation_update(
    config_path: &str,
    automation_id: &str,
    label: Option<String>,
    prompt: Option<String>,
    agent_id: Option<String>,
    workspace_dir: Option<String>,
    thread_id: Option<String>,
    schedule: AutomationScheduleArgs,
    enable: bool,
    disable: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let automation_id = trim_required_cli(automation_id, "automation_id")?;
    let mut body = Map::new();

    if let Some(label) = label {
        body.insert(
            "label".to_owned(),
            json!(trim_required_cli(&label, "label")?),
        );
    }
    if let Some(prompt) = prompt {
        body.insert(
            "prompt".to_owned(),
            json!(trim_required_cli(&prompt, "prompt")?),
        );
    }
    if let Some(agent_id) = trim_optional_cli(agent_id) {
        body.insert("agentId".to_owned(), json!(agent_id));
    }
    if let Some(workspace_dir) = workspace_dir {
        body.insert(
            "workspaceDir".to_owned(),
            json!(resolve_automation_workspace_dir(Some(workspace_dir))?),
        );
    }
    if let Some(thread_id) = trim_optional_cli(thread_id) {
        body.insert("targetThreadId".to_owned(), json!(thread_id));
    }
    if let Some(schedule) = automation_schedule_from_cli_args(&schedule, false)? {
        body.insert("schedule".to_owned(), json!(schedule));
    }
    if enable {
        body.insert("enabled".to_owned(), json!(true));
    } else if disable {
        body.insert("enabled".to_owned(), json!(false));
    }
    if body.is_empty() {
        return Err("provide at least one automation field to update".into());
    }

    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json(
        &gateway,
        &format!("/api/automations/{}", urlencoding::encode(&automation_id)),
        &Value::Object(body),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_automation_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_automation_delete(
    config_path: &str,
    automation_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let automation_id = trim_required_cli(automation_id, "automation_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json(
        &gateway,
        &format!("/api/automations/{}", urlencoding::encode(&automation_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Deleted automation: {automation_id}");
    Ok(())
}

pub(crate) async fn cmd_automation_run(
    config_path: &str,
    automation_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let automation_id = trim_required_cli(automation_id, "automation_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        &format!(
            "/api/automations/{}/run-now",
            urlencoding::encode(&automation_id)
        ),
        &json!({}),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_automation_activity_entry(&payload);
    Ok(())
}

async fn patch_automation_enabled(
    config_path: &str,
    automation_id: &str,
    enabled: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let automation_id = trim_required_cli(automation_id, "automation_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json(
        &gateway,
        &format!("/api/automations/{}", urlencoding::encode(&automation_id)),
        &json!({ "enabled": enabled }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_automation_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_automation_pause(
    config_path: &str,
    automation_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    patch_automation_enabled(config_path, automation_id, false, json).await
}

pub(crate) async fn cmd_automation_resume(
    config_path: &str,
    automation_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    patch_automation_enabled(config_path, automation_id, true, json).await
}

pub(crate) async fn cmd_automation_activity(
    config_path: &str,
    automation_id: &str,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let automation_id = trim_required_cli(automation_id, "automation_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/automations/{}/activity?limit={}&offset={}",
            urlencoding::encode(&automation_id),
            limit,
            offset
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    let items = payload["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        println!("Automation activity: (none)");
        return Ok(());
    }
    println!(
        "{:<38}  {:<8}  {:<25}  {:<38}  EXCERPT",
        "RUN ID", "STATUS", "STARTED", "THREAD"
    );
    println!("{}", "-".repeat(130));
    for item in &items {
        let run_id = item["runId"].as_str().unwrap_or("-");
        let status = item["status"].as_str().unwrap_or("-");
        let started = item["startedAt"].as_str().unwrap_or("-");
        let thread_id = item["threadId"].as_str().unwrap_or("-");
        let excerpt = item["excerpt"]
            .as_str()
            .map(|text| command_prompt_preview(text, 48))
            .unwrap_or_else(|| "-".to_owned());
        println!("{run_id:<38}  {status:<8}  {started:<25}  {thread_id:<38}  {excerpt}");
    }
    Ok(())
}

fn automation_print_data_triggers(payload: &Value) {
    let triggers = payload["triggers"].as_array().cloned().unwrap_or_default();
    if triggers.is_empty() {
        println!("Triggers: (none)");
        return;
    }
    println!(
        "{:<38}  {:<7}  {:<20}  {:<15}  LABEL",
        "ID", "ENABLED", "TABLE", "EVENT"
    );
    println!("{}", "-".repeat(110));
    for trigger in triggers {
        println!(
            "{:<38}  {:<7}  {:<20}  {:<15}  {}",
            trigger["id"].as_str().unwrap_or("-"),
            trigger["enabled"].as_bool().unwrap_or(false),
            trigger["tableName"].as_str().unwrap_or("-"),
            trigger["eventType"].as_str().unwrap_or("-"),
            trigger["label"]
                .as_str()
                .unwrap_or_else(|| { trigger["titleTemplate"].as_str().unwrap_or("-") }),
        );
    }
}

pub(crate) async fn cmd_automation_data_trigger_list(
    config_path: &str,
    table: Option<String>,
    event_type: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut query = Vec::new();
    if let Some(table) = trim_optional_cli(table) {
        query.push(format!("table={}", urlencoding::encode(&table)));
    }
    if let Some(event_type) = trim_optional_cli(event_type) {
        query.push(format!("eventType={}", urlencoding::encode(&event_type)));
    }
    let path = if query.is_empty() {
        "/api/automations/triggers/data".to_owned()
    } else {
        format!("/api/automations/triggers/data?{}", query.join("&"))
    };
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, &path).await?;
    if json {
        return print_pretty_json(&payload);
    }
    automation_print_data_triggers(&payload);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_automation_data_trigger_create(
    config_path: &str,
    table: &str,
    event_type: &str,
    label: &str,
    title: &str,
    body_text: &str,
    agent_id: Option<String>,
    workspace_dir: Option<String>,
    disabled: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut body = json!({
        "tableName": trim_required_cli(table, "table")?,
        "eventType": trim_required_cli(event_type, "event_type")?,
        "label": trim_required_cli(label, "label")?,
        "titleTemplate": trim_required_cli(title, "title")?,
        "bodyTemplate": trim_required_cli(body_text, "body")?,
        "enabled": !disabled,
    });
    if let Some(agent_id) = trim_optional_cli(agent_id) {
        body["agentId"] = json!(agent_id);
    }
    if let Some(workspace_dir) = trim_optional_cli(workspace_dir) {
        body["workspaceDir"] = json!(workspace_dir);
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(&gateway, "/api/automations/triggers/data", &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "Created data trigger: {}",
        payload["trigger"]["id"].as_str().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_automation_data_trigger_set_enabled(
    config_path: &str,
    trigger_id: &str,
    enabled: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let trigger_id = trim_required_cli(trigger_id, "trigger_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json(
        &gateway,
        &format!(
            "/api/automations/triggers/data/{}",
            urlencoding::encode(&trigger_id)
        ),
        &json!({ "enabled": enabled }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "{} data trigger: {}",
        if enabled { "Enabled" } else { "Disabled" },
        trigger_id
    );
    Ok(())
}

pub(crate) async fn cmd_automation_data_trigger_delete(
    config_path: &str,
    trigger_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let trigger_id = trim_required_cli(trigger_id, "trigger_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json(
        &gateway,
        &format!(
            "/api/automations/triggers/data/{}",
            urlencoding::encode(&trigger_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Deleted data trigger: {trigger_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use axum::{
        Json, Router,
        extract::Path as AxumPath,
        http::StatusCode,
        routing::{get, patch, post},
    };
    use std::sync::{Arc as StdArc, Mutex};
    use tempfile::tempdir;
    use tokio::{net::TcpListener, task::JoinHandle};

    async fn spawn_automation_http_test_server(
        requests: StdArc<Mutex<Vec<RecordedRequest>>>,
    ) -> (String, JoinHandle<()>) {
        let list_requests = requests.clone();
        let create_requests = requests.clone();
        let get_requests = requests.clone();
        let update_requests = requests.clone();
        let delete_requests = requests.clone();
        let run_requests = requests.clone();
        let activity_requests = requests.clone();
        let trigger_list_requests = requests.clone();
        let trigger_create_requests = requests.clone();
        let trigger_patch_requests = requests.clone();
        let trigger_delete_requests = requests.clone();

        let app = Router::new()
        .route(
            "/api/automations",
            get(move || {
                let requests = list_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path: "/api/automations".to_owned(),
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "automations": []
                        })),
                    )
                }
            })
            .post(move |Json(payload): Json<Value>| {
                let requests = create_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path: "/api/automations".to_owned(),
                            body: payload.clone(),
                        });
                    (
                        StatusCode::CREATED,
                        Json(json!({
                            "id": "automation::created",
                            "label": payload["label"],
                            "prompt": payload["prompt"],
                            "agentId": payload.get("agentId").cloned().unwrap_or_else(|| json!("claude")),
                            "enabled": payload.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                            "workspaceDir": payload["workspaceDir"],
                            "nextRun": "2030-05-01T08:30:00Z",
                            "lastStatus": "skipped",
                            "schedule": payload["schedule"],
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/{automation_id}",
            get(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = get_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "id": automation_id,
                            "label": "Daily triage",
                            "prompt": "Summarize repo state",
                            "agentId": "claude",
                            "enabled": true,
                            "workspaceDir": "/tmp/repo",
                            "nextRun": "2030-05-01T08:30:00Z",
                            "lastStatus": "skipped",
                            "schedule": {"kind": "interval", "hours": 6},
                        })),
                    )
                }
            })
            .patch(
                move |AxumPath(automation_id): AxumPath<String>, Json(payload): Json<Value>| {
                    let requests = update_requests.clone();
                    async move {
                        let path = format!("/api/automations/{automation_id}");
                        requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "PATCH".to_owned(),
                                path,
                                body: payload.clone(),
                            });
                        (
                            StatusCode::OK,
                            Json(json!({
                                "id": automation_id,
                                "label": payload.get("label").cloned().unwrap_or_else(|| json!("Daily triage")),
                                "prompt": payload.get("prompt").cloned().unwrap_or_else(|| json!("Summarize repo state")),
                                "agentId": payload.get("agentId").cloned().unwrap_or_else(|| json!("claude")),
                                "enabled": payload.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                                "workspaceDir": payload.get("workspaceDir").cloned().unwrap_or_else(|| json!("/tmp/repo")),
                                "nextRun": "2030-05-01T08:30:00Z",
                                "lastStatus": "skipped",
                                "schedule": payload.get("schedule").cloned().unwrap_or_else(|| json!({"kind": "interval", "hours": 6})),
                            })),
                        )
                    }
                },
            )
            .delete(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = delete_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "DELETE".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "deleted": true,
                            "id": automation_id,
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/{automation_id}/run-now",
            post(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = run_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}/run-now");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "runId": "run-1",
                            "status": "success",
                            "startedAt": "2030-05-01T08:30:00Z",
                            "finishedAt": "2030-05-01T08:30:01Z",
                            "durationMs": 1000,
                            "threadId": "thread::automation-test",
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/{automation_id}/activity",
            get(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = activity_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}/activity");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "items": [],
                            "threadId": null,
                            "count": 0,
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/triggers/data",
            get(move || {
                let requests = trigger_list_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path: "/api/automations/triggers/data".to_owned(),
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "triggers": []
                        })),
                    )
                }
            })
            .post(move |Json(payload): Json<Value>| {
                let requests = trigger_create_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path: "/api/automations/triggers/data".to_owned(),
                            body: payload.clone(),
                        });
                    (
                        StatusCode::CREATED,
                        Json(json!({
                                "trigger": {
                                    "id": "autodata_test",
                                    "label": payload["label"],
                                    "tableName": payload["tableName"],
                                    "eventType": payload["eventType"],
                                "titleTemplate": payload["titleTemplate"],
                                "bodyTemplate": payload["bodyTemplate"],
                                "agentId": payload.get("agentId").cloned().unwrap_or(Value::Null),
                                "workspaceDir": payload.get("workspaceDir").cloned().unwrap_or(Value::Null),
                                "enabled": payload.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                                "createdAt": "2030-05-01T08:30:00Z",
                                "updatedAt": "2030-05-01T08:30:00Z"
                            }
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/triggers/data/{trigger_id}",
            patch(
                move |AxumPath(trigger_id): AxumPath<String>, Json(payload): Json<Value>| {
                    let requests = trigger_patch_requests.clone();
                    async move {
                        let path = format!("/api/automations/triggers/data/{trigger_id}");
                        requests.lock().expect("request lock").push(RecordedRequest {
                            method: "PATCH".to_owned(),
                            path,
                            body: payload.clone(),
                        });
                        (
                            StatusCode::OK,
                            Json(json!({
                                "trigger": {
                                    "id": trigger_id,
                                    "tableName": "contacts",
                                    "eventType": "record.created",
                                    "label": "Contact review",
                                    "titleTemplate": "New record {record_id}",
                                    "bodyTemplate": "Review {table_name}",
                                    "enabled": payload["enabled"],
                                    "createdAt": "2030-05-01T08:30:00Z",
                                    "updatedAt": "2030-05-01T08:31:00Z"
                                }
                            })),
                        )
                    }
                },
            )
            .delete(move |AxumPath(trigger_id): AxumPath<String>| {
                let requests = trigger_delete_requests.clone();
                async move {
                    let path = format!("/api/automations/triggers/data/{trigger_id}");
                    requests.lock().expect("request lock").push(RecordedRequest {
                        method: "DELETE".to_owned(),
                        path,
                        body: Value::Null,
                    });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "deleted": true,
                            "id": trigger_id,
                        })),
                    )
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn automation_schedule_args_build_interval_schedule() {
        let args = crate::cli::AutomationScheduleArgs {
            every_hours: Some(6),
            ..Default::default()
        };

        let schedule = automation_schedule_from_cli_args(&args, true)
            .expect("schedule parse")
            .expect("schedule");

        assert_eq!(schedule, AutomationScheduleView::Interval { hours: 6 });
    }

    #[test]
    fn automation_schedule_args_build_daily_schedule() {
        let args = crate::cli::AutomationScheduleArgs {
            daily_time: Some("08:30".to_owned()),
            weekdays: vec!["mon".to_owned(), "fri".to_owned()],
            timezone: Some("Asia/Shanghai".to_owned()),
            ..Default::default()
        };

        let schedule = automation_schedule_from_cli_args(&args, true)
            .expect("schedule parse")
            .expect("schedule");

        assert_eq!(
            schedule,
            AutomationScheduleView::Daily {
                time: "08:30".to_owned(),
                weekdays: vec!["mon".to_owned(), "fri".to_owned()],
                timezone: "Asia/Shanghai".to_owned(),
            }
        );
    }

    #[test]
    fn automation_schedule_args_reject_ambiguous_schedule_shape() {
        let args = crate::cli::AutomationScheduleArgs {
            every_hours: Some(6),
            once_at: Some("2030-05-01T08:30".to_owned()),
            ..Default::default()
        };

        let error = automation_schedule_from_cli_args(&args, true)
            .expect_err("ambiguous schedule should fail")
            .to_string();

        assert!(error.contains("choose exactly one schedule shape"));
    }

    #[tokio::test]
    async fn cmd_automation_create_posts_disabled_interval_payload() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_automation_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_automation_create(
            config_path.to_str().expect("config path"),
            "Daily triage".to_owned(),
            Some("Summarize repo state".to_owned()),
            Some("codex".to_owned()),
            Some(dir.path().to_string_lossy().to_string()),
            None,
            crate::cli::AutomationScheduleArgs {
                every_hours: Some(6),
                ..Default::default()
            },
            true,
            false,
        )
        .await
        .expect("automation create should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/api/automations");
        assert_eq!(records[0].body["label"], "Daily triage");
        assert_eq!(records[0].body["prompt"], "Summarize repo state");
        assert_eq!(records[0].body["agentId"], "codex");
        assert_eq!(
            records[0].body["workspaceDir"].as_str(),
            Some(
                dir.path()
                    .canonicalize()
                    .expect("canonical tempdir")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert_eq!(records[0].body["enabled"], false);
        assert_eq!(records[0].body["schedule"]["kind"], "interval");
        assert_eq!(records[0].body["schedule"]["hours"], 6);
    }

    #[tokio::test]
    async fn cmd_automation_data_trigger_create_posts_automation_payload() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_automation_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_automation_data_trigger_create(
            config_path.to_str().expect("config path"),
            "contacts",
            "record.created",
            "Contact review",
            "New record {record_id}",
            "Review {table_name}",
            Some("codex".to_owned()),
            Some("/tmp/work".to_owned()),
            true,
            false,
        )
        .await
        .expect("automation data trigger create should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/api/automations/triggers/data");
        assert_eq!(records[0].body["tableName"], "contacts");
        assert_eq!(records[0].body["eventType"], "record.created");
        assert_eq!(records[0].body["label"], "Contact review");
        assert_eq!(records[0].body["titleTemplate"], "New record {record_id}");
        assert_eq!(records[0].body["bodyTemplate"], "Review {table_name}");
        assert_eq!(records[0].body["agentId"], "codex");
        assert_eq!(records[0].body["workspaceDir"], "/tmp/work");
        assert_eq!(records[0].body["enabled"], false);
    }

    #[tokio::test]
    async fn cmd_automation_update_patches_requested_fields() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_automation_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_automation_update(
            config_path.to_str().expect("config path"),
            "automation::created",
            Some("Weekly triage".to_owned()),
            None,
            None,
            None,
            None,
            crate::cli::AutomationScheduleArgs {
                daily_time: Some("09:45".to_owned()),
                timezone: Some("Asia/Shanghai".to_owned()),
                ..Default::default()
            },
            false,
            true,
            false,
        )
        .await
        .expect("automation update should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PATCH");
        assert_eq!(records[0].path, "/api/automations/automation::created");
        assert_eq!(records[0].body["label"], "Weekly triage");
        assert_eq!(records[0].body["enabled"], false);
        assert!(records[0].body.get("prompt").is_none());
        assert_eq!(records[0].body["schedule"]["kind"], "daily");
        assert_eq!(records[0].body["schedule"]["time"], "09:45");
        assert_eq!(records[0].body["schedule"]["timezone"], "Asia/Shanghai");
    }
}
