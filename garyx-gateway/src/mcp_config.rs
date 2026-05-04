use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::IntoResponse,
};
use futures_util::future::BoxFuture;
use garyx_models::config::{GaryxConfig, McpServerConfig, McpTransport};
use garyx_models::config_loader::{ConfigWriteOptions, write_config_value_atomic};
use garyx_models::local_paths::default_mcp_sync_state_path;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::server::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpsertMcpServerBody {
    pub name: String,
    #[serde(default)]
    pub transport: Option<String>,

    // STDIO fields
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default, alias = "workingDir")]
    pub working_dir: Option<String>,

    // Streamable HTTP fields
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, alias = "bearerTokenEnv")]
    pub bearer_token_env: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,

    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct ToggleMcpServerBody {
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct ManagedMcpSyncState {
    #[serde(default)]
    managed_names: Vec<String>,
}

fn default_true() -> bool {
    garyx_models::config::default_true()
}

fn home_dir() -> Option<PathBuf> {
    garyx_models::local_paths::home_dir()
}

fn managed_sync_state_path(home: &Path) -> PathBuf {
    let _ = home;
    default_mcp_sync_state_path()
}

fn invalid_backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config");
    path.with_file_name(format!(
        "{file_name}.invalid-{}.bak",
        chrono::Utc::now().format("%Y%m%d%H%M%S%3f")
    ))
}

fn backup_invalid_file(path: &Path) -> Result<PathBuf, String> {
    let backup_path = invalid_backup_path(path);
    fs::rename(path, &backup_path).map_err(|error| {
        format!(
            "failed to back up invalid config {} to {}: {error}",
            path.display(),
            backup_path.display()
        )
    })?;
    Ok(backup_path)
}

fn is_valid_mcp_server_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn normalize_mcp_server(
    body: UpsertMcpServerBody,
) -> Result<(String, McpServerConfig), (StatusCode, Json<Value>)> {
    let name = body.name.trim().to_owned();
    if !is_valid_mcp_server_name(&name) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "server name must match [A-Za-z0-9_-]",
            })),
        ));
    }

    let transport = match body.transport.as_deref() {
        Some("streamable_http") => McpTransport::StreamableHttp,
        _ => McpTransport::Stdio,
    };

    match transport {
        McpTransport::Stdio => {
            let command = body
                .command
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .to_owned();
            if command.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "command is required for stdio transport" })),
                ));
            }

            for key in body.env.keys() {
                if !is_valid_env_key(key) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": format!("invalid env var name: {key}") })),
                    ));
                }
            }

            Ok((
                name,
                McpServerConfig {
                    transport,
                    command,
                    args: body.args,
                    env: body.env,
                    enabled: body.enabled,
                    working_dir: body
                        .working_dir
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned),
                    url: None,
                    bearer_token_env: None,
                    headers: HashMap::new(),
                },
            ))
        }
        McpTransport::StreamableHttp => {
            let url = body.url.as_deref().map(str::trim).unwrap_or("").to_owned();
            if url.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "url is required for streamable_http transport" })),
                ));
            }

            Ok((
                name,
                McpServerConfig {
                    transport,
                    command: String::new(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    enabled: body.enabled,
                    working_dir: None,
                    url: Some(url),
                    bearer_token_env: body
                        .bearer_token_env
                        .as_deref()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(ToOwned::to_owned),
                    headers: body.headers,
                },
            ))
        }
    }
}

fn mcp_server_entry(name: &str, server: &McpServerConfig) -> Value {
    let transport_str = match server.transport {
        McpTransport::Stdio => "stdio",
        McpTransport::StreamableHttp => "streamable_http",
    };
    json!({
        "name": name,
        "transport": transport_str,
        "command": server.command,
        "args": server.args,
        "env": server.env,
        "enabled": server.enabled,
        "working_dir": server.working_dir,
        "url": server.url,
        "bearer_token_env": server.bearer_token_env,
        "headers": server.headers,
    })
}

fn sorted_server_entries(servers: &HashMap<String, McpServerConfig>) -> Vec<Value> {
    let mut entries = servers
        .iter()
        .map(|(name, server)| (name.clone(), server.clone()))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
        .into_iter()
        .map(|(name, server)| mcp_server_entry(&name, &server))
        .collect()
}

fn read_json_file(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(serde_json::Map::new()));
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }

    match serde_json::from_str(&raw) {
        Ok(value) => Ok(value),
        Err(error) => {
            let backup_path = backup_invalid_file(path)?;
            tracing::warn!(
                "backed up invalid JSON config {} to {} after parse error: {}",
                path.display(),
                backup_path.display(),
                error
            );
            Ok(Value::Object(serde_json::Map::new()))
        }
    }
}

fn write_json_file(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let raw = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    fs::write(path, raw).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn read_managed_names(path: &Path) -> Result<HashSet<String>, String> {
    if !path.exists() {
        return Ok(HashSet::new());
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(HashSet::new());
    }

    match serde_json::from_str::<ManagedMcpSyncState>(&raw) {
        Ok(state) => Ok(state.managed_names.into_iter().collect()),
        Err(error) => {
            // If the sync state file is corrupted, log a warning and start
            // fresh rather than failing all MCP operations.
            tracing::warn!(
                "MCP sync state {} is corrupted ({error}), resetting",
                path.display()
            );
            Ok(HashSet::new())
        }
    }
}

fn write_managed_names(path: &Path, names: &HashSet<String>) -> Result<(), String> {
    let mut managed_names = names.iter().cloned().collect::<Vec<_>>();
    managed_names.sort();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }

    let raw =
        serde_json::to_vec_pretty(&ManagedMcpSyncState { managed_names }).map_err(|error| {
            format!(
                "failed to serialize managed MCP sync state {}: {error}",
                path.display()
            )
        })?;
    fs::write(path, raw).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn ensure_json_object<'a>(
    root: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a mut serde_json::Map<String, Value>, String> {
    let value = root
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    value
        .as_object_mut()
        .ok_or_else(|| format!("{key} must be a JSON object"))
}

fn sync_claude_mcp_json(
    path: &Path,
    previous_managed_names: &HashSet<String>,
    servers: &HashMap<String, McpServerConfig>,
) -> Result<(), String> {
    let mut root = match read_json_file(path)? {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let table = ensure_json_object(&mut root, "mcpServers")?;

    for name in previous_managed_names {
        table.remove(name);
    }
    for (name, server) in servers {
        if !server.enabled {
            continue;
        }
        let entry = match server.transport {
            McpTransport::Stdio => json!({
                "command": server.command,
                "args": server.args,
                "env": server.env,
            }),
            McpTransport::StreamableHttp => {
                let mut entry = serde_json::Map::new();
                entry.insert("type".to_owned(), json!("url"));
                if let Some(ref url) = server.url {
                    entry.insert("url".to_owned(), json!(url));
                }
                if !server.headers.is_empty() {
                    entry.insert("headers".to_owned(), json!(server.headers));
                }
                // Claude Code supports bearer_token via an env var —
                // if set, inject it via headers at sync time.
                Value::Object(entry)
            }
        };
        table.insert(name.clone(), entry);
    }

    write_json_file(path, &Value::Object(root))
}

fn sync_codex_mcp_json(
    path: &Path,
    previous_managed_names: &HashSet<String>,
    servers: &HashMap<String, McpServerConfig>,
) -> Result<(), String> {
    let mut root = match read_json_file(path)? {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let table = ensure_json_object(&mut root, "mcp_servers")?;

    for name in previous_managed_names {
        table.remove(name);
    }
    for (name, server) in servers {
        let entry = match server.transport {
            McpTransport::Stdio => json!({
                "command": server.command,
                "args": server.args,
                "env": server.env,
                "enabled": server.enabled,
                "cwd": server.working_dir,
            }),
            McpTransport::StreamableHttp => {
                let mut entry = serde_json::Map::new();
                entry.insert("type".to_owned(), json!("url"));
                if let Some(ref url) = server.url {
                    entry.insert("url".to_owned(), json!(url));
                }
                entry.insert("enabled".to_owned(), json!(server.enabled));
                if !server.headers.is_empty() {
                    entry.insert("headers".to_owned(), json!(server.headers));
                }
                Value::Object(entry)
            }
        };
        table.insert(name.clone(), entry);
    }

    write_json_file(path, &Value::Object(root))
}

fn read_toml_file(path: &Path) -> Result<toml::Value, String> {
    if !path.exists() {
        return Ok(toml::Value::Table(Default::default()));
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(toml::Value::Table(Default::default()));
    }

    match toml::from_str(&raw) {
        Ok(value) => Ok(value),
        Err(error) => {
            let backup_path = backup_invalid_file(path)?;
            tracing::warn!(
                "backed up invalid TOML config {} to {} after parse error: {}",
                path.display(),
                backup_path.display(),
                error
            );
            Ok(toml::Value::Table(Default::default()))
        }
    }
}

fn write_toml_file(path: &Path, value: &toml::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let raw = toml::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    fs::write(path, raw).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn ensure_toml_table<'a>(
    root: &'a mut toml::map::Map<String, toml::Value>,
    key: &str,
) -> Result<&'a mut toml::map::Map<String, toml::Value>, String> {
    let value = root
        .entry(key.to_owned())
        .or_insert_with(|| toml::Value::Table(Default::default()));
    value
        .as_table_mut()
        .ok_or_else(|| format!("{key} must be a TOML table"))
}

fn sync_codex_config_toml(
    path: &Path,
    previous_managed_names: &HashSet<String>,
    servers: &HashMap<String, McpServerConfig>,
) -> Result<(), String> {
    let mut root = read_toml_file(path)?;
    let root_table = root
        .as_table_mut()
        .ok_or_else(|| format!("{} must contain a TOML table at the root", path.display()))?;
    let table = ensure_toml_table(root_table, "mcp_servers")?;

    for name in previous_managed_names {
        table.remove(name);
    }
    for (name, server) in servers {
        let mut entry = toml::map::Map::new();
        match server.transport {
            McpTransport::Stdio => {
                entry.insert(
                    "command".to_owned(),
                    toml::Value::String(server.command.clone()),
                );
                entry.insert(
                    "args".to_owned(),
                    toml::Value::Array(
                        server
                            .args
                            .iter()
                            .cloned()
                            .map(toml::Value::String)
                            .collect(),
                    ),
                );
                if !server.env.is_empty() {
                    let mut env_table = toml::map::Map::new();
                    let mut env_entries = server.env.iter().collect::<Vec<_>>();
                    env_entries.sort_by(|left, right| left.0.cmp(right.0));
                    for (key, value) in env_entries {
                        env_table.insert(key.clone(), toml::Value::String(value.clone()));
                    }
                    entry.insert("env".to_owned(), toml::Value::Table(env_table));
                }
                if let Some(cwd) = server
                    .working_dir
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    entry.insert("cwd".to_owned(), toml::Value::String(cwd.to_owned()));
                }
            }
            McpTransport::StreamableHttp => {
                entry.insert("type".to_owned(), toml::Value::String("url".to_owned()));
                if let Some(ref url) = server.url {
                    entry.insert("url".to_owned(), toml::Value::String(url.clone()));
                }
                if !server.headers.is_empty() {
                    let mut headers_table = toml::map::Map::new();
                    let mut header_entries = server.headers.iter().collect::<Vec<_>>();
                    header_entries.sort_by(|left, right| left.0.cmp(right.0));
                    for (key, value) in header_entries {
                        headers_table.insert(key.clone(), toml::Value::String(value.clone()));
                    }
                    entry.insert("headers".to_owned(), toml::Value::Table(headers_table));
                }
            }
        }
        entry.insert("enabled".to_owned(), toml::Value::Boolean(server.enabled));
        table.insert(name.clone(), toml::Value::Table(entry));
    }

    write_toml_file(path, &root)
}

fn sync_external_mcp_files_blocking(
    servers: &HashMap<String, McpServerConfig>,
) -> Result<(), String> {
    let Some(home) = home_dir() else {
        return Ok(());
    };

    let managed_state_path = managed_sync_state_path(&home);
    let previous_managed_names = read_managed_names(&managed_state_path)?;
    let current_managed_names = servers.keys().cloned().collect::<HashSet<_>>();

    // Sync each external config file independently. If one fails (e.g. due to
    // a corrupt/unparseable file), log and continue syncing the others rather
    // than aborting the entire operation.
    let mut warnings: Vec<String> = Vec::new();

    if let Err(error) = sync_claude_mcp_json(
        &home.join(".claude").join("mcp.json"),
        &previous_managed_names,
        servers,
    ) {
        tracing::warn!("failed to sync claude mcp.json (non-fatal): {error}");
        warnings.push(error);
    }
    if let Err(error) = sync_codex_mcp_json(
        &home.join(".codex").join("mcp.json"),
        &previous_managed_names,
        servers,
    ) {
        tracing::warn!("failed to sync codex mcp.json (non-fatal): {error}");
        warnings.push(error);
    }
    if let Err(error) = sync_codex_config_toml(
        &home.join(".codex").join("config.toml"),
        &previous_managed_names,
        servers,
    ) {
        tracing::warn!("failed to sync codex config.toml (non-fatal): {error}");
        warnings.push(error);
    }

    // Always update managed names even if some syncs failed.
    write_managed_names(&managed_state_path, &current_managed_names)?;

    Ok(())
}

pub(crate) async fn sync_external_configs_from_servers(
    servers: &HashMap<String, McpServerConfig>,
) -> Result<(), String> {
    let servers = servers.clone();
    tokio::task::spawn_blocking(move || sync_external_mcp_files_blocking(&servers))
        .await
        .map_err(|error| format!("failed to join external MCP sync task: {error}"))?
}

async fn persist_config_file(path: PathBuf, config: &GaryxConfig) -> Result<(), String> {
    let value = serde_json::to_value(config)
        .map_err(|error| format!("failed to serialize config: {error}"))?;
    let path_for_write = path.clone();
    tokio::task::spawn_blocking(move || {
        let write_opts = ConfigWriteOptions {
            backup_keep: 3,
            mode: Some(0o600),
        };
        write_config_value_atomic(&path_for_write, &value, &write_opts)
    })
    .await
    .map_err(|error| format!("failed to join config persistence task: {error}"))?
    .map_err(|error| format!("failed to persist config file {}: {error}", path.display()))
}

async fn rollback_config_change<F>(
    state: &Arc<AppState>,
    previous_config: &GaryxConfig,
    sync_external_configs: &F,
) -> Result<(), String>
where
    F: for<'a> Fn(&'a HashMap<String, McpServerConfig>) -> BoxFuture<'a, Result<(), String>>,
{
    state
        .apply_runtime_config(previous_config.clone())
        .await
        .map_err(|error| format!("failed to restore runtime config: {error}"))?;
    sync_external_configs(&previous_config.mcp_servers)
        .await
        .map_err(|error| format!("failed to restore external MCP configs: {error}"))
}

fn append_rollback_error(primary_error: String, rollback_result: Result<(), String>) -> String {
    match rollback_result {
        Ok(()) => primary_error,
        Err(rollback_error) => format!("{primary_error}; rollback failed: {rollback_error}"),
    }
}

async fn persist_and_apply_config_with_sync<F>(
    state: &Arc<AppState>,
    config: &GaryxConfig,
    sync_external_configs: F,
) -> Result<(), String>
where
    F: for<'a> Fn(&'a HashMap<String, McpServerConfig>) -> BoxFuture<'a, Result<(), String>>,
{
    let previous_config = (*state.config_snapshot()).clone();

    if let Err(error) = state.apply_runtime_config(config.clone()).await {
        return Err(format!("failed to apply runtime config: {error}"));
    }

    if let Err(error) = sync_external_configs(&config.mcp_servers).await {
        let rollback =
            rollback_config_change(state, &previous_config, &sync_external_configs).await;
        return Err(append_rollback_error(error, rollback));
    }

    if let Some(path) = state.ops.config_path.clone()
        && let Err(error) = persist_config_file(path, config).await
    {
        let rollback =
            rollback_config_change(state, &previous_config, &sync_external_configs).await;
        return Err(append_rollback_error(error, rollback));
    }

    Ok(())
}

pub(crate) async fn persist_and_apply_config(
    state: &Arc<AppState>,
    config: &GaryxConfig,
) -> Result<(), String> {
    persist_and_apply_config_with_sync(state, config, |servers| {
        Box::pin(sync_external_configs_from_servers(servers))
    })
    .await
}

pub async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config_snapshot();
    (
        StatusCode::OK,
        Json(json!({
            "servers": sorted_server_entries(&config.mcp_servers),
        })),
    )
}

pub async fn create_mcp_server(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpsertMcpServerBody>,
) -> impl IntoResponse {
    let (name, server) = match normalize_mcp_server(body) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };

    let mut config = (*state.config_snapshot()).clone();
    if config.mcp_servers.contains_key(&name) {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("MCP server '{name}' already exists"),
            })),
        )
            .into_response();
    }

    config.mcp_servers.insert(name.clone(), server.clone());
    match persist_and_apply_config(&state, &config).await {
        Ok(()) => (StatusCode::CREATED, Json(mcp_server_entry(&name, &server))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

pub async fn update_mcp_server(
    State(state): State<Arc<AppState>>,
    AxumPath(current_name): AxumPath<String>,
    Json(body): Json<UpsertMcpServerBody>,
) -> impl IntoResponse {
    let (name, server) = match normalize_mcp_server(body) {
        Ok(value) => value,
        Err(error) => return error.into_response(),
    };

    let current_name = current_name.trim().to_owned();
    let mut config = (*state.config_snapshot()).clone();
    if !config.mcp_servers.contains_key(&current_name) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("MCP server '{current_name}' not found"),
            })),
        )
            .into_response();
    }
    if name != current_name && config.mcp_servers.contains_key(&name) {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("MCP server '{name}' already exists"),
            })),
        )
            .into_response();
    }

    config.mcp_servers.remove(&current_name);
    config.mcp_servers.insert(name.clone(), server.clone());
    match persist_and_apply_config(&state, &config).await {
        Ok(()) => (StatusCode::OK, Json(mcp_server_entry(&name, &server))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

pub async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    let name = name.trim().to_owned();
    let mut config = (*state.config_snapshot()).clone();
    if config.mcp_servers.remove(&name).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("MCP server '{name}' not found"),
            })),
        )
            .into_response();
    }

    match persist_and_apply_config(&state, &config).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "deleted": true,
                "name": name,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

pub async fn toggle_mcp_server(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<ToggleMcpServerBody>,
) -> impl IntoResponse {
    let name = name.trim().to_owned();
    let mut config = (*state.config_snapshot()).clone();
    let Some(server) = config.mcp_servers.get_mut(&name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("MCP server '{name}' not found"),
            })),
        )
            .into_response();
    };

    server.enabled = body.enabled;
    let updated = server.clone();
    match persist_and_apply_config(&state, &config).await {
        Ok(()) => (StatusCode::OK, Json(mcp_server_entry(&name, &updated))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests;
