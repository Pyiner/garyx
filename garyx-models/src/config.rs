use std::collections::HashMap;

use serde::de::Error as DeError;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// Reply-to mode for controlling how the bot uses reply threading.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplyToMode {
    Off,
    #[default]
    First,
    All,
}

/// Feishu domain.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeishuDomain {
    #[default]
    Feishu,
    Lark,
}

/// Feishu topic session mode.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TopicSessionMode {
    #[default]
    Disabled,
    Enabled,
}

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

/// Serde helper: returns `true` for use with `#[serde(default = "...")]`.
pub fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_search_model")]
    pub model: String,
}

fn default_search_model() -> String {
    "gemini-3-flash-preview".to_owned()
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: default_search_model(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConversationIndexConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_conversation_index_model")]
    pub model: String,
    #[serde(default = "default_conversation_index_base_url")]
    pub base_url: String,
}

fn default_conversation_index_model() -> String {
    "text-embedding-3-small".to_owned()
}

fn default_conversation_index_base_url() -> String {
    "https://api.openai.com/v1".to_owned()
}

impl Default for ConversationIndexConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
            model: default_conversation_index_model(),
            base_url: default_conversation_index_base_url(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GatewayConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub public_url: String,
    #[serde(default)]
    pub auth_token: String,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub conversation_index: ConversationIndexConfig,
    #[serde(default)]
    pub auto_update: GatewayAutoUpdateConfig,
}

fn default_port() -> u16 {
    31337
}
fn default_host() -> String {
    "0.0.0.0".to_owned()
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            public_url: String::new(),
            auth_token: String::new(),
            search: SearchConfig::default(),
            conversation_index: ConversationIndexConfig::default(),
            auto_update: GatewayAutoUpdateConfig::default(),
        }
    }
}

/// Configuration for the gateway-binary self-updater (separate from
/// `PluginsConfig` which governs plugin auto-update). The gateway
/// loop checks GitHub Releases periodically; when a strictly higher
/// version is available it waits for a "no in-flight stream" window,
/// downloads the release, verifies the sha256, codesigns on macOS,
/// atomically swaps the binary into the install path, then SIGTERM's
/// itself so the OS supervisor (launchd / systemd) restarts on the
/// new binary.
///
/// All durations are seconds because that's how the on-disk JSON
/// schema stores them.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GatewayAutoUpdateConfig {
    /// Master switch. When `false` the loop is never spawned and no
    /// release check fires. Default `true` for parity with the
    /// desktop client's auto-update.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Seconds between recurring checks. Default 6h. Lower bound
    /// enforced in `garyx_auto_update::MIN_INTERVAL_SECS` (operator
    /// can shorten for testing but not below the floor — protects
    /// the GitHub API quota).
    #[serde(default = "default_gateway_auto_update_interval_secs")]
    pub check_interval_secs: u64,
    /// How many consecutive seconds the gateway must observe zero
    /// active streams across all subprocess plugins before the
    /// stream-idle gate releases and lets the swap proceed.
    #[serde(default = "default_stream_idle_required_secs")]
    pub stream_idle_required_secs: u64,
    /// How often the stream-idle gate polls
    /// `total_active_streams()`. Lower = faster reaction at the cost
    /// of more lock contention.
    #[serde(default = "default_stream_idle_poll_interval_secs")]
    pub stream_idle_poll_interval_secs: u64,
    /// Wall-clock cap on how long a single tick is allowed to spin
    /// inside the stream-idle gate before giving up. Defaults to 24h
    /// — the loop is forgiving by design; the next tick retries.
    #[serde(default = "default_stream_idle_max_wait_secs")]
    pub stream_idle_max_wait_secs: u64,
    /// GitHub `owner/repo` that hosts the gateway release archives.
    /// Override for testing against a fork.
    #[serde(default = "default_github_repo")]
    pub github_repo: String,
}

fn default_gateway_auto_update_interval_secs() -> u64 {
    21_600
}
fn default_stream_idle_required_secs() -> u64 {
    60
}
fn default_stream_idle_poll_interval_secs() -> u64 {
    5
}
fn default_stream_idle_max_wait_secs() -> u64 {
    24 * 60 * 60
}
fn default_github_repo() -> String {
    "Pyiner/garyx".to_owned()
}

impl Default for GatewayAutoUpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_secs: default_gateway_auto_update_interval_secs(),
            stream_idle_required_secs: default_stream_idle_required_secs(),
            stream_idle_poll_interval_secs: default_stream_idle_poll_interval_secs(),
            stream_idle_max_wait_secs: default_stream_idle_max_wait_secs(),
            github_repo: default_github_repo(),
        }
    }
}

/// Provider runtime configuration used by concrete provider instances.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AgentProviderConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider_id: String,

    #[serde(default = "default_provider_type")]
    pub provider_type: String,

    // Shared settings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<i64>,
    #[serde(default)]
    pub timeout_seconds: f64,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    // Claude-specific
    #[serde(default = "crate::provider::default_claude_cli_mode")]
    pub claude_cli_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub claude_cli_path: String,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    #[serde(default = "default_mcp_base_url")]
    pub mcp_base_url: String,

    // Codex app-server specific
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_reasoning_effort: String,
    #[serde(default)]
    pub model_service_tier: String,
    #[serde(default)]
    pub experimental_api: bool,

    // Gemini CLI specific
    #[serde(default)]
    pub gemini_bin: String,
    #[serde(default = "default_gemini_approval_mode")]
    pub approval_mode: String,

    // Native GPT model backend specific
    #[serde(default = "default_garyx_native_auth_source")]
    pub auth_source: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub codex_home: String,
    #[serde(default = "default_garyx_native_max_tool_iterations")]
    pub max_tool_iterations: u32,
    #[serde(default = "default_native_request_timeout")]
    pub request_timeout_seconds: f64,
}

fn default_provider_type() -> String {
    "claude_code".to_owned()
}
pub fn default_permission_mode() -> String {
    "bypassPermissions".to_owned()
}
pub fn default_mcp_base_url() -> String {
    "http://127.0.0.1:31337".to_owned()
}
pub fn default_gemini_approval_mode() -> String {
    "yolo".to_owned()
}
pub fn default_garyx_native_auth_source() -> String {
    "codex".to_owned()
}
pub fn default_garyx_native_max_tool_iterations() -> u32 {
    32
}
pub fn default_native_request_timeout() -> f64 {
    300.0
}

impl Default for AgentProviderConfig {
    fn default() -> Self {
        Self {
            provider_id: String::new(),
            provider_type: default_provider_type(),
            workspace_dir: None,
            default_model: String::new(),
            max_turns: None,
            timeout_seconds: 0.0,
            env: HashMap::new(),
            claude_cli_mode: crate::provider::default_claude_cli_mode(),
            claude_cli_path: String::new(),
            permission_mode: default_permission_mode(),
            mcp_base_url: default_mcp_base_url(),
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            experimental_api: false,
            gemini_bin: String::new(),
            approval_mode: default_gemini_approval_mode(),
            auth_source: default_garyx_native_auth_source(),
            base_url: String::new(),
            codex_home: String::new(),
            max_tool_iterations: default_garyx_native_max_tool_iterations(),
            request_timeout_seconds: default_native_request_timeout(),
        }
    }
}

fn default_channel_agent_id() -> String {
    "claude".to_owned()
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnerTargetConfig {
    #[serde(default)]
    pub target_type: String,
    #[serde(default)]
    pub target_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TelegramTopicConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_mention: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

impl Default for TelegramTopicConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            require_mention: None,
            allow_from: None,
            system_prompt: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TelegramGroupConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub require_mention: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub topics: HashMap<String, TelegramTopicConfig>,
}

impl Default for TelegramGroupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            require_mention: true,
            allow_from: None,
            system_prompt: None,
            topics: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TelegramAccount {
    pub token: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "default_channel_agent_id")]
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_target: Option<OwnerTargetConfig>,
    #[serde(default)]
    pub groups: HashMap<String, TelegramGroupConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TelegramConfig {
    #[serde(default)]
    pub accounts: HashMap<String, TelegramAccount>,
}

fn default_discord_api_base() -> String {
    "https://discord.com/api/v10".to_owned()
}

fn default_discord_gateway_url() -> String {
    "wss://gateway.discord.gg/?v=10&encoding=json".to_owned()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiscordAccount {
    pub token: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "default_channel_agent_id")]
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_target: Option<OwnerTargetConfig>,
    #[serde(default = "default_true")]
    pub require_mention: bool,
    #[serde(default = "default_discord_api_base")]
    pub api_base: String,
    #[serde(default = "default_discord_gateway_url")]
    pub gateway_url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DiscordConfig {
    #[serde(default)]
    pub accounts: HashMap<String, DiscordAccount>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FeishuAccount {
    pub app_id: String,
    pub app_secret: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub domain: FeishuDomain,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "default_channel_agent_id")]
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_target: Option<OwnerTargetConfig>,
    #[serde(default = "default_true")]
    pub require_mention: bool,
    #[serde(default)]
    pub topic_session_mode: TopicSessionMode,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FeishuConfig {
    #[serde(default)]
    pub accounts: HashMap<String, FeishuAccount>,
}

fn default_weixin_base_url() -> String {
    "https://ilinkai.weixin.qq.com".to_owned()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WeixinAccount {
    pub token: String,
    #[serde(default)]
    pub uin: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_weixin_base_url")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "default_channel_agent_id")]
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    /// Enable in-place message updates for streamed assistant text.
    /// When false, fall back to the legacy one-FINISH-per-flush path.
    #[serde(default = "default_true")]
    pub streaming_update: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct WeixinConfig {
    #[serde(default)]
    pub accounts: HashMap<String, WeixinAccount>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApiAccount {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "default_channel_agent_id")]
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_mode: Option<String>,
}

impl Default for ApiAccount {
    fn default() -> Self {
        Self {
            enabled: true,
            name: None,
            agent_id: default_channel_agent_id(),
            workspace_dir: None,
            workspace_mode: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ApiConfig {
    #[serde(default)]
    pub accounts: HashMap<String, ApiAccount>,
}

/// §9.3: generic plugin-owned channel accounts.
///
/// One entry per plugin (subprocess or built-in). The `config` payload is
/// validated by the plugin against its JSON Schema when the settings are
/// applied, and otherwise forwarded verbatim — the gateway never inspects
/// it. Built-in channels (telegram / feishu / weixin) also live in this
/// shape; there are no separate top-level built-in config buckets.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PluginChannelConfig {
    #[serde(default)]
    pub accounts: HashMap<String, PluginAccountEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginAccountEntry {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_mode: Option<String>,
    /// Opaque JSON validated by the plugin's JSON Schema on save. The
    /// gateway does not introspect any field inside it.
    #[serde(default)]
    pub config: Value,
}

impl Default for PluginAccountEntry {
    fn default() -> Self {
        Self {
            enabled: true,
            name: None,
            agent_id: None,
            workspace_dir: None,
            workspace_mode: None,
            config: Value::Null,
        }
    }
}

pub const BUILTIN_CHANNEL_PLUGIN_TELEGRAM: &str = "telegram";
pub const BUILTIN_CHANNEL_PLUGIN_DISCORD: &str = "discord";
pub const BUILTIN_CHANNEL_PLUGIN_FEISHU: &str = "feishu";
pub const BUILTIN_CHANNEL_PLUGIN_WEIXIN: &str = "weixin";

#[derive(Debug, Clone, Default)]
pub struct ChannelsConfig {
    pub api: ApiConfig,
    /// Generic channel configs keyed by channel id. This stores both
    /// built-in channels (`telegram`, `feishu`, `weixin`) and external
    /// subprocess-backed channel ids. The user-facing JSON shape is
    /// flattened to `channels.<channel_id>`; the internal `plugins`
    /// field remains for backwards-compatible runtime code.
    pub plugins: HashMap<String, PluginChannelConfig>,
}

impl Serialize for ChannelsConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1 + self.plugins.len()))?;
        map.serialize_entry("api", &self.api)?;
        for (channel_id, channel_cfg) in &self.plugins {
            map.serialize_entry(channel_id, channel_cfg)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ChannelsConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Some(mut root) = value.as_object().cloned() else {
            return Err(D::Error::custom("channels must be an object"));
        };

        let api = match root.remove("api") {
            Some(value) => serde_json::from_value(value).map_err(D::Error::custom)?,
            None => ApiConfig::default(),
        };

        let mut plugins = HashMap::new();

        if let Some(legacy_plugins) = root.remove("plugins") {
            let Some(legacy_plugins) = legacy_plugins.as_object() else {
                return Err(D::Error::custom("channels.plugins must be an object"));
            };
            for (channel_id, channel_cfg) in legacy_plugins {
                plugins.insert(
                    channel_id.clone(),
                    serde_json::from_value(channel_cfg.clone()).map_err(D::Error::custom)?,
                );
            }
        }

        for (channel_id, channel_cfg) in root {
            plugins.insert(
                channel_id,
                serde_json::from_value(channel_cfg).map_err(D::Error::custom)?,
            );
        }

        Ok(Self { api, plugins })
    }
}

impl ChannelsConfig {
    pub fn plugin_channel(&self, plugin_id: &str) -> Option<&PluginChannelConfig> {
        self.plugins.get(plugin_id)
    }

    pub fn plugin_channel_mut(&mut self, plugin_id: &str) -> &mut PluginChannelConfig {
        self.plugins.entry(plugin_id.to_owned()).or_default()
    }

    pub fn resolved_telegram_config(&self) -> Result<TelegramConfig, serde_json::Error> {
        Ok(TelegramConfig {
            accounts: resolve_builtin_accounts(
                self.plugin_channel(BUILTIN_CHANNEL_PLUGIN_TELEGRAM),
                telegram_account_from_plugin_entry,
            )?,
        })
    }

    pub fn resolved_discord_config(&self) -> Result<DiscordConfig, serde_json::Error> {
        Ok(DiscordConfig {
            accounts: resolve_builtin_accounts(
                self.plugin_channel(BUILTIN_CHANNEL_PLUGIN_DISCORD),
                discord_account_from_plugin_entry,
            )?,
        })
    }

    pub fn resolved_feishu_config(&self) -> Result<FeishuConfig, serde_json::Error> {
        Ok(FeishuConfig {
            accounts: resolve_builtin_accounts(
                self.plugin_channel(BUILTIN_CHANNEL_PLUGIN_FEISHU),
                feishu_account_from_plugin_entry,
            )?,
        })
    }

    pub fn resolved_weixin_config(&self) -> Result<WeixinConfig, serde_json::Error> {
        Ok(WeixinConfig {
            accounts: resolve_builtin_accounts(
                self.plugin_channel(BUILTIN_CHANNEL_PLUGIN_WEIXIN),
                weixin_account_from_plugin_entry,
            )?,
        })
    }
}

fn resolve_builtin_accounts<T: Clone>(
    channel: Option<&PluginChannelConfig>,
    decode: fn(&PluginAccountEntry) -> Result<T, serde_json::Error>,
) -> Result<HashMap<String, T>, serde_json::Error> {
    let mut accounts = HashMap::new();
    let Some(channel) = channel else {
        return Ok(accounts);
    };
    for (account_id, entry) in &channel.accounts {
        accounts.insert(account_id.clone(), decode(entry)?);
    }
    Ok(accounts)
}

fn plugin_entry_payload_with_envelope(entry: &PluginAccountEntry) -> Value {
    let mut payload = match &entry.config {
        Value::Object(map) => map.clone(),
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("config".to_owned(), other.clone());
            map
        }
    };
    payload.insert("enabled".to_owned(), Value::Bool(entry.enabled));
    if let Some(name) = &entry.name {
        payload.insert("name".to_owned(), Value::String(name.clone()));
    }
    if let Some(agent_id) = &entry.agent_id {
        payload.insert("agent_id".to_owned(), Value::String(agent_id.clone()));
    }
    if let Some(workspace_dir) = &entry.workspace_dir {
        payload.insert(
            "workspace_dir".to_owned(),
            Value::String(workspace_dir.clone()),
        );
    }
    if let Some(workspace_mode) = &entry.workspace_mode {
        payload.insert(
            "workspace_mode".to_owned(),
            Value::String(workspace_mode.clone()),
        );
    }
    Value::Object(payload)
}

pub fn telegram_account_to_plugin_entry(account: &TelegramAccount) -> PluginAccountEntry {
    let mut config =
        serde_json::to_value(account).unwrap_or_else(|_| Value::Object(Default::default()));
    if let Some(map) = config.as_object_mut() {
        map.remove("enabled");
        map.remove("name");
        map.remove("agent_id");
        map.remove("workspace_dir");
        map.remove("workspace_mode");
    }
    PluginAccountEntry {
        enabled: account.enabled,
        name: account.name.clone(),
        agent_id: Some(account.agent_id.clone()),
        workspace_dir: account.workspace_dir.clone(),
        workspace_mode: None,
        config,
    }
}

pub fn discord_account_to_plugin_entry(account: &DiscordAccount) -> PluginAccountEntry {
    let mut config =
        serde_json::to_value(account).unwrap_or_else(|_| Value::Object(Default::default()));
    if let Some(map) = config.as_object_mut() {
        map.remove("enabled");
        map.remove("name");
        map.remove("agent_id");
        map.remove("workspace_dir");
        map.remove("workspace_mode");
    }
    PluginAccountEntry {
        enabled: account.enabled,
        name: account.name.clone(),
        agent_id: Some(account.agent_id.clone()),
        workspace_dir: account.workspace_dir.clone(),
        workspace_mode: None,
        config,
    }
}

pub fn feishu_account_to_plugin_entry(account: &FeishuAccount) -> PluginAccountEntry {
    let mut config =
        serde_json::to_value(account).unwrap_or_else(|_| Value::Object(Default::default()));
    if let Some(map) = config.as_object_mut() {
        map.remove("enabled");
        map.remove("name");
        map.remove("agent_id");
        map.remove("workspace_dir");
        map.remove("workspace_mode");
    }
    PluginAccountEntry {
        enabled: account.enabled,
        name: account.name.clone(),
        agent_id: Some(account.agent_id.clone()),
        workspace_dir: account.workspace_dir.clone(),
        workspace_mode: None,
        config,
    }
}

pub fn weixin_account_to_plugin_entry(account: &WeixinAccount) -> PluginAccountEntry {
    let mut config =
        serde_json::to_value(account).unwrap_or_else(|_| Value::Object(Default::default()));
    if let Some(map) = config.as_object_mut() {
        map.remove("enabled");
        map.remove("name");
        map.remove("agent_id");
        map.remove("workspace_dir");
        map.remove("workspace_mode");
    }
    PluginAccountEntry {
        enabled: account.enabled,
        name: account.name.clone(),
        agent_id: Some(account.agent_id.clone()),
        workspace_dir: account.workspace_dir.clone(),
        workspace_mode: None,
        config,
    }
}

pub fn telegram_account_from_plugin_entry(
    entry: &PluginAccountEntry,
) -> Result<TelegramAccount, serde_json::Error> {
    serde_json::from_value(plugin_entry_payload_with_envelope(entry))
}

pub fn discord_account_from_plugin_entry(
    entry: &PluginAccountEntry,
) -> Result<DiscordAccount, serde_json::Error> {
    serde_json::from_value(plugin_entry_payload_with_envelope(entry))
}

pub fn feishu_account_from_plugin_entry(
    entry: &PluginAccountEntry,
) -> Result<FeishuAccount, serde_json::Error> {
    serde_json::from_value(plugin_entry_payload_with_envelope(entry))
}

pub fn weixin_account_from_plugin_entry(
    entry: &PluginAccountEntry,
) -> Result<WeixinAccount, serde_json::Error> {
    serde_json::from_value(plugin_entry_payload_with_envelope(entry))
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct SlashCommand {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSlashCommand {
    pub name: String,
    pub description: String,
    pub prompt: Option<String>,
    pub skill_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    #[default]
    Stdio,
    StreamableHttp,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpServerConfig {
    #[serde(default)]
    pub transport: McpTransport,

    // --- STDIO fields ---
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub working_dir: Option<String>,

    // --- Streamable HTTP fields ---
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,

    // --- Common ---
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            transport: McpTransport::default(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
            url: None,
            bearer_token_env: None,
            headers: HashMap::new(),
            enabled: true,
        }
    }
}

fn extract_slash_command_name(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let token = trimmed.split_whitespace().next().unwrap_or(trimmed);
    let without_slash = token.strip_prefix('/')?;
    let command = without_slash
        .split('@')
        .next()
        .unwrap_or(without_slash)
        .trim();
    if command.is_empty() {
        None
    } else {
        Some(command)
    }
}

// ---------------------------------------------------------------------------
// Cron configuration
// ---------------------------------------------------------------------------

/// Action to perform when a cron job fires.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CronAction {
    /// Log a message (useful for testing / keep-alive).
    #[default]
    Log,
    /// Send a system event message into a target session.
    SystemEvent,
    /// Start an agent turn run for a target session.
    AgentTurn,
}

/// Product-level subtype carried by persisted cron jobs.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CronJobKind {
    #[default]
    AutomationPrompt,
}

/// UI-friendly schedule form preserved for automation jobs.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AutomationScheduleView {
    Daily {
        time: String,
        #[serde(default)]
        weekdays: Vec<String>,
        timezone: String,
    },
    Interval {
        hours: u64,
    },
    Monthly {
        day: u8,
        time: String,
        timezone: String,
    },
    Once {
        at: String,
    },
}

/// Schedule type for a cron job.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CronSchedule {
    /// Fire every `interval_secs` seconds.
    Interval { interval_secs: u64 },
    /// Fire once at an ISO-8601 timestamp.
    Once { at: String },
    /// Fire on a cron expression (seconds-precision format).
    Cron {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
    },
}

impl Default for CronSchedule {
    fn default() -> Self {
        Self::Interval {
            interval_secs: 3600,
        }
    }
}

/// A single cron job definition (as stored in config).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CronJobConfig {
    pub id: String,
    #[serde(default)]
    pub kind: CronJobKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub schedule: CronSchedule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_schedule: Option<AutomationScheduleView>,
    #[serde(default)]
    pub action: CronAction,
    /// Target delivery/thread handle ("last", "thread:<key>", or thread key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Message payload for `system_event` / `agent_turn` actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Delete job after first successful run.
    #[serde(default)]
    pub delete_after_run: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Top-level cron section.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CronConfig {
    #[serde(default)]
    pub jobs: Vec<CronJobConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionConfig {
    /// Base directory for file-based session storage.
    /// Defaults to `~/.gary/data` if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DesktopLabsConfig {
    #[serde(default = "default_true")]
    pub auto_research: bool,
}

impl Default for DesktopLabsConfig {
    fn default() -> Self {
        Self {
            auto_research: true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DesktopConfig {
    #[serde(default)]
    pub labs: DesktopLabsConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TasksConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Top-level configuration for the host-side plugin auto-updater.
///
/// Mirrors the desktop app's electron-updater behavior conceptually:
/// the gateway periodically checks each installed subprocess plugin
/// against its declared `[update].manifest_url`, downloads + verifies
/// + atomically promotes a new bundle when available, then hot-swaps
/// the live subprocess via the §9.4 respawn path. Plugins must opt
/// in to the hot-swap via `[capabilities].survives_respawn = true`;
/// for plugins without that flag the auto-updater still downloads
/// the new bundle but leaves the running subprocess on the old image
/// and logs a "restart required" hint.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PluginsConfig {
    /// Master switch. When `false`, no periodic check fires and no
    /// download is attempted. Defaults to `true` for parity with
    /// `garyx-desktop`'s `autoDownload = true`.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Seconds between recurring checks. Default `21600` (6 h)
    /// matches `RECURRING_CHECK_INTERVAL_MS` in the desktop updater.
    /// Minimum enforced at 60s — anything lower hammers manifest
    /// hosts and risks rate-limiting.
    #[serde(default = "default_auto_update_interval_secs")]
    pub auto_update_check_interval_secs: u64,
}

fn default_auto_update_interval_secs() -> u64 {
    21_600
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            auto_update: true,
            auto_update_check_interval_secs: default_auto_update_interval_secs(),
        }
    }
}

/// Dreams topic extraction and automatic scan configuration.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DreamsConfig {
    /// Master switch for app entry points and automatic scans. API/CLI
    /// manual scans stay available when this is false.
    #[serde(default)]
    pub enabled: bool,
    /// Seconds between automatic scan attempts while enabled.
    #[serde(default = "default_dreams_scan_interval_secs")]
    pub scan_interval_secs: u64,
    /// Lookback window, in hours, for each automatic scan.
    #[serde(default = "default_dreams_scan_since_hours")]
    pub scan_since_hours: i64,
}

fn default_dreams_scan_interval_secs() -> u64 {
    3_600
}

fn default_dreams_scan_since_hours() -> i64 {
    1
}

impl Default for DreamsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            scan_interval_secs: default_dreams_scan_interval_secs(),
            scan_since_hours: default_dreams_scan_since_hours(),
        }
    }
}

/// Root configuration for Garyx.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct GaryxConfig {
    #[serde(default)]
    pub agents: HashMap<String, Value>,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub sessions: SessionConfig,
    #[serde(default)]
    pub desktop: DesktopConfig,
    #[serde(default)]
    pub tasks: TasksConfig,
    #[serde(default)]
    pub dreams: DreamsConfig,
    #[serde(default)]
    pub cron: CronConfig,
    #[serde(default)]
    pub commands: Vec<SlashCommand>,
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(default)]
    pub plugins: PluginsConfig,
}

impl GaryxConfig {
    pub fn resolve_slash_command(&self, text: &str) -> Option<ResolvedSlashCommand> {
        let command_name = extract_slash_command_name(text)?;
        self.commands
            .iter()
            .find(|command| command.name.eq_ignore_ascii_case(command_name))
            .map(|command| ResolvedSlashCommand {
                name: command.name.clone(),
                description: command.description.clone(),
                prompt: command
                    .prompt
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                skill_id: command
                    .skill_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            })
    }
}

#[cfg(test)]
mod tests;
