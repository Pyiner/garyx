use super::*;

use garyx_models::config::SlashCommand;
use tempfile::tempdir;

use crate::server::AppStateBuilder;

fn config_with_command(name: &str) -> GaryxConfig {
    let mut config = GaryxConfig::default();
    config.commands.push(SlashCommand {
        name: name.to_owned(),
        description: format!("{name} description"),
        prompt: Some(format!("run {name}")),
        skill_id: None,
    });
    config
}

fn test_server_config() -> McpServerConfig {
    McpServerConfig {
        command: "npx".to_owned(),
        args: vec!["@modelcontextprotocol/server-filesystem".to_owned()],
        env: HashMap::new(),
        working_dir: Some("/tmp".to_owned()),
        ..Default::default()
    }
}

#[test]
fn sync_codex_config_toml_recovers_invalid_file() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("config.toml");
    fs::write(&path, "d\"\n").unwrap();

    let servers = HashMap::from([("filesystem".to_owned(), test_server_config())]);
    sync_codex_config_toml(&path, &HashSet::new(), &servers).unwrap();

    let raw = fs::read_to_string(&path).unwrap();
    let parsed: toml::Value = toml::from_str(&raw).unwrap();
    assert_eq!(
        parsed["mcp_servers"]["filesystem"]["command"].as_str(),
        Some("npx")
    );
    assert_eq!(
        parsed["mcp_servers"]["filesystem"]["cwd"].as_str(),
        Some("/tmp")
    );

    let backup_paths = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| {
            entry
                .file_name()
                .and_then(|value| value.to_str())
                .map(|name| name.starts_with("config.toml.invalid-") && name.ends_with(".bak"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    assert_eq!(backup_paths.len(), 1);
    assert_eq!(fs::read_to_string(&backup_paths[0]).unwrap(), "d\"\n");
}

#[test]
fn sync_claude_mcp_json_recovers_invalid_file() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("mcp.json");
    fs::write(&path, "{ broken json").unwrap();

    let servers = HashMap::from([("filesystem".to_owned(), test_server_config())]);
    sync_claude_mcp_json(&path, &HashSet::new(), &servers).unwrap();

    let raw = fs::read_to_string(&path).unwrap();
    let parsed: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        parsed["mcpServers"]["filesystem"]["command"].as_str(),
        Some("npx")
    );

    let backup_paths = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| {
            entry
                .file_name()
                .and_then(|value| value.to_str())
                .map(|name| name.starts_with("mcp.json.invalid-") && name.ends_with(".bak"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    assert_eq!(backup_paths.len(), 1);
    assert_eq!(
        fs::read_to_string(&backup_paths[0]).unwrap(),
        "{ broken json"
    );
}

#[tokio::test]
async fn rolls_back_runtime_and_disk_when_external_sync_fails() {
    let temp = tempdir().unwrap();
    let config_path = temp.path().join("gary.json");
    let initial = GaryxConfig::default();
    persist_config_file(config_path.clone(), &initial)
        .await
        .unwrap();

    let state = AppStateBuilder::new(initial.clone())
        .with_config_path(config_path.clone())
        .build();
    let next = config_with_command("sync-fail");

    let error = persist_and_apply_config_with_sync(&state, &next, |_| {
        Box::pin(async { Err("simulated external sync failure".to_owned()) })
    })
    .await
    .unwrap_err();

    assert!(error.contains("simulated external sync failure"));
    assert!(state.config_snapshot().commands.is_empty());

    let persisted: GaryxConfig = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert!(persisted.commands.is_empty());
}

#[tokio::test]
async fn concurrent_config_writes_do_not_lose_updates() {
    // Regression: every config-mutating handler does a whole-document
    // read-modify-write (clone snapshot -> edit -> apply+persist). If those
    // writes are not serialized, concurrent writers all read the same base
    // snapshot and the last one to persist silently clobbers the others.
    //
    // On the buggy (unserialized) path every request still returns CREATED —
    // the loss is silent — but only one server survives in the committed
    // config. Under a serialized mutator all N survive.
    let temp = tempdir().unwrap();
    let config_path = temp.path().join("gary.json");
    let initial = GaryxConfig::default();
    persist_config_file(config_path.clone(), &initial)
        .await
        .unwrap();

    let state = AppStateBuilder::new(initial)
        .with_config_path(config_path.clone())
        .build();

    const N: usize = 16;
    let barrier = Arc::new(tokio::sync::Barrier::new(N));
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let state = state.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            let body: UpsertMcpServerBody = serde_json::from_value(json!({
                "name": format!("server-{i}"),
                "transport": "stdio",
                "command": "npx",
                "args": [format!("pkg-{i}")],
            }))
            .expect("body deserializes");
            // Line all writers up so their read-modify-write windows overlap.
            barrier.wait().await;
            create_mcp_server(State(state), Json(body))
                .await
                .into_response()
                .status()
        }));
    }

    let mut created = 0usize;
    for handle in handles {
        if handle.await.unwrap() == StatusCode::CREATED {
            created += 1;
        }
    }
    assert_eq!(created, N, "every create request should report success");

    let live = state.config_snapshot();
    let survived: Vec<usize> = (0..N)
        .filter(|i| live.mcp_servers.contains_key(&format!("server-{i}")))
        .collect();
    assert_eq!(
        survived.len(),
        N,
        "concurrent creates lost updates: only {} of {N} servers survived in live config: {survived:?}",
        survived.len(),
    );

    let persisted: GaryxConfig = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        persisted.mcp_servers.len(),
        N,
        "concurrent creates lost updates on disk: {} of {N} servers persisted",
        persisted.mcp_servers.len(),
    );
}

#[tokio::test]
async fn rolls_back_runtime_when_persisting_config_fails() {
    let temp = tempdir().unwrap();
    let config_path = temp.path().join("gary.json");
    std::fs::create_dir_all(&config_path).unwrap();

    let initial = GaryxConfig::default();
    let state = AppStateBuilder::new(initial.clone())
        .with_config_path(config_path)
        .build();
    let next = config_with_command("persist-fail");

    let error = persist_and_apply_config_with_sync(&state, &next, |_| Box::pin(async { Ok(()) }))
        .await
        .unwrap_err();

    assert!(error.contains("failed to persist config file"));
    assert!(state.config_snapshot().commands.is_empty());
}
