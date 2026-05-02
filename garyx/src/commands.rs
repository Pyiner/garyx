use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Local};
use flate2::read::GzDecoder;
use garyx_bridge::MultiProviderBridge;
use garyx_channels::auth_flow::{AuthDisplayItem, AuthFlowExecutor, AuthPollResult};
use garyx_channels::feishu::FeishuAuthExecutor;
use garyx_channels::plugin_host::{
    InboundHandler, ManifestDiscoverer, PluginErrorCode, PluginManifest, SpawnOptions,
    SubprocessAuthFlowExecutor, SubprocessPlugin,
};
use garyx_channels::{
    BuiltInPluginDiscoverer, ChannelPluginManager, LocalDescriptorDiscoverer, PluginMetadata,
    WeixinAuthExecutor, builtin_plugin_metadata_list,
};
use garyx_gateway::server::AppState;
use garyx_gateway::server::Gateway;
use garyx_models::command_catalog::{
    CommandCatalogOptions, CommandSurface, is_valid_shortcut_command_name,
    normalize_shortcut_command_name,
};
use garyx_models::config::{
    ApiAccount, BUILTIN_CHANNEL_PLUGIN_FEISHU, BUILTIN_CHANNEL_PLUGIN_TELEGRAM,
    BUILTIN_CHANNEL_PLUGIN_WEIXIN, FeishuAccount, FeishuDomain, GaryxConfig, PluginAccountEntry,
    SlashCommand, TelegramAccount, WeixinAccount, feishu_account_to_plugin_entry,
    telegram_account_to_plugin_entry, weixin_account_to_plugin_entry,
};
use garyx_models::config_loader::{
    ConfigHotReloadOptions, ConfigHotReloader, ConfigLoadOptions, ConfigRuntimeOverrides,
    ConfigWriteOptions, write_config_value_atomic,
};
use garyx_models::local_paths::{
    default_agent_teams_state_path, default_custom_agents_state_path, default_log_file_path,
    default_session_data_dir, thread_transcripts_dir_for_data_dir,
};
use garyx_models::{
    AgentTeamProfile, CustomAgentProfile, ProviderType, builtin_provider_agent_profiles,
};
use garyx_router::{
    DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, FileThreadStore, ThreadStore, ThreadTranscriptStore,
    command_catalog_for_config, extract_run_id, is_thread_key, reserved_command_names,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tar::Archive;
use uuid::Uuid;

use crate::config_support::{
    default_config_path_buf, load_config_or_default, prepare_config_path_for_io_buf,
    print_diagnostics, print_errors,
};
use crate::runtime_assembler::RuntimeAssembler;

#[derive(Debug, Clone)]
pub(crate) struct OnboardCommandOptions {
    pub force: bool,
    pub json: bool,
    pub api_account: String,
    pub search_api_key: Option<String>,
    pub image_gen_api_key: Option<String>,
    pub conversation_index_api_key: Option<String>,
    pub enable_conversation_index: bool,
    pub disable_conversation_index: bool,
    pub conversation_index_model: Option<String>,
    pub conversation_index_base_url: Option<String>,
    pub run_gateway: bool,
    pub port_override: Option<u16>,
    pub host_override: Option<String>,
    pub no_channels: bool,
}

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

const CLAUDE_ENV_METADATA_KEY: &str = "desktop_claude_env";
const CODEX_ENV_METADATA_KEY: &str = "desktop_codex_env";
const CLAUDE_OAUTH_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
const CODEX_API_KEY_ENV: &str = "OPENAI_API_KEY";
const GITHUB_RELEASE_REPO: &str = "Pyiner/garyx";
const DEFAULT_CHANNEL_AGENT_ID: &str = "claude";

#[derive(Debug, Deserialize)]
struct GitHubReleaseSummary {
    tag_name: String,
}

#[cfg(test)]
pub(crate) fn routing_rebuild_channels(config: &GaryxConfig) -> Vec<String> {
    crate::runtime_assembler::routing_rebuild_channels(config)
}

fn normalize_release_version(value: &str) -> String {
    value.trim().trim_start_matches('v').to_owned()
}

fn detect_release_target_for(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        _ => Err(format!("unsupported platform for self-update: {os}/{arch}")),
    }
}

fn detect_release_target() -> Result<&'static str, Box<dyn std::error::Error>> {
    detect_release_target_for(std::env::consts::OS, std::env::consts::ARCH).map_err(|e| e.into())
}

fn parse_sha256_checksum(contents: &str) -> Result<String, Box<dyn std::error::Error>> {
    let checksum = contents
        .lines()
        .find_map(|line| line.split_whitespace().next())
        .filter(|value| !value.is_empty())
        .ok_or("checksum file is empty or malformed")?;
    Ok(checksum.to_owned())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

async fn latest_release_version(
    client: &reqwest::Client,
) -> Result<String, Box<dyn std::error::Error>> {
    let summary = client
        .get(format!(
            "https://api.github.com/repos/{GITHUB_RELEASE_REPO}/releases/latest"
        ))
        .send()
        .await?
        .error_for_status()?
        .json::<GitHubReleaseSummary>()
        .await?;
    Ok(normalize_release_version(&summary.tag_name))
}

fn replacement_binary_path(
    install_path: Option<PathBuf>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = install_path {
        return Ok(path);
    }
    Ok(std::env::current_exe()?)
}

fn register_plugin_state_logging(plugin_manager: &mut ChannelPluginManager) {
    plugin_manager.register_state_hook(|status| {
        tracing::info!(
            plugin_id = %status.metadata.id,
            state = ?status.state,
            source = %status.metadata.source,
            error = status.last_error.as_deref().unwrap_or(""),
            "channel plugin state changed"
        );
    });
}

async fn rebuild_channel_plugins(
    plugin_manager: &tokio::sync::Mutex<ChannelPluginManager>,
    config: &GaryxConfig,
    state: &Arc<AppState>,
    bridge: &Arc<MultiProviderBridge>,
    no_channels: bool,
) -> Result<(), String> {
    {
        let mut manager = plugin_manager.lock().await;
        manager.stop_all().await;
        manager.cleanup_all().await;

        let mut replacement = ChannelPluginManager::new();
        register_plugin_state_logging(&mut replacement);
        // Attach the production `SwappableDispatcher` so subprocess
        // plugins registered below can fork-and-store into it. Built-in
        // plugins go through `BuiltInPluginDiscoverer` instead and do
        // not need this handle.
        replacement.attach_dispatcher(state.channel_dispatcher_swap());

        if !no_channels {
            let built_in_discoverer = BuiltInPluginDiscoverer::new(
                config.channels.clone(),
                state.threads.router.clone(),
                bridge.clone(),
                config.gateway.public_url.clone(),
            );
            replacement.discover_and_register(&built_in_discoverer)?;

            let local_discoverer = LocalDescriptorDiscoverer::from_env();
            replacement.discover_and_register(&local_discoverer)?;

            replacement.initialize_all().await;
            replacement.start_all().await;
        }

        *manager = replacement;
    }

    // Subprocess plugins go through a separate discovery path
    // (`plugin.toml` manifests + preflight + `register_subprocess_plugin`).
    // Done outside the manager lock above because
    // `register_manifest_plugins` itself re-acquires the lock per
    // plugin — async `register_subprocess_plugin` spawns a child and
    // runs the initialize/start handshake under that lock.
    if !no_channels {
        crate::channel_plugin_host::register_manifest_plugins(
            plugin_manager,
            config,
            VERSION,
            crate::channel_plugin_host::HostDeps {
                router: state.threads.router.clone(),
                bridge: bridge.clone(),
                swap: state.channel_dispatcher_swap(),
            },
        )
        .await;
    }

    Ok(())
}

pub(crate) async fn run_gateway(
    config_path: &str,
    port_override: Option<u16>,
    host_override: Option<String>,
    no_channels: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Garyx v{} starting...", VERSION);

    let loaded = load_config_or_default(
        config_path,
        ConfigRuntimeOverrides {
            gateway_host: host_override.clone(),
            gateway_port: port_override,
        },
    )?;
    print_diagnostics(&loaded.diagnostics);
    let config_path = loaded.path;
    let config = loaded.config;
    tracing::info!("Loading config from {}", config_path.display());

    let host = config.gateway.host.clone();
    let port = config.gateway.port;

    let assembly = RuntimeAssembler::new(&config_path, config.clone())
        .assemble()
        .await?;
    let state = assembly.state.clone();
    let bridge = assembly.bridge.clone();
    let cron_service = assembly.cron_service;

    // 3.0 Share the channel plugin manager with AppState so HTTP
    // endpoints (`GET /api/channels/plugins`) see the same
    // registrations the boot path creates. Single source of truth.
    let plugin_manager = state.channel_plugin_manager();
    register_plugin_state_logging(&mut *plugin_manager.lock().await);

    // 3.1 Start hot reload watcher (best effort)
    let hot_reloader = {
        let options = ConfigHotReloadOptions {
            poll_interval: Duration::from_millis(500),
            debounce: Duration::from_millis(400),
            load_options: ConfigLoadOptions {
                default_path: default_config_path_buf(),
                runtime_overrides: ConfigRuntimeOverrides::default(),
            },
        };

        let reloader = ConfigHotReloader::start(config_path.clone(), config.clone(), options);

        let state_cb = state.clone();
        let bridge_cb = bridge.clone();
        let plugin_manager_cb = plugin_manager.clone();
        let tokio_handle = tokio::runtime::Handle::current();
        reloader.register_callback(move |new_config, diagnostics| {
            print_diagnostics(&diagnostics);
            let state = state_cb.clone();
            let bridge = bridge_cb.clone();
            let plugin_manager = plugin_manager_cb.clone();
            tokio_handle.spawn(async move {
                if let Err(error) = state.apply_runtime_config(new_config.clone()).await {
                    tracing::warn!(
                        error = %error,
                        "Failed to fully apply hot-reloaded config"
                    );
                    return;
                }

                if let Err(error) = rebuild_channel_plugins(
                    &plugin_manager,
                    &new_config,
                    &state,
                    &bridge,
                    no_channels,
                )
                .await
                {
                    tracing::warn!(
                        error = %error,
                        "Failed to rebuild channel plugins after hot-reloaded config"
                    );
                }
            });
        });

        reloader
    };

    // 4. Discover and start channel plugins (if not disabled).
    let run_result: Result<(), Box<dyn std::error::Error>> = async {
        rebuild_channel_plugins(&plugin_manager, &config, &state, &bridge, no_channels)
            .await
            .map_err(std::io::Error::other)?;
        {
            let wake_state = state.clone();
            tokio::spawn(async move {
                garyx_gateway::restart_wake::drain_pending_restart_wakes(wake_state).await;
            });
        }

        // Runtime services are started and wired by RuntimeAssembler.
        let gateway = Gateway::new(state);
        let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
        tracing::info!("Gateway listening on {}", addr);
        gateway.serve(addr).await?;
        Ok(())
    }
    .await;

    // Always run shutdown sequence, even when startup/serve fails.
    {
        let mut plugin_manager = plugin_manager.lock().await;
        plugin_manager.stop_all().await;
        plugin_manager.cleanup_all().await;
    }

    match Arc::try_unwrap(cron_service) {
        Ok(mut svc) => svc.stop().await,
        Err(_) => tracing::warn!("Cron service still has outstanding references on shutdown"),
    }

    let metrics = hot_reloader.metrics();
    tracing::info!(
        attempts = metrics.attempts,
        successes = metrics.successes,
        failures = metrics.failures,
        callback_notifications = metrics.callback_notifications,
        "config hot-reload metrics"
    );
    hot_reloader.stop();
    run_result
}

#[derive(Debug, Serialize)]
struct ThreadTranscriptMigrationThreadReport {
    thread_id: String,
    original_message_count: usize,
    transcript_message_count: usize,
    snapshot_message_count: usize,
    transcript_file: Option<String>,
    verified: bool,
}

#[derive(Debug, Serialize)]
struct ThreadTranscriptMigrationReport {
    data_dir: String,
    transcript_dir: String,
    backup_dir: Option<String>,
    rewrite_records: bool,
    total_threads: usize,
    total_messages: usize,
    verified_threads: usize,
    failed_threads: usize,
    failures: Vec<String>,
    threads: Vec<ThreadTranscriptMigrationThreadReport>,
}

pub(crate) async fn cmd_migrate_thread_transcripts(
    config_path: &str,
    data_dir_override: Option<&str>,
    backup_dir_override: Option<&str>,
    rewrite_records: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let session_data_dir = if let Some(data_dir) = data_dir_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        PathBuf::from(data_dir)
    } else {
        load_config_or_default(config_path, ConfigRuntimeOverrides::default())?
            .config
            .sessions
            .data_dir
            .map(PathBuf::from)
            .unwrap_or_else(default_session_data_dir)
    };
    let transcript_dir = thread_transcripts_dir_for_data_dir(&session_data_dir);
    let backup_dir = backup_dir_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            rewrite_records.then(|| {
                session_data_dir.join("migration-backups").join(format!(
                    "thread-transcripts-{}",
                    chrono::Utc::now().format("%Y%m%d%H%M%S")
                ))
            })
        });

    if let Some(backup_dir) = &backup_dir {
        fs::create_dir_all(backup_dir)?;
    }

    let store = FileThreadStore::new(&session_data_dir).await?;
    let store: Arc<dyn ThreadStore> = Arc::new(store);
    let transcript_store = ThreadTranscriptStore::file(&transcript_dir).await?;

    let mut report = ThreadTranscriptMigrationReport {
        data_dir: session_data_dir.display().to_string(),
        transcript_dir: transcript_dir.display().to_string(),
        backup_dir: backup_dir.as_ref().map(|path| path.display().to_string()),
        rewrite_records,
        total_threads: 0,
        total_messages: 0,
        verified_threads: 0,
        failed_threads: 0,
        failures: Vec::new(),
        threads: Vec::new(),
    };

    for thread_id in store.list_keys(None).await {
        if !is_thread_key(&thread_id) {
            continue;
        }
        report.total_threads += 1;
        let Some(mut thread_data) = store.get(&thread_id).await else {
            report.failed_threads += 1;
            report
                .failures
                .push(format!("{thread_id}: missing thread record"));
            continue;
        };
        let messages = thread_data
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let original_message_count = messages.len();
        report.total_messages += original_message_count;

        if rewrite_records {
            if let Some(backup_dir) = &backup_dir {
                let backup_path =
                    backup_dir.join(format!("{}.json", encode_thread_backup_key(&thread_id)));
                fs::write(&backup_path, serde_json::to_vec_pretty(&thread_data)?)?;
            }
        }

        let rewrite_result = transcript_store
            .rewrite_from_messages(&thread_id, &messages)
            .await;
        let append_result = match rewrite_result {
            Ok(result) => result,
            Err(error) => {
                report.failed_threads += 1;
                report.failures.push(format!("{thread_id}: {error}"));
                continue;
            }
        };
        let transcript_message_count = transcript_store.message_count(&thread_id).await?;
        let verified = transcript_message_count == original_message_count;
        if verified {
            report.verified_threads += 1;
        } else {
            report.failed_threads += 1;
            report.failures.push(format!(
                "{thread_id}: transcript count mismatch (expected {original_message_count}, got {transcript_message_count})"
            ));
        }

        let snapshot_messages = if original_message_count > DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT {
            messages[original_message_count - DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT..].to_vec()
        } else {
            messages.clone()
        };

        if rewrite_records {
            if let Some(object) = thread_data.as_object_mut() {
                object.insert(
                    "messages".to_owned(),
                    Value::Array(snapshot_messages.clone()),
                );
                object.insert(
                    "message_count".to_owned(),
                    Value::Number(serde_json::Number::from(transcript_message_count as u64)),
                );
                let recent_committed_run_ids = collect_recent_run_ids(&messages);
                object.insert(
                    "history".to_owned(),
                    json!({
                        "source": "transcript_v1",
                        "transcript_file": append_result
                            .transcript_file
                            .as_ref()
                            .map(|path| path.display().to_string()),
                        "message_count": transcript_message_count,
                        "snapshot_limit": DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT,
                        "snapshot_truncated": transcript_message_count > DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT,
                        "last_message_at": append_result.last_message_at,
                        "recent_committed_run_ids": recent_committed_run_ids,
                    }),
                );
            }
            store.set(&thread_id, thread_data).await;
        }

        report.threads.push(ThreadTranscriptMigrationThreadReport {
            thread_id,
            original_message_count,
            transcript_message_count,
            snapshot_message_count: snapshot_messages.len(),
            transcript_file: append_result
                .transcript_file
                .as_ref()
                .map(|path| path.display().to_string()),
            verified,
        });
    }

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub(crate) fn cmd_config_show(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    match load_config_or_default(config_path, ConfigRuntimeOverrides::default()) {
        Ok(loaded) => {
            print_diagnostics(&loaded.diagnostics);
            println!("{}", serde_json::to_string_pretty(&loaded.config)?);
        }
        Err(err) => {
            print_errors(&err.diagnostics);
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn cmd_config_validate(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    match load_config_or_default(config_path, ConfigRuntimeOverrides::default()) {
        Ok(loaded) => {
            print_diagnostics(&loaded.diagnostics);
            println!("Config is valid");
        }
        Err(err) => {
            print_errors(&err.diagnostics);
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn cmd_config_path(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(config_path);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    println!("{}", absolute.display());
    Ok(())
}

fn load_config_value(config_path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    if config_path.exists() {
        let raw = fs::read_to_string(config_path)?;
        Ok(serde_json::from_str::<Value>(&raw)?)
    } else {
        Ok(serde_json::to_value(GaryxConfig::default())?)
    }
}

fn save_config_value(config_path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let _validated: GaryxConfig = serde_json::from_value(value.clone())?;
    write_config_value_atomic(config_path, value, &ConfigWriteOptions::default())?;
    Ok(())
}

fn save_config_struct(
    config_path: &Path,
    config: &GaryxConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let value = serde_json::to_value(config)?;
    write_config_value_atomic(config_path, &value, &ConfigWriteOptions::default())?;
    Ok(())
}

fn encode_thread_backup_key(thread_id: &str) -> String {
    let mut encoded = String::with_capacity(thread_id.len() * 2);
    for byte in thread_id.as_bytes() {
        encoded.push_str(&format!("{byte:02x}"));
    }
    format!("k_{encoded}")
}

fn collect_recent_run_ids(messages: &[Value]) -> Vec<String> {
    let mut run_ids = Vec::new();
    for message in messages {
        let Some(run_id) = extract_run_id(message) else {
            continue;
        };
        run_ids.retain(|existing| existing != &run_id);
        run_ids.push(run_id);
        if run_ids.len() > 256 {
            let drop_count = run_ids.len() - 256;
            run_ids.drain(0..drop_count);
        }
    }
    run_ids
}

fn get_dotted_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.trim().is_empty() {
        return Some(root);
    }
    let mut current = root;
    for seg in path.split('.') {
        match current {
            Value::Object(map) => {
                current = map.get(seg)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn set_dotted_path(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    if path.trim().is_empty() {
        *root = value;
        return Ok(());
    }
    let mut current = root;
    let mut parts = path.split('.').peekable();
    while let Some(seg) = parts.next() {
        if parts.peek().is_none() {
            match current {
                Value::Object(map) => {
                    map.insert(seg.to_owned(), value);
                    return Ok(());
                }
                _ => return Err("target parent is not an object".to_owned()),
            }
        }

        match current {
            Value::Object(map) => {
                let entry = map
                    .entry(seg.to_owned())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !entry.is_object() {
                    *entry = Value::Object(serde_json::Map::new());
                }
                current = entry;
            }
            _ => return Err("path traverses a non-object value".to_owned()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn unset_dotted_path(root: &mut Value, path: &str) -> Result<bool, String> {
    if path.trim().is_empty() {
        return Err("path cannot be empty for unset".to_owned());
    }

    let mut parts: Vec<&str> = path.split('.').collect();
    let key = parts.pop().unwrap_or_default();
    let mut current = root;
    for seg in parts {
        match current {
            Value::Object(map) => {
                let Some(next) = map.get_mut(seg) else {
                    return Ok(false);
                };
                current = next;
            }
            _ => return Err("path traverses a non-object value".to_owned()),
        }
    }

    match current {
        Value::Object(map) => Ok(map.remove(key).is_some()),
        _ => Err("target parent is not an object".to_owned()),
    }
}

pub(crate) fn cmd_config_get(
    config_path: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let value = load_config_value(&prepared.active_path)?;
    let Some(found) = get_dotted_path(&value, path) else {
        eprintln!("Path not found: {path}");
        std::process::exit(1);
    };
    if found.is_string() {
        println!("{}", found.as_str().unwrap_or_default());
    } else {
        println!("{}", serde_json::to_string_pretty(found)?);
    }
    Ok(())
}

pub(crate) fn cmd_config_set(
    config_path: &str,
    path: &str,
    value_text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let mut value = load_config_value(&prepared.active_path)?;
    let new_value = serde_json::from_str::<Value>(value_text)
        .unwrap_or_else(|_| Value::String(value_text.to_owned()));
    if let Err(err) = set_dotted_path(&mut value, path, new_value) {
        eprintln!("Set failed for path '{path}': {err}");
        std::process::exit(1);
    }
    save_config_value(&prepared.active_path, &value)?;
    println!("Updated {path}");
    Ok(())
}

pub(crate) fn cmd_config_unset(
    config_path: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let mut value = load_config_value(&prepared.active_path)?;
    match unset_dotted_path(&mut value, path) {
        Ok(true) => {
            save_config_value(&prepared.active_path, &value)?;
            println!("Removed {path}");
        }
        Ok(false) => {
            eprintln!("Path not found: {path}");
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("Unset failed for path '{path}': {err}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn cmd_config_init(
    config_path: &str,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let path = prepared.active_path;
    if path.exists() && !force {
        eprintln!(
            "Config already exists at {}. Use --force to overwrite.",
            path.display()
        );
        std::process::exit(1);
    }
    let default_value = serde_json::to_value(GaryxConfig::default())?;
    write_config_value_atomic(&path, &default_value, &ConfigWriteOptions::default())?;
    println!("Initialized config at {}", path.display());
    Ok(())
}

fn parse_command_surface(value: &str) -> Result<CommandSurface, Box<dyn std::error::Error>> {
    let normalized = value.trim().replace('-', "_").to_ascii_lowercase();
    match normalized.as_str() {
        "router" => Ok(CommandSurface::Router),
        "gateway_api" | "gateway" | "api" => Ok(CommandSurface::GatewayApi),
        "desktop_composer" | "desktop" | "composer" => Ok(CommandSurface::DesktopComposer),
        "telegram" => Ok(CommandSurface::Telegram),
        "api_chat" | "chat" => Ok(CommandSurface::ApiChat),
        "plugin" | "plugins" => Ok(CommandSurface::Plugin),
        _ => Err(format!(
            "unknown command surface '{value}'. Expected router, gateway_api, desktop_composer, telegram, api_chat, or plugin"
        )
        .into()),
    }
}

fn command_prompt_preview(prompt: &str, max_chars: usize) -> String {
    let compact = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let mut preview = compact
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        preview.push('…');
        preview
    }
}

fn read_shortcut_prompt(prompt: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(prompt) = prompt {
        let trimmed = prompt.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_owned());
        }
    }

    if std::io::stdin().is_terminal() {
        return Err("provide --prompt or pipe prompt text on stdin".into());
    }

    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    let prompt = buffer.trim();
    if prompt.is_empty() {
        return Err("prompt cannot be empty".into());
    }
    Ok(prompt.to_owned())
}

fn normalize_cli_shortcut_name(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let normalized = normalize_shortcut_command_name(name);
    if !is_valid_shortcut_command_name(&normalized) {
        return Err(
            "command name must be 1-32 chars using only lowercase a-z, 0-9, and _"
                .to_owned()
                .into(),
        );
    }
    Ok(normalized)
}

pub(crate) fn cmd_command_list(
    config_path: &str,
    json_output: bool,
    surface: Option<String>,
    channel: Option<String>,
    account_id: Option<String>,
    include_hidden: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let surface = surface.as_deref().map(parse_command_surface).transpose()?;
    let catalog = command_catalog_for_config(
        &loaded.config,
        CommandCatalogOptions {
            surface,
            channel,
            account_id,
            include_hidden,
        },
    );

    if json_output {
        println!("{}", serde_json::to_string_pretty(&catalog)?);
        return Ok(());
    }

    for warning in &catalog.warnings {
        eprintln!("warning: {}: {}", warning.code, warning.message);
    }

    if catalog.commands.is_empty() {
        println!("No commands found");
        return Ok(());
    }

    for command in &catalog.commands {
        println!(
            "{:<18} {:<14} {}",
            command.slash,
            serde_json::to_string(&command.kind)?.trim_matches('"'),
            command.description
        );
    }
    Ok(())
}

pub(crate) fn cmd_command_get(
    config_path: &str,
    name: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let normalized = normalize_cli_shortcut_name(name)?;
    let Some(command) = loaded
        .config
        .commands
        .iter()
        .find(|command| normalize_shortcut_command_name(&command.name) == normalized)
    else {
        return Err(format!("shortcut '/{normalized}' not found").into());
    };

    if json_output {
        let slash = format!("/{normalized}");
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": normalized.clone(),
                "slash": slash,
                "description": command.description.clone(),
                "prompt": command.prompt.as_deref().unwrap_or_default(),
            }))?
        );
    } else {
        println!("/{normalized}");
        if !command.description.trim().is_empty() {
            println!("Description: {}", command.description.trim());
        }
        println!("Prompt:");
        println!("{}", command.prompt.as_deref().unwrap_or_default());
    }
    Ok(())
}

pub(crate) async fn cmd_command_set(
    config_path: &str,
    name: String,
    prompt: Option<String>,
    description: Option<String>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let config_path = loaded.path;
    let mut config = loaded.config;
    let normalized = normalize_cli_shortcut_name(&name)?;
    if reserved_command_names().contains(normalized.as_str()) {
        return Err(
            format!("shortcut '/{normalized}' collides with a built-in channel command").into(),
        );
    }
    let prompt = read_shortcut_prompt(prompt)?;
    let description = description
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| command_prompt_preview(&prompt, 80));

    let mut created = true;
    if let Some(existing) = config
        .commands
        .iter_mut()
        .find(|command| normalize_shortcut_command_name(&command.name) == normalized)
    {
        existing.name = normalized.clone();
        existing.description = description.clone();
        existing.prompt = Some(prompt.clone());
        existing.skill_id = None;
        created = false;
    } else {
        config.commands.push(SlashCommand {
            name: normalized.clone(),
            description: description.clone(),
            prompt: Some(prompt.clone()),
            skill_id: None,
        });
    }

    save_config_struct(&config_path, &config)?;
    if json_output {
        notify_gateway_reload_quiet(&config_path).await;
        let slash = format!("/{normalized}");
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "created": created,
                "name": normalized.clone(),
                "slash": slash,
                "description": description,
                "prompt": prompt,
            }))?
        );
    } else {
        notify_gateway_reload(&config_path).await;
        println!(
            "{} /{}",
            if created { "Created" } else { "Updated" },
            normalized
        );
    }
    Ok(())
}

pub(crate) async fn cmd_command_delete(
    config_path: &str,
    name: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    print_diagnostics(&loaded.diagnostics);
    let config_path = loaded.path;
    let mut config = loaded.config;
    let normalized = normalize_cli_shortcut_name(name)?;
    let before = config.commands.len();
    config
        .commands
        .retain(|command| normalize_shortcut_command_name(&command.name) != normalized);
    if config.commands.len() == before {
        return Err(format!("shortcut '/{normalized}' not found").into());
    }

    save_config_struct(&config_path, &config)?;
    if json_output {
        notify_gateway_reload_quiet(&config_path).await;
        let slash = format!("/{normalized}");
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "deleted": true,
                "name": normalized.clone(),
                "slash": slash,
            }))?
        );
    } else {
        notify_gateway_reload(&config_path).await;
        println!("Deleted /{normalized}");
    }
    Ok(())
}

pub(crate) async fn cmd_status(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let port = config.gateway.port;
    let host = if config.gateway.host == "0.0.0.0" {
        "127.0.0.1"
    } else {
        &config.gateway.host
    };
    let url = format!("http://{}:{}/health", host, port);

    let running = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .is_ok();

    if json {
        let obj = serde_json::json!({
            "running": running,
            "host": config.gateway.host,
            "port": port,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        let status = if running { "running" } else { "not running" };
        println!("Gateway: {} ({}:{})", status, config.gateway.host, port);
    }
    Ok(())
}

pub(crate) async fn cmd_update(
    version: Option<String>,
    install_path: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent(format!("garyx-cli/{VERSION}"))
        .build()?;

    let requested_version = match version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) => normalize_release_version(value),
        None => latest_release_version(&client).await?,
    };
    let target = detect_release_target()?;
    let destination = replacement_binary_path(install_path)?;
    let parent = destination
        .parent()
        .ok_or_else(|| {
            format!(
                "update target has no parent directory: {}",
                destination.display()
            )
        })?
        .to_path_buf();

    if version.is_none() && requested_version == VERSION {
        println!(
            "garyx is already up to date at v{} ({})",
            VERSION,
            destination.display()
        );
        return Ok(());
    }

    println!("Updating garyx to v{requested_version} for {target}...");

    let archive_name = format!("garyx-{requested_version}-{target}.tar.gz");
    let base_url =
        format!("https://github.com/{GITHUB_RELEASE_REPO}/releases/download/v{requested_version}");

    let archive_bytes = client
        .get(format!("{base_url}/{archive_name}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let checksum_text = client
        .get(format!("{base_url}/{archive_name}.sha256"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected_sha = parse_sha256_checksum(&checksum_text)?;
    let actual_sha = sha256_hex(&archive_bytes);
    if expected_sha != actual_sha {
        return Err(format!(
            "download checksum mismatch for {archive_name}: expected {expected_sha}, got {actual_sha}"
        )
        .into());
    }

    let tempdir = tempfile::tempdir()?;
    let decoder = GzDecoder::new(std::io::Cursor::new(archive_bytes));
    let mut archive = Archive::new(decoder);
    archive.unpack(tempdir.path())?;

    let extracted_binary = tempdir
        .path()
        .join(format!("garyx-{requested_version}-{target}"))
        .join("garyx");
    if !extracted_binary.is_file() {
        return Err(format!(
            "release archive did not contain expected binary at {}",
            extracted_binary.display()
        )
        .into());
    }

    fs::create_dir_all(&parent)?;
    let staged_path = parent.join(format!(".garyx-update-{}.tmp", Uuid::new_v4().simple()));
    fs::copy(&extracted_binary, &staged_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&staged_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&staged_path, perms)?;
    }
    fs::rename(&staged_path, &destination)?;

    println!(
        "Updated garyx from v{} to v{} at {}",
        VERSION,
        requested_version,
        destination.display()
    );
    Ok(())
}

pub(crate) async fn cmd_gateway_reload_config(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let response = post_gateway_json(&gateway, "/api/settings/reload", &json!({})).await?;
    print_pretty_json(&response)?;
    Ok(())
}

/// Best-effort reload trigger for CLI flows that mutate the on-disk config
/// (channel add/login). The TOML is written atomically before we get here, so
/// a running gateway just needs to re-read it. Silent when the gateway isn't
/// up — that's the normal "configure before starting" case.
async fn notify_gateway_reload(config_path: &Path) {
    notify_gateway_reload_with_output(config_path, true).await;
}

async fn notify_gateway_reload_quiet(config_path: &Path) {
    notify_gateway_reload_with_output(config_path, false).await;
}

async fn notify_gateway_reload_with_output(config_path: &Path, print_success: bool) {
    let path_str = config_path.to_string_lossy();
    let Ok(gateway) = gateway_endpoint(&path_str) else {
        return;
    };
    let url = format!("{}/api/settings/reload", gateway.base_url);
    let response = gateway_request(
        reqwest::Client::new()
            .post(&url)
            .json(&json!({}))
            .timeout(std::time::Duration::from_secs(5)),
        &gateway,
    )
    .send()
    .await;
    match response {
        Ok(r) if r.status().is_success() => {
            if print_success {
                println!("Gateway config reloaded");
            }
        }
        Ok(r) => {
            eprintln!("warning: gateway reload returned HTTP {}", r.status());
        }
        Err(e) if e.is_connect() || e.is_timeout() => {}
        Err(e) => {
            eprintln!("warning: gateway reload failed: {e}");
        }
    }
}

pub(crate) async fn cmd_gateway_token(
    config_path: &str,
    rotate: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    let config_path = prepared.active_path;
    let loaded = load_config_or_default(
        config_path
            .to_str()
            .ok_or("config path must be valid UTF-8")?,
        ConfigRuntimeOverrides::default(),
    )?;
    let mut config = loaded.config;
    let existing = config.gateway.auth_token.trim().to_owned();

    let (token, changed) = if !existing.is_empty() && !rotate {
        (existing, false)
    } else {
        let token = format!("gx_{}", Uuid::new_v4().simple());
        config.gateway.auth_token = token.clone();
        let value = serde_json::to_value(&config)?;
        write_config_value_atomic(&config_path, &value, &ConfigWriteOptions::default())?;
        (token, true)
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "token": token,
                "changed": changed,
                "config_path": config_path.display().to_string(),
            }))?
        );
    } else {
        println!("{token}");
    }
    Ok(())
}

pub(crate) async fn cmd_gateway_install(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = build_service_spec(config_path)?;
    let manager = crate::service_manager::active_manager()?;
    let report = manager.install(&spec)?;
    crate::service_manager::wait_for_port(spec.port, Duration::from_secs(30)).await?;
    println!(
        "Gateway service installed via {}: {} (port {})",
        report.backend,
        report.unit_path.display(),
        spec.port
    );
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
    Ok(())
}

pub(crate) async fn cmd_gateway_uninstall() -> Result<(), Box<dyn std::error::Error>> {
    let manager = crate::service_manager::active_manager()?;
    manager.uninstall()?;
    println!("Gateway service uninstalled via {}", manager.backend_name());
    Ok(())
}

pub(crate) async fn cmd_gateway_start(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let manager = crate::service_manager::active_manager()?;
    if !manager.is_installed() {
        return Err(format!(
            "gateway service is not installed — run `garyx gateway install` first ({} backend)",
            manager.backend_name()
        )
        .into());
    }
    manager.start()?;
    crate::service_manager::wait_for_port(config.gateway.port, Duration::from_secs(30)).await?;
    println!(
        "Gateway service started via {} (port {})",
        manager.backend_name(),
        config.gateway.port
    );
    Ok(())
}

pub(crate) async fn cmd_gateway_restart(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = build_service_spec(config_path)?;
    let manager = crate::service_manager::active_manager()?;
    let report = manager.restart(&spec)?;
    crate::service_manager::wait_for_port(spec.port, Duration::from_secs(30)).await?;
    println!(
        "Gateway service restarted via {}: {} (port {})",
        report.backend,
        report.unit_path.display(),
        spec.port
    );
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
    Ok(())
}

pub(crate) async fn cmd_gateway_stop() -> Result<(), Box<dyn std::error::Error>> {
    let manager = crate::service_manager::active_manager()?;
    manager.stop()?;
    println!("Gateway service stopped via {}", manager.backend_name());
    Ok(())
}

/// Resolve every input the platform backend needs to render a unit / plist.
fn build_service_spec(
    config_path: &str,
) -> Result<crate::service_manager::ServiceSpec, Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let log_dir = crate::service_manager::log_dir_path()?;
    let binary_path = std::env::current_exe()?;
    Ok(crate::service_manager::ServiceSpec {
        binary_path,
        host: config.gateway.host.clone(),
        port: config.gateway.port,
        log_dir,
        workspace_root: detect_workspace_root(),
    })
}

/// If the CLI was invoked from a garyx repo checkout, return its root so the
/// managed service can inherit local development context when needed.
fn detect_workspace_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("Cargo.toml").exists() && cwd.join("garyx").is_dir() {
        Some(cwd)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct GatewayEndpoint {
    base_url: String,
    auth_token: Option<String>,
}

fn gateway_endpoint(config_path: &str) -> Result<GatewayEndpoint, Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let public_url = config.gateway.public_url.trim();
    let base_url = if !public_url.is_empty() {
        public_url.trim_end_matches('/').to_owned()
    } else {
        let host = if config.gateway.host == "0.0.0.0" {
            "127.0.0.1".to_owned()
        } else {
            config.gateway.host
        };
        format!("http://{}:{}", host, config.gateway.port)
    };
    let auth_token = (!config.gateway.auth_token.trim().is_empty())
        .then(|| config.gateway.auth_token.trim().to_owned());
    Ok(GatewayEndpoint {
        base_url,
        auth_token,
    })
}

#[cfg(test)]
fn gateway_base_url(config_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(gateway_endpoint(config_path)?.base_url)
}

fn gateway_request(
    mut builder: reqwest::RequestBuilder,
    gateway: &GatewayEndpoint,
) -> reqwest::RequestBuilder {
    if let Some(token) = gateway.auth_token.as_deref() {
        builder = builder.bearer_auth(token);
    }
    builder
}

fn is_local_gateway_url(base_url: &str) -> bool {
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "0.0.0.0" | "::1" | "[::1]")
    )
}

fn build_provider_metadata_for_local_gateway(base_url: &str) -> Option<Value> {
    if !is_local_gateway_url(base_url) {
        return None;
    }

    let mut metadata = serde_json::Map::new();

    if let Ok(token) = std::env::var(CLAUDE_OAUTH_ENV) {
        let token = token.trim();
        if !token.is_empty() {
            metadata.insert(
                CLAUDE_ENV_METADATA_KEY.to_owned(),
                json!({ CLAUDE_OAUTH_ENV: token }),
            );
        }
    }

    if let Ok(api_key) = std::env::var(CODEX_API_KEY_ENV) {
        let api_key = api_key.trim();
        if !api_key.is_empty() {
            metadata.insert(
                CODEX_ENV_METADATA_KEY.to_owned(),
                json!({ CODEX_API_KEY_ENV: api_key }),
            );
        }
    }

    (!metadata.is_empty()).then(|| Value::Object(metadata))
}

async fn fetch_gateway_json(
    gateway: &GatewayEndpoint,
    path_and_query: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path_and_query);
    let response = gateway_request(reqwest::Client::new().get(&url), gateway)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

fn print_pretty_json(value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

async fn post_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().post(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

async fn post_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().post(&url), gateway)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

async fn patch_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().patch(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

async fn patch_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().patch(&url), gateway)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

async fn put_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().put(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

async fn delete_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().delete(&url), gateway)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

async fn delete_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().delete(&url), gateway)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&body)?)
}

fn print_agent_team_summary(team: &Value) {
    let team_id = team["team_id"].as_str().unwrap_or("-");
    let name = team["display_name"].as_str().unwrap_or("-");
    let leader = team["leader_agent_id"].as_str().unwrap_or("-");
    let members = team["member_agent_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.as_str().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    println!("Team: {team_id}");
    println!("Name: {name}");
    println!("Leader: {leader}");
    println!("Members: {}", members.join(", "));
    if let Some(workflow) = team["workflow_text"].as_str() {
        println!("Workflow: {workflow}");
    }
}

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

fn print_auto_research_run_summary(run: &Value, latest_iteration: Option<&Value>) {
    let run_id = run["run_id"].as_str().unwrap_or("-");
    let state = run["state"].as_str().unwrap_or("-");
    let goal = run["goal"].as_str().unwrap_or("-");
    println!("Run: {run_id}");
    println!("State: {state}");
    println!("Goal: {goal}");
    println!(
        "Iterations: {}/{}",
        run["iterations_used"].as_u64().unwrap_or(0),
        run["max_iterations"].as_u64().unwrap_or(0)
    );
    if let Some(reason) = run["terminal_reason"].as_str() {
        println!("Terminal reason: {reason}");
    }
    if let Some(iteration) = latest_iteration {
        println!(
            "Latest iteration: {}",
            iteration["iteration_index"].as_u64().unwrap_or(0)
        );
    }
    // Verdict lives on candidates, not iterations
    let candidates = run["candidates"].as_array();
    if let Some(best) = candidates.and_then(|cs| {
        cs.iter()
            .filter(|c| c["verdict"]["score"].as_f64().is_some())
            .max_by(|a, b| {
                a["verdict"]["score"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .partial_cmp(&b["verdict"]["score"].as_f64().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }) {
        if let Some(s) = best["verdict"]["score"].as_f64() {
            println!("Best score: {s:.1}");
        }
        if let Some(text) = best["verdict"]["feedback"].as_str() {
            println!("Best feedback: {text}");
        }
    }
}

pub(crate) async fn cmd_auto_research_create(
    config_path: &str,
    goal: String,
    workspace_dir: Option<String>,
    max_iterations: u32,
    time_budget_secs: u64,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let goal = goal.trim().to_owned();
    if goal.is_empty() {
        return Err("goal cannot be empty".into());
    }
    let workspace_dir = workspace_dir
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let workspace_dir = workspace_dir
        .canonicalize()
        .unwrap_or(workspace_dir)
        .display()
        .to_string();

    let gateway = gateway_endpoint(config_path)?;
    let mut payload = json!({
        "goal": goal,
        "workspace_dir": workspace_dir,
        "max_iterations": max_iterations,
        "time_budget_secs": time_budget_secs,
    });
    if let Some(provider_metadata) = build_provider_metadata_for_local_gateway(&gateway.base_url) {
        payload["provider_metadata"] = provider_metadata;
    }
    let created = post_gateway_json(&gateway, "/api/auto-research/runs", &payload).await?;
    let run_id = created["run_id"]
        .as_str()
        .ok_or("missing run_id in create response")?
        .to_owned();
    if json {
        return print_pretty_json(&created);
    }
    println!("Auto Research started: {run_id}");
    print_auto_research_run_summary(&created, None);
    Ok(())
}

pub(crate) async fn cmd_auto_research_get(
    config_path: &str,
    run_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/auto-research/runs/{}", urlencoding::encode(run_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_auto_research_run_summary(&payload["run"], payload.get("latest_iteration"));
    Ok(())
}

pub(crate) async fn cmd_auto_research_iterations(
    config_path: &str,
    run_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/auto-research/runs/{}/iterations",
            urlencoding::encode(run_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Run: {run_id}");
    let items = payload["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        println!("Iterations: (none)");
        return Ok(());
    }
    for item in items {
        let iteration_index = item["iteration_index"].as_u64().unwrap_or(0);
        let state = item["state"].as_str().unwrap_or("-");
        println!("  - #{iteration_index}  {state}");
    }
    Ok(())
}

pub(crate) async fn cmd_auto_research_stop(
    config_path: &str,
    run_id: &str,
    reason: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        &format!(
            "/api/auto-research/runs/{}/stop",
            urlencoding::encode(run_id)
        ),
        &json!({ "reason": reason }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_auto_research_run_summary(&payload, None);
    Ok(())
}

pub(crate) async fn cmd_auto_research_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/auto-research/runs").await?;
    if json {
        return print_pretty_json(&payload);
    }
    let items = payload["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        println!("No Auto Research runs found.");
        return Ok(());
    }
    println!(
        "{:<38}  {:<16}  {:>5}  {:<40}",
        "RUN ID", "STATE", "ITER", "GOAL"
    );
    println!("{}", "-".repeat(105));
    for item in &items {
        let run_id = item["run_id"].as_str().unwrap_or("-");
        let state = item["state"].as_str().unwrap_or("-");
        let max_iter = item["max_iterations"].as_u64().unwrap_or(0);
        let iterations_used = item["iterations_used"].as_u64().unwrap_or(0);
        let goal = item["goal"].as_str().unwrap_or("-");
        let goal_truncated: String = goal.chars().take(40).collect();
        println!("{run_id:<38}  {state:<16}  {iterations_used:>2}/{max_iter:<2}  {goal_truncated}",);
    }
    println!("\n{} run(s) total", items.len());
    Ok(())
}

pub(crate) async fn cmd_auto_research_candidates(
    config_path: &str,
    run_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/auto-research/runs/{}/candidates",
            urlencoding::encode(run_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    let candidates = payload["candidates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if candidates.is_empty() {
        println!("No candidates for run {run_id}.");
        return Ok(());
    }
    let best = payload["best_candidate_id"].as_str().unwrap_or("");
    println!("Run: {run_id}");
    if !best.is_empty() {
        println!("Best: {best}");
    }
    println!();
    println!(
        "{:<8}  {:>6}  {:>8}  {}",
        "ID", "SCORE", "ITER", "OUTPUT (truncated)"
    );
    println!("{}", "-".repeat(90));
    for c in &candidates {
        let cid = c["candidate_id"].as_str().unwrap_or("-");
        let score = c["verdict"]["score"]
            .as_f64()
            .map(|s| format!("{s:.1}"))
            .unwrap_or_else(|| "-".to_string());
        let iter = c["iteration"].as_u64().unwrap_or(0);
        let output = c["output"].as_str().unwrap_or("-");
        let output_truncated: String = output.chars().take(60).collect();
        println!("{cid:<8}  {score:>6}  {iter:>8}  {output_truncated}");
    }
    Ok(())
}

pub(crate) async fn cmd_auto_research_patch(
    config_path: &str,
    run_id: &str,
    max_iterations: Option<u32>,
    time_budget_secs: Option<u64>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    if max_iterations.is_none() && time_budget_secs.is_none() {
        return Err("at least one of --max-iterations, --time-budget-secs must be provided".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let mut patch = serde_json::Map::new();
    if let Some(v) = max_iterations {
        patch.insert("max_iterations".into(), json!(v));
    }
    if let Some(v) = time_budget_secs {
        patch.insert("time_budget_secs".into(), json!(v));
    }
    let payload = patch_gateway_json(
        &gateway,
        &format!("/api/auto-research/runs/{}", urlencoding::encode(run_id)),
        &Value::Object(patch),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_auto_research_run_summary(&payload, None);
    Ok(())
}

pub(crate) async fn cmd_auto_research_feedback(
    config_path: &str,
    run_id: &str,
    message: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        &format!(
            "/api/auto-research/runs/{}/feedback",
            urlencoding::encode(run_id)
        ),
        &json!({ "message": message }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("✓ Feedback injected into run {run_id}");
    print_auto_research_run_summary(&payload, None);
    Ok(())
}

pub(crate) async fn cmd_auto_research_reverify(
    config_path: &str,
    run_id: &str,
    candidate_id: &str,
    guidance: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        &format!(
            "/api/auto-research/runs/{}/reverify",
            urlencoding::encode(run_id)
        ),
        &json!({ "candidate_id": candidate_id, "guidance": guidance }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("✓ Re-verify requested for candidate {candidate_id} in run {run_id}");
    print_auto_research_run_summary(&payload, None);
    Ok(())
}

pub(crate) async fn cmd_auto_research_select(
    config_path: &str,
    run_id: &str,
    candidate_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        &format!(
            "/api/auto-research/runs/{}/select/{}",
            urlencoding::encode(run_id),
            urlencoding::encode(candidate_id),
        ),
        &json!({}),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("✓ Candidate {candidate_id} selected for run {run_id}");
    print_auto_research_run_summary(&payload, None);
    Ok(())
}

// ---------------------------------------------------------------------------
// Custom Agent commands
// ---------------------------------------------------------------------------

pub(crate) async fn cmd_agent_list(
    config_path: &str,
    include_builtin: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/custom-agents").await?;
    if json {
        return print_pretty_json(&payload);
    }
    let agents = payload["agents"].as_array().cloned().unwrap_or_default();
    let visible: Vec<&Value> = agents
        .iter()
        .filter(|a| include_builtin || a["built_in"].as_bool() != Some(true))
        .collect();
    if visible.is_empty() {
        println!("Agents: (none)");
        return Ok(());
    }
    for a in visible {
        print_agent_summary(a);
        println!();
    }
    Ok(())
}

pub(crate) async fn cmd_agent_get(
    config_path: &str,
    agent_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/custom-agents/{}", urlencoding::encode(agent_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

fn build_agent_mutation_body(
    agent_id: String,
    display_name: String,
    provider: String,
    model: Option<String>,
    system_prompt: String,
) -> Result<Value, Box<dyn std::error::Error>> {
    let agent_id = agent_id.trim().to_owned();
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".into());
    }
    Ok(json!({
        "agent_id": agent_id,
        "display_name": display_name.trim(),
        "provider_type": provider.trim(),
        "model": model.as_deref().map(str::trim).unwrap_or(""),
        "system_prompt": system_prompt,
    }))
}

pub(crate) async fn cmd_agent_create(
    config_path: &str,
    agent_id: String,
    display_name: String,
    provider: String,
    model: Option<String>,
    system_prompt: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let body = build_agent_mutation_body(agent_id, display_name, provider, model, system_prompt)?;
    let payload = post_gateway_json(&gateway, "/api/custom-agents", &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_update(
    config_path: &str,
    agent_id: String,
    display_name: String,
    provider: String,
    model: Option<String>,
    system_prompt: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let body = build_agent_mutation_body(
        agent_id.clone(),
        display_name,
        provider,
        model,
        system_prompt,
    )?;
    let url = format!(
        "/api/custom-agents/{}",
        urlencoding::encode(agent_id.trim())
    );
    let payload = put_gateway_json(&gateway, &url, &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_upsert(
    config_path: &str,
    agent_id: String,
    display_name: String,
    provider: String,
    model: Option<String>,
    system_prompt: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let body = build_agent_mutation_body(
        agent_id.clone(),
        display_name,
        provider,
        model,
        system_prompt,
    )?;
    let url = format!(
        "/api/custom-agents/{}",
        urlencoding::encode(agent_id.trim())
    );
    let payload = match put_gateway_json(&gateway, &url, &body).await {
        Ok(p) => p,
        Err(_) => post_gateway_json(&gateway, "/api/custom-agents", &body).await?,
    };
    if json {
        return print_pretty_json(&payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_delete(
    config_path: &str,
    agent_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json(
        &gateway,
        &format!("/api/custom-agents/{}", urlencoding::encode(agent_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Deleted agent: {agent_id}");
    Ok(())
}

fn print_agent_summary(a: &Value) {
    let agent_id = a["agent_id"].as_str().unwrap_or("-");
    let name = a["display_name"].as_str().unwrap_or("-");
    let provider = a["provider_type"].as_str().unwrap_or("-");
    let model = a["model"].as_str().unwrap_or("").trim();
    let builtin = a["built_in"].as_bool().unwrap_or(false);
    println!(
        "Agent: {agent_id}{}",
        if builtin { " (built-in)" } else { "" }
    );
    println!("Name: {name}");
    println!("Provider: {provider}");
    if !model.is_empty() {
        println!("Model: {model}");
    }
    if let Some(prompt) = a["system_prompt"].as_str() {
        let preview: String = prompt.chars().take(120).collect();
        let ellipsis = if prompt.len() > 120 { "…" } else { "" };
        println!("Prompt: {preview}{ellipsis}");
    }
}

// ---------------------------------------------------------------------------
// Team commands
// ---------------------------------------------------------------------------

pub(crate) async fn cmd_agent_team_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/teams").await?;
    if json {
        return print_pretty_json(&payload);
    }
    let items = payload["teams"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        println!("Teams: (none)");
        return Ok(());
    }
    for item in items {
        print_agent_team_summary(&item);
        println!();
    }
    Ok(())
}

pub(crate) async fn cmd_agent_team_get(
    config_path: &str,
    team_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let team_id = team_id.trim();
    if team_id.is_empty() {
        return Err("team_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/teams/{}", urlencoding::encode(team_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_agent_team_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_team_create(
    config_path: &str,
    team_id: String,
    display_name: String,
    leader_agent_id: String,
    member_agent_ids: Vec<String>,
    workflow_text: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let team_id = team_id.trim().to_owned();
    let display_name = display_name.trim().to_owned();
    let leader_agent_id = leader_agent_id.trim().to_owned();
    let workflow_text = workflow_text.trim().to_owned();
    let member_agent_ids = member_agent_ids
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if team_id.is_empty() {
        return Err("team_id cannot be empty".into());
    }
    if display_name.is_empty() {
        return Err("display_name cannot be empty".into());
    }
    if leader_agent_id.is_empty() {
        return Err("leader_agent_id cannot be empty".into());
    }
    if workflow_text.is_empty() {
        return Err("workflow_text cannot be empty".into());
    }
    if member_agent_ids.is_empty() {
        return Err("member_agent_ids cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        "/api/teams",
        &json!({
            "teamId": team_id,
            "displayName": display_name,
            "leaderAgentId": leader_agent_id,
            "memberAgentIds": member_agent_ids,
            "workflowText": workflow_text,
        }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_agent_team_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_team_update(
    config_path: &str,
    team_id: String,
    new_team_id: Option<String>,
    display_name: String,
    leader_agent_id: String,
    member_agent_ids: Vec<String>,
    workflow_text: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let team_id = team_id.trim().to_owned();
    if team_id.is_empty() {
        return Err("team_id cannot be empty".into());
    }
    let next_team_id = new_team_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| team_id.clone());
    let display_name = display_name.trim().to_owned();
    let leader_agent_id = leader_agent_id.trim().to_owned();
    let workflow_text = workflow_text.trim().to_owned();
    let member_agent_ids = member_agent_ids
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if display_name.is_empty() {
        return Err("display_name cannot be empty".into());
    }
    if leader_agent_id.is_empty() {
        return Err("leader_agent_id cannot be empty".into());
    }
    if workflow_text.is_empty() {
        return Err("workflow_text cannot be empty".into());
    }
    if member_agent_ids.is_empty() {
        return Err("member_agent_ids cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = put_gateway_json(
        &gateway,
        &format!("/api/teams/{}", urlencoding::encode(&team_id)),
        &json!({
            "teamId": next_team_id,
            "displayName": display_name,
            "leaderAgentId": leader_agent_id,
            "memberAgentIds": member_agent_ids,
            "workflowText": workflow_text,
        }),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_agent_team_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_team_delete(
    config_path: &str,
    team_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let team_id = team_id.trim();
    if team_id.is_empty() {
        return Err("team_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json(
        &gateway,
        &format!("/api/teams/{}", urlencoding::encode(team_id)),
    )
    .await?;
    if json {
        let payload =
            if payload.is_object() && payload.as_object().is_some_and(|value| value.is_empty()) {
                json!({
                    "deleted": true,
                    "team_id": team_id,
                })
            } else {
                payload
            };
        return print_pretty_json(&payload);
    }
    println!("Deleted team: {team_id}");
    if payload["deleted"] == Value::Bool(true) {
        return Ok(());
    }
    Ok(())
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
    task_ref: String,
    message: String,
    workspace_dir: Option<String>,
    timeout_secs: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/tasks/{}", encode_task_ref(&task_ref)?),
    )
    .await?;
    let thread_id = payload
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("task '{task_ref}' did not resolve to a thread"))?
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
                            if matches!(event_type, "done" | "complete" | "error") {
                                break;
                            }
                            continue;
                        }
                        match event_type {
                            "assistant_delta" => {
                                if !response_started {
                                    response_started = true;
                                }
                                if let Some(delta) = event["delta"].as_str() {
                                    print!("{delta}");
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

pub(crate) async fn cmd_task_list(
    config_path: &str,
    scope: Option<&str>,
    status: Option<&str>,
    assignee: Option<&str>,
    include_done: bool,
    limit: usize,
    offset: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut params = vec![
        ("limit".to_owned(), limit.clamp(1, 200).to_string()),
        ("offset".to_owned(), offset.to_string()),
    ];
    if let Some(scope) = scope {
        params.push(("scope".to_owned(), normalize_scope_query(scope)?));
    }
    if let Some(status) = status {
        params.push(("status".to_owned(), normalize_task_status(status)?));
    }
    if let Some(assignee) = assignee.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("assignee".to_owned(), assignee.to_owned()));
    }
    if include_done {
        params.push(("include_done".to_owned(), "true".to_owned()));
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
        return Ok(());
    }
    for task in tasks {
        print_task_summary(&task);
        println!();
    }
    if payload["has_more"].as_bool().unwrap_or(false) {
        println!("More tasks available; increase --offset to continue.");
    }
    Ok(())
}

pub(crate) async fn cmd_task_get(
    config_path: &str,
    task_ref: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/tasks/{}", encode_task_ref(task_ref)?),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    let history_payload = payload
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|thread_id| async move {
            fetch_gateway_json(
                &gateway,
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
    scope: Option<&str>,
    title: Option<String>,
    body: Option<String>,
    assignee: Option<&str>,
    start: bool,
    agent_id: Option<String>,
    workspace_dir: Option<String>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let assignee = assignee.map(principal_payload).transpose()?;
    let agent_id = agent_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let workspace_dir = workspace_dir
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let gateway = gateway_endpoint(config_path)?;
    let mut request = json!({
        "title": title,
        "body": body,
        "assignee": assignee,
        "start": start,
        "runtime": {
            "agent_id": agent_id,
            "workspace_dir": workspace_dir,
        },
    });
    if let Some(scope) = scope {
        request["scope"] = scope_payload(scope)?;
    }
    let payload = post_gateway_json_as_cli_actor(&gateway, "/api/tasks", &request).await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_task_promote(
    config_path: &str,
    thread_id: &str,
    title: Option<String>,
    assignee: Option<&str>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(
        &gateway,
        "/api/tasks/promote",
        &json!({
            "thread_id": thread_id,
            "title": title,
            "assignee": assignee.map(principal_payload).transpose()?,
        }),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_task_claim(
    config_path: &str,
    task_ref: &str,
    actor: Option<&str>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let encoded_ref = encode_task_ref(task_ref)?;
    let assignee = actor
        .map(principal_payload)
        .transpose()?
        .unwrap_or_else(cli_actor_payload);
    let assign_path = format!("/api/tasks/{encoded_ref}/assign");
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &assign_path,
        &json!({
            "to": assignee.clone(),
        }),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_task_release(
    config_path: &str,
    task_ref: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let encoded_ref = encode_task_ref(task_ref)?;
    let status_path = format!("/api/tasks/{encoded_ref}/status");
    let assign_path = format!("/api/tasks/{encoded_ref}/assign");
    patch_gateway_json_as_cli_actor(
        &gateway,
        &status_path,
        &json!({
            "to": "todo",
            "note": Value::Null,
            "force": false,
        }),
    )
    .await?;
    let payload = delete_gateway_json_as_cli_actor(&gateway, &assign_path)
        .await
        .map_err(|error| format!("status moved to todo but unassign failed: {error}"))?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_task_assign(
    config_path: &str,
    task_ref: &str,
    principal: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/assign", encode_task_ref(task_ref)?),
        &json!({
            "to": principal_payload(principal)?,
        }),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_task_unassign(
    config_path: &str,
    task_ref: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/assign", encode_task_ref(task_ref)?),
    )
    .await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_task_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_task_update(
    config_path: &str,
    task_ref: &str,
    status: &str,
    note: Option<String>,
    force: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    patch_task_status(config_path, task_ref, status, note, force, json_output).await
}

pub(crate) async fn cmd_task_reopen(
    config_path: &str,
    task_ref: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    patch_task_status(config_path, task_ref, "todo", None, false, json_output).await
}

pub(crate) async fn cmd_task_set_title(
    config_path: &str,
    task_ref: &str,
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
        &format!("/api/tasks/{}/title", encode_task_ref(task_ref)?),
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
    task_ref: &str,
    limit: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/tasks/{}/history?limit={}",
            encode_task_ref(task_ref)?,
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
    task_ref: &str,
    status: &str,
    note: Option<String>,
    force: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/status", encode_task_ref(task_ref)?),
        &json!({
            "to": normalize_task_status(status)?,
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

fn normalize_scope_query(scope: &str) -> Result<String, Box<dyn std::error::Error>> {
    let scope = scope.trim().to_ascii_lowercase();
    if scope_payload(&scope).is_err() {
        return Err("scope must be <channel>/<account_id>".into());
    }
    Ok(scope)
}

fn scope_payload(scope: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let parts = scope
        .trim()
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err("scope must be <channel>/<account_id>".into());
    }
    Ok(json!({
        "channel": parts[0].to_ascii_lowercase(),
        "account_id": parts[1].to_ascii_lowercase(),
    }))
}

fn principal_payload(principal: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let principal = principal.trim();
    if principal.is_empty() {
        return Err("principal cannot be empty".into());
    }
    if let Some(user_id) = principal.strip_prefix("human:") {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            return Err("human principal cannot be empty".into());
        }
        return Ok(json!({ "kind": "human", "user_id": user_id }));
    }
    if let Some(agent_id) = principal.strip_prefix("agent:") {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err("agent principal cannot be empty".into());
        }
        return Ok(json!({ "kind": "agent", "agent_id": agent_id }));
    }
    Ok(json!({ "kind": "agent", "agent_id": principal }))
}

fn cli_actor_payload() -> Value {
    if let Some(actor) = env_nonempty("GARYX_ACTOR") {
        return principal_payload(&actor)
            .unwrap_or_else(|_| json!({ "kind": "human", "user_id": cli_actor_user_id() }));
    }
    if let Some(agent_id) = env_nonempty("GARYX_AGENT_ID") {
        return json!({ "kind": "agent", "agent_id": agent_id });
    }
    json!({ "kind": "human", "user_id": cli_actor_user_id() })
}

fn cli_actor_user_id() -> String {
    env_nonempty("GARYX_USER").unwrap_or_else(|| "owner".to_owned())
}

fn cli_actor_header_value() -> String {
    format_principal(&cli_actor_payload())
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
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

fn encode_task_ref(task_ref: &str) -> Result<String, Box<dyn std::error::Error>> {
    let task_ref = task_ref.trim();
    if task_ref.is_empty() {
        return Err("task_ref cannot be empty".into());
    }
    Ok(urlencoding::encode(task_ref).into_owned())
}

fn print_task_summary(value: &Value) {
    let task = value.get("task").unwrap_or(value);
    let task_ref = value
        .get("task_ref")
        .and_then(Value::as_str)
        .or_else(|| task.get("task_ref").and_then(Value::as_str))
        .unwrap_or("-");
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
    let scope_label = task
        .get("scope")
        .or_else(|| value.get("scope"))
        .and_then(|scope| {
            Some(format!(
                "{}/{}",
                scope.get("channel")?.as_str()?,
                scope.get("account_id")?.as_str()?
            ))
        })
        .unwrap_or_else(|| "-".to_owned());
    let unassigned = Value::Null;
    let assignee = task
        .get("assignee")
        .or_else(|| value.get("assignee"))
        .unwrap_or(&unassigned);
    println!("Task: {task_ref}");
    println!("Title: {title}");
    println!("Status: {status}");
    println!("Scope: {scope_label}");
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
    let task_ref = task_payload
        .get("task_ref")
        .and_then(Value::as_str)
        .or_else(|| task.get("task_ref").and_then(Value::as_str))
        .unwrap_or("-");
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
    let _ = writeln!(&mut output, "Task: {task_ref}");
    let _ = writeln!(&mut output, "Title: {title}");
    let _ = writeln!(&mut output, "Status: {status}");
    let _ = writeln!(&mut output, "Assignee: {}", format_principal(assignee));
    let _ = writeln!(&mut output, "Updated by: {}", format_principal(updated_by));
    if thread_id != "-" {
        let _ = writeln!(&mut output, "Thread: {thread_id}");
    }
    output.push('\n');
    output.push_str("Progress:\n");

    let messages = task_progress_messages(task_payload, history_payload);
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
            "\nFull thread with tool calls: garyx debug thread {thread_id} --limit 200 --json"
        );
    }
    output
}

fn turn_timestamp_label(turn: &TaskProgressTurn) -> String {
    turn.user_timestamp
        .as_deref()
        .map(|timestamp| format!(" {timestamp}"))
        .unwrap_or_default()
}

fn task_progress_messages(
    task_payload: &Value,
    history_payload: Option<&Value>,
) -> Vec<TaskProgressMessage> {
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

    if let Some(thread_messages) = task_payload
        .get("thread")
        .and_then(|thread| thread.get("messages"))
        .and_then(Value::as_array)
    {
        for message in thread_messages {
            if let Some(entry) = task_progress_message_from_thread(message, source_order) {
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

fn task_progress_message_from_thread(
    value: &Value,
    source_order: usize,
) -> Option<TaskProgressMessage> {
    let role = value
        .get("role")
        .and_then(Value::as_str)?
        .trim()
        .to_ascii_lowercase();
    let text = message_text_from_value(value).unwrap_or_default();
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
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

fn format_principal(value: &Value) -> String {
    if value.is_null() {
        return "(unassigned)".to_owned();
    }
    match value
        .get("kind")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("-")
    {
        "human" => format!(
            "human:{}",
            value.get("user_id").and_then(Value::as_str).unwrap_or("-")
        ),
        "agent" => format!(
            "agent:{}",
            value.get("agent_id").and_then(Value::as_str).unwrap_or("-")
        ),
        other => other.to_owned(),
    }
}

pub(crate) async fn cmd_debug_thread(
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
            "/api/debug/thread?thread_id={}&limit={}",
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
        let updated_at = format_local_debug_timestamp(active_run["updated_at"].as_str());
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
            let updated_at = format_local_debug_timestamp(record["updated_at"].as_str());
            let excerpt = record["text_excerpt"].as_str().unwrap_or("");
            println!("  - {updated_at}  {status}  reason={reason}  {excerpt}");
        }
    }
    Ok(())
}

pub(crate) async fn cmd_bot_status(
    config_path: &str,
    bot_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bot_id = bot_id.trim();
    if bot_id.is_empty() {
        return Err("bot_id cannot be empty".into());
    }

    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/bot/status?bot_id={}", urlencoding::encode(bot_id)),
    )
    .await?;

    if json {
        return print_pretty_json(&payload);
    }
    if !payload["ok"].as_bool().unwrap_or(false) {
        let message = payload["error"]
            .as_str()
            .or_else(|| payload["reason"].as_str())
            .unwrap_or("bot status failed");
        return Err(message.into());
    }

    println!("Bot: {bot_id}");
    println!(
        "Main endpoint: {}",
        payload["main_endpoint_status"]
            .as_str()
            .unwrap_or("unknown")
    );
    println!(
        "Current thread status: {}",
        payload["current_thread_status"]
            .as_str()
            .unwrap_or("unknown")
    );
    println!(
        "Current thread: {}",
        payload["current_thread_id"].as_str().unwrap_or("-")
    );
    if let Some(workspace_dir) = payload["main_endpoint"]["workspace_dir"].as_str() {
        if !workspace_dir.trim().is_empty() {
            println!("Workspace: {workspace_dir}");
        }
    }
    if let Some(binding_key) = payload["main_endpoint"]["binding_key"].as_str() {
        if !binding_key.trim().is_empty() {
            println!("Binding key: {binding_key}");
        }
    }
    println!(
        "Provider: {}",
        payload["thread_runtime"]["provider_label"]
            .as_str()
            .unwrap_or("-")
    );
    let active_run = &payload["thread_runtime"]["active_run"];
    if active_run.is_null() {
        println!("Active run: -");
    } else {
        println!(
            "Active run: {}",
            active_run["run_id"].as_str().unwrap_or("-")
        );
    }
    println!("Send command: garyx thread send bot {bot_id} <message>");

    Ok(())
}

fn format_local_debug_timestamp(value: Option<&str>) -> String {
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
        "claude_code" => "Claude",
        _ => "-",
    }
}

pub(crate) async fn cmd_doctor(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let config_path = prepared.active_path;
    let config_exists = config_path.exists();
    let config_path_display = config_path.to_string_lossy().to_string();
    let claude_available = which("claude");
    let codex_available = which("codex");

    let checks = [
        ("config_file", config_path_display.as_str(), config_exists),
        ("claude_binary", "claude", claude_available),
        ("codex_binary", "codex", codex_available),
    ];

    if json {
        let obj: serde_json::Value = checks
            .iter()
            .map(|(name, _, ok)| (name.to_string(), serde_json::json!({ "ok": ok })))
            .collect::<serde_json::Map<String, serde_json::Value>>()
            .into();
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        for (name, detail, ok) in &checks {
            let mark = if *ok { "ok" } else { "MISSING" };
            println!("[{}] {} ({})", mark, name, detail);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretPromptUpdate {
    Keep,
    Clear,
    Set,
}

#[derive(Debug, Serialize)]
struct OnboardSummary {
    ok: bool,
    config_path: String,
    created_config: bool,
    api_account: String,
    api_account_created: bool,
    search_api_key_configured: bool,
    image_gen_api_key_configured: bool,
    conversation_index_enabled: bool,
    conversation_index_api_key_configured: bool,
    conversation_index_model: String,
    conversation_index_base_url: String,
    gateway_run_requested: bool,
    /// `channel.account` identifiers bound during this onboarding session.
    channels_bound: Vec<String>,
    /// Total account count across all user-facing channels (excludes `api`).
    total_user_channel_accounts: usize,
    next_steps: Vec<String>,
}

fn stdin_is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn trim_to_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn prompt_line(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_owned())
}

fn prompt_secret_line(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    if stdin_is_interactive() {
        Ok(rpassword::prompt_password(prompt)?.trim().to_owned())
    } else {
        prompt_line(prompt)
    }
}

fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        let value = prompt_line(&format!("{prompt} {suffix} "))?;
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Ok(default);
        }
        match normalized.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                println!("Please answer y or n.");
            }
        }
    }
}

fn prompt_secret_update(
    label: &str,
    configured: bool,
) -> Result<(SecretPromptUpdate, Option<String>), Box<dyn std::error::Error>> {
    let prompt = if configured {
        format!("{label} (Enter keeps current, '-' clears): ")
    } else {
        format!("{label} (optional, Enter skips): ")
    };
    let value = prompt_secret_line(&prompt)?;
    if value.is_empty() {
        return Ok((SecretPromptUpdate::Keep, None));
    }
    if value == "-" {
        return Ok((SecretPromptUpdate::Clear, None));
    }
    Ok((SecretPromptUpdate::Set, Some(value)))
}

fn ensure_onboard_api_account(config: &mut GaryxConfig, account_id: &str) -> bool {
    let account_id = account_id.trim();
    if let Some(account) = config.channels.api.accounts.get_mut(account_id) {
        account.enabled = true;
        return false;
    }
    config.channels.api.accounts.insert(
        account_id.to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );
    true
}

fn user_channel_account_count(cfg: &GaryxConfig) -> usize {
    cfg.channels
        .plugins
        .values()
        .map(|plugin_cfg| plugin_cfg.accounts.len())
        .sum::<usize>()
}

fn next_onboard_steps(cfg: &GaryxConfig) -> Vec<String> {
    let mut steps = Vec::new();
    // If the user skipped channel binding entirely, nudge them back to
    // `garyx channels add` so they don't end up with a gateway that has
    // nothing to talk to.
    if user_channel_account_count(cfg) == 0 {
        steps.push("garyx channels add  # 绑定至少一个聊天渠道".to_owned());
    }
    steps.push("garyx config show".to_owned());
    steps.push("garyx doctor".to_owned());
    steps.push("garyx gateway install  # 安装并启动后台 gateway".to_owned());
    steps
}

fn print_onboard_summary(summary: &OnboardSummary) {
    println!("Onboarding complete.");
    println!("Config: {}", summary.config_path);
    if summary.api_account_created {
        println!("API account: {} (created)", summary.api_account);
    } else {
        println!("API account: {} (enabled)", summary.api_account);
    }
    println!(
        "Search API key: {}",
        if summary.search_api_key_configured {
            "configured"
        } else {
            "missing"
        }
    );
    println!(
        "Image generation API key: {}",
        if summary.image_gen_api_key_configured {
            "configured"
        } else {
            "missing"
        }
    );
    println!(
        "Conversation index: {} ({}, model={}, base_url={})",
        if summary.conversation_index_enabled {
            "enabled"
        } else {
            "disabled"
        },
        if summary.conversation_index_api_key_configured {
            "key configured"
        } else {
            "key missing"
        },
        summary.conversation_index_model,
        summary.conversation_index_base_url,
    );
    if summary.channels_bound.is_empty() {
        println!(
            "User-facing channels: {} configured (none bound this session)",
            summary.total_user_channel_accounts
        );
    } else {
        println!(
            "User-facing channels: {} configured (bound this session: {})",
            summary.total_user_channel_accounts,
            summary.channels_bound.join(", "),
        );
    }
}

pub(crate) async fn cmd_onboard(
    config_path: &str,
    options: OnboardCommandOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let config_path = prepared.active_path;
    let existed_before = config_path.exists();
    let created_config = options.force || !existed_before;
    if created_config {
        let default_value = serde_json::to_value(GaryxConfig::default())?;
        write_config_value_atomic(&config_path, &default_value, &ConfigWriteOptions::default())?;
    }

    let loaded = load_config_or_default(
        &config_path.to_string_lossy(),
        ConfigRuntimeOverrides::default(),
    )?;
    print_diagnostics(&loaded.diagnostics);
    let mut cfg = loaded.config;

    let api_account =
        trim_to_option(Some(options.api_account.as_str())).unwrap_or_else(|| "main".to_owned());
    let api_account_created = ensure_onboard_api_account(&mut cfg, &api_account);

    if let Some(value) = trim_to_option(options.search_api_key.as_deref()) {
        cfg.gateway.search.api_key = value;
    }
    if let Some(value) = trim_to_option(options.image_gen_api_key.as_deref()) {
        cfg.gateway.image_gen.api_key = value;
    }
    let explicit_conversation_key = trim_to_option(options.conversation_index_api_key.as_deref());
    if let Some(value) = explicit_conversation_key.clone() {
        cfg.gateway.conversation_index.api_key = value;
        cfg.gateway.conversation_index.enabled = true;
    }
    if let Some(value) = trim_to_option(options.conversation_index_model.as_deref()) {
        cfg.gateway.conversation_index.model = value;
    }
    if let Some(value) = trim_to_option(options.conversation_index_base_url.as_deref()) {
        cfg.gateway.conversation_index.base_url = value;
    }
    if options.enable_conversation_index {
        cfg.gateway.conversation_index.enabled = true;
    }
    if options.disable_conversation_index {
        cfg.gateway.conversation_index.enabled = false;
    }

    let interactive = !options.json && stdin_is_interactive();
    let mut channels_bound: Vec<String> = Vec::new();
    if interactive {
        println!("Garyx onboarding");
        if created_config {
            println!("Initialized config at {}", config_path.display());
        } else {
            println!("Using existing config at {}", config_path.display());
        }
        println!("Gateway/API account `{api_account}` will be available after setup.");

        if options.search_api_key.is_none() {
            let (action, value) = prompt_secret_update(
                "Search API key",
                !cfg.gateway.search.api_key.trim().is_empty(),
            )?;
            match action {
                SecretPromptUpdate::Keep => {}
                SecretPromptUpdate::Clear => cfg.gateway.search.api_key.clear(),
                SecretPromptUpdate::Set => {
                    if let Some(value) = value {
                        cfg.gateway.search.api_key = value;
                    }
                }
            }
        }

        if options.image_gen_api_key.is_none() {
            let (action, value) = prompt_secret_update(
                "Image generation API key",
                !cfg.gateway.image_gen.api_key.trim().is_empty(),
            )?;
            match action {
                SecretPromptUpdate::Keep => {}
                SecretPromptUpdate::Clear => cfg.gateway.image_gen.api_key.clear(),
                SecretPromptUpdate::Set => {
                    if let Some(value) = value {
                        cfg.gateway.image_gen.api_key = value;
                    }
                }
            }
        }

        let mut conversation_key_changed = explicit_conversation_key.is_some();
        if options.conversation_index_api_key.is_none() {
            let (action, value) = prompt_secret_update(
                "Conversation index OpenAI API key",
                !cfg.gateway.conversation_index.api_key.trim().is_empty(),
            )?;
            match action {
                SecretPromptUpdate::Keep => {}
                SecretPromptUpdate::Clear => {
                    cfg.gateway.conversation_index.api_key.clear();
                    cfg.gateway.conversation_index.enabled = false;
                    conversation_key_changed = true;
                }
                SecretPromptUpdate::Set => {
                    if let Some(value) = value {
                        cfg.gateway.conversation_index.api_key = value;
                        cfg.gateway.conversation_index.enabled = true;
                        conversation_key_changed = true;
                    }
                }
            }
        }

        if !options.enable_conversation_index && !options.disable_conversation_index {
            let should_prompt = conversation_key_changed
                || cfg.gateway.conversation_index.enabled
                || !cfg.gateway.conversation_index.api_key.trim().is_empty();
            if should_prompt {
                cfg.gateway.conversation_index.enabled = prompt_yes_no(
                    "Enable conversation vector index now?",
                    cfg.gateway.conversation_index.enabled,
                )?;
            }
        }

        // ---- Channel binding ----
        // The api.* account auto-created above lets programs talk to gateway,
        // but a human needs at least one user-facing channel (telegram /
        // feishu / weixin / subprocess plugin) to actually chat with gary.
        // Push the user through that now rather than leaving it as a footnote.
        let pre_bound = user_channel_account_count(&cfg);
        if pre_bound == 0 {
            println!();
            println!("还需要绑定至少一个聊天渠道，garyx 才能在 IM 里使用。");
        }
        let mut want_bind = prompt_yes_no(
            if pre_bound == 0 {
                "现在绑定渠道？"
            } else {
                "再绑定一个渠道？"
            },
            pre_bound == 0,
        )?;
        while want_bind {
            let channel = prompt_channel_choice()?;
            match interactive_bind_channel(&mut cfg, &channel).await {
                Ok(account_id) => {
                    // Persist immediately so a ctrl-c while deciding about
                    // the next channel doesn't throw away a successful QR
                    // scan or a freshly typed token.
                    if let Err(err) = save_config_struct(&config_path, &cfg) {
                        eprintln!("[warn] 保存渠道配置失败：{err}");
                    } else {
                        println!("✓ 已绑定 {channel}.{account_id}");
                    }
                    channels_bound.push(format!("{channel}.{account_id}"));
                }
                Err(err) => {
                    eprintln!("绑定 {channel} 失败：{err}");
                }
            }
            want_bind = prompt_yes_no("继续绑定下一个渠道？", false)?;
        }
    }

    save_config_struct(&config_path, &cfg)?;

    let summary = OnboardSummary {
        ok: true,
        config_path: config_path.display().to_string(),
        created_config,
        api_account: api_account.clone(),
        api_account_created,
        search_api_key_configured: !cfg.gateway.search.api_key.trim().is_empty(),
        image_gen_api_key_configured: !cfg.gateway.image_gen.api_key.trim().is_empty(),
        conversation_index_enabled: cfg.gateway.conversation_index.enabled,
        conversation_index_api_key_configured: !cfg
            .gateway
            .conversation_index
            .api_key
            .trim()
            .is_empty(),
        conversation_index_model: cfg.gateway.conversation_index.model.clone(),
        conversation_index_base_url: cfg.gateway.conversation_index.base_url.clone(),
        gateway_run_requested: options.run_gateway,
        channels_bound,
        total_user_channel_accounts: user_channel_account_count(&cfg),
        next_steps: next_onboard_steps(&cfg),
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print_onboard_summary(&summary);
    }

    let should_run_gateway = if options.run_gateway {
        true
    } else if interactive {
        prompt_yes_no("Start gateway now?", true)?
    } else {
        false
    };

    if should_run_gateway {
        if !options.json {
            println!("Starting gateway...");
        }
        return run_gateway(
            &config_path.to_string_lossy(),
            options.port_override,
            options.host_override,
            options.no_channels,
        )
        .await;
    }

    if !options.json {
        println!("Next steps:");
        for (index, step) in summary.next_steps.iter().enumerate() {
            println!("{}. {}", index + 1, step);
        }
    }
    Ok(())
}

pub(crate) fn canonical_channel_id(channel: &str) -> String {
    let normalized = channel.trim().to_ascii_lowercase();
    available_channel_options()
        .into_iter()
        .find_map(|option| {
            (option.id == normalized || option.aliases.iter().any(|alias| alias == &normalized))
                .then_some(option.id)
        })
        .unwrap_or(normalized)
}

#[derive(Debug, Clone)]
struct ChannelOption {
    id: String,
    display_name: String,
    aliases: Vec<String>,
}

impl ChannelOption {
    fn from_metadata(metadata: PluginMetadata) -> Self {
        Self {
            id: metadata.id,
            display_name: metadata.display_name,
            aliases: metadata.aliases,
        }
    }
}

fn available_channel_options() -> Vec<ChannelOption> {
    let mut options: Vec<ChannelOption> = builtin_plugin_metadata_list()
        .into_iter()
        .map(ChannelOption::from_metadata)
        .collect();

    if let Ok(outcome) = ManifestDiscoverer::new(crate::channel_plugin_host::plugin_root_paths(
        &GaryxConfig::default(),
    ))
    .discover()
    {
        for manifest in outcome.plugins {
            if options.iter().any(|option| option.id == manifest.plugin.id) {
                continue;
            }
            options.push(ChannelOption {
                id: manifest.plugin.id.clone(),
                display_name: if manifest.plugin.display_name.trim().is_empty() {
                    manifest.plugin.id.clone()
                } else {
                    manifest.plugin.display_name.clone()
                },
                aliases: manifest.plugin.aliases.clone(),
            });
        }
    }

    options.push(ChannelOption {
        id: "api".to_owned(),
        display_name: "API".to_owned(),
        aliases: Vec::new(),
    });
    options
}

fn channel_display_name(channel: &str) -> Option<String> {
    let resolved = canonical_channel_id(channel);
    available_channel_options()
        .into_iter()
        .find(|option| option.id == resolved)
        .map(|option| option.display_name)
}

fn plugin_account_mut<'a>(
    cfg: &'a mut GaryxConfig,
    channel: &str,
    account: &str,
) -> Option<&'a mut PluginAccountEntry> {
    cfg.channels
        .plugins
        .get_mut(channel)
        .and_then(|plugin_cfg| plugin_cfg.accounts.get_mut(account))
}

pub(crate) fn cmd_channels_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let cfg = loaded.config;
    if json {
        println!("{}", serde_json::to_string_pretty(&cfg.channels)?);
    } else {
        println!("api:");
        for (id, account) in &cfg.channels.api.accounts {
            println!("  - {} (enabled={})", id, account.enabled);
        }
        for (plugin_id, plugin_cfg) in &cfg.channels.plugins {
            println!("{plugin_id} (plugin):");
            for (id, entry) in &plugin_cfg.accounts {
                println!("  - {} (enabled={})", id, entry.enabled);
            }
        }
    }
    Ok(())
}

pub(crate) async fn cmd_channels_enable(
    config_path: &str,
    channel: &str,
    account: &str,
    enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    let channel = canonical_channel_id(channel);
    match channel.as_str() {
        "api" => {
            let Some(acc) = cfg.channels.api.accounts.get_mut(account) else {
                eprintln!("API account not found: {account}");
                std::process::exit(1);
            };
            acc.enabled = enabled;
        }
        _ => {
            let Some(entry) = plugin_account_mut(&mut cfg, &channel, account) else {
                eprintln!("{channel} account not found: {account}");
                std::process::exit(1);
            };
            entry.enabled = enabled;
        }
    }
    save_config_struct(&config_path, &cfg)?;
    println!("Updated {channel}.{account}.enabled={enabled}");
    notify_gateway_reload(&config_path).await;
    Ok(())
}

pub(crate) async fn cmd_channels_remove(
    config_path: &str,
    channel: &str,
    account: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    let channel = canonical_channel_id(channel);
    let removed = match channel.as_str() {
        "api" => cfg.channels.api.accounts.remove(account).is_some(),
        _ => cfg
            .channels
            .plugins
            .get_mut(&channel)
            .and_then(|plugin_cfg| plugin_cfg.accounts.remove(account))
            .is_some(),
    };
    if !removed {
        eprintln!("Account not found: {channel}.{account}");
        std::process::exit(1);
    }
    save_config_struct(&config_path, &cfg)?;
    println!("Removed {channel}.{account}");
    notify_gateway_reload(&config_path).await;
    Ok(())
}

/// Collected channel-account fields gathered from either CLI flags or
/// interactive prompts. Shared by `garyx channels add` and the onboarding
/// flow so they stay perfectly in sync.
#[derive(Debug, Default, Clone)]
pub(crate) struct ChannelOverrides {
    pub account: Option<String>,
    pub name: Option<String>,
    pub workspace_dir: Option<String>,
    pub agent_id: Option<String>,
    pub token: Option<String>,
    pub uin: Option<String>,
    pub base_url: Option<String>,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    /// Feishu brand: `feishu` (国内 / default) or `lark` (海外).
    /// Parsed via [`parse_feishu_domain`] when calling `upsert_channel_account`.
    pub domain: Option<String>,
    /// Extra channel-owned config fields not modelled by the built-in channel
    /// flag surface. These get merged into `channels.<channel_id>.accounts[*].config`.
    pub plugin_extras: Map<String, Value>,
}

/// Parse a user-provided domain string into a [`FeishuDomain`], accepting
/// the common synonyms users actually type.
pub(crate) fn parse_feishu_domain(raw: &str) -> Option<FeishuDomain> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "feishu" | "lark-cn" | "cn" | "飞书" | "国内" | "domestic" => {
            Some(FeishuDomain::Feishu)
        }
        "lark" | "larksuite" | "intl" | "international" | "海外" => Some(FeishuDomain::Lark),
        _ => None,
    }
}

fn trim_opt(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn discover_plugin_manifest(
    channel: &str,
) -> Result<Option<PluginManifest>, Box<dyn std::error::Error>> {
    let resolved = canonical_channel_id(channel);
    if matches!(
        resolved.as_str(),
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM
            | BUILTIN_CHANNEL_PLUGIN_FEISHU
            | BUILTIN_CHANNEL_PLUGIN_WEIXIN
            | "api"
    ) {
        return Ok(None);
    }

    let outcome = ManifestDiscoverer::new(crate::channel_plugin_host::plugin_root_paths(
        &GaryxConfig::default(),
    ))
    .discover()?;
    Ok(outcome
        .plugins
        .into_iter()
        .find(|manifest| manifest.plugin.id == resolved))
}

fn plugin_form_state_from_overrides(overrides: &ChannelOverrides) -> Map<String, Value> {
    let mut map = overrides.plugin_extras.clone();
    if let Some(value) = overrides
        .token
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("token".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .uin
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("uin".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .base_url
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("base_url".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .app_id
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("app_id".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .app_secret
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("app_secret".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .domain
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("domain".to_owned(), Value::String(value.to_owned()));
    }
    map
}

fn set_plugin_form_value(overrides: &mut ChannelOverrides, key: &str, value: Value) {
    match (key, value) {
        ("token", Value::String(value)) => overrides.token = Some(value),
        ("uin", Value::String(value)) => overrides.uin = Some(value),
        ("base_url", Value::String(value)) => overrides.base_url = Some(value),
        ("app_id", Value::String(value)) => overrides.app_id = Some(value),
        ("app_secret", Value::String(value)) => overrides.app_secret = Some(value),
        ("domain", Value::String(value)) => overrides.domain = Some(value),
        (other_key, other_value) => {
            overrides
                .plugin_extras
                .insert(other_key.to_owned(), other_value);
        }
    }
}

fn plugin_suggested_account_id(
    values: &std::collections::BTreeMap<String, Value>,
) -> Option<String> {
    for key in ["account_id", "agent_id"] {
        match values.get(key) {
            Some(Value::String(value)) if !value.trim().is_empty() => return Some(value.clone()),
            Some(_) | None => {}
        }
    }
    None
}

fn plugin_schema_declares_field(schema: &Value, key: &str) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(key))
}

fn strip_plugin_identity_hints(
    values: &mut std::collections::BTreeMap<String, Value>,
    schema: &Value,
) {
    if !plugin_schema_declares_field(schema, "account_id") {
        values.remove("account_id");
    }
    if !plugin_schema_declares_field(schema, "agent_id") {
        values.remove("agent_id");
    }
}

fn value_is_missing(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.trim().is_empty(),
        Some(Value::Array(values)) => values.is_empty(),
        Some(Value::Object(values)) => values.is_empty(),
        _ => false,
    }
}

fn plugin_required_fields(schema: &Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn validate_plugin_required_fields(
    channel: &str,
    schema: &Value,
    form_state: &Map<String, Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    let missing: Vec<String> = plugin_required_fields(schema)
        .into_iter()
        .filter(|key| value_is_missing(form_state.get(key)))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing required fields for plugin `{channel}`: {}",
            missing.join(", ")
        )
        .into())
    }
}

fn schema_property_secret(name: &str, schema: &Value) -> bool {
    let name = name.to_ascii_lowercase();
    if name.contains("token")
        || name.contains("secret")
        || name.contains("password")
        || name.contains("api_key")
        || name.contains("apikey")
    {
        return true;
    }
    if schema.get("format").and_then(Value::as_str) == Some("password") {
        return true;
    }
    schema
        .get("x-garyx")
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("secret"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

struct CliPluginAuthFlowHandler;

#[async_trait]
impl InboundHandler for CliPluginAuthFlowHandler {
    async fn on_request(&self, method: String, _params: Value) -> Result<Value, (i32, String)> {
        Err((
            PluginErrorCode::MethodNotFound.as_i32(),
            format!("cli auth-flow host does not handle `{method}`"),
        ))
    }

    async fn on_notification(&self, _method: String, _params: Value) {}
}

async fn run_plugin_auth_flow(
    manifest: &PluginManifest,
    form_state: Value,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    run_plugin_auth_flow_with_options(manifest, form_state, false).await
}

async fn run_plugin_auth_flow_with_options(
    manifest: &PluginManifest,
    form_state: Value,
    json_events: bool,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    let plugin = SubprocessPlugin::spawn(
        manifest,
        SpawnOptions::default(),
        Arc::new(CliPluginAuthFlowHandler),
    )?;
    let executor = SubprocessAuthFlowExecutor::new(manifest.plugin.id.clone(), plugin.client());
    let result = run_auth_flow_with_options(&executor, form_state, json_events).await;
    let _ = plugin.shutdown_gracefully().await;
    result
}

fn prompt_plugin_auth_mode(manifest: &PluginManifest) -> Result<bool, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    if manifest.auth_flows.len() == 1 {
        let prompt = manifest.auth_flows[0].prompt.trim();
        if !prompt.is_empty() {
            println!("{prompt}");
        }
    }
    let auto_label = if manifest.auth_flows.len() == 1 {
        let label = manifest.auth_flows[0].label.trim();
        if label.is_empty() {
            format!("使用 {} 的登录流程", manifest.plugin.display_name)
        } else {
            label.to_owned()
        }
    } else {
        format!("使用 {} 的登录流程", manifest.plugin.display_name)
    };
    let items = [auto_label.as_str(), "手动填写插件配置"];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择绑定方式")
        .default(0)
        .items(&items)
        .interact()?;
    Ok(selection == 0)
}

fn prompt_channel_identity_if_missing(
    channel: &str,
    overrides: &mut ChannelOverrides,
) -> Result<(), Box<dyn std::error::Error>> {
    if overrides.account.is_some() && overrides.name.is_some() {
        return Ok(());
    }
    let default_label = overrides
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            overrides
                .account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| channel_display_name(channel));
    let value = prompt_value("名称", default_label.as_deref(), false)?;
    let value = value.trim().to_owned();
    if overrides.account.is_none() {
        overrides.account = Some(value.clone());
    }
    if overrides.name.is_none() {
        overrides.name = Some(value);
    }
    Ok(())
}

fn prompt_plugin_schema_value(
    field_name: &str,
    field_schema: &Value,
    required: bool,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let label = field_schema
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| field_schema.get("description").and_then(Value::as_str))
        .unwrap_or(field_name);
    let default_string = match field_schema.get("default") {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Bool(value)) => Some(if *value { "true" } else { "false" }.to_owned()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    };
    let field_type = field_schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("string");

    match field_type {
        "string" => {
            let value = if schema_property_secret(field_name, field_schema) {
                prompt_secret(label)?
            } else {
                prompt_value(label, default_string.as_deref(), !required)?
            };
            if value.trim().is_empty() && !required {
                Ok(None)
            } else {
                Ok(Some(Value::String(value)))
            }
        }
        "integer" => {
            let value = prompt_value(label, default_string.as_deref(), !required)?;
            if value.trim().is_empty() && !required {
                return Ok(None);
            }
            let parsed = value
                .trim()
                .parse::<i64>()
                .map_err(|err| format!("field `{field_name}` expects integer: {err}"))?;
            Ok(Some(Value::Number(parsed.into())))
        }
        "number" => {
            let value = prompt_value(label, default_string.as_deref(), !required)?;
            if value.trim().is_empty() && !required {
                return Ok(None);
            }
            let parsed = value
                .trim()
                .parse::<f64>()
                .map_err(|err| format!("field `{field_name}` expects number: {err}"))?;
            Ok(Some(json!(parsed)))
        }
        "boolean" => {
            let value = prompt_value(label, default_string.as_deref(), !required)?;
            if value.trim().is_empty() && !required {
                return Ok(None);
            }
            let parsed = match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "y" | "on" => true,
                "false" | "0" | "no" | "n" | "off" => false,
                other => {
                    return Err(
                        format!("field `{field_name}` expects boolean, got `{other}`").into(),
                    );
                }
            };
            Ok(Some(Value::Bool(parsed)))
        }
        other => {
            if required {
                Err(format!(
                    "plugin field `{field_name}` uses unsupported schema type `{other}`; use the desktop UI for this plugin"
                )
                .into())
            } else {
                Ok(None)
            }
        }
    }
}

async fn interactive_fill_plugin_channel_overrides(
    manifest: &PluginManifest,
    mut overrides: ChannelOverrides,
) -> Result<ChannelOverrides, Box<dyn std::error::Error>> {
    let mut form_state = plugin_form_state_from_overrides(&overrides);
    if !manifest.auth_flows.is_empty()
        && plugin_required_fields(&manifest.schema)
            .iter()
            .any(|field| value_is_missing(form_state.get(field)))
    {
        if prompt_plugin_auth_mode(manifest)? {
            let mut values =
                run_plugin_auth_flow(manifest, Value::Object(form_state.clone())).await?;
            if overrides.account.is_none() {
                if let Some(account_id) = plugin_suggested_account_id(&values) {
                    overrides.account = Some(account_id);
                }
            }
            strip_plugin_identity_hints(&mut values, &manifest.schema);
            for (key, value) in values {
                set_plugin_form_value(&mut overrides, &key, value);
            }
            form_state = plugin_form_state_from_overrides(&overrides);
        }
    }

    let properties = manifest
        .schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = plugin_required_fields(&manifest.schema);

    for (field_name, field_schema) in &properties {
        if value_is_missing(form_state.get(field_name))
            && required.iter().any(|name| name == field_name)
        {
            if let Some(value) = prompt_plugin_schema_value(field_name, field_schema, true)? {
                set_plugin_form_value(&mut overrides, field_name, value);
                form_state = plugin_form_state_from_overrides(&overrides);
            }
        }
    }
    for (field_name, field_schema) in &properties {
        if required.iter().any(|name| name == field_name)
            || !value_is_missing(form_state.get(field_name))
        {
            continue;
        }
        if let Some(value) = prompt_plugin_schema_value(field_name, field_schema, false)? {
            set_plugin_form_value(&mut overrides, field_name, value);
            form_state = plugin_form_state_from_overrides(&overrides);
        }
    }

    Ok(overrides)
}

/// Fill any still-empty fields of `overrides` by prompting the user.
///
/// Assumes the caller already verified that stdin/stdout are interactive
/// (via `can_prompt_interactively`). For weixin, this triggers the full QR
/// login flow and reads `token` + `account` out of the scan result.
async fn interactive_fill_channel_overrides(
    channel: &str,
    mut overrides: ChannelOverrides,
) -> Result<ChannelOverrides, Box<dyn std::error::Error>> {
    if let Some(manifest) = discover_plugin_manifest(channel)? {
        if overrides.workspace_dir.is_none() {
            let value = prompt_value("运行目录（回车跳过，默认用户目录）", None, true)?;
            if !value.trim().is_empty() {
                overrides.workspace_dir = Some(value);
            }
        }
        if overrides.agent_id.is_none() {
            overrides.agent_id = Some(prompt_agent_reference_choice(Some(
                DEFAULT_CHANNEL_AGENT_ID,
            ))?);
        }
        overrides = interactive_fill_plugin_channel_overrides(&manifest, overrides).await?;
        prompt_channel_identity_if_missing(channel, &mut overrides)?;
        return Ok(overrides);
    }

    // One "名称" prompt fills both the internal account id (HashMap key in
    // config) and the display name — users almost always want them to match,
    // and asking twice is just noise. Callers who care about distinguishing
    // the two can still pass `--account` and `--name` explicitly.
    if channel != "weixin" && (overrides.account.is_none() || overrides.name.is_none()) {
        prompt_channel_identity_if_missing(channel, &mut overrides)?;
    }
    if overrides.workspace_dir.is_none() {
        let value = prompt_value("运行目录（回车跳过，默认用户目录）", None, true)?;
        if !value.trim().is_empty() {
            overrides.workspace_dir = Some(value);
        }
    }
    if overrides.agent_id.is_none() {
        overrides.agent_id = Some(prompt_agent_reference_choice(Some(
            DEFAULT_CHANNEL_AGENT_ID,
        ))?);
    }
    match channel {
        "telegram" => {
            if overrides.token.is_none() {
                overrides.token = Some(prompt_secret("Telegram Bot Token")?);
            }
        }
        "feishu" => {
            // If the caller already passed app_id + app_secret via flags,
            // skip the mode selector — they clearly want manual.
            let has_credentials = overrides.app_id.is_some() && overrides.app_secret.is_some();
            if !has_credentials {
                let use_device_flow = prompt_feishu_auth_mode()?;
                if use_device_flow {
                    let domain = match overrides.domain.as_deref().and_then(parse_feishu_domain) {
                        Some(value) => value,
                        None => prompt_feishu_domain_choice()?,
                    };
                    let domain_str = match domain {
                        FeishuDomain::Feishu => "feishu",
                        FeishuDomain::Lark => "lark",
                    };
                    let executor = FeishuAuthExecutor::new(reqwest::Client::new());
                    let mut values =
                        run_auth_flow(&executor, json!({ "domain": domain_str })).await?;
                    overrides.app_id = Some(take_string(&mut values, "app_id")?);
                    overrides.app_secret = Some(take_string(&mut values, "app_secret")?);
                    overrides.domain = Some(take_string(&mut values, "domain")?);
                } else {
                    if overrides.app_id.is_none() {
                        overrides.app_id = Some(prompt_value("Feishu App ID", None, false)?);
                    }
                    if overrides.app_secret.is_none() {
                        overrides.app_secret = Some(prompt_secret("Feishu App Secret")?);
                    }
                    if overrides.domain.is_none() {
                        let domain = prompt_feishu_domain_choice()?;
                        overrides.domain = Some(match domain {
                            FeishuDomain::Feishu => "feishu".to_owned(),
                            FeishuDomain::Lark => "lark".to_owned(),
                        });
                    }
                }
            }
        }
        "weixin" => {
            if overrides.account.is_none() {
                let value = prompt_value("账号名（回车则使用扫码返回的 bot id）", None, true)?;
                if !value.trim().is_empty() {
                    overrides.account = Some(value);
                }
            }
            // Weixin base_url intentionally not prompted — default works
            // for everyone; advanced users can still pass --base-url.
            if overrides.token.is_none() {
                let executor = WeixinAuthExecutor::new(reqwest::Client::new());
                let mut values = run_auth_flow(
                    &executor,
                    json!({
                        "base_url": normalize_weixin_base_url(overrides.base_url.clone()),
                        "timeout_secs": 480,
                    }),
                )
                .await?;
                let token = take_string(&mut values, "token")?;
                let base_url = take_string(&mut values, "base_url")?;
                let account_id = take_string(&mut values, "account_id")?;
                if overrides.account.is_none() {
                    overrides.account = Some(account_id);
                }
                overrides.token = Some(token);
                overrides.base_url = Some(base_url);
            }
        }
        "api" => {}
        _ => {}
    }
    Ok(overrides)
}

/// Fully interactive bind of one channel into the in-memory config.
/// Returns the account id that was inserted (or updated). Caller is
/// responsible for persisting the config afterwards.
async fn interactive_bind_channel(
    cfg: &mut GaryxConfig,
    channel: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let overrides =
        interactive_fill_channel_overrides(channel, ChannelOverrides::default()).await?;
    let account = overrides
        .account
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("missing account id")?
        .to_owned();
    upsert_channel_account(
        cfg,
        channel,
        &account,
        overrides.name,
        overrides.workspace_dir,
        overrides.agent_id,
        overrides.token,
        overrides.uin,
        overrides.base_url,
        overrides.app_id,
        overrides.app_secret,
        overrides.domain,
        overrides.plugin_extras,
    )?;
    Ok(account)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_channels_add(
    config_path: &str,
    channel: Option<String>,
    account: Option<String>,
    name: Option<String>,
    workspace_dir: Option<String>,
    agent_id: Option<String>,
    token: Option<String>,
    uin: Option<String>,
    base_url: Option<String>,
    app_id: Option<String>,
    app_secret: Option<String>,
    domain: Option<String>,
    auto_register: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let interactive = can_prompt_interactively();
    // `--domain` only makes sense for feishu/lark, so if the caller passes
    // it without an explicit channel we infer feishu. `--auto-register`
    // works for feishu now, so it no longer pins a channel by itself —
    // the user still picks interactively.
    let inferred_feishu = domain.is_some();
    let channel = match channel.as_deref() {
        Some(value) => canonical_channel_id(value),
        None if inferred_feishu => "feishu".to_owned(),
        None if interactive => prompt_channel_choice()?,
        None => return Err("missing channel; use e.g. `garyx channels add weixin ...`".into()),
    };

    let mut overrides = ChannelOverrides {
        account: trim_opt(account),
        name: trim_opt(name),
        workspace_dir: trim_opt(workspace_dir),
        agent_id: trim_opt(agent_id),
        token,
        uin,
        base_url,
        app_id,
        app_secret,
        domain: trim_opt(domain),
        plugin_extras: Map::new(),
    };

    // Non-interactive `--auto-register`: trigger device flow directly
    // without the mode-selection prompt. This still requires a controlling
    // TTY so the user can scan the printed QR / click the URL — we don't
    // attempt to run device flow completely headless.
    if auto_register {
        if let Some(manifest) = discover_plugin_manifest(&channel)? {
            let mut values = run_plugin_auth_flow(
                &manifest,
                Value::Object(plugin_form_state_from_overrides(&overrides)),
            )
            .await?;
            if overrides.account.is_none() {
                if let Some(account_id) = plugin_suggested_account_id(&values) {
                    overrides.account = Some(account_id);
                }
            }
            strip_plugin_identity_hints(&mut values, &manifest.schema);
            for (key, value) in values {
                set_plugin_form_value(&mut overrides, &key, value);
            }
        } else {
            match channel.as_str() {
                "feishu" => {
                    if overrides.app_id.is_some() && overrides.app_secret.is_some() {
                        return Err(
                        "`--auto-register` cannot be combined with `--app-id` / `--app-secret`; \
                         drop the credentials flags or drop `--auto-register`"
                            .into(),
                    );
                    }
                    let domain = match overrides.domain.as_deref().and_then(parse_feishu_domain) {
                        Some(value) => value,
                        None => FeishuDomain::default(),
                    };
                    let domain_str = match domain {
                        FeishuDomain::Feishu => "feishu",
                        FeishuDomain::Lark => "lark",
                    };
                    let executor = FeishuAuthExecutor::new(reqwest::Client::new());
                    let mut values =
                        run_auth_flow(&executor, json!({ "domain": domain_str })).await?;
                    overrides.app_id = Some(take_string(&mut values, "app_id")?);
                    overrides.app_secret = Some(take_string(&mut values, "app_secret")?);
                    overrides.domain = Some(take_string(&mut values, "domain")?);
                }
                _ => {
                    return Err(format!(
                        "`--auto-register` is only supported for feishu or plugins with auth flows, not `{channel}`"
                    )
                    .into());
                }
            }
        }
    }

    if interactive {
        overrides = interactive_fill_channel_overrides(&channel, overrides).await?;
    }

    let account = overrides
        .account
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("missing account id")?
        .to_owned();

    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    upsert_channel_account(
        &mut cfg,
        &channel,
        &account,
        overrides.name,
        overrides.workspace_dir,
        overrides.agent_id,
        overrides.token,
        overrides.uin,
        overrides.base_url,
        overrides.app_id,
        overrides.app_secret,
        overrides.domain,
        overrides.plugin_extras,
    )?;
    save_config_struct(&config_path, &cfg)?;
    println!("Added {channel}.{account}");
    notify_gateway_reload(&config_path).await;
    Ok(())
}

fn normalize_weixin_base_url(input: Option<String>) -> String {
    input
        .unwrap_or_else(|| "https://ilinkai.weixin.qq.com".to_owned())
        .trim()
        .trim_end_matches('/')
        .to_owned()
}

fn can_prompt_interactively() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn prompt_value(
    label: &str,
    default: Option<&str>,
    allow_empty: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    loop {
        match default {
            Some(value) if !value.is_empty() => {
                print!("{label} [{value}]: ");
            }
            _ => {
                print!("{label}: ");
            }
        }
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            if let Some(value) = default {
                if !value.is_empty() || allow_empty {
                    return Ok(value.to_owned());
                }
            }
            if allow_empty {
                return Ok(String::new());
            }
            println!("该字段不能为空。");
            continue;
        }
        return Ok(trimmed.to_owned());
    }
}

fn prompt_secret(label: &str) -> Result<String, Box<dyn std::error::Error>> {
    loop {
        let value = rpassword::prompt_password(format!("{label}: "))?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            println!("该字段不能为空。");
            continue;
        }
        return Ok(trimmed.to_owned());
    }
}

fn prompt_channel_choice() -> Result<String, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = available_channel_options();
    let labels: Vec<&str> = items
        .iter()
        .map(|option| option.display_name.as_str())
        .collect();
    let default_index = items
        .iter()
        .position(|option| option.id == "feishu")
        .unwrap_or(0);
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择渠道")
        .default(default_index)
        .items(&labels)
        .interact()?;
    Ok(items[selection].id.clone())
}

#[derive(Debug, Clone)]
struct AgentReferenceOption {
    id: String,
    label: String,
}

fn provider_type_label(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::CodexAppServer => "Codex",
        ProviderType::GeminiCli => "Gemini",
        ProviderType::AgentTeam => "Team",
        ProviderType::ClaudeCode => "Claude",
    }
}

fn load_cli_agent_profiles() -> Vec<CustomAgentProfile> {
    let mut agents = builtin_provider_agent_profiles()
        .into_iter()
        .map(|profile| (profile.agent_id.clone(), profile))
        .collect::<std::collections::HashMap<_, _>>();
    let path = default_custom_agents_state_path();
    if path.exists()
        && let Ok(content) = fs::read_to_string(&path)
        && !content.trim().is_empty()
        && let Ok(persisted) =
            serde_json::from_str::<std::collections::HashMap<String, CustomAgentProfile>>(&content)
    {
        for (agent_id, profile) in persisted {
            agents.insert(agent_id, profile);
        }
    }

    let mut profiles = agents
        .into_values()
        .filter(|profile| profile.standalone)
        .collect::<Vec<_>>();
    profiles.sort_by(|left, right| {
        left.built_in
            .cmp(&right.built_in)
            .reverse()
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.agent_id.cmp(&right.agent_id))
    });
    profiles
}

fn load_cli_team_profiles() -> Vec<AgentTeamProfile> {
    let path = default_agent_teams_state_path();
    let mut teams = if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .filter(|content| !content.trim().is_empty())
            .and_then(|content| {
                serde_json::from_str::<std::collections::HashMap<String, AgentTeamProfile>>(
                    &content,
                )
                .ok()
            })
            .map(|persisted| persisted.into_values().collect::<Vec<_>>())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    teams.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.team_id.cmp(&right.team_id))
    });
    teams
}

fn format_cli_agent_label(agent: &CustomAgentProfile) -> String {
    let display = if agent.display_name.trim() == agent.agent_id.trim() {
        agent.display_name.clone()
    } else {
        format!("{} ({})", agent.display_name, agent.agent_id)
    };
    let kind = if agent.built_in {
        "内置 Agent"
    } else {
        "自定义 Agent"
    };
    format!(
        "{kind} · {display} · {}",
        provider_type_label(&agent.provider_type)
    )
}

fn format_cli_team_label(team: &AgentTeamProfile) -> String {
    if team.display_name.trim() == team.team_id.trim() {
        format!("Agent Team · {} (team)", team.display_name)
    } else {
        format!(
            "Agent Team · {} ({}, team)",
            team.display_name, team.team_id
        )
    }
}

fn available_agent_reference_options() -> Vec<AgentReferenceOption> {
    let mut options = load_cli_agent_profiles()
        .into_iter()
        .map(|agent| {
            let label = format_cli_agent_label(&agent);
            AgentReferenceOption {
                id: agent.agent_id,
                label,
            }
        })
        .collect::<Vec<_>>();
    options.extend(load_cli_team_profiles().into_iter().map(|team| {
        let label = format_cli_team_label(&team);
        AgentReferenceOption {
            id: team.team_id,
            label,
        }
    }));
    options
}

fn prompt_agent_reference_choice(
    default_id: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = available_agent_reference_options();
    if items.is_empty() {
        return Ok(DEFAULT_CHANNEL_AGENT_ID.to_owned());
    }
    let labels = items
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    let default_index = default_id
        .and_then(|id| items.iter().position(|item| item.id == id))
        .or_else(|| {
            items
                .iter()
                .position(|item| item.id == DEFAULT_CHANNEL_AGENT_ID)
        })
        .unwrap_or(0);
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择 Agent")
        .default(default_index)
        .items(&labels)
        .interact()?;
    Ok(items[selection].id.clone())
}

#[allow(clippy::too_many_arguments)]
fn upsert_channel_account(
    cfg: &mut GaryxConfig,
    channel: &str,
    account: &str,
    name: Option<String>,
    workspace_dir: Option<String>,
    agent_id: Option<String>,
    token: Option<String>,
    uin: Option<String>,
    base_url: Option<String>,
    app_id: Option<String>,
    app_secret: Option<String>,
    domain: Option<String>,
    plugin_extras: Map<String, Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent_id = trim_opt(agent_id).unwrap_or_else(|| DEFAULT_CHANNEL_AGENT_ID.to_owned());
    match channel {
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM => {
            let Some(token) = token else {
                return Err("missing telegram token".into());
            };
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_TELEGRAM)
                .accounts
                .insert(
                    account.to_owned(),
                    telegram_account_to_plugin_entry(&TelegramAccount {
                        token,
                        enabled: true,
                        name,
                        agent_id: agent_id.clone(),
                        workspace_dir,
                        owner_target: None,
                        groups: Default::default(),
                    }),
                );
        }
        BUILTIN_CHANNEL_PLUGIN_FEISHU => {
            let Some(app_id) = app_id else {
                return Err("missing feishu app_id".into());
            };
            let Some(app_secret) = app_secret else {
                return Err("missing feishu app_secret".into());
            };
            let resolved_domain = domain
                .as_deref()
                .and_then(parse_feishu_domain)
                .unwrap_or_default();
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_FEISHU)
                .accounts
                .insert(
                    account.to_owned(),
                    feishu_account_to_plugin_entry(&FeishuAccount {
                        app_id,
                        app_secret,
                        enabled: true,
                        domain: resolved_domain,
                        name,
                        agent_id: agent_id.clone(),
                        workspace_dir,
                        owner_target: None,
                        require_mention: true,
                        topic_session_mode: Default::default(),
                    }),
                );
        }
        BUILTIN_CHANNEL_PLUGIN_WEIXIN => {
            let Some(token) = token else {
                return Err("missing weixin token".into());
            };
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_WEIXIN)
                .accounts
                .insert(
                    account.to_owned(),
                    weixin_account_to_plugin_entry(&WeixinAccount {
                        token,
                        uin: uin.unwrap_or_default(),
                        enabled: true,
                        base_url: base_url
                            .unwrap_or_else(|| "https://ilinkai.weixin.qq.com".to_owned()),
                        name,
                        agent_id: agent_id.clone(),
                        workspace_dir,
                        streaming_update: true,
                    }),
                );
        }
        "api" => {
            cfg.channels.api.accounts.insert(
                account.to_owned(),
                ApiAccount {
                    enabled: true,
                    name,
                    agent_id: agent_id.clone(),
                    workspace_dir,
                },
            );
        }
        _ => {
            let Some(manifest) = discover_plugin_manifest(channel)? else {
                return Err(format!("unknown channel: {channel}").into());
            };
            let overrides = ChannelOverrides {
                account: Some(account.to_owned()),
                name: name.clone(),
                workspace_dir: workspace_dir.clone(),
                agent_id: Some(agent_id.clone()),
                token,
                uin,
                base_url,
                app_id,
                app_secret,
                domain,
                plugin_extras,
            };
            let form_state = plugin_form_state_from_overrides(&overrides);
            validate_plugin_required_fields(channel, &manifest.schema, &form_state)?;
            cfg.channels
                .plugin_channel_mut(&manifest.plugin.id)
                .accounts
                .insert(
                    account.to_owned(),
                    PluginAccountEntry {
                        enabled: true,
                        name,
                        agent_id: Some(agent_id),
                        workspace_dir,
                        config: Value::Object(form_state),
                    },
                );
        }
    }
    Ok(())
}

/// Prompt: "one-click authorize" (default) vs "manual input App ID / Secret".
///
/// Returns `true` for one-click device flow, `false` for manual input.
fn prompt_feishu_auth_mode() -> Result<bool, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = [
        "一键授权（推荐）：扫码 / 点击链接，自动获取 App ID 和 Secret",
        "手动输入：在飞书开放平台创建应用后，粘贴 App ID 和 Secret",
    ];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择绑定方式")
        .default(0)
        .items(&items)
        .interact()?;
    Ok(selection == 0)
}

fn prompt_feishu_domain_choice() -> Result<FeishuDomain, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = ["飞书（国内 feishu.cn）", "Lark（海外 larksuite.com）"];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择租户")
        .default(0)
        .items(&items)
        .interact()?;
    Ok(if selection == 0 {
        FeishuDomain::Feishu
    } else {
        FeishuDomain::Lark
    })
}

/// Render a URL as a terminal QR using Unicode half-block characters.
///
/// Returns `None` if the underlying encoder refuses the payload (e.g. it is
/// too long for QR version 40). Callers are expected to fall back to
/// printing the URL verbatim so the flow never becomes un-scannable.
fn render_terminal_qr(payload: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    use qrcode::{EcLevel, QrCode};

    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M).ok()?;
    let rendered = code
        .render::<Dense1x2>()
        // Give the QR a one-module quiet zone on every side so terminal
        // scanners lock on reliably.
        .quiet_zone(true)
        // Dark = foreground module (drawn on the terminal), Light = bg.
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .build();
    Some(rendered)
}

/// Walk a batch of [`AuthDisplayItem`]s and print them to stdout. Text
/// items render as-is; Qr items are encoded via [`render_terminal_qr`]
/// with a plain-text fallback so a scanner-hostile terminal never hides
/// the underlying payload from the user. Unknown items (forward-compat)
/// are silently skipped.
fn render_display_with_options(items: &[AuthDisplayItem], json_events: bool) {
    for item in items {
        match item {
            AuthDisplayItem::Text { value } => {
                if json_events {
                    println!(
                        "{}",
                        json!({
                            "event": "auth_display",
                            "kind": "text",
                            "value": value,
                        })
                    );
                    continue;
                }
                println!("{value}");
                // Convenience: if a Text item is a standalone URL on
                // its own line, try to open it in the user's default
                // browser (preserves the pre-auth-flow UX where the
                // per-channel device-auth helpers auto-opened the
                // verification URL). We keep this in the CLI's
                // render layer — not in the executor — so the
                // abstraction stays channel-blind: the executor
                // just emits Text; the terminal "UI" decides what
                // that means.
                if looks_like_standalone_url(value) {
                    let _ = open_url_in_browser(value);
                }
            }
            AuthDisplayItem::Qr { value } => {
                if json_events {
                    println!(
                        "{}",
                        json!({
                            "event": "auth_display",
                            "kind": "qr",
                            "value": value,
                        })
                    );
                    continue;
                }
                match render_terminal_qr(value) {
                    Some(art) => println!("{art}"),
                    None => {
                        eprintln!("[warn] 无法在终端渲染二维码，请使用下列原始内容：");
                        println!("{value}");
                    }
                }
            }
            AuthDisplayItem::Unknown => {}
        }
    }
}

/// True when `s` is a http(s) URL with no surrounding whitespace /
/// punctuation — i.e. a line the user could copy verbatim. Narrow
/// heuristic on purpose so hint prose like `"打开链接: https://…"`
/// doesn't get auto-opened (the channel-authored prose and the URL
/// ride on separate Text items by convention).
fn looks_like_standalone_url(s: &str) -> bool {
    let t = s.trim();
    if t.len() != s.len() {
        return false; // surrounding whitespace
    }
    (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace)
}

/// Best-effort: shell out to the platform's default-browser opener.
/// Silent on failure — the URL is already printed above so the user
/// can always copy-paste.
fn open_url_in_browser(url: &str) -> io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let cmd = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let cmd = Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    // NB: do NOT shell out through `cmd /C start` — that reinterprets
    // the URL on cmd.exe's command line, giving shell metacharacters
    // like `&`/`|`/`^` (legal inside a URL) attack surface. Use
    // `rundll32 url.dll,FileProtocolHandler` which takes the URL as
    // a direct argv[2] with no shell involvement.
    let cmd = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let cmd: io::Result<std::process::Child> = Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no known opener for this target",
    ));
    cmd.map(|_| ())
}

/// Channel-blind driver for any [`AuthFlowExecutor`]. Invokes `start`,
/// renders the initial display, then polls at the executor's cadence
/// until the session confirms, fails, or the deadline elapses. The
/// returned map is the executor's `Confirmed { values }` patch — the
/// caller is responsible for pulling the keys it needs via
/// [`take_string`] (e.g. `app_id` / `token` / `base_url`).
async fn run_auth_flow<E: AuthFlowExecutor + ?Sized>(
    executor: &E,
    form_state: Value,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    run_auth_flow_with_options(executor, form_state, false).await
}

async fn run_auth_flow_with_options<E: AuthFlowExecutor + ?Sized>(
    executor: &E,
    form_state: Value,
    json_events: bool,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    let session = executor
        .start(form_state)
        .await
        .map_err(|e| format!("auth flow start failed: {e}"))?;
    render_display_with_options(&session.display, json_events);

    let mut interval = session.poll_interval_secs.max(1);
    // Honour the executor's exact TTL — callers MUST NOT poll past
    // `expires_in_secs` per the trait contract. Only guard against
    // a zero value (pathological executor) by treating it as "no
    // cap beyond the server's own 401s".
    let deadline = std::time::Instant::now()
        + Duration::from_secs(if session.expires_in_secs == 0 {
            u64::MAX / 2
        } else {
            session.expires_in_secs
        });

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(format!("auth flow timed out after {}s", session.expires_in_secs).into());
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;

        match executor
            .poll(&session.session_id)
            .await
            .map_err(|e| format!("auth flow poll failed: {e}"))?
        {
            AuthPollResult::Pending {
                display,
                next_interval_secs,
            } => {
                if let Some(d) = display {
                    render_display_with_options(&d, json_events);
                }
                if let Some(i) = next_interval_secs {
                    interval = i.max(1);
                }
            }
            AuthPollResult::Confirmed { values } => return Ok(values),
            AuthPollResult::Failed { reason } => {
                return Err(format!("auth flow failed: {reason}").into());
            }
        }
    }
}

/// Pop a required string field out of a `Confirmed { values }` map.
/// Returns a descriptive error if the key is absent or not a JSON
/// string — that only happens when the executor's documented contract
/// changes, so the message points at the contract rather than the end
/// user.
fn take_string(
    values: &mut std::collections::BTreeMap<String, Value>,
    key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match values.remove(key) {
        Some(Value::String(s)) => Ok(s),
        Some(other) => Err(format!("auth flow returned non-string `{key}`: {other}").into()),
        None => Err(format!("auth flow did not return `{key}`").into()),
    }
}

fn reauthorize_account_entry(
    cfg: &GaryxConfig,
    channel: &str,
    reauthorize: Option<&str>,
) -> Result<Option<PluginAccountEntry>, Box<dyn std::error::Error>> {
    let Some(account_id) = reauthorize else {
        return Ok(None);
    };
    let entry = cfg
        .channels
        .plugin_channel(channel)
        .and_then(|plugin| plugin.accounts.get(account_id))
        .cloned()
        .ok_or_else(|| format!("--reauthorize `{account_id}` was not found in {channel}"))?;
    Ok(Some(entry))
}

fn config_string(entry: &PluginAccountEntry, key: &str) -> Option<String> {
    entry
        .config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn finish_reauthorization(
    cfg: &mut GaryxConfig,
    channel: &str,
    reauthorize: Option<&str>,
    target_account: &str,
    forget_previous: bool,
) -> Result<Option<&'static str>, Box<dyn std::error::Error>> {
    let Some(old_account) = reauthorize else {
        return Ok(None);
    };
    if old_account == target_account {
        return Ok(None);
    }
    let accounts = &mut cfg.channels.plugin_channel_mut(channel).accounts;
    if forget_previous {
        accounts.remove(old_account);
        return Ok(Some("deleted"));
    }
    let entry = accounts
        .get_mut(old_account)
        .ok_or_else(|| format!("--reauthorize `{old_account}` disappeared from {channel}"))?;
    entry.enabled = false;
    Ok(Some("disabled"))
}

fn print_login_json_summary(
    channel: &str,
    account_id: &str,
    action: &str,
    reauthorize: Option<&str>,
    previous_account_action: Option<&str>,
    config_path: &Path,
) {
    println!(
        "{}",
        json!({
            "ok": true,
            "event": "channel_login_saved",
            "channel": channel,
            "account_id": account_id,
            "action": action,
            "reauthorize": reauthorize,
            "previous_account_action": previous_account_action,
            "config_path": config_path.to_string_lossy().to_string(),
        })
    );
}

fn previous_account_action_label(action: &str) -> &str {
    match action {
        "deleted" => "删除",
        "disabled" => "禁用",
        _ => action,
    }
}

pub(crate) async fn cmd_channels_login(
    config_path: &str,
    channel: &str,
    account: Option<String>,
    reauthorize: Option<String>,
    forget_previous: bool,
    name: Option<String>,
    workspace_dir: Option<String>,
    agent_id: Option<String>,
    uin: Option<String>,
    base_url: Option<String>,
    domain: Option<String>,
    timeout_seconds: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let channel = canonical_channel_id(channel);
    if forget_previous && trim_opt(reauthorize.clone()).is_none() {
        return Err("--forget-previous requires --reauthorize".into());
    }
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    let reauthorize = trim_opt(reauthorize);
    let reauthorize_entry = reauthorize_account_entry(&cfg, &channel, reauthorize.as_deref())?;
    let inherited_auth_form_state = reauthorize_entry
        .as_ref()
        .and_then(|entry| entry.config.as_object().cloned())
        .unwrap_or_default();
    let selected_name =
        trim_opt(name).or_else(|| reauthorize_entry.as_ref().and_then(|e| e.name.clone()));
    let selected_workspace_dir = trim_opt(workspace_dir).or_else(|| {
        reauthorize_entry
            .as_ref()
            .and_then(|e| e.workspace_dir.clone())
    });
    let selected_uin = trim_opt(uin).or_else(|| {
        reauthorize_entry
            .as_ref()
            .and_then(|entry| config_string(entry, "uin"))
    });
    let mut selected_agent_id =
        trim_opt(agent_id).or_else(|| reauthorize_entry.as_ref().and_then(|e| e.agent_id.clone()));
    if selected_agent_id.is_none() && can_prompt_interactively() && !json_output {
        selected_agent_id = Some(prompt_agent_reference_choice(Some(
            DEFAULT_CHANNEL_AGENT_ID,
        ))?);
    }

    match channel.as_str() {
        BUILTIN_CHANNEL_PLUGIN_WEIXIN => {
            let executor = WeixinAuthExecutor::new(reqwest::Client::new());
            let login_base_url = trim_opt(base_url.clone()).or_else(|| {
                reauthorize_entry
                    .as_ref()
                    .and_then(|entry| config_string(entry, "base_url"))
            });
            let mut values = run_auth_flow_with_options(
                &executor,
                json!({
                    "base_url": normalize_weixin_base_url(login_base_url),
                    "timeout_secs": timeout_seconds,
                }),
                json_output,
            )
            .await?;
            let token = take_string(&mut values, "token")?;
            let login_base_url = take_string(&mut values, "base_url")?;
            let scanned_account_id = take_string(&mut values, "account_id")?;
            let target_account_id = account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| scanned_account_id.clone());
            let existed = cfg
                .channels
                .plugin_channel(BUILTIN_CHANNEL_PLUGIN_WEIXIN)
                .and_then(|plugin| plugin.accounts.get(&target_account_id))
                .is_some();

            upsert_channel_account(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_WEIXIN,
                &target_account_id,
                selected_name.clone(),
                selected_workspace_dir.clone(),
                selected_agent_id.clone(),
                Some(token),
                selected_uin.clone(),
                Some(login_base_url),
                None,
                None,
                None,
                Map::new(),
            )?;
            let previous_account_action = finish_reauthorization(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_WEIXIN,
                reauthorize.as_deref(),
                &target_account_id,
                forget_previous,
            )?;
            save_config_struct(&config_path, &cfg)?;
            if json_output {
                notify_gateway_reload_quiet(&config_path).await;
                print_login_json_summary(
                    BUILTIN_CHANNEL_PLUGIN_WEIXIN,
                    &target_account_id,
                    if existed { "updated" } else { "added" },
                    reauthorize.as_deref(),
                    previous_account_action,
                    &config_path,
                );
            } else {
                println!("已添加 weixin 账号: {target_account_id}");
                if let Some(action) = previous_account_action {
                    println!(
                        "已{}旧 weixin 账号: {}",
                        previous_account_action_label(action),
                        reauthorize.as_deref().unwrap()
                    );
                }
                notify_gateway_reload(&config_path).await;
            }
            Ok(())
        }
        BUILTIN_CHANNEL_PLUGIN_FEISHU => {
            let login_domain = trim_opt(domain.clone()).or_else(|| {
                reauthorize_entry
                    .as_ref()
                    .and_then(|entry| config_string(entry, "domain"))
            });
            let resolved_domain = login_domain
                .as_deref()
                .and_then(parse_feishu_domain)
                .unwrap_or_default();
            let domain_str = match resolved_domain {
                FeishuDomain::Feishu => "feishu",
                FeishuDomain::Lark => "lark",
            };
            let executor = FeishuAuthExecutor::new(reqwest::Client::new());
            let mut values =
                run_auth_flow_with_options(&executor, json!({ "domain": domain_str }), json_output)
                    .await?;
            let app_id = take_string(&mut values, "app_id")?;
            let app_secret = take_string(&mut values, "app_secret")?;
            let confirmed_domain = take_string(&mut values, "domain")?;
            let target_account_id = account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| app_id.clone());
            let existed = cfg
                .channels
                .plugin_channel(BUILTIN_CHANNEL_PLUGIN_FEISHU)
                .and_then(|plugin| plugin.accounts.get(&target_account_id))
                .is_some();

            upsert_channel_account(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_FEISHU,
                &target_account_id,
                selected_name.clone(),
                selected_workspace_dir.clone(),
                selected_agent_id.clone(),
                None,
                None,
                None,
                Some(app_id),
                Some(app_secret),
                Some(confirmed_domain),
                Map::new(),
            )?;
            let previous_account_action = finish_reauthorization(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_FEISHU,
                reauthorize.as_deref(),
                &target_account_id,
                forget_previous,
            )?;
            save_config_struct(&config_path, &cfg)?;
            if json_output {
                notify_gateway_reload_quiet(&config_path).await;
                print_login_json_summary(
                    BUILTIN_CHANNEL_PLUGIN_FEISHU,
                    &target_account_id,
                    if existed { "updated" } else { "added" },
                    reauthorize.as_deref(),
                    previous_account_action,
                    &config_path,
                );
            } else {
                println!("已添加 feishu 账号: {target_account_id}");
                if let Some(action) = previous_account_action {
                    println!(
                        "已{}旧 feishu 账号: {}",
                        previous_account_action_label(action),
                        reauthorize.as_deref().unwrap()
                    );
                }
                notify_gateway_reload(&config_path).await;
            }
            Ok(())
        }
        _ => {
            let Some(manifest) = discover_plugin_manifest(&channel)? else {
                return Err(format!(
                    "channel login is currently supported only for feishu/weixin or plugins with auth flows, got: {channel}"
                )
                .into());
            };
            if manifest.auth_flows.is_empty() {
                return Err(format!("channel `{channel}` does not advertise any auth flow").into());
            }
            let mut values = run_plugin_auth_flow_with_options(
                &manifest,
                Value::Object({
                    let mut form_state = inherited_auth_form_state.clone();
                    if let Some(value) = trim_opt(base_url) {
                        form_state.insert("base_url".to_owned(), Value::String(value));
                    }
                    if let Some(value) = trim_opt(domain) {
                        form_state.insert("domain".to_owned(), Value::String(value));
                    }
                    form_state
                }),
                json_output,
            )
            .await?;
            let target_account_id = account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .or_else(|| plugin_suggested_account_id(&values))
                .ok_or_else(|| {
                    format!("plugin `{channel}` auth flow did not return an account id hint")
                })?;
            let existed = cfg
                .channels
                .plugin_channel(&channel)
                .and_then(|plugin| plugin.accounts.get(&target_account_id))
                .is_some();
            strip_plugin_identity_hints(&mut values, &manifest.schema);
            let mut overrides = ChannelOverrides {
                account: Some(target_account_id.clone()),
                name: selected_name.clone(),
                workspace_dir: selected_workspace_dir.clone(),
                agent_id: selected_agent_id.clone(),
                plugin_extras: Map::new(),
                ..ChannelOverrides::default()
            };
            for (key, value) in values {
                set_plugin_form_value(&mut overrides, &key, value);
            }
            upsert_channel_account(
                &mut cfg,
                &channel,
                &target_account_id,
                overrides.name,
                overrides.workspace_dir,
                overrides.agent_id,
                overrides.token,
                overrides.uin,
                overrides.base_url,
                overrides.app_id,
                overrides.app_secret,
                overrides.domain,
                overrides.plugin_extras,
            )?;
            let previous_account_action = finish_reauthorization(
                &mut cfg,
                &channel,
                reauthorize.as_deref(),
                &target_account_id,
                forget_previous,
            )?;
            save_config_struct(&config_path, &cfg)?;
            if json_output {
                notify_gateway_reload_quiet(&config_path).await;
                print_login_json_summary(
                    &channel,
                    &target_account_id,
                    if existed { "updated" } else { "added" },
                    reauthorize.as_deref(),
                    previous_account_action,
                    &config_path,
                );
            } else {
                println!("已添加 {channel} 账号: {target_account_id}");
                if let Some(action) = previous_account_action {
                    println!(
                        "已{}旧 {channel} 账号: {}",
                        previous_account_action_label(action),
                        reauthorize.as_deref().unwrap()
                    );
                }
                notify_gateway_reload(&config_path).await;
            }
            Ok(())
        }
    }
}

fn default_log_path(path_override: Option<String>) -> String {
    path_override.unwrap_or_else(|| {
        std::env::var("GARYX_LOG_FILE")
            .unwrap_or_else(|_| default_log_file_path().to_string_lossy().to_string())
    })
}

pub(crate) fn cmd_logs_path(path: Option<String>) {
    println!("{}", default_log_path(path));
}

pub(crate) async fn cmd_logs_tail(
    path: Option<String>,
    lines: usize,
    pattern: Option<String>,
    follow: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let log_path = default_log_path(path);
    let mut last_line_count = 0usize;

    loop {
        let content = fs::read_to_string(&log_path).unwrap_or_default();
        let mut all_lines: Vec<&str> = content.lines().collect();
        if let Some(ref p) = pattern {
            all_lines = all_lines
                .into_iter()
                .filter(|line| line.contains(p))
                .collect();
        }

        if !follow {
            let start = all_lines.len().saturating_sub(lines);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            break;
        }

        if all_lines.len() > last_line_count {
            let start = all_lines.len().saturating_sub(lines).max(last_line_count);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            std::io::stdout().flush()?;
            last_line_count = all_lines.len();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(())
}

pub(crate) fn cmd_logs_clear(path: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let log_path = default_log_path(path);
    if let Some(parent) = PathBuf::from(&log_path).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&log_path, "")?;
    println!("Cleared {}", log_path);
    Ok(())
}

pub(crate) async fn cmd_audit(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut warnings = Vec::new();
    let mut critical = Vec::new();

    match load_config_or_default(config_path, ConfigRuntimeOverrides::default()) {
        Ok(loaded) => {
            let cfg = loaded.config;
            let enabled_plugins = cfg
                .channels
                .plugins
                .values()
                .any(|plugin_cfg| plugin_cfg.accounts.values().any(|a| a.enabled));
            let enabled_api = cfg.channels.api.accounts.values().any(|a| a.enabled);
            if !enabled_plugins && !enabled_api {
                warnings.push("no enabled channel accounts".to_owned());
            }
        }
        Err(err) => {
            critical.push(format!("config load failed: {}", err));
        }
    }

    if !which("claude") && !which("codex") {
        warnings.push("neither `claude` nor `codex` binary found on PATH".to_owned());
    }

    let code = if !critical.is_empty() {
        2
    } else if !warnings.is_empty() {
        1
    } else {
        0
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": code == 0,
                "warnings": warnings,
                "critical": critical,
                "exit_code": code
            }))?
        );
    } else {
        for item in &critical {
            eprintln!("[critical] {item}");
        }
        for item in &warnings {
            eprintln!("[warning] {item}");
        }
        if code == 0 {
            println!("Audit passed");
        }
    }

    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// Check whether a binary exists on PATH.
pub(crate) fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let full = dir.join(name);
                full.is_file() || full.with_extension("exe").is_file()
            })
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// garyx message
// ---------------------------------------------------------------------------

pub(crate) async fn cmd_send_message(
    config_path: &str,
    bot: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if text.trim().is_empty() {
        return Err("message text is required".into());
    }

    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let port = config.gateway.port;
    let host = if config.gateway.host == "0.0.0.0" {
        "127.0.0.1"
    } else {
        &config.gateway.host
    };
    let url = format!("http://{}:{}/api/send", host, port);

    let body = serde_json::json!({
        "bot": bot,
        "text": text,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let status = resp.status();
    let payload: serde_json::Value = resp.json().await?;

    if status.is_success() && payload.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        let ids = payload
            .get("message_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!("✅ 已发送 (message_ids: {ids})");
    } else {
        let error = payload
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        eprintln!("❌ 发送失败: {error}");
        std::process::exit(1);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Wiki commands
// ---------------------------------------------------------------------------

pub(crate) async fn cmd_wiki_init(
    config_path: &str,
    path: String,
    topic: String,
    id: Option<String>,
    agent: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = std::fs::canonicalize(std::path::PathBuf::from(path.trim()))
        .unwrap_or_else(|_| std::path::PathBuf::from(path.trim()))
        .to_string_lossy()
        .to_string();
    let topic = topic.trim().to_owned();
    if topic.is_empty() {
        return Err("--topic is required".into());
    }

    // Generate wiki_id from id or topic
    let wiki_id = id.unwrap_or_else(|| {
        topic
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_owned()
    });

    // Create directory structure locally
    let wiki_path = std::path::Path::new(&path);
    for dir in &[
        "raw/assets",
        "entities",
        "concepts",
        "sources",
        "synthesis",
        "tools",
    ] {
        std::fs::create_dir_all(wiki_path.join(dir))?;
    }

    // Create WIKI.md schema
    let wiki_md = format!(
        "# {} Wiki — Schema\n\n\
         > Topic: {}\n\
         > Created: {}\n\n\
         ## Overview\n\n\
         This is a personal knowledge base about **{}**.\n\n\
         ## Directory Structure\n\n\
         - `raw/` — Source materials (read-only)\n\
         - `entities/` — Entity pages\n\
         - `concepts/` — Concept pages\n\
         - `sources/` — Per-source summaries\n\
         - `synthesis/` — Cross-cutting analysis\n\n\
         ## Conventions\n\n\
         - File names: kebab-case\n\
         - Cross-references: Obsidian wiki-links `[[path/page-name]]`\n\
         - Every page must have YAML frontmatter\n",
        topic,
        topic,
        chrono::Utc::now().format("%Y-%m-%d"),
        topic
    );
    std::fs::write(wiki_path.join("WIKI.md"), &wiki_md)?;

    // Create index.md
    let index_md = format!(
        "# {} Wiki — Index\n\n\
         > Last updated: {}\n\
         > Total pages: 0 | Sources processed: 0\n\n\
         ## Entities\n\n(none yet)\n\n\
         ## Concepts\n\n(none yet)\n\n\
         ## Sources\n\n(none yet)\n\n\
         ## Synthesis\n\n(none yet)\n",
        topic,
        chrono::Utc::now().format("%Y-%m-%d")
    );
    std::fs::write(wiki_path.join("index.md"), &index_md)?;

    // Create log.md
    let log_md = format!(
        "# Wiki Log\n\n\
         ## [{}] init | Wiki created\n\
         - Topic: {}\n\
         - Path: {}\n",
        chrono::Utc::now().format("%Y-%m-%d"),
        topic,
        path
    );
    std::fs::write(wiki_path.join("log.md"), &log_md)?;

    // Create .gitignore
    std::fs::write(
        wiki_path.join(".gitignore"),
        "graphify-out/\n.obsidian/\n*.pyc\n__pycache__/\n",
    )?;

    // Initialize git repo if not already one
    if !wiki_path.join(".git").exists() {
        let _ = std::process::Command::new("git")
            .args(["init"])
            .current_dir(wiki_path)
            .output();
        let _ = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(wiki_path)
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", &format!("wiki: initialize {}", topic)])
            .current_dir(wiki_path)
            .output();
    }

    // Register with gateway
    let gateway = gateway_endpoint(config_path)?;
    let display_name = topic.clone();
    let payload = json!({
        "wiki_id": wiki_id,
        "display_name": display_name,
        "path": path,
        "topic": topic,
        "agent_id": agent,
    });
    let result = post_gateway_json(&gateway, "/api/wikis", &payload).await?;

    println!("Wiki initialized:");
    println!(
        "  ID:    {}",
        result["wiki_id"].as_str().unwrap_or(&wiki_id)
    );
    println!("  Path:  {}", path);
    println!("  Topic: {}", topic);
    println!("  Agent: {}", agent);
    println!();
    println!("Next steps:");
    println!("  1. Drop source files into {}/raw/", path);
    println!("  2. Open a thread with the wiki-curator agent to start ingesting");
    println!("  3. Open the wiki folder in Obsidian for browsing");

    Ok(())
}

pub(crate) async fn cmd_wiki_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let response = fetch_gateway_json(&gateway, "/api/wikis").await?;

    if json {
        return print_pretty_json(&response);
    }

    let wikis = response["wikis"].as_array();
    match wikis {
        Some(wikis) if !wikis.is_empty() => {
            for wiki in wikis {
                println!(
                    "  {:<20} {:<30} {} sources, {} pages",
                    wiki["wiki_id"].as_str().unwrap_or("?"),
                    wiki["topic"].as_str().unwrap_or("?"),
                    wiki["source_count"].as_u64().unwrap_or(0),
                    wiki["page_count"].as_u64().unwrap_or(0),
                );
            }
        }
        _ => println!("No wikis registered. Run `garyx wiki init` to create one."),
    }
    Ok(())
}

pub(crate) async fn cmd_wiki_get(
    config_path: &str,
    wiki_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let response = fetch_gateway_json(
        &gateway,
        &format!("/api/wikis/{}", urlencoding::encode(wiki_id)),
    )
    .await?;

    if json {
        return print_pretty_json(&response);
    }

    println!("Wiki: {}", response["wiki_id"].as_str().unwrap_or("?"));
    println!(
        "  Name:    {}",
        response["display_name"].as_str().unwrap_or("?")
    );
    println!("  Topic:   {}", response["topic"].as_str().unwrap_or("?"));
    println!("  Path:    {}", response["path"].as_str().unwrap_or("?"));
    println!(
        "  Agent:   {}",
        response["agent_id"].as_str().unwrap_or("wiki-curator")
    );
    println!(
        "  Sources: {}",
        response["source_count"].as_u64().unwrap_or(0)
    );
    println!(
        "  Pages:   {}",
        response["page_count"].as_u64().unwrap_or(0)
    );
    println!(
        "  Created: {}",
        response["created_at"].as_str().unwrap_or("?")
    );
    println!(
        "  Updated: {}",
        response["updated_at"].as_str().unwrap_or("?")
    );
    Ok(())
}

pub(crate) async fn cmd_wiki_delete(
    config_path: &str,
    wiki_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    delete_gateway_json(
        &gateway,
        &format!("/api/wikis/{}", urlencoding::encode(wiki_id)),
    )
    .await?;
    println!(
        "Wiki '{}' unregistered. Files on disk were NOT deleted.",
        wiki_id
    );
    Ok(())
}

pub(crate) async fn cmd_wiki_status(
    config_path: &str,
    wiki_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // First get wiki entry from gateway
    let gateway = gateway_endpoint(config_path)?;
    let wiki = fetch_gateway_json(
        &gateway,
        &format!("/api/wikis/{}", urlencoding::encode(wiki_id)),
    )
    .await?;

    let wiki_path = wiki["path"].as_str().unwrap_or(".");

    // Count files locally
    let raw_count = std::fs::read_dir(format!("{}/raw", wiki_path))
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && e.file_name() != "assets")
                .count()
        })
        .unwrap_or(0);

    let wiki_page_count = ["entities", "concepts", "sources", "synthesis"]
        .iter()
        .map(|dir| {
            std::fs::read_dir(format!("{}/wiki/{}", wiki_path, dir))
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
                        .count()
                })
                .unwrap_or(0)
        })
        .sum::<usize>();

    // Read last few log entries
    let log_path = format!("{}/log.md", wiki_path);
    let recent_log = std::fs::read_to_string(&log_path)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("## ["))
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    if json {
        let status = json!({
            "wiki_id": wiki_id,
            "path": wiki_path,
            "raw_files": raw_count,
            "wiki_pages": wiki_page_count,
            "recent_log": recent_log,
        });
        return print_pretty_json(&status);
    }

    println!("Wiki Status: {}", wiki_id);
    println!("  Path:       {}", wiki_path);
    println!("  Raw files:  {}", raw_count);
    println!("  Wiki pages: {}", wiki_page_count);
    println!();
    if !recent_log.is_empty() {
        println!("Recent activity:");
        for line in recent_log.lines() {
            println!("  {}", line);
        }
    } else {
        println!("No activity logged yet.");
    }

    // Check for unprocessed files
    let log_content = std::fs::read_to_string(&log_path).unwrap_or_default();
    let unprocessed: Vec<String> = std::fs::read_dir(format!("{}/raw", wiki_path))
        .into_iter()
        .flat_map(|entries| entries.filter_map(|e| e.ok()))
        .filter(|e| e.path().is_file())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            !log_content.contains(&name)
        })
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    if !unprocessed.is_empty() {
        println!();
        println!("Unprocessed files in raw/ ({}):", unprocessed.len());
        for f in &unprocessed {
            println!("  - {}", f);
        }
    }

    Ok(())
}
