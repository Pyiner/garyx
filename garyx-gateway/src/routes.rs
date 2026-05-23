use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::Utc;
use garyx_channels::plugin::{PluginAccountUi, PluginConversationEndpoint, PluginMainEndpoint};
use garyx_models::config::ChannelsConfig;
#[cfg(test)]
use garyx_models::config::TelegramAccount;
use garyx_models::provider::ProviderType;
use garyx_models::routing::{DELIVERY_TARGET_TYPE_CHAT_ID, DELIVERY_TARGET_TYPE_OPEN_ID};
use garyx_router::{
    ChannelBinding, KnownChannelEndpoint, ThreadEnsureOptions, WorkspaceMode,
    active_run_snapshot_run_id, bindings_from_value, detach_endpoint_from_thread,
    history_message_count, is_hidden_thread_value, is_thread_key, list_known_channel_endpoints,
    thread_kind_from_value, update_thread_record, workspace_dir_from_value,
    workspace_git_status as router_workspace_git_status,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

use crate::agent_identity::create_thread_for_agent_reference;
use crate::garyx_db::{GaryxDbError, PinnedThreadRecord};
use crate::provider_session_locator::recover_local_provider_session;
use crate::server::AppState;
use crate::skills::SkillStoreError;
use crate::workspace_mode::worktree_base_dir_for_config;
#[cfg(test)]
use garyx_router::create_thread_record;

#[derive(Clone)]
pub(crate) struct ResolvedMainEndpoint {
    pub(crate) endpoint_key: String,
    pub(crate) channel: String,
    pub(crate) account_id: String,
    pub(crate) binding_key: String,
    pub(crate) chat_id: String,
    pub(crate) delivery_target_type: String,
    pub(crate) delivery_target_id: String,
    pub(crate) delivery_thread_id: Option<String>,
    pub(crate) display_label: String,
    pub(crate) thread_id: Option<String>,
    pub(crate) thread_label: Option<String>,
    pub(crate) workspace_dir: Option<String>,
    pub(crate) thread_updated_at: Option<String>,
    pub(crate) last_inbound_at: Option<String>,
    pub(crate) last_delivery_at: Option<String>,
    pub(crate) source: String,
}

impl ResolvedMainEndpoint {
    pub(crate) fn to_binding(&self) -> ChannelBinding {
        ChannelBinding {
            channel: self.channel.clone(),
            account_id: self.account_id.clone(),
            binding_key: self.binding_key.clone(),
            chat_id: self.chat_id.clone(),
            delivery_target_type: self.delivery_target_type.clone(),
            delivery_target_id: self.delivery_target_id.clone(),
            display_label: self.display_label.clone(),
            last_inbound_at: self.last_inbound_at.clone(),
            last_delivery_at: self.last_delivery_at.clone(),
        }
    }

    pub(crate) fn to_value(&self) -> Value {
        let conversation = resolved_main_endpoint_conversation_details(self);
        json!({
            "endpoint_key": self.endpoint_key,
            "channel": self.channel,
            "account_id": self.account_id,
            "binding_key": self.binding_key,
            "peer_id": self.binding_key,
            "chat_id": self.chat_id,
            "delivery_target_type": self.delivery_target_type,
            "delivery_target_id": self.delivery_target_id,
            "delivery_thread_id": self.delivery_thread_id,
            "thread_scope": self.delivery_thread_id,
            "display_label": self.display_label,
            "thread_id": self.thread_id,
            "thread_label": self.thread_label,
            "workspace_dir": self.workspace_dir,
            "thread_updated_at": self.thread_updated_at,
            "last_inbound_at": self.last_inbound_at,
            "last_delivery_at": self.last_delivery_at,
            "conversation_kind": conversation.kind,
            "conversation_label": conversation.label,
            "source": self.source,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelEndpointBindResult {
    pub(crate) thread_id: String,
    pub(crate) previous_thread_id: Option<String>,
    pub(crate) endpoint_key: String,
    pub(crate) binding: ChannelBinding,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelEndpointDetachResult {
    pub(crate) previous_thread_id: Option<String>,
    pub(crate) endpoint_key: String,
    pub(crate) binding: Option<ChannelBinding>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelEndpointMutationError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ChannelEndpointMutationError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<PluginMainEndpoint> for ResolvedMainEndpoint {
    fn from(value: PluginMainEndpoint) -> Self {
        Self {
            endpoint_key: value.endpoint_key,
            channel: value.channel,
            account_id: value.account_id,
            binding_key: value.binding_key,
            chat_id: value.chat_id,
            delivery_target_type: value.delivery_target_type,
            delivery_target_id: value.delivery_target_id,
            delivery_thread_id: value.delivery_thread_id,
            display_label: value.display_label,
            thread_id: value.thread_id,
            thread_label: value.thread_label,
            workspace_dir: value.workspace_dir,
            thread_updated_at: value.thread_updated_at,
            last_inbound_at: value.last_inbound_at,
            last_delivery_at: value.last_delivery_at,
            source: value.source,
        }
    }
}

pub(crate) fn binding_delivery_thread_id(binding_key: &str, chat_id: &str) -> Option<String> {
    let binding_key = binding_key.trim();
    let chat_id = chat_id.trim();
    if binding_key.is_empty() || binding_key == chat_id {
        None
    } else {
        Some(binding_key.to_owned())
    }
}

fn normalize_endpoint_lookup_key(endpoint_key: &str) -> String {
    let trimmed = endpoint_key.trim();
    let parts: Vec<&str> = trimmed.split("::").collect();
    if parts.len() >= 4 {
        format!("{}::{}::{}", parts[0], parts[1], parts[parts.len() - 1])
    } else {
        trimmed.to_owned()
    }
}

fn endpoint_key_matches(candidate: &str, requested: &str) -> bool {
    let requested = requested.trim();
    candidate == requested || candidate == normalize_endpoint_lookup_key(requested)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EndpointConversationDetails {
    kind: &'static str,
    label: String,
}

#[derive(Clone)]
struct ConfiguredChannelAccount {
    channel: String,
    account_id: String,
    enabled: bool,
    name: Option<String>,
    agent_id: Option<String>,
    workspace_dir: Option<String>,
    workspace_mode: Option<String>,
}

fn public_workspace_mode(value: Option<&str>) -> &'static str {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("worktree") => "worktree",
        _ => "local",
    }
}

fn configured_channel_accounts(channels: &ChannelsConfig) -> Vec<ConfiguredChannelAccount> {
    let mut accounts = Vec::new();
    for (plugin_id, plugin_cfg) in &channels.plugins {
        for (account_id, entry) in &plugin_cfg.accounts {
            accounts.push(ConfiguredChannelAccount {
                channel: plugin_id.clone(),
                account_id: account_id.clone(),
                enabled: entry.enabled,
                name: entry.name.clone(),
                agent_id: entry.agent_id.clone(),
                workspace_dir: entry.workspace_dir.clone(),
                workspace_mode: entry.workspace_mode.clone(),
            });
        }
    }
    accounts.sort_by(|left, right| {
        left.channel
            .cmp(&right.channel)
            .then_with(|| left.account_id.cmp(&right.account_id))
    });
    accounts
}

fn trimmed_nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
}

fn default_endpoint_conversation_label(endpoint: &garyx_router::KnownChannelEndpoint) -> String {
    trimmed_nonempty(Some(&endpoint.display_label))
        .or_else(|| trimmed_nonempty(endpoint.thread_label.as_deref()))
        .or_else(|| trimmed_nonempty(Some(&endpoint.chat_id)))
        .or_else(|| trimmed_nonempty(Some(&endpoint.binding_key)))
        .unwrap_or_else(|| "Conversation".to_owned())
}

fn endpoint_scope(endpoint: &garyx_router::KnownChannelEndpoint) -> Option<&str> {
    let binding_key = endpoint.binding_key.trim();
    let chat_id = endpoint.chat_id.trim();
    if binding_key.is_empty() || binding_key == chat_id {
        None
    } else {
        Some(binding_key)
    }
}

fn endpoint_is_topic(endpoint: &garyx_router::KnownChannelEndpoint) -> bool {
    let scope = endpoint_scope(endpoint);
    let chat_id = endpoint.chat_id.trim();
    matches!(scope, Some(value) if !chat_id.is_empty() && value != chat_id)
}

fn endpoint_conversation_details(
    endpoint: &garyx_router::KnownChannelEndpoint,
) -> EndpointConversationDetails {
    let fallback_label = default_endpoint_conversation_label(endpoint);

    let kind = if endpoint.channel == "discord" {
        let binding_key = endpoint.binding_key.trim();
        let chat_id = endpoint.chat_id.trim();
        if !binding_key.is_empty() && !chat_id.is_empty() && binding_key == chat_id {
            "group"
        } else {
            "private"
        }
    } else if endpoint.channel == "feishu" {
        if endpoint.delivery_target_type == DELIVERY_TARGET_TYPE_OPEN_ID {
            "private"
        } else if endpoint_is_topic(endpoint) {
            "topic"
        } else if endpoint.delivery_target_type == DELIVERY_TARGET_TYPE_CHAT_ID {
            "group"
        } else {
            "private"
        }
    } else if endpoint_is_topic(endpoint) {
        "topic"
    } else if endpoint_scope(endpoint).is_some() {
        "group"
    } else {
        let binding_key = endpoint.binding_key.trim();
        let chat_id = endpoint.chat_id.trim();
        if !binding_key.is_empty() && !chat_id.is_empty() && binding_key != chat_id {
            "group"
        } else {
            "private"
        }
    };

    EndpointConversationDetails {
        kind,
        label: fallback_label,
    }
}

fn resolved_main_endpoint_conversation_details(
    endpoint: &ResolvedMainEndpoint,
) -> EndpointConversationDetails {
    let kind = if endpoint.channel == "discord" {
        let binding_key = endpoint.binding_key.trim();
        let chat_id = endpoint.chat_id.trim();
        if !binding_key.is_empty() && !chat_id.is_empty() && binding_key == chat_id {
            "group"
        } else {
            "private"
        }
    } else if endpoint.delivery_thread_id.is_some() {
        "topic"
    } else {
        let binding_key = endpoint.binding_key.trim();
        let chat_id = endpoint.chat_id.trim();
        if !binding_key.is_empty() && !chat_id.is_empty() && binding_key != chat_id {
            "group"
        } else {
            "private"
        }
    };

    let label = trimmed_nonempty(Some(&endpoint.display_label))
        .or_else(|| trimmed_nonempty(endpoint.thread_label.as_deref()))
        .or_else(|| trimmed_nonempty(Some(&endpoint.chat_id)))
        .or_else(|| trimmed_nonempty(Some(&endpoint.binding_key)))
        .unwrap_or_else(|| "Conversation".to_owned());

    EndpointConversationDetails { kind, label }
}

fn channel_endpoint_response_value(endpoint: &garyx_router::KnownChannelEndpoint) -> Value {
    let conversation = endpoint_conversation_details(endpoint);
    json!({
        "endpoint_key": endpoint.endpoint_key,
        "channel": endpoint.channel,
        "account_id": endpoint.account_id,
        "binding_key": endpoint.binding_key,
        "peer_id": endpoint.binding_key,
        "chat_id": endpoint.chat_id,
        "delivery_target_type": endpoint.delivery_target_type,
        "delivery_target_id": endpoint.delivery_target_id,
        "delivery_thread_id": binding_delivery_thread_id(&endpoint.binding_key, &endpoint.chat_id),
        "thread_scope": binding_delivery_thread_id(&endpoint.binding_key, &endpoint.chat_id),
        "display_label": endpoint.display_label,
        "thread_id": endpoint.thread_id,
        "thread_label": endpoint.thread_label,
        "workspace_dir": endpoint.workspace_dir,
        "thread_updated_at": endpoint.thread_updated_at,
        "last_inbound_at": endpoint.last_inbound_at,
        "last_delivery_at": endpoint.last_delivery_at,
        "conversation_kind": conversation.kind,
        "conversation_label": conversation.label,
    })
}

fn sort_channel_endpoint_values_by_identity(items: &mut [Value]) {
    items.sort_by(|left, right| {
        left.get("display_label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("display_label")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
            .then_with(|| {
                left.get("endpoint_key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .cmp(
                        right
                            .get("endpoint_key")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )
            })
    });
}

fn plugin_conversation_endpoint_value(
    endpoint: &garyx_router::KnownChannelEndpoint,
) -> PluginConversationEndpoint {
    let conversation = endpoint_conversation_details(endpoint);
    PluginConversationEndpoint {
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
        conversation_kind: Some(conversation.kind.to_owned()),
        conversation_label: Some(conversation.label),
    }
}

fn bot_display_name(name: Option<&str>, account_id: &str) -> String {
    name.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
    .unwrap_or_else(|| account_id.to_owned())
}

fn bot_title(_channel: &str, account_id: &str, name: Option<&str>) -> String {
    bot_display_name(name, account_id)
}

fn bot_subtitle(channel_label: &str, account_id: &str) -> String {
    format!("{channel_label} Bot · {account_id}")
}

async fn channel_plugin_for(
    state: &Arc<AppState>,
    channel: &str,
) -> Option<Arc<dyn garyx_channels::plugin::ChannelPlugin>> {
    let manager = state.channel_plugin_manager();

    {
        let guard = manager.lock().await;
        guard.plugin(channel)
    }
}

fn account_root_behavior_value(
    behavior: garyx_channels::plugin_host::AccountRootBehavior,
) -> &'static str {
    match behavior {
        garyx_channels::plugin_host::AccountRootBehavior::OpenDefault => "open_default",
        garyx_channels::plugin_host::AccountRootBehavior::ExpandOnly => "expand_only",
    }
}

async fn channel_root_behavior(state: &Arc<AppState>, channel: &str) -> &'static str {
    channel_plugin_for(state, channel)
        .await
        .map(|plugin| account_root_behavior_value(plugin.account_root_behavior()))
        .unwrap_or("open_default")
}

async fn resolve_main_endpoint_with_endpoints(
    state: &Arc<AppState>,
    channel: &str,
    account_id: &str,
    endpoints: &[garyx_router::KnownChannelEndpoint],
) -> Option<ResolvedMainEndpoint> {
    let plugin = channel_plugin_for(state, channel).await?;
    plugin
        .resolve_main_endpoint(account_id, endpoints)
        .await
        .map(Into::into)
}

async fn resolve_account_ui_with_endpoints(
    state: &Arc<AppState>,
    channel: &str,
    account_id: &str,
    endpoints: &[garyx_router::KnownChannelEndpoint],
) -> Option<PluginAccountUi> {
    let plugin_endpoints: Vec<PluginConversationEndpoint> = endpoints
        .iter()
        .filter(|endpoint| endpoint.channel == channel && endpoint.account_id == account_id)
        .map(plugin_conversation_endpoint_value)
        .collect();

    let plugin = channel_plugin_for(state, channel).await?;
    plugin
        .resolve_account_ui(account_id, &plugin_endpoints)
        .await
}

fn resolve_default_open_endpoint_from_account_ui(
    account_ui: Option<&PluginAccountUi>,
    endpoints: &[garyx_router::KnownChannelEndpoint],
) -> Option<Value> {
    let endpoint_key = account_ui.and_then(|ui| ui.default_open_endpoint_key.as_deref())?;
    let endpoint = endpoints
        .iter()
        .find(|candidate| candidate.endpoint_key == endpoint_key)?;
    Some(channel_endpoint_response_value(endpoint))
}

fn conversation_nodes_from_account_ui(
    account_ui: Option<&PluginAccountUi>,
    endpoints: &[garyx_router::KnownChannelEndpoint],
) -> Option<Vec<Value>> {
    let account_ui = account_ui?;
    let endpoint_map: HashMap<&str, &garyx_router::KnownChannelEndpoint> = endpoints
        .iter()
        .map(|endpoint| (endpoint.endpoint_key.as_str(), endpoint))
        .collect();
    let mut nodes = Vec::new();
    for node in &account_ui.conversation_nodes {
        let Some(endpoint) = endpoint_map.get(node.endpoint_key.as_str()).copied() else {
            continue;
        };
        nodes.push(json!({
            "id": node.id,
            "endpoint": channel_endpoint_response_value(endpoint),
            "kind": node.kind,
            "title": node.title,
            "badge": node.badge,
            "latest_activity": node.latest_activity,
            "openable": node.openable,
        }));
    }
    Some(nodes)
}

pub(crate) async fn resolve_main_endpoint_by_bot(
    state: &Arc<AppState>,
    channel: &str,
    account_id: &str,
) -> Option<ResolvedMainEndpoint> {
    let endpoints = list_known_channel_endpoints(&state.threads.thread_store).await;
    resolve_main_endpoint_with_endpoints(state, channel, account_id, &endpoints).await
}

async fn resolve_main_endpoint_by_key(
    state: &Arc<AppState>,
    endpoint_key_value: &str,
) -> Option<ResolvedMainEndpoint> {
    let config = state.config_snapshot();
    let endpoints = list_known_channel_endpoints(&state.threads.thread_store).await;

    for account in configured_channel_accounts(&config.channels) {
        let Some(endpoint) = resolve_main_endpoint_with_endpoints(
            state,
            &account.channel,
            &account.account_id,
            &endpoints,
        )
        .await
        else {
            continue;
        };
        if endpoint.endpoint_key == endpoint_key_value {
            return Some(endpoint);
        }
    }

    None
}

/// GET /health - basic health check
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.runtime.start_time.elapsed().as_secs();
    Json(json!({
        "status": "ok",
        "uptime_seconds": uptime,
    }))
}

/// GET /health/detailed - comprehensive health report
pub async fn health_detailed(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let report = state.runtime.health_checker.run_checks().await;
    Json(serde_json::to_value(report).unwrap_or_default())
}

/// GET /runtime - service runtime information
pub async fn runtime_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cfg = state.config_snapshot();
    let uptime = state.runtime.start_time.elapsed().as_secs();
    Json(json!({
        "runtime": {
            "uptime_seconds": uptime,
            "version": env!("CARGO_PKG_VERSION"),
        },
        "gateway": {
            "host": cfg.gateway.host,
            "port": cfg.gateway.port,
        },
    }))
}

// ---------------------------------------------------------------------------
// GET /api/threads - list threads with pagination/filtering
// ---------------------------------------------------------------------------

const DEFAULT_THREAD_LIMIT: usize = 100;
const MAX_THREAD_LIMIT: usize = 1000;

#[derive(Deserialize)]
pub struct ListThreadsParams {
    /// Maximum number of threads to return.
    #[serde(default = "default_thread_limit")]
    pub limit: usize,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: usize,
    /// Optional prefix filter for thread ids.
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub include_hidden: bool,
}

#[derive(Deserialize)]
pub struct ThreadLogParams {
    #[serde(default)]
    pub cursor: Option<u64>,
}

fn default_thread_limit() -> usize {
    DEFAULT_THREAD_LIMIT
}

fn parse_sdk_session_provider_hint(value: Option<&str>) -> Result<Option<ProviderType>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    ProviderType::from_slug(&value.to_ascii_lowercase())
        .map(Some)
        .ok_or_else(|| {
            format!(
                "Unsupported sdkSessionProviderHint '{value}'. Use claude, codex, gemini, gpt, anthropic, or google."
            )
        })
}

fn provider_hint_label(value: &ProviderType) -> &'static str {
    match value {
        ProviderType::ClaudeCode => "Claude",
        ProviderType::CodexAppServer => "Codex",
        ProviderType::GeminiCli => "Gemini",
        ProviderType::Gpt => "GPT",
        ProviderType::ClaudeLlm => "Claude",
        ProviderType::GeminiLlm => "Gemini",
        ProviderType::AgentTeam => "Team",
    }
}

const IMPORTED_SESSION_SNAPSHOT_LIMIT: usize = 100;

async fn seed_imported_thread_history(
    state: &Arc<AppState>,
    thread_id: &str,
    thread_data: &mut Value,
    messages: &[Value],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    let append_result = state
        .threads
        .history
        .transcript_store()
        .rewrite_from_messages(thread_id, messages)
        .await
        .map_err(|error| format!("failed to import local provider session history: {error}"))?;

    let Some(object) = thread_data.as_object_mut() else {
        return Err(format!("thread payload is not an object: {thread_id}"));
    };

    let snapshot_start = messages
        .len()
        .saturating_sub(IMPORTED_SESSION_SNAPSHOT_LIMIT);
    object.insert(
        "messages".to_owned(),
        Value::Array(messages[snapshot_start..].to_vec()),
    );
    object.insert(
        "message_count".to_owned(),
        Value::Number(serde_json::Number::from(
            append_result.total_messages as u64,
        )),
    );

    let history = object
        .entry("history".to_owned())
        .or_insert_with(|| json!({}));
    if !history.is_object() {
        *history = json!({});
    }
    let history_object = history.as_object_mut().expect("history must be object");

    history_object.insert(
        "source".to_owned(),
        Value::String("transcript_v1".to_owned()),
    );
    if let Some(path) = state
        .threads
        .history
        .transcript_store()
        .transcript_path(thread_id)
    {
        history_object.insert(
            "transcript_file".to_owned(),
            Value::String(path.display().to_string()),
        );
    }
    history_object.insert(
        "message_count".to_owned(),
        Value::Number(serde_json::Number::from(
            append_result.total_messages as u64,
        )),
    );
    history_object.insert(
        "snapshot_limit".to_owned(),
        Value::Number(serde_json::Number::from(
            garyx_router::DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT as u64,
        )),
    );
    history_object.insert(
        "snapshot_truncated".to_owned(),
        Value::Bool(
            append_result.total_messages > garyx_router::DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT,
        ),
    );
    match append_result.last_message_at {
        Some(last_message_at) if !last_message_at.trim().is_empty() => {
            history_object.insert("last_message_at".to_owned(), Value::String(last_message_at));
        }
        _ => {
            history_object.remove("last_message_at");
        }
    }
    history_object.insert(
        "recent_committed_run_ids".to_owned(),
        Value::Array(Vec::new()),
    );
    history_object.remove("active_run_snapshot");

    object.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
    );
    state
        .threads
        .thread_store
        .set(thread_id, thread_data.clone())
        .await;
    state
        .threads
        .history
        .enqueue_conversation_index_for_thread(thread_id);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThreadBody {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default, alias = "workspace_mode")]
    pub workspace_mode: WorkspaceMode,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
    /// Agent or team ID. Backend resolves whether it's a team or custom agent.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Optional provider-native session id to resume from on the first run.
    #[serde(default, alias = "sessionId")]
    pub sdk_session_id: Option<String>,
    /// Optional provider hint for sdkSessionId. Supported values: claude, codex, gemini.
    #[serde(default)]
    pub sdk_session_provider_hint: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitStatusParams {
    #[serde(default, alias = "workspace_dir")]
    pub workspace_dir: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadBody {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindChannelEndpointBody {
    pub endpoint_key: String,
    pub thread_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachChannelEndpointBody {
    pub endpoint_key: String,
}

#[derive(Deserialize)]
pub struct CreateSkillBody {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Deserialize)]
pub struct UpdateSkillBody {
    pub name: String,
    pub description: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileParams {
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteSkillFileBody {
    pub path: String,
    pub content: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSkillEntryBody {
    pub path: String,
    pub entry_type: String,
}

async fn ensure_existing_thread_id(state: &Arc<AppState>, key: &str) -> Option<String> {
    let trimmed = key.trim();
    if trimmed.is_empty() || !is_thread_key(trimmed) {
        return None;
    }
    if state.threads.thread_store.exists(trimmed).await {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

async fn rebuild_thread_indexes(state: &Arc<AppState>) {
    let mut router = state.threads.router.lock().await;
    router.rebuild_thread_indexes().await;
}

fn binding_from_known_endpoint(endpoint: &KnownChannelEndpoint) -> ChannelBinding {
    ChannelBinding {
        channel: endpoint.channel.clone(),
        account_id: endpoint.account_id.clone(),
        binding_key: endpoint.binding_key.clone(),
        chat_id: endpoint.chat_id.clone(),
        delivery_target_type: endpoint.delivery_target_type.clone(),
        delivery_target_id: endpoint.delivery_target_id.clone(),
        display_label: endpoint.display_label.clone(),
        last_inbound_at: endpoint.last_inbound_at.clone(),
        last_delivery_at: endpoint.last_delivery_at.clone(),
    }
}

async fn resolve_channel_binding_for_endpoint_key(
    state: &Arc<AppState>,
    requested_endpoint_key: &str,
) -> Option<ChannelBinding> {
    let endpoints = list_known_channel_endpoints(&state.threads.thread_store).await;
    if let Some(binding) = endpoints
        .into_iter()
        .find(|endpoint| endpoint_key_matches(&endpoint.endpoint_key, requested_endpoint_key))
        .map(|endpoint| binding_from_known_endpoint(&endpoint))
    {
        return Some(binding);
    }
    resolve_main_endpoint_by_key(state, requested_endpoint_key)
        .await
        .map(|endpoint| endpoint.to_binding())
}

pub(crate) async fn bind_channel_endpoint_key_to_thread(
    state: &Arc<AppState>,
    endpoint_key: &str,
    thread_id: &str,
) -> Result<ChannelEndpointBindResult, ChannelEndpointMutationError> {
    let requested_endpoint_key = normalize_endpoint_lookup_key(endpoint_key);
    let Some(thread_id) = ensure_existing_thread_id(state, thread_id).await else {
        return Err(ChannelEndpointMutationError::new(
            StatusCode::NOT_FOUND,
            "target thread not found",
        ));
    };

    let Some(binding) =
        resolve_channel_binding_for_endpoint_key(state, &requested_endpoint_key).await
    else {
        return Err(ChannelEndpointMutationError::new(
            StatusCode::NOT_FOUND,
            "endpoint not found",
        ));
    };

    let bind_result = {
        let mut router = state.threads.router.lock().await;
        router
            .bind_endpoint_runtime(&thread_id, binding.clone())
            .await
    };

    match bind_result {
        Ok(previous_thread_id) => {
            rebuild_thread_indexes(state).await;
            state.invalidate_gateway_sync_caches().await;
            Ok(ChannelEndpointBindResult {
                thread_id,
                previous_thread_id,
                endpoint_key: requested_endpoint_key,
                binding,
            })
        }
        Err(error) if error.contains("thread not found") => Err(ChannelEndpointMutationError::new(
            StatusCode::NOT_FOUND,
            error,
        )),
        Err(error) => Err(ChannelEndpointMutationError::new(
            StatusCode::BAD_REQUEST,
            error,
        )),
    }
}

pub(crate) async fn detach_channel_endpoint_key(
    state: &Arc<AppState>,
    endpoint_key: &str,
) -> Result<ChannelEndpointDetachResult, ChannelEndpointMutationError> {
    let requested_endpoint_key = normalize_endpoint_lookup_key(endpoint_key);
    match detach_endpoint_from_thread(&state.threads.thread_store, &requested_endpoint_key).await {
        Ok(previous_thread_id) => {
            let detached_endpoint = list_known_channel_endpoints(&state.threads.thread_store)
                .await
                .into_iter()
                .find(|endpoint| {
                    endpoint_key_matches(&endpoint.endpoint_key, &requested_endpoint_key)
                });
            if let (Some(thread_id), Some(endpoint)) =
                (previous_thread_id.as_deref(), detached_endpoint.as_ref())
            {
                let delivery_thread_id =
                    binding_delivery_thread_id(&endpoint.binding_key, &endpoint.chat_id);
                let mut router = state.threads.router.lock().await;
                router
                    .clear_reply_routing_for_chat_with_persistence(
                        thread_id,
                        &endpoint.channel,
                        &endpoint.account_id,
                        &endpoint.chat_id,
                        delivery_thread_id.as_deref(),
                    )
                    .await;
                router
                    .clear_last_delivery_for_chat_with_persistence(
                        thread_id,
                        &endpoint.channel,
                        &endpoint.account_id,
                        &endpoint.chat_id,
                        delivery_thread_id.as_deref(),
                    )
                    .await;
                router.rebuild_routing_index(&endpoint.channel).await;
            }
            rebuild_thread_indexes(state).await;
            state.invalidate_gateway_sync_caches().await;
            Ok(ChannelEndpointDetachResult {
                previous_thread_id,
                endpoint_key: requested_endpoint_key,
                binding: detached_endpoint.as_ref().map(binding_from_known_endpoint),
            })
        }
        Err(error) => Err(ChannelEndpointMutationError::new(
            StatusCode::BAD_REQUEST,
            error,
        )),
    }
}

fn summarize_text(value: Option<&str>, limit: usize) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= limit {
        return Some(text.to_owned());
    }
    Some(
        text.chars()
            .take(limit.saturating_sub(1))
            .collect::<String>()
            .trim_end()
            .to_owned()
            + "…",
    )
}

fn last_message_preview(data: &Value, role: &str) -> Option<String> {
    let messages = data.get("messages").and_then(Value::as_array)?;
    for message in messages.iter().rev() {
        let Some(obj) = message.as_object() else {
            continue;
        };
        if obj.get("role").and_then(Value::as_str) != Some(role) {
            continue;
        }
        let Some(content) = obj.get("content") else {
            continue;
        };
        let text = match content {
            Value::String(value) => Some(value.as_str()),
            _ => None,
        };
        if let Some(summary) = summarize_text(text, 160) {
            return Some(summary);
        }
    }
    None
}

fn thread_summary(thread_id: &str, data: &Value) -> Value {
    let message_count = history_message_count(data);
    let label = data.get("label").cloned().unwrap_or(Value::Null);
    let updated_at = data.get("updated_at").cloned().unwrap_or(Value::Null);
    let created_at = data.get("created_at").cloned().unwrap_or(Value::Null);
    let workspace_dir = workspace_dir_from_value(data)
        .map(Value::String)
        .unwrap_or(Value::Null);
    let channel_bindings = serde_json::to_value(bindings_from_value(data))
        .unwrap_or_else(|_| Value::Array(Vec::new()));
    let agent_id = data.get("agent_id").cloned().unwrap_or(Value::Null);
    let provider_type = data.get("provider_type").cloned().unwrap_or(Value::Null);
    let worktree = data.get("worktree").cloned().unwrap_or(Value::Null);
    let recent_run_id = data
        .get("history")
        .and_then(|history| history.get("recent_committed_run_ids"))
        .and_then(Value::as_array)
        .and_then(|entries| entries.last())
        .cloned()
        .unwrap_or(Value::Null);
    let active_run_id = active_run_snapshot_run_id(data)
        .map(Value::String)
        .unwrap_or(Value::Null);

    json!({
        "thread_id": thread_id,
        "thread_key": thread_id,
        "thread_type": thread_kind_from_value(data).unwrap_or_else(|| "chat".to_owned()),
        "label": label,
        "workspace_dir": workspace_dir,
        "channel_bindings": channel_bindings,
        "updated_at": updated_at,
        "created_at": created_at,
        "message_count": message_count,
        "last_user_message": last_message_preview(data, "user"),
        "last_assistant_message": last_message_preview(data, "assistant"),
        "agent_id": agent_id,
        "provider_type": provider_type,
        "worktree": worktree,
        "recent_run_id": recent_run_id,
        "active_run_id": active_run_id,
    })
}

fn thread_pin_ids(records: &[PinnedThreadRecord]) -> Vec<String> {
    records
        .iter()
        .map(|record| record.thread_id.clone())
        .collect()
}

fn thread_pins_payload(records: &[PinnedThreadRecord]) -> Value {
    let thread_ids = records
        .iter()
        .map(|record| Value::String(record.thread_id.clone()))
        .collect::<Vec<_>>();
    json!({
        "thread_ids": thread_ids,
        "pins": records,
    })
}

fn garyx_db_error_response(error: GaryxDbError) -> (StatusCode, Json<Value>) {
    let (status, code) = match &error {
        GaryxDbError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BadRequest"),
        GaryxDbError::LockPoisoned | GaryxDbError::Io(_) | GaryxDbError::Sqlite(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "InternalError")
        }
    };
    (
        status,
        Json(json!({
            "error": code,
            "message": error.to_string(),
        })),
    )
}

/// Build the read-only `team` block for a thread metadata response when the
/// thread's `agent_id` resolves to an AgentTeam. Returns `None` for
/// standalone-agent threads (including threads without an `agent_id`).
///
/// This is the projection the desktop client consumes to render team branding
/// and the per-sub-agent "peek" tabs. It is a pure projection of the Group
/// store's current state; no side effects.
pub(crate) async fn team_block_for_thread(
    state: &Arc<AppState>,
    thread_id: &str,
    data: &Value,
) -> Option<Value> {
    let agent_id = data
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    // `get_team` returns `None` for non-team agent_ids — that's the
    // standalone-agent case, and we emit nothing.
    let team = state.ops.agent_teams.get_team(agent_id).await?;
    let valid_members = team
        .member_agent_ids
        .iter()
        .cloned()
        .chain(std::iter::once(team.leader_agent_id.clone()))
        .collect::<std::collections::HashSet<_>>();

    // The Group only exists once the provider has dispatched at least one
    // turn; before that, `child_thread_ids` is simply empty.
    let child_thread_ids: serde_json::Map<String, Value> =
        match state.ops.agent_team_group_store.load(thread_id).await {
            Some(group) => group
                .child_threads
                .into_iter()
                .filter(|(agent, _)| valid_members.contains(agent))
                .map(|(agent, tid)| (agent, Value::String(tid)))
                .collect(),
            None => serde_json::Map::new(),
        };

    Some(json!({
        "team_id": team.team_id,
        "display_name": team.display_name,
        "leader_agent_id": team.leader_agent_id,
        "member_agent_ids": team.member_agent_ids,
        "child_thread_ids": Value::Object(child_thread_ids),
    }))
}

async fn thread_metadata_response(state: &Arc<AppState>, thread_id: &str, data: &Value) -> Value {
    let mut value = data.clone();
    // `get_thread` returns the thread object itself — nest `team` inside
    // alongside `thread_id` so the desktop client sees it as part of the
    // thread shape. `api::thread_history_for_key` uses a different envelope
    // and attaches `team` at the response root instead (see the comment
    // there). This asymmetry is intentional.
    let team_block = team_block_for_thread(state, thread_id, data).await;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("thread_mode");
        obj.entry("thread_id".to_owned())
            .or_insert_with(|| Value::String(thread_id.to_owned()));
        obj.entry("thread_key".to_owned())
            .or_insert_with(|| Value::String(thread_id.to_owned()));
        if let Some(block) = team_block {
            obj.insert("team".to_owned(), block);
        }
    }
    value
}

/// GET /api/threads - list threads with filtering and pagination.
pub async fn list_threads(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListThreadsParams>,
) -> impl IntoResponse {
    let entries = state.cached_thread_list_entries().await;
    let mut candidates = Vec::new();
    for entry in entries {
        if let Some(prefix) = params.prefix.as_deref()
            && !entry.key.starts_with(prefix)
        {
            continue;
        }
        let data = entry.data;
        if !params.include_hidden && is_hidden_thread_value(&data) {
            continue;
        }
        candidates.push((entry.key, data));
    }

    let total = candidates.len();
    let limit = params.limit.min(MAX_THREAD_LIMIT);
    let offset = params.offset.min(total);
    let page_candidates: Vec<(String, Value)> =
        candidates.into_iter().skip(offset).take(limit).collect();
    let mut page = Vec::with_capacity(page_candidates.len());
    for (key, data) in page_candidates {
        page.push(thread_summary(&key, &data));
    }
    let count = page.len();

    Json(json!({
        "threads": page,
        "count": count,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
}

/// GET /api/threads/:key - get thread metadata
pub async fn get_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let Some(thread_id) = ensure_existing_thread_id(&state, &key).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "thread not found"})),
        );
    };
    match state.threads.thread_store.get(&thread_id).await {
        Some(data) => (
            StatusCode::OK,
            Json(thread_metadata_response(&state, &thread_id, &data).await),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "thread not found"})),
        ),
    }
}

/// GET /api/thread-pins - list pinned thread ids in display order.
pub async fn list_thread_pins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.ops.garyx_db.list_pinned_threads() {
        Ok(records) => (StatusCode::OK, Json(thread_pins_payload(&records))).into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// PUT /api/thread-pins/:key - mark a thread as pinned.
pub async fn pin_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let Some(thread_id) = ensure_existing_thread_id(&state, &key).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"pinned": false, "error": "thread not found"})),
        )
            .into_response();
    };
    match state.ops.garyx_db.pin_thread(&thread_id) {
        Ok(record) => match state.ops.garyx_db.list_pinned_threads() {
            Ok(records) => (
                StatusCode::OK,
                Json(json!({
                    "pinned": true,
                    "pin": record,
                    "thread_ids": thread_pin_ids(&records),
                    "pins": records,
                })),
            )
                .into_response(),
            Err(error) => garyx_db_error_response(error).into_response(),
        },
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// DELETE /api/thread-pins/:key - remove a thread pin.
pub async fn unpin_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let thread_id = ensure_existing_thread_id(&state, &key)
        .await
        .unwrap_or_else(|| key.trim().to_owned());
    match state.ops.garyx_db.unpin_thread(&thread_id) {
        Ok(removed) => match state.ops.garyx_db.list_pinned_threads() {
            Ok(records) => (
                StatusCode::OK,
                Json(json!({
                    "pinned": false,
                    "removed": removed,
                    "thread_id": thread_id,
                    "thread_ids": thread_pin_ids(&records),
                    "pins": records,
                })),
            )
                .into_response(),
            Err(error) => garyx_db_error_response(error).into_response(),
        },
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/threads/:key/logs - get full or incremental thread log content
pub async fn get_thread_logs(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Query(params): Query<ThreadLogParams>,
) -> impl IntoResponse {
    let Some(thread_id) = ensure_existing_thread_id(&state, &key).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "thread not found"})),
        );
    };

    match state
        .ops
        .thread_logs
        .read_chunk(&thread_id, params.cursor)
        .await
    {
        Ok(chunk) => (StatusCode::OK, Json(json!(chunk))),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        ),
    }
}

/// POST /api/threads - create a canonical thread
pub async fn create_thread(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateThreadBody>,
) -> impl IntoResponse {
    let requested_session_id = body
        .sdk_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested_session_id.is_some() && body.workspace_mode.is_worktree() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "workspaceMode=worktree cannot be combined with sdkSessionId resume"
            })),
        );
    }
    let requested_session_provider_hint =
        match parse_sdk_session_provider_hint(body.sdk_session_provider_hint.as_deref()) {
            Ok(value) => value,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": error })));
            }
        };
    let recovered_session = match requested_session_id {
        Some(session_id) => match recover_local_provider_session(
            session_id,
            requested_session_provider_hint.clone(),
        ) {
            Ok(Some(recovered)) => Some(recovered),
            Ok(None) => {
                let provider_label = requested_session_provider_hint
                    .as_ref()
                    .map(provider_hint_label)
                    .unwrap_or("Claude, Codex, or Gemini");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": format!(
                            "No local {provider_label} session was found for session id '{session_id}'. Resume must start from an existing local {provider_label} session on this Mac."
                        )
                    })),
                );
            }
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": error })));
            }
        },
        None => None,
    };

    let options = ThreadEnsureOptions {
        label: body.label.clone(),
        workspace_dir: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.workspace_dir.clone())
            .or_else(|| body.workspace_dir.clone()),
        workspace_mode: body.workspace_mode,
        worktree_base_dir: Some(worktree_base_dir_for_config(&state.config_snapshot())),
        agent_id: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.agent_id.clone())
            .or_else(|| body.agent_id.clone()),
        metadata: body.metadata.clone(),
        provider_type: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.provider_type.clone()),
        sdk_session_id: body.sdk_session_id.clone(),
        thread_kind: None,
        origin_channel: None,
        origin_account_id: None,
        origin_from_id: None,
        is_group: None,
    };

    match create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        state.ops.agent_teams.clone(),
        options,
    )
    .await
    {
        Ok((thread_id, mut data, _resolved)) => {
            if let Some(recovered) = recovered_session.as_ref()
                && let Err(error) =
                    seed_imported_thread_history(&state, &thread_id, &mut data, &recovered.messages)
                        .await
            {
                state.threads.thread_store.delete(&thread_id).await;
                let _ = state
                    .threads
                    .history
                    .delete_thread_history(&thread_id)
                    .await;
                state
                    .integration
                    .bridge
                    .set_thread_workspace_binding(&thread_id, None)
                    .await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": error })),
                );
            }
            rebuild_thread_indexes(&state).await;
            state.invalidate_gateway_sync_caches().await;
            (StatusCode::CREATED, Json(thread_summary(&thread_id, &data)))
        }
        Err(error)
            if error.starts_with("unknown agent_id:")
                || error.starts_with("agent_id is not standalone:")
                || error.starts_with("team '")
                || error.starts_with("workspace_mode=worktree") =>
        {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
    }
}

/// PATCH /api/threads/:key - update canonical thread metadata
pub async fn update_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<UpdateThreadBody>,
) -> impl IntoResponse {
    let Some(thread_id) = ensure_existing_thread_id(&state, &key).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "thread not found"})),
        );
    };

    match update_thread_record(
        &state.threads.thread_store,
        &thread_id,
        body.label,
        body.workspace_dir,
    )
    .await
    {
        Ok(data) => {
            state
                .integration
                .bridge
                .set_thread_workspace_binding(&thread_id, workspace_dir_from_value(&data))
                .await;
            state.invalidate_gateway_sync_caches().await;
            (StatusCode::OK, Json(thread_summary(&thread_id, &data)))
        }
        Err(error) if error.contains("thread not found") => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": error })))
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
    }
}

/// DELETE /api/threads/:key - delete thread
pub async fn delete_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let Some(thread_id) = ensure_existing_thread_id(&state, &key).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"deleted": false, "error": "thread not found"})),
        );
    };
    let Some(thread_data) = state.threads.thread_store.get(&thread_id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"deleted": false, "error": "thread not found"})),
        );
    };
    // Block deletion only while at least one binding still points at a bot
    // account that is still enabled. Orphan bindings (left behind when a bot
    // is removed from channels config) and disabled bots must not keep the
    // thread alive — there is no other way to clean up their transcripts.
    let bindings = bindings_from_value(&thread_data);
    if !bindings.is_empty() {
        let config = state.config_snapshot();
        let has_live_binding = bindings.iter().any(|binding| {
            config
                .channels
                .plugins
                .get(&binding.channel)
                .and_then(|cfg| cfg.accounts.get(&binding.account_id))
                .map(|entry| entry.enabled)
                .unwrap_or(false)
        });
        if has_live_binding {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "deleted": false,
                    "error": "cannot delete thread with active channel bindings",
                })),
            );
        }
    }

    let provider_key = thread_data
        .get("provider_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let _ = state.integration.bridge.abort_thread_runs(&thread_id).await;
    if !state.threads.thread_store.delete(&thread_id).await {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"deleted": false, "error": format!("thread not found: {thread_id}") })),
        );
    }

    state
        .integration
        .bridge
        .clear_thread_state(&thread_id, provider_key.as_deref())
        .await;
    state.integration.bridge.drop_thread_state(&thread_id).await;
    state
        .threads
        .router
        .lock()
        .await
        .clear_thread_references(&thread_id);
    {
        let mut router = state.threads.router.lock().await;
        router.clear_last_delivery(&thread_id);
        router.message_routing_index_mut().clear_thread(&thread_id);
    }
    let _ = state
        .threads
        .history
        .delete_thread_history(&thread_id)
        .await;
    let _ = state.ops.thread_logs.delete_thread(&thread_id).await;
    let _ = state.ops.garyx_db.unpin_thread(&thread_id);
    rebuild_thread_indexes(&state).await;
    state.invalidate_gateway_sync_caches().await;
    (
        StatusCode::OK,
        Json(json!({"deleted": true, "thread_id": thread_id})),
    )
}

/// GET /api/channel-endpoints - list known channel endpoints
pub async fn list_channel_endpoints(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let endpoints = state.cached_channel_endpoints().await;
    Json(json!({
        "endpoints": endpoints.iter().map(channel_endpoint_response_value).collect::<Vec<_>>(),
    }))
}

/// GET /api/workspaces/git-status - report whether a workspace can use worktree mode
pub async fn workspace_git_status(
    Query(params): Query<WorkspaceGitStatusParams>,
) -> impl IntoResponse {
    let workspace_dir = params.workspace_dir.trim();
    if workspace_dir.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "workspace_dir is required" })),
        );
    }
    match router_workspace_git_status(workspace_dir).await {
        Ok(status) => (StatusCode::OK, Json(json!(status))),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
    }
}

/// GET /api/configured-bots - list all configured channel bot accounts from config
pub async fn list_configured_bots(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config_snapshot();
    let endpoints = state.cached_channel_endpoints().await;
    let mut bots = Vec::new();

    for account in configured_channel_accounts(&config.channels) {
        if !account.enabled {
            continue;
        }
        let root_behavior = channel_root_behavior(&state, &account.channel).await;
        let account_ui = resolve_account_ui_with_endpoints(
            &state,
            &account.channel,
            &account.account_id,
            &endpoints,
        )
        .await;
        let main_endpoint = resolve_main_endpoint_with_endpoints(
            &state,
            &account.channel,
            &account.account_id,
            &endpoints,
        )
        .await;
        let default_open_endpoint = if root_behavior == "expand_only" {
            None
        } else if let Some(endpoint) = main_endpoint.as_ref() {
            Some(endpoint.to_value())
        } else {
            resolve_default_open_endpoint_from_account_ui(account_ui.as_ref(), &endpoints)
        };
        let default_open_thread_id = default_open_endpoint
            .as_ref()
            .and_then(|value| value.get("thread_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let display_name = bot_display_name(account.name.as_deref(), &account.account_id);
        bots.push(json!({
            "channel": account.channel,
            "account_id": account.account_id,
            "display_name": display_name,
            "name": account.name.as_deref(),
            "enabled": account.enabled,
            "agent_id": account.agent_id.as_deref().unwrap_or(""),
            "workspace_dir": account.workspace_dir.as_deref(),
            "workspace_mode": public_workspace_mode(account.workspace_mode.as_deref()),
            "root_behavior": root_behavior,
            "main_endpoint_status": if main_endpoint.is_some() { "resolved" } else { "unresolved" },
            "main_endpoint": main_endpoint.as_ref().map(ResolvedMainEndpoint::to_value),
            "main_endpoint_thread_id": main_endpoint.as_ref().and_then(|endpoint| endpoint.thread_id.clone()),
            "default_open_endpoint": default_open_endpoint,
            "default_open_thread_id": default_open_thread_id,
        }));
    }

    Json(json!({ "bots": bots }))
}

/// GET /api/bot-consoles - list aggregated bot console summaries
pub async fn list_bot_consoles(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config_snapshot();
    let endpoints = state.cached_channel_endpoints().await;
    let mut groups = Vec::<Value>::new();
    let mut group_indexes = HashMap::<String, usize>::new();

    for account in configured_channel_accounts(&config.channels) {
        if !account.enabled {
            continue;
        }
        let id = format!("{}::{}", account.channel, account.account_id);
        let root_behavior = channel_root_behavior(&state, &account.channel).await;
        let account_ui = resolve_account_ui_with_endpoints(
            &state,
            &account.channel,
            &account.account_id,
            &endpoints,
        )
        .await;
        let main_endpoint = resolve_main_endpoint_with_endpoints(
            &state,
            &account.channel,
            &account.account_id,
            &endpoints,
        )
        .await;
        let default_open_endpoint = if root_behavior == "expand_only" {
            None
        } else if let Some(endpoint) = main_endpoint.as_ref() {
            Some(endpoint.to_value())
        } else {
            resolve_default_open_endpoint_from_account_ui(account_ui.as_ref(), &endpoints)
        };
        let default_open_thread_id = default_open_endpoint
            .as_ref()
            .and_then(|value| value.get("thread_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let display_name = bot_display_name(account.name.as_deref(), &account.account_id);
        group_indexes.insert(id.clone(), groups.len());
        groups.push(json!({
                "id": id,
                "channel": account.channel,
                "account_id": account.account_id,
                "display_name": display_name,
                "name": account.name.as_deref(),
                "title": bot_title(&account.channel, &account.account_id, account.name.as_deref()),
                "subtitle": bot_subtitle(&account.channel, &account.account_id),
                "agent_id": account.agent_id.as_deref().unwrap_or(""),
                "workspace_dir": account.workspace_dir.as_deref(),
                "workspace_mode": public_workspace_mode(account.workspace_mode.as_deref()),
                "root_behavior": root_behavior,
                "endpoint_count": 0,
                "bound_endpoint_count": 0,
                "latest_activity": Value::Null,
                "status": "idle",
                "main_endpoint_status": if main_endpoint.is_some() { "resolved" } else { "unresolved" },
                "main_endpoint": main_endpoint.as_ref().map(ResolvedMainEndpoint::to_value),
                "main_endpoint_thread_id": main_endpoint.as_ref().and_then(|endpoint| endpoint.thread_id.clone()),
                "default_open_endpoint": default_open_endpoint,
                "default_open_thread_id": default_open_thread_id,
                "conversation_nodes": conversation_nodes_from_account_ui(
                    account_ui.as_ref(),
                    &endpoints,
                ).unwrap_or_default(),
                "endpoints": [],
            }));
    }

    for endpoint in endpoints
        .iter()
        .filter(|endpoint| endpoint.thread_id.is_some())
    {
        let id = format!("{}::{}", endpoint.channel, endpoint.account_id);
        let Some(index) = group_indexes.get(&id).copied() else {
            continue;
        };
        let Some(entry) = groups.get_mut(index) else {
            continue;
        };
        let activity = endpoint
            .last_inbound_at
            .as_ref()
            .or(endpoint.last_delivery_at.as_ref())
            .or(endpoint.thread_updated_at.as_ref())
            .cloned();

        if let Some(obj) = entry.as_object_mut() {
            let endpoint_count = obj
                .get("endpoint_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            obj.insert("endpoint_count".to_owned(), Value::from(endpoint_count));

            if endpoint.thread_id.is_some() {
                let bound_count = obj
                    .get("bound_endpoint_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    + 1;
                obj.insert("bound_endpoint_count".to_owned(), Value::from(bound_count));
                obj.insert("status".to_owned(), Value::String("connected".to_owned()));
            }

            let replace_activity = match (
                obj.get("latest_activity").and_then(Value::as_str),
                activity.as_deref(),
            ) {
                (Some(current), Some(candidate)) => candidate > current,
                (None, Some(_)) => true,
                _ => false,
            };
            if replace_activity {
                obj.insert(
                    "latest_activity".to_owned(),
                    activity.clone().map(Value::String).unwrap_or(Value::Null),
                );
            }

            let endpoints_value = obj
                .entry("endpoints".to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(items) = endpoints_value.as_array_mut() {
                items.push(channel_endpoint_response_value(endpoint));
            }
        }
    }

    for group in &mut groups {
        if let Some(items) = group.get_mut("endpoints").and_then(Value::as_array_mut) {
            sort_channel_endpoint_values_by_identity(items);
        }
    }

    Json(json!({ "bots": groups }))
}

/// GET /api/skills - list skills from local and project registries.
pub async fn list_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.ops.skills.list_skills() {
        Ok(skills) => (StatusCode::OK, Json(json!({ "skills": skills }))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// POST /api/skills - create a new local skill under ~/.garyx/skills.
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSkillBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .create_skill(&body.id, &body.name, &body.description, &body.body)
    {
        Ok(skill) => (StatusCode::CREATED, Json(json!(skill))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// PATCH /api/skills/:id - update skill metadata in SKILL.md frontmatter.
pub async fn update_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSkillBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .update_skill(&id, &body.name, &body.description)
    {
        Ok(skill) => (StatusCode::OK, Json(json!(skill))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// PATCH /api/skills/:id/toggle - flip enabled state.
pub async fn toggle_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.ops.skills.toggle_skill(&id) {
        Ok(skill) => (StatusCode::OK, Json(json!(skill))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// DELETE /api/skills/:id - remove a skill directory.
pub async fn delete_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.ops.skills.delete_skill(&id) {
        Ok(()) => (StatusCode::OK, Json(json!({ "deleted": true, "id": id }))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// GET /api/skills/:id/tree - list all files/directories inside one skill.
pub async fn skill_tree(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.ops.skills.skill_editor_state(&id) {
        Ok(editor) => (StatusCode::OK, Json(json!(editor))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// GET /api/skills/:id/file - read one skill file as editable text or preview payload.
pub async fn read_skill_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<SkillFileParams>,
) -> impl IntoResponse {
    match state.ops.skills.read_skill_file(&id, &params.path) {
        Ok(document) => (StatusCode::OK, Json(json!(document))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// PUT /api/skills/:id/file - save one editable text file inside a skill directory.
pub async fn write_skill_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<WriteSkillFileBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .write_skill_file(&id, &body.path, &body.content)
    {
        Ok(document) => (StatusCode::OK, Json(json!(document))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// POST /api/skills/:id/entries - create a file or directory inside a skill.
pub async fn create_skill_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateSkillEntryBody>,
) -> impl IntoResponse {
    match state
        .ops
        .skills
        .create_skill_entry(&id, &body.path, &body.entry_type)
    {
        Ok(editor) => (StatusCode::CREATED, Json(json!(editor))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// DELETE /api/skills/:id/entries?path=... - remove one file or directory inside a skill.
pub async fn delete_skill_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<SkillFileParams>,
) -> impl IntoResponse {
    match state.ops.skills.delete_skill_entry(&id, &params.path) {
        Ok(editor) => (StatusCode::OK, Json(json!(editor))).into_response(),
        Err(error) => skill_error_response(error).into_response(),
    }
}

/// POST /api/channel-bindings/bind - move endpoint to another thread
pub async fn bind_channel_endpoint(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BindChannelEndpointBody>,
) -> impl IntoResponse {
    match bind_channel_endpoint_key_to_thread(&state, &body.endpoint_key, &body.thread_id).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "thread_id": result.thread_id,
                "previous_thread_id": result.previous_thread_id,
                "endpoint_key": result.endpoint_key,
            })),
        ),
        Err(error) => (error.status, Json(json!({ "error": error.message }))),
    }
}

/// POST /api/channel-bindings/detach - detach endpoint from current thread
pub async fn detach_channel_endpoint(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DetachChannelEndpointBody>,
) -> impl IntoResponse {
    match detach_channel_endpoint_key(&state, &body.endpoint_key).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": result.previous_thread_id.is_some(),
                "previous_thread_id": result.previous_thread_id,
                "endpoint_key": result.endpoint_key,
            })),
        ),
        Err(error) => (error.status, Json(json!({ "error": error.message }))),
    }
}

/// GET /api/status - detailed system status
pub async fn system_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.runtime.start_time.elapsed().as_secs();
    let thread_count = state.threads.thread_store.list_keys(None).await.len();
    let stream_drops = state.ops.events.dropped_count();
    let stream_history_size = state.ops.events.history_len().await;

    Json(json!({
        "status": "running",
        "uptime_seconds": uptime,
        "threads": {
            "count": thread_count,
        },
        "stream": {
            "drops": stream_drops,
            "history_size": stream_history_size,
        },
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Fallback handler for unknown routes
pub async fn fallback() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json::<Value>(json!({"error": "not found"})),
    )
}

fn skill_error_response(error: SkillStoreError) -> (StatusCode, Json<Value>) {
    match error {
        SkillStoreError::Validation(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
        }
        SkillStoreError::AlreadyExists(message) => {
            (StatusCode::CONFLICT, Json(json!({ "error": message })))
        }
        SkillStoreError::NotFound(message) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": message })))
        }
        SkillStoreError::Io(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ),
    }
}

#[cfg(test)]
mod tests;
