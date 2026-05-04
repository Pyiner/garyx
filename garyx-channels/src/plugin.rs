use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{info, warn};

use garyx_bridge::MultiProviderBridge;
use garyx_models::config::{
    ChannelsConfig, FeishuAccount, FeishuConfig, FeishuDomain, OwnerTargetConfig, TelegramAccount,
    TelegramConfig, WeixinAccount, WeixinConfig,
};
use garyx_models::routing::{DELIVERY_TARGET_TYPE_CHAT_ID, DELIVERY_TARGET_TYPE_OPEN_ID};
use garyx_router::MessageRouter;
use garyx_router::{KnownChannelEndpoint, endpoint_key};

use crate::auth_flow::AuthFlowExecutor;
use crate::channel_trait::{Channel, ChannelError};
use crate::dispatcher::{OutboundMessage, SendMessageResult, SwappableDispatcher};
use crate::plugin_host::manifest::ManifestCapabilities;
use crate::plugin_host::{
    AccountDescriptor, AccountRootBehavior, HostContext, InboundHandler, InitializeParams,
    InitializeResult, PluginErrorCode, PluginManifest, PluginRpcClient, PluginSenderHandle,
    RpcError, SpawnOptions, SubprocessError, SubprocessPlugin,
};
use crate::{FeishuChannel, TelegramChannel, WeixinChannel};

// ---------------------------------------------------------------------------
// Plugin model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Loaded,
    Initializing,
    Ready,
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub source: String,
    /// Configuration methods this channel advertises. The desktop UI
    /// walks the list in order and renders each method as its own
    /// block (schema-driven form, auto-login button, …). Empty ⇒ the
    /// channel exposes no configuration surface (treated as a
    /// misconfig by the UI). `#[serde(default)]` keeps older
    /// serialised payloads (pre-§11) deserialisable as an empty
    /// vec — do not remove.
    #[serde(default)]
    pub config_methods: Vec<crate::auth_flow::ConfigMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStatus {
    pub metadata: PluginMetadata,
    pub state: PluginState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginMainEndpoint {
    pub endpoint_key: String,
    pub channel: String,
    pub account_id: String,
    pub binding_key: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    pub delivery_thread_id: Option<String>,
    pub display_label: String,
    pub thread_id: Option<String>,
    pub thread_label: Option<String>,
    pub workspace_dir: Option<String>,
    pub thread_updated_at: Option<String>,
    pub last_inbound_at: Option<String>,
    pub last_delivery_at: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginConversationEndpoint {
    pub endpoint_key: String,
    pub channel: String,
    pub account_id: String,
    pub binding_key: String,
    pub chat_id: String,
    pub delivery_target_type: String,
    pub delivery_target_id: String,
    pub delivery_thread_id: Option<String>,
    pub display_label: String,
    pub thread_id: Option<String>,
    pub thread_label: Option<String>,
    pub workspace_dir: Option<String>,
    pub thread_updated_at: Option<String>,
    pub last_inbound_at: Option<String>,
    pub last_delivery_at: Option<String>,
    pub conversation_kind: Option<String>,
    pub conversation_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginConversationNode {
    pub id: String,
    pub endpoint_key: String,
    pub kind: String,
    pub title: String,
    pub badge: Option<String>,
    pub latest_activity: Option<String>,
    pub openable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginAccountUi {
    pub default_open_endpoint_key: Option<String>,
    #[serde(default)]
    pub conversation_nodes: Vec<PluginConversationNode>,
}

/// Lifecycle hooks for a registered plugin. Takes `&self` (not
/// `&mut self`) so callers can hold an `Arc<dyn ChannelPlugin>`
/// and still drive the lifecycle — interior mutability in the
/// implementing adapter is where the real mutation happens (the
/// builtin adapter's inner `Box<dyn Channel>` is behind a
/// `tokio::sync::Mutex`; the subprocess adapter's RPC client is an
/// `Arc<Mutex<...>>`).
///
/// This lets `ChannelPluginManager` store a single
/// `BTreeMap<String, Arc<dyn ChannelPlugin>>` instead of the
/// pre-refactor split between builtin and subprocess maps.
#[async_trait]
pub trait PluginLifecycle: Send + Sync {
    async fn initialize(&self) -> Result<(), String>;
    async fn start(&self) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn cleanup(&self) -> Result<(), String>;
}

/// Unified plugin abstraction covering both built-in channels
/// (feishu / weixin / telegram) and subprocess plugins.
///
/// The trait is deliberately agnostic to **where** the plugin's
/// code runs — spawn / exit / respawn are implementation details
/// of the subprocess layer and stay out of this interface. Built-in
/// plugins that don't need an operation (e.g. Telegram has no auto
/// login) get a default implementation that returns the "not
/// supported" signal so callers don't have to special-case.
///
/// Method groups:
///
/// - **Identity & metadata**: [`metadata`], [`capabilities`],
///   [`schema`].
/// - **Channel-semantic lifecycle** (the existing
///   [`PluginLifecycle`] super-trait): `initialize` / `start` /
///   `stop` / `cleanup`.
/// - **Auth flow**: [`auth_flow`] — returns a channel-blind
///   executor or `None`.
///
/// - **Account state**: [`reload_accounts`] — host pushes the
///   authoritative account list to the plugin (§6.5); plugin
///   replaces its internal view. Both built-ins and subprocess
///   plugins implement this so the gateway's
///   `apply_runtime_config` path can treat every plugin the same
///   way.
#[async_trait]
pub trait ChannelPlugin: PluginLifecycle {
    fn metadata(&self) -> &PluginMetadata;

    fn account_root_behavior(&self) -> AccountRootBehavior {
        AccountRootBehavior::OpenDefault
    }

    /// Downcast hook: return `Some` iff this plugin is the built-in
    /// `ManagedChannelPlugin`. Used by the manager's
    /// `reload_builtin_senders` path to update per-account outbound
    /// senders on config change without a full plugin rebuild.
    /// Default `None` for subprocess plugins — they track their own
    /// accounts via the manifest handshake.
    fn as_managed(&self) -> Option<&ManagedChannelPlugin> {
        None
    }

    /// The plugin's capability bits (outbound / inbound / streaming
    /// / images / files / delivery_model). Built-in channels return
    /// their hardcoded [`crate::builtin_catalog::builtin_capabilities`]
    /// shape; subprocess plugins return whatever their manifest
    /// declared. The gateway uses this to decide whether a plugin
    /// can satisfy e.g. "send an image" without having to special-
    /// case built-in vs subprocess.
    ///
    /// Default implementation returns a no-op capability set so
    /// older built-in impls that haven't been updated yet keep
    /// compiling; they should be migrated to report accurate bits.
    fn capabilities(&self) -> ManifestCapabilities {
        crate::builtin_catalog::builtin_capabilities(false, false, false)
    }

    /// JSON Schema (2020-12) describing ONE account's config. The
    /// desktop UI renders this verbatim through its generic
    /// `JsonSchemaForm`. Built-ins return hardcoded schemas from
    /// [`crate::builtin_catalog`]; subprocess plugins return what
    /// their `describe` RPC produced. Changing shape here is a
    /// protocol revision — coordinate with the desktop form
    /// renderer.
    ///
    /// Default returns an empty object schema (no fields) so a
    /// misconfigured plugin can't brick the UI; a real impl MUST
    /// return the full schema.
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {}
        })
    }

    /// The channel-blind auto-login executor for this plugin, if
    /// any. Returns `None` for plugins that only support form
    /// configuration (Telegram) — the UI walks `metadata()
    /// .config_methods` to decide whether to render an auto-login
    /// button and only calls this when it needs an executor.
    ///
    /// Default `None` so existing built-in impls that haven't been
    /// extended yet keep compiling.
    fn auth_flow(&self) -> Option<Arc<dyn AuthFlowExecutor>> {
        None
    }

    /// Validate a single account configuration before the host persists
    /// it. Implementations should perform the strongest safe check the
    /// channel supports: built-ins can call provider APIs, while generic
    /// subprocess plugins can opt into this later without changing the
    /// gateway or desktop API surface.
    async fn validate_account_config(
        &self,
        _account: AccountDescriptor,
    ) -> Result<AccountValidationResult, String> {
        Ok(AccountValidationResult::skipped(format!(
            "plugin '{}' does not expose account connectivity validation",
            self.metadata().id
        )))
    }

    /// Channel-blind outbound dispatch. Subprocess plugins forward
    /// the request to the child's `dispatch_outbound` RPC;
    /// [`ManagedChannelPlugin`] (built-in) currently delegates to
    /// [`crate::dispatcher::SwappableDispatcher`] via the
    /// per-channel sender maps — see the default impl below.
    ///
    /// This method is intentionally additive. The existing
    /// dispatcher-driven path (`SwappableDispatcher::send_message`)
    /// remains the production route for now; built-ins return
    /// `Unsupported` here until the dispatcher is refactored to
    /// call this trait method instead. Subprocess plugins work
    /// end-to-end through it today.
    async fn dispatch_outbound(
        &self,
        _msg: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        Err(ChannelError::Config(format!(
            "plugin '{}' does not route outbound through the trait — \
             use SwappableDispatcher::send_message instead",
            self.metadata().id
        )))
    }

    /// Push the authoritative account list to the plugin (§6.5).
    /// Host owns `ChannelsConfig`; whenever the config changes, the
    /// gateway's `apply_runtime_config` iterates every registered
    /// plugin and calls this method. The plugin replaces its
    /// internal view — what that means depends on execution model:
    /// - Built-in (`ManagedChannelPlugin`): rebuild the per-account
    ///   `OutboundSender` map atomically via `ArcSwap`.
    /// - Subprocess (`SubprocessChannelPlugin`): forward as an
    ///   `accounts/reload` JSON-RPC call; the child replaces its
    ///   own account store.
    ///
    /// Default returns `Ok(())` so plugins that don't care about
    /// account-level hot-reload (or that handle it via respawn)
    /// stay compilable without an empty override.
    ///
    /// Errors: built-ins never reject; subprocess plugins may
    /// return `Err(reason)` for transport failures or
    /// `ConfigRejected`. The gateway logs but does not abort the
    /// outer config-apply on a single-plugin reload failure — the
    /// plugin's state remains whatever it was before the call.
    async fn reload_accounts(&self, _accounts: Vec<AccountDescriptor>) -> Result<(), String> {
        Ok(())
    }

    async fn resolve_main_endpoint(
        &self,
        _account_id: &str,
        _endpoints: &[KnownChannelEndpoint],
    ) -> Option<PluginMainEndpoint> {
        None
    }

    async fn resolve_account_ui(
        &self,
        _account_id: &str,
        _endpoints: &[PluginConversationEndpoint],
    ) -> Option<PluginAccountUi> {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountValidationResult {
    pub validated: bool,
    pub message: String,
}

impl AccountValidationResult {
    fn verified(message: impl Into<String>) -> Self {
        Self {
            validated: true,
            message: message.into(),
        }
    }

    fn skipped(message: impl Into<String>) -> Self {
        Self {
            validated: false,
            message: message.into(),
        }
    }
}

/// Built-in channel adapter: wraps a concrete `Channel` impl +
/// optional [`AuthFlowExecutor`] and presents them as a
/// [`ChannelPlugin`]. This is the "Scheme A" sibling to
/// `SubprocessPluginImpl` — built-in channels ride the same trait
/// so `ChannelPluginManager` can dispatch uniformly.
pub struct ManagedChannelPlugin {
    metadata: PluginMetadata,
    /// Inner built-in `Channel` impl. Wrapped in a tokio mutex so
    /// the `PluginLifecycle` trait methods can take `&self` — the
    /// underlying `Channel` trait still uses `&mut self` for
    /// start/stop, so interior mutability is necessary and must
    /// span the `.await` inside each method.
    channel: tokio::sync::Mutex<Box<dyn Channel>>,
    capabilities: ManifestCapabilities,
    schema: serde_json::Value,
    auth_flow: Option<Arc<dyn AuthFlowExecutor>>,
    account_root_behavior: AccountRootBehavior,
    account_descriptors: arc_swap::ArcSwap<HashMap<String, AccountDescriptor>>,
    /// Per-account outbound sender table, lock-free reads via
    /// [`arc_swap::ArcSwap`]. Atomic replace on runtime-config
    /// change so the gateway's `apply_runtime_config` path updates
    /// the map in-place without tearing down the Channel's inbound
    /// polling (§9.4 Codex review P1).
    ///
    /// An `Arc<HashMap::new()>` is the stable "no senders" state —
    /// every `dispatch_outbound` lookup misses with "account not
    /// registered" and the caller knows outbound routing is not
    /// wired yet. No `Option` wrapper because the swap target must
    /// always be a live `Arc`.
    outbound_senders:
        arc_swap::ArcSwap<HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>>>,
}

/// Builder-style options for [`ManagedChannelPlugin`]. Added so the
/// constructor doesn't balloon to 5 positional args every time we
/// extend the trait. Required fields go through `new`; the rest
/// are optional.
pub struct ManagedChannelPluginOptions {
    pub capabilities: ManifestCapabilities,
    pub schema: serde_json::Value,
    pub auth_flow: Option<Arc<dyn AuthFlowExecutor>>,
    pub account_root_behavior: AccountRootBehavior,
    pub accounts: Option<Vec<AccountDescriptor>>,
    /// Per-account outbound senders. When `Some(map)`, the plugin
    /// routes `dispatch_outbound` by looking up
    /// `map[msg.account_id]` and delegating to
    /// `OutboundSender::send_outbound`. `None` leaves the plugin's
    /// dispatch path unwired — gateway `send_message` falls through
    /// to the dispatcher's legacy per-channel sender maps.
    pub outbound_senders: Option<HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>>>,
}

impl ManagedChannelPlugin {
    /// Minimal constructor: metadata + channel only. Capabilities and
    /// schema fall back to the trait defaults (no-op). Prefer
    /// [`Self::with_options`] for built-in channels — the default
    /// capabilities bits are always wrong in production.
    pub fn new(metadata: PluginMetadata, channel: Box<dyn Channel>) -> Self {
        Self {
            metadata,
            channel: tokio::sync::Mutex::new(channel),
            capabilities: crate::builtin_catalog::builtin_capabilities(false, false, false),
            schema: serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {}
            }),
            auth_flow: None,
            account_root_behavior: AccountRootBehavior::OpenDefault,
            account_descriptors: arc_swap::ArcSwap::from_pointee(HashMap::new()),
            outbound_senders: arc_swap::ArcSwap::from_pointee(HashMap::new()),
        }
    }

    /// Constructor for channels that support auto-login but the
    /// caller doesn't want to spell out full options. Equivalent to
    /// `with_options` with default capabilities + empty schema.
    /// Prefer `with_options` in new code.
    pub fn with_auth_flow(
        metadata: PluginMetadata,
        channel: Box<dyn Channel>,
        auth_flow: Option<Arc<dyn AuthFlowExecutor>>,
    ) -> Self {
        let mut this = Self::new(metadata, channel);
        this.auth_flow = auth_flow;
        this
    }

    /// Full constructor — every built-in channel registration goes
    /// through this. The `schema` ends up as the JSON the desktop UI
    /// renders into a form; `capabilities` drives which outbound /
    /// inbound paths the gateway exposes; `auth_flow` is `Some` for
    /// channels with automated login.
    pub fn with_options(
        metadata: PluginMetadata,
        channel: Box<dyn Channel>,
        opts: ManagedChannelPluginOptions,
    ) -> Self {
        Self {
            metadata,
            channel: tokio::sync::Mutex::new(channel),
            capabilities: opts.capabilities,
            schema: opts.schema,
            auth_flow: opts.auth_flow,
            account_root_behavior: opts.account_root_behavior,
            account_descriptors: arc_swap::ArcSwap::from_pointee(
                opts.accounts
                    .unwrap_or_default()
                    .into_iter()
                    .map(|account| (account.id.clone(), account))
                    .collect(),
            ),
            outbound_senders: arc_swap::ArcSwap::from_pointee(
                opts.outbound_senders.unwrap_or_default(),
            ),
        }
    }

    /// Atomically replace the per-account sender table. The gateway
    /// calls this from `apply_runtime_config` after the user edits
    /// the channel config so `dispatch_outbound` picks up new / edited
    /// accounts without a full plugin teardown. Reads are lock-free —
    /// any in-flight `dispatch_outbound` that already grabbed the old
    /// Arc completes against that snapshot.
    pub fn replace_outbound_senders(
        &self,
        senders: HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>>,
    ) {
        self.outbound_senders.store(Arc::new(senders));
    }

    fn account_descriptor(&self, account_id: &str) -> Option<AccountDescriptor> {
        self.account_descriptors.load().get(account_id).cloned()
    }
}

#[async_trait]
impl PluginLifecycle for ManagedChannelPlugin {
    async fn initialize(&self) -> Result<(), String> {
        Ok(())
    }

    async fn start(&self) -> Result<(), String> {
        self.channel
            .lock()
            .await
            .start()
            .await
            .map_err(|e| e.to_string())
    }

    async fn stop(&self) -> Result<(), String> {
        self.channel
            .lock()
            .await
            .stop()
            .await
            .map_err(|e| e.to_string())
    }

    async fn cleanup(&self) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
impl ChannelPlugin for ManagedChannelPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    fn account_root_behavior(&self) -> AccountRootBehavior {
        self.account_root_behavior
    }

    fn as_managed(&self) -> Option<&ManagedChannelPlugin> {
        Some(self)
    }

    fn capabilities(&self) -> ManifestCapabilities {
        self.capabilities.clone()
    }

    fn schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    fn auth_flow(&self) -> Option<Arc<dyn AuthFlowExecutor>> {
        self.auth_flow.clone()
    }

    async fn validate_account_config(
        &self,
        account: AccountDescriptor,
    ) -> Result<AccountValidationResult, String> {
        match self.metadata.id.as_str() {
            "telegram" => validate_telegram_account_config(account).await,
            "feishu" => validate_feishu_account_config(account).await,
            "weixin" => validate_weixin_account_config(account),
            _ => Ok(AccountValidationResult::skipped(format!(
                "plugin '{}' does not expose account connectivity validation",
                self.metadata.id
            ))),
        }
    }

    async fn dispatch_outbound(
        &self,
        msg: crate::dispatcher::OutboundMessage,
    ) -> Result<crate::dispatcher::SendMessageResult, ChannelError> {
        // Trait-level routing for built-ins: atomic snapshot read
        // of the per-account sender table (lock-free — `ArcSwap`
        // hands back the inner Arc which we clone before releasing
        // the guard). Unknown account id means the dispatch
        // destination doesn't exist in the current config; a fresh
        // Arc swap from `replace_outbound_senders` would reflect
        // new accounts on the next call.
        let snapshot = self.outbound_senders.load();
        let sender = snapshot.get(&msg.account_id).cloned().ok_or_else(|| {
            ChannelError::Config(format!(
                "{} account '{}' not registered",
                self.metadata.id, msg.account_id
            ))
        })?;
        // Drop the guard before `await` so concurrent swaps don't
        // race with a long send.
        drop(snapshot);
        sender.send_outbound(msg).await
    }

    /// Rebuild the per-account `OutboundSender` map from the
    /// host-pushed account list. `AccountDescriptor.config` is the
    /// plugin-schema-shaped JSON (e.g. `{"token": "…"}` for
    /// telegram); deserialise it into the matching account struct
    /// and construct a fresh sender. `ArcSwap::store` publishes
    /// the new map atomically — an in-flight `dispatch_outbound`
    /// that already loaded the old snapshot finishes against it.
    ///
    /// Dispatches by `self.metadata.id` because each built-in has
    /// a different account-struct shape. An id we don't recognise
    /// is a no-op (better than panicking: the caller may be
    /// iterating every plugin including some not covered here).
    async fn reload_accounts(&self, accounts: Vec<AccountDescriptor>) -> Result<(), String> {
        let mut out: HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>> = HashMap::new();
        match self.metadata.id.as_str() {
            "telegram" => {
                for acc in &accounts {
                    let parsed: TelegramAccount = serde_json::from_value(acc.config.clone())
                        .map_err(|e| {
                            format!("telegram account '{}' config rejected: {e}", acc.id)
                        })?;
                    if !parsed.enabled && !acc.enabled {
                        continue;
                    }
                    let sender: Arc<dyn crate::dispatcher::OutboundSender> =
                        Arc::new(crate::dispatcher::TelegramSender {
                            account_id: acc.id.clone(),
                            token: parsed.token,
                            http: reqwest::Client::new(),
                            api_base: "https://api.telegram.org".to_owned(),
                            is_running: false,
                        });
                    out.insert(acc.id.clone(), sender);
                }
            }
            "feishu" => {
                for acc in &accounts {
                    let parsed: FeishuAccount = serde_json::from_value(acc.config.clone())
                        .map_err(|e| format!("feishu account '{}' config rejected: {e}", acc.id))?;
                    if !parsed.enabled && !acc.enabled {
                        continue;
                    }
                    let api_base = match parsed.domain {
                        garyx_models::config::FeishuDomain::Feishu => {
                            "https://open.feishu.cn/open-apis"
                        }
                        garyx_models::config::FeishuDomain::Lark => {
                            "https://open.larksuite.com/open-apis"
                        }
                    };
                    let sender: Arc<dyn crate::dispatcher::OutboundSender> =
                        Arc::new(crate::dispatcher::FeishuSender::new(
                            acc.id.clone(),
                            parsed.app_id,
                            parsed.app_secret,
                            api_base.to_owned(),
                            false,
                        ));
                    out.insert(acc.id.clone(), sender);
                }
            }
            "weixin" => {
                for acc in &accounts {
                    let parsed: garyx_models::config::WeixinAccount =
                        serde_json::from_value(acc.config.clone()).map_err(|e| {
                            format!("weixin account '{}' config rejected: {e}", acc.id)
                        })?;
                    if !parsed.enabled && !acc.enabled {
                        continue;
                    }
                    let sender: Arc<dyn crate::dispatcher::OutboundSender> =
                        Arc::new(crate::dispatcher::WeixinSender {
                            account_id: acc.id.clone(),
                            account: parsed,
                            http: reqwest::Client::new(),
                            is_running: false,
                        });
                    out.insert(acc.id.clone(), sender);
                }
            }
            _ => {
                // Unknown built-in id — no sender builder registered.
                // Leave the existing map in place rather than
                // wiping it; a future channel author adds a case
                // above when they ship.
                return Ok(());
            }
        }
        self.account_descriptors.store(Arc::new(
            accounts
                .into_iter()
                .map(|account| (account.id.clone(), account))
                .collect(),
        ));
        self.outbound_senders.store(Arc::new(out));
        Ok(())
    }

    async fn resolve_main_endpoint(
        &self,
        account_id: &str,
        endpoints: &[KnownChannelEndpoint],
    ) -> Option<PluginMainEndpoint> {
        match self.metadata.id.as_str() {
            "telegram" => {
                let account: TelegramAccount =
                    serde_json::from_value(self.account_descriptor(account_id)?.config).ok()?;
                resolve_telegram_main_endpoint(account_id, &account, endpoints).await
            }
            "feishu" => {
                let account: FeishuAccount =
                    serde_json::from_value(self.account_descriptor(account_id)?.config).ok()?;
                resolve_feishu_main_endpoint(account_id, &account, endpoints).await
            }
            "weixin" => {
                self.account_descriptor(account_id)?;
                best_private_endpoint(endpoints, "weixin", account_id)
            }
            _ => None,
        }
    }

    async fn resolve_account_ui(
        &self,
        account_id: &str,
        endpoints: &[PluginConversationEndpoint],
    ) -> Option<PluginAccountUi> {
        match self.metadata.id.as_str() {
            "telegram" | "feishu" | "weixin" => Some(build_builtin_account_ui(
                self.account_root_behavior,
                &self.metadata.id,
                account_id,
                endpoints,
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct TelegramGetMeResponse {
    ok: bool,
    #[serde(default)]
    description: String,
}

async fn validate_telegram_account_config(
    account: AccountDescriptor,
) -> Result<AccountValidationResult, String> {
    let parsed: TelegramAccount = serde_json::from_value(account.config)
        .map_err(|error| format!("telegram account '{}' config rejected: {error}", account.id))?;
    let token = parsed.token.trim();
    if token.is_empty() {
        return Err("Telegram token is required.".to_owned());
    }

    let http = HttpClient::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| format!("failed to build Telegram validator: {error}"))?;
    let url = format!("https://api.telegram.org/bot{token}/getMe");
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Telegram getMe request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Telegram getMe HTTP {status}"));
    }
    let payload: TelegramGetMeResponse = response
        .json()
        .await
        .map_err(|error| format!("Telegram getMe response parse failed: {error}"))?;
    if !payload.ok {
        let reason = payload.description.trim();
        return Err(if reason.is_empty() {
            "Telegram getMe rejected this token.".to_owned()
        } else {
            format!("Telegram getMe rejected this token: {reason}")
        });
    }
    Ok(AccountValidationResult::verified(
        "Telegram bot token verified with getMe.",
    ))
}

#[derive(Debug, Deserialize)]
struct FeishuTokenValidationResponse {
    code: i64,
    #[serde(default)]
    tenant_access_token: String,
    #[serde(default)]
    msg: String,
}

async fn validate_feishu_account_config(
    account: AccountDescriptor,
) -> Result<AccountValidationResult, String> {
    let parsed: FeishuAccount = serde_json::from_value(account.config)
        .map_err(|error| format!("feishu account '{}' config rejected: {error}", account.id))?;
    if parsed.app_id.trim().is_empty() || parsed.app_secret.trim().is_empty() {
        return Err("Feishu app_id and app_secret are required.".to_owned());
    }

    let api_base = match parsed.domain {
        FeishuDomain::Feishu => "https://open.feishu.cn/open-apis",
        FeishuDomain::Lark => "https://open.larksuite.com/open-apis",
    };
    let http = HttpClient::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| format!("failed to build Feishu validator: {error}"))?;
    let response = http
        .post(format!("{api_base}/auth/v3/tenant_access_token/internal"))
        .json(&json!({
            "app_id": parsed.app_id,
            "app_secret": parsed.app_secret,
        }))
        .send()
        .await
        .map_err(|error| format!("Feishu token request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Feishu token request HTTP {status}"));
    }
    let payload: FeishuTokenValidationResponse = response
        .json()
        .await
        .map_err(|error| format!("Feishu token response parse failed: {error}"))?;
    if payload.code != 0 {
        return Err(format!(
            "Feishu token request rejected credentials (code={}): {}",
            payload.code, payload.msg
        ));
    }
    if payload.tenant_access_token.trim().is_empty() {
        return Err("Feishu token request returned an empty token.".to_owned());
    }
    Ok(AccountValidationResult::verified(
        "Feishu app credentials verified with tenant access token.",
    ))
}

fn validate_weixin_account_config(
    account: AccountDescriptor,
) -> Result<AccountValidationResult, String> {
    let parsed: WeixinAccount = serde_json::from_value(account.config)
        .map_err(|error| format!("weixin account '{}' config rejected: {error}", account.id))?;
    if parsed.token.trim().is_empty() {
        return Err("Weixin token is required.".to_owned());
    }
    reqwest::Url::parse(parsed.base_url.trim())
        .map_err(|error| format!("Weixin base_url is invalid: {error}"))?;
    Ok(AccountValidationResult::skipped(
        "Weixin has no safe side-effect-free connectivity probe yet; token shape and base_url were checked.",
    ))
}

fn binding_delivery_thread_id(binding_key: &str, chat_id: &str) -> Option<String> {
    let binding_key = binding_key.trim();
    let chat_id = chat_id.trim();
    if binding_key.is_empty() || binding_key == chat_id {
        None
    } else {
        Some(binding_key.to_owned())
    }
}

fn endpoint_scope(endpoint: &KnownChannelEndpoint) -> Option<&str> {
    let binding_key = endpoint.binding_key.trim();
    let chat_id = endpoint.chat_id.trim();
    if binding_key.is_empty() || binding_key == chat_id {
        None
    } else {
        Some(binding_key)
    }
}

fn is_telegram_private_candidate(endpoint: &KnownChannelEndpoint) -> bool {
    endpoint.channel == "telegram"
        && endpoint_scope(endpoint).is_none()
        && !endpoint.binding_key.trim().is_empty()
        && endpoint.binding_key.trim() == endpoint.chat_id.trim()
}

fn is_feishu_private_candidate(endpoint: &KnownChannelEndpoint) -> bool {
    endpoint.channel == "feishu"
        && endpoint_scope(endpoint).is_none()
        && endpoint.binding_key.trim().starts_with("ou_")
        && (!endpoint.chat_id.trim().is_empty()
            && (endpoint.chat_id.trim().starts_with("oc_")
                || endpoint.chat_id.trim() == endpoint.binding_key.trim()))
}

fn is_weixin_private_candidate(endpoint: &KnownChannelEndpoint) -> bool {
    endpoint.channel == "weixin"
        && endpoint_scope(endpoint).is_none()
        && !endpoint.binding_key.trim().is_empty()
        && !endpoint.chat_id.trim().is_empty()
        && endpoint.binding_key.trim() == endpoint.chat_id.trim()
}

fn normalized_owner_target(
    channel: &str,
    owner_target: Option<&OwnerTargetConfig>,
) -> Option<(String, String)> {
    let target = owner_target?;
    let target_id = target.target_id.trim();
    if target_id.is_empty() {
        return None;
    }
    let target_type = match channel {
        "feishu" if target.target_type.trim().is_empty() => DELIVERY_TARGET_TYPE_OPEN_ID.to_owned(),
        _ if target.target_type.trim().is_empty() => DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        _ => target.target_type.trim().to_owned(),
    };
    Some((target_type, target_id.to_owned()))
}

#[allow(clippy::too_many_arguments)]
fn synthetic_main_endpoint(
    channel: &str,
    account_id: &str,
    binding_key: &str,
    chat_id: &str,
    delivery_target_type: &str,
    delivery_target_id: &str,
    workspace_dir: Option<&str>,
    source: &str,
) -> PluginMainEndpoint {
    PluginMainEndpoint {
        endpoint_key: endpoint_key(channel, account_id, binding_key),
        channel: channel.to_owned(),
        account_id: account_id.to_owned(),
        binding_key: binding_key.to_owned(),
        chat_id: chat_id.to_owned(),
        delivery_target_type: delivery_target_type.to_owned(),
        delivery_target_id: delivery_target_id.to_owned(),
        delivery_thread_id: binding_delivery_thread_id(binding_key, chat_id),
        display_label: "Main Chat".to_owned(),
        thread_id: None,
        thread_label: None,
        workspace_dir: workspace_dir.map(ToOwned::to_owned),
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
        source: source.to_owned(),
    }
}

fn attach_known_endpoint_metadata(
    mut endpoint: PluginMainEndpoint,
    endpoints: &[KnownChannelEndpoint],
) -> PluginMainEndpoint {
    let Some(known) = endpoints.iter().find(|candidate| {
        candidate.endpoint_key == endpoint.endpoint_key
            && candidate.channel == endpoint.channel
            && candidate.account_id == endpoint.account_id
    }) else {
        return endpoint;
    };

    if !known.chat_id.trim().is_empty() {
        endpoint.chat_id = known.chat_id.clone();
    }
    if !known.delivery_target_type.trim().is_empty() {
        endpoint.delivery_target_type = known.delivery_target_type.clone();
    }
    if !known.delivery_target_id.trim().is_empty() {
        endpoint.delivery_target_id = known.delivery_target_id.clone();
    }
    if !known.display_label.trim().is_empty() {
        endpoint.display_label = known.display_label.clone();
    }
    endpoint.delivery_thread_id =
        binding_delivery_thread_id(&endpoint.binding_key, &endpoint.chat_id);
    endpoint.thread_id = known.thread_id.clone();
    endpoint.thread_label = known.thread_label.clone();
    if known.workspace_dir.is_some() {
        endpoint.workspace_dir = known.workspace_dir.clone();
    }
    endpoint.thread_updated_at = known.thread_updated_at.clone();
    endpoint.last_inbound_at = known.last_inbound_at.clone();
    endpoint.last_delivery_at = known.last_delivery_at.clone();
    endpoint
}

fn best_private_endpoint(
    endpoints: &[KnownChannelEndpoint],
    channel: &str,
    account_id: &str,
) -> Option<PluginMainEndpoint> {
    let mut candidates: Vec<_> = endpoints
        .iter()
        .filter(|endpoint| endpoint.account_id == account_id && endpoint.channel == channel)
        .filter(|endpoint| match channel {
            "telegram" => is_telegram_private_candidate(endpoint),
            "feishu" => is_feishu_private_candidate(endpoint),
            "weixin" => is_weixin_private_candidate(endpoint),
            _ => false,
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        let ts = |ep: &KnownChannelEndpoint| -> String {
            let inbound = ep.last_inbound_at.as_deref().unwrap_or("");
            let delivery = ep.last_delivery_at.as_deref().unwrap_or("");
            std::cmp::max(inbound, delivery).to_owned()
        };
        ts(b).cmp(&ts(a))
    });

    let endpoint = candidates[0];
    Some(PluginMainEndpoint {
        endpoint_key: endpoint.endpoint_key.clone(),
        channel: endpoint.channel.clone(),
        account_id: endpoint.account_id.clone(),
        binding_key: endpoint.binding_key.clone(),
        chat_id: endpoint.chat_id.clone(),
        delivery_target_type: endpoint.delivery_target_type.clone(),
        delivery_target_id: endpoint.delivery_target_id.clone(),
        delivery_thread_id: binding_delivery_thread_id(&endpoint.binding_key, &endpoint.chat_id),
        display_label: endpoint.display_label.clone(),
        thread_id: endpoint.thread_id.clone(),
        thread_label: endpoint.thread_label.clone(),
        workspace_dir: endpoint.workspace_dir.clone(),
        thread_updated_at: endpoint.thread_updated_at.clone(),
        last_inbound_at: endpoint.last_inbound_at.clone(),
        last_delivery_at: endpoint.last_delivery_at.clone(),
        source: "existing_private_endpoint".to_owned(),
    })
}

async fn resolve_feishu_owner_open_id(account_id: &str, account: &FeishuAccount) -> Option<String> {
    let api_base = match account.domain {
        FeishuDomain::Lark => "https://open.larksuite.com/open-apis",
        FeishuDomain::Feishu => "https://open.feishu.cn/open-apis",
    };
    let sender = crate::dispatcher::FeishuSender::new(
        account_id.to_owned(),
        account.app_id.clone(),
        account.app_secret.clone(),
        api_base.to_owned(),
        false,
    );
    sender
        .fetch_app_owner_open_id()
        .await
        .map_err(
            |error| warn!(account_id, error = %error, "failed to fetch feishu app owner open_id"),
        )
        .ok()
        .flatten()
}

async fn resolve_telegram_main_endpoint(
    account_id: &str,
    account: &TelegramAccount,
    endpoints: &[KnownChannelEndpoint],
) -> Option<PluginMainEndpoint> {
    if let Some((delivery_target_type, delivery_target_id)) =
        normalized_owner_target("telegram", account.owner_target.as_ref())
    {
        return Some(attach_known_endpoint_metadata(
            synthetic_main_endpoint(
                "telegram",
                account_id,
                &delivery_target_id,
                &delivery_target_id,
                &delivery_target_type,
                &delivery_target_id,
                account.workspace_dir.as_deref(),
                "owner_target",
            ),
            endpoints,
        ));
    }

    if let Some(endpoint) = best_private_endpoint(endpoints, "telegram", account_id) {
        return Some(endpoint);
    }

    None
}

async fn resolve_feishu_main_endpoint(
    account_id: &str,
    account: &FeishuAccount,
    endpoints: &[KnownChannelEndpoint],
) -> Option<PluginMainEndpoint> {
    if let Some((delivery_target_type, delivery_target_id)) =
        normalized_owner_target("feishu", account.owner_target.as_ref())
    {
        return Some(attach_known_endpoint_metadata(
            synthetic_main_endpoint(
                "feishu",
                account_id,
                &delivery_target_id,
                &delivery_target_id,
                &delivery_target_type,
                &delivery_target_id,
                account.workspace_dir.as_deref(),
                "owner_target",
            ),
            endpoints,
        ));
    }

    if let Some(endpoint) = best_private_endpoint(endpoints, "feishu", account_id) {
        return Some(endpoint);
    }

    let owner_open_id = resolve_feishu_owner_open_id(account_id, account).await?;
    Some(attach_known_endpoint_metadata(
        synthetic_main_endpoint(
            "feishu",
            account_id,
            &owner_open_id,
            &owner_open_id,
            DELIVERY_TARGET_TYPE_OPEN_ID,
            &owner_open_id,
            account.workspace_dir.as_deref(),
            "app_owner",
        ),
        endpoints,
    ))
}

fn endpoint_snapshot_activity(endpoint: &PluginConversationEndpoint) -> Option<&str> {
    [
        endpoint.last_inbound_at.as_deref(),
        endpoint.last_delivery_at.as_deref(),
        endpoint.thread_updated_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .max()
}

fn conversation_kind_rank(kind: &str) -> u8 {
    match kind {
        "private" => 0,
        "group" => 1,
        "topic" => 2,
        _ => 3,
    }
}

fn conversation_node_id(endpoint: &PluginConversationEndpoint, kind: &str) -> String {
    if kind == "topic" && endpoint.delivery_thread_id.as_deref().is_some() {
        return format!(
            "{}:{}:{}:{}",
            endpoint.channel,
            endpoint.account_id,
            endpoint.chat_id,
            endpoint.delivery_thread_id.as_deref().unwrap_or_default()
        );
    }
    if kind == "group" {
        return format!(
            "{}:{}:{}",
            endpoint.channel, endpoint.account_id, endpoint.chat_id
        );
    }
    endpoint.endpoint_key.clone()
}

fn build_builtin_account_ui(
    root_behavior: AccountRootBehavior,
    channel: &str,
    account_id: &str,
    endpoints: &[PluginConversationEndpoint],
) -> PluginAccountUi {
    let mut account_endpoints: Vec<&PluginConversationEndpoint> = endpoints
        .iter()
        .filter(|endpoint| endpoint.channel == channel && endpoint.account_id == account_id)
        .collect();

    account_endpoints.sort_by(|left, right| {
        right
            .thread_id
            .is_some()
            .cmp(&left.thread_id.is_some())
            .then_with(|| {
                conversation_kind_rank(left.conversation_kind.as_deref().unwrap_or("unknown")).cmp(
                    &conversation_kind_rank(
                        right.conversation_kind.as_deref().unwrap_or("unknown"),
                    ),
                )
            })
            .then_with(|| {
                endpoint_snapshot_activity(right)
                    .unwrap_or("")
                    .cmp(endpoint_snapshot_activity(left).unwrap_or(""))
            })
            .then_with(|| left.endpoint_key.cmp(&right.endpoint_key))
    });

    let default_open_endpoint_key = if matches!(root_behavior, AccountRootBehavior::ExpandOnly) {
        None
    } else {
        account_endpoints
            .first()
            .map(|endpoint| endpoint.endpoint_key.clone())
    };

    let mut deduped = BTreeMap::<String, PluginConversationNode>::new();
    for endpoint in account_endpoints {
        let kind = endpoint
            .conversation_kind
            .as_deref()
            .unwrap_or("unknown")
            .to_owned();
        if kind != "group" && kind != "topic" {
            continue;
        }
        if endpoint.thread_id.is_none() {
            continue;
        }
        let node_id = conversation_node_id(endpoint, &kind);
        if deduped.contains_key(&node_id) {
            continue;
        }
        deduped.insert(
            node_id.clone(),
            PluginConversationNode {
                id: node_id,
                endpoint_key: endpoint.endpoint_key.clone(),
                kind: kind.clone(),
                title: endpoint
                    .conversation_label
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| endpoint.display_label.trim())
                    .to_owned(),
                badge: Some(if kind == "topic" { "Topic" } else { "Group" }.to_owned()),
                latest_activity: endpoint_snapshot_activity(endpoint).map(ToOwned::to_owned),
                openable: endpoint.thread_id.is_some(),
            },
        );
    }

    PluginAccountUi {
        default_open_endpoint_key,
        conversation_nodes: deduped.into_values().collect(),
    }
}

pub async fn resolve_main_endpoint_from_channels_config(
    channels: &ChannelsConfig,
    channel: &str,
    account_id: &str,
    endpoints: &[KnownChannelEndpoint],
) -> Option<PluginMainEndpoint> {
    match channel {
        "telegram" => {
            let config = channels.resolved_telegram_config().ok()?;
            let account = config.accounts.get(account_id)?;
            resolve_telegram_main_endpoint(account_id, account, endpoints).await
        }
        "feishu" => {
            let config = channels.resolved_feishu_config().ok()?;
            let account = config.accounts.get(account_id)?;
            resolve_feishu_main_endpoint(account_id, account, endpoints).await
        }
        "weixin" => {
            let config = channels.resolved_weixin_config().ok()?;
            config.accounts.get(account_id)?;
            best_private_endpoint(endpoints, "weixin", account_id)
        }
        _ => None,
    }
}

pub fn resolve_account_ui_from_channels_config(
    channel: &str,
    account_id: &str,
    endpoints: &[PluginConversationEndpoint],
) -> Option<PluginAccountUi> {
    match channel {
        "telegram" | "feishu" | "weixin" => Some(build_builtin_account_ui(
            AccountRootBehavior::OpenDefault,
            channel,
            account_id,
            endpoints,
        )),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum PluginRegistryError {
    #[error("duplicate plugin id: {0}")]
    DuplicateId(String),
    #[error("duplicate plugin alias `{alias}` (existing: {existing_id}, new: {new_id})")]
    DuplicateAlias {
        alias: String,
        existing_id: String,
        new_id: String,
    },
}

#[derive(Default)]
pub struct PluginRegistry {
    ids: HashSet<String>,
    alias_to_id: HashMap<String, String>,
}

impl PluginRegistry {
    pub fn register(&mut self, metadata: &PluginMetadata) -> Result<(), PluginRegistryError> {
        if !self.ids.insert(metadata.id.clone()) {
            return Err(PluginRegistryError::DuplicateId(metadata.id.clone()));
        }

        // Registering multiple aliases is transactional: if alias
        // N+1 collides, we undo aliases 1..=N AND the id insert so the
        // registry is left exactly as it was. Without this, a partial
        // commit would leak ghost aliases that a subsequent unregister
        // can't find (they point at the failed id).
        //
        // Record the alias in `inserted` BEFORE mutating `alias_to_id`
        // so a panic in the intervening `insert` (theoretically OOM)
        // does not leave the registry with an alias that rollback
        // wouldn't know about.
        let mut inserted: Vec<String> = Vec::new();
        for alias in &metadata.aliases {
            if let Some(existing) = self.alias_to_id.get(alias) {
                let existing = existing.clone();
                for ghost in &inserted {
                    self.alias_to_id.remove(ghost);
                }
                self.ids.remove(&metadata.id);
                return Err(PluginRegistryError::DuplicateAlias {
                    alias: alias.clone(),
                    existing_id: existing,
                    new_id: metadata.id.clone(),
                });
            }
            inserted.push(alias.clone());
            self.alias_to_id.insert(alias.clone(), metadata.id.clone());
        }
        Ok(())
    }

    /// Release a previously-registered id and its aliases. Used by
    /// `register_subprocess_plugin` to undo the registry claim when a
    /// child's spawn/handshake fails — otherwise a retry under the
    /// same id would trip `DuplicateId`.
    pub fn unregister(&mut self, metadata: &PluginMetadata) {
        self.ids.remove(&metadata.id);
        for alias in &metadata.aliases {
            if self
                .alias_to_id
                .get(alias)
                .is_some_and(|owner| owner == &metadata.id)
            {
                self.alias_to_id.remove(alias);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

type StateHook = Arc<dyn Fn(PluginStatus) + Send + Sync + 'static>;

struct PluginEntry {
    /// `Arc` (not `Box`) so the manager can hand out clones to
    /// gateway HTTP handlers without holding its internal mutex for
    /// the duration of a slow RPC. Both built-in and subprocess
    /// plugins go through the same storage; the trait methods all
    /// take `&self` so `Arc<dyn ChannelPlugin>` is enough for every
    /// operation the trait exposes.
    plugin: Arc<dyn ChannelPlugin>,
    state: PluginState,
    last_error: Option<String>,
    /// Present only when the plugin is hosted by a subprocess — the
    /// raw child handle + manifest + spawn options the respawn path
    /// needs. Invisible through the `ChannelPlugin` trait so trait
    /// callers stay blind to the host model.
    subprocess: Option<SubprocessSideState>,
}

/// Manager-owned state for subprocess plugins that is NOT part of
/// the `ChannelPlugin` trait surface. Used by the specialized
/// `register_subprocess_plugin` / `respawn_plugin` paths. A plugin
/// whose entry has `subprocess: None` is a built-in.
struct SubprocessSideState {
    manifest: PluginManifest,
    spawn_options: SpawnOptions,
    handler: Arc<dyn InboundHandler>,
    host: HostContext,
    accounts: Vec<AccountDescriptor>,
    /// Live child. `None` only briefly while respawn moves it into
    /// the shutdown task; outside of that window it is always
    /// `Some`.
    plugin: Option<SubprocessPlugin>,
}

// ---------------------------------------------------------------------------
// Subprocess plugin integration (Scheme B)
// ---------------------------------------------------------------------------

/// §11.1 host-enforced deadline for lifecycle RPCs (`initialize`,
/// `start`, `stop`). Kept separate from the dispatch timeout because
/// lifecycle calls must fail fast when a child is wedged on boot; a
/// 30s wait there would block respawn for every misbehaving plugin.
pub const LIFECYCLE_RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Poll interval used by the §9.4 drain loop. The loop waits up to
/// `stop_grace_ms` for `pending_count()` to fall to zero after the OLD
/// plugin's `stop` RPC returns. 25 ms is short enough that the drain
/// completes within one tick for the common case where `stop` also
/// drains the outbound queue, and long enough that we don't spin the
/// runtime scheduler.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Errors surfaced by subprocess-plugin lifecycle operations
/// (`register_subprocess_plugin`, `respawn_plugin`). Separate from
/// `PluginRegistryError` so callers can distinguish "bad registration"
/// from "child misbehaved" without string-matching.
#[derive(Debug, Error)]
pub enum SubprocessPluginError {
    #[error("dispatcher not attached; call attach_dispatcher before subprocess plugins")]
    DispatcherNotAttached,
    #[error("unknown subprocess plugin: {0}")]
    UnknownPlugin(String),
    #[error(transparent)]
    Registry(#[from] PluginRegistryError),
    #[error("subprocess spawn failed: {0}")]
    Spawn(#[from] SubprocessError),
    #[error("plugin '{plugin_id}' lifecycle rpc `{method}` failed: {source}")]
    LifecycleRpc {
        plugin_id: String,
        method: String,
        #[source]
        source: RpcError,
    },
    #[error("plugin '{plugin_id}' rejected initialize: {message}")]
    InitializeRejected { plugin_id: String, message: String },
    #[error(transparent)]
    Dispatcher(#[from] ChannelError),
}

/// Adapter that lets us spawn a [`SubprocessPlugin`] with an
/// [`Arc<dyn InboundHandler>`] even though the primitive's generic
/// bound is `H: InboundHandler` (Sized).
///
/// The manager owns the caller's handler as a trait object so it can
/// be re-used verbatim across respawns. `SubprocessPlugin::spawn` wants
/// a concrete `Arc<H>`, so we wrap the trait object in this adapter
/// and pass the adapter itself. One additional virtual call per
/// inbound request / notification.
struct DynHandler(Arc<dyn InboundHandler>);

#[async_trait]
impl InboundHandler for DynHandler {
    async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)> {
        self.0.on_request(method, params).await
    }

    async fn on_notification(&self, method: String, params: Value) {
        self.0.on_notification(method, params).await
    }
}

/// Read-only snapshot of a subprocess plugin's catalog-visible
/// metadata — everything the desktop UI or a `GET /api/channels/plugins`
/// consumer needs to render a schema-driven account configuration
/// form (§11) without leaking manager-internal state (the live
/// `SubprocessPlugin`, the `InboundHandler`, etc.).
#[derive(Debug, Clone, Serialize)]
pub struct SubprocessPluginCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub state: PluginState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub capabilities: crate::plugin_host::ManifestCapabilities,
    /// JSON Schema for a single account's config. Rendered by the UI
    /// as a dynamic form. Shape matches the plugin's own
    /// `describe.schema` response.
    pub schema: serde_json::Value,
    pub auth_flows: Vec<crate::plugin_host::AuthFlowDescriptor>,
    /// Configuration methods the UI should render for this plugin
    /// (schema form + optional auto-login button, etc.). Kept
    /// alongside `auth_flows` because the two describe different
    /// dimensions: `auth_flows` is the manifest-declared list of
    /// automated login drivers, `config_methods` is the UI-level
    /// list of configuration blocks to render. A plugin can declare
    /// `[Form, AutoLogin]` here and still only advertise one
    /// `AuthFlowDescriptor`.
    #[serde(default)]
    pub config_methods: Vec<crate::auth_flow::ConfigMethod>,
    /// Currently-configured accounts (id + enabled flag only — the
    /// full `config` is a user-provided secret we don't leak to the
    /// UI unless the user asks for it).
    pub accounts: Vec<AccountDescriptor>,
    /// Inline `data:` URL for the plugin's brand icon, read directly
    /// from the icon file on disk (the file the plugin shipped and
    /// `garyx plugins install` copied next to the binary). The
    /// desktop UI can bind this to `<img src={icon_data_url}>` with
    /// no extra HTTP hop. `None` when the plugin doesn't ship an
    /// icon or the file is unreadable — the UI should fall back to
    /// a generic logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_data_url: Option<String>,
    #[serde(default)]
    pub account_root_behavior: crate::plugin_host::AccountRootBehavior,
}

impl SubprocessPluginCatalogEntry {
    /// Clamp each account config to the plugin-declared schema before it is
    /// exposed through the catalog. The schema is treated as a closed UI model:
    /// only `properties` are surfaced unless the plugin explicitly declares
    /// `additionalProperties`.
    pub fn project_account_configs_through_schema(&mut self) {
        for account in &mut self.accounts {
            account.config = project_value_through_schema(&account.config, &self.schema);
        }
    }
}

fn project_value_through_schema(value: &Value, schema: &Value) -> Value {
    if schema_object_like(schema) {
        return project_object_through_schema(value, schema);
    }

    if schema.get("type").and_then(Value::as_str) == Some("array") {
        let Some(items_schema) = schema.get("items") else {
            return value.clone();
        };
        let Some(values) = value.as_array() else {
            return value.clone();
        };
        return Value::Array(
            values
                .iter()
                .map(|item| project_value_through_schema(item, items_schema))
                .collect(),
        );
    }

    value.clone()
}

fn schema_object_like(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
        || schema.get("properties").is_some()
        || schema.get("additionalProperties").is_some()
}

fn project_object_through_schema(value: &Value, schema: &Value) -> Value {
    let Some(input) = value.as_object() else {
        return Value::Object(serde_json::Map::new());
    };

    let properties = schema.get("properties").and_then(Value::as_object);
    let mut output = serde_json::Map::new();
    if let Some(properties) = properties {
        for (key, field_schema) in properties {
            if let Some(field_value) = input.get(key) {
                output.insert(
                    key.clone(),
                    project_value_through_schema(field_value, field_schema),
                );
            }
        }
    }

    match schema.get("additionalProperties") {
        Some(Value::Bool(true)) => {
            for (key, field_value) in input {
                if properties.is_some_and(|props| props.contains_key(key)) {
                    continue;
                }
                output.insert(key.clone(), field_value.clone());
            }
        }
        Some(additional_schema) if additional_schema.is_object() => {
            for (key, field_value) in input {
                if properties.is_some_and(|props| props.contains_key(key)) {
                    continue;
                }
                output.insert(
                    key.clone(),
                    project_value_through_schema(field_value, additional_schema),
                );
            }
        }
        _ => {}
    }

    Value::Object(output)
}

pub struct ChannelPluginManager {
    registry: PluginRegistry,
    /// Unified map holding every plugin — built-in and subprocess —
    /// keyed by canonical id. Subprocess-specific state (manifest,
    /// handler, raw child handle) lives in `PluginEntry.subprocess`
    /// so the respawn path can reach it; callers that only need the
    /// `ChannelPlugin` trait surface ignore that field.
    plugins: BTreeMap<String, PluginEntry>,
    hooks: Vec<StateHook>,
    /// Shared dispatcher the respawn path rewires into. `None` when
    /// the manager is running only in-process plugins (tests + the
    /// current production path until §11.3 flips the flag).
    dispatcher: Option<Arc<SwappableDispatcher>>,
}

impl Default for ChannelPluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelPluginManager {
    pub fn new() -> Self {
        Self {
            registry: PluginRegistry::default(),
            plugins: BTreeMap::new(),
            hooks: Vec::new(),
            dispatcher: None,
        }
    }

    /// Bind a [`SwappableDispatcher`] to the manager so subprocess
    /// plugins can publish `PluginSenderHandle`s into it (initial
    /// registration and §9.4 respawn hot-swap).
    ///
    /// Safe to call once; subsequent calls replace the handle, which is
    /// useful in tests but not intended in production.
    pub fn attach_dispatcher(&mut self, dispatcher: Arc<SwappableDispatcher>) {
        self.dispatcher = Some(dispatcher);
    }

    pub fn register_state_hook<F>(&mut self, hook: F)
    where
        F: Fn(PluginStatus) + Send + Sync + 'static,
    {
        self.hooks.push(Arc::new(hook));
    }

    pub fn register_plugin(
        &mut self,
        plugin: Box<dyn ChannelPlugin>,
    ) -> Result<(), PluginRegistryError> {
        let metadata = plugin.metadata().clone();
        self.registry.register(&metadata)?;
        self.plugins.insert(
            metadata.id.clone(),
            PluginEntry {
                plugin: Arc::from(plugin),
                state: PluginState::Loaded,
                last_error: None,
                subprocess: None,
            },
        );
        self.emit_status(&metadata.id);
        Ok(())
    }

    pub fn discover_and_register(
        &mut self,
        discoverer: &dyn PluginDiscoverer,
    ) -> Result<(), String> {
        let discovered = discoverer.discover()?;
        for plugin in discovered {
            self.register_plugin(plugin).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub async fn initialize_all(&mut self) {
        let ids: Vec<String> = self.plugins.keys().cloned().collect();
        for id in ids {
            let _ = self.initialize_plugin(&id).await;
        }
    }

    pub async fn start_all(&mut self) {
        let ids: Vec<String> = self.plugins.keys().cloned().collect();
        for id in ids {
            if let Err(err) = self.start_plugin(&id).await {
                warn!(plugin_id = %id, error = %err, "plugin start failed");
            }
        }
    }

    pub async fn stop_all(&mut self) {
        let mut ids: Vec<String> = self.plugins.keys().cloned().collect();
        ids.reverse();
        for id in ids {
            if let Err(err) = self.stop_plugin(&id).await {
                warn!(plugin_id = %id, error = %err, "plugin stop failed");
            }
        }

        // Subprocess plugins own their own `shutdown_gracefully`
        // path (§6.3 consumes `SubprocessPlugin` by value). Drain
        // subprocess-hosted entries here so `stop_all` →
        // `cleanup_all` leaves no live children. We iterate the
        // unified map and only act on entries whose `subprocess`
        // side-state is `Some`.
        let subprocess_ids: Vec<String> = self
            .plugins
            .iter()
            .filter(|(_, entry)| entry.subprocess.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in subprocess_ids {
            if let Some(mut entry) = self.plugins.remove(&id) {
                let metadata = entry.plugin.metadata().clone();
                entry.state = PluginState::Stopped;
                self.registry.unregister(&metadata);
                // Unregister from the dispatcher as well so a stale
                // `PluginSenderHandle` isn't still routable after the
                // child is gone.
                if let Some(dispatcher) = &self.dispatcher {
                    let mut forked = (*dispatcher.load()).clone();
                    forked.unregister_plugin(&id);
                    dispatcher.store(Arc::new(forked));
                }
                if let Some(mut sub) = entry.subprocess.take()
                    && let Some(plugin) = sub.plugin.take()
                {
                    let _report = plugin.shutdown_gracefully().await;
                }
                // Emit the final status under the removed metadata
                // so observers see the transition, even though the
                // entry itself is gone from the map.
                let status = PluginStatus {
                    metadata,
                    state: entry.state,
                    last_error: entry.last_error.clone(),
                };
                for hook in &self.hooks {
                    hook(status.clone());
                }
            }
        }
    }

    pub async fn cleanup_all(&mut self) {
        let ids: Vec<String> = self.plugins.keys().cloned().collect();
        for id in ids {
            if let Some(entry) = self.plugins.get_mut(&id) {
                if let Err(err) = entry.plugin.cleanup().await {
                    entry.state = PluginState::Error;
                    entry.last_error = Some(err.clone());
                    warn!(plugin_id = %id, error = %err, "plugin cleanup failed");
                } else {
                    entry.state = PluginState::Stopped;
                    entry.last_error = None;
                }
            }
            self.emit_status(&id);
        }
        // Subprocess entries are drained inside `stop_all`; nothing
        // to clean up separately here. The invariant that callers can
        // rely on: after `stop_all` + `cleanup_all`, neither map holds
        // a live plugin.
    }

    pub async fn restart_plugin(&mut self, plugin_id: &str) -> Result<(), String> {
        self.stop_plugin(plugin_id).await?;
        self.start_plugin(plugin_id).await
    }

    pub fn statuses(&self) -> Vec<PluginStatus> {
        self.plugins
            .values()
            .map(|entry| PluginStatus {
                metadata: entry.plugin.metadata().clone(),
                state: entry.state,
                last_error: entry.last_error.clone(),
            })
            .collect()
    }

    /// Refresh every built-in plugin's per-account outbound sender
    /// table from a (possibly updated) `ChannelsConfig`. Called by
    /// the gateway's `apply_runtime_config` path after the user
    /// edits their channel config — `dispatch_outbound` picks up
    /// the new / edited accounts atomically (lock-free read via
    /// `ArcSwap`) without tearing down any Channel's polling loop.
    ///
    /// Subprocess plugins are left alone; their account set is
    /// driven by the manifest handshake + respawn path, not the
    /// host's `ChannelsConfig`.
    pub fn reload_builtin_senders(&self, channels: &ChannelsConfig) {
        let telegram = resolved_telegram_config(channels);
        let feishu = resolved_feishu_config(channels);
        let weixin = resolved_weixin_config(channels);
        for (id, entry) in &self.plugins {
            let Some(managed) = entry.plugin.as_managed() else {
                continue; // subprocess plugin — leave it alone
            };
            let new_map = match id.as_str() {
                "telegram" => build_telegram_senders(&telegram),
                "feishu" => build_feishu_senders(&feishu),
                "weixin" => build_weixin_senders(&weixin),
                _ => continue, // unknown built-in id — don't touch
            };
            managed.replace_outbound_senders(new_map);
        }
    }

    /// Push the fresh per-plugin account snapshot through every
    /// registered plugin's `ChannelPlugin::reload_accounts` method.
    /// Channel-blind: the manager iterates the unified plugin map
    /// and hands each plugin exactly the accounts the (new) config
    /// says belong to it. Built-ins (e.g. telegram) and subprocess
    /// plugins dispatch through the same trait method with no
    /// if-else at the call site.
    ///
    /// Per-plugin failures (e.g. a subprocess plugin returning
    /// `ConfigRejected` on a bad account config) are collected and
    /// returned so the caller can log / decide whether to abort the
    /// outer config apply. Successful plugins are committed; a
    /// partial failure leaves the manager in a mixed state (some
    /// plugins new, some old).
    ///
    /// Account-list derivation:
    ///   - `telegram` / `feishu` / `weixin` → each built-in reads
    ///     from `channels.{telegram,feishu,weixin}.accounts` and
    ///     maps to `AccountDescriptor { id, enabled, config }`
    ///     where `config` is the serialized account struct.
    ///   - Subprocess plugins → read from
    ///     `channels.plugins[plugin_id].accounts`.
    ///   - Unknown plugin id (tests, dead stubs) → passes an empty
    ///     list, plugin's default impl (`Ok(())`) no-ops.
    pub async fn reload_plugin_accounts(&self, channels: &ChannelsConfig) -> Vec<(String, String)> {
        let mut failures: Vec<(String, String)> = Vec::new();
        for (id, entry) in &self.plugins {
            let accounts = build_accounts_for_plugin(id, channels);
            if let Err(e) = entry.plugin.reload_accounts(accounts).await {
                failures.push((id.clone(), e));
            }
        }
        failures
    }

    /// Channel-blind lookup for the plugin's auto-login executor.
    /// Used by the gateway's `/api/channels/plugins/{id}/auth_flow/*`
    /// endpoints so the Mac App drives feishu (built-in) and any
    /// future subprocess plugin with the exact same code path.
    ///
    /// Resolution order:
    ///   1. In-process plugin map — delegates to
    ///      [`ChannelPlugin::auth_flow`]. Telegram returns `None`
    ///      (form-only); feishu / weixin return their respective
    ///      [`AuthFlowExecutor`] instances.
    ///   2. Subprocess plugin map — wraps the child's RPC client in
    ///      a [`crate::plugin_host::SubprocessAuthFlowExecutor`]
    ///      that forwards `start` / `poll` as
    ///      `auth_flow/start` / `auth_flow/poll` JSON-RPC calls.
    ///
    /// Returns `None` when the id is unknown or the plugin has no
    /// auto-login path. `id` accepts both canonical ids and
    /// aliases the registry knows about (e.g. `"lark"` → feishu).
    pub fn auth_flow_executor(
        &self,
        plugin_id: &str,
    ) -> Option<Arc<dyn crate::auth_flow::AuthFlowExecutor>> {
        let canonical = self
            .registry
            .alias_to_id
            .get(plugin_id)
            .cloned()
            .unwrap_or_else(|| plugin_id.to_owned());

        // Single unified lookup — the trait's `auth_flow()` method
        // returns the right executor regardless of built-in vs
        // subprocess. For built-ins: `ManagedChannelPlugin`'s stored
        // executor clone. For subprocess plugins:
        // `SubprocessChannelPlugin` constructs a fresh
        // `SubprocessAuthFlowExecutor` from its live RPC client.
        self.plugins.get(&canonical)?.plugin.auth_flow()
    }

    pub fn plugin(&self, plugin_id: &str) -> Option<Arc<dyn ChannelPlugin>> {
        let canonical = self
            .registry
            .alias_to_id
            .get(plugin_id)
            .cloned()
            .unwrap_or_else(|| plugin_id.to_owned());
        self.plugins
            .get(&canonical)
            .map(|entry| entry.plugin.clone())
    }

    /// Catalog of subprocess plugins with the full metadata the
    /// desktop UI needs for schema-driven channel configuration
    /// (§11): id, version, capabilities, account-config schema, and
    /// auth_flows. The in-tree `Box<dyn ChannelPlugin>` entries
    /// deliberately don't appear here — they don't have manifests.
    /// As built-in channels migrate to subprocess plugins they get
    /// added to this catalog for free.
    pub fn subprocess_plugin_catalog(&self) -> Vec<SubprocessPluginCatalogEntry> {
        self.plugins
            .values()
            .filter_map(|entry| {
                let sub = entry.subprocess.as_ref()?;
                let metadata = entry.plugin.metadata();
                let mut catalog_entry = SubprocessPluginCatalogEntry {
                    id: metadata.id.clone(),
                    display_name: metadata.display_name.clone(),
                    version: metadata.version.clone(),
                    description: metadata.description.clone(),
                    state: entry.state,
                    last_error: entry.last_error.clone(),
                    capabilities: sub.manifest.capabilities.clone(),
                    schema: sub.manifest.schema.clone(),
                    auth_flows: sub.manifest.auth_flows.clone(),
                    config_methods: metadata.config_methods.clone(),
                    accounts: sub.accounts.clone(),
                    // Inline the icon bytes as a data URL so the
                    // desktop renderer can bind it directly to
                    // `<img src={...}>`. Icons are typically 1-10 KB
                    // (SVG) or 20-50 KB (PNG) — negligible wire
                    // overhead, one fewer round trip, and the UI
                    // doesn't need to know the gateway base URL.
                    // Unreadable icon files yield `None` and the UI
                    // falls back to a generic logo.
                    icon_data_url: sub
                        .manifest
                        .plugin
                        .icon
                        .as_ref()
                        .and_then(|rel| resolve_plugin_icon_path(&sub.manifest.manifest_dir, rel))
                        .and_then(|path| read_icon_as_data_url(&path)),
                    account_root_behavior: sub.manifest.ui.account_root_behavior,
                };
                catalog_entry.project_account_configs_through_schema();
                Some(catalog_entry)
            })
            .collect()
    }

    /// Resolve the absolute on-disk path of a subprocess plugin's
    /// brand icon, if it ships one and the declared path stays
    /// inside the plugin's install dir (see
    /// [`resolve_plugin_icon_path`] for the containment rules).
    /// Useful for tools that want to copy / inspect the file
    /// without serialising it through the catalog. Returns `None`
    /// when the plugin doesn't have `plugin.icon`, the id is not a
    /// subprocess plugin, or the declared path would escape the
    /// install dir.
    pub fn subprocess_plugin_icon_path(&self, plugin_id: &str) -> Option<PathBuf> {
        let entry = self.plugins.get(plugin_id)?;
        let sub = entry.subprocess.as_ref()?;
        sub.manifest
            .plugin
            .icon
            .as_ref()
            .and_then(|rel| resolve_plugin_icon_path(&sub.manifest.manifest_dir, rel))
    }

    async fn initialize_plugin(&mut self, plugin_id: &str) -> Result<(), String> {
        {
            let entry = self
                .plugins
                .get_mut(plugin_id)
                .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
            entry.state = PluginState::Initializing;
            entry.last_error = None;
        }
        self.emit_status(plugin_id);

        let init_result = {
            let entry = self
                .plugins
                .get_mut(plugin_id)
                .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
            entry.plugin.initialize().await
        };

        match init_result {
            Ok(()) => {
                let entry = self
                    .plugins
                    .get_mut(plugin_id)
                    .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
                entry.state = PluginState::Ready;
                entry.last_error = None;
                self.emit_status(plugin_id);
                Ok(())
            }
            Err(err) => {
                let entry = self
                    .plugins
                    .get_mut(plugin_id)
                    .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
                entry.state = PluginState::Error;
                entry.last_error = Some(err.clone());
                self.emit_status(plugin_id);
                Err(err)
            }
        }
    }

    async fn start_plugin(&mut self, plugin_id: &str) -> Result<(), String> {
        let current_state = self
            .plugins
            .get(plugin_id)
            .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?
            .state;

        if matches!(current_state, PluginState::Loaded) {
            self.initialize_plugin(plugin_id).await?;
        }

        let start_result = {
            let entry = self
                .plugins
                .get_mut(plugin_id)
                .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
            entry.plugin.start().await
        };

        match start_result {
            Ok(()) => {
                let entry = self
                    .plugins
                    .get_mut(plugin_id)
                    .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
                entry.state = PluginState::Running;
                entry.last_error = None;
                info!(plugin_id = %plugin_id, "plugin started");
                self.emit_status(plugin_id);
                Ok(())
            }
            Err(err) => {
                let entry = self
                    .plugins
                    .get_mut(plugin_id)
                    .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
                entry.state = PluginState::Error;
                entry.last_error = Some(err.clone());
                self.emit_status(plugin_id);
                Err(err)
            }
        }
    }

    async fn stop_plugin(&mut self, plugin_id: &str) -> Result<(), String> {
        let stop_result = {
            let entry = self
                .plugins
                .get_mut(plugin_id)
                .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
            entry.plugin.stop().await
        };

        match stop_result {
            Ok(()) => {
                let entry = self
                    .plugins
                    .get_mut(plugin_id)
                    .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
                entry.state = PluginState::Stopped;
                entry.last_error = None;
                self.emit_status(plugin_id);
                Ok(())
            }
            Err(err) => {
                let entry = self
                    .plugins
                    .get_mut(plugin_id)
                    .ok_or_else(|| format!("unknown plugin: {plugin_id}"))?;
                entry.state = PluginState::Error;
                entry.last_error = Some(err.clone());
                self.emit_status(plugin_id);
                Err(err)
            }
        }
    }

    // -- Subprocess plugin lifecycle (Scheme B) -------------------------

    /// Spawn a subprocess-backed channel plugin, run the §6.2 handshake
    /// (`initialize` + `start`), and publish its [`PluginSenderHandle`]
    /// into the attached [`SwappableDispatcher`].
    ///
    /// On any lifecycle failure the freshly-spawned child is torn down
    /// via [`SubprocessPlugin::shutdown_gracefully`] before the error
    /// returns — there is no half-registered state.
    pub async fn register_subprocess_plugin(
        &mut self,
        manifest: PluginManifest,
        spawn_options: SpawnOptions,
        host: HostContext,
        accounts: Vec<AccountDescriptor>,
        handler: Arc<dyn InboundHandler>,
    ) -> Result<(), SubprocessPluginError> {
        let dispatcher = self
            .dispatcher
            .clone()
            .ok_or(SubprocessPluginError::DispatcherNotAttached)?;

        let plugin_id = manifest.plugin.id.clone();
        let metadata = manifest_to_metadata(&manifest);
        self.registry.register(&metadata)?;

        let (plugin, sender) = match Self::spawn_and_handshake(
            &manifest,
            spawn_options.clone(),
            &host,
            &accounts,
            handler.clone(),
        )
        .await
        {
            Ok(pair) => pair,
            Err(err) => {
                // §9.4 bookkeeping: an abandoned child cannot own the
                // id. Drop the registry claim before returning so a
                // retry under the same id succeeds.
                self.registry.unregister(&metadata);
                return Err(err);
            }
        };

        // Publish sender into the dispatcher before stashing the entry
        // so a racing `send_message` to the same id either sees the
        // fully-wired plugin or nothing at all — never a half-wired
        // state where the entry exists but no sender is registered.
        // If the publish fails (id collides with a reserved channel),
        // the spawned child is orphaned relative to the dispatcher —
        // roll back by tearing it down AND releasing the registry
        // claim, otherwise a retry under the same id would hit
        // `DuplicateId`.
        let forked = match dispatcher.load().fork_with_plugin_sender(sender.clone()) {
            Ok(forked) => forked,
            Err(err) => {
                let _ = plugin.shutdown_gracefully().await;
                self.registry.unregister(&metadata);
                return Err(err.into());
            }
        };
        dispatcher.store(Arc::new(forked));

        // Build the ChannelPlugin adapter first — this is what
        // trait-blind callers (gateway, auth_flow_executor, etc.)
        // see in the unified plugins map.
        let adapter = crate::plugin_host::SubprocessChannelPlugin::new(
            metadata.clone(),
            manifest.schema.clone(),
            manifest.capabilities.clone(),
            manifest.ui.account_root_behavior,
            sender,
            plugin.client(),
        );
        let adapter_arc: Arc<dyn ChannelPlugin> = Arc::new(adapter);

        self.plugins.insert(
            plugin_id.clone(),
            PluginEntry {
                plugin: adapter_arc,
                state: PluginState::Running,
                last_error: None,
                subprocess: Some(SubprocessSideState {
                    manifest,
                    spawn_options,
                    handler,
                    host,
                    accounts,
                    plugin: Some(plugin),
                }),
            },
        );
        self.emit_status(&plugin_id);
        info!(plugin_id = %plugin_id, "subprocess plugin registered");
        Ok(())
    }

    /// Respawn a subprocess plugin per §6.4 / §9.4.
    ///
    /// `new_accounts == None` preserves the currently-initialized set
    /// (used for crash recovery). `Some(v)` replaces it (used for
    /// account-config edits that force a reinit).
    ///
    /// Order of operations matches §9.4:
    /// 1. Spawn NEW child and run `initialize` + `start` (each bounded
    ///    by [`LIFECYCLE_RPC_TIMEOUT`]). A failure here leaves OLD
    ///    fully in place — the entry stays `Running` and the
    ///    dispatcher still routes to OLD.
    /// 2. Publish a forked dispatcher pointing at the NEW sender so
    ///    outbound traffic flips before OLD is touched. A publish
    ///    failure tears down NEW and leaves OLD untouched.
    /// 3. Swap the entry's `plugin` slot to NEW; OLD is moved into a
    ///    local.
    /// 4. Send `stop` RPC to OLD bounded by [`LIFECYCLE_RPC_TIMEOUT`]
    ///    (§11.1) and poll `pending_count()` for up to
    ///    `manifest.runtime.stop_grace_ms` (§9.4). Stragglers still
    ///    pending at grace expiry are aborted with the §9.4-mandated
    ///    [`ChannelError::Connection`]`("... outbound aborted")` via
    ///    [`PluginRpcClient::abort_pending`].
    /// 5. Call `shutdown_gracefully()` on OLD to run §6.3 escalation.
    pub async fn respawn_plugin(
        &mut self,
        plugin_id: &str,
        new_accounts: Option<Vec<AccountDescriptor>>,
    ) -> Result<(), SubprocessPluginError> {
        let dispatcher = self
            .dispatcher
            .clone()
            .ok_or(SubprocessPluginError::DispatcherNotAttached)?;

        // Snapshot of everything we need to rebuild the child. Holding
        // a short mutable borrow here only to verify the entry exists;
        // state is NOT flipped to Initializing because the spec says
        // OLD keeps serving until step 3 — advertising Initializing
        // would misrepresent the live state to observers.
        let (manifest, spawn_options, handler, host, accounts_to_use) = {
            let entry = self
                .plugins
                .get(plugin_id)
                .ok_or_else(|| SubprocessPluginError::UnknownPlugin(plugin_id.to_owned()))?;
            let sub = entry
                .subprocess
                .as_ref()
                .ok_or_else(|| SubprocessPluginError::UnknownPlugin(plugin_id.to_owned()))?;
            (
                sub.manifest.clone(),
                sub.spawn_options.clone(),
                sub.handler.clone(),
                sub.host.clone(),
                new_accounts.unwrap_or_else(|| sub.accounts.clone()),
            )
        };

        // Step 1: spawn NEW and finish the handshake. On failure OLD
        // is untouched, the dispatcher still points at it, and the
        // entry state is unchanged (Running). The error still carries
        // the plugin-reported context for the caller to log.
        let (new_plugin, new_sender) =
            Self::spawn_and_handshake(&manifest, spawn_options, &host, &accounts_to_use, handler)
                .await?;

        // Step 2: hot-swap dispatcher. Done before we touch OLD so any
        // new outbound traffic is already on the NEW plugin. If publish
        // fails (e.g. reserved-channel collision — normally impossible
        // for an already-registered id, but we defend anyway), tear
        // NEW down and leave OLD serving.
        let forked = match dispatcher
            .load()
            .fork_with_plugin_sender(new_sender.clone())
        {
            Ok(forked) => forked,
            Err(err) => {
                let _ = new_plugin.shutdown_gracefully().await;
                return Err(err.into());
            }
        };
        dispatcher.store(Arc::new(forked));

        // Step 3: swap the entry slot. OLD now lives in a local and
        // can be driven without holding a borrow on `self`. The entry
        // MUST still exist — it was checked above and only this method
        // mutates the map. We also rebuild the `SubprocessChannelPlugin`
        // adapter Arc with the new RPC client so trait-blind callers
        // see the new child on their next call — `replace_client` is
        // the cheaper alternative but the type-erased Arc would need
        // downcast to reach it, so building a fresh adapter is simpler.
        let stop_grace = Duration::from_millis(manifest.runtime.stop_grace_ms);
        let new_client = new_plugin.client();
        let new_metadata = self
            .plugins
            .get(plugin_id)
            .map(|e| e.plugin.metadata().clone())
            .ok_or_else(|| SubprocessPluginError::UnknownPlugin(plugin_id.to_owned()))?;
        let new_adapter: Arc<dyn ChannelPlugin> =
            Arc::new(crate::plugin_host::SubprocessChannelPlugin::new(
                new_metadata,
                manifest.schema.clone(),
                manifest.capabilities.clone(),
                manifest.ui.account_root_behavior,
                new_sender,
                new_client,
            ));
        let old_plugin = {
            let entry = self
                .plugins
                .get_mut(plugin_id)
                .ok_or_else(|| SubprocessPluginError::UnknownPlugin(plugin_id.to_owned()))?;
            let sub = entry
                .subprocess
                .as_mut()
                .ok_or_else(|| SubprocessPluginError::UnknownPlugin(plugin_id.to_owned()))?;
            let old = sub.plugin.take();
            sub.plugin = Some(new_plugin);
            sub.accounts = accounts_to_use;
            entry.plugin = new_adapter;
            entry.state = PluginState::Running;
            entry.last_error = None;
            old
        };
        self.emit_status(plugin_id);

        // Step 4 + 5: quiesce OLD. Failures here are logged but do not
        // fail the respawn — the NEW plugin is already serving traffic.
        if let Some(old) = old_plugin {
            let rpc = old.client();
            Self::quiesce_old_plugin(plugin_id, rpc, stop_grace).await;
            let _report = old.shutdown_gracefully().await;
        }

        info!(plugin_id = %plugin_id, "subprocess plugin respawned");
        Ok(())
    }

    /// Core spawn + handshake path. Used by both initial registration
    /// and respawn. On any error the child is torn down before the
    /// error returns.
    async fn spawn_and_handshake(
        manifest: &PluginManifest,
        spawn_options: SpawnOptions,
        host: &HostContext,
        accounts: &[AccountDescriptor],
        handler: Arc<dyn InboundHandler>,
    ) -> Result<(SubprocessPlugin, PluginSenderHandle), SubprocessPluginError> {
        let plugin_id = manifest.plugin.id.clone();
        let adapter = Arc::new(DynHandler(handler));
        let plugin = SubprocessPlugin::spawn(manifest, spawn_options, adapter)?;
        let rpc = plugin.client();

        let init_params = InitializeParams {
            protocol_version: crate::plugin_host::PROTOCOL_VERSION,
            host: host.clone(),
            accounts: accounts.to_vec(),
            dry_run: false,
        };
        let init_result: InitializeResult = match rpc
            .call_with_timeout("initialize", &init_params, Some(LIFECYCLE_RPC_TIMEOUT))
            .await
        {
            Ok(v) => v,
            Err(RpcError::Remote { code, message })
                if code == PluginErrorCode::ConfigRejected.as_i32() =>
            {
                // §5.3: `ConfigRejected` is the spec's lifetime-time
                // refusal code. Surface it as a dedicated variant so
                // callers (notably `garyx doctor` / UI) can distinguish
                // "bad config" from "plugin bug" without string-
                // matching. Teardown before returning so we never leak
                // a child.
                let _ = plugin.shutdown_gracefully().await;
                return Err(SubprocessPluginError::InitializeRejected { plugin_id, message });
            }
            Err(err) => {
                // Every other remote error (`MethodNotFound`,
                // `InvalidParams`, unknown code, …) is a plugin bug or
                // transport failure. Keep the full `RpcError` so
                // `source` preserves the wire-level code for logs.
                let _ = plugin.shutdown_gracefully().await;
                return Err(SubprocessPluginError::LifecycleRpc {
                    plugin_id,
                    method: "initialize".to_owned(),
                    source: err,
                });
            }
        };

        if let Err(err) = rpc
            .call_value_with_timeout("start", json!({}), Some(LIFECYCLE_RPC_TIMEOUT))
            .await
        {
            let _ = plugin.shutdown_gracefully().await;
            return Err(SubprocessPluginError::LifecycleRpc {
                plugin_id,
                method: "start".to_owned(),
                source: err,
            });
        }

        let sender =
            PluginSenderHandle::new(manifest.plugin.id.clone(), rpc, init_result.capabilities);
        Ok((plugin, sender))
    }

    /// §9.4 quiesce: send `stop` to OLD, wait up to `stop_grace` for
    /// `dispatch_outbound` waiters to drain, then abort stragglers
    /// with [`ChannelError::Connection`]`("... outbound aborted")`.
    ///
    /// Two separate budgets are in play per the spec:
    /// - The `stop` RPC itself is a §11.1 lifecycle RPC, bounded by
    ///   [`LIFECYCLE_RPC_TIMEOUT`] (10s). Manifest tuning of
    ///   `stop_grace_ms` does NOT extend it — a plugin that can't
    ///   acknowledge `stop` within 10s is wedged, and we drop to
    ///   escalation.
    /// - The drain window AFTER `stop` returns is
    ///   `manifest.runtime.stop_grace_ms` (§9.4). `pending_count()` is
    ///   polled inside this window; anything still pending at expiry
    ///   gets [`PluginRpcClient::abort_pending`] with the
    ///   spec-mandated error text, so those callers observe
    ///   Connection (retryable) instead of `Disconnected`'s generic
    ///   "plugin unavailable" after the SIGKILL path.
    ///
    /// Does not force the child down — that is
    /// `shutdown_gracefully`'s job. Best-effort throughout.
    async fn quiesce_old_plugin(plugin_id: &str, rpc: PluginRpcClient, stop_grace: Duration) {
        if let Err(err) = rpc
            .call_value_with_timeout("stop", json!({}), Some(LIFECYCLE_RPC_TIMEOUT))
            .await
        {
            warn!(
                plugin_id = %plugin_id,
                error = %err,
                "OLD plugin `stop` rpc did not complete; continuing drain",
            );
        }

        let drain_deadline = Instant::now() + stop_grace;
        while Instant::now() < drain_deadline {
            if rpc.pending_count() == 0 {
                return;
            }
            tokio::time::sleep(DRAIN_POLL_INTERVAL).await;
        }
        let remaining = rpc.pending_count();
        if remaining > 0 {
            warn!(
                plugin_id = %plugin_id,
                pending = remaining,
                "OLD plugin pending rpcs exceeded stop_grace; aborting per §9.4",
            );
            // §9.4 wording is normative: callers match on the exact
            // string. Do NOT wrap plugin_id in quotes.
            rpc.abort_pending(format!("plugin {plugin_id} respawning; outbound aborted"));
        }
    }

    fn emit_status(&self, plugin_id: &str) {
        let Some(entry) = self.plugins.get(plugin_id) else {
            return;
        };
        let status = PluginStatus {
            metadata: entry.plugin.metadata().clone(),
            state: entry.state,
            last_error: entry.last_error.clone(),
        };
        for hook in &self.hooks {
            hook(status.clone());
        }
    }
}

fn manifest_to_metadata(manifest: &PluginManifest) -> PluginMetadata {
    let display_name = if manifest.plugin.display_name.is_empty() {
        manifest.plugin.id.clone()
    } else {
        manifest.plugin.display_name.clone()
    };
    PluginMetadata {
        id: manifest.plugin.id.clone(),
        aliases: manifest.plugin.aliases.clone(),
        display_name,
        version: manifest.plugin.version.clone(),
        description: manifest.plugin.description.clone(),
        source: format!("subprocess:{}", manifest.manifest_dir.display()),
        // Every subprocess plugin exposes a JSON Schema via
        // `describe.schema`, so `Form` is always applicable. If the
        // manifest also declares at least one `[[auth_flows]]`, the
        // UI should additionally render an auto-login block. This
        // mirrors the information the manifest carries today without
        // requiring a manifest-schema extension.
        config_methods: {
            let mut m = vec![crate::auth_flow::ConfigMethod::Form];
            if !manifest.auth_flows.is_empty() {
                m.push(crate::auth_flow::ConfigMethod::AutoLogin);
            }
            m
        },
    }
}

/// Hard cap on a plugin icon's on-disk size. Icons ride inline
/// inside the JSON catalog payload — a rogue/misconfigured plugin
/// shipping a 100 MB PNG would otherwise bloat every
/// `GET /api/channels/plugins` call for every client. 1 MB is
/// generous for a branding asset (a retina-sized PNG is ~50 KB;
/// even a full macOS `.icns`-scale PNG rarely exceeds 200 KB) and
/// cheap to hold in memory during response generation.
const MAX_ICON_BYTES: u64 = 1_024 * 1_024;

/// Read an icon file from disk and package it as a `data:` URL so
/// it can ride inside the JSON catalog straight to the browser. The
/// media type is derived from the file extension because the user-
/// controlled install path guarantees one of the four sanctioned
/// suffixes (SVG > PNG > WebP > JPG).
///
/// Returns `None` on any of:
/// - filesystem error (missing / permissions / etc.),
/// - file size exceeds [`MAX_ICON_BYTES`] (logged loudly so an
///   operator can see why branding disappeared),
/// - unknown / unsupported extension.
///
/// A catalog builder that's already committed to returning a
/// response must not fail just because a plugin's branding file is
/// malformed; the UI falls back to its generic logo instead.
pub(crate) fn read_icon_as_data_url(path: &std::path::Path) -> Option<String> {
    use base64::Engine as _;
    let media = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        // Unknown / unsupported extension — refuse instead of
        // serving an unknown blob with a generic media type.
        _ => {
            tracing::warn!(
                path = %path.display(),
                "channel plugin icon has unsupported extension; falling back to generic logo"
            );
            return None;
        }
    };
    // Stat-before-read so we can bail on oversized files without
    // allocating gigabytes.
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_ICON_BYTES {
        tracing::warn!(
            path = %path.display(),
            size = metadata.len(),
            cap = MAX_ICON_BYTES,
            "channel plugin icon exceeds cap; falling back to generic logo"
        );
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{media};base64,{encoded}"))
}

// ---------------------------------------------------------------------------
// Built-in outbound sender builders
// ---------------------------------------------------------------------------
//
// Shared between `BuiltInPluginDiscoverer::discover` (initial
// registration) and `ChannelPluginManager::reload_builtin_senders`
// (runtime config updates). Single source of truth so adding a new
// built-in account doesn't leave the two paths out of sync.

/// Shape `channels.{telegram,feishu,weixin,…}.accounts` into the
/// wire-compatible
/// `Vec<AccountDescriptor>` the `ChannelPlugin::reload_accounts`
/// trait method takes. Channel-blind: the caller passes a plugin
/// id and gets back whatever accounts belong to it per the current
/// `ChannelsConfig`.
///
/// For built-ins the `config` field is the serialized account
/// struct (TelegramAccount / FeishuAccount / WeixinAccount);
/// `ManagedChannelPlugin::reload_accounts` deserialises it back.
/// For subprocess plugins the `config` field is the opaque JSON
/// from `channels.<id>.accounts[aid].config`; subprocess
/// handlers validate it against their own schema.
///
/// Returning an empty `Vec` is a valid outcome: an unknown
/// plugin id, or a known id whose accounts map is empty, gets a
/// no-op reload.
fn build_accounts_for_plugin(plugin_id: &str, channels: &ChannelsConfig) -> Vec<AccountDescriptor> {
    match plugin_id {
        "telegram" => resolved_telegram_config(channels)
            .accounts
            .iter()
            .map(|(id, acc)| AccountDescriptor {
                id: id.clone(),
                enabled: acc.enabled,
                config: serde_json::to_value(acc).unwrap_or(serde_json::Value::Null),
            })
            .collect(),
        "feishu" => resolved_feishu_config(channels)
            .accounts
            .iter()
            .map(|(id, acc)| AccountDescriptor {
                id: id.clone(),
                enabled: acc.enabled,
                config: serde_json::to_value(acc).unwrap_or(serde_json::Value::Null),
            })
            .collect(),
        "weixin" => resolved_weixin_config(channels)
            .accounts
            .iter()
            .map(|(id, acc)| AccountDescriptor {
                id: id.clone(),
                enabled: acc.enabled,
                config: serde_json::to_value(acc).unwrap_or(serde_json::Value::Null),
            })
            .collect(),
        other => channels
            .plugins
            .get(other)
            .map(|cfg| {
                cfg.accounts
                    .iter()
                    .map(|(id, entry)| AccountDescriptor {
                        id: id.clone(),
                        enabled: entry.enabled,
                        config: entry.config.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn resolved_telegram_config(channels: &ChannelsConfig) -> TelegramConfig {
    channels.resolved_telegram_config().unwrap_or_else(|error| {
        warn!(error = %error, "failed to resolve telegram accounts from channels config");
        TelegramConfig::default()
    })
}

fn resolved_feishu_config(channels: &ChannelsConfig) -> FeishuConfig {
    channels.resolved_feishu_config().unwrap_or_else(|error| {
        warn!(error = %error, "failed to resolve feishu accounts from channels config");
        FeishuConfig::default()
    })
}

fn resolved_weixin_config(channels: &ChannelsConfig) -> WeixinConfig {
    channels.resolved_weixin_config().unwrap_or_else(|error| {
        warn!(error = %error, "failed to resolve weixin accounts from channels config");
        WeixinConfig::default()
    })
}

fn build_telegram_senders(
    cfg: &garyx_models::config::TelegramConfig,
) -> HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>> {
    let mut out: HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>> = HashMap::new();
    for (id, acc) in &cfg.accounts {
        if !acc.enabled {
            continue;
        }
        let sender: Arc<dyn crate::dispatcher::OutboundSender> =
            Arc::new(crate::dispatcher::TelegramSender {
                account_id: id.clone(),
                token: acc.token.clone(),
                http: reqwest::Client::new(),
                api_base: "https://api.telegram.org".to_owned(),
                is_running: false,
            });
        out.insert(id.clone(), sender);
    }
    out
}

fn build_feishu_senders(
    cfg: &garyx_models::config::FeishuConfig,
) -> HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>> {
    let mut out: HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>> = HashMap::new();
    for (id, acc) in &cfg.accounts {
        if !acc.enabled {
            continue;
        }
        let api_base = match acc.domain {
            garyx_models::config::FeishuDomain::Feishu => "https://open.feishu.cn/open-apis",
            garyx_models::config::FeishuDomain::Lark => "https://open.larksuite.com/open-apis",
        };
        let sender: Arc<dyn crate::dispatcher::OutboundSender> =
            Arc::new(crate::dispatcher::FeishuSender::new(
                id.clone(),
                acc.app_id.clone(),
                acc.app_secret.clone(),
                api_base.to_owned(),
                false,
            ));
        out.insert(id.clone(), sender);
    }
    out
}

fn build_weixin_senders(
    cfg: &garyx_models::config::WeixinConfig,
) -> HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>> {
    let mut out: HashMap<String, Arc<dyn crate::dispatcher::OutboundSender>> = HashMap::new();
    for (id, acc) in &cfg.accounts {
        if !acc.enabled {
            continue;
        }
        let sender: Arc<dyn crate::dispatcher::OutboundSender> =
            Arc::new(crate::dispatcher::WeixinSender {
                account_id: id.clone(),
                account: acc.clone(),
                http: reqwest::Client::new(),
                is_running: false,
            });
        out.insert(id.clone(), sender);
    }
    out
}

/// Resolve a manifest-declared icon path (relative to the plugin's
/// install dir) and verify it doesn't escape that directory via
/// `..` segments or absolute-path tricks. Returns `None` when the
/// plugin declared an icon path that points outside its install
/// dir — a subtle but important security property given that
/// `manifest.toml` is user-editable.
///
/// We deliberately do NOT `canonicalize()` the result: an icon
/// symlink pointing outside the plugin dir is legitimate for the
/// operator's own tooling. Path-string containment is enough to
/// reject the obvious attacks (`../../../etc/passwd`,
/// `/etc/passwd`) without the i/o cost or TOCTOU seam of
/// canonicalization.
pub(crate) fn resolve_plugin_icon_path(
    manifest_dir: &std::path::Path,
    rel: &str,
) -> Option<PathBuf> {
    let candidate = std::path::Path::new(rel);
    // Reject absolute paths outright — manifest is meant to declare
    // bundled assets relative to itself.
    if candidate.is_absolute() {
        tracing::warn!(
            icon = %rel,
            "rejecting plugin icon at absolute path; must be relative to manifest dir"
        );
        return None;
    }
    // Reject any `..` segment: these are the only way to climb
    // above the manifest dir. Leading `./` is fine and canonical.
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        tracing::warn!(
            icon = %rel,
            "rejecting plugin icon with parent-dir traversal"
        );
        return None;
    }
    Some(manifest_dir.join(candidate))
}

// ---------------------------------------------------------------------------
// Discoverers
// ---------------------------------------------------------------------------

pub trait PluginDiscoverer {
    fn discover(&self) -> Result<Vec<Box<dyn ChannelPlugin>>, String>;
}

pub fn builtin_plugin_metadata(plugin_id: &str) -> Option<PluginMetadata> {
    crate::builtin_catalog::builtin_channel_descriptor(plugin_id).map(|descriptor| PluginMetadata {
        id: descriptor.id.to_owned(),
        aliases: descriptor
            .aliases
            .iter()
            .map(|alias| (*alias).to_owned())
            .collect(),
        display_name: descriptor.display_name.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        description: descriptor.description.to_owned(),
        source: "builtin".to_owned(),
        config_methods: descriptor.config_methods(),
    })
}

pub fn builtin_plugin_metadata_list() -> Vec<PluginMetadata> {
    crate::builtin_catalog::builtin_channel_descriptors()
        .iter()
        .map(|descriptor| PluginMetadata {
            id: descriptor.id.to_owned(),
            aliases: descriptor
                .aliases
                .iter()
                .map(|alias| (*alias).to_owned())
                .collect(),
            display_name: descriptor.display_name.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            description: descriptor.description.to_owned(),
            source: "builtin".to_owned(),
            config_methods: descriptor.config_methods(),
        })
        .collect()
}

pub struct BuiltInPluginDiscoverer {
    channels: ChannelsConfig,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    public_url: String,
}

impl BuiltInPluginDiscoverer {
    pub fn new(
        channels: ChannelsConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        public_url: String,
    ) -> Self {
        Self {
            channels,
            router,
            bridge,
            public_url,
        }
    }
}

fn builtin_auth_flow_executor(plugin_id: &str) -> Option<Arc<dyn AuthFlowExecutor>> {
    match crate::builtin_catalog::builtin_channel_descriptor(plugin_id)?.kind {
        crate::builtin_catalog::BuiltinChannelKind::Telegram => None,
        crate::builtin_catalog::BuiltinChannelKind::Feishu => {
            Some(Arc::new(crate::feishu::FeishuAuthExecutor::default()))
        }
        crate::builtin_catalog::BuiltinChannelKind::Weixin => {
            Some(Arc::new(crate::WeixinAuthExecutor::default()))
        }
    }
}

impl PluginDiscoverer for BuiltInPluginDiscoverer {
    fn discover(&self) -> Result<Vec<Box<dyn ChannelPlugin>>, String> {
        let mut plugins: Vec<Box<dyn ChannelPlugin>> = Vec::new();
        let telegram = resolved_telegram_config(&self.channels);
        let feishu = resolved_feishu_config(&self.channels);
        let weixin = resolved_weixin_config(&self.channels);

        // Built-ins are always registered, even with zero enabled accounts.
        // Auto-login needs Feishu/Weixin before the first account exists, and
        // desktop form saves need Telegram available for pre-save token
        // validation. Empty-account runtimes are inert: senders / poll loops
        // iterate over accounts and no-op when the map is empty.
        {
            let channel =
                TelegramChannel::new(telegram.clone(), self.router.clone(), self.bridge.clone());
            // Build one `TelegramSender` per enabled account so the
            // plugin's trait-level `dispatch_outbound` can route
            // without consulting the dispatcher's legacy map.
            let telegram_senders = build_telegram_senders(&telegram);
            let descriptor = crate::builtin_catalog::builtin_channel_descriptor("telegram")
                .expect("builtin telegram descriptor");
            plugins.push(Box::new(ManagedChannelPlugin::with_options(
                builtin_plugin_metadata("telegram").expect("builtin telegram metadata"),
                Box::new(channel),
                ManagedChannelPluginOptions {
                    capabilities: descriptor.capabilities(),
                    schema: descriptor.schema(),
                    auth_flow: builtin_auth_flow_executor(descriptor.id),
                    account_root_behavior: descriptor.account_root_behavior,
                    accounts: Some(build_accounts_for_plugin("telegram", &self.channels)),
                    outbound_senders: Some(telegram_senders),
                },
            )));
        }

        {
            let channel = FeishuChannel::new(
                feishu.clone(),
                self.router.clone(),
                self.bridge.clone(),
                self.public_url.clone(),
            );
            let feishu_senders = build_feishu_senders(&feishu);
            let descriptor = crate::builtin_catalog::builtin_channel_descriptor("feishu")
                .expect("builtin feishu descriptor");
            plugins.push(Box::new(ManagedChannelPlugin::with_options(
                builtin_plugin_metadata("feishu").expect("builtin feishu metadata"),
                Box::new(channel),
                ManagedChannelPluginOptions {
                    capabilities: descriptor.capabilities(),
                    schema: descriptor.schema(),
                    auth_flow: builtin_auth_flow_executor(descriptor.id),
                    account_root_behavior: descriptor.account_root_behavior,
                    accounts: Some(build_accounts_for_plugin("feishu", &self.channels)),
                    outbound_senders: Some(feishu_senders),
                },
            )));
        }

        {
            let channel =
                WeixinChannel::new(weixin.clone(), self.router.clone(), self.bridge.clone());
            let weixin_senders = build_weixin_senders(&weixin);
            let descriptor = crate::builtin_catalog::builtin_channel_descriptor("weixin")
                .expect("builtin weixin descriptor");
            plugins.push(Box::new(ManagedChannelPlugin::with_options(
                builtin_plugin_metadata("weixin").expect("builtin weixin metadata"),
                Box::new(channel),
                ManagedChannelPluginOptions {
                    capabilities: descriptor.capabilities(),
                    schema: descriptor.schema(),
                    auth_flow: builtin_auth_flow_executor(descriptor.id),
                    account_root_behavior: descriptor.account_root_behavior,
                    accounts: Some(build_accounts_for_plugin("weixin", &self.channels)),
                    outbound_senders: Some(weixin_senders),
                },
            )));
        }

        Ok(plugins)
    }
}

pub struct LocalDescriptorDiscoverer {
    pub descriptor_dir: Option<PathBuf>,
}

impl LocalDescriptorDiscoverer {
    pub fn from_env() -> Self {
        Self {
            descriptor_dir: std::env::var_os("GARYX_CHANNEL_PLUGIN_DIR").map(PathBuf::from),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LocalPluginDescriptor {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    /// Optional list of [`crate::auth_flow::ConfigMethod`] the plugin
    /// advertises. Absent ⇒ empty vec (legacy descriptors predating
    /// §11 keep parsing cleanly).
    #[serde(default)]
    config_methods: Vec<crate::auth_flow::ConfigMethod>,
}

struct LocalPlaceholderPlugin {
    metadata: PluginMetadata,
}

#[async_trait]
impl PluginLifecycle for LocalPlaceholderPlugin {
    async fn initialize(&self) -> Result<(), String> {
        Ok(())
    }

    async fn start(&self) -> Result<(), String> {
        Err("local plugin runtime is not implemented yet".to_owned())
    }

    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }

    async fn cleanup(&self) -> Result<(), String> {
        Ok(())
    }
}

impl ChannelPlugin for LocalPlaceholderPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }
}

impl PluginDiscoverer for LocalDescriptorDiscoverer {
    fn discover(&self) -> Result<Vec<Box<dyn ChannelPlugin>>, String> {
        let Some(dir) = &self.descriptor_dir else {
            return Ok(Vec::new());
        };
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut plugins: Vec<Box<dyn ChannelPlugin>> = Vec::new();
        let entries = fs::read_dir(dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let descriptor: LocalPluginDescriptor =
                serde_json::from_str(&content).map_err(|e| e.to_string())?;
            plugins.push(Box::new(LocalPlaceholderPlugin {
                metadata: PluginMetadata {
                    id: descriptor.id.clone(),
                    aliases: descriptor.aliases,
                    display_name: if descriptor.display_name.is_empty() {
                        descriptor.id
                    } else {
                        descriptor.display_name
                    },
                    version: if descriptor.version.is_empty() {
                        "0.0.0".to_owned()
                    } else {
                        descriptor.version
                    },
                    description: descriptor.description,
                    source: format!("local:{}", path.display()),
                    config_methods: descriptor.config_methods,
                },
            }));
        }
        Ok(plugins)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
