//! Rust half of transcript-mirror batching differentials against the official
//! TypeScript SDK. This is intentionally an example binary so the Bun oracle
//! can exercise the public Rust SDK through a real fake-CLI process.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use claude_agent_sdk::{
    ClaudeAgentOptions, LocalDirectorySessionStore, Message, OutboundUserMessage, SessionKey,
    SessionStore, SessionStoreEntry, SessionStoreFlush, SessionStoreSession, run_streaming,
    session_project_key,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};

const SESSION_ID: &str = "11111111-2222-4333-8444-555555555555";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppendTrace {
    key: SessionKey,
    entry_count: usize,
    first_marker: Option<Value>,
    last_marker: Option<Value>,
    outcome: &'static str,
}

struct RecordingStore {
    inner: LocalDirectorySessionStore,
    failures_remaining: AtomicUsize,
    calls: Mutex<Vec<AppendTrace>>,
    background: Option<Arc<BackgroundState>>,
}

struct BackgroundState {
    events: Mutex<Vec<&'static str>>,
    released: watch::Sender<bool>,
    watchdog_used: AtomicBool,
}

impl RecordingStore {
    fn marker(entry: Option<&Value>) -> Option<Value> {
        entry.and_then(|entry| entry.get("marker").cloned())
    }
}

#[async_trait]
impl SessionStore for RecordingStore {
    async fn append(
        &self,
        key: &SessionKey,
        entries: &[SessionStoreEntry],
    ) -> claude_agent_sdk::Result<()> {
        let failed = self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok();
        self.calls.lock().await.push(AppendTrace {
            key: key.clone(),
            entry_count: entries.len(),
            first_marker: Self::marker(entries.first()),
            last_marker: Self::marker(entries.last()),
            outcome: if failed { "error" } else { "ok" },
        });
        if let Some(background) = self.background.as_ref() {
            background.events.lock().await.push("append-start");
            let mut released = background.released.subscribe();
            if !*released.borrow()
                && tokio::time::timeout(Duration::from_millis(500), released.changed())
                    .await
                    .is_err()
            {
                background.watchdog_used.store(true, Ordering::SeqCst);
                background.events.lock().await.push("watchdog");
            }
            background.events.lock().await.push("append-end");
        }
        if failed {
            return Err(claude_agent_sdk::ClaudeSDKError::SessionStore(
                "intentional parity probe failure".to_owned(),
            ));
        }
        self.inner.append(key, entries).await
    }

    async fn load(
        &self,
        key: &SessionKey,
    ) -> claude_agent_sdk::Result<Option<Vec<SessionStoreEntry>>> {
        self.inner.load(key).await
    }

    async fn list_sessions(
        &self,
        project_key: &str,
    ) -> claude_agent_sdk::Result<Option<Vec<SessionStoreSession>>> {
        self.inner.list_sessions(project_key).await
    }

    async fn delete(&self, key: &SessionKey) -> claude_agent_sdk::Result<bool> {
        self.inner.delete(key).await
    }

    async fn list_subkeys(
        &self,
        key: &SessionKey,
    ) -> claude_agent_sdk::Result<Option<Vec<String>>> {
        self.inner.list_subkeys(key).await
    }

    fn native_projects_root(&self) -> Option<&Path> {
        Some(self.inner.root())
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let scratch = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "missing scratch directory".to_owned())?;
    let node = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "missing node executable".to_owned())?;
    let fake_cli = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "missing fake CLI path".to_owned())?;
    let scenario = args
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(|| "missing probe scenario".to_owned())?;
    let cwd = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "missing shared workspace directory".to_owned())?;
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_owned());
    }

    let store_root = scratch.join("canonical-projects");
    let profile = scratch.join("selected-profile");
    tokio::fs::create_dir_all(&cwd)
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::create_dir_all(&profile)
        .await
        .map_err(|error| error.to_string())?;

    let project_key = session_project_key(&cwd);
    let main_key = SessionKey::main(&project_key, SESSION_ID);
    let inner = LocalDirectorySessionStore::new(&store_root);
    inner
        .append(&main_key, &[json!({"type":"seed"})])
        .await
        .map_err(|error| error.to_string())?;
    let background = (scenario == "eager-background").then(|| {
        let (released, _receiver) = watch::channel(false);
        Arc::new(BackgroundState {
            events: Mutex::new(Vec::new()),
            released,
            watchdog_used: AtomicBool::new(false),
        })
    });
    let store = Arc::new(RecordingStore {
        inner,
        failures_remaining: AtomicUsize::new(match scenario.as_str() {
            "retry" => 2,
            "failure" => 3,
            _ => 0,
        }),
        calls: Mutex::new(Vec::new()),
        background: background.clone(),
    });
    let store_trait: Arc<dyn SessionStore> = store.clone();
    let flush = if scenario.starts_with("eager") {
        SessionStoreFlush::Eager
    } else {
        SessionStoreFlush::Batched
    };

    let mut run = run_streaming(ClaudeAgentOptions {
        resume: Some(SESSION_ID.to_owned()),
        cwd: Some(cwd),
        cli_path: Some(node),
        cli_prefix_args: vec![fake_cli.to_string_lossy().into_owned()],
        env: HashMap::from([
            (
                "CLAUDE_CONFIG_DIR".to_owned(),
                profile.to_string_lossy().into_owned(),
            ),
            ("PROBE_SCENARIO".to_owned(), scenario.clone()),
        ]),
        session_store: Some(store_trait),
        session_store_flush: flush,
        ..ClaudeAgentOptions::default()
    })
    .await
    .map_err(|error| error.to_string())?;
    run.control()
        .send_user_message(OutboundUserMessage::text("continue", ""))
        .await
        .map_err(|error| error.to_string())?;

    let mut mirror_errors = 0;
    let result = loop {
        match run.next_message().await {
            Some(Ok(Message::Result(result))) => break result,
            Some(Ok(Message::Assistant(_))) if background.is_some() => {
                let background = background.as_ref().unwrap();
                background.events.lock().await.push("assistant");
                background.released.send_replace(true);
            }
            Some(Ok(Message::System(message))) if message.subtype == "mirror_error" => {
                mirror_errors += 1;
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error.to_string()),
            None => return Err("fake CLI ended without a result".to_owned()),
        }
    };
    run.finish().await.map_err(|error| error.to_string())?;
    let result_value: Value = serde_json::from_str(
        result
            .result
            .as_deref()
            .ok_or_else(|| "fake result was empty".to_owned())?,
    )
    .map_err(|error| error.to_string())?;
    let events = if let Some(background) = background.as_ref() {
        background.events.lock().await.clone()
    } else {
        Vec::new()
    };
    let watchdog_used = background
        .as_ref()
        .is_some_and(|background| background.watchdog_used.load(Ordering::SeqCst));

    println!(
        "{}",
        serde_json::to_string(&json!({
            "result": result_value,
            "calls": &*store.calls.lock().await,
            "mirrorErrors": mirror_errors,
            "events": events,
            "watchdogUsed": watchdog_used,
        }))
        .map_err(|error| error.to_string())?
    );
    Ok(())
}
