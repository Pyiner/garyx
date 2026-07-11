use super::*;

pub(crate) async fn cmd_task_list(
    config_path: &str,
    status: Option<&str>,
    source_thread: Option<&str>,
    source_task: Option<&str>,
    source_bot: Option<&str>,
    limit: usize,
    offset: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut params = vec![
        ("limit".to_owned(), limit.clamp(1, 200).to_string()),
        ("offset".to_owned(), offset.to_string()),
        // Done tasks are shown by default; the gateway hides them unless asked.
        ("include_done".to_owned(), "true".to_owned()),
    ];
    if let Some(status) = status {
        params.push(("status".to_owned(), normalize_task_status(status)?));
    }
    if let Some(source_thread) = source_thread
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        params.push(("source_thread_id".to_owned(), source_thread.to_owned()));
    }
    if let Some(source_task) = source_task.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("source_task_id".to_owned(), source_task.to_owned()));
    }
    if let Some(source_bot) = source_bot.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("source_bot_id".to_owned(), source_bot.to_owned()));
    }
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, &format!("/api/tasks?{query}")).await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    let tasks = payload["tasks"].as_array().cloned().unwrap_or_default();
    if tasks.is_empty() {
        println!("Tasks: (none)");
        println!("Filter: --status <todo|in_progress|in_review|done>");
        return Ok(());
    }
    let shown = tasks.len();
    for task in tasks {
        print_task_summary(&task);
        println!();
    }
    match payload["total"].as_u64() {
        Some(total) => println!("Showing {shown} of {total} tasks (offset {offset})."),
        None => println!("Showing {shown} tasks (offset {offset})."),
    }
    if payload["has_more"].as_bool().unwrap_or(false) {
        println!("Next page: --offset {}", offset.saturating_add(shown));
    }
    println!("Filter: --status <todo|in_progress|in_review|done>");
    Ok(())
}

pub(crate) async fn cmd_task_get(
    config_path: &str,
    task_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/tasks/{}", encode_task_id(task_id)?),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    let history_gateway = gateway.clone();
    let history_payload = payload
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|thread_id| async move {
            fetch_gateway_json(
                &history_gateway,
                &format!(
                    "/api/threads/history?thread_id={}&limit=500&include_tool_messages=true",
                    urlencoding::encode(thread_id)
                ),
            )
            .await
            .ok()
        });
    let history_payload = match history_payload {
        Some(fetch) => fetch.await,
        None => None,
    };
    print!(
        "{}",
        format_task_progress(&payload, history_payload.as_ref())
    );
    Ok(())
}

pub(crate) async fn cmd_task_create(
    config_path: &str,
    title: Option<String>,
    body: Option<String>,
    workspace_dir: Option<String>,
    worktree: bool,
    agent: Option<String>,
    notify: Vec<String>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let executor = task_executor_payload(agent)?;
    let notification_target = task_notification_target_payload(notify)?;
    let notify_current_thread = notification_targets_current_thread(&notification_target);
    let source = task_source_payload_from_env();
    let workspace_dir = workspace_dir
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let gateway = gateway_endpoint(config_path)?;
    let request = json!({
        "title": title,
        "body": body,
        "assignee": Value::Null,
        "start": true,
        "workspace_dir": Value::Null,
        "executor": executor,
        "runtime": {
            "agent_id": Value::Null,
            "workspace_dir": workspace_dir,
            "workspace_mode": if worktree { "worktree" } else { "local" },
        },
        "notification_target": notification_target,
        "source": source,
    });
    let payload = post_gateway_json_as_cli_actor(&gateway, "/api/tasks", &request).await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    if notify_current_thread {
        println!();
        println!(
            "You don't need to poll this task or hold this turn open — Garyx will message this thread automatically when it finishes. You can stop now."
        );
    }
    Ok(())
}

/// True when the resolved notification target points at the thread this CLI run
/// is executing inside (the default "notify the current thread" case). Used to
/// reassure an agent caller that it can stop instead of polling the new task.
fn notification_targets_current_thread(target: &Value) -> bool {
    if target.get("kind").and_then(Value::as_str) != Some("thread") {
        return false;
    }
    match (
        target.get("thread_id").and_then(Value::as_str),
        env_nonempty("GARYX_THREAD_ID"),
    ) {
        (Some(target_thread), Some(current_thread)) => target_thread == current_thread,
        _ => false,
    }
}

fn task_executor_payload(agent: Option<String>) -> Result<Value, Box<dyn std::error::Error>> {
    let agent_id = agent
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(agent_id) = agent_id {
        return Ok(json!({
            "type": "agent",
            "agentId": agent_id,
        }));
    }
    Err("Task creation is a delegation feature, so you must specify an Agent with --agent.".into())
}

fn task_source_payload_from_env() -> Option<Value> {
    let thread_id = env_nonempty("GARYX_THREAD_ID");
    let task_id = env_nonempty("GARYX_TASK_ID");
    let task_thread_id = task_id.as_ref().and_then(|_| thread_id.clone());
    let channel = env_nonempty("GARYX_CHANNEL");
    let account_id = env_nonempty("GARYX_ACCOUNT_ID");
    let bot_id = env_nonempty("GARYX_BOT_ID").or_else(|| match (&channel, &account_id) {
        (Some(channel), Some(account_id)) => Some(format!("{channel}:{account_id}")),
        _ => None,
    });

    if thread_id.is_none() && task_id.is_none() && bot_id.is_none() {
        return None;
    }

    let mut source = serde_json::Map::new();
    if let Some(thread_id) = thread_id {
        source.insert("thread_id".to_owned(), Value::String(thread_id));
    }
    if let Some(task_id) = task_id {
        source.insert("task_id".to_owned(), Value::String(task_id));
    }
    if let Some(task_thread_id) = task_thread_id {
        source.insert("task_thread_id".to_owned(), Value::String(task_thread_id));
    }
    if let Some(bot_id) = bot_id {
        source.insert("bot_id".to_owned(), Value::String(bot_id));
    }
    if let Some(channel) = channel {
        source.insert("channel".to_owned(), Value::String(channel));
    }
    if let Some(account_id) = account_id {
        source.insert("account_id".to_owned(), Value::String(account_id));
    }
    Some(Value::Object(source))
}

fn task_notification_target_payload(
    parts: Vec<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let parts: Vec<String> = parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        // No `--notify` given: notify the current thread when running inside one
        // (e.g. an agent delegating a task), otherwise stay silent.
        return Ok(match env_nonempty("GARYX_THREAD_ID") {
            Some(thread_id) if is_thread_key(&thread_id) => {
                json!({ "kind": "thread", "thread_id": thread_id })
            }
            _ => json!({ "kind": "none" }),
        });
    }
    let target = parts[0].to_ascii_lowercase().replace('-', "_");
    match target.as_str() {
        "none" => {
            if parts.len() != 1 {
                return Err("--notify none does not accept an extra value".into());
            }
            Ok(json!({ "kind": "none" }))
        }
        "current_thread" => {
            if parts.len() != 1 {
                return Err("--notify current-thread does not accept an extra value".into());
            }
            let thread_id = env_nonempty("GARYX_THREAD_ID")
                .ok_or("--notify current-thread requires GARYX_THREAD_ID")?;
            if !is_thread_key(&thread_id) {
                return Err(
                    format!("GARYX_THREAD_ID is not a canonical thread id: {thread_id}").into(),
                );
            }
            Ok(json!({ "kind": "thread", "thread_id": thread_id }))
        }
        "thread" => {
            if parts.len() != 2 {
                return Err("--notify thread requires exactly one thread id".into());
            }
            let thread_id = parts[1].trim();
            if !is_thread_key(thread_id) {
                return Err(
                    format!("notification thread id must be canonical: {thread_id}").into(),
                );
            }
            Ok(json!({ "kind": "thread", "thread_id": thread_id }))
        }
        "bot" => {
            if parts.len() != 2 {
                return Err("--notify bot requires <channel:account_id>".into());
            }
            let selector = parts[1].trim();
            let Some((channel, account_id)) = selector.split_once(':') else {
                return Err("--notify bot expects <channel:account_id>".into());
            };
            let channel = channel.trim();
            let account_id = account_id.trim();
            if channel.is_empty() || account_id.is_empty() {
                return Err("--notify bot expects non-empty channel and account id".into());
            }
            Ok(json!({
                "kind": "bot",
                "channel": channel,
                "account_id": account_id,
            }))
        }
        _ => Err(format!(
            "unknown --notify target: {}; use current-thread, thread, bot, or none",
            parts[0]
        )
        .into()),
    }
}

pub(crate) async fn cmd_task_stop(
    config_path: &str,
    task_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/stop", encode_task_id(task_id)?),
        &json!({}),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    if payload
        .get("interrupted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let aborted = payload
            .get("aborted_runs")
            .and_then(Value::as_array)
            .map(|runs| {
                runs.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "provider session".to_owned());
        println!("Stopped run: {aborted}");
    } else {
        println!("Stopped run: none active");
    }
    Ok(())
}

pub(crate) async fn cmd_task_delete(
    config_path: &str,
    task_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}", encode_task_id(task_id)?),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    let deleted_task_id = payload
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or(task_id);
    println!("Deleted task: {deleted_task_id}");
    if let Some(thread_id) = payload.get("thread_id").and_then(Value::as_str) {
        println!("Thread retained: {thread_id}");
        println!("Transcripts retained: yes");
    }
    if payload
        .get("interrupted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let aborted = payload
            .get("aborted_runs")
            .and_then(Value::as_array)
            .map(|runs| {
                runs.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "provider session".to_owned());
        println!("Stopped run: {aborted}");
    }
    Ok(())
}

pub(crate) async fn cmd_task_update(
    config_path: &str,
    task_id: &str,
    status: &str,
    note: Option<String>,
    force: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    patch_task_status(config_path, task_id, status, note, force, json_output).await
}

pub(crate) async fn cmd_task_reopen(
    config_path: &str,
    task_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    patch_task_status(config_path, task_id, "todo", None, false, json_output).await
}

pub(crate) async fn cmd_task_set_title(
    config_path: &str,
    task_id: &str,
    title: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let title = title.trim();
    if title.is_empty() {
        return Err("title cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/title", encode_task_id(task_id)?),
        &json!({
            "title": title,
        }),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_task_history(
    config_path: &str,
    task_id: &str,
    limit: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/tasks/{}/history?limit={}",
            encode_task_id(task_id)?,
            limit.clamp(1, 200)
        ),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    let events = payload["events"].as_array().cloned().unwrap_or_default();
    if events.is_empty() {
        println!("Events: (none)");
        return Ok(());
    }
    for event in events {
        let at = event["at"].as_str().unwrap_or("-");
        let actor = format_principal(&event["actor"]);
        let kind = event["kind"]["kind"]
            .as_str()
            .or_else(|| event["kind"]["type"].as_str())
            .unwrap_or("-");
        println!("- {at}  {actor}  {kind}");
    }
    Ok(())
}

async fn patch_task_status(
    config_path: &str,
    task_id: &str,
    status: &str,
    note: Option<String>,
    force: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let to = normalize_task_status(status)?;
    if let Some(message) = blocked_status_update(&gateway, task_id, &to, force).await? {
        eprintln!("{message}");
        std::process::exit(1);
    }
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/status", encode_task_id(task_id)?),
        &json!({
            "to": to,
            "note": note,
            "force": force,
        }),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

/// Decides whether a manual `garyx task update` should be refused at the CLI,
/// returning the guidance to print when it is. The CLI refuses two transitions
/// by default (see [`blocked_task_status_transition`]); `--force` is an explicit
/// override that skips the guard and lets the gateway apply the change.
///
/// The task's current status is only fetched when it could matter, so a forced
/// update, completing a task (`done`), or reopening it (`todo`) returns
/// `Ok(None)` without an extra request.
async fn blocked_status_update(
    gateway: &GatewayEndpoint,
    task_id: &str,
    to: &str,
    force: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if force || (to != "in_progress" && to != "in_review") {
        return Ok(None);
    }
    let current =
        fetch_gateway_json(gateway, &format!("/api/tasks/{}", encode_task_id(task_id)?)).await?;
    Ok(current_task_status(&current)
        .and_then(|from| blocked_task_status_transition(from, to, task_id)))
}

/// Reads a task's current status from a `GET /api/tasks/{id}` payload. The
/// status lives under `task.status`, with a top-level `status` fallback for
/// responses that flatten the task (matching [`print_task_summary`]).
fn current_task_status(value: &Value) -> Option<&str> {
    value
        .get("task")
        .and_then(|task| task.get("status"))
        .and_then(Value::as_str)
        .or_else(|| value.get("status").and_then(Value::as_str))
}

/// CLI-side guard for manual `garyx task update` status changes. Two
/// transitions are refused by default (the caller lets `--force` override):
///
/// - `in_review -> in_progress`: review only moves forward to `done`. To keep
///   working on a task under review, send it a message instead of reopening it.
/// - `in_progress -> in_review`: the system moves a task to review on its own
///   when the run ends, so it is not set manually.
///
/// Returns the user-facing guidance to print when the transition is blocked, or
/// `None` when it should be forwarded to the gateway. `from`/`to` are the
/// normalized status strings produced by [`normalize_task_status`].
fn blocked_task_status_transition(from: &str, to: &str, task_id: &str) -> Option<String> {
    match (from, to) {
        ("in_review", "in_progress") => Some(format!(
            "Refusing to move task {task_id} from In Review to In Progress.\n\
             In Review can only move to Done. To keep working on this task, send it a message:\n  \
             garyx thread send task '{task_id}' \"<your message>\""
        )),
        ("in_progress", "in_review") => Some(format!(
            "Refusing to move task {task_id} from In Progress to In Review.\n\
             A task moves to In Review automatically when its run ends; it cannot be set manually."
        )),
        _ => None,
    }
}

fn normalize_task_status(status: &str) -> Result<String, Box<dyn std::error::Error>> {
    let normalized = status.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "todo" | "to_do" | "open" => Ok("todo".to_owned()),
        "in_progress" | "progress" | "doing" => Ok("in_progress".to_owned()),
        "in_review" | "review" | "reviewing" => Ok("in_review".to_owned()),
        "done" | "complete" | "completed" | "closed" => Ok("done".to_owned()),
        _ => Err(format!("unknown task status: {status}").into()),
    }
}

pub(super) fn encode_task_id(task_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        return Err("task_id cannot be empty".into());
    }
    Ok(urlencoding::encode(task_id).into_owned())
}

fn print_task_summary(value: &Value) {
    let task = value.get("task").unwrap_or(value);
    let task_id = task_id_display(value, task);
    let thread_id = value
        .get("thread_id")
        .and_then(Value::as_str)
        .or_else(|| task.get("thread_id").and_then(Value::as_str))
        .unwrap_or("-");
    let title = task
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| value.get("title").and_then(Value::as_str))
        .unwrap_or("-");
    let status = task
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| value.get("status").and_then(Value::as_str))
        .unwrap_or("-");
    let unassigned = Value::Null;
    let assignee = task
        .get("assignee")
        .or_else(|| value.get("assignee"))
        .unwrap_or(&unassigned);
    println!("Task: {task_id}");
    println!("Title: {title}");
    println!("Status: {status}");
    println!("Assignee: {}", format_principal(assignee));
    if let Some(dispatch) = value.get("dispatch").filter(|dispatch| {
        dispatch
            .get("queued")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }) {
        let run_id = dispatch
            .get("run_id")
            .and_then(Value::as_str)
            .unwrap_or("-");
        println!("Dispatch: queued ({run_id})");
    }
    if thread_id != "-" {
        println!("Thread: {thread_id}");
    }
}

fn task_id_display(value: &Value, task: &Value) -> String {
    value
        .get("task_id")
        .and_then(Value::as_str)
        .or_else(|| task.get("task_id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("number")
                .and_then(Value::as_u64)
                .or_else(|| task.get("number").and_then(Value::as_u64))
                .map(|number| format!("#TASK-{number}"))
        })
        .unwrap_or_else(|| "-".to_owned())
}

#[derive(Debug, Clone)]
struct TaskProgressMessage {
    role: String,
    text: String,
    timestamp: Option<String>,
    sort_time: Option<DateTime<FixedOffset>>,
    source_order: usize,
    internal: bool,
}

#[derive(Debug, Clone)]
struct TaskProgressTurn {
    user_text: String,
    user_timestamp: Option<String>,
    internal: bool,
    assistant_text: Option<String>,
}

fn format_task_progress(task_payload: &Value, history_payload: Option<&Value>) -> String {
    let task = task_payload.get("task").unwrap_or(task_payload);
    let task_id = task_id_display(task_payload, task);
    let thread_id = task_payload
        .get("thread_id")
        .and_then(Value::as_str)
        .or_else(|| task.get("thread_id").and_then(Value::as_str))
        .unwrap_or("-");
    let title = task
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| task_payload.get("title").and_then(Value::as_str))
        .unwrap_or("-");
    let status = task
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| task_payload.get("status").and_then(Value::as_str))
        .unwrap_or("-");
    let unassigned = Value::Null;
    let assignee = task
        .get("assignee")
        .or_else(|| task_payload.get("assignee"))
        .unwrap_or(&unassigned);
    let updated_by = task
        .get("updated_by")
        .or_else(|| task_payload.get("updated_by"))
        .unwrap_or(&Value::Null);

    let mut output = String::new();
    let _ = writeln!(&mut output, "Task: {task_id}");
    let _ = writeln!(&mut output, "Title: {title}");
    let _ = writeln!(&mut output, "Status: {status}");
    let _ = writeln!(&mut output, "Assignee: {}", format_principal(assignee));
    let _ = writeln!(&mut output, "Updated by: {}", format_principal(updated_by));
    if thread_id != "-" {
        let _ = writeln!(&mut output, "Thread: {thread_id}");
    }
    output.push('\n');
    output.push_str("Progress:\n");

    let messages = task_progress_messages(history_payload);
    let turns = task_progress_turns(&messages);
    if turns.is_empty() {
        output.push_str("(no user messages recorded)\n");
    } else {
        for (idx, turn) in turns.iter().enumerate() {
            let _ = writeln!(
                &mut output,
                "\n[{}] User{}",
                idx + 1,
                turn_timestamp_label(turn)
            );
            if turn.internal {
                output.push_str("(internal dispatch)\n");
            }
            output.push_str(&indent_block(&turn.user_text, "  "));
            output.push('\n');
            output.push_str("Agent:\n");
            if let Some(text) = turn.assistant_text.as_deref() {
                output.push_str(&indent_block(text, "  "));
                output.push('\n');
            } else {
                output.push_str("  (no text reply yet)\n");
            }
        }
    }

    if thread_id != "-" {
        let _ = writeln!(
            &mut output,
            "\nFull thread with tool calls: garyx thread history {thread_id} --limit 200 --json"
        );
    }
    output
}

fn turn_timestamp_label(turn: &TaskProgressTurn) -> String {
    turn.user_timestamp
        .as_deref()
        .map(|timestamp| format!(" {}", format_local_timestamp(Some(timestamp))))
        .unwrap_or_default()
}

fn task_progress_messages(history_payload: Option<&Value>) -> Vec<TaskProgressMessage> {
    let mut messages = Vec::new();
    let mut seen = HashSet::new();
    let mut source_order = 0_usize;

    if let Some(history_messages) = history_payload
        .and_then(|payload| payload.get("messages"))
        .and_then(Value::as_array)
    {
        for message in history_messages {
            if let Some(entry) = task_progress_message_from_history(message, source_order) {
                push_unique_task_progress_message(&mut messages, &mut seen, entry);
                source_order += 1;
            }
        }
    }

    messages.sort_by(|left, right| {
        left.sort_time
            .cmp(&right.sort_time)
            .then_with(|| left.source_order.cmp(&right.source_order))
    });
    messages
}

fn push_unique_task_progress_message(
    messages: &mut Vec<TaskProgressMessage>,
    seen: &mut HashSet<String>,
    entry: TaskProgressMessage,
) {
    let key = format!(
        "{}\n{}\n{}",
        entry.role,
        entry.timestamp.as_deref().unwrap_or(""),
        entry.text
    );
    if seen.insert(key) {
        messages.push(entry);
    }
}

fn task_progress_message_from_history(
    value: &Value,
    source_order: usize,
) -> Option<TaskProgressMessage> {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/message/role").and_then(Value::as_str))?
        .trim()
        .to_ascii_lowercase();
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| value.get("message").and_then(message_text_from_value))
        .unwrap_or_default();
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/message/timestamp").and_then(Value::as_str))
        .map(ToOwned::to_owned);
    Some(TaskProgressMessage {
        role,
        text,
        sort_time: timestamp.as_deref().and_then(parse_rfc3339_timestamp),
        timestamp,
        source_order,
        internal: value
            .get("internal")
            .and_then(Value::as_bool)
            .or_else(|| value.pointer("/message/internal").and_then(Value::as_bool))
            .unwrap_or(false),
    })
}

fn task_progress_turns(messages: &[TaskProgressMessage]) -> Vec<TaskProgressTurn> {
    let mut turns = Vec::new();
    let mut current: Option<TaskProgressTurn> = None;
    let mut current_assistant_group = Vec::new();
    let mut last_assistant_group: Option<String> = None;

    for message in messages {
        match message.role.as_str() {
            "user" => {
                flush_assistant_group(&mut current_assistant_group, &mut last_assistant_group);
                if let Some(mut turn) = current.take() {
                    turn.assistant_text = last_assistant_group.take();
                    turns.push(turn);
                }
                current = Some(TaskProgressTurn {
                    user_text: message.text.clone(),
                    user_timestamp: message.timestamp.clone(),
                    internal: message.internal,
                    assistant_text: None,
                });
            }
            "assistant" => {
                if current.is_some() && !message.text.trim().is_empty() {
                    current_assistant_group.push(message.text.clone());
                }
            }
            _ => {
                flush_assistant_group(&mut current_assistant_group, &mut last_assistant_group);
            }
        }
    }
    flush_assistant_group(&mut current_assistant_group, &mut last_assistant_group);
    if let Some(mut turn) = current {
        turn.assistant_text = last_assistant_group;
        turns.push(turn);
    }
    turns
}

fn flush_assistant_group(group: &mut Vec<String>, last_group: &mut Option<String>) {
    if group.is_empty() {
        return;
    }
    *last_group = Some(group.join("\n\n"));
    group.clear();
}

fn parse_rfc3339_timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn message_text_from_value(value: &Value) -> Option<String> {
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            let mut parts = Vec::new();
            collect_message_text(value.get("content").unwrap_or(&Value::Null), &mut parts, 0);
            (!parts.is_empty()).then(|| parts.join("\n"))
        })
}

fn collect_message_text(value: &Value, parts: &mut Vec<String>, depth: usize) {
    if depth > 32 {
        return;
    }
    match value {
        Value::String(text) => push_message_text_part(parts, text),
        Value::Array(items) => {
            for item in items {
                collect_message_text(item, parts, depth + 1);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                push_message_text_part(parts, text);
            }
            if let Some(content) = map.get("content") {
                collect_message_text(content, parts, depth + 1);
            }
            if let Some(parts_value) = map.get("parts") {
                collect_message_text(parts_value, parts, depth + 1);
            }
            if let Some(items_value) = map.get("items") {
                collect_message_text(items_value, parts, depth + 1);
            }
        }
        _ => {}
    }
}

fn push_message_text_part(parts: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_owned());
    }
}

fn indent_block(text: &str, prefix: &str) -> String {
    text.lines()
        .flat_map(|line| {
            if line.is_empty() {
                vec![prefix.trim_end().to_owned()]
            } else {
                wrap_text_line(line, 100, prefix)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn wrap_text_line(line: &str, width: usize, prefix: &str) -> Vec<String> {
    if line.chars().count() <= width {
        return vec![format!("{prefix}{line}")];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in line.split_whitespace() {
        let next_len =
            current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
        if next_len > width && !current.is_empty() {
            lines.push(format!("{prefix}{current}"));
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(format!("{prefix}{current}"));
    }
    if lines.is_empty() {
        lines.push(format!("{prefix}{line}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use axum::{Json, Router, extract::Path as AxumPath, routing::get};
    use std::sync::{Arc as StdArc, Mutex};
    use tempfile::tempdir;
    use tokio::{net::TcpListener, task::JoinHandle};

    #[test]
    fn task_executor_payload_requires_a_delegation_target() {
        for agent in [None, Some(" \t".to_owned())] {
            let error = task_executor_payload(agent)
                .expect_err("task creation without an executor should fail");

            assert_eq!(
                error.to_string(),
                "Task creation is a delegation feature, so you must specify an Agent with --agent."
            );
        }
    }

    #[tokio::test]
    async fn cmd_task_create_posts_worktree_runtime_mode() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_task_create(
            config_path.to_str().expect("config path"),
            Some("Task worktree".to_owned()),
            Some("Do the work".to_owned()),
            Some("/tmp/garyx-repo".to_owned()),
            true,
            Some("claude".to_owned()),
            vec!["none".to_owned()],
            true,
        )
        .await
        .expect("task create should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/api/tasks");
        assert_eq!(records[0].body["title"], "Task worktree");
        assert_eq!(
            records[0].body["runtime"]["workspace_dir"],
            "/tmp/garyx-repo"
        );
        assert_eq!(records[0].body["runtime"]["workspace_mode"], "worktree");
    }

    #[tokio::test]
    async fn cmd_task_create_posts_agent_executor() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_task_create(
            config_path.to_str().expect("config path"),
            Some("Agent task".to_owned()),
            Some("Do the work".to_owned()),
            Some("/tmp/garyx-repo".to_owned()),
            false,
            Some("claude".to_owned()),
            vec!["none".to_owned()],
            true,
        )
        .await
        .expect("task create should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].body["executor"]["type"], "agent");
        assert_eq!(records[0].body["executor"]["agentId"], "claude");
        assert_eq!(records[0].body["assignee"], Value::Null);
        assert_eq!(records[0].body["start"], true);
        assert_eq!(
            records[0].body["runtime"]["workspace_dir"],
            "/tmp/garyx-repo"
        );
    }

    #[test]
    fn task_notification_target_accepts_bot_and_none() {
        assert_eq!(
            task_notification_target_payload(vec!["none".to_owned()]).unwrap(),
            json!({ "kind": "none" })
        );
        assert_eq!(
            task_notification_target_payload(vec!["bot".to_owned(), "telegram:main".to_owned()])
                .unwrap(),
            json!({ "kind": "bot", "channel": "telegram", "account_id": "main" })
        );
    }

    #[test]
    fn task_notification_target_resolves_current_thread_from_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _thread_id = ScopedEnvVar::set_string("GARYX_THREAD_ID", "thread::current");

        assert_eq!(
            task_notification_target_payload(vec!["current-thread".to_owned()]).unwrap(),
            json!({ "kind": "thread", "thread_id": "thread::current" })
        );
    }

    #[test]
    fn task_notification_target_defaults_to_current_thread_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _thread_id = ScopedEnvVar::set_string("GARYX_THREAD_ID", "thread::current");

        // No `--notify` given while running inside a thread defaults to that thread.
        assert_eq!(
            task_notification_target_payload(vec![]).unwrap(),
            json!({ "kind": "thread", "thread_id": "thread::current" })
        );
    }

    #[test]
    fn task_notification_target_defaults_to_none_outside_a_thread() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _thread_id = ScopedEnvVar::remove("GARYX_THREAD_ID");

        // No `--notify` and no surrounding thread (e.g. a plain terminal) stays silent.
        assert_eq!(
            task_notification_target_payload(vec![]).unwrap(),
            json!({ "kind": "none" })
        );
    }

    #[test]
    fn notification_targets_current_thread_matches_running_thread() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _thread_id = ScopedEnvVar::set_string("GARYX_THREAD_ID", "thread::current");

        assert!(notification_targets_current_thread(
            &json!({ "kind": "thread", "thread_id": "thread::current" })
        ));
        // A different thread, or a non-thread target, is not the current thread.
        assert!(!notification_targets_current_thread(
            &json!({ "kind": "thread", "thread_id": "thread::other" })
        ));
        assert!(!notification_targets_current_thread(
            &json!({ "kind": "none" })
        ));
        assert!(!notification_targets_current_thread(
            &json!({ "kind": "bot", "channel": "telegram", "account_id": "main" })
        ));
    }

    #[test]
    fn notification_targets_current_thread_false_without_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _thread_id = ScopedEnvVar::remove("GARYX_THREAD_ID");

        assert!(!notification_targets_current_thread(
            &json!({ "kind": "thread", "thread_id": "thread::current" })
        ));
    }

    #[test]
    fn task_source_payload_reads_runtime_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _thread_id = ScopedEnvVar::set_string("GARYX_THREAD_ID", "thread::current");
        let _task_id = ScopedEnvVar::set_string("GARYX_TASK_ID", "#TASK-7");
        let _bot_id = ScopedEnvVar::set_string("GARYX_BOT_ID", "telegram:main");
        let _channel = ScopedEnvVar::set_string("GARYX_CHANNEL", "telegram");
        let _account = ScopedEnvVar::set_string("GARYX_ACCOUNT_ID", "main");

        assert_eq!(
            task_source_payload_from_env().unwrap(),
            json!({
                "thread_id": "thread::current",
                "task_id": "#TASK-7",
                "task_thread_id": "thread::current",
                "bot_id": "telegram:main",
                "channel": "telegram",
                "account_id": "main",
            })
        );
    }

    #[test]
    fn task_id_display_falls_back_to_task_number() {
        let payload = json!({
            "task": {
                "number": 42,
                "title": "Fallback ref"
            }
        });

        assert_eq!(task_id_display(&payload, &payload["task"]), "#TASK-42");
    }

    #[test]
    fn format_task_progress_groups_each_user_turn_with_last_assistant_text_group() {
        let task_payload = json!({
            "task_id": "#TASK-42",
            "thread_id": "thread::task-42",
            "task": {
                "title": "Ship task progress",
                "status": "done",
                "assignee": {"kind": "agent", "agent_id": "claude"},
                "updated_by": {"kind": "agent", "agent_id": "claude"}
            },
            "thread": {
                "messages": [
                    {"role": "user", "content": "original request", "timestamp": "2026-05-03T00:00:00Z"}
                ]
            }
        });
        let history_payload = json!({
            "messages": [
                {
                    "role": "user",
                    "text": "please do it",
                    "timestamp": "2026-05-03T00:00:01Z",
                    "internal": false
                },
                {
                    "role": "assistant",
                    "text": "first text before tools",
                    "timestamp": "2026-05-03T00:00:02Z"
                },
                {
                    "role": "tool_use",
                    "text": "Bash",
                    "timestamp": "2026-05-03T00:00:03Z",
                    "tool_related": true
                },
                {
                    "role": "assistant",
                    "text": "final answer after tools",
                    "timestamp": "2026-05-03T00:00:04Z"
                },
                {
                    "role": "user",
                    "text": "follow up",
                    "timestamp": "2026-05-03T00:00:05Z",
                    "internal": true
                }
            ]
        });

        let rendered = format_task_progress(&task_payload, Some(&history_payload));

        // Human-facing timestamps render as local wall-clock time (see
        // format_local_timestamp); compute expectations through the same
        // helper so the assertions hold in any machine timezone.
        let local = |raw: &str| format_local_timestamp(Some(raw));
        assert!(rendered.contains("Task: #TASK-42"));
        // The transcript history is the only progress source: the record
        // `messages` supplement branch is gone (#TASK-1864 batch 1), so
        // the API thread payload's messages are ignored.
        assert!(
            !rendered.contains("original request"),
            "thread payload messages must not render: {rendered}"
        );
        assert!(rendered.contains(&format!("[1] User {}", local("2026-05-03T00:00:01Z"))));
        assert!(rendered.contains("please do it"));
        assert!(rendered.contains("final answer after tools"));
        assert!(
            !rendered.contains("first text before tools"),
            "only the last assistant text group after a user turn should render: {rendered}"
        );
        assert!(rendered.contains(&format!("[2] User {}", local("2026-05-03T00:00:05Z"))));
        assert!(rendered.contains("(internal dispatch)"));
        assert!(rendered.contains(
            "Full thread with tool calls: garyx thread history thread::task-42 --limit 200 --json"
        ));
    }

    #[test]
    fn task_progress_turns_keeps_last_consecutive_assistant_group() {
        let messages = vec![
            TaskProgressMessage {
                role: "user".to_owned(),
                text: "u1".to_owned(),
                timestamp: None,
                sort_time: None,
                source_order: 0,
                internal: false,
            },
            TaskProgressMessage {
                role: "assistant".to_owned(),
                text: "a1".to_owned(),
                timestamp: None,
                sort_time: None,
                source_order: 1,
                internal: false,
            },
            TaskProgressMessage {
                role: "assistant".to_owned(),
                text: "a2".to_owned(),
                timestamp: None,
                sort_time: None,
                source_order: 2,
                internal: false,
            },
        ];

        let turns = task_progress_turns(&messages);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].assistant_text.as_deref(), Some("a1\n\na2"));
    }

    #[test]
    fn blocked_transition_in_review_to_in_progress_points_to_send_message() {
        let message = blocked_task_status_transition("in_review", "in_progress", "#TASK-12")
            .expect("in_review -> in_progress must be blocked");
        assert!(message.contains("In Review"));
        assert!(message.contains("In Progress"));
        // The guidance must hand the user a copy-pasteable send-message command;
        // the task id is single-quoted so the shell does not treat the leading `#`
        // in canonical `#TASK-*` ids as a comment.
        assert!(message.contains("garyx thread send task '#TASK-12'"));
    }

    #[test]
    fn blocked_transition_in_progress_to_in_review_explains_it_is_automatic() {
        let message = blocked_task_status_transition("in_progress", "in_review", "#TASK-12")
            .expect("in_progress -> in_review must be blocked");
        assert!(message.contains("automatically"));
        assert!(message.contains("cannot be set manually"));
    }

    #[test]
    fn allowed_transitions_are_not_blocked() {
        // The one allowed move out of review, plus the ordinary start/stop/reopen
        // transitions, must all pass through to the gateway untouched.
        for (from, to) in [
            ("in_review", "done"),
            ("todo", "in_progress"),
            ("in_progress", "todo"),
            ("done", "todo"),
        ] {
            assert!(
                blocked_task_status_transition(from, to, "#TASK-1").is_none(),
                "{from} -> {to} should be allowed"
            );
        }
    }

    #[test]
    fn current_task_status_reads_nested_then_top_level() {
        assert_eq!(
            current_task_status(&json!({ "task": { "status": "in_review" } })),
            Some("in_review")
        );
        assert_eq!(
            current_task_status(&json!({ "status": "in_progress" })),
            Some("in_progress")
        );
        assert_eq!(
            current_task_status(&json!({ "thread_id": "thread::x" })),
            None
        );
    }

    /// Mock gateway serving `GET /api/tasks/{id}` with a fixed status and recording
    /// every lookup, so tests can assert both the decision and whether the status
    /// lookup was issued at all.
    async fn spawn_task_get_server(
        status: &'static str,
        requests: StdArc<Mutex<Vec<RecordedRequest>>>,
    ) -> (String, JoinHandle<()>) {
        let app = Router::new().route(
            "/api/tasks/{task_id}",
            get(move |AxumPath(task_id): AxumPath<String>| {
                let requests = requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path: format!("/api/tasks/{task_id}"),
                            body: Value::Null,
                        });
                    Json(json!({ "task": { "status": status } }))
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

    #[tokio::test]
    async fn blocked_status_update_refuses_review_to_progress_after_one_lookup() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_task_get_server("in_review", requests.clone()).await;
        let gateway = GatewayEndpoint {
            base_url,
            auth_token: None,
        };

        let blocked = blocked_status_update(&gateway, "#TASK-7", "in_progress", false)
            .await
            .expect("status lookup should succeed");

        handle.abort();
        let message = blocked.expect("in_review -> in_progress must be blocked");
        assert!(message.contains("garyx thread send task '#TASK-7'"));
        assert_eq!(
            requests.lock().expect("request lock").len(),
            1,
            "should look up current status exactly once"
        );
    }

    #[tokio::test]
    async fn blocked_status_update_refuses_progress_to_review() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_task_get_server("in_progress", requests.clone()).await;
        let gateway = GatewayEndpoint {
            base_url,
            auth_token: None,
        };

        let blocked = blocked_status_update(&gateway, "#TASK-7", "in_review", false)
            .await
            .expect("status lookup should succeed");

        handle.abort();
        assert!(
            blocked
                .expect("in_progress -> in_review must be blocked")
                .contains("automatically")
        );
    }

    #[tokio::test]
    async fn blocked_status_update_allows_todo_to_progress() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_task_get_server("todo", requests.clone()).await;
        let gateway = GatewayEndpoint {
            base_url,
            auth_token: None,
        };

        let blocked = blocked_status_update(&gateway, "#TASK-7", "in_progress", false)
            .await
            .expect("status lookup should succeed");

        handle.abort();
        assert!(blocked.is_none(), "starting a todo task must be allowed");
        assert_eq!(requests.lock().expect("request lock").len(), 1);
    }

    #[tokio::test]
    async fn blocked_status_update_skips_lookup_when_completing() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_task_get_server("in_review", requests.clone()).await;
        let gateway = GatewayEndpoint {
            base_url,
            auth_token: None,
        };

        // Completing a reviewed task is the allowed move and must not be gated, so
        // it should never issue the current-status lookup.
        let blocked = blocked_status_update(&gateway, "#TASK-7", "done", false)
            .await
            .expect("done update should not error");

        handle.abort();
        assert!(blocked.is_none());
        assert!(
            requests.lock().expect("request lock").is_empty(),
            "completing a task should not look up current status"
        );
    }

    #[tokio::test]
    async fn blocked_status_update_force_overrides_guard_without_lookup() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_task_get_server("in_review", requests.clone()).await;
        let gateway = GatewayEndpoint {
            base_url,
            auth_token: None,
        };

        // --force is an explicit override: even the otherwise-blocked
        // in_review -> in_progress move is allowed through, and the guard does not
        // even look up the current status.
        let blocked = blocked_status_update(&gateway, "#TASK-7", "in_progress", true)
            .await
            .expect("forced update should not error");

        handle.abort();
        assert!(blocked.is_none(), "--force must override the guard");
        assert!(
            requests.lock().expect("request lock").is_empty(),
            "a forced update should not look up current status"
        );
    }
}
