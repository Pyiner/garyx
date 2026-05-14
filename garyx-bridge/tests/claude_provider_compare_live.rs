use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use garyx_bridge::claude_provider::ClaudeCliProvider;
use garyx_bridge::claude_tty_provider::ClaudeTtyProvider;
use garyx_bridge::provider_trait::AgentLoopProvider;
use garyx_models::provider::{ClaudeCodeConfig, ProviderRunOptions, StreamEvent};
use serde_json::{Value, json};

const MARKER: &str = "GARYX_PROVIDER_COMPARE_OK";
const EXPECTED_SCORE: i64 = 43;

#[derive(Debug)]
struct CompareRun {
    label: &'static str,
    result_success: bool,
    response: String,
    sdk_session_id: Option<String>,
    actual_model: Option<String>,
    thread_title: Option<String>,
    event_summary: EventSummary,
    transcript_summary: Option<TranscriptSummary>,
}

#[derive(Default, Debug)]
struct EventSummary {
    session_bound: usize,
    user_ack: usize,
    delta: usize,
    tool_use: usize,
    tool_result: usize,
    title_updated: usize,
    done: usize,
    delta_text: String,
}

#[derive(Debug)]
struct TranscriptSummary {
    line_count: usize,
    user_lines: usize,
    assistant_lines: usize,
    tool_use_seen: bool,
    tool_result_seen: bool,
    marker_seen: bool,
}

#[tokio::test]
#[ignore = "requires a logged-in local Claude CLI and makes two real Claude requests"]
async fn live_compare_claude_sdk_and_tty_on_same_task() {
    if !binary_available("claude").await {
        eprintln!("claude not found, skipping");
        return;
    }

    let workspace = tempfile::tempdir().expect("temp workspace");
    prepare_workspace(workspace.path()).expect("prepare workspace");
    let workspace_dir = workspace.path().to_string_lossy().into_owned();
    let task = compare_task_prompt();

    let sdk_workspace = workspace_dir.clone();
    let sdk_task = task.clone();
    let sdk_run = tokio::spawn(async move {
        let config = live_compare_config();
        let provider = ClaudeCliProvider::new(config);
        run_provider("claude_sdk", provider, sdk_workspace, sdk_task).await
    });

    let tty_workspace = workspace_dir.clone();
    let tty_task = task.clone();
    let tty_run = tokio::spawn(async move {
        let config = live_compare_config();
        let provider = ClaudeTtyProvider::new(config);
        run_provider("claude_tty", provider, tty_workspace, tty_task).await
    });

    let (sdk, tty) = tokio::join!(sdk_run, tty_run);
    let sdk = sdk.expect("sdk task join").expect("sdk provider run");
    let tty = tty.expect("tty task join").expect("tty provider run");

    print_compare_run(&sdk);
    print_compare_run(&tty);

    assert_compare_run(&sdk);
    assert_compare_run(&tty);
}

async fn run_provider<P>(
    label: &'static str,
    mut provider: P,
    workspace_dir: String,
    task: String,
) -> Result<CompareRun, String>
where
    P: AgentLoopProvider + 'static,
{
    provider
        .initialize()
        .await
        .map_err(|error| format!("{label} initialize: {error}"))?;

    let thread_id = format!("thread::garyx-compare-{label}");
    let bot_id = format!("compare:{label}");
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(180),
        provider.run_streaming(
            &ProviderRunOptions {
                thread_id: thread_id.clone(),
                message: task,
                workspace_dir: Some(workspace_dir.clone()),
                images: None,
                metadata: HashMap::from([
                    (
                        "client_run_id".to_owned(),
                        json!(format!("compare-{label}")),
                    ),
                    (
                        "runtime_context".to_owned(),
                        json!({
                            "thread_id": thread_id,
                            "bot_id": bot_id,
                            "workspace_dir": workspace_dir,
                            "channel": "compare-live",
                            "account_id": "synthetic",
                        }),
                    ),
                ]),
            },
            Box::new(move |event| {
                captured.lock().expect("events lock").push(event);
            }),
        ),
    )
    .await
    .map_err(|_| format!("{label} timed out"))?
    .map_err(|error| format!("{label} run_streaming: {error}"))?;

    let sdk_session_id = result.sdk_session_id.clone();
    let transcript_summary = sdk_session_id
        .as_deref()
        .and_then(find_claude_transcript_by_session_id)
        .and_then(|path| summarize_transcript(&path).ok());
    let event_summary = summarize_events(&events.lock().expect("events lock"));

    provider
        .shutdown()
        .await
        .map_err(|error| format!("{label} shutdown: {error}"))?;

    Ok(CompareRun {
        label,
        result_success: result.success,
        response: result.response,
        sdk_session_id,
        actual_model: result.actual_model,
        thread_title: result.thread_title,
        event_summary,
        transcript_summary,
    })
}

async fn binary_available(name: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn live_compare_config() -> ClaudeCodeConfig {
    ClaudeCodeConfig {
        permission_mode: "bypassPermissions".to_owned(),
        timeout_seconds: 180.0,
        max_retries: 1,
        mcp_base_url: String::new(),
        ..Default::default()
    }
}

fn prepare_workspace(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path.join("notes"))?;
    fs::write(
        path.join("facts.json"),
        r#"{"alpha":6,"beta":7,"delta":5,"phrase":"kiwi apple mango apple"}"#,
    )?;
    fs::write(
        path.join("notes/rules.md"),
        "Use penalty = 4. Sort unique words from facts.phrase alphabetically.\n",
    )
}

fn compare_task_prompt() -> String {
    format!(
        "You are validating two Garyx Claude providers on the same task.\n\
         Use only the local workspace files; do not use network.\n\
         Read facts.json and notes/rules.md.\n\
         Return compact JSON only, no Markdown.\n\
         Required keys:\n\
         marker: \"{MARKER}\"\n\
         thread_id: the thread_id from <garyx_thread_metadata>\n\
         bot_id: the bot_id from <garyx_thread_metadata>\n\
         workspace_basename: the basename of the current workspace directory\n\
         score: alpha * beta + delta - penalty\n\
         sorted_words: sorted unique lowercase words from facts.phrase\n\
         files_checked: [\"facts.json\", \"notes/rules.md\"]\n\
         confidence: \"high\""
    )
}

fn summarize_events(events: &[StreamEvent]) -> EventSummary {
    let mut summary = EventSummary::default();
    for event in events {
        match event {
            StreamEvent::SessionBound { .. } => summary.session_bound += 1,
            StreamEvent::Delta { text } => {
                summary.delta += 1;
                summary.delta_text.push_str(text);
            }
            StreamEvent::ToolUse { .. } => summary.tool_use += 1,
            StreamEvent::ToolResult { .. } => summary.tool_result += 1,
            StreamEvent::Boundary { .. } => summary.user_ack += 1,
            StreamEvent::ThreadTitleUpdated { .. } => summary.title_updated += 1,
            StreamEvent::Done => summary.done += 1,
        }
    }
    summary
}

fn find_claude_transcript_by_session_id(session_id: &str) -> Option<PathBuf> {
    let projects = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join(".claude")
        .join("projects");
    find_file_named(&projects, &format!("{session_id}.jsonl"))
}

fn find_file_named(root: &Path, file_name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, file_name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
            return Some(path);
        }
    }
    None
}

fn summarize_transcript(path: &Path) -> std::io::Result<TranscriptSummary> {
    let content = fs::read_to_string(path)?;
    let mut summary = TranscriptSummary {
        line_count: 0,
        user_lines: 0,
        assistant_lines: 0,
        tool_use_seen: false,
        tool_result_seen: false,
        marker_seen: false,
    };
    for line in content.lines() {
        summary.line_count += 1;
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                summary.user_lines += 1;
                if line.contains("tool_result") {
                    summary.tool_result_seen = true;
                }
            }
            Some("assistant") => {
                summary.assistant_lines += 1;
                if line.contains("\"tool_use\"") {
                    summary.tool_use_seen = true;
                }
                if line.contains(MARKER) {
                    summary.marker_seen = true;
                }
            }
            _ => {}
        }
    }
    Ok(summary)
}

fn assert_compare_run(run: &CompareRun) {
    assert!(run.result_success, "{} result was unsuccessful", run.label);
    assert!(
        run.response.contains(MARKER),
        "{} missing marker in response: {}",
        run.label,
        run.response
    );
    assert!(
        run.response.contains(&EXPECTED_SCORE.to_string()),
        "{} missing expected score in response: {}",
        run.label,
        run.response
    );
    assert!(
        run.response
            .contains(&format!("thread::garyx-compare-{}", run.label)),
        "{} did not preserve thread metadata: {}",
        run.label,
        run.response
    );
    assert!(
        run.response.contains(&format!("compare:{}", run.label)),
        "{} did not preserve bot metadata: {}",
        run.label,
        run.response
    );
    assert!(
        run.event_summary.session_bound > 0,
        "{} no session",
        run.label
    );
    assert!(run.event_summary.user_ack > 0, "{} no user ack", run.label);
    assert!(run.event_summary.delta > 0, "{} no delta", run.label);
    assert_eq!(run.event_summary.done, 1, "{} no done event", run.label);
    let transcript = run
        .transcript_summary
        .as_ref()
        .unwrap_or_else(|| panic!("{} missing Claude transcript", run.label));
    assert!(
        transcript.marker_seen,
        "{} transcript missing marker: {:?}",
        run.label, transcript
    );
}

fn print_compare_run(run: &CompareRun) {
    eprintln!(
        "[compare:{}] success={} session={} model={} title={} events=session:{} ack:{} delta:{} tool_use:{} tool_result:{} title:{} done:{} transcript={:?} response={}",
        run.label,
        run.result_success,
        run.sdk_session_id.as_deref().unwrap_or("<none>"),
        run.actual_model.as_deref().unwrap_or("<none>"),
        run.thread_title.as_deref().unwrap_or("<none>"),
        run.event_summary.session_bound,
        run.event_summary.user_ack,
        run.event_summary.delta,
        run.event_summary.tool_use,
        run.event_summary.tool_result,
        run.event_summary.title_updated,
        run.event_summary.done,
        run.transcript_summary,
        run.response
    );
}
