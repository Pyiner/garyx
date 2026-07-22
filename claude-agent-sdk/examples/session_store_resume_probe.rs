//! Rust half of the official-TS-vs-Rust fake CLI resume differential.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use claude_agent_sdk::{
    ClaudeAgentOptions, LocalDirectorySessionStore, Message, OutboundUserMessage, SessionKey,
    SessionStore, run_streaming, session_project_key,
};
use serde_json::{Value, json};

const SESSION_ID: &str = "11111111-2222-4333-8444-555555555555";
const ENTRY_UUID: &str = "72f50f98-c34b-4e4e-a586-58179c5536f1";

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
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_owned());
    }

    let cwd = scratch.join("workspace");
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
    let subkey = SessionKey {
        project_key: project_key.clone(),
        session_id: SESSION_ID.to_owned(),
        subpath: Some("subagents/agent-probe".to_owned()),
    };
    let store = Arc::new(LocalDirectorySessionStore::new(&store_root));
    store
        .append(
            &main_key,
            &[json!({
                "type": "user",
                "uuid": "0d02bd0d-f6cf-4f87-81c6-849acac8712b",
                "sessionId": SESSION_ID,
                "message": {"role":"user","content":"original turn"}
            })],
        )
        .await
        .map_err(|error| error.to_string())?;
    store
        .append(
            &subkey,
            &[
                json!({"type":"assistant","uuid":"63f73d62-5ca2-409a-96fe-bf3b36f1ba31"}),
                json!({"type":"agent_metadata","toolUseId":"tool-probe"}),
            ],
        )
        .await
        .map_err(|error| error.to_string())?;

    let store_trait: Arc<dyn SessionStore> = store.clone();
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
            ("PROBE_ENTRY_UUID".to_owned(), ENTRY_UUID.to_owned()),
        ]),
        session_store: Some(store_trait),
        ..ClaudeAgentOptions::default()
    })
    .await
    .map_err(|error| error.to_string())?;
    run.control()
        .send_user_message(OutboundUserMessage::text("continue", ""))
        .await
        .map_err(|error| error.to_string())?;

    let result = loop {
        match run.next_message().await {
            Some(Ok(Message::Result(result))) => break result,
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error.to_string()),
            None => return Err("fake CLI ended without a result".to_owned()),
        }
    };
    run.finish().await.map_err(|error| error.to_string())?;
    let entries = store
        .load(&main_key)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "canonical transcript disappeared".to_owned())?;
    let result_value: Value = serde_json::from_str(
        result
            .result
            .as_deref()
            .ok_or_else(|| "fake result was empty".to_owned())?,
    )
    .map_err(|error| error.to_string())?;

    println!(
        "{}",
        serde_json::to_string(&json!({
            "result": result_value,
            "entries": entries,
        }))
        .map_err(|error| error.to_string())?
    );
    Ok(())
}
