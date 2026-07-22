//! Small JSON command adapter used by the cross-language SessionStore
//! conformance gate. It intentionally exposes no production CLI surface.

use std::path::PathBuf;

use claude_agent_sdk::{LocalDirectorySessionStore, SessionKey, SessionStore, session_project_key};
use serde_json::{Value, json};

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string field {key}"))
}

fn decode_key(value: &Value) -> Result<SessionKey, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
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
    let root = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "usage: session_store_contract <root> <operation-json>".to_owned())?;
    let raw = args
        .next()
        .ok_or_else(|| "missing operation JSON".to_owned())?;
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_owned());
    }
    let operation: Value = serde_json::from_str(&raw.to_string_lossy())
        .map_err(|error| format!("invalid operation JSON: {error}"))?;
    let kind = required_string(&operation, "op")?;
    let store = LocalDirectorySessionStore::new(root);

    let output = match kind {
        "append" => {
            let key = decode_key(
                operation
                    .get("key")
                    .ok_or_else(|| "missing key".to_owned())?,
            )?;
            let entries = operation
                .get("entries")
                .and_then(Value::as_array)
                .ok_or_else(|| "missing entries".to_owned())?;
            store
                .append(&key, entries)
                .await
                .map_err(|error| error.to_string())?;
            Value::Null
        }
        "load" => {
            let key = decode_key(
                operation
                    .get("key")
                    .ok_or_else(|| "missing key".to_owned())?,
            )?;
            serde_json::to_value(store.load(&key).await.map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?
        }
        "listSessions" => {
            let project_key = required_string(&operation, "projectKey")?;
            serde_json::to_value(
                store
                    .list_sessions(project_key)
                    .await
                    .map_err(|error| error.to_string())?
                    .unwrap_or_default(),
            )
            .map_err(|error| error.to_string())?
        }
        "delete" => {
            let key = decode_key(
                operation
                    .get("key")
                    .ok_or_else(|| "missing key".to_owned())?,
            )?;
            store
                .delete(&key)
                .await
                .map_err(|error| error.to_string())?;
            Value::Null
        }
        "listSubkeys" => {
            let key = decode_key(
                operation
                    .get("key")
                    .ok_or_else(|| "missing key".to_owned())?,
            )?;
            serde_json::to_value(
                store
                    .list_subkeys(&key)
                    .await
                    .map_err(|error| error.to_string())?
                    .unwrap_or_default(),
            )
            .map_err(|error| error.to_string())?
        }
        "projectKey" => json!(session_project_key(PathBuf::from(required_string(
            &operation, "path"
        )?))),
        other => return Err(format!("unsupported operation {other}")),
    };

    println!(
        "{}",
        serde_json::to_string(&output).map_err(|error| error.to_string())?
    );
    Ok(())
}
