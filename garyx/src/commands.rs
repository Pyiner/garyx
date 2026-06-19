#![allow(clippy::too_many_arguments)]

use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Local};
use flate2::read::GzDecoder;
use garyx_bridge::MultiProviderBridge;
use garyx_channels::auth_flow::{AuthDisplayItem, AuthFlowExecutor, AuthPollResult};
use garyx_channels::feishu::FeishuAuthExecutor;
use garyx_channels::generated_images::{
    GeneratedImageResult, build_image_generation_prompt, extract_image_generation_result,
    provider_message_item_type,
};
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
    ApiAccount, AutomationScheduleView, BUILTIN_CHANNEL_PLUGIN_DISCORD,
    BUILTIN_CHANNEL_PLUGIN_FEISHU, BUILTIN_CHANNEL_PLUGIN_TELEGRAM, BUILTIN_CHANNEL_PLUGIN_WEIXIN,
    DiscordAccount, FeishuAccount, FeishuDomain, GaryxConfig, PluginAccountEntry, SlashCommand,
    TelegramAccount, WeixinAccount, discord_account_from_plugin_entry,
    discord_account_to_plugin_entry, feishu_account_from_plugin_entry,
    feishu_account_to_plugin_entry, telegram_account_from_plugin_entry,
    telegram_account_to_plugin_entry, weixin_account_from_plugin_entry,
    weixin_account_to_plugin_entry,
};
use garyx_models::config_loader::{
    ConfigHotReloadOptions, ConfigHotReloader, ConfigLoadOptions, ConfigRuntimeOverrides,
    ConfigWriteOptions, write_config_value_atomic,
};
use garyx_models::local_paths::{
    default_agent_teams_state_path, default_custom_agents_state_path, default_log_file_path,
    default_session_data_dir, gary_home_dir,
};
use garyx_models::provider::{
    AgentRunRequest, ProviderMessage, StreamEvent, default_claude_cli_mode,
};
use garyx_models::{
    AgentTeamProfile, CustomAgentProfile, ProviderType, builtin_provider_agent_profiles,
};
use garyx_router::{command_catalog_for_config, is_thread_key, reserved_command_names};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tar::Archive;
use tokio::process::Command;
use uuid::Uuid;

use crate::cli::AutomationScheduleArgs;
use crate::config_support::{
    default_config_path_buf, load_config_or_default, prepare_config_path_for_io_buf,
    print_diagnostics, print_errors,
};
use crate::runtime_assembler::{RuntimeAssembler, RuntimeAssembly};

#[derive(Debug, Clone)]
pub(crate) struct OnboardCommandOptions {
    pub force: bool,
    pub json: bool,
    pub api_account: String,
    pub search_api_key: Option<String>,
    pub run_gateway: bool,
    pub port_override: Option<u16>,
    pub host_override: Option<String>,
    pub no_channels: bool,
}

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

const GITHUB_RELEASE_REPO: &str = "Pyiner/garyx";
/// Re-exported under a `_DEFAULT` name so callers (the gateway
/// auto-update loop) can fall back to it when the config field is
/// blank. Keeping the const private and exposing the alias keeps
/// the surface explicit about which name is intended for fallback
/// vs internal use.
pub(crate) const GITHUB_RELEASE_REPO_DEFAULT: &str = GITHUB_RELEASE_REPO;
#[cfg(any(target_os = "macos", test))]
const MACOS_CLI_CODESIGN_IDENTIFIER: &str = "com.garyx.gateway";
const MACOS_CCTTY_CODESIGN_IDENTIFIER: &str = "com.garyx.cctty";
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

pub(crate) async fn latest_release_version(
    client: &reqwest::Client,
) -> Result<String, Box<dyn std::error::Error>> {
    let token = github_token_from_env();
    latest_release_version_for_repo(client, GITHUB_RELEASE_REPO, token.as_deref()).await
}

/// Variant of [`latest_release_version`] that lets the caller
/// override the GitHub `owner/repo`. The gateway auto-update loop
/// reads its repo from `garyx.json::gateway.auto_update.github_repo`
/// so operators can point at a fork for testing; the manual
/// `garyx update` path keeps the compile-time default.
///
/// `token` is an optional GitHub personal access token. When set the
/// request is bearer-authenticated, lifting the unauthenticated
/// 60 req/h IP-rate-limit to the per-token 5000 req/h budget. Read
/// from the `GARYX_GITHUB_TOKEN` env var rather than `garyx.json` so
/// the secret never lands on disk in the config file.
pub(crate) async fn latest_release_version_for_repo(
    client: &reqwest::Client,
    repo: &str,
    token: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut request = client.get(format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ));
    if let Some(value) = token {
        request = request.bearer_auth(value);
    }
    let summary = request
        .send()
        .await?
        .error_for_status()?
        .json::<GitHubReleaseSummary>()
        .await?;
    Ok(normalize_release_version(&summary.tag_name))
}

/// Read `GARYX_GITHUB_TOKEN` and return `Some` only when non-empty
/// after trim. Lifts the GitHub unauthenticated rate limit when set.
/// Lives outside the config struct on purpose — secrets in env vars,
/// not in `garyx.json`.
pub(crate) fn github_token_from_env() -> Option<String> {
    std::env::var("GARYX_GITHUB_TOKEN")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub(crate) fn replacement_binary_path(
    install_path: Option<PathBuf>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = install_path {
        return Ok(path);
    }
    Ok(std::env::current_exe()?)
}

#[cfg(any(target_os = "macos", test))]
fn macos_cli_codesign_args_with_identifier(
    binary_path: &Path,
    identifier: &str,
) -> Vec<std::ffi::OsString> {
    let mut args = vec![
        std::ffi::OsString::from("--force"),
        std::ffi::OsString::from("--sign"),
        std::ffi::OsString::from("-"),
        std::ffi::OsString::from("--identifier"),
        std::ffi::OsString::from(identifier),
    ];
    args.push(binary_path.as_os_str().to_os_string());
    args
}

#[cfg(any(target_os = "macos", test))]
#[cfg(test)]
fn macos_cli_codesign_args(binary_path: &Path) -> Vec<std::ffi::OsString> {
    macos_cli_codesign_args_with_identifier(binary_path, MACOS_CLI_CODESIGN_IDENTIFIER)
}

#[cfg(target_os = "macos")]
fn ad_hoc_codesign_macos_binary(binary_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    ad_hoc_codesign_macos_binary_with_identifier(binary_path, MACOS_CLI_CODESIGN_IDENTIFIER)
}

#[cfg(target_os = "macos")]
fn ad_hoc_codesign_macos_binary_with_identifier(
    binary_path: &Path,
    identifier: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(macos_cli_codesign_args_with_identifier(
            binary_path,
            identifier,
        ))
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "codesign failed for {} with identifier {}: {}{}",
        binary_path.display(),
        MACOS_CLI_CODESIGN_IDENTIFIER,
        stdout,
        stderr
    )
    .into())
}

#[cfg(not(target_os = "macos"))]
fn ad_hoc_codesign_macos_binary(_binary_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn ad_hoc_codesign_macos_binary_with_identifier(
    _binary_path: &Path,
    _identifier: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
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
    plugin_manager: &std::sync::Arc<tokio::sync::Mutex<ChannelPluginManager>>,
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
            let built_in_discoverer = BuiltInPluginDiscoverer::with_dispatcher(
                config.channels.clone(),
                state.threads.router.clone(),
                bridge.clone(),
                state.channel_dispatcher(),
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
                // Weak handle so the `request_self_replace` host
                // RPC can drive respawn after a successful swap
                // without forming a manager↔handler reference
                // cycle that would survive gateway shutdown.
                plugin_manager: std::sync::Arc::downgrade(plugin_manager),
                // Plugin-side master kill switch is the existing
                // `plugins.auto_update` config field. Captured here
                // (one-shot read) so handlers see the right
                // initial value; future hot-reload paths flip the
                // AtomicBool without rebuilding handlers.
                plugin_auto_update_enabled: std::sync::Arc::new(
                    std::sync::atomic::AtomicBool::new(config.plugins.auto_update),
                ),
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

    let RuntimeAssembly {
        state,
        bridge,
        cron_service,
    } = RuntimeAssembler::new(&config_path, config.clone())
        .assemble()
        .await?;

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

        // Architecture C: host no longer runs a periodic plugin
        // update loop. Each plugin owns its own timer + advertised-
        // version source and sends `request_self_replace` reverse
        // RPCs when it decides to upgrade; the host responds with
        // the safe-swap pipeline (idle gate + atomic rename +
        // respawn) inside `plugin_self_replace::handle`. The
        // `plugins.auto_update` config flag now controls whether
        // those RPCs are accepted at all, gated per-call rather
        // than at spawn-time.

        // Spawn the gateway self-updater. Separate loop from the
        // plugin one because the two have independent kill switches
        // (`gateway.auto_update.enabled` vs `plugins.auto_update`)
        // and target different release sources. We keep the handle
        // alive but don't track it in `auto_update_handle` — that
        // one is plugin-specific (e.g. hot-reload may want to cycle
        // it). Gateway self-update is intentionally fire-and-forget;
        // there's no scenario where we restart this loop without
        // also restarting the gateway, so a hot-reload doesn't need
        // to manage it.
        let _gateway_auto_update_handle = crate::gateway_auto_update::spawn(
            plugin_manager.clone(),
            config.gateway.auto_update.clone(),
        );

        // Runtime services are started and wired by RuntimeAssembler.
        let gateway = Gateway::new(state);
        let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
        tracing::info!("Gateway listening on {}", addr);
        gateway.serve(addr).await?;
        Ok(())
    }
    .await;

    let metrics = hot_reloader.metrics();
    tracing::info!(
        attempts = metrics.attempts,
        successes = metrics.successes,
        failures = metrics.failures,
        callback_notifications = metrics.callback_notifications,
        "config hot-reload metrics"
    );
    drop(hot_reloader);

    // Always run shutdown sequence, even when startup/serve fails.
    //
    {
        let mut plugin_manager = plugin_manager.lock().await;
        plugin_manager.stop_all().await;
        plugin_manager.cleanup_all().await;
    }

    match Arc::try_unwrap(cron_service) {
        Ok(mut svc) => svc.stop().await,
        Err(_) => tracing::debug!("Cron service still has outstanding references on shutdown"),
    }
    run_result
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
            let plugin_schemas = discover_installed_plugin_schemas()
                .map(|(schemas, _)| schemas)
                .unwrap_or_default();
            let issues = validate_channel_account_configs(&loaded.config, &plugin_schemas);
            if !issues.is_empty() {
                print_config_validation_issues(&issues);
                std::process::exit(1);
            }
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

pub(crate) async fn cmd_config_claude_cli(
    config_path: &str,
    mode: Option<String>,
    path: Option<String>,
    clear_path: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut value = serde_json::to_value(loaded.config)?;
    let root = value
        .as_object_mut()
        .ok_or("config root is not an object")?;
    let agents = root
        .entry("agents".to_owned())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("config.agents is not an object")?;
    let claude = agents
        .entry("claude".to_owned())
        .or_insert_with(|| json!({ "provider_type": "claude_code" }));
    if !claude.is_object() {
        *claude = json!({ "provider_type": "claude_code" });
    }
    let claude = claude.as_object_mut().expect("claude config object");
    claude.insert("provider_type".to_owned(), json!("claude_code"));

    if let Some(mode) = mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        claude.insert("claude_cli_mode".to_owned(), json!(mode));
    }
    if clear_path {
        claude.remove("claude_cli_path");
    } else if let Some(path) = path.as_deref().map(str::trim) {
        if path.is_empty() {
            claude.remove("claude_cli_path");
        } else {
            claude.insert("claude_cli_path".to_owned(), json!(path));
        }
    }

    let validated: GaryxConfig = serde_json::from_value(value.clone())?;
    save_config_struct(&config_path, &validated)?;
    notify_gateway_reload_quiet(&config_path).await;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(
                value
                    .get("agents")
                    .and_then(|agents| agents.get("claude"))
                    .unwrap_or(&Value::Null)
            )?
        );
    } else {
        let configured = value
            .get("agents")
            .and_then(|agents| agents.get("claude"))
            .and_then(Value::as_object);
        let mode = configured
            .and_then(|object| object.get("claude_cli_mode"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_claude_cli_mode);
        let path = configured
            .and_then(|object| object.get("claude_cli_path"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if path.is_empty() {
            println!("Claude Agent SDK CLI: mode={mode}, path=<auto>");
        } else {
            println!("Claude Agent SDK CLI: mode={mode}, path={path}");
        }
    }
    Ok(())
}

fn provider_model_config_key(provider_type: &ProviderType) -> Result<&'static str, String> {
    match provider_type {
        ProviderType::ClaudeCode => Ok("claude"),
        ProviderType::CodexAppServer => Ok("codex"),
        ProviderType::Traex => Ok("traex"),
        ProviderType::GeminiCli => Ok("gemini"),
        ProviderType::Gpt => Ok("gpt"),
        ProviderType::ClaudeLlm => Ok("anthropic"),
        ProviderType::GeminiLlm => Ok("google"),
        ProviderType::AgentTeam => Err("agent_team is not a model provider".to_owned()),
    }
}

pub(crate) async fn cmd_config_provider_model(
    config_path: &str,
    provider: &str,
    model: Option<String>,
    clear_model: bool,
    model_reasoning_effort: Option<String>,
    clear_model_reasoning_effort: bool,
    claude_cli_mode: Option<String>,
    clear_claude_cli_mode: bool,
    claude_cli_path: Option<String>,
    clear_claude_cli_path: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let provider_type = ProviderType::from_slug(provider)
        .ok_or_else(|| format!("unsupported provider type: {provider}"))?;
    let provider_key = provider_model_config_key(&provider_type)?;
    let has_claude_cli_update = claude_cli_mode.is_some()
        || clear_claude_cli_mode
        || claude_cli_path.is_some()
        || clear_claude_cli_path;
    if has_claude_cli_update && provider_type != ProviderType::ClaudeCode {
        return Err("--claude-cli-* options are only supported for provider claude_code".into());
    }
    if model.is_none()
        && !clear_model
        && model_reasoning_effort.is_none()
        && !clear_model_reasoning_effort
        && !has_claude_cli_update
    {
        return Err(
            "set --model, --clear-model, --model-reasoning-effort, --clear-model-reasoning-effort, --claude-cli-mode, --clear-claude-cli-mode, --claude-cli-path, or --clear-claude-cli-path"
                .into(),
        );
    }

    let mut provider_config = serde_json::Map::new();
    provider_config.insert("provider_type".to_owned(), json!(provider_type.as_slug()));
    if clear_model {
        provider_config.insert("default_model".to_owned(), json!(""));
    } else if let Some(model) = model.as_deref().map(str::trim) {
        provider_config.insert("default_model".to_owned(), json!(model));
    }
    if clear_model_reasoning_effort {
        provider_config.insert("model_reasoning_effort".to_owned(), json!(""));
    } else if let Some(effort) = model_reasoning_effort.as_deref().map(str::trim) {
        provider_config.insert("model_reasoning_effort".to_owned(), json!(effort));
    }
    if clear_claude_cli_mode {
        provider_config.insert("claude_cli_mode".to_owned(), json!(""));
    } else if let Some(mode) = claude_cli_mode.as_deref().map(str::trim) {
        provider_config.insert("claude_cli_mode".to_owned(), json!(mode));
    }
    if clear_claude_cli_path {
        provider_config.insert("claude_cli_path".to_owned(), json!(""));
    } else if let Some(path) = claude_cli_path.as_deref().map(str::trim) {
        provider_config.insert("claude_cli_path".to_owned(), json!(path));
    }

    let mut agents_patch = serde_json::Map::new();
    agents_patch.insert(
        provider_key.to_owned(),
        Value::Object(provider_config.clone()),
    );
    let patch = json!({
        "agents": Value::Object(agents_patch)
    });
    let gateway = gateway_endpoint(config_path)?;
    put_gateway_json(&gateway, "/api/settings?merge=true", &patch).await?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "provider": provider_type.as_slug(),
                "config_key": provider_key,
                "config": Value::Object(provider_config),
            }))?
        );
    } else {
        let default_model = provider_config
            .get("default_model")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        let effort = provider_config
            .get("model_reasoning_effort")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        let cli_mode = provider_config
            .get("claude_cli_mode")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        let cli_path = provider_config
            .get("claude_cli_path")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        println!(
            "Updated provider defaults: {} (key={provider_key}, model={default_model}, reasoning={effort}, claude_cli_mode={cli_mode}, claude_cli_path={cli_path})",
            provider_type.as_slug()
        );
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

/// Outcome of [`try_swap_garyx_binary`] — the unattended sibling to
/// the user-facing `cmd_update` path. Used by both callers (manual
/// CLI + background auto-update loop) to log a "from→to" line after
/// a successful swap, and by the loop to decide whether to SIGTERM
/// self for restart.
#[derive(Debug)]
pub(crate) struct SwapOutcome {
    /// Version that was installed pre-swap.
    pub from_version: String,
    /// Version that was installed post-swap (= the requested version).
    pub to_version: String,
    /// Final path of the installed binary on disk.
    pub install_path: PathBuf,
}

/// Download a specific garyx release from GitHub, verify it, codesign
/// it (macOS), and atomically swap it into `destination_path`. Used
/// by both `cmd_update` (manual CLI) and the gateway auto-update loop
/// (background tick). The function does NOT print anything — callers
/// log/print as appropriate for their context.
///
/// `requested_version` must already be normalized (no leading `v`).
/// `repo` is the GitHub `owner/repo` to download from; the gateway
/// loop passes `gateway.auto_update.github_repo` so fork-testing
/// retrieves both the tag AND the binary from the same fork (codex
/// review caught the asymmetry on landing — the "latest" lookup
/// honored the override but the asset download went to the const).
/// `destination_path` is where the new binary lands on success.
pub(crate) async fn try_swap_garyx_binary(
    requested_version: &str,
    repo: &str,
    destination_path: &Path,
) -> Result<SwapOutcome, Box<dyn std::error::Error>> {
    let target = detect_release_target()?;
    let parent = destination_path
        .parent()
        .ok_or_else(|| {
            format!(
                "update target has no parent directory: {}",
                destination_path.display()
            )
        })?
        .to_path_buf();

    let archive_name = format!("garyx-{requested_version}-{target}.tar.gz");
    let base_url = format!("https://github.com/{repo}/releases/download/v{requested_version}");

    let client = reqwest::Client::builder()
        .user_agent(format!("garyx-cli/{VERSION}"))
        .build()?;
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
    let extracted_cctty = tempdir
        .path()
        .join(format!("garyx-{requested_version}-{target}"))
        .join("cctty");
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
    ad_hoc_codesign_macos_binary(&staged_path)?;
    let cctty_destination = parent.join("cctty");
    let cctty_staged_path = if extracted_cctty.is_file() {
        let staged = parent.join(format!(".cctty-update-{}.tmp", Uuid::new_v4().simple()));
        fs::copy(&extracted_cctty, &staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&staged)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&staged, perms)?;
        }
        ad_hoc_codesign_macos_binary_with_identifier(&staged, MACOS_CCTTY_CODESIGN_IDENTIFIER)?;
        Some(staged)
    } else {
        None
    };
    fs::rename(&staged_path, destination_path)?;
    if let Some(staged) = cctty_staged_path {
        fs::rename(staged, cctty_destination)?;
    }

    Ok(SwapOutcome {
        from_version: VERSION.to_owned(),
        to_version: requested_version.to_owned(),
        install_path: destination_path.to_path_buf(),
    })
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
    let destination = replacement_binary_path(install_path)?;
    let target = detect_release_target()?;

    // Validate destination has a parent BEFORE the short-circuit so
    // `garyx update --path /nonexistent/dir/garyx` still surfaces the
    // missing-parent error even on the "already up to date" path.
    // Pre-refactor behavior was the same; codex review #4 caught the
    // accidental ordering change when this got pulled into a thin
    // wrapper around `try_swap_garyx_binary`.
    if destination.parent().is_none() {
        return Err(format!(
            "update target has no parent directory: {}",
            destination.display()
        )
        .into());
    }

    if version.is_none() && requested_version == VERSION {
        println!(
            "garyx is already up to date at v{} ({})",
            VERSION,
            destination.display()
        );
        return Ok(());
    }

    println!("Updating garyx to v{requested_version} for {target}...");
    let outcome =
        try_swap_garyx_binary(&requested_version, GITHUB_RELEASE_REPO, &destination).await?;
    println!(
        "Updated garyx from v{} to v{} at {}",
        outcome.from_version,
        outcome.to_version,
        outcome.install_path.display()
    );
    Ok(())
}

/// Print current auto-update state for `auto-update status`. Reads
/// the on-disk config (not the running gateway's in-memory state)
/// because the gateway may not be running, and a freshly-edited
/// config that hasn't been reloaded yet is what the user cares about.
pub(crate) async fn cmd_auto_update_status(
    config_path: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, Default::default())?;
    let gw = &loaded.config.gateway.auto_update;
    let plugins = &loaded.config.plugins;

    let latest = match reqwest::Client::builder()
        .user_agent(format!("garyx-cli/{VERSION}"))
        .build()
    {
        Ok(client) => latest_release_version(&client).await.ok(),
        Err(_) => None,
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "installed_version": VERSION,
                "latest_known_version": latest,
                "gateway": {
                    "enabled": gw.enabled,
                    "check_interval_secs": gw.check_interval_secs,
                    "github_repo": gw.github_repo,
                },
                "plugin": {
                    "enabled": plugins.auto_update,
                    "check_interval_secs": plugins.auto_update_check_interval_secs,
                },
            }))?
        );
    } else {
        println!("auto-update status (from {})", loaded.path.display());
        println!("  installed:   v{VERSION}");
        match latest.as_deref() {
            Some(v) => println!("  latest:      v{v}"),
            None => println!("  latest:      <fetch failed>"),
        }
        println!(
            "  gateway:     {} (every {}s, repo={})",
            if gw.enabled { "ENABLED" } else { "disabled" },
            gw.check_interval_secs,
            gw.github_repo,
        );
        println!(
            "  plugin:      {} (every {}s)",
            if plugins.auto_update {
                "ENABLED"
            } else {
                "disabled"
            },
            plugins.auto_update_check_interval_secs,
        );
    }
    Ok(())
}

/// Implementation shared by `cmd_auto_update_disable` and
/// `cmd_auto_update_enable`. `target_gateway` and `target_plugin`
/// describe which loops to touch (both true means "all"); `enabled`
/// is the new value. Returns the post-mutation tuple `(gateway,
/// plugin)` so the caller can print a sensible summary.
async fn set_auto_update_flags(
    config_path: &str,
    target_gateway: bool,
    target_plugin: bool,
    enabled: bool,
) -> Result<(bool, bool), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, Default::default())?;
    let resolved_config_path = loaded.path;
    let mut config = loaded.config;

    // No explicit target → touch both. Matches the help text on the
    // CLI subcommands.
    let (touch_gateway, touch_plugin) = if !target_gateway && !target_plugin {
        (true, true)
    } else {
        (target_gateway, target_plugin)
    };

    if touch_gateway {
        config.gateway.auto_update.enabled = enabled;
    }
    if touch_plugin {
        config.plugins.auto_update = enabled;
    }

    save_config_struct(&resolved_config_path, &config)?;
    notify_gateway_reload_quiet(&resolved_config_path).await;

    Ok((
        config.gateway.auto_update.enabled,
        config.plugins.auto_update,
    ))
}

pub(crate) async fn cmd_auto_update_disable(
    config_path: &str,
    gateway: bool,
    plugin: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (gw_after, plugin_after) =
        set_auto_update_flags(config_path, gateway, plugin, false).await?;
    println!(
        "auto-update updated: gateway={} plugin={}",
        if gw_after { "ENABLED" } else { "disabled" },
        if plugin_after { "ENABLED" } else { "disabled" },
    );
    Ok(())
}

pub(crate) async fn cmd_auto_update_enable(
    config_path: &str,
    gateway: bool,
    plugin: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (gw_after, plugin_after) =
        set_auto_update_flags(config_path, gateway, plugin, true).await?;
    println!(
        "auto-update updated: gateway={} plugin={}",
        if gw_after { "ENABLED" } else { "disabled" },
        if plugin_after { "ENABLED" } else { "disabled" },
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

async fn gateway_is_reachable(config_path: &Path) -> bool {
    let path_str = config_path.to_string_lossy();
    let Ok(gateway) = gateway_endpoint(&path_str) else {
        return false;
    };
    let url = format!("{}/health", gateway.base_url);
    let response = gateway_request(
        reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(5)),
        &gateway,
    )
    .send()
    .await;
    matches!(response, Ok(r) if r.status().is_success())
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

async fn post_gateway_json_with_timeout(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
    timeout_secs: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().post(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(timeout_secs.max(1)))
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

// ---------------------------------------------------------------------------
// Scheduled automation commands
// ---------------------------------------------------------------------------

fn trim_required_cli(value: &str, field: &str) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty").into());
    }
    Ok(trimmed.to_owned())
}

fn trim_optional_cli(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

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

pub(crate) async fn cmd_workflow_definition_list(
    config_path: &str,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/workflow-definitions?limit={limit}&offset={offset}"),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    let definitions = payload["workflowDefinitions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if definitions.is_empty() {
        println!("No workflow definitions.");
        return Ok(());
    }
    println!("{:<34}  {:<7}  NAME", "WORKFLOW ID", "VERSION");
    println!("{}", "-".repeat(90));
    for definition in definitions {
        println!(
            "{:<34}  {:<7}  {}",
            definition["workflowId"].as_str().unwrap_or("-"),
            definition["version"].as_u64().unwrap_or_default(),
            definition["name"].as_str().unwrap_or("-")
        );
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_definition_get(
    config_path: &str,
    workflow_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_id, "workflow_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/workflow-definitions/{}",
            urlencoding::encode(&workflow_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_definition_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_workflow_definition_upsert(
    config_path: &str,
    file: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = PathBuf::from(file);
    let (package_dir, manifest_path) = workflow_package_source(&manifest_path)?;
    let raw = std::fs::read_to_string(&manifest_path)?;
    let body: Value = serde_json::from_str(&raw)?;
    let workflow_id = body
        .get("workflowId")
        .or_else(|| body.get("workflow_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "workflowId is required"))?
        .to_owned();
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let root = workflow_definitions_root_for_config(&config);
    let destination = root.join(workflow_package_dir_name(&workflow_id));
    install_workflow_package(&package_dir, &destination)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/workflow-definitions/{}",
            urlencoding::encode(&workflow_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_definition_summary(&payload);
    Ok(())
}

const WORKFLOW_MANIFEST_FILE: &str = "garyx.workflow.json";

fn workflow_package_source(path: &Path) -> io::Result<(PathBuf, PathBuf)> {
    if path.is_dir() {
        let manifest = path.join(WORKFLOW_MANIFEST_FILE);
        if !manifest.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("workflow package is missing {WORKFLOW_MANIFEST_FILE}"),
            ));
        }
        return Ok((path.to_path_buf(), manifest));
    }
    let package_dir = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "workflow manifest must have a parent directory",
        )
    })?;
    Ok((package_dir.to_path_buf(), path.to_path_buf()))
}

fn workflow_definitions_root_for_config(config: &GaryxConfig) -> PathBuf {
    let data_dir = config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(default_session_data_dir);
    if data_dir.file_name().and_then(|name| name.to_str()) == Some("data") {
        data_dir
            .parent()
            .map(|parent| parent.join("workflows"))
            .unwrap_or_else(|| data_dir.join("workflows"))
    } else {
        data_dir.join("workflows")
    }
}

fn workflow_package_dir_name(workflow_id: &str) -> String {
    let mut output = workflow_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    output = output.trim_matches('-').to_owned();
    if output.is_empty() {
        "workflow".to_owned()
    } else {
        output
    }
}

fn install_workflow_package(source: &Path, destination: &Path) -> io::Result<()> {
    let source = source.canonicalize()?;
    let destination_parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "workflow package destination must have a parent",
        )
    })?;
    fs::create_dir_all(destination_parent)?;
    let destination_canonical = destination.canonicalize().ok();
    if destination_canonical.as_deref() == Some(source.as_path()) {
        return Ok(());
    }
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    copy_dir_all(&source, destination)
}

fn copy_dir_all(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_list(
    config_path: &str,
    parent_thread_id: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let path = if let Some(thread_id) = trim_optional_cli(parent_thread_id) {
        format!(
            "/api/workflows?parentThreadId={}",
            urlencoding::encode(&thread_id)
        )
    } else {
        "/api/workflows".to_owned()
    };
    let payload = fetch_gateway_json(&gateway, &path).await?;
    if json {
        return print_pretty_json(&payload);
    }
    let workflows = payload["workflows"].as_array().cloned().unwrap_or_default();
    if workflows.is_empty() {
        println!("Workflows: (none)");
        return Ok(());
    }
    println!("{:<42}  {:<10}  {:<20}  NAME", "RUN ID", "STATUS", "PARENT");
    println!("{}", "-".repeat(100));
    for workflow in workflows {
        let workflow_id = workflow["workflowRunId"]
            .as_str()
            .or_else(|| workflow["workflowId"].as_str())
            .unwrap_or("-");
        let status = workflow["status"].as_str().unwrap_or("-");
        let parent = workflow["parentThreadId"].as_str().unwrap_or("-");
        let name = workflow["name"].as_str().unwrap_or("-");
        println!("{workflow_id:<42}  {status:<10}  {parent:<20}  {name}");
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_get(
    config_path: &str,
    workflow_run_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_run_id, "workflow_run_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/workflows/{}", urlencoding::encode(&workflow_id)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_workflow_events(
    config_path: &str,
    workflow_run_id: &str,
    after: u64,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_run_id, "workflow_run_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/workflows/{}/events?after={after}",
            urlencoding::encode(&workflow_id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    for event in payload["events"].as_array().cloned().unwrap_or_default() {
        let seq = event["eventSeq"].as_u64().unwrap_or_default();
        let typ = event["eventType"].as_str().unwrap_or("-");
        let child = event["workflowChildRunId"].as_str().unwrap_or("-");
        println!("{seq:<8}  {typ:<28}  {child}");
    }
    Ok(())
}

pub(crate) async fn cmd_workflow_cancel(
    config_path: &str,
    workflow_run_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow_id = trim_required_cli(workflow_run_id, "workflow_run_id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        &format!(
            "/api/workflows/{}/cancel",
            urlencoding::encode(&workflow_id)
        ),
        &json!({}),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_workflow_summary(&payload);
    Ok(())
}

fn print_workflow_summary(payload: &Value) {
    let workflow = payload.get("workflow").unwrap_or(payload);
    println!(
        "Workflow Run: {}",
        workflow["workflowRunId"]
            .as_str()
            .or_else(|| workflow["workflowId"].as_str())
            .unwrap_or("-")
    );
    println!("Name: {}", workflow["name"].as_str().unwrap_or("-"));
    println!("Status: {}", workflow["status"].as_str().unwrap_or("-"));
    println!(
        "Parent: {}",
        workflow["parentThreadId"].as_str().unwrap_or("-")
    );
    if let Some(output_text) = workflow["outputText"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        println!("Output: {output_text}");
    }
    if let Some(error) = workflow["error"].as_str().filter(|value| !value.is_empty()) {
        println!("Error: {error}");
    }
    let children = payload["children"].as_array().cloned().unwrap_or_default();
    if !children.is_empty() {
        println!("Children:");
        for child in children {
            println!(
                "- {} [{}] thread {}",
                child["label"].as_str().unwrap_or("-"),
                child["status"].as_str().unwrap_or("-"),
                child["threadId"].as_str().unwrap_or("-"),
            );
        }
    }
}

fn print_workflow_definition_summary(payload: &Value) {
    let definition = payload.get("workflowDefinition").unwrap_or(payload);
    println!(
        "Workflow Definition: {}",
        definition["workflowId"].as_str().unwrap_or("-")
    );
    println!("Name: {}", definition["name"].as_str().unwrap_or("-"));
    println!(
        "Version: {}",
        definition["version"].as_u64().unwrap_or_default()
    );
    if let Some(description) = definition["description"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
    {
        println!("Description: {description}");
    }
}

// ---------------------------------------------------------------------------
// App database commands
// ---------------------------------------------------------------------------

fn parse_db_field_spec(spec: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let (name, field_type) = spec
        .split_once(':')
        .ok_or_else(|| format!("field spec must be name:TYPE, got {spec}"))?;
    Ok(json!({
        "name": trim_required_cli(name, "field name")?,
        "type": trim_required_cli(field_type, "field type")?,
    }))
}

fn parse_json_object(
    input: &str,
    label: &str,
) -> Result<Map<String, Value>, Box<dyn std::error::Error>> {
    let value = serde_json::from_str::<Value>(input)?;
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(format!("{label} must be a JSON object").into()),
    }
}

fn parse_optional_json_value(
    input: Option<String>,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    input
        .map(|value| serde_json::from_str::<Value>(&value).map_err(Into::into))
        .transpose()
}

fn db_print_table_list(payload: &Value) {
    let tables = payload["tables"].as_array().cloned().unwrap_or_default();
    if tables.is_empty() {
        println!("Tables: (none)");
        return;
    }
    println!(
        "{:<32}  {:<8}  {:<8}  DISPLAY",
        "TABLE", "VERSION", "RECORDS"
    );
    println!("{}", "-".repeat(72));
    for table in tables {
        println!(
            "{:<32}  {:<8}  {:<8}  {}",
            table["table_name"].as_str().unwrap_or("-"),
            table["schema_version"].as_i64().unwrap_or_default(),
            table["record_count"].as_i64().unwrap_or_default(),
            table["display_name"].as_str().unwrap_or("-")
        );
    }
}

fn db_print_schema(payload: &Value) {
    println!("Table: {}", payload["table_name"].as_str().unwrap_or("-"));
    if let Some(display_name) = payload["display_name"].as_str() {
        println!("Display: {display_name}");
    }
    println!(
        "Schema version: {}",
        payload["schema_version"].as_i64().unwrap_or_default()
    );
    println!();
    println!(
        "{:<24}  {:<8}  {:<8}  {:<6}  {:<6}  DISPLAY",
        "FIELD", "TYPE", "NOTNULL", "UNIQ", "INDEX"
    );
    println!("{}", "-".repeat(86));
    for field in payload["system_fields"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(payload["fields"].as_array().into_iter().flatten())
    {
        println!(
            "{:<24}  {:<8}  {:<8}  {:<6}  {:<6}  {}",
            field["name"].as_str().unwrap_or("-"),
            field["type"].as_str().unwrap_or("-"),
            field["not_null"].as_bool().unwrap_or(false),
            field["unique"].as_bool().unwrap_or(false),
            field["indexed"].as_bool().unwrap_or(false),
            field["display_name"].as_str().unwrap_or("-"),
        );
    }
}

fn db_print_sql_result(payload: &Value) {
    let rows = payload["rows"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("Rows: (none)");
    } else {
        for row in rows {
            println!(
                "{}",
                serde_json::to_string(&row).unwrap_or_else(|_| row.to_string())
            );
        }
    }
    if payload["truncated"].as_bool().unwrap_or(false) {
        println!("Result truncated");
    }
}

fn db_print_events(payload: &Value) {
    let events = payload["events"].as_array().cloned().unwrap_or_default();
    if events.is_empty() {
        println!("Events: (none)");
        return;
    }
    println!(
        "{:<34}  {:<15}  {:<24}  {:<24}  RECORD",
        "EVENT", "TYPE", "TABLE", "CREATED"
    );
    println!("{}", "-".repeat(118));
    for event in events {
        println!(
            "{:<34}  {:<15}  {:<24}  {:<24}  {}",
            event["id"].as_str().unwrap_or("-"),
            event["event_type"].as_str().unwrap_or("-"),
            event["table_name"].as_str().unwrap_or("-"),
            event["created_at"].as_str().unwrap_or("-"),
            event["record_id"].as_str().unwrap_or("-"),
        );
    }
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

pub(crate) async fn cmd_db_table_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/db/tables").await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_table_list(&payload);
    Ok(())
}

pub(crate) async fn cmd_db_table_create(
    config_path: &str,
    table: &str,
    display_name: Option<String>,
    fields: Vec<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let fields = fields
        .iter()
        .map(|field| parse_db_field_spec(field))
        .collect::<Result<Vec<_>, _>>()?;
    let mut body = json!({
        "table_name": table,
        "fields": fields,
    });
    if let Some(display_name) = trim_optional_cli(display_name) {
        body["display_name"] = json!(display_name);
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(&gateway, "/api/db/tables", &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "Created table: {}",
        payload["event"]["table_name"].as_str().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_db_table_schema(
    config_path: &str,
    table: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/db/tables/{}", urlencoding::encode(&table)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_schema(&payload);
    Ok(())
}

pub(crate) async fn cmd_db_table_drop(
    config_path: &str,
    table: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/db/tables/{}", urlencoding::encode(&table)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Dropped table: {table}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_db_field_add(
    config_path: &str,
    table: &str,
    field: &str,
    field_type: &str,
    not_null: bool,
    unique: bool,
    indexed: bool,
    display_name: Option<String>,
    default_value: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let mut body = json!({
        "name": trim_required_cli(field, "field")?,
        "type": trim_required_cli(field_type, "type")?,
        "not_null": not_null,
        "unique": unique,
        "indexed": indexed,
    });
    if let Some(display_name) = trim_optional_cli(display_name) {
        body["display_name"] = json!(display_name);
    }
    if let Some(default_value) = parse_optional_json_value(default_value)? {
        body["default"] = default_value;
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/db/tables/{}/fields", urlencoding::encode(&table)),
        &body,
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "Added field: {table}.{}",
        body["name"].as_str().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_db_field_drop(
    config_path: &str,
    table: &str,
    field: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let field = trim_required_cli(field, "field")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!(
            "/api/db/tables/{}/fields/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&field)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Dropped field: {table}.{field}");
    Ok(())
}

pub(crate) async fn cmd_db_record_insert(
    config_path: &str,
    table: &str,
    data: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let body = json!({ "record": parse_json_object(data, "data")? });
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/db/tables/{}/records", urlencoding::encode(&table)),
        &body,
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "Inserted record: {}",
        payload["record"]["id"].as_str().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_db_record_get(
    config_path: &str,
    table: &str,
    id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let id = trim_required_cli(id, "id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/db/tables/{}/records/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_pretty_json(&payload["record"])?;
    Ok(())
}

pub(crate) async fn cmd_db_record_update(
    config_path: &str,
    table: &str,
    id: &str,
    data: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let id = trim_required_cli(id, "id")?;
    let body = json!({ "record": parse_json_object(data, "data")? });
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &format!(
            "/api/db/tables/{}/records/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&id)
        ),
        &body,
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Updated record: {id}");
    Ok(())
}

pub(crate) async fn cmd_db_record_delete(
    config_path: &str,
    table: &str,
    id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let id = trim_required_cli(id, "id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!(
            "/api/db/tables/{}/records/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Deleted record: {id}");
    Ok(())
}

pub(crate) async fn cmd_db_sql(
    config_path: &str,
    sql: Vec<String>,
    limit: Option<usize>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let sql = trim_required_cli(&sql.join(" "), "sql")?;
    let mut body = json!({ "sql": sql });
    if let Some(limit) = limit {
        body["limit"] = json!(limit);
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(&gateway, "/api/db/sql", &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_sql_result(&payload);
    Ok(())
}

pub(crate) async fn cmd_db_events(
    config_path: &str,
    table: Option<String>,
    event_type: Option<String>,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut query = vec![format!("limit={limit}"), format!("offset={offset}")];
    if let Some(table) = trim_optional_cli(table) {
        query.push(format!("table={}", urlencoding::encode(&table)));
    }
    if let Some(event_type) = trim_optional_cli(event_type) {
        query.push(format!("eventType={}", urlencoding::encode(&event_type)));
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload =
        fetch_gateway_json(&gateway, &format!("/api/db/events?{}", query.join("&"))).await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_events(&payload);
    Ok(())
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

// ---------------------------------------------------------------------------
// Custom Agent commands
// ---------------------------------------------------------------------------

pub(crate) async fn cmd_agent_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/custom-agents").await?;
    if json {
        return print_pretty_json(&decorate_agent_list_json(payload));
    }
    let mut agents = payload["agents"].as_array().cloned().unwrap_or_default();
    sort_agents_builtin_first(&mut agents);
    if agents.is_empty() {
        println!("Agents: (none)");
        return Ok(());
    }
    for a in &agents {
        print_agent_summary(a);
        println!();
    }
    Ok(())
}

fn sort_agents_builtin_first(agents: &mut [Value]) {
    agents.sort_by(|a, b| {
        let a_builtin = a["built_in"].as_bool().unwrap_or(false);
        let b_builtin = b["built_in"].as_bool().unwrap_or(false);
        // Reversed: builtin (true) should sort before custom (false).
        b_builtin.cmp(&a_builtin).then_with(|| {
            let a_id = a["agent_id"].as_str().unwrap_or("");
            let b_id = b["agent_id"].as_str().unwrap_or("");
            a_id.cmp(b_id)
        })
    });
}

fn decorate_agent_list_json(mut payload: Value) -> Value {
    if let Some(agents) = payload
        .get_mut("agents")
        .and_then(|value| value.as_array_mut())
    {
        sort_agents_builtin_first(agents);
        for agent in agents {
            let is_builtin = agent["built_in"].as_bool().unwrap_or(false);
            if let Some(obj) = agent.as_object_mut() {
                obj.insert(
                    "kind".to_string(),
                    Value::String(if is_builtin { "builtin" } else { "custom" }.to_string()),
                );
            }
        }
    }
    payload
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
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    provider_auth_source: Option<String>,
    provider_api_key: Option<String>,
    default_workspace_dir: Option<String>,
    system_prompt: String,
) -> Result<Value, Box<dyn std::error::Error>> {
    let agent_id = agent_id.trim().to_owned();
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".into());
    }
    let mut body = json!({
        "agent_id": agent_id,
        "display_name": display_name.trim(),
        "provider_type": provider.trim(),
        "model": model.as_deref().map(str::trim).unwrap_or(""),
        "model_reasoning_effort": model_reasoning_effort.as_deref().map(str::trim).unwrap_or(""),
        "model_service_tier": model_service_tier.as_deref().map(str::trim).unwrap_or(""),
        "system_prompt": system_prompt,
    });
    let provider_type = ProviderType::from_slug(provider.trim());
    let auth_source = provider_auth_source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(auth_source) = auth_source {
        body["auth_source"] = Value::String(auth_source.to_owned());
    }
    if let Some(api_key) = provider_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let env_name = match provider_type.as_ref() {
            Some(ProviderType::Gpt) => "OPENAI_API_KEY",
            Some(ProviderType::ClaudeLlm) => "ANTHROPIC_API_KEY",
            Some(ProviderType::GeminiLlm) => "GEMINI_API_KEY",
            _ => {
                return Err(
                    "--provider-api-key is only supported for gpt, anthropic, or google providers"
                        .into(),
                );
            }
        };
        body["provider_env"] = json!({ env_name: api_key });
        if matches!(provider_type, Some(ProviderType::Gpt)) && auth_source.is_none() {
            body["auth_source"] = Value::String("api_key".to_owned());
        }
    }
    if let Some(default_workspace_dir) = default_workspace_dir {
        body["default_workspace_dir"] = Value::String(default_workspace_dir.trim().to_owned());
    }
    Ok(body)
}

pub(crate) async fn cmd_agent_create(
    config_path: &str,
    agent_id: String,
    display_name: String,
    provider: String,
    model: Option<String>,
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    provider_auth_source: Option<String>,
    provider_api_key: Option<String>,
    default_workspace_dir: Option<String>,
    system_prompt: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let body = build_agent_mutation_body(
        agent_id,
        display_name,
        provider,
        model,
        model_reasoning_effort,
        model_service_tier,
        provider_auth_source,
        provider_api_key,
        default_workspace_dir,
        system_prompt,
    )?;
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
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    provider_auth_source: Option<String>,
    provider_api_key: Option<String>,
    default_workspace_dir: Option<String>,
    system_prompt: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let body = build_agent_mutation_body(
        agent_id.clone(),
        display_name,
        provider,
        model,
        model_reasoning_effort,
        model_service_tier,
        provider_auth_source,
        provider_api_key,
        default_workspace_dir,
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
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    provider_auth_source: Option<String>,
    provider_api_key: Option<String>,
    default_workspace_dir: Option<String>,
    system_prompt: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let body = build_agent_mutation_body(
        agent_id.clone(),
        display_name,
        provider,
        model,
        model_reasoning_effort,
        model_service_tier,
        provider_auth_source,
        provider_api_key,
        default_workspace_dir,
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
    let model_reasoning_effort = a["model_reasoning_effort"].as_str().unwrap_or("").trim();
    let model_service_tier = a["model_service_tier"].as_str().unwrap_or("").trim();
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
    if !model_reasoning_effort.is_empty() {
        println!("Reasoning effort: {model_reasoning_effort}");
    }
    if !model_service_tier.is_empty() {
        println!("Service tier: {model_service_tier}");
    }
    if let Some(default_workspace_dir) = a["default_workspace_dir"].as_str()
        && !default_workspace_dir.trim().is_empty()
    {
        println!("Default workspace: {}", default_workspace_dir.trim());
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
    worktree: bool,
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

#[derive(Debug, Clone)]
struct ImageGenerationCliResult {
    path: PathBuf,
    bytes: usize,
    media_type: Option<String>,
    runtime_thread_id: String,
    run_id: String,
    extra_images_seen: bool,
}

const TOOL_SEARCH_GEMINI_MODEL: &str = "gemini-3-flash-preview";

fn tool_workspace_dir(tool_name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = gary_home_dir().join("tool-workspaces").join(tool_name);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImageGenerationEventError {
    MalformedPayload(String),
}

impl std::fmt::Display for ImageGenerationEventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedPayload(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ImageGenerationEventError {}

#[derive(Debug)]
struct ToolProviderRun {
    runtime_thread_id: String,
    run_id: String,
    events: Vec<StreamEvent>,
}

async fn run_provider_tool(
    config_path: &str,
    provider_type: ProviderType,
    tool_name: &str,
    message: String,
    timeout_secs: u64,
    metadata: HashMap<String, Value>,
) -> Result<ToolProviderRun, Box<dyn std::error::Error>> {
    if timeout_secs == 0 {
        return Err("timeout must be greater than 0 seconds".into());
    }

    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let bridge = MultiProviderBridge::new();
    bridge.initialize_from_config(&loaded.config).await?;

    let workspace_dir = tool_workspace_dir(tool_name)?;
    let runtime_thread_id = format!("tool::{tool_name}::{}", Uuid::new_v4());
    let run_id = format!("tool-run-{}", Uuid::new_v4());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        let _ = tx.send(event);
    });

    let request = AgentRunRequest::new(
        runtime_thread_id.clone(),
        message,
        run_id.clone(),
        "tool",
        tool_name,
        metadata,
    )
    .with_workspace_dir(Some(workspace_dir.to_string_lossy().into_owned()))
    .with_requested_provider(Some(provider_type));

    if let Err(error) = bridge.start_agent_run(request, Some(callback)).await {
        bridge.shutdown().await;
        return Err(error.into());
    }

    let deadline = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(deadline);
    let mut events = Vec::new();

    loop {
        tokio::select! {
            _ = &mut deadline => {
                let _ = bridge.abort_run(&run_id).await;
                bridge.shutdown().await;
                return Err(format!("timed out after {timeout_secs}s waiting for provider tool `{tool_name}`").into());
            }
            event = rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                let done = matches!(event, StreamEvent::Done);
                events.push(event);
                if done {
                    break;
                }
            }
        }
    }

    bridge.shutdown().await;
    Ok(ToolProviderRun {
        runtime_thread_id,
        run_id,
        events,
    })
}

pub(crate) async fn cmd_tool_image(
    config_path: &str,
    prompt: String,
    output: PathBuf,
    timeout_secs: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = run_tool_image(config_path, &prompt, output, timeout_secs).await?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "path": result.path.display().to_string(),
                "bytes": result.bytes,
                "media_type": result.media_type,
                "runtime_thread_id": result.runtime_thread_id,
                "run_id": result.run_id,
                "extra_images_seen": result.extra_images_seen,
            }))?
        );
        return Ok(());
    }

    println!("Saved image: {}", result.path.display());
    println!("Bytes: {}", result.bytes);
    if let Some(media_type) = result.media_type.as_deref() {
        println!("Media type: {media_type}");
    }
    println!("Runtime thread: {}", result.runtime_thread_id);
    println!("Run: {}", result.run_id);
    if result.extra_images_seen {
        println!("Extra images were generated and ignored.");
    }
    Ok(())
}

async fn run_tool_image(
    config_path: &str,
    prompt: &str,
    output: PathBuf,
    timeout_secs: u64,
) -> Result<ImageGenerationCliResult, Box<dyn std::error::Error>> {
    let provider_run = run_provider_tool(
        config_path,
        ProviderType::CodexAppServer,
        "image",
        build_image_generation_prompt(prompt),
        timeout_secs,
        HashMap::from([("source".to_owned(), json!("garyx_tool_image"))]),
    )
    .await?;
    let mut first_image: Option<GeneratedImageResult> = None;
    let mut extra_images_seen = false;

    for event in &provider_run.events {
        if let Some(image) = extract_image_from_stream_event(event)? {
            if first_image.is_some() {
                extra_images_seen = true;
            } else {
                first_image = Some(image);
            }
        }
    }

    let image = first_image.ok_or("CodeX completed without generating an image")?;
    let output = resolve_image_output_path(output, image.extension);
    write_generated_image_output(&output, &image.bytes).await?;
    Ok(ImageGenerationCliResult {
        path: output,
        bytes: image.bytes.len(),
        media_type: image.media_type,
        runtime_thread_id: provider_run.runtime_thread_id,
        run_id: provider_run.run_id,
        extra_images_seen,
    })
}

fn extract_image_from_tool_result_message(
    message: &ProviderMessage,
) -> Result<Option<GeneratedImageResult>, ImageGenerationEventError> {
    if provider_message_item_type(message) != Some("imageGeneration") {
        return Ok(None);
    }
    let result = message
        .content
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if result.is_empty() {
        return Ok(None);
    }
    extract_image_generation_result(message)
        .map(Some)
        .ok_or_else(|| {
            ImageGenerationEventError::MalformedPayload(
                "generated image payload was malformed or not valid base64".to_owned(),
            )
        })
}

fn extract_image_from_stream_event(
    event: &StreamEvent,
) -> Result<Option<GeneratedImageResult>, ImageGenerationEventError> {
    match event {
        StreamEvent::ToolResult { message } => extract_image_from_tool_result_message(message),
        _ => Ok(None),
    }
}

fn resolve_image_output_path(output: PathBuf, extension: &str) -> PathBuf {
    if output.extension().is_some() {
        output
    } else {
        output.with_extension(extension)
    }
}

async fn write_generated_image_output(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SearchSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SearchToolMetadata {
    tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sources: Vec<SearchSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SearchStreamState {
    pub(crate) answer: String,
    #[cfg(test)]
    pub(crate) thread_id: Option<String>,
    #[cfg(test)]
    pub(crate) run_id: Option<String>,
    pub(crate) searched: bool,
    pub(crate) sources: Vec<SearchSource>,
    pub(crate) tool_metadata: Vec<SearchToolMetadata>,
}

#[derive(Debug, Clone, Serialize)]
struct SearchCommandOutput {
    ok: bool,
    query: String,
    answer: String,
    sources: Vec<SearchSource>,
    runtime_thread_id: String,
    run_id: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    searched: bool,
    tool_metadata: Vec<SearchToolMetadata>,
}

#[derive(Debug, Default)]
struct GeminiCliSearchSummary {
    session_id: Option<String>,
    model: Option<String>,
    status: Option<String>,
    duration_ms: Option<u64>,
}

fn build_gemini_search_prompt(query: &str) -> String {
    format!(
        "You are handling `garyx tool search`.\n\n\
You must use Gemini CLI's provider-native `google_web_search` tool for this request. \
Do not use Garyx MCP `search`, do not call any Garyx MCP web search helper, \
and do not answer only from memory. The tool call is mandatory even if you already know the answer.\n\n\
After the search tool returns, write a concise answer followed by source citations.\n\n\
<user_query_verbatim>\n{query}\n</user_query_verbatim>"
    )
}

fn gemini_search_policy_text() -> &'static str {
    r#"[[rule]]
toolName = "*"
decision = "deny"
priority = 900
interactive = false

[[rule]]
toolName = "google_web_search"
decision = "allow"
priority = 999
interactive = false
"#
}

#[derive(Debug)]
struct TemporaryGeminiSearchPolicy {
    path: PathBuf,
    dir: PathBuf,
}

impl TemporaryGeminiSearchPolicy {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryGeminiSearchPolicy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

async fn write_gemini_search_policy() -> io::Result<TemporaryGeminiSearchPolicy> {
    let dir = std::env::temp_dir().join(format!("garyx-gemini-search-policy-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("search-tool-only-policy.toml");
    tokio::fs::write(&path, gemini_search_policy_text()).await?;
    Ok(TemporaryGeminiSearchPolicy { path, dir })
}

fn strip_source_url_punctuation(url: &str) -> &str {
    url.trim_matches(|ch: char| matches!(ch, ')' | ']' | '}' | '>' | '.' | ',' | ';' | ':'))
}

fn push_source_unique(sources: &mut Vec<SearchSource>, source: SearchSource) {
    if source.url.trim().is_empty() || sources.iter().any(|item| item.url == source.url) {
        return;
    }
    sources.push(source);
}

pub(crate) fn extract_search_sources_from_text(text: &str) -> Vec<SearchSource> {
    let mut sources = Vec::new();
    let mut remainder = text;
    while let Some(open) = remainder.find('[') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find("](") else {
            remainder = after_open;
            continue;
        };
        let title = after_open[..close].trim();
        let after_url = &after_open[close + 2..];
        let Some(end) = after_url.find(')') else {
            break;
        };
        let url = strip_source_url_punctuation(after_url[..end].trim());
        if url.starts_with("http://") || url.starts_with("https://") {
            push_source_unique(
                &mut sources,
                SearchSource {
                    title: (!title.is_empty()).then(|| title.to_owned()),
                    url: url.to_owned(),
                },
            );
        }
        remainder = &after_url[end + 1..];
    }

    for raw in text.split_whitespace() {
        let Some(start) = raw.find("http://").or_else(|| raw.find("https://")) else {
            continue;
        };
        let url = strip_source_url_punctuation(&raw[start..]);
        if url.starts_with("http://") || url.starts_with("https://") {
            push_source_unique(
                &mut sources,
                SearchSource {
                    title: None,
                    url: url.to_owned(),
                },
            );
        }
    }
    sources
}

fn committed_message(event: &Value) -> Option<&Value> {
    (event.get("type").and_then(Value::as_str) == Some("committed_message"))
        .then(|| event.get("message"))
        .flatten()
}

fn committed_assistant_text(event: &Value) -> Option<&str> {
    let message = committed_message(event)?;
    (message.get("role").and_then(Value::as_str) == Some("assistant"))
        .then(|| {
            message
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| message.get("content").and_then(Value::as_str))
        })
        .flatten()
}

fn committed_control_kind(event: &Value) -> Option<&str> {
    committed_message(event)?
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
}

#[cfg(test)]
fn value_search_sources(value: &Value) -> Vec<SearchSource> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item
                        .get("url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    Some(SearchSource {
                        title: item
                            .get("title")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned),
                        url: url.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_search_like_tool_name(tool_name: &str) -> bool {
    let lower = tool_name.to_ascii_lowercase();
    lower.contains("google_web_search")
        || lower.contains("web_search")
        || lower.contains("google search")
        || lower.contains("search")
}

#[cfg(test)]
pub(crate) fn apply_search_stream_event(state: &mut SearchStreamState, event: &Value) {
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    if let Some(thread_id) = event
        .get("threadId")
        .or_else(|| event.get("thread_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        state.thread_id = Some(thread_id.to_owned());
    }
    if let Some(run_id) = event
        .get("runId")
        .or_else(|| event.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        state.run_id = Some(run_id.to_owned());
    }

    match event_type {
        "committed_message" => {
            let Some(message) = event.get("message") else {
                return;
            };
            match message.get("role").and_then(Value::as_str).unwrap_or("") {
                "assistant" => {
                    if let Some(text) = message
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| message.get("content").and_then(Value::as_str))
                    {
                        state.answer.push_str(text);
                    }
                }
                "tool_use" | "tool_result" => apply_search_tool_message(state, message),
                _ => {}
            }
        }
        "tool_use" | "tool_result" => {
            let Some(message) = event.get("message") else {
                return;
            };
            apply_search_tool_message(state, message);
        }
        _ => {}
    }
}

#[cfg(test)]
fn apply_search_tool_message(state: &mut SearchStreamState, message: &Value) {
    let tool_name = message
        .get("tool_name")
        .and_then(Value::as_str)
        .or_else(|| message.get("toolName").and_then(Value::as_str))
        .or_else(|| {
            message
                .get("content")
                .and_then(|content| content.get("rawInput"))
                .and_then(|raw| raw.get("name"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            message
                .get("content")
                .and_then(|content| content.get("title"))
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    let search_metadata = message
        .get("metadata")
        .and_then(|metadata| metadata.get("gemini_search"));
    if is_search_like_tool_name(tool_name) || search_metadata.is_some() {
        state.searched = true;
    }
    let Some(search_metadata) = search_metadata else {
        return;
    };
    let mut sources = value_search_sources(&search_metadata["sources"]);
    if sources.is_empty()
        && let Some(output) = search_metadata.get("output").and_then(Value::as_str)
    {
        sources = extract_search_sources_from_text(output);
    }
    for source in &sources {
        push_source_unique(&mut state.sources, source.clone());
    }
    state.tool_metadata.push(SearchToolMetadata {
        tool_name: tool_name.to_owned(),
        tool_use_id: message
            .get("tool_use_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        sources,
        output: search_metadata
            .get("output")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    });
}

fn apply_gemini_cli_search_event(
    state: &mut SearchStreamState,
    summary: &mut GeminiCliSearchSummary,
    event: &Value,
) {
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "init" => {
            summary.session_id = event
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            summary.model = event
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
        }
        "tool_use" => {
            let tool_name = event.get("tool_name").and_then(Value::as_str).unwrap_or("");
            if is_search_like_tool_name(tool_name) {
                state.searched = true;
                state.tool_metadata.push(SearchToolMetadata {
                    tool_name: tool_name.to_owned(),
                    tool_use_id: event
                        .get("tool_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    sources: Vec::new(),
                    output: None,
                });
            }
        }
        "tool_result" => {
            let tool_id = event
                .get("tool_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let output = event
                .get("output")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if let (Some(existing), Some(output)) = (
                tool_id.as_deref().and_then(|id| {
                    state
                        .tool_metadata
                        .iter_mut()
                        .find(|item| item.tool_use_id.as_deref() == Some(id))
                }),
                output,
            ) {
                existing.output = Some(output);
            }
        }
        "message" => {
            if event.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(content) = event.get("content").and_then(Value::as_str)
            {
                state.answer.push_str(content);
            }
        }
        "result" => {
            summary.status = event
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let stats = event.get("stats").and_then(Value::as_object);
            summary.duration_ms = stats
                .and_then(|stats| stats.get("duration_ms"))
                .and_then(Value::as_u64);
            if let Some(tool_calls) = stats
                .and_then(|stats| stats.get("tool_calls"))
                .and_then(Value::as_u64)
                && tool_calls > 0
            {
                state.searched = true;
            }
        }
        _ => {}
    }
}

fn sanitize_gemini_cli_stderr(stderr: &str) -> String {
    stderr
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("authorization")
                || lower.contains("access_token")
                || lower.contains("refresh_token")
                || lower.contains("credential")
                || lower.contains("api key")
                || lower.contains("apikey")
            {
                "[redacted sensitive stderr line]".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn run_gemini_cli_search(
    query: &str,
    timeout_secs: u64,
) -> Result<SearchCommandOutput, Box<dyn std::error::Error>> {
    let workspace_dir = tool_workspace_dir("search")?;
    let policy = write_gemini_search_policy().await?;
    let run_id = format!("tool-run-{}", Uuid::new_v4());
    let mut command = Command::new("gemini");
    command
        .current_dir(&workspace_dir)
        .kill_on_drop(true)
        .arg("--approval-mode")
        .arg("yolo")
        .arg("--model")
        .arg(TOOL_SEARCH_GEMINI_MODEL)
        .arg("--policy")
        .arg(policy.path())
        .arg("--output-format")
        .arg("stream-json")
        .arg("-p")
        .arg(build_gemini_search_prompt(query))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match tokio::time::timeout(Duration::from_secs(timeout_secs), command.output())
        .await
    {
        Ok(output) => output?,
        Err(_) => {
            return Err(
                format!("timed out after {timeout_secs}s waiting for Gemini CLI search").into(),
            );
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let sanitized = sanitize_gemini_cli_stderr(&stderr);
        return Err(format!(
            "Gemini CLI search failed with status {}{}",
            output.status,
            if sanitized.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", sanitized.trim())
            }
        )
        .into());
    }

    let mut state = SearchStreamState::default();
    let mut summary = GeminiCliSearchSummary::default();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let event: Value = serde_json::from_str(line)
            .map_err(|error| format!("Gemini CLI emitted malformed stream JSON: {error}"))?;
        apply_gemini_cli_search_event(&mut state, &mut summary, &event);
    }

    let answer = state.answer.trim().to_owned();
    if summary
        .status
        .as_deref()
        .is_some_and(|status| status != "success")
    {
        return Err(format!(
            "Gemini CLI search finished with status {}",
            summary.status.as_deref().unwrap_or("unknown")
        )
        .into());
    }
    if !state.searched {
        return Err("Gemini completed without using provider-native search".into());
    }
    if answer.is_empty() {
        return Err("Gemini returned no answer".into());
    }
    if state.sources.is_empty() {
        state.sources = extract_search_sources_from_text(&answer);
    }

    let runtime_thread_id = summary
        .session_id
        .as_deref()
        .map(|session_id| format!("gemini-cli::{session_id}"))
        .unwrap_or_else(|| format!("tool::search::{}", Uuid::new_v4()));
    Ok(SearchCommandOutput {
        ok: true,
        query: query.to_owned(),
        answer,
        sources: state.sources,
        runtime_thread_id,
        run_id,
        model: summary
            .model
            .unwrap_or_else(|| TOOL_SEARCH_GEMINI_MODEL.to_owned()),
        duration_ms: summary.duration_ms,
        searched: state.searched,
        tool_metadata: state.tool_metadata,
    })
}

pub(crate) async fn cmd_tool_search(
    _config_path: &str,
    query_parts: Vec<String>,
    json_output: bool,
    timeout_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let query = query_parts.join(" ").trim().to_owned();
    if query.is_empty() {
        return Err("query cannot be empty".into());
    }
    if timeout_secs == 0 {
        return Err("timeout must be greater than zero".into());
    }

    let output = run_gemini_cli_search(&query, timeout_secs).await?;

    if json_output {
        return print_pretty_json(&serde_json::to_value(output)?);
    }

    println!("{}", output.answer);
    if output.sources.is_empty() {
        println!("\nSources: (none returned by provider; no URLs found in final answer)");
    } else {
        println!("\nSources:");
        for source in output.sources {
            match source.title.as_deref() {
                Some(title) => println!("- {title}: {}", source.url),
                None => println!("- {}", source.url),
            }
        }
    }
    Ok(())
}

pub(crate) async fn cmd_dream_list(
    config_path: &str,
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    limit: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let query = dream_query(from, to, since_hours, Some(limit), None)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, &format!("/api/dreams?{query}")).await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_dream_list(&payload);
    Ok(())
}

pub(crate) async fn cmd_dream_scan(
    config_path: &str,
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    mode: &str,
    limit: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = dream_scan_payload(from, to, since_hours, mode, limit)?;
    let gateway = gateway_endpoint(config_path)?;
    let response =
        post_gateway_json_with_timeout(&gateway, "/api/dreams/scan", &payload, 180).await?;
    if json_output {
        return print_pretty_json(&response);
    }
    if let Some(scan) = response.get("scan") {
        let status = scan
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let topics = scan
            .get("topics_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let spans = scan.get("spans_count").and_then(Value::as_u64).unwrap_or(0);
        println!("Dream scan: {status} ({topics} topics, {spans} spans)");
        if let Some(error) = scan.get("error").and_then(Value::as_str) {
            println!("Extractor note: {error}");
        }
        println!();
    }
    print_dream_list(&response);
    Ok(())
}

pub(crate) async fn cmd_dream_auto(
    config_path: &str,
    state: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let normalized = state.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "on" => {
            put_gateway_json(
                &gateway,
                "/api/settings?merge=true",
                &json!({"dreams": {"enabled": true}}),
            )
            .await?;
        }
        "off" => {
            put_gateway_json(
                &gateway,
                "/api/settings?merge=true",
                &json!({"dreams": {"enabled": false}}),
            )
            .await?;
        }
        "status" => {}
        _ => return Err("dream auto state must be status, on, or off".into()),
    }

    let settings = fetch_gateway_json(&gateway, "/api/settings").await?;
    let dreams = settings.get("dreams").unwrap_or(&Value::Null);
    let enabled = dreams
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let scan_interval_secs = dreams
        .get("scan_interval_secs")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let scan_since_hours = dreams
        .get("scan_since_hours")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    let payload = json!({
        "enabled": enabled,
        "scan_interval_secs": scan_interval_secs,
        "scan_since_hours": scan_since_hours,
    });
    if json_output {
        return print_pretty_json(&payload);
    }

    println!(
        "Dream auto scan: {}",
        if enabled { "enabled" } else { "disabled" }
    );
    println!("Interval: {scan_interval_secs}s");
    println!("Lookback: {scan_since_hours}h");
    Ok(())
}

pub(crate) async fn cmd_dream_show(
    config_path: &str,
    dream_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let encoded = urlencoding::encode(dream_id);
    let payload = fetch_gateway_json(&gateway, &format!("/api/dreams/{encoded}")).await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    if let Some(dream) = payload.get("dream") {
        print_dream_topic(dream);
        return Ok(());
    }
    println!("Dream: (not found)");
    Ok(())
}

fn dream_query(
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    limit: Option<usize>,
    mode: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if since_hours <= 0 {
        return Err("since-hours must be greater than zero".into());
    }
    let mut params = Vec::new();
    if let Some(from) = from.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("from".to_owned(), from.to_owned()));
    } else {
        params.push(("since_hours".to_owned(), since_hours.to_string()));
    }
    if let Some(to) = to.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("to".to_owned(), to.to_owned()));
    }
    if let Some(limit) = limit {
        params.push(("limit".to_owned(), limit.to_string()));
    }
    if let Some(mode) = mode.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("mode".to_owned(), mode.to_owned()));
    }
    Ok(params
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&"))
}

fn dream_scan_payload(
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    mode: &str,
    limit: usize,
) -> Result<Value, Box<dyn std::error::Error>> {
    if since_hours <= 0 {
        return Err("since-hours must be greater than zero".into());
    }
    let mut obj = serde_json::Map::new();
    if let Some(from) = from.map(str::trim).filter(|value| !value.is_empty()) {
        obj.insert("from".to_owned(), Value::String(from.to_owned()));
    } else {
        obj.insert("since_hours".to_owned(), json!(since_hours));
    }
    if let Some(to) = to.map(str::trim).filter(|value| !value.is_empty()) {
        obj.insert("to".to_owned(), Value::String(to.to_owned()));
    }
    obj.insert("mode".to_owned(), Value::String(mode.trim().to_owned()));
    obj.insert("limit".to_owned(), json!(limit));
    Ok(Value::Object(obj))
}

fn print_dream_list(payload: &Value) {
    let dreams = payload
        .get("dreams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if dreams.is_empty() {
        println!("Dreams: (none)");
        return;
    }
    for (index, dream) in dreams.iter().enumerate() {
        if index > 0 {
            println!();
        }
        print_dream_topic(dream);
    }
}

fn print_dream_topic(dream: &Value) {
    let dream_id = dream
        .get("dream_id")
        .or_else(|| dream.get("dreamId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let title = dream
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled Dream");
    println!("{title}");
    if !dream_id.is_empty() {
        println!("  id: {dream_id}");
    }
    let source = dream
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let confidence = dream
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let message_count = dream
        .get("message_count")
        .or_else(|| dream.get("messageCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let span_count = dream
        .get("span_count")
        .or_else(|| dream.get("spanCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_message_at = dream
        .get("last_message_at")
        .or_else(|| dream.get("lastMessageAt"))
        .and_then(Value::as_str)
        .unwrap_or("");
    println!(
        "  {message_count} messages, {span_count} spans, source={source}, confidence={confidence:.2}"
    );
    if !last_message_at.is_empty() {
        println!("  last: {last_message_at}");
    }
    if let Some(summary) = dream
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        println!("  {summary}");
    }
    if let Some(spans) = dream.get("spans").and_then(Value::as_array) {
        for span in spans {
            let thread_id = span
                .get("thread_id")
                .or_else(|| span.get("threadId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let start_seq = span
                .get("start_seq")
                .or_else(|| span.get("startSeq"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let end_seq = span
                .get("end_seq")
                .or_else(|| span.get("endSeq"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let excerpt = span
                .get("excerpt")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            println!("  - {thread_id} #{start_seq}-{end_seq}: {excerpt}");
        }
    }
}

pub(crate) async fn cmd_task_list(
    config_path: &str,
    status: Option<&str>,
    assignee: Option<&str>,
    source_thread: Option<&str>,
    source_task: Option<&str>,
    source_bot: Option<&str>,
    include_done: bool,
    limit: usize,
    offset: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut params = vec![
        ("limit".to_owned(), limit.clamp(1, 200).to_string()),
        ("offset".to_owned(), offset.to_string()),
    ];
    if let Some(status) = status {
        params.push(("status".to_owned(), normalize_task_status(status)?));
    }
    if let Some(assignee) = assignee.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("assignee".to_owned(), assignee.to_owned()));
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
    let workflow_runs_payload = payload
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|task_id| async move {
            fetch_gateway_json(
                &gateway,
                &format!(
                    "/api/tasks/{}/workflow-runs?limit=10",
                    encode_task_id(task_id).ok()?
                ),
            )
            .await
            .ok()
        });
    let workflow_runs_payload = match workflow_runs_payload {
        Some(fetch) => fetch.await,
        None => None,
    };
    let mut output = format_task_progress(&payload, history_payload.as_ref());
    append_task_workflow_runs(&mut output, workflow_runs_payload.as_ref());
    print!("{output}");
    Ok(())
}

pub(crate) async fn cmd_task_create(
    config_path: &str,
    title: Option<String>,
    body: Option<String>,
    assignee: Option<&str>,
    start: bool,
    workspace_dir: Option<String>,
    worktree: bool,
    agent: Option<String>,
    team: Option<String>,
    workflow: Option<String>,
    input: Option<String>,
    input_file: Option<PathBuf>,
    input_json: Option<String>,
    notify: Vec<String>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let executor = task_executor_payload(agent, team, workflow, input, input_file, input_json)?;
    let assignee = if executor.is_some() {
        if assignee
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            return Err("task executor cannot be combined with --assignee".into());
        }
        None
    } else {
        task_create_assignee_payload(assignee)?
    };
    let runtime_agent_id = task_runtime_agent_id_from_assignee(&assignee);
    let start = start || assignee.is_some() || executor.is_some();
    let notification_target = task_notification_target_payload(notify)?;
    let source = task_source_payload_from_env();
    let workspace_dir = workspace_dir
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let gateway = gateway_endpoint(config_path)?;
    let workflow_workspace_dir = executor
        .as_ref()
        .filter(|executor| executor.get("type").and_then(Value::as_str) == Some("workflow"))
        .and(workspace_dir.clone());
    let request = json!({
        "title": title,
        "body": body,
        "assignee": assignee,
        "start": start,
        "workspace_dir": workflow_workspace_dir,
        "executor": executor,
        "runtime": {
            "agent_id": runtime_agent_id,
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
    Ok(())
}

fn task_executor_payload(
    agent: Option<String>,
    team: Option<String>,
    workflow: Option<String>,
    input: Option<String>,
    input_file: Option<PathBuf>,
    input_json: Option<String>,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let agent_id = agent
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let team_id = team
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let workflow_id = workflow
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let executor_count = usize::from(agent_id.is_some())
        + usize::from(team_id.is_some())
        + usize::from(workflow_id.is_some());
    if executor_count > 1 {
        return Err("choose only one task executor: --agent, --team, or --workflow".into());
    }
    if let Some(agent_id) = agent_id {
        return Ok(Some(json!({
            "type": "agent",
            "agentId": agent_id,
        })));
    }
    if let Some(team_id) = team_id {
        return Ok(Some(json!({
            "type": "team",
            "teamId": team_id,
        })));
    }
    let Some(workflow_id) = workflow_id else {
        return Ok(None);
    };
    let input = workflow_task_input_payload(input, input_file, input_json)?;
    Ok(Some(json!({
        "type": "workflow",
        "workflowId": workflow_id,
        "input": input,
    })))
}

fn workflow_task_input_payload(
    input: Option<String>,
    input_file: Option<PathBuf>,
    input_json: Option<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let text_input = trim_optional_cli(input);
    let json_input = trim_optional_cli(input_json);
    let provided_count = usize::from(text_input.is_some())
        + usize::from(input_file.is_some())
        + usize::from(json_input.is_some());
    if provided_count > 1 {
        return Err(
            "choose only one workflow input: --input, --input-file, or --input-json".into(),
        );
    }
    if let Some(text) = text_input {
        return Ok(Value::String(text));
    }
    if let Some(path) = input_file {
        let contents = fs::read_to_string(&path)?;
        return Ok(if contents.trim().is_empty() {
            Value::Null
        } else {
            Value::String(contents)
        });
    }
    if let Some(raw) = json_input {
        return Ok(serde_json::from_str::<Value>(&raw)?);
    }
    Ok(Value::Null)
}

fn task_create_assignee_payload(
    assignee: Option<&str>,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    assignee.map(principal_payload).transpose()
}

fn task_runtime_agent_id_from_assignee(assignee: &Option<Value>) -> Option<String> {
    let assignee = assignee.as_ref()?.as_object()?;
    (assignee.get("kind").and_then(Value::as_str) == Some("agent"))
        .then(|| assignee.get("agent_id").and_then(Value::as_str))
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
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
        return Err("--notify is required for task creation; use --notify current-thread, --notify bot <channel:account_id>, --notify thread <thread_id>, or --notify none".into());
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

pub(crate) async fn cmd_task_claim(
    config_path: &str,
    task_id: &str,
    actor: Option<&str>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let encoded_id = encode_task_id(task_id)?;
    let assignee = actor
        .map(principal_payload)
        .transpose()?
        .unwrap_or_else(cli_actor_payload);
    let assign_path = format!("/api/tasks/{encoded_id}/assign");
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
    task_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let encoded_id = encode_task_id(task_id)?;
    let status_path = format!("/api/tasks/{encoded_id}/status");
    let assign_path = format!("/api/tasks/{encoded_id}/assign");
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

pub(crate) async fn cmd_task_assign(
    config_path: &str,
    task_id: &str,
    principal: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/assign", encode_task_id(task_id)?),
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
    task_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/tasks/{}/assign", encode_task_id(task_id)?),
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

fn encode_task_id(task_id: &str) -> Result<String, Box<dyn std::error::Error>> {
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
            "\nFull thread with tool calls: garyx thread history {thread_id} --limit 200 --json"
        );
    }
    output
}

fn append_task_workflow_runs(output: &mut String, workflow_runs_payload: Option<&Value>) {
    let Some(workflow_runs) = workflow_runs_payload
        .and_then(|payload| payload.get("workflowRuns"))
        .and_then(Value::as_array)
        .filter(|runs| !runs.is_empty())
    else {
        return;
    };
    output.push('\n');
    output.push_str("Workflow Runs:\n");
    for run in workflow_runs {
        let workflow = run.get("workflow").unwrap_or(run);
        let workflow_id = workflow
            .get("workflowRunId")
            .or_else(|| workflow.get("workflowId"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        let status = workflow
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let definition_id = workflow
            .get("workflowDefinitionId")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let definition_version = workflow
            .get("workflowDefinitionVersion")
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned());
        let total_children = workflow
            .get("totalChildren")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let completed_children = workflow
            .get("completedChildren")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let failed_children = workflow
            .get("failedChildren")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let _ = writeln!(
            output,
            "- {workflow_id} [{status}] definition {definition_id}@{definition_version} children {completed_children}/{total_children} failed {failed_children}"
        );
        if let Some(output_text) = workflow
            .get("outputText")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let _ = writeln!(output, "  Output: {output_text}");
        }
        if let Some(error) = workflow
            .get("error")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let _ = writeln!(output, "  Error: {error}");
        }
        let Some(children) = run.get("children").and_then(Value::as_array) else {
            continue;
        };
        for child in children {
            let label = child.get("label").and_then(Value::as_str).unwrap_or("-");
            let child_status = child.get("status").and_then(Value::as_str).unwrap_or("-");
            let thread_id = child.get("threadId").and_then(Value::as_str).unwrap_or("-");
            let phase_title = child
                .get("phaseTitle")
                .and_then(Value::as_str)
                .unwrap_or("-");
            let _ = writeln!(
                output,
                "  - {label} [{child_status}] phase {phase_title} thread {thread_id}"
            );
        }
    }
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
        let updated_at = format_local_thread_timestamp(active_run["updated_at"].as_str());
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
            let updated_at = format_local_thread_timestamp(record["updated_at"].as_str());
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
    if let Some(workspace_dir) = payload["main_endpoint"]["workspace_dir"].as_str()
        && !workspace_dir.trim().is_empty()
    {
        println!("Workspace: {workspace_dir}");
    }
    println!(
        "Workspace mode: {}",
        payload["workspace_mode"].as_str().unwrap_or("local")
    );
    if let Some(binding_key) = payload["main_endpoint"]["binding_key"].as_str()
        && !binding_key.trim().is_empty()
    {
        println!("Binding key: {binding_key}");
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

fn normalize_bot_selector_arg(bot: &str) -> Result<String, Box<dyn std::error::Error>> {
    let bot = bot.trim();
    let Some((channel, account_id)) = bot.split_once(':') else {
        return Err("bot must be `channel:account_id`, e.g. `telegram:main`".into());
    };
    if channel.trim().is_empty() || account_id.trim().is_empty() {
        return Err("bot must be `channel:account_id`, e.g. `telegram:main`".into());
    }
    Ok(format!("{}:{}", channel.trim(), account_id.trim()))
}

fn normalize_thread_id_arg(thread_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() || !is_thread_key(thread_id) {
        return Err("thread must be a canonical thread id like `thread::...`".into());
    }
    Ok(thread_id.to_owned())
}

fn normalize_endpoint_key_arg(endpoint_key: &str) -> Result<String, Box<dyn std::error::Error>> {
    let endpoint_key = endpoint_key.trim();
    let parts = endpoint_key.split("::").collect::<Vec<_>>();
    if parts.len() < 3 || parts.iter().take(3).any(|part| part.trim().is_empty()) {
        return Err(
            "endpoint must be an endpoint key like `channel::account_id::binding_key`".into(),
        );
    }
    Ok(endpoint_key.to_owned())
}

pub(crate) async fn cmd_endpoint_list(
    config_path: &str,
    bot: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bot = match bot {
        Some(bot) => Some(normalize_bot_selector_arg(bot)?),
        None => None,
    };
    let gateway = gateway_endpoint(config_path)?;
    let mut payload = fetch_gateway_json(&gateway, "/api/channel-endpoints").await?;
    if let Some(bot) = bot.as_deref()
        && let Some((channel, account_id)) = bot.split_once(':')
        && let Some(endpoints) = payload.get_mut("endpoints").and_then(Value::as_array_mut)
    {
        endpoints.retain(|endpoint| {
            endpoint["channel"].as_str() == Some(channel)
                && endpoint["account_id"].as_str() == Some(account_id)
        });
    }

    if json {
        return print_pretty_json(&payload);
    }

    let endpoints = payload
        .get("endpoints")
        .and_then(Value::as_array)
        .ok_or("gateway response missing endpoints")?;
    if endpoints.is_empty() {
        println!("No endpoints found.");
        return Ok(());
    }
    for endpoint in endpoints {
        let endpoint_key = endpoint["endpoint_key"].as_str().unwrap_or("-");
        let channel = endpoint["channel"].as_str().unwrap_or("-");
        let account_id = endpoint["account_id"].as_str().unwrap_or("-");
        let kind = endpoint["conversation_kind"].as_str().unwrap_or("unknown");
        let thread_id = endpoint["thread_id"].as_str().unwrap_or("-");
        let label = endpoint["display_label"].as_str().unwrap_or("-");
        println!("{endpoint_key}");
        println!("  Bot: {channel}:{account_id}");
        println!("  Kind: {kind}");
        println!("  Thread: {thread_id}");
        println!("  Label: {label}");
        if let Some(workspace_dir) = endpoint["workspace_dir"].as_str()
            && !workspace_dir.trim().is_empty()
        {
            println!("  Workspace: {workspace_dir}");
        }
    }
    Ok(())
}

fn print_endpoint_binding_result(
    payload: &Value,
    fallback_endpoint: &str,
    action: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        return print_pretty_json(payload);
    }
    if !payload["ok"].as_bool().unwrap_or(false) {
        let message = payload["error"]
            .as_str()
            .or_else(|| payload["reason"].as_str())
            .unwrap_or("endpoint binding failed");
        return Err(message.into());
    }
    println!(
        "Endpoint: {}",
        payload["endpoint_key"]
            .as_str()
            .unwrap_or(fallback_endpoint)
    );
    println!("Action: {action}");
    println!(
        "Current thread: {}",
        payload["thread_id"]
            .as_str()
            .or_else(|| payload["current_thread_id"].as_str())
            .unwrap_or("-")
    );
    if let Some(previous_thread_id) = payload["previous_thread_id"].as_str()
        && !previous_thread_id.trim().is_empty()
    {
        println!("Previous thread: {previous_thread_id}");
    }
    Ok(())
}

pub(crate) async fn cmd_endpoint_bind(
    config_path: &str,
    endpoint: &str,
    thread: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = normalize_endpoint_key_arg(endpoint)?;
    let thread = normalize_thread_id_arg(thread)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        "/api/channel-bindings/bind",
        &json!({
            "endpointKey": endpoint,
            "threadId": thread,
        }),
    )
    .await?;
    print_endpoint_binding_result(&payload, &endpoint, "bind", json)
}

pub(crate) async fn cmd_endpoint_detach(
    config_path: &str,
    endpoint: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = normalize_endpoint_key_arg(endpoint)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(
        &gateway,
        "/api/channel-bindings/detach",
        &json!({
            "endpointKey": endpoint,
        }),
    )
    .await?;
    print_endpoint_binding_result(&payload, &endpoint, "detach", json)
}

fn format_local_thread_timestamp(value: Option<&str>) -> String {
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
        "gpt" => "GPT",
        "garyx_native" => "GPT",
        "claude_code" => "Claude",
        _ => "-",
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct DoctorIssue {
    code: String,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

impl DoctorIssue {
    fn new(code: impl Into<String>, message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct DoctorCheck {
    ok: bool,
    detail: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    issues: Vec<DoctorIssue>,
}

impl DoctorCheck {
    fn new(ok: bool, detail: impl Into<String>) -> Self {
        Self {
            ok,
            detail: detail.into(),
            issues: Vec::new(),
        }
    }

    fn with_issues(ok: bool, detail: impl Into<String>, issues: Vec<DoctorIssue>) -> Self {
        Self {
            ok,
            detail: detail.into(),
            issues,
        }
    }
}

fn json_path_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        format!(".{key}")
    } else {
        format!(
            "[{}]",
            serde_json::to_string(key).unwrap_or_else(|_| "\"<invalid>\"".to_owned())
        )
    }
}

fn channel_account_config_path(channel: &str, account: &str) -> String {
    format!(
        "$.channels{}.accounts{}.config",
        json_path_key(channel),
        json_path_key(account)
    )
}

fn push_channel_account_issue(
    issues: &mut Vec<DoctorIssue>,
    code: &'static str,
    channel: &str,
    account: &str,
    message: impl Into<String>,
) {
    issues.push(DoctorIssue::new(
        code,
        message,
        Some(channel_account_config_path(channel, account)),
    ));
}

fn validate_builtin_channel_account(
    channel: &str,
    account_id: &str,
    entry: &PluginAccountEntry,
    issues: &mut Vec<DoctorIssue>,
) {
    match channel {
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM => match telegram_account_from_plugin_entry(entry) {
            Ok(account) => {
                if account.token.trim().is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!("channel `{channel}` account `{account_id}` is missing token"),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        BUILTIN_CHANNEL_PLUGIN_DISCORD => match discord_account_from_plugin_entry(entry) {
            Ok(account) => {
                if account.token.trim().is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!("channel `{channel}` account `{account_id}` is missing token"),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        BUILTIN_CHANNEL_PLUGIN_FEISHU => match feishu_account_from_plugin_entry(entry) {
            Ok(account) => {
                let mut missing = Vec::new();
                if account.app_id.trim().is_empty() {
                    missing.push("app_id");
                }
                if account.app_secret.trim().is_empty() {
                    missing.push("app_secret");
                }
                if !missing.is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!(
                            "channel `{channel}` account `{account_id}` is missing {}",
                            missing.join(", ")
                        ),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        BUILTIN_CHANNEL_PLUGIN_WEIXIN => match weixin_account_from_plugin_entry(entry) {
            Ok(account) => {
                if account.token.trim().is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!("channel `{channel}` account `{account_id}` is missing token"),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        _ => {}
    }
}

fn validate_plugin_schema_account(
    channel: &str,
    account_id: &str,
    entry: &PluginAccountEntry,
    schema: &Value,
    issues: &mut Vec<DoctorIssue>,
) {
    let Some(form_state) = entry.config.as_object() else {
        push_channel_account_issue(
            issues,
            "CONFIG_CHANNEL_ACCOUNT_CONFIG_TYPE",
            channel,
            account_id,
            format!("channel `{channel}` account `{account_id}` config must be an object"),
        );
        return;
    };

    let missing: Vec<String> = plugin_required_fields(schema)
        .into_iter()
        .filter(|key| value_is_missing(form_state.get(key)))
        .collect();
    if !missing.is_empty() {
        push_channel_account_issue(
            issues,
            "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
            channel,
            account_id,
            format!(
                "channel `{channel}` account `{account_id}` is missing {}",
                missing.join(", ")
            ),
        );
    }
}

fn validate_channel_account_configs(
    cfg: &GaryxConfig,
    plugin_schemas: &HashMap<String, Value>,
) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    for (channel_id, plugin_cfg) in &cfg.channels.plugins {
        for (account_id, entry) in &plugin_cfg.accounts {
            if entry.config.is_null() {
                push_channel_account_issue(
                    &mut issues,
                    "CONFIG_CHANNEL_ACCOUNT_CONFIG_NULL",
                    channel_id,
                    account_id,
                    format!(
                        "channel `{channel_id}` account `{account_id}` has null config; re-run channel setup for this account or remove the stale account"
                    ),
                );
                continue;
            }

            validate_builtin_channel_account(channel_id, account_id, entry, &mut issues);

            if !matches!(
                channel_id.as_str(),
                BUILTIN_CHANNEL_PLUGIN_TELEGRAM
                    | BUILTIN_CHANNEL_PLUGIN_DISCORD
                    | BUILTIN_CHANNEL_PLUGIN_FEISHU
                    | BUILTIN_CHANNEL_PLUGIN_WEIXIN
            ) && let Some(schema) = plugin_schemas.get(channel_id)
            {
                validate_plugin_schema_account(channel_id, account_id, entry, schema, &mut issues);
            }
        }
    }
    issues
}

fn discover_installed_plugin_schemas() -> Result<(HashMap<String, Value>, Vec<DoctorIssue>), String>
{
    let outcome = ManifestDiscoverer::new(crate::channel_plugin_host::plugin_root_paths(
        &GaryxConfig::default(),
    ))
    .discover()
    .map_err(|err| err.to_string())?;

    let schemas = outcome
        .plugins
        .into_iter()
        .map(|manifest| (manifest.plugin.id, manifest.schema))
        .collect();
    let issues = outcome
        .errors
        .into_iter()
        .map(|err| DoctorIssue::new("PLUGIN_MANIFEST_INVALID", err.to_string(), None))
        .collect();
    Ok((schemas, issues))
}

fn print_doctor_issues(issues: &[DoctorIssue]) {
    for issue in issues {
        if let Some(path) = &issue.path {
            println!("  - [{}] {} ({path})", issue.code, issue.message);
        } else {
            println!("  - [{}] {}", issue.code, issue.message);
        }
    }
}

fn print_config_validation_issues(issues: &[DoctorIssue]) {
    for issue in issues {
        if let Some(path) = &issue.path {
            eprintln!("[error][{}] {} ({path})", issue.code, issue.message);
        } else {
            eprintln!("[error][{}] {}", issue.code, issue.message);
        }
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
    let loaded = load_config_or_default(&config_path_display, ConfigRuntimeOverrides::default());
    let config_load_check = match &loaded {
        Ok(loaded) => {
            print_diagnostics(&loaded.diagnostics);
            DoctorCheck::new(true, "config loads successfully")
        }
        Err(err) => {
            print_errors(&err.diagnostics);
            DoctorCheck::new(false, err.to_string())
        }
    };

    let (plugin_schemas, plugin_manifest_check) = match discover_installed_plugin_schemas() {
        Ok((schemas, issues)) => {
            let detail = if issues.is_empty() {
                format!("{} installed plugin manifest(s) loaded", schemas.len())
            } else {
                format!("{} plugin manifest issue(s)", issues.len())
            };
            (
                schemas,
                DoctorCheck::with_issues(issues.is_empty(), detail, issues),
            )
        }
        Err(err) => (
            HashMap::new(),
            DoctorCheck::with_issues(
                false,
                "failed to discover installed plugin manifests",
                vec![DoctorIssue::new("PLUGIN_MANIFEST_DISCOVERY", err, None)],
            ),
        ),
    };

    let account_issues = loaded
        .as_ref()
        .map(|loaded| validate_channel_account_configs(&loaded.config, &plugin_schemas))
        .unwrap_or_default();
    let config_accounts_check = if loaded.is_ok() {
        let detail = if account_issues.is_empty() {
            "channel account configs look valid".to_owned()
        } else {
            format!("{} invalid channel account config(s)", account_issues.len())
        };
        DoctorCheck::with_issues(account_issues.is_empty(), detail, account_issues)
    } else {
        DoctorCheck::new(false, "skipped because config did not load")
    };

    let checks = vec![
        (
            "config_file",
            DoctorCheck::new(config_exists, config_path_display.clone()),
        ),
        ("config_load", config_load_check),
        ("config_accounts", config_accounts_check),
        ("plugin_manifests", plugin_manifest_check),
        (
            "claude_binary",
            DoctorCheck::new(claude_available, "claude"),
        ),
        ("codex_binary", DoctorCheck::new(codex_available, "codex")),
    ];

    if json {
        let obj: serde_json::Value = checks
            .iter()
            .map(|(name, check)| (name.to_string(), serde_json::to_value(check).unwrap()))
            .collect::<serde_json::Map<String, serde_json::Value>>()
            .into();
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        for (name, check) in &checks {
            let mark = if check.ok {
                "ok"
            } else if *name == "config_file" || name.ends_with("_binary") {
                "MISSING"
            } else {
                "FAILED"
            };
            println!("[{}] {} ({})", mark, name, check.detail);
            print_doctor_issues(&check.issues);
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
            workspace_mode: None,
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
    steps.push("garyx status".to_owned());
    steps.push("garyx config show".to_owned());
    steps.push("garyx doctor".to_owned());
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

        // ---- Channel binding ----
        // The api.* account auto-created above lets programs talk to gateway,
        // but a human needs at least one user-facing channel (telegram /
        // discord / feishu / weixin / subprocess plugin) to actually chat with gary.
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
    notify_gateway_reload_quiet(&config_path).await;
    let gateway_running = gateway_is_reachable(&config_path).await;

    let summary = OnboardSummary {
        ok: true,
        config_path: config_path.display().to_string(),
        created_config,
        api_account: api_account.clone(),
        api_account_created,
        search_api_key_configured: !cfg.gateway.search.api_key.trim().is_empty(),
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

    let should_run_gateway = if gateway_running {
        if options.run_gateway && !options.json {
            println!("Gateway is already running.");
        }
        false
    } else if options.run_gateway {
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
    pub workspace_mode: Option<String>,
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

fn normalize_channel_workspace_mode(
    value: Option<String>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(value) = trim_opt(value) else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "local" => Ok(Some("local".to_owned())),
        "worktree" => Ok(Some("worktree".to_owned())),
        _ => Err(format!("invalid workspace mode `{value}`; use `local` or `worktree`").into()),
    }
}

fn discover_plugin_manifest(
    channel: &str,
) -> Result<Option<PluginManifest>, Box<dyn std::error::Error>> {
    let resolved = canonical_channel_id(channel);
    if matches!(
        resolved.as_str(),
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM
            | BUILTIN_CHANNEL_PLUGIN_DISCORD
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
        && prompt_plugin_auth_mode(manifest)?
    {
        let mut values = run_plugin_auth_flow(manifest, Value::Object(form_state.clone())).await?;
        if overrides.account.is_none()
            && let Some(account_id) = plugin_suggested_account_id(&values)
        {
            overrides.account = Some(account_id);
        }
        strip_plugin_identity_hints(&mut values, &manifest.schema);
        for (key, value) in values {
            set_plugin_form_value(&mut overrides, &key, value);
        }
        form_state = plugin_form_state_from_overrides(&overrides);
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
            && let Some(value) = prompt_plugin_schema_value(field_name, field_schema, true)?
        {
            set_plugin_form_value(&mut overrides, field_name, value);
            form_state = plugin_form_state_from_overrides(&overrides);
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
        "discord" => {
            if overrides.token.is_none() {
                overrides.token = Some(prompt_secret("Discord Bot Token")?);
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
        overrides.workspace_mode,
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
    workspace_mode: Option<String>,
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
        workspace_mode: trim_opt(workspace_mode),
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
            if overrides.account.is_none()
                && let Some(account_id) = plugin_suggested_account_id(&values)
            {
                overrides.account = Some(account_id);
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
                    let domain = overrides
                        .domain
                        .as_deref()
                        .and_then(parse_feishu_domain)
                        .unwrap_or_default();
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
        overrides.workspace_mode,
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
            if let Some(value) = default
                && (!value.is_empty() || allow_empty)
            {
                return Ok(value.to_owned());
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
        ProviderType::Traex => "Traex",
        ProviderType::GeminiCli => "Gemini",
        ProviderType::Gpt => "GPT",
        ProviderType::ClaudeLlm => "Claude",
        ProviderType::GeminiLlm => "Gemini",
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
    workspace_mode: Option<String>,
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
    let workspace_mode = normalize_channel_workspace_mode(workspace_mode)?;
    match channel {
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM => {
            let Some(token) = token else {
                return Err("missing telegram token".into());
            };
            let mut entry = telegram_account_to_plugin_entry(&TelegramAccount {
                token,
                enabled: true,
                name,
                agent_id: agent_id.clone(),
                workspace_dir,
                owner_target: None,
                groups: Default::default(),
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_TELEGRAM)
                .accounts
                .insert(account.to_owned(), entry);
        }
        BUILTIN_CHANNEL_PLUGIN_DISCORD => {
            let Some(token) = token else {
                return Err("missing discord token".into());
            };
            let mut entry = discord_account_to_plugin_entry(&DiscordAccount {
                token,
                enabled: true,
                name,
                agent_id: agent_id.clone(),
                workspace_dir,
                owner_target: None,
                require_mention: true,
                api_base: "https://discord.com/api/v10".to_owned(),
                gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_owned(),
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_DISCORD)
                .accounts
                .insert(account.to_owned(), entry);
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
            let mut entry = feishu_account_to_plugin_entry(&FeishuAccount {
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
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_FEISHU)
                .accounts
                .insert(account.to_owned(), entry);
        }
        BUILTIN_CHANNEL_PLUGIN_WEIXIN => {
            let Some(token) = token else {
                return Err("missing weixin token".into());
            };
            let mut entry = weixin_account_to_plugin_entry(&WeixinAccount {
                token,
                uin: uin.unwrap_or_default(),
                enabled: true,
                base_url: base_url.unwrap_or_else(|| "https://ilinkai.weixin.qq.com".to_owned()),
                name,
                agent_id: agent_id.clone(),
                workspace_dir,
                streaming_update: true,
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_WEIXIN)
                .accounts
                .insert(account.to_owned(), entry);
        }
        "api" => {
            cfg.channels.api.accounts.insert(
                account.to_owned(),
                ApiAccount {
                    enabled: true,
                    name,
                    agent_id: agent_id.clone(),
                    workspace_dir,
                    workspace_mode,
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
                workspace_mode: workspace_mode.clone(),
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
                        workspace_mode,
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

/// True when the environment looks like a graphical session a real
/// browser could open into. Treats an empty value the same as unset —
/// that's how X11 and xdg-open's fallback chain read it.
#[cfg(any(target_os = "linux", test))]
fn gui_session_available(
    display: Option<&std::ffi::OsStr>,
    wayland: Option<&std::ffi::OsStr>,
) -> bool {
    let set = |v: Option<&std::ffi::OsStr>| v.is_some_and(|s| !s.is_empty());
    set(display) || set(wayland)
}

/// Best-effort: shell out to the platform's default-browser opener.
/// Silent on failure — the URL is already printed above so the user
/// can always copy-paste. All stdio is detached so an opener that
/// resolves to a terminal browser (xdg-open → w3m via `$BROWSER` or a
/// misconfigured desktop) can never hijack the live auth-flow TTY.
fn open_url_in_browser(url: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    #[cfg(target_os = "macos")]
    let cmd = Command::new("open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    #[cfg(target_os = "linux")]
    let cmd = if !gui_session_available(
        std::env::var_os("DISPLAY").as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
    ) {
        // Headless box: xdg-open degrades to terminal browsers (w3m),
        // which take over the terminal mid-auth-flow. The URL is
        // already printed above, so skipping auto-open loses nothing.
        return Ok(());
    } else {
        Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    #[cfg(target_os = "windows")]
    // NB: do NOT shell out through `cmd /C start` — that reinterprets
    // the URL on cmd.exe's command line, giving shell metacharacters
    // like `&`/`|`/`^` (legal inside a URL) attack surface. Use
    // `rundll32 url.dll,FileProtocolHandler` which takes the URL as
    // a direct argv[2] with no shell involvement.
    let cmd = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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
    workspace_mode: Option<String>,
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
    let selected_workspace_mode = trim_opt(workspace_mode).or_else(|| {
        reauthorize_entry
            .as_ref()
            .and_then(|entry| entry.workspace_mode.clone())
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
                selected_workspace_mode.clone(),
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
                selected_workspace_mode.clone(),
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
                workspace_mode: selected_workspace_mode.clone(),
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
                overrides.workspace_mode,
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
            all_lines.retain(|line| line.contains(p));
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
    bot: Option<&str>,
    image: Option<&Path>,
    file: Option<&Path>,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bot = resolve_cli_message_bot(bot)?;
    if image.is_some() && file.is_some() {
        return Err("message supports at most one attachment: choose --image or --file".into());
    }
    let image_path = image.map(|path| {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string()
    });
    let file_path = file.map(|path| {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string()
    });
    if text.trim().is_empty() && image_path.is_none() && file_path.is_none() {
        return Err("message text, --image, or --file is required".into());
    }

    let gateway = gateway_endpoint(config_path)?;
    let url = format!("{}/api/send", gateway.base_url);

    let mut body = serde_json::json!({
        "bot": bot,
        "text": text,
    });
    if let Some(image_path) = image_path {
        body["image"] = serde_json::Value::String(image_path);
    }
    if let Some(file_path) = file_path {
        body["file"] = serde_json::Value::String(file_path);
    }

    let client = reqwest::Client::new();
    let resp = gateway_request(client.post(&url), &gateway)
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

fn resolve_cli_message_bot(bot: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(bot) = bot.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(bot.to_owned());
    }
    if let Ok(bot) = std::env::var("GARYX_BOT") {
        let bot = bot.trim();
        if !bot.is_empty() {
            return Ok(bot.to_owned());
        }
    }
    let channel = std::env::var("GARYX_CHANNEL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let account_id = std::env::var("GARYX_ACCOUNT_ID")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let (Some(channel), Some(account_id)) = (channel, account_id) {
        return Ok(format!("{channel}:{account_id}"));
    }
    Err("bot is required: pass --bot channel:account_id or set GARYX_BOT/GARYX_CHANNEL+GARYX_ACCOUNT_ID".into())
}
