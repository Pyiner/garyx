#![allow(clippy::too_many_arguments)]

use crate::cli::AutomationScheduleArgs;
use crate::config_support::{
    default_config_path_buf, load_config_or_default, prepare_config_path_for_io_buf,
    print_diagnostics, print_errors,
};
use crate::runtime_assembler::{RuntimeAssembler, RuntimeAssembly};
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
    AgentProviderConfig, ApiAccount, AutomationScheduleView, BUILTIN_CHANNEL_PLUGIN_DISCORD,
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
    default_custom_agents_state_path, default_log_file_path, default_session_data_dir,
    gary_home_dir,
};
use garyx_models::provider::{
    AgentRunRequest, ProviderMessage, StreamEvent, default_claude_cli_mode,
};
use garyx_models::{CustomAgentProfile, ProviderType, builtin_provider_agent_profiles};
use garyx_router::{command_catalog_for_config, is_thread_key, reserved_command_names};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tar::Archive;
use tokio::process::Command;
use uuid::Uuid;

mod agent;
mod automation;
mod bot;
mod channels;
mod config;
mod db;
mod doctor;
mod gateway;
mod gateway_client;
mod logs;
mod message;
mod onboard;
mod provider;
mod self_update;
mod shared;
mod shortcuts;
mod task;
#[cfg(test)]
mod test_support;
mod thread;
mod tool;
mod workflow;

pub(crate) use agent::*;
pub(crate) use automation::*;
pub(crate) use bot::*;
pub(crate) use channels::*;
pub(crate) use config::*;
pub(crate) use db::*;
pub(crate) use doctor::*;
pub(crate) use gateway::*;
use gateway_client::*;
pub(crate) use gateway_client::{GatewayCliError, GatewayErrorKind};
pub(crate) use logs::*;
pub(crate) use message::*;
pub(crate) use onboard::*;
pub(crate) use provider::*;
pub(crate) use self_update::*;
use shared::*;
pub(crate) use shortcuts::*;
pub(crate) use task::*;
pub(crate) use thread::*;
pub(crate) use tool::*;
pub(crate) use workflow::*;

#[derive(Debug, Clone)]
pub(crate) struct OnboardCommandOptions {
    pub force: bool,
    pub json: bool,
    pub api_account: String,
    pub run_gateway: bool,
    pub port_override: Option<u16>,
    pub host_override: Option<String>,
    pub no_channels: bool,
}

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
