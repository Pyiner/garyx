use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use futures_util::StreamExt;
use garyx_channels::plugin::{PluginAccountUi, PluginConversationEndpoint, PluginMainEndpoint};
use garyx_models::RenderSnapshot;
use garyx_models::config::ChannelsConfig;
#[cfg(test)]
use garyx_models::config::TelegramAccount;
use garyx_models::provider::{
    FORK_FROM_PROVIDER_TYPE_METADATA_KEY, FORK_FROM_SDK_SESSION_ID_METADATA_KEY,
    FORK_FROM_THREAD_ID_METADATA_KEY, MODEL_METADATA_KEY, MODEL_OVERRIDE_METADATA_KEY,
    MODEL_REASONING_EFFORT_METADATA_KEY, MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
    MODEL_SERVICE_TIER_METADATA_KEY, MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY, ProviderType,
    SDK_SESSION_FORK_METADATA_KEY,
};
use garyx_models::routing::{DELIVERY_TARGET_TYPE_CHAT_ID, DELIVERY_TARGET_TYPE_OPEN_ID};
use garyx_router::ThreadStoreExt;
use garyx_router::{
    ArchiveBarrier, ChannelBinding, CoordinationError, KnownChannelEndpoint,
    THREAD_TRANSCRIPT_REPLAY_CAP, ThreadCreationError, ThreadEnsureOptions, ThreadTranscriptRecord,
    WorkspaceMode, bindings_from_value, history_message_count, is_thread_key, update_thread_record,
    workspace_dir_from_value, workspace_git_status as router_workspace_git_status,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream;
use tokio_stream::wrappers::BroadcastStream;

use crate::agent_identity::create_thread_for_agent_reference;
use crate::endpoint_binding_mutator::DeleteBindingPreflight;
use crate::garyx_db::{
    FavoriteThreadResult, GaryxDbError, GaryxDbResult, LifecycleDecisionInput,
    LifecycleMutationInput, LifecycleOperationKind, LifecycleOperationLookup,
    LifecycleOperationOutcome, LifecycleOperationRecord, LifecycleTransactionResult,
    MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE, RecentThreadRecord, RecentThreadTaskFilter,
    ReorderThreadPinsResult, ThreadFavoritesPage, ThreadMetaRecord, ThreadPinsPage,
    ThreadSummaryTaskFilter,
};
use crate::provider_session_locator::{
    list_recent_local_provider_sessions, recover_local_provider_session,
};
use crate::server::AppState;
use crate::skills::SkillStoreError;
use crate::thread_lifecycle::{
    LIFECYCLE_JOIN_WINDOW, MutationSupervisor, OperationCellResult, OperationJoinHandle,
    OperationKey, OperationOwnerGuard, OperationRegistration, OperationRegistrationError,
    OperationWaitError, canonical_lifecycle_fingerprint,
};
use crate::thread_meta_projection::normalize_for_search;
use crate::thread_runtime::{
    AgentCatalogSnapshot, build_thread_runtime_summary, build_thread_runtime_summary_from_meta,
    build_thread_runtime_summary_with_catalog,
};
use crate::thread_type::thread_summary_type_from_record;
use crate::workspace_mode::{
    ensure_implicit_thread_workspace_for_config, worktree_base_dir_for_config,
};
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

fn endpoint_conversation_kind(
    channel: &str,
    binding_key: &str,
    chat_id: &str,
    delivery_target_type: &str,
    delivery_thread_id: Option<&str>,
) -> &'static str {
    let binding_key = binding_key.trim();
    let chat_id = chat_id.trim();
    // A legacy scoped binding without its parent chat remains a group; the
    // original endpoint classifier only treated concrete chat scopes as topics.
    let is_topic = delivery_thread_id.is_some() && !chat_id.is_empty();

    if channel == "discord" {
        if !binding_key.is_empty() && !chat_id.is_empty() && binding_key == chat_id {
            "group"
        } else {
            "private"
        }
    } else if channel == "feishu" {
        if delivery_target_type == DELIVERY_TARGET_TYPE_OPEN_ID {
            "private"
        } else if is_topic {
            "topic"
        } else if delivery_target_type == DELIVERY_TARGET_TYPE_CHAT_ID {
            "group"
        } else {
            "private"
        }
    } else if is_topic {
        "topic"
    } else if !binding_key.is_empty() && binding_key != chat_id {
        "group"
    } else {
        "private"
    }
}

fn endpoint_conversation_details(
    endpoint: &garyx_router::KnownChannelEndpoint,
) -> EndpointConversationDetails {
    let fallback_label = default_endpoint_conversation_label(endpoint);
    let kind = endpoint_conversation_kind(
        &endpoint.channel,
        &endpoint.binding_key,
        &endpoint.chat_id,
        &endpoint.delivery_target_type,
        endpoint_scope(endpoint),
    );

    EndpointConversationDetails {
        kind,
        label: fallback_label,
    }
}

fn resolved_main_endpoint_conversation_details(
    endpoint: &ResolvedMainEndpoint,
) -> EndpointConversationDetails {
    let kind = endpoint_conversation_kind(
        &endpoint.channel,
        &endpoint.binding_key,
        &endpoint.chat_id,
        &endpoint.delivery_target_type,
        endpoint.delivery_thread_id.as_deref(),
    );

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

fn endpoint_activity(endpoint: &garyx_router::KnownChannelEndpoint) -> Option<&str> {
    endpoint
        .last_inbound_at
        .as_deref()
        .or(endpoint.last_delivery_at.as_deref())
        .or(endpoint.thread_updated_at.as_deref())
}

fn default_open_endpoint_from_projected_endpoints(
    endpoints: &[&garyx_router::KnownChannelEndpoint],
) -> Option<Value> {
    endpoints
        .iter()
        .filter(|endpoint| endpoint.thread_id.is_some())
        .max_by(|left, right| {
            endpoint_activity(left)
                .unwrap_or_default()
                .cmp(endpoint_activity(right).unwrap_or_default())
                .then_with(|| left.endpoint_key.cmp(&right.endpoint_key))
        })
        .map(|endpoint| channel_endpoint_response_value(endpoint))
}

fn conversation_nodes_from_projected_endpoints(
    endpoints: &[&garyx_router::KnownChannelEndpoint],
) -> Vec<Value> {
    let mut sorted = endpoints
        .iter()
        .filter(|endpoint| endpoint.thread_id.is_some())
        .copied()
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        endpoint_activity(right)
            .unwrap_or_default()
            .cmp(endpoint_activity(left).unwrap_or_default())
            .then_with(|| left.endpoint_key.cmp(&right.endpoint_key))
    });

    sorted
        .into_iter()
        .map(|endpoint| {
            let conversation = endpoint_conversation_details(endpoint);
            json!({
                "id": endpoint.endpoint_key.replace("::", ":"),
                "endpoint": channel_endpoint_response_value(endpoint),
                "kind": conversation.kind,
                "title": conversation.label,
                "badge": Value::Null,
                "latest_activity": endpoint_activity(endpoint),
                "openable": endpoint.thread_id.is_some(),
            })
        })
        .collect()
}

pub(crate) async fn resolve_main_endpoint_by_bot(
    state: &Arc<AppState>,
    channel: &str,
    account_id: &str,
) -> Result<Option<ResolvedMainEndpoint>, garyx_router::ThreadStoreError> {
    let endpoints = state.cached_channel_endpoints().await?;
    Ok(resolve_main_endpoint_with_endpoints(state, channel, account_id, &endpoints).await)
}

/// Fresh-read variant for status surfaces whose response IS the
/// resolution result (#TASK-2134): never satisfied from the snapshot
/// cache, so a live storage outage cannot hide behind a recent hit.
pub(crate) async fn resolve_main_endpoint_by_bot_fresh(
    state: &Arc<AppState>,
    channel: &str,
    account_id: &str,
) -> Result<Option<ResolvedMainEndpoint>, garyx_router::ThreadStoreError> {
    let endpoints = state.channel_endpoints_fresh().await?;
    Ok(resolve_main_endpoint_with_endpoints(state, channel, account_id, &endpoints).await)
}

async fn resolve_main_endpoint_by_key(
    state: &Arc<AppState>,
    endpoint_key_value: &str,
) -> Result<Option<ResolvedMainEndpoint>, garyx_router::ThreadStoreError> {
    let config = state.config_snapshot();
    let endpoints = state.cached_channel_endpoints().await?;

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
            return Ok(Some(endpoint));
        }
    }

    Ok(None)
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

/// GET /api/store-identity - bootstrap identity for the favorites CAS domain.
pub async fn store_identity(State(state): State<Arc<AppState>>) -> axum::response::Response {
    match state.ops.garyx_db.store_incarnation_id() {
        Ok(store_incarnation_id) => Json(json!({
            "store_incarnation_id": store_incarnation_id,
            "server_boot_id": state.server_boot_id(),
        }))
        .into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/threads - list threads with pagination/filtering
// ---------------------------------------------------------------------------

const DEFAULT_THREAD_LIMIT: usize = 100;
const MAX_THREAD_LIMIT: usize = 1000;
const DEFAULT_RECENT_THREAD_LIMIT: usize = 30;
const MAX_RECENT_THREAD_LIMIT: usize = 200;
const DEFAULT_THREAD_SUMMARY_LIMIT: usize = 30;
const MAX_THREAD_SUMMARY_LIMIT: usize = 100;

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
#[serde(deny_unknown_fields)]
pub struct ListRecentThreadsParams {
    /// Maximum number of recent threads to return.
    #[serde(default)]
    pub limit: Option<String>,
    /// Task membership filter. Omitting it preserves the existing unfiltered
    /// recent-thread response.
    #[serde(default)]
    pub tasks: Option<String>,
    /// Opaque filter-bound keyset cursor returned by the preceding page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListThreadSummariesParams {
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub tasks: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
}

#[derive(Deserialize)]
pub struct ThreadFavoritesSnapshotQuery {
    #[serde(default)]
    pub include_summaries: Option<bool>,
}

#[derive(Deserialize)]
pub struct ThreadFavoritesMutationQuery {
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub expected_store_incarnation: Option<String>,
}

#[derive(Deserialize)]
pub struct ThreadLogParams {
    #[serde(default)]
    pub cursor: Option<u64>,
}

#[derive(Deserialize)]
pub struct BotListParams {
    #[serde(default)]
    pub include_endpoints: bool,
}

fn default_thread_limit() -> usize {
    DEFAULT_THREAD_LIMIT
}

fn parse_recent_thread_task_filter(value: Option<&str>) -> Result<RecentThreadTaskFilter, String> {
    match value {
        None | Some("include") => Ok(RecentThreadTaskFilter::Include),
        Some("exclude") => Ok(RecentThreadTaskFilter::Exclude),
        Some("only") => Ok(RecentThreadTaskFilter::Only),
        Some(_) => Err("tasks must be one of: include, exclude, only".to_owned()),
    }
}

fn parse_thread_summary_task_filter(
    value: Option<&str>,
) -> Result<ThreadSummaryTaskFilter, String> {
    match value {
        None | Some("include") => Ok(ThreadSummaryTaskFilter::Include),
        Some("exclude") => Ok(ThreadSummaryTaskFilter::Exclude),
        Some("only") => Ok(ThreadSummaryTaskFilter::Only),
        Some(_) => Err("tasks must be one of: include, exclude, only".to_owned()),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecentThreadsCursor {
    v: u8,
    filter: String,
    activity_seq: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadSummariesCursor {
    v: u8,
    scope: String,
    tasks: String,
    q: Option<String>,
    incarnation: String,
    sort_key: i64,
    thread_id: String,
}

struct ParsedThreadSummariesParams {
    workspace_dir: Option<String>,
    filter: ThreadSummaryTaskFilter,
    query: Option<String>,
    scope: String,
    limit: usize,
    cursor: Option<ThreadSummariesCursor>,
}

fn thread_summary_scope(workspace_dir: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_dir.unwrap_or_default().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn decode_thread_summaries_cursor(raw: &str) -> Result<ThreadSummariesCursor, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "cursor must be an opaque thread-summaries cursor".to_owned())?;
    let cursor: ThreadSummariesCursor = serde_json::from_slice(&bytes)
        .map_err(|_| "cursor must be an opaque thread-summaries cursor".to_owned())?;
    if cursor.v != 1 {
        return Err("cursor version is not supported".to_owned());
    }
    if cursor.thread_id.trim().is_empty() {
        return Err("cursor must be an opaque thread-summaries cursor".to_owned());
    }
    Ok(cursor)
}

fn encode_thread_summaries_cursor(
    scope: &str,
    filter: ThreadSummaryTaskFilter,
    query: Option<&str>,
    incarnation: &str,
    sort_key: i64,
    thread_id: &str,
) -> String {
    let encoded = serde_json::to_vec(&ThreadSummariesCursor {
        v: 1,
        scope: scope.to_owned(),
        tasks: filter.cursor_value().to_owned(),
        q: query.map(ToOwned::to_owned),
        incarnation: incarnation.to_owned(),
        sort_key,
        thread_id: thread_id.to_owned(),
    })
    .expect("thread-summaries cursor serialization is infallible");
    URL_SAFE_NO_PAD.encode(encoded)
}

fn parse_thread_summaries_params(
    query: Result<Query<ListThreadSummariesParams>, axum::extract::rejection::QueryRejection>,
) -> Result<ParsedThreadSummariesParams, String> {
    let Query(params) = query.map_err(|error| error.to_string())?;
    let limit = match params.limit.as_deref() {
        None => DEFAULT_THREAD_SUMMARY_LIMIT,
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=MAX_THREAD_SUMMARY_LIMIT).contains(value))
            .ok_or_else(|| {
                format!("limit must be an integer from 1 through {MAX_THREAD_SUMMARY_LIMIT}")
            })?,
    };
    let workspace_dir = params.workspace_dir;
    if workspace_dir
        .as_deref()
        .is_some_and(|workspace_dir| !FsPath::new(workspace_dir).is_absolute())
    {
        return Err("workspace_dir must be an absolute path".to_owned());
    }
    let filter = parse_thread_summary_task_filter(params.tasks.as_deref())?;
    let query = params
        .q
        .as_deref()
        .map(str::trim)
        .map(normalize_for_search)
        .filter(|value| !value.is_empty());
    if query
        .as_deref()
        .is_some_and(|query| query.chars().count() > 100)
    {
        return Err(
            "q must contain at most 100 Unicode scalar values after normalization".to_owned(),
        );
    }
    let scope = thread_summary_scope(workspace_dir.as_deref());
    let cursor = params
        .cursor
        .as_deref()
        .map(decode_thread_summaries_cursor)
        .transpose()?;
    if let Some(cursor) = &cursor {
        if cursor.scope != scope {
            return Err("cursor does not belong to the requested workspace scope".to_owned());
        }
        if cursor.tasks != filter.cursor_value() {
            return Err("cursor does not belong to the requested tasks filter".to_owned());
        }
        if cursor.q != query {
            return Err("cursor does not belong to the requested search query".to_owned());
        }
    }
    Ok(ParsedThreadSummariesParams {
        workspace_dir,
        filter,
        query,
        scope,
        limit,
        cursor,
    })
}

fn decode_recent_threads_cursor(raw: &str, filter: RecentThreadTaskFilter) -> Result<i64, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "cursor must be an opaque recent-threads cursor".to_owned())?;
    let cursor: RecentThreadsCursor = serde_json::from_slice(&bytes)
        .map_err(|_| "cursor must be an opaque recent-threads cursor".to_owned())?;
    if cursor.v != 1 {
        return Err("cursor version is not supported".to_owned());
    }
    if cursor.filter != filter.cursor_value() {
        return Err("cursor does not belong to the requested tasks filter".to_owned());
    }
    if !(0..MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE).contains(&cursor.activity_seq) {
        return Err("cursor activity_seq is outside the supported range".to_owned());
    }
    Ok(cursor.activity_seq)
}

fn encode_recent_threads_cursor(filter: RecentThreadTaskFilter, activity_seq: i64) -> String {
    let encoded = serde_json::to_vec(&RecentThreadsCursor {
        v: 1,
        filter: filter.cursor_value().to_owned(),
        activity_seq,
    })
    .expect("recent cursor serialization is infallible");
    URL_SAFE_NO_PAD.encode(encoded)
}

fn parse_recent_threads_params(
    query: Result<Query<ListRecentThreadsParams>, axum::extract::rejection::QueryRejection>,
) -> Result<(RecentThreadTaskFilter, usize, Option<i64>), String> {
    let Query(params) = query.map_err(|error| error.to_string())?;
    let limit = match params.limit.as_deref() {
        None => DEFAULT_RECENT_THREAD_LIMIT,
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=MAX_RECENT_THREAD_LIMIT).contains(value))
            .ok_or_else(|| {
                format!("limit must be an integer from 1 through {MAX_RECENT_THREAD_LIMIT}")
            })?,
    };
    let filter = parse_recent_thread_task_filter(params.tasks.as_deref())?;
    let before_activity_seq = params
        .cursor
        .as_deref()
        .map(|cursor| decode_recent_threads_cursor(cursor, filter))
        .transpose()?;
    Ok((filter, limit, before_activity_seq))
}

fn parse_sdk_session_provider_hint(value: Option<&str>) -> Result<Option<ProviderType>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    ProviderType::from_slug(&value.to_ascii_lowercase())
        .map(Some)
        .ok_or_else(|| {
            format!(
                "Unsupported sdkSessionProviderHint '{value}'. Use claude, codex, traex, or antigravity."
            )
        })
}

fn provider_hint_label(value: &ProviderType) -> &'static str {
    match value {
        ProviderType::ClaudeCode => "Claude",
        ProviderType::CodexAppServer => "Codex",
        ProviderType::Traex => "Traex",
        ProviderType::AntigravityCli => "Antigravity",
    }
}

fn is_resume_provider(value: &ProviderType) -> bool {
    // Traex is intentionally excluded: garyx does not support disk-based session
    // recovery / fork-from-session for TRAE CLI (its sessions live under
    // ~/.trae and are not wired into the provider session locator).
    matches!(
        value,
        ProviderType::ClaudeCode | ProviderType::CodexAppServer
    )
}

fn provider_type_from_thread_value(thread_data: &Value) -> Option<ProviderType> {
    thread_data
        .get("provider_type")
        .cloned()
        .and_then(|value| serde_json::from_value::<ProviderType>(value).ok())
}

fn non_empty_json_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn fork_source_sdk_session_id(thread_data: &Value, provider_type: &ProviderType) -> Option<String> {
    if provider_type_from_thread_value(thread_data)
        .as_ref()
        .is_some_and(|persisted_provider_type| persisted_provider_type == provider_type)
        && let Some(session_id) = non_empty_json_string(thread_data.get("sdk_session_id"))
    {
        return Some(session_id);
    }

    let provider_scoped_session_ids = thread_data
        .get("provider_sdk_session_ids")
        .and_then(Value::as_object)?;
    if provider_scoped_session_ids.len() == 1 {
        return provider_scoped_session_ids
            .values()
            .next()
            .and_then(|value| non_empty_json_string(Some(value)));
    }
    None
}

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

    // The transcript is the only imported-content copy (#TASK-1864
    // batch 1c): no record `messages` snapshot is seeded. The write-time
    // preview fields are derived from the imported content directly.
    for role in ["user", "assistant"] {
        if let Some(field) = garyx_models::message_preview::preview_field_for_role(role)
            && let Some(preview) =
                garyx_models::message_preview::last_message_preview_for_role(messages.iter(), role)
        {
            object.insert(field.to_owned(), Value::String(preview));
        }
    }
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

    object.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
    );
    state
        .threads
        .thread_store
        .set(thread_id, thread_data.clone())
        .await
        .map_err(|error| error.to_string())?;
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
    /// Agent ID.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Optional per-thread model override; wins over the agent's configured model.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional per-thread reasoning/thinking level override.
    #[serde(default)]
    pub model_reasoning_effort: Option<String>,
    /// Optional per-thread service tier override.
    #[serde(default)]
    pub model_service_tier: Option<String>,
    /// Optional provider-native session id to resume from on the first run.
    #[serde(default, alias = "sessionId")]
    pub sdk_session_id: Option<String>,
    /// Optional provider hint for sdkSessionId. Supported values: claude, codex.
    #[serde(default)]
    pub sdk_session_provider_hint: Option<String>,
    /// Optional Garyx thread id to fork from using the provider-native session fork.
    #[serde(default)]
    pub fork_from_thread_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProviderSessionsParams {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitStatusParams {
    #[serde(default, alias = "workspace_dir")]
    pub workspace_dir: String,
}

/// GET /api/provider-sessions/recent - list recent local provider-native sessions
pub async fn list_recent_provider_sessions(
    Query(params): Query<RecentProviderSessionsParams>,
) -> impl IntoResponse {
    let provider_hint = match parse_sdk_session_provider_hint(params.provider.as_deref()) {
        Ok(value) => value,
        Err(error) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))),
    };
    if let Some(provider_hint) = provider_hint.as_ref()
        && !is_resume_provider(provider_hint)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "provider must be claude or codex"
            })),
        );
    }
    let limit = params.limit.unwrap_or(10).clamp(1, 50);
    let sessions = list_recent_local_provider_sessions(provider_hint, limit);
    (StatusCode::OK, Json(json!({ "sessions": sessions })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateThreadBody {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    /// Optional per-thread model override. An empty string clears the override.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional per-thread reasoning/thinking level override. An empty string clears it.
    #[serde(default)]
    pub model_reasoning_effort: Option<String>,
    /// Optional per-thread service tier override. An empty string clears it.
    #[serde(default)]
    pub model_service_tier: Option<String>,
}

/// Write one thread runtime cell (single-cell semantics): `body` values
/// rewrite the cell key that the run path and runtime summary read, an empty
/// string empties the cell so provider/agent defaults apply again, and any
/// legacy dual-track override key is migrated away (deleted) whenever the
/// cell is touched.
fn apply_thread_metadata_cell(
    data: &mut Value,
    cell_key: &str,
    legacy_override_key: &str,
    input: &Option<String>,
) -> bool {
    let Some(input) = input.as_deref() else {
        return false;
    };
    let Some(obj) = data.as_object_mut() else {
        return false;
    };
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return obj
            .get_mut("metadata")
            .and_then(Value::as_object_mut)
            .map(|metadata| {
                let removed_cell = metadata.remove(cell_key).is_some();
                let removed_legacy = metadata.remove(legacy_override_key).is_some();
                removed_cell || removed_legacy
            })
            .unwrap_or(false);
    }

    if !obj.get("metadata").is_some_and(Value::is_object) {
        obj.insert("metadata".to_owned(), Value::Object(Map::new()));
    }
    let Some(metadata) = obj.get_mut("metadata").and_then(Value::as_object_mut) else {
        return false;
    };
    let removed_legacy = metadata.remove(legacy_override_key).is_some();
    let next = Value::String(trimmed.to_owned());
    if !removed_legacy && metadata.get(cell_key) == Some(&next) {
        return false;
    }
    metadata.insert(cell_key.to_owned(), next);
    true
}

fn apply_thread_runtime_cells(data: &mut Value, body: &UpdateThreadBody) -> bool {
    let mut changed = false;
    changed |= apply_thread_metadata_cell(
        data,
        MODEL_METADATA_KEY,
        MODEL_OVERRIDE_METADATA_KEY,
        &body.model,
    );
    changed |= apply_thread_metadata_cell(
        data,
        MODEL_REASONING_EFFORT_METADATA_KEY,
        MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
        &body.model_reasoning_effort,
    );
    changed |= apply_thread_metadata_cell(
        data,
        MODEL_SERVICE_TIER_METADATA_KEY,
        MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY,
        &body.model_service_tier,
    );
    changed
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
#[serde(rename_all = "camelCase")]
pub struct ArchiveThreadBody {
    #[serde(alias = "operation_id")]
    pub operation_id: String,
    #[serde(alias = "expected_store_incarnation")]
    pub expected_store_incarnation: String,
    #[serde(default, alias = "endpoint_keys")]
    pub endpoint_keys: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteThreadBody {
    #[serde(alias = "operation_id")]
    pub operation_id: String,
    #[serde(alias = "expected_store_incarnation")]
    pub expected_store_incarnation: String,
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

/// Resolve an existing thread id at a request boundary: `Ok(None)` is a
/// plain 404, while a store backend failure surfaces as `Err` so handlers
/// answer 500 instead of a misleading not-found.
async fn ensure_existing_thread_id(
    state: &Arc<AppState>,
    key: &str,
) -> Result<Option<String>, (StatusCode, Json<Value>)> {
    let trimmed = key.trim();
    if trimmed.is_empty() || !is_thread_key(trimmed) {
        return Ok(None);
    }
    match state.threads.thread_store.exists(trimmed).await {
        Ok(true) => Ok(Some(trimmed.to_owned())),
        Ok(false) => Ok(None),
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )),
    }
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

pub(crate) async fn bind_channel_endpoint_key_to_thread(
    state: &Arc<AppState>,
    endpoint_key: &str,
    thread_id: &str,
) -> Result<ChannelEndpointBindResult, ChannelEndpointMutationError> {
    let requested_endpoint_key = normalize_endpoint_lookup_key(endpoint_key);
    let Some(thread_id) =
        ensure_existing_thread_id(state, thread_id)
            .await
            .map_err(|(status, body)| {
                ChannelEndpointMutationError::new(
                    status,
                    body.0["error"].as_str().unwrap_or("thread store failed"),
                )
            })?
    else {
        return Err(ChannelEndpointMutationError::new(
            StatusCode::NOT_FOUND,
            "target thread not found",
        ));
    };

    let known_endpoint = state
        .cached_channel_endpoints()
        .await
        .map_err(|error| {
            ChannelEndpointMutationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("thread store error: {error}"),
            )
        })?
        .into_iter()
        .find(|endpoint| endpoint_key_matches(&endpoint.endpoint_key, &requested_endpoint_key));

    let binding = if let Some(endpoint) = known_endpoint.as_ref() {
        binding_from_known_endpoint(endpoint)
    } else if let Some(binding) = resolve_main_endpoint_by_key(state, &requested_endpoint_key)
        .await
        .map_err(|error| {
            ChannelEndpointMutationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("thread store error: {error}"),
            )
        })?
        .map(|endpoint| endpoint.to_binding())
    {
        binding
    } else {
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
        Ok(mutation) => {
            // bind_endpoint_runtime upserts the endpoint index entry itself;
            // no full index rebuild is needed here.
            state.invalidate_gateway_sync_caches().await;
            Ok(ChannelEndpointBindResult {
                thread_id,
                previous_thread_id: mutation.previous_thread_id,
                endpoint_key: requested_endpoint_key,
                binding: mutation.binding,
            })
        }
        Err(error) => Err(endpoint_mutation_error_response(error)),
    }
}

/// Structured status mapping for endpoint binding mutations: a storage
/// outage inside the mutator surfaces as 500, never as a client error
/// (#TASK-2147).
fn endpoint_mutation_error_response(
    error: garyx_router::EndpointBindingMutationError,
) -> ChannelEndpointMutationError {
    use garyx_router::EndpointBindingMutationError as MutationError;
    let status = match &error {
        MutationError::TargetNotFound(_) => StatusCode::NOT_FOUND,
        MutationError::TargetArchived(_) => StatusCode::GONE,
        MutationError::ThreadLifecycleInProgress(_) => StatusCode::CONFLICT,
        MutationError::Incompatible(_) => StatusCode::BAD_REQUEST,
        MutationError::Unavailable
        | MutationError::Projection(_)
        | MutationError::PreviousOwnerUnavailable(_)
        | MutationError::WriteFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ChannelEndpointMutationError::new(status, error.to_string())
}

pub(crate) async fn detach_channel_endpoint_key(
    state: &Arc<AppState>,
    endpoint_key: &str,
) -> Result<ChannelEndpointDetachResult, ChannelEndpointMutationError> {
    let requested_endpoint_key = normalize_endpoint_lookup_key(endpoint_key);
    let mutation = {
        let mut router = state.threads.router.lock().await;
        router
            .detach_endpoint_runtime(&requested_endpoint_key)
            .await
    };
    match mutation {
        Ok(mutation) => {
            let previous_thread_id = mutation.previous_thread_id;
            state.invalidate_channel_endpoint_cache().await;
            if let (Some(thread_id), Some(binding)) =
                (previous_thread_id.as_deref(), mutation.binding.as_ref())
            {
                let delivery_thread_id =
                    binding_delivery_thread_id(&binding.binding_key, &binding.chat_id);
                let mut router = state.threads.router.lock().await;
                router
                    .clear_last_delivery_for_chat_with_known_thread_persistence(
                        thread_id,
                        &binding.channel,
                        &binding.account_id,
                        &binding.chat_id,
                        delivery_thread_id.as_deref(),
                    )
                    .await;
            }
            state.invalidate_gateway_sync_caches().await;
            Ok(ChannelEndpointDetachResult {
                previous_thread_id,
                endpoint_key: requested_endpoint_key,
                binding: mutation.binding,
            })
        }
        Err(error) => Err(endpoint_mutation_error_response(error)),
    }
}

fn last_message_preview(data: &Value, role: &str) -> Option<String> {
    // Write-time preview fields are the source (#TASK-1864 batch 1).
    if let Some(preview) = garyx_models::message_preview::preview_field_for_role(role)
        .and_then(|field| data.get(field))
        .and_then(Value::as_str)
    {
        return Some(preview.to_owned());
    }
    None
}

pub(crate) fn thread_summary(thread_id: &str, data: &Value) -> Value {
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
    let active_run_id = Value::Null;

    json!({
        "thread_id": thread_id,
        "thread_key": thread_id,
        "thread_type": thread_summary_type_from_record(data),
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

fn thread_summary_from_meta(record: &ThreadMetaRecord) -> Value {
    let worktree = record
        .worktree_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .unwrap_or(Value::Null);
    json!({
        "thread_id": record.thread_id.as_str(),
        "thread_key": record.thread_id.as_str(),
        "thread_type": record.thread_type.as_str(),
        "label": record.thread_label.as_deref(),
        "workspace_dir": record.workspace_dir.as_deref(),
        "channel_bindings": [],
        "updated_at": record.updated_at.as_deref(),
        "created_at": record.created_at.as_deref(),
        "message_count": record.message_count,
        "last_user_message": record.last_user_message.as_deref(),
        "last_assistant_message": record.last_assistant_message.as_deref(),
        "last_message_preview": record.last_message_preview.as_deref(),
        "agent_id": record.agent_id.as_deref(),
        "provider_type": record.provider_type.as_deref(),
        "worktree": worktree,
        "recent_run_id": record.recent_run_id.as_deref(),
        "active_run_id": record.active_run_id.as_deref(),
    })
}

fn thread_pin_ids(page: &ThreadPinsPage) -> Vec<String> {
    page.pins
        .iter()
        .map(|record| record.thread_id.clone())
        .collect()
}

fn thread_pins_payload(page: &ThreadPinsPage) -> Value {
    json!({
        "thread_ids": thread_pin_ids(page),
        "pins": page.pins,
        "revision": page.revision,
    })
}

const THREAD_FAVORITES_GET_OPERATION: &str = "thread_favorites_get";
const THREAD_FAVORITES_PUT_OPERATION: &str = "thread_favorites_put";
const THREAD_FAVORITES_DELETE_OPERATION: &str = "thread_favorites_delete";
const THREAD_FAVORITES_SNAPSHOT_OPERATION: &str = "thread_favorites_snapshot";
const RECENT_THREADS_LIST_OPERATION: &str = "recent_threads_list";

fn thread_favorite_ids(page: &ThreadFavoritesPage) -> Vec<String> {
    page.favorites
        .iter()
        .map(|favorite| favorite.thread_id.clone())
        .collect()
}

fn thread_favorites_payload(page: &ThreadFavoritesPage, server_boot_id: &str) -> Value {
    json!({
        "store_incarnation_id": page.store_incarnation_id,
        "server_boot_id": server_boot_id,
        "revision": page.revision,
        "thread_ids": thread_favorite_ids(page),
        "favorites": page.favorites,
    })
}

fn extend_json_object(payload: &mut Value, fields: Value) {
    let Some(payload) = payload.as_object_mut() else {
        return;
    };
    let Some(fields) = fields.as_object() else {
        return;
    };
    payload.extend(fields.clone());
}

fn thread_favorites_tagged_error(
    status: StatusCode,
    operation: &'static str,
    code: &'static str,
    message: impl Into<String>,
    page: Option<&ThreadFavoritesPage>,
    server_boot_id: &str,
    fields: Value,
) -> axum::response::Response {
    let mut payload = page.map_or_else(
        || {
            json!({
                "server_boot_id": server_boot_id,
            })
        },
        |page| thread_favorites_payload(page, server_boot_id),
    );
    extend_json_object(
        &mut payload,
        json!({
            "kind": "garyx_api_error",
            "operation": operation,
            "code": code,
            "message": message.into(),
        }),
    );
    extend_json_object(&mut payload, fields);
    (status, Json(payload)).into_response()
}

fn parse_thread_favorites_mutation_query(
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<(i64, String), String> {
    let Query(query) = query.map_err(|error| error.to_string())?;
    let expected_revision = query
        .expected_revision
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|revision| *revision >= 0)
        .ok_or_else(|| "expected_revision must be a non-negative integer".to_owned())?;
    let expected_store_incarnation = query
        .expected_store_incarnation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(|uuid| uuid.to_string())
        .ok_or_else(|| "expected_store_incarnation must be a UUID".to_owned())?;
    Ok((expected_revision, expected_store_incarnation))
}

async fn tagged_favorites_invalid_request(
    state: &Arc<AppState>,
    operation: &'static str,
    message: String,
) -> axum::response::Response {
    let page = state
        .ops
        .garyx_db
        .run_blocking(|db| db.list_thread_favorites())
        .await
        .ok();
    thread_favorites_tagged_error(
        StatusCode::BAD_REQUEST,
        operation,
        "invalid_request",
        message,
        page.as_ref(),
        state.server_boot_id(),
        json!({}),
    )
}

fn parse_reorder_thread_pins_request(payload: &Value) -> Result<(Vec<String>, i64), GaryxDbError> {
    let object = payload
        .as_object()
        .ok_or_else(|| GaryxDbError::BadRequest("request body must be a JSON object".to_owned()))?;
    let expected_revision = object
        .get("expected_revision")
        .and_then(Value::as_i64)
        .filter(|revision| *revision >= 0)
        .ok_or_else(|| {
            GaryxDbError::BadRequest("expected_revision must be a non-negative integer".to_owned())
        })?;
    let values = object
        .get("thread_ids")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| {
            GaryxDbError::BadRequest("thread_ids must be a non-empty array".to_owned())
        })?;
    let mut thread_ids = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let thread_id = value
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                GaryxDbError::BadRequest(
                    "thread_ids must contain only non-empty strings".to_owned(),
                )
            })?;
        if !seen.insert(thread_id.to_owned()) {
            return Err(GaryxDbError::BadRequest(format!(
                "duplicate thread_id: {thread_id}"
            )));
        }
        thread_ids.push(thread_id.to_owned());
    }
    Ok((thread_ids, expected_revision))
}

async fn recent_threads_payload(
    state: &Arc<AppState>,
    records: &[RecentThreadRecord],
    filter: RecentThreadTaskFilter,
    limit: usize,
    total: usize,
    has_more: bool,
    store_incarnation_id: &str,
) -> Value {
    let threads = recent_thread_values(state, records).await;
    let next_cursor = has_more.then(|| {
        let last = records
            .last()
            .expect("a positive page limit with has_more must return a row");
        encode_recent_threads_cursor(filter, last.activity_seq)
    });
    json!({
        "store_incarnation_id": store_incarnation_id,
        "server_boot_id": state.server_boot_id(),
        "threads": threads,
        "count": records.len(),
        "limit": limit,
        "total": total,
        "has_more": has_more,
        "next_cursor": next_cursor,
    })
}

fn recent_threads_invalid_request(message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "kind": "garyx_api_error",
            "operation": RECENT_THREADS_LIST_OPERATION,
            "code": "invalid_request",
            "message": message.into(),
        })),
    )
        .into_response()
}

fn thread_summaries_invalid_request(message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "kind": "garyx_api_error",
            "operation": "thread_summaries_list",
            "code": "invalid_request",
            "message": message.into(),
        })),
    )
        .into_response()
}

async fn recent_thread_values(state: &Arc<AppState>, records: &[RecentThreadRecord]) -> Vec<Value> {
    let mut threads = Vec::with_capacity(records.len());
    let catalog = AgentCatalogSnapshot::load(state).await;
    for record in records {
        let mut thread = serde_json::to_value(record).unwrap_or(Value::Null);
        attach_thread_runtime_summary_with_catalog(state, &record.thread_id, &mut thread, &catalog)
            .await;
        threads.push(thread);
    }
    threads
}

fn garyx_db_error_response(error: GaryxDbError) -> (StatusCode, Json<Value>) {
    let (status, code) = match &error {
        GaryxDbError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BadRequest"),
        GaryxDbError::ThreadArchived(_) => (StatusCode::GONE, "ThreadArchived"),
        GaryxDbError::LockPoisoned
        | GaryxDbError::Join(_)
        | GaryxDbError::Configuration(_)
        | GaryxDbError::DataDirLocked { .. }
        | GaryxDbError::ParentHandoffTimedOut { .. }
        | GaryxDbError::Io(_)
        | GaryxDbError::Sqlite(_) => (StatusCode::INTERNAL_SERVER_ERROR, "InternalError"),
    };
    (
        status,
        Json(json!({
            "error": code,
            "message": error.to_string(),
        })),
    )
}

async fn thread_metadata_response(state: &Arc<AppState>, thread_id: &str, data: &Value) -> Value {
    let mut value = data.clone();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("thread_mode");
        obj.entry("thread_id".to_owned())
            .or_insert_with(|| Value::String(thread_id.to_owned()));
        obj.entry("thread_key".to_owned())
            .or_insert_with(|| Value::String(thread_id.to_owned()));
        obj.insert(
            "thread_type".to_owned(),
            Value::String(thread_summary_type_from_record(data)),
        );
        obj.insert(
            "thread_runtime".to_owned(),
            build_thread_runtime_summary(state, Some(data)).await,
        );
    }
    value
}

async fn attach_thread_runtime_summary_with_catalog(
    state: &Arc<AppState>,
    thread_id: &str,
    summary: &mut Value,
    catalog: &AgentCatalogSnapshot,
) {
    let thread_value = state.threads.thread_store.get_logged(thread_id).await;
    if let Some(obj) = summary.as_object_mut() {
        obj.insert(
            "thread_runtime".to_owned(),
            build_thread_runtime_summary_with_catalog(state, thread_value.as_ref(), catalog),
        );
    }
}

/// GET /api/threads - list threads with filtering and pagination.
pub async fn list_threads(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListThreadsParams>,
) -> impl IntoResponse {
    let limit = params.limit.min(MAX_THREAD_LIMIT);
    let include_hidden = params.include_hidden;
    let prefix = params.prefix.clone();
    let requested_offset = params.offset;
    // Count + page in one blocking hop: SQLite work must not hold a runtime
    // worker (#TASK-1829 batch 3).
    let paged = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            let total = db.count_thread_meta_list(include_hidden, prefix.as_deref())?;
            let offset = requested_offset.min(total);
            let records =
                db.list_thread_meta_page(limit, offset, include_hidden, prefix.as_deref())?;
            Ok((total, offset, records))
        })
        .await;
    let (total, offset, records) = match paged {
        Ok(paged) => paged,
        Err(error) => return garyx_db_error_response(error).into_response(),
    };
    let catalog = AgentCatalogSnapshot::load(&state).await;
    let mut page = Vec::with_capacity(records.len());
    for record in &records {
        let mut summary = thread_summary_from_meta(record);
        if let Some(obj) = summary.as_object_mut() {
            obj.insert(
                "thread_runtime".to_owned(),
                build_thread_runtime_summary_from_meta(&state, record, &catalog),
            );
        }
        page.push(summary);
    }
    let count = page.len();

    (
        StatusCode::OK,
        Json(json!({
        "threads": page,
        "count": count,
        "total": total,
        "limit": limit,
        "offset": offset,
        })),
    )
        .into_response()
}

/// GET /api/recent-threads - list recently active threads for compact clients.
pub async fn list_recent_threads(
    State(state): State<Arc<AppState>>,
    query: Result<Query<ListRecentThreadsParams>, axum::extract::rejection::QueryRejection>,
) -> impl IntoResponse {
    let (filter, limit, before_activity_seq) = match parse_recent_threads_params(query) {
        Ok(params) => params,
        Err(message) => return recent_threads_invalid_request(message),
    };
    let paged = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            let page = db.list_recent_threads_keyset_page(filter, limit, before_activity_seq)?;
            let store_incarnation_id = db.store_incarnation_id()?;
            Ok((page, store_incarnation_id))
        })
        .await;
    match paged {
        Ok((page, store_incarnation_id)) => (
            StatusCode::OK,
            Json(
                recent_threads_payload(
                    &state,
                    &page.records,
                    filter,
                    limit,
                    page.total,
                    page.has_more,
                    &store_incarnation_id,
                )
                .await,
            ),
        )
            .into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/thread-summaries - keyset-paged canonical thread summaries.
pub async fn list_thread_summaries(
    State(state): State<Arc<AppState>>,
    query: Result<Query<ListThreadSummariesParams>, axum::extract::rejection::QueryRejection>,
) -> impl IntoResponse {
    let params = match parse_thread_summaries_params(query) {
        Ok(params) => params,
        Err(message) => return thread_summaries_invalid_request(message),
    };
    let workspace_dir = params.workspace_dir.clone();
    let normalized_query = params.query.clone();
    let cursor_key = params
        .cursor
        .as_ref()
        .map(|cursor| (cursor.sort_key, cursor.thread_id.clone()));
    let cursor_incarnation = params
        .cursor
        .as_ref()
        .map(|cursor| cursor.incarnation.clone());
    let filter = params.filter;
    let limit = params.limit;
    let paged = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            db.list_thread_summaries_keyset_page(
                filter,
                workspace_dir.as_deref(),
                normalized_query.as_deref(),
                limit,
                cursor_key
                    .as_ref()
                    .map(|(sort_key, thread_id)| (*sort_key, thread_id.as_str())),
                cursor_incarnation.as_deref(),
            )
        })
        .await;
    match paged {
        Ok(page) => {
            let next_cursor = page.has_more.then(|| {
                let last = page
                    .records
                    .last()
                    .expect("a positive summary page limit with has_more must return a row");
                encode_thread_summaries_cursor(
                    &params.scope,
                    params.filter,
                    params.query.as_deref(),
                    &page.store_incarnation_id,
                    last.sort_updated_at_us,
                    &last.thread_id,
                )
            });
            (
                StatusCode::OK,
                Json(json!({
                    "threads": page.records,
                    "next_cursor": next_cursor,
                    "has_more": page.has_more,
                    "store_incarnation_id": page.store_incarnation_id,
                    "server_boot_id": state.server_boot_id(),
                })),
            )
                .into_response()
        }
        Err(GaryxDbError::BadRequest(message)) => thread_summaries_invalid_request(message),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/threads/:key - get thread metadata
pub async fn get_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            );
        }
        Err(response) => return response,
    };
    match state.threads.thread_store.get(&thread_id).await {
        Ok(Some(data)) => (
            StatusCode::OK,
            Json(thread_metadata_response(&state, &thread_id, &data).await),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "thread not found"})),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error.to_string()})),
        ),
    }
}

/// GET /api/thread-pins - list pinned thread ids in display order.
pub async fn list_thread_pins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state
        .ops
        .garyx_db
        .run_blocking(|db| db.list_pinned_threads())
        .await
    {
        Ok(page) => (StatusCode::OK, Json(thread_pins_payload(&page))).into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/thread-favorites - one atomic membership/revision/identity page.
pub async fn list_thread_favorites(State(state): State<Arc<AppState>>) -> axum::response::Response {
    match state
        .ops
        .garyx_db
        .run_blocking(|db| db.list_thread_favorites())
        .await
    {
        Ok(page) => (
            StatusCode::OK,
            Json(thread_favorites_payload(&page, state.server_boot_id())),
        )
            .into_response(),
        Err(error) => thread_favorites_tagged_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            THREAD_FAVORITES_GET_OPERATION,
            "unavailable",
            error.to_string(),
            None,
            state.server_boot_id(),
            json!({}),
        ),
    }
}

/// GET /api/thread-favorites/snapshot - membership and joined recent rows
/// from one SQLite read transaction.
pub async fn thread_favorites_snapshot(
    State(state): State<Arc<AppState>>,
    query: Result<Query<ThreadFavoritesSnapshotQuery>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => {
            return tagged_favorites_invalid_request(
                &state,
                THREAD_FAVORITES_SNAPSHOT_OPERATION,
                error.to_string(),
            )
            .await;
        }
    };
    if query.include_summaries.unwrap_or(false) {
        return match state
            .ops
            .garyx_db
            .run_blocking(|db| db.thread_favorites_snapshot_with_summaries())
            .await
        {
            Ok(enhanced) => {
                let threads = recent_thread_values(&state, &enhanced.snapshot.recent_threads).await;
                let mut payload =
                    thread_favorites_payload(&enhanced.snapshot.page, state.server_boot_id());
                extend_json_object(
                    &mut payload,
                    json!({
                        "recent": {
                            "threads": threads,
                            "total": enhanced.snapshot.recent_total,
                            "truncated": enhanced.snapshot.recent_truncated,
                        },
                        "summaries": enhanced.summaries,
                        "summaries_truncated": enhanced.summaries_truncated,
                    }),
                );
                (StatusCode::OK, Json(payload)).into_response()
            }
            Err(error) => thread_favorites_tagged_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                THREAD_FAVORITES_SNAPSHOT_OPERATION,
                "unavailable",
                error.to_string(),
                None,
                state.server_boot_id(),
                json!({}),
            ),
        };
    }
    match state
        .ops
        .garyx_db
        .run_blocking(|db| db.thread_favorites_snapshot())
        .await
    {
        Ok(snapshot) => {
            let threads = recent_thread_values(&state, &snapshot.recent_threads).await;
            let mut payload = thread_favorites_payload(&snapshot.page, state.server_boot_id());
            extend_json_object(
                &mut payload,
                json!({
                    "recent": {
                        "threads": threads,
                        "total": snapshot.recent_total,
                        "truncated": snapshot.recent_truncated,
                    }
                }),
            );
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(error) => thread_favorites_tagged_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            THREAD_FAVORITES_SNAPSHOT_OPERATION,
            "unavailable",
            error.to_string(),
            None,
            state.server_boot_id(),
            json!({}),
        ),
    }
}

/// PUT /api/thread-favorites/:key - conditionally favorite one thread.
pub async fn favorite_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    mutate_thread_favorite(state, key, query, true, THREAD_FAVORITES_PUT_OPERATION).await
}

/// DELETE /api/thread-favorites/:key - conditionally unfavorite one thread.
pub async fn unfavorite_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    mutate_thread_favorite(state, key, query, false, THREAD_FAVORITES_DELETE_OPERATION).await
}

async fn mutate_thread_favorite(
    state: Arc<AppState>,
    key: String,
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
    favorited: bool,
    operation: &'static str,
) -> axum::response::Response {
    let (expected_revision, expected_store_incarnation) =
        match parse_thread_favorites_mutation_query(query) {
            Ok(expected) => expected,
            Err(message) => {
                return tagged_favorites_invalid_request(&state, operation, message).await;
            }
        };
    let thread_id = key.trim().to_owned();
    if !is_thread_key(&thread_id) {
        return tagged_favorites_invalid_request(
            &state,
            operation,
            "thread key must use the thread:: prefix".to_owned(),
        )
        .await;
    }
    let mutation_thread_id = thread_id.clone();
    let mutation_expected_store_incarnation = expected_store_incarnation.clone();
    let result = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            db.set_thread_favorite(
                &mutation_thread_id,
                favorited,
                expected_revision,
                &mutation_expected_store_incarnation,
            )
        })
        .await;
    match result {
        Ok(FavoriteThreadResult::Updated { changed, page }) => {
            let mut payload = thread_favorites_payload(&page, state.server_boot_id());
            extend_json_object(
                &mut payload,
                if favorited {
                    json!({
                        "favorited": true,
                        "changed": changed,
                        "thread_id": thread_id,
                    })
                } else {
                    json!({
                        "favorited": false,
                        "removed": changed,
                        "thread_id": thread_id,
                    })
                },
            );
            (StatusCode::OK, Json(payload)).into_response()
        }
        Ok(FavoriteThreadResult::Conflict(page)) => thread_favorites_tagged_error(
            StatusCode::CONFLICT,
            operation,
            "conflict",
            "favorites revision does not match",
            Some(&page),
            state.server_boot_id(),
            json!({
                "conflict": true,
                "expected_revision": expected_revision,
                "favorited": page.favorites.iter().any(|item| item.thread_id == thread_id),
            }),
        ),
        Ok(FavoriteThreadResult::WrongIncarnation(page)) => thread_favorites_tagged_error(
            StatusCode::CONFLICT,
            operation,
            "wrong_incarnation",
            "store incarnation does not match",
            Some(&page),
            state.server_boot_id(),
            json!({
                "expected_store_incarnation": expected_store_incarnation,
                "favorited": page.favorites.iter().any(|item| item.thread_id == thread_id),
            }),
        ),
        Ok(FavoriteThreadResult::NotFound(page)) => thread_favorites_tagged_error(
            StatusCode::NOT_FOUND,
            operation,
            "not_found",
            format!("thread not found: {thread_id}"),
            Some(&page),
            state.server_boot_id(),
            json!({
                "favorited": false,
                "thread_id": thread_id,
            }),
        ),
        Err(error) => {
            let (status, code) = if matches!(error, GaryxDbError::BadRequest(_)) {
                (StatusCode::BAD_REQUEST, "invalid_request")
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, "unavailable")
            };
            thread_favorites_tagged_error(
                status,
                operation,
                code,
                error.to_string(),
                None,
                state.server_boot_id(),
                json!({}),
            )
        }
    }
}

/// PUT /api/thread-pins - reorder the pinned collection with revision CAS.
pub async fn reorder_thread_pins(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let (thread_ids, expected_revision) = match parse_reorder_thread_pins_request(&payload) {
        Ok(request) => request,
        Err(error) => return garyx_db_error_response(error).into_response(),
    };
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.reorder_thread_pins(thread_ids, expected_revision))
        .await
    {
        Ok(ReorderThreadPinsResult::Updated(page)) => {
            (StatusCode::OK, Json(thread_pins_payload(&page))).into_response()
        }
        Ok(ReorderThreadPinsResult::Conflict(page)) => {
            (StatusCode::CONFLICT, Json(thread_pins_payload(&page))).into_response()
        }
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// PUT /api/thread-pins/:key - mark a thread as pinned.
pub async fn pin_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"pinned": false, "error": "thread not found"})),
            )
                .into_response();
        }
        Err(response) => return response.into_response(),
    };
    let pin_thread_id = thread_id.clone();
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.pin_thread(&pin_thread_id))
        .await
    {
        Ok(page) => {
            let pin = page
                .pins
                .iter()
                .find(|record| record.thread_id == thread_id)
                .cloned();
            (
                StatusCode::OK,
                Json(json!({
                "pinned": true,
                "pin": pin,
                "thread_ids": thread_pin_ids(&page),
                "pins": page.pins,
                "revision": page.revision,
                })),
            )
                .into_response()
        }
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// DELETE /api/thread-pins/:key - remove a thread pin.
pub async fn unpin_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(resolved) => resolved.unwrap_or_else(|| key.trim().to_owned()),
        Err(response) => return response.into_response(),
    };
    let unpin_thread_id = thread_id.clone();
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.unpin_thread(&unpin_thread_id))
        .await
    {
        Ok((removed, page)) => (
            StatusCode::OK,
            Json(json!({
                "pinned": false,
                "removed": removed,
                "thread_id": thread_id,
                "thread_ids": thread_pin_ids(&page),
                "pins": page.pins,
                "revision": page.revision,
            })),
        )
            .into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/threads/:key/logs - get full or incremental thread log content
pub async fn get_thread_logs(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Query(params): Query<ThreadLogParams>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            );
        }
        Err(response) => return response,
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

#[derive(Deserialize)]
pub struct ThreadStreamParams {
    /// Resume cursor: replay committed messages with seq strictly greater than this.
    #[serde(default)]
    pub after_seq: u64,
    #[serde(default)]
    pub replay_scope: Option<ThreadStreamReplayScope>,
    #[serde(default)]
    pub initial_user_turns: Option<usize>,
    #[serde(default)]
    pub render_floor: Option<u64>,
    /// Capability negotiation (#TASK-1956 knife 1): `delta` declares that
    /// live frames may carry `render_delta` instead of a full
    /// `render_state`. Undeclared connections receive full frames
    /// indefinitely — a permanent contract surface, not a transition flag.
    #[serde(default)]
    pub render_mode: Option<ThreadStreamRenderMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStreamReplayScope {
    Resume,
    Initial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStreamRenderMode {
    Full,
    Delta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThreadStreamReplayOptions {
    replay_scope: ThreadStreamReplayScope,
    initial_user_turns: Option<usize>,
    render_floor: u64,
}

/// Serialized-replay byte budget for resume connections; over this, any
/// resume degrades to the initial window — unconditionally, for every
/// consumer (design: thread-render-frame-incremental.md knife 2).
const THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET: usize = 1024 * 1024;
/// User-turn window served for a degraded resume — same default the
/// desktop and iOS cold-open planners use.
const THREAD_STREAM_DEGRADED_RESUME_USER_TURNS: usize = 3;

#[cfg(test)]
impl ThreadStreamReplayOptions {
    fn resume(render_floor: u64) -> Self {
        Self {
            replay_scope: ThreadStreamReplayScope::Resume,
            initial_user_turns: None,
            render_floor,
        }
    }
}

fn thread_stream_replay_options(
    params: &ThreadStreamParams,
    last_event_id: Option<u64>,
    has_last_event_id: bool,
) -> (u64, ThreadStreamReplayOptions) {
    let after_seq = last_event_id.unwrap_or(params.after_seq);
    let replay_scope = if has_last_event_id {
        ThreadStreamReplayScope::Resume
    } else {
        params
            .replay_scope
            .unwrap_or(ThreadStreamReplayScope::Resume)
    };
    let initial_user_turns = match replay_scope {
        ThreadStreamReplayScope::Initial => params.initial_user_turns,
        ThreadStreamReplayScope::Resume => None,
    };
    (
        after_seq,
        ThreadStreamReplayOptions {
            replay_scope,
            initial_user_turns,
            render_floor: params.render_floor.unwrap_or(0),
        },
    )
}

/// GET /api/threads/:key/stream - resumable per-thread transcript stream (S5).
///
/// Replays committed messages with `seq > after_seq` (or the `Last-Event-ID`
/// header on reconnect), then streams that thread's live events. The bus is
/// subscribed BEFORE the replay snapshot is read so no commit is missed in the
/// gap, and exact duplicate `committed_message` payloads are deduped so the
/// resulting replay/live overlap is idempotent while same-seq overwrite events
/// still reach clients.
pub async fn thread_stream(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Query(params): Query<ThreadStreamParams>,
    headers: HeaderMap,
) -> axum::response::Response {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            )
                .into_response();
        }
        Err(response) => return response.into_response(),
    };

    // Resume via Last-Event-ID (standard SSE) or the after_seq query param.
    let last_event_id_header = headers.get("last-event-id");
    let last_event_id = last_event_id_header
        .as_ref()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let (after_seq, replay_options) =
        thread_stream_replay_options(&params, last_event_id, last_event_id_header.is_some());

    // Subscribe BEFORE reading the replay snapshot (no gap); seq dedup below makes
    // the overlap idempotent.
    let rx = state.ops.events.subscribe();

    // Delta negotiation (#TASK-1956 knife 1): declared connections get a
    // per-connection diff base; every full frame (replay/snapshot-only
    // below, first-live/same-seq-reseed in the live loop) seeds it.
    let delta_base: Option<Arc<ThreadStreamDeltaBase>> = (params.render_mode
        == Some(ThreadStreamRenderMode::Delta))
    .then(|| Arc::new(std::sync::Mutex::new(None)));

    tracing::info!(
        thread_id = %thread_id,
        after_seq,
        render_floor = replay_options.render_floor,
        replay_scope = ?replay_options.replay_scope,
        render_delta = delta_base.is_some(),
        "per-thread stream connected"
    );

    let replay = build_thread_stream_replay(
        &state,
        &thread_id,
        after_seq,
        replay_options,
        delta_base.as_deref(),
    )
    .await;
    let render_floor_for_live = replay.render_floor;
    let replay_events = replay
        .events
        .into_iter()
        .map(|event| event.map(ThreadStreamEvent::into_sse_event));
    let mut sent_committed_payloads = replay.sent_payloads;

    let thread_for_live = thread_id.clone();
    let state_for_live = state.clone();
    let state_for_drops = state.clone();
    let mut last_sent_seq = replay.max_seq;
    let live = BroadcastStream::new(rx)
        .then(move |item| {
            let state_for_live = state_for_live.clone();
            let thread_for_live = thread_for_live.clone();
            let delta_base_for_live = delta_base.clone();
            let forwarded = match item {
                Ok(raw) => committed_thread_stream_live_payload(
                    &raw,
                    &thread_for_live,
                    &mut sent_committed_payloads,
                    &mut last_sent_seq,
                ),
                Err(_) => {
                    // Lagged: a slow consumer dropped events. Terminate this SSE
                    // response so the client reconnects from the last delivered seq and
                    // the file-backed replay fills the gap.
                    state_for_drops.ops.events.record_drop();
                    Err(thread_stream_reconnect_error("broadcast lagged"))
                }
            };
            async move {
                match forwarded {
                    Ok(Some((seq, payload))) => Some(
                        committed_thread_stream_live_event(
                            &state_for_live,
                            &thread_for_live,
                            seq,
                            payload,
                            render_floor_for_live,
                            delta_base_for_live.as_deref(),
                        )
                        .await,
                    ),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                }
            }
        })
        .filter_map(|event| async move {
            event.map(|event| event.map(ThreadStreamEvent::into_sse_event))
        });

    let combined = tokio_stream::iter(replay_events).chain(live);
    Sse::new(combined)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("ping"),
        )
        .into_response()
}

struct ThreadStreamReplay {
    events: Vec<Result<ThreadStreamEvent, io::Error>>,
    max_seq: u64,
    sent_payloads: HashMap<u64, String>,
    render_floor: u64,
}

/// Per-connection delta base for `render_mode=delta` connections
/// (#TASK-1956 knife 1): the seq and rows digest of the last frame this
/// connection sent. `None` until the first full frame seeds it. Seeding
/// rule (one rule, everywhere): every frame that carries a full
/// `render_state` — replay, snapshot-only, or a full live frame — resets
/// this base to that frame's snapshot; the very next live frame may be a
/// delta. Wrapped in a `Mutex` only because the live stream's future
/// cannot borrow closure captures; access is strictly sequential.
type ThreadStreamDeltaBase = std::sync::Mutex<Option<ThreadStreamRenderBase>>;

struct ThreadStreamRenderBase {
    seq: u64,
    rows_hash: u64,
    row_hashes: HashMap<String, u64>,
}

fn lock_thread_stream_delta_base(
    base: &ThreadStreamDeltaBase,
) -> std::sync::MutexGuard<'_, Option<ThreadStreamRenderBase>> {
    // A poisoned base only means a panic elsewhere while seeding; the
    // cached digest is still coherent (worst case the next frame goes
    // out full and reseeds), so recover instead of propagating the panic.
    base.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Seed the connection's delta base from a full snapshot and stamp the
/// snapshot with its `rows_hash` chain token. Called by every full-frame
/// constructor on delta connections.
fn seed_thread_stream_delta_base(base: &ThreadStreamDeltaBase, render_state: &mut RenderSnapshot) {
    let digest = garyx_models::render_rows_digest(&render_state.rows);
    render_state.rows_hash = Some(digest.rows_hash);
    *lock_thread_stream_delta_base(base) = Some(ThreadStreamRenderBase {
        seq: render_state.based_on_seq,
        rows_hash: digest.rows_hash,
        row_hashes: digest.row_hashes,
    });
}

struct ThreadStreamReplayBuilder {
    event_payloads: Vec<Value>,
    max_seq: u64,
    sent_payloads: HashMap<u64, String>,
    serialized_bytes: usize,
}

struct ThreadStreamEvent {
    id: u64,
    payload: String,
}

impl ThreadStreamEvent {
    fn into_sse_event(self) -> Event {
        Event::default().id(self.id.to_string()).data(self.payload)
    }
}

async fn build_thread_stream_replay(
    state: &Arc<AppState>,
    thread_id: &str,
    after_seq: u64,
    options: ThreadStreamReplayOptions,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    if matches!(options.replay_scope, ThreadStreamReplayScope::Initial) {
        if let Some(initial_user_turns) = options.initial_user_turns {
            let window = state
                .threads
                .history
                .transcript_store()
                .cold_open_user_turn_window(
                    thread_id,
                    initial_user_turns,
                    THREAD_TRANSCRIPT_REPLAY_CAP,
                )
                .await
                .unwrap_or_else(|_| garyx_router::ThreadTranscriptWindow {
                    records: Vec::new(),
                    floor_seq: 0,
                    has_more_above: false,
                });
            return thread_stream_replay_from_records(
                state,
                thread_id,
                after_seq,
                window.records,
                window.floor_seq,
                delta_base,
            )
            .await;
        }
    }

    let tail = state
        .threads
        .history
        .transcript_store()
        .records_after_seq(thread_id, after_seq, THREAD_TRANSCRIPT_REPLAY_CAP)
        .await
        .unwrap_or_default();

    let tail_has_gap = tail
        .first()
        .is_some_and(|record| record.seq > after_seq.saturating_add(1));
    if !tail_has_gap {
        let mut replay = ThreadStreamReplayBuilder {
            event_payloads: Vec::with_capacity(tail.len()),
            max_seq: after_seq,
            sent_payloads: HashMap::new(),
            serialized_bytes: 0,
        };
        append_thread_stream_replay_records(&mut replay, thread_id, tail);
        // Stale resume over the byte budget: abandon the span replay and
        // serve the initial window instead — unconditionally, for every
        // consumer (design: thread-render-frame-incremental.md knife 2).
        if replay.serialized_bytes > THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET {
            return degraded_windowed_resume_replay(state, thread_id, after_seq, delta_base).await;
        }
        return finalize_thread_stream_replay(
            state,
            thread_id,
            replay,
            options.render_floor,
            None,
            delta_base,
        )
        .await;
    }

    // Gap self-heal: page the span in forward. A sub-budget gap keeps the
    // verbatim paged replay; the moment the accumulated serialization
    // crosses the byte budget the resume degrades to the window instead of
    // paging in megabytes (design: thread-render-frame-incremental.md
    // knife 2).
    let mut cursor = after_seq;
    let mut replay = ThreadStreamReplayBuilder {
        event_payloads: Vec::new(),
        max_seq: after_seq,
        sent_payloads: HashMap::new(),
        serialized_bytes: 0,
    };
    loop {
        let page = state
            .threads
            .history
            .transcript_store()
            .records_after_seq_page(thread_id, cursor, THREAD_TRANSCRIPT_REPLAY_CAP)
            .await
            .unwrap_or_default();
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        append_thread_stream_replay_records(&mut replay, thread_id, page);
        if replay.serialized_bytes > THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET {
            return degraded_windowed_resume_replay(state, thread_id, after_seq, delta_base).await;
        }
        if replay.max_seq == cursor || page_len < THREAD_TRANSCRIPT_REPLAY_CAP {
            break;
        }
        cursor = replay.max_seq;
    }
    finalize_thread_stream_replay(
        state,
        thread_id,
        replay,
        options.render_floor,
        None,
        delta_base,
    )
    .await
}

/// Serve an over-budget stale resume as the initial window: same records a
/// `replay_scope=initial` connection would get, marked
/// `replay:"windowed"` so the client rebuilds from the window instead of
/// appending.
async fn degraded_windowed_resume_replay(
    state: &Arc<AppState>,
    thread_id: &str,
    after_seq: u64,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    let window = state
        .threads
        .history
        .transcript_store()
        .cold_open_user_turn_window(
            thread_id,
            THREAD_STREAM_DEGRADED_RESUME_USER_TURNS,
            THREAD_TRANSCRIPT_REPLAY_CAP,
        )
        .await
        .unwrap_or_else(|_| garyx_router::ThreadTranscriptWindow {
            records: Vec::new(),
            floor_seq: 0,
            has_more_above: false,
        });
    let mut replay = ThreadStreamReplayBuilder {
        event_payloads: Vec::with_capacity(window.records.len()),
        max_seq: after_seq,
        sent_payloads: HashMap::new(),
        serialized_bytes: 0,
    };
    append_thread_stream_replay_records(&mut replay, thread_id, window.records);
    finalize_thread_stream_replay(
        state,
        thread_id,
        replay,
        window.floor_seq,
        Some("windowed"),
        delta_base,
    )
    .await
}

async fn thread_stream_replay_from_records(
    state: &Arc<AppState>,
    thread_id: &str,
    after_seq: u64,
    records: Vec<ThreadTranscriptRecord>,
    render_floor: u64,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    let mut replay = ThreadStreamReplayBuilder {
        event_payloads: Vec::with_capacity(records.len()),
        max_seq: after_seq,
        sent_payloads: HashMap::new(),
        serialized_bytes: 0,
    };
    append_thread_stream_replay_records(&mut replay, thread_id, records);
    finalize_thread_stream_replay(state, thread_id, replay, render_floor, None, delta_base).await
}

async fn finalize_thread_stream_replay(
    state: &Arc<AppState>,
    thread_id: &str,
    replay: ThreadStreamReplayBuilder,
    render_floor: u64,
    replay_kind: Option<&'static str>,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    let mut events = Vec::new();
    let mut max_seq = replay.max_seq;
    if !replay.event_payloads.is_empty() {
        let event = thread_stream_frame_event(
            state,
            thread_id,
            replay.max_seq,
            replay.event_payloads,
            render_floor,
            replay_kind,
            delta_base,
        )
        .await;
        events.push(event);
    } else {
        let event = thread_stream_snapshot_only_frame_event(
            state,
            thread_id,
            replay.max_seq,
            render_floor,
            replay_kind,
            delta_base,
        )
        .await;
        if let Ok(event) = &event {
            max_seq = event.id;
        }
        events.push(event);
    }
    ThreadStreamReplay {
        events,
        max_seq,
        sent_payloads: replay.sent_payloads,
        render_floor,
    }
}

fn append_thread_stream_replay_records(
    replay: &mut ThreadStreamReplayBuilder,
    thread_id: &str,
    records: Vec<ThreadTranscriptRecord>,
) {
    for record in records {
        replay.max_seq = replay.max_seq.max(record.seq);
        let payload = committed_thread_stream_replay_payload_value(thread_id, &record);
        let serialized = payload.to_string();
        replay.serialized_bytes += serialized.len();
        replay.sent_payloads.insert(record.seq, serialized);
        replay.event_payloads.push(payload);
    }
}

fn committed_thread_stream_replay_payload_value(
    thread_id: &str,
    record: &ThreadTranscriptRecord,
) -> Value {
    json!({
        "type": "committed_message",
        "thread_id": thread_id,
        "run_id": record.run_id.as_deref(),
        "seq": record.seq,
        "message": &record.message,
    })
}

fn committed_thread_stream_live_payload(
    raw: &str,
    thread_id: &str,
    sent_payloads: &mut HashMap<u64, String>,
    last_sent_seq: &mut u64,
) -> Result<Option<(u64, Value)>, io::Error> {
    let value: Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("thread_id").and_then(Value::as_str) != Some(thread_id) {
        return Ok(None);
    }
    if value.get("type").and_then(Value::as_str) != Some("committed_message") {
        return Ok(None);
    }
    let seq = value.get("seq").and_then(Value::as_u64).unwrap_or(0);
    match should_forward_committed_payload(sent_payloads, last_sent_seq, seq, raw) {
        CommittedPayloadAction::Forward => Ok(Some((seq, value))),
        CommittedPayloadAction::Skip => Ok(None),
        CommittedPayloadAction::Reconnect => Err(thread_stream_reconnect_error(
            "non-contiguous committed seq",
        )),
    }
}

async fn committed_thread_stream_live_event(
    state: &Arc<AppState>,
    thread_id: &str,
    seq: u64,
    payload: Value,
    render_floor: u64,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> Result<ThreadStreamEvent, io::Error> {
    let Some(delta_base) = delta_base else {
        // Undeclared connection: full frames, byte-identical to the
        // pre-delta contract (no rows_hash).
        return thread_stream_frame_event(
            state,
            thread_id,
            seq,
            vec![payload],
            render_floor,
            None,
            None,
        )
        .await;
    };
    let mut render_state =
        thread_render_snapshot_at_seq(state, thread_id, seq, render_floor).await?;
    if render_state.based_on_seq != seq {
        return Err(thread_stream_reconnect_error(
            "render snapshot seq mismatch",
        ));
    }
    let digest = garyx_models::render_rows_digest(&render_state.rows);
    let delta = {
        let mut guard = lock_thread_stream_delta_base(delta_base);
        let delta = match guard.as_ref() {
            // Base strictly behind this frame: normal delta step.
            Some(base) if base.seq < seq => Some(garyx_models::derive_render_delta_from_base(
                base.seq,
                base.rows_hash,
                &base.row_hashes,
                &render_state,
                digest.rows_hash,
            )),
            // Same-seq overwrite (a changed payload re-landed on
            // `seq == last_sent_seq`; design "Same-seq overwrites") or no
            // base yet: emit a full frame instead of a delta.
            _ => None,
        };
        // Either way this frame becomes the new base (the seeding rule for
        // full frames; for delta frames the chain simply advances).
        *guard = Some(ThreadStreamRenderBase {
            seq,
            rows_hash: digest.rows_hash,
            row_hashes: digest.row_hashes,
        });
        delta
    };
    match delta {
        Some(delta) => Ok(ThreadStreamEvent {
            id: seq,
            payload: thread_stream_delta_frame_payload(thread_id, vec![payload], &delta),
        }),
        None => {
            render_state.rows_hash = Some(digest.rows_hash);
            Ok(ThreadStreamEvent {
                id: seq,
                payload: thread_stream_frame_payload(thread_id, vec![payload], &render_state, None),
            })
        }
    }
}

async fn thread_stream_snapshot_only_frame_event(
    state: &Arc<AppState>,
    thread_id: &str,
    requested_seq: u64,
    render_floor: u64,
    replay_kind: Option<&'static str>,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> Result<ThreadStreamEvent, io::Error> {
    let mut render_state =
        thread_render_snapshot_at_seq(state, thread_id, requested_seq, render_floor).await?;
    if let Some(delta_base) = delta_base {
        seed_thread_stream_delta_base(delta_base, &mut render_state);
    }
    let id = render_state.based_on_seq;
    Ok(ThreadStreamEvent {
        id,
        payload: thread_stream_frame_payload(thread_id, Vec::new(), &render_state, replay_kind),
    })
}

async fn thread_stream_frame_event(
    state: &Arc<AppState>,
    thread_id: &str,
    seq: u64,
    event_payloads: Vec<Value>,
    render_floor: u64,
    replay_kind: Option<&'static str>,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> Result<ThreadStreamEvent, io::Error> {
    let mut render_state =
        thread_render_snapshot_at_seq(state, thread_id, seq, render_floor).await?;
    if render_state.based_on_seq != seq {
        return Err(thread_stream_reconnect_error(
            "render snapshot seq mismatch",
        ));
    }
    if let Some(delta_base) = delta_base {
        seed_thread_stream_delta_base(delta_base, &mut render_state);
    }
    Ok(ThreadStreamEvent {
        id: seq,
        payload: thread_stream_frame_payload(thread_id, event_payloads, &render_state, replay_kind),
    })
}

async fn thread_render_snapshot_at_seq(
    state: &Arc<AppState>,
    thread_id: &str,
    seq: u64,
    render_floor: u64,
) -> Result<RenderSnapshot, io::Error> {
    let store = state.threads.history.transcript_store();
    let result = if render_floor > 0 {
        store
            .render_snapshot_in_window(thread_id, render_floor, seq)
            .await
    } else {
        store.render_snapshot_at_seq(thread_id, seq).await
    };
    result.map_err(|error| io::Error::other(format!("failed to derive render snapshot: {error}")))
}

fn thread_stream_frame_payload(
    thread_id: &str,
    event_payloads: Vec<Value>,
    render_state: &RenderSnapshot,
    replay_kind: Option<&'static str>,
) -> String {
    let mut payload = json!({
        "type": "thread_render_frame",
        "thread_id": thread_id,
        "events": event_payloads,
        "render_state": render_state,
    });
    if let (Some(kind), Some(obj)) = (replay_kind, payload.as_object_mut()) {
        obj.insert("replay".to_owned(), Value::String(kind.to_owned()));
    }
    payload.to_string()
}

/// Live frame for a `render_mode=delta` connection: `render_delta`
/// replaces `render_state`; `events` stay the cursor/body source of
/// truth, unchanged.
fn thread_stream_delta_frame_payload(
    thread_id: &str,
    event_payloads: Vec<Value>,
    render_delta: &garyx_models::RenderDelta,
) -> String {
    json!({
        "type": "thread_render_frame",
        "thread_id": thread_id,
        "events": event_payloads,
        "render_delta": render_delta,
    })
    .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommittedPayloadAction {
    Forward,
    Skip,
    Reconnect,
}

fn should_forward_committed_payload(
    sent_payloads: &mut HashMap<u64, String>,
    last_sent_seq: &mut u64,
    seq: u64,
    raw: &str,
) -> CommittedPayloadAction {
    if seq == 0 {
        return CommittedPayloadAction::Skip;
    }
    if sent_payloads.get(&seq).is_some_and(|sent| sent == raw) {
        return CommittedPayloadAction::Skip;
    }
    if seq > last_sent_seq.saturating_add(1) {
        return CommittedPayloadAction::Reconnect;
    }
    if seq < *last_sent_seq {
        return CommittedPayloadAction::Skip;
    }
    sent_payloads.insert(seq, raw.to_owned());
    *last_sent_seq = (*last_sent_seq).max(seq);
    CommittedPayloadAction::Forward
}

fn thread_stream_reconnect_error(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, reason)
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
    let requested_fork_thread_key = body
        .fork_from_thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested_session_id.is_some() && requested_fork_thread_key.is_some() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "sdkSessionId resume cannot be combined with forkFromThreadId"
            })),
        );
    }
    if requested_session_id.is_some() && body.workspace_mode.is_worktree() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "workspaceMode=worktree cannot be combined with sdkSessionId resume"
            })),
        );
    }
    if requested_fork_thread_key.is_some() && body.workspace_mode.is_worktree() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "workspaceMode=worktree cannot be combined with forkFromThreadId"
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
                    .unwrap_or("Claude or Codex");
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

    let fork_source = match requested_fork_thread_key {
        Some(source_key) => {
            let source_thread_id = match ensure_existing_thread_id(&state, source_key).await {
                Ok(Some(source_thread_id)) => source_thread_id,
                Ok(None) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "fork source thread not found"})),
                    );
                }
                Err(response) => return response,
            };
            // NotFound stays a 400 claim; a backend failure must not
            // masquerade as "fork source thread not found" (#TASK-2130).
            let source_thread_data = match state.threads.thread_store.get(&source_thread_id).await {
                Ok(Some(source_thread_data)) => source_thread_data,
                Ok(None) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "fork source thread not found"})),
                    );
                }
                Err(error) => return thread_store_error_response(&error),
            };
            let Some(provider_type) = provider_type_from_thread_value(&source_thread_data) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "fork source thread has no provider type"})),
                );
            };
            if !is_resume_provider(&provider_type) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "forkFromThreadId is only supported for Claude or Codex provider sessions"
                    })),
                );
            }
            let Some(sdk_session_id) =
                fork_source_sdk_session_id(&source_thread_data, &provider_type)
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "fork source thread has no provider session id yet"
                    })),
                );
            };
            Some((
                source_thread_id,
                source_thread_data,
                provider_type,
                sdk_session_id,
            ))
        }
        None => None,
    };

    let mut metadata = body.metadata.clone();
    garyx_models::strip_server_owned_agent_metadata(&mut metadata);
    // Seed the thread's single runtime cells (metadata.model & co.), the keys
    // the run path and runtime summary read. The legacy dual-track
    // `*_override` keys are read-compat only and are never written anymore.
    for (cell_key, requested) in [
        (MODEL_METADATA_KEY, body.model.as_deref()),
        (
            MODEL_REASONING_EFFORT_METADATA_KEY,
            body.model_reasoning_effort.as_deref(),
        ),
        (
            MODEL_SERVICE_TIER_METADATA_KEY,
            body.model_service_tier.as_deref(),
        ),
    ] {
        if let Some(value) = requested.map(str::trim).filter(|value| !value.is_empty()) {
            metadata.insert(cell_key.to_owned(), Value::String(value.to_owned()));
        }
    }
    if let Some((source_thread_id, _source_thread_data, provider_type, sdk_session_id)) =
        fork_source.as_ref()
    {
        metadata.insert(
            FORK_FROM_THREAD_ID_METADATA_KEY.to_owned(),
            Value::String(source_thread_id.clone()),
        );
        metadata.insert(
            FORK_FROM_SDK_SESSION_ID_METADATA_KEY.to_owned(),
            Value::String(sdk_session_id.clone()),
        );
        metadata.insert(
            FORK_FROM_PROVIDER_TYPE_METADATA_KEY.to_owned(),
            serde_json::to_value(provider_type).unwrap_or(Value::Null),
        );
        metadata.insert(SDK_SESSION_FORK_METADATA_KEY.to_owned(), Value::Bool(true));
    }

    let options = ThreadEnsureOptions {
        label: body.label.clone(),
        workspace_dir: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.workspace_dir.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .and_then(|(_, source_thread_data, _, _)| {
                        workspace_dir_from_value(source_thread_data)
                    })
            })
            .or_else(|| body.workspace_dir.clone()),
        workspace_mode: body.workspace_mode,
        worktree_base_dir: Some(worktree_base_dir_for_config(&state.config_snapshot())),
        agent_id: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.agent_id.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .and_then(|(_, source_thread_data, _, _)| {
                        source_thread_data
                            .get("agent_id")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                    })
            })
            .or_else(|| body.agent_id.clone()),
        metadata,
        provider_type: recovered_session
            .as_ref()
            .map(|recovered| recovered.binding.provider_type.clone())
            .or_else(|| {
                fork_source
                    .as_ref()
                    .map(|(_, _, provider_type, _)| provider_type.clone())
            }),
        sdk_session_id: body.sdk_session_id.clone(),
        thread_kind: None,
        origin_channel: None,
        origin_account_id: None,
        origin_from_id: None,
        is_group: None,
    };

    let binding_intent = if recovered_session.is_some() {
        crate::agent_identity::AgentBindingIntent::RecoverExistingSession
    } else if fork_source.is_some() {
        crate::agent_identity::AgentBindingIntent::Fork
    } else {
        crate::agent_identity::AgentBindingIntent::Fresh
    };
    match create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        options,
        binding_intent,
    )
    .await
    {
        Ok((thread_id, mut data, _resolved)) => {
            if workspace_dir_from_value(&data).is_none() {
                let implicit_update = match ensure_implicit_thread_workspace_for_config(
                    &state.config_snapshot(),
                    &thread_id,
                )
                .await
                {
                    Ok(workspace_dir) => {
                        update_thread_record(
                            &state.threads.thread_store,
                            &thread_id,
                            None,
                            Some(workspace_dir),
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
                match implicit_update {
                    Ok(updated) => {
                        data = updated;
                        state
                            .integration
                            .bridge
                            .set_thread_workspace_binding(
                                &thread_id,
                                workspace_dir_from_value(&data),
                            )
                            .await;
                    }
                    Err(error) => {
                        state.threads.thread_store.delete_logged(&thread_id).await;
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": error })),
                        );
                    }
                }
            }
            if let Some(recovered) = recovered_session.as_ref()
                && let Err(error) =
                    seed_imported_thread_history(&state, &thread_id, &mut data, &recovered.messages)
                        .await
            {
                state.threads.thread_store.delete_logged(&thread_id).await;
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
            // A freshly created thread has no channel-endpoint bindings yet,
            // so it cannot invalidate the router's endpoint/binding indexes;
            // no index maintenance is needed on this path.
            state.invalidate_gateway_sync_caches().await;
            (StatusCode::CREATED, Json(thread_summary(&thread_id, &data)))
        }
        Err(ThreadCreationError::AgentBinding(error)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string() })),
        ),
        Err(ThreadCreationError::Other(error))
            if error.starts_with("unknown agent_id:")
                || error.starts_with("agent_id is not standalone:")
                || error.starts_with("workspace_mode=worktree") =>
        {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": error })))
        }
        Err(ThreadCreationError::Storage(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        ),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        ),
    }
}

/// PATCH /api/threads/:key - update canonical thread metadata
pub async fn update_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<UpdateThreadBody>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            );
        }
        Err(response) => return response,
    };

    match update_thread_record(
        &state.threads.thread_store,
        &thread_id,
        body.label.clone(),
        body.workspace_dir.clone(),
    )
    .await
    {
        Ok(mut data) => {
            let runtime_cells_changed = apply_thread_runtime_cells(&mut data, &body);
            if runtime_cells_changed {
                if let Some(obj) = data.as_object_mut() {
                    obj.insert(
                        "updated_at".to_owned(),
                        Value::String(Utc::now().to_rfc3339()),
                    );
                }
                if let Err(error) = state
                    .threads
                    .thread_store
                    .set(&thread_id, data.clone())
                    .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": error.to_string() })),
                    );
                }
            }
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

fn cron_target_thread_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_thread_key(trimmed) {
        return Some(trimmed.to_owned());
    }
    let stripped = trimmed.strip_prefix("thread:")?;
    is_thread_key(stripped).then(|| stripped.to_owned())
}

fn cron_job_references_thread(job: &crate::cron::CronJob, thread_id: &str) -> bool {
    job.thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| value == thread_id)
        || job
            .target
            .as_deref()
            .and_then(cron_target_thread_id)
            .is_some_and(|target| target == thread_id)
}

async fn automation_job_for_archive_conflict(
    state: &Arc<AppState>,
    thread_id: &str,
) -> Option<String> {
    let service = state.ops.cron_service.as_ref()?;
    service
        .list_all()
        .await
        .into_iter()
        .find(|job| cron_job_references_thread(job, thread_id))
        .map(|job| job.id)
}

#[derive(Clone)]
struct LifecycleRequest {
    kind: LifecycleOperationKind,
    thread_id: String,
    operation_id: String,
    expected_store_incarnation: String,
    fingerprint: String,
    endpoint_keys: Vec<String>,
}

enum LifecycleChildResult {
    Drain(Result<(), CoordinationError>),
    Transaction(GaryxDbResult<LifecycleTransactionResult>),
}

type LifecycleMutationSupervisor = MutationSupervisor<LifecycleChildResult>;

fn lifecycle_operation_name(kind: LifecycleOperationKind) -> &'static str {
    match kind {
        LifecycleOperationKind::Archive => "thread_archive",
        LifecycleOperationKind::Delete => "thread_delete",
    }
}

fn lifecycle_tagged_error(
    status: StatusCode,
    kind: LifecycleOperationKind,
    code: &'static str,
    message: impl Into<String>,
    fields: Value,
) -> axum::response::Response {
    let mut payload = json!({
        "kind": "garyx_api_error",
        "operation": lifecycle_operation_name(kind),
        "code": code,
        "message": message.into(),
    });
    extend_json_object(&mut payload, fields);
    (status, Json(payload)).into_response()
}

fn parse_lifecycle_request(
    kind: LifecycleOperationKind,
    key: &str,
    operation_id: &str,
    expected_store_incarnation: &str,
    endpoint_keys: Vec<String>,
) -> Result<LifecycleRequest, axum::response::Response> {
    let thread_id = key.trim();
    if !is_thread_key(thread_id) {
        return Err(lifecycle_tagged_error(
            StatusCode::BAD_REQUEST,
            kind,
            "invalid_request",
            "thread key must use the thread:: prefix",
            json!({}),
        ));
    }
    let operation_id = uuid::Uuid::parse_str(operation_id.trim())
        .map(|value| value.to_string())
        .map_err(|_| {
            lifecycle_tagged_error(
                StatusCode::BAD_REQUEST,
                kind,
                "invalid_request",
                "operation_id must be a UUID",
                json!({ "thread_id": thread_id }),
            )
        })?;
    let expected_store_incarnation = uuid::Uuid::parse_str(expected_store_incarnation.trim())
        .map(|value| value.to_string())
        .map_err(|_| {
            lifecycle_tagged_error(
                StatusCode::BAD_REQUEST,
                kind,
                "invalid_request",
                "expected_store_incarnation must be a UUID",
                json!({
                    "thread_id": thread_id,
                    "operation_id": operation_id,
                }),
            )
        })?;
    let endpoint_keys = endpoint_keys
        .into_iter()
        .map(|key| normalize_endpoint_lookup_key(&key))
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    let fingerprint =
        canonical_lifecycle_fingerprint(kind, thread_id, endpoint_keys).map_err(|error| {
            lifecycle_tagged_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                kind,
                "unavailable",
                format!("failed to canonicalize lifecycle request: {error}"),
                json!({
                    "thread_id": thread_id,
                    "operation_id": operation_id.clone(),
                }),
            )
        })?;
    Ok(LifecycleRequest {
        kind,
        thread_id: thread_id.to_owned(),
        operation_id,
        expected_store_incarnation,
        fingerprint: fingerprint.canonical,
        endpoint_keys: fingerprint.endpoint_keys,
    })
}

fn operation_matches_request(
    operation: &LifecycleOperationRecord,
    request: &LifecycleRequest,
) -> bool {
    operation.kind == request.kind
        && operation.thread_id == request.thread_id
        && operation.fingerprint == request.fingerprint
}

fn completed_lifecycle_response(operation: &LifecycleOperationRecord) -> axum::response::Response {
    match operation.outcome {
        LifecycleOperationOutcome::AppliedChanged | LifecycleOperationOutcome::AppliedNoop => {
            let detached_endpoint_keys = operation
                .result_payload
                .as_ref()
                .and_then(|payload| payload.get("detached_endpoint_keys"))
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let mut payload = json!({
                "operation_id": operation.operation_id,
                "outcome": operation.outcome,
                "thread_id": operation.thread_id,
                "changed": operation.outcome == LifecycleOperationOutcome::AppliedChanged,
                "detached_endpoint_keys": detached_endpoint_keys,
            });
            match operation.kind {
                LifecycleOperationKind::Archive => {
                    extend_json_object(&mut payload, json!({ "archived": true, "deleted": true }));
                }
                LifecycleOperationKind::Delete => {
                    extend_json_object(&mut payload, json!({ "deleted": true }));
                }
            }
            (StatusCode::OK, Json(payload)).into_response()
        }
        LifecycleOperationOutcome::RejectedConflict
        | LifecycleOperationOutcome::RejectedNotFound => {
            let status = if operation.outcome == LifecycleOperationOutcome::RejectedConflict {
                StatusCode::CONFLICT
            } else {
                StatusCode::NOT_FOUND
            };
            let message = operation
                .detail
                .as_ref()
                .and_then(|detail| detail.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("thread lifecycle request was rejected");
            let reason_code = operation
                .detail
                .as_ref()
                .and_then(|detail| detail.get("code"))
                .cloned();
            let mut fields = json!({
                "operation_id": operation.operation_id,
                "outcome": operation.outcome,
                "thread_id": operation.thread_id,
                "error": message,
                "detail": operation.detail,
                "reason_code": reason_code,
            });
            match operation.kind {
                LifecycleOperationKind::Archive => {
                    extend_json_object(&mut fields, json!({ "archived": false }));
                }
                LifecycleOperationKind::Delete => {
                    extend_json_object(&mut fields, json!({ "deleted": false }));
                }
            }
            if let Some(detail) = operation.detail.as_ref().and_then(Value::as_object) {
                for (key, value) in detail {
                    if key != "code" && key != "message" {
                        fields[key] = value.clone();
                    }
                }
            }
            lifecycle_tagged_error(
                status,
                operation.kind,
                match operation.outcome {
                    LifecycleOperationOutcome::RejectedConflict => "rejected_conflict",
                    LifecycleOperationOutcome::RejectedNotFound => "rejected_not_found",
                    _ => unreachable!(),
                },
                message,
                fields,
            )
        }
    }
}

fn lifecycle_cell_response(
    request: &LifecycleRequest,
    result: &OperationCellResult,
) -> axum::response::Response {
    match result {
        OperationCellResult::Completed(operation) => {
            if operation_matches_request(operation, request) {
                completed_lifecycle_response(operation)
            } else {
                lifecycle_tagged_error(
                    StatusCode::CONFLICT,
                    request.kind,
                    "operation_id_conflict",
                    "operation_id was reused with a different lifecycle request",
                    json!({
                        "thread_id": request.thread_id,
                        "operation_id": request.operation_id,
                    }),
                )
            }
        }
        OperationCellResult::OperationIdConflict => lifecycle_tagged_error(
            StatusCode::CONFLICT,
            request.kind,
            "operation_id_conflict",
            "operation_id was reused with a different lifecycle request",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
            }),
        ),
        OperationCellResult::WrongIncarnation {
            current_store_incarnation,
        } => lifecycle_tagged_error(
            StatusCode::CONFLICT,
            request.kind,
            "wrong_incarnation",
            "store incarnation does not match",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
                "expected_store_incarnation": request.expected_store_incarnation,
                "current_store_incarnation": current_store_incarnation,
            }),
        ),
        OperationCellResult::InProgress => lifecycle_tagged_error(
            StatusCode::CONFLICT,
            request.kind,
            "operation_in_progress",
            "thread lifecycle operation is still in progress",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
            }),
        ),
        OperationCellResult::TransientFailure => lifecycle_tagged_error(
            StatusCode::SERVICE_UNAVAILABLE,
            request.kind,
            "unavailable",
            "thread lifecycle result is temporarily unavailable",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
            }),
        ),
    }
}

async fn wait_for_lifecycle_result(
    request: &LifecycleRequest,
    waiter: OperationJoinHandle,
) -> axum::response::Response {
    match waiter.wait(LIFECYCLE_JOIN_WINDOW).await {
        Ok(result) => lifecycle_cell_response(request, &result),
        Err(OperationWaitError::InProgress) => {
            lifecycle_cell_response(request, &OperationCellResult::InProgress)
        }
        Err(OperationWaitError::TransientFailure) => {
            lifecycle_cell_response(request, &OperationCellResult::TransientFailure)
        }
    }
}

fn coordination_failure(error: CoordinationError) -> OperationCellResult {
    match error {
        CoordinationError::Unavailable => OperationCellResult::InProgress,
        CoordinationError::Store(_) | CoordinationError::Abort(_) => {
            OperationCellResult::TransientFailure
        }
    }
}

enum LifecycleDbCommand {
    Mutation(LifecycleMutationInput),
    Decision(LifecycleDecisionInput),
}

async fn execute_lifecycle_db_command(
    state: &Arc<AppState>,
    request: &LifecycleRequest,
    supervisor: &mut LifecycleMutationSupervisor,
    reservation: garyx_router::LifecycleReservation,
    command: LifecycleDbCommand,
) -> OperationCellResult {
    let witness = reservation.commit_witness();
    supervisor.insert_guard(reservation);
    let db = state.ops.garyx_db.clone();
    supervisor.spawn_blocking_child(move || {
        let result = match command {
            LifecycleDbCommand::Mutation(input) => db.execute_lifecycle_mutation(input),
            LifecycleDbCommand::Decision(input) => db.execute_lifecycle_decision(input),
        };
        if let Ok(
            LifecycleTransactionResult::Completed {
                durable_terminal, ..
            }
            | LifecycleTransactionResult::Existing {
                durable_terminal, ..
            },
        ) = &result
        {
            witness.mark_committed(*durable_terminal);
        }
        LifecycleChildResult::Transaction(result)
    });
    let joined = supervisor.join_child().await;
    let mut reservation = supervisor
        .take_guard::<garyx_router::LifecycleReservation>()
        .expect("lifecycle supervisor lost its reservation");
    match joined {
        Ok(LifecycleChildResult::Transaction(Ok(
            LifecycleTransactionResult::Completed {
                operation,
                durable_terminal,
            }
            | LifecycleTransactionResult::Existing {
                operation,
                durable_terminal,
            },
        ))) => {
            if matches!(
                operation.outcome,
                LifecycleOperationOutcome::AppliedChanged | LifecycleOperationOutcome::AppliedNoop
            ) {
                reservation.settle_committed(durable_terminal);
            } else {
                reservation.settle_decision(durable_terminal);
            }
            state.ops.lifecycle.wake_outbox();
            if operation_matches_request(&operation, request) {
                OperationCellResult::Completed(operation)
            } else {
                OperationCellResult::OperationIdConflict
            }
        }
        Ok(LifecycleChildResult::Transaction(Ok(
            LifecycleTransactionResult::WrongIncarnation {
                current_store_incarnation,
            },
        ))) => {
            let prior = reservation.prior_terminal();
            reservation.settle_transient(prior);
            OperationCellResult::WrongIncarnation {
                current_store_incarnation,
            }
        }
        Ok(LifecycleChildResult::Transaction(Err(_))) | Err(_) => {
            let prior = reservation.prior_terminal();
            reservation.settle_transient(prior);
            OperationCellResult::TransientFailure
        }
        Ok(LifecycleChildResult::Drain(_)) => {
            unreachable!("transaction child returned a drain result")
        }
    }
}

async fn run_lifecycle_owner_inner(
    state: &Arc<AppState>,
    request: &LifecycleRequest,
    supervisor: &mut LifecycleMutationSupervisor,
) -> OperationCellResult {
    // Owner double-check closes completed-lookup → registration races. The
    // DB helper repeats identity first under the same read transaction.
    let db = state.ops.garyx_db.clone();
    let expected = request.expected_store_incarnation.clone();
    let operation_id = request.operation_id.clone();
    let lookup = db
        .run_blocking(move |db| db.lookup_lifecycle_operation(&expected, &operation_id))
        .await;
    match lookup {
        Ok(LifecycleOperationLookup::WrongIncarnation {
            current_store_incarnation,
        }) => {
            return OperationCellResult::WrongIncarnation {
                current_store_incarnation,
            };
        }
        Ok(LifecycleOperationLookup::Current(Some(operation))) => {
            return if operation_matches_request(&operation, request) {
                OperationCellResult::Completed(operation)
            } else {
                OperationCellResult::OperationIdConflict
            };
        }
        Ok(LifecycleOperationLookup::Current(None)) => {}
        Err(_) => return OperationCellResult::TransientFailure,
    }

    let coordinator = state.threads.thread_store.run_coordinator();
    if request.kind == LifecycleOperationKind::Archive {
        let thread_exists = match state.threads.thread_store.get(&request.thread_id).await {
            Ok(record) => record.is_some(),
            Err(_) => return OperationCellResult::TransientFailure,
        };
        if thread_exists
            && let Some(automation_id) =
                automation_job_for_archive_conflict(state, &request.thread_id).await
        {
            let reservation = match coordinator
                .reserve_decision(state.threads.thread_store.as_ref(), &request.thread_id)
                .await
            {
                Ok(reservation) => reservation,
                Err(error) => return coordination_failure(error),
            };
            return execute_lifecycle_db_command(
                state,
                request,
                supervisor,
                reservation,
                LifecycleDbCommand::Decision(LifecycleDecisionInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    outcome: LifecycleOperationOutcome::RejectedConflict,
                    detail: json!({
                        "code": "automation_target",
                        "message": "cannot archive thread targeted by automation",
                        "automation_id": automation_id,
                    }),
                }),
            )
            .await;
        }

        let (reservation, command) = match coordinator
            .reserve_archive(state.threads.thread_store.as_ref(), &request.thread_id)
            .await
        {
            Ok(ArchiveBarrier::Ready(reservation)) => (
                reservation,
                LifecycleDbCommand::Mutation(LifecycleMutationInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    endpoint_keys: request.endpoint_keys.clone(),
                    enabled_channel_accounts: BTreeSet::new(),
                }),
            ),
            Ok(ArchiveBarrier::ActiveLease(reservation)) => (
                reservation,
                LifecycleDbCommand::Decision(LifecycleDecisionInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    outcome: LifecycleOperationOutcome::RejectedConflict,
                    detail: json!({
                        "code": "active_run",
                        "message": "cannot archive thread with active run",
                        "active_run_id": Value::Null,
                    }),
                }),
            ),
            Err(error) => return coordination_failure(error),
        };
        return execute_lifecycle_db_command(state, request, supervisor, reservation, command)
            .await;
    }

    let preflight = state
        .ops
        .endpoint_binding_mutator
        .preflight_and_freeze(&request.thread_id, || state.config_snapshot())
        .await;
    match preflight {
        Ok(DeleteBindingPreflight::InProgress) => OperationCellResult::InProgress,
        Ok(DeleteBindingPreflight::RejectedEnabledBinding) => {
            let reservation = match coordinator
                .reserve_decision(state.threads.thread_store.as_ref(), &request.thread_id)
                .await
            {
                Ok(reservation) => reservation,
                Err(error) => return coordination_failure(error),
            };
            execute_lifecycle_db_command(
                state,
                request,
                supervisor,
                reservation,
                LifecycleDbCommand::Decision(LifecycleDecisionInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    outcome: LifecycleOperationOutcome::RejectedConflict,
                    detail: json!({
                        "code": "active_channel_binding",
                        "message": "cannot delete thread with active channel bindings",
                    }),
                }),
            )
            .await
        }
        Ok(DeleteBindingPreflight::Frozen {
            guard,
            enabled_channel_accounts,
        }) => {
            supervisor.insert_guard(guard);
            let reservation = match coordinator
                .reserve_delete(state.threads.thread_store.as_ref(), &request.thread_id)
                .await
            {
                Ok(reservation) => reservation,
                Err(error) => return coordination_failure(error),
            };
            let drain = reservation.abort_and_drain_future();
            supervisor.insert_guard(reservation);
            supervisor.spawn_child(async move { LifecycleChildResult::Drain(drain.await) });
            let drain_result = supervisor.join_child().await;
            match drain_result {
                Ok(LifecycleChildResult::Drain(Ok(()))) => {}
                Ok(LifecycleChildResult::Drain(Err(error))) => {
                    let mut reservation = supervisor
                        .take_guard::<garyx_router::LifecycleReservation>()
                        .expect("lifecycle supervisor lost its reservation");
                    let prior = reservation.prior_terminal();
                    reservation.settle_transient(prior);
                    return coordination_failure(error);
                }
                Err(_) => {
                    let mut reservation = supervisor
                        .take_guard::<garyx_router::LifecycleReservation>()
                        .expect("lifecycle supervisor lost its reservation");
                    let prior = reservation.prior_terminal();
                    reservation.settle_transient(prior);
                    return OperationCellResult::TransientFailure;
                }
                Ok(LifecycleChildResult::Transaction(_)) => {
                    unreachable!("drain child returned a transaction result")
                }
            }
            let reservation = supervisor
                .take_guard::<garyx_router::LifecycleReservation>()
                .expect("lifecycle supervisor lost its reservation after drain");
            execute_lifecycle_db_command(
                state,
                request,
                supervisor,
                reservation,
                LifecycleDbCommand::Mutation(LifecycleMutationInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    endpoint_keys: Vec::new(),
                    enabled_channel_accounts,
                }),
            )
            .await
        }
        Err(_) => OperationCellResult::TransientFailure,
    }
}

async fn run_lifecycle_owner(
    state: Arc<AppState>,
    request: LifecycleRequest,
    owner: OperationOwnerGuard,
) {
    let mut supervisor = LifecycleMutationSupervisor::new();
    supervisor.insert_guard(owner);
    #[cfg(any(test, feature = "test-seams"))]
    if state.ops.lifecycle.take_owner_panic() {
        panic!("injected lifecycle owner panic");
    }
    let result = run_lifecycle_owner_inner(&state, &request, &mut supervisor).await;
    let owner = supervisor
        .take_guard::<OperationOwnerGuard>()
        .expect("lifecycle supervisor lost its operation owner");
    owner.publish(result);
}

async fn handle_lifecycle_request(
    state: Arc<AppState>,
    request: LifecycleRequest,
) -> axum::response::Response {
    let db = state.ops.garyx_db.clone();
    let expected = request.expected_store_incarnation.clone();
    let operation_id = request.operation_id.clone();
    let lookup = db
        .run_blocking(move |db| db.lookup_lifecycle_operation(&expected, &operation_id))
        .await;
    match lookup {
        Ok(LifecycleOperationLookup::WrongIncarnation {
            current_store_incarnation,
        }) => {
            return lifecycle_cell_response(
                &request,
                &OperationCellResult::WrongIncarnation {
                    current_store_incarnation,
                },
            );
        }
        Ok(LifecycleOperationLookup::Current(Some(operation))) => {
            return lifecycle_cell_response(&request, &OperationCellResult::Completed(operation));
        }
        Ok(LifecycleOperationLookup::Current(None)) => {}
        Err(_) => {
            return lifecycle_cell_response(&request, &OperationCellResult::TransientFailure);
        }
    }

    #[cfg(any(test, feature = "test-seams"))]
    state
        .ops
        .lifecycle
        .pause_after_initial_lookup_if_configured()
        .await;

    let key = OperationKey {
        store_incarnation: request.expected_store_incarnation.clone(),
        operation_id: request.operation_id.clone(),
    };
    match state
        .ops
        .lifecycle
        .registry
        .register(key, &request.fingerprint)
    {
        Ok(OperationRegistration::Join(waiter)) => {
            wait_for_lifecycle_result(&request, waiter).await
        }
        Ok(OperationRegistration::Owner(owner)) => {
            let waiter = owner.join_handle();
            tokio::spawn(run_lifecycle_owner(state, request.clone(), owner));
            wait_for_lifecycle_result(&request, waiter).await
        }
        Err(OperationRegistrationError::FingerprintConflict) => {
            lifecycle_cell_response(&request, &OperationCellResult::OperationIdConflict)
        }
    }
}

/// POST /api/threads/:key/archive - idempotent product archive.
pub async fn archive_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<ArchiveThreadBody>,
) -> axum::response::Response {
    let request = match parse_lifecycle_request(
        LifecycleOperationKind::Archive,
        &key,
        &body.operation_id,
        &body.expected_store_incarnation,
        body.endpoint_keys,
    ) {
        Ok(request) => request,
        Err(response) => return response,
    };
    handle_lifecycle_request(state, request).await
}

/// DELETE /api/threads/:key - idempotent destructive delete.
pub async fn delete_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<DeleteThreadBody>,
) -> axum::response::Response {
    let request = match parse_lifecycle_request(
        LifecycleOperationKind::Delete,
        &key,
        &body.operation_id,
        &body.expected_store_incarnation,
        Vec::new(),
    ) {
        Ok(request) => request,
        Err(response) => return response,
    };
    handle_lifecycle_request(state, request).await
}

/// GET /api/channel-endpoints - list known channel endpoints
pub async fn list_channel_endpoints(
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    // A store/projection failure must surface as 500, never as an empty
    // endpoint listing (#TASK-2128) — and never be satisfied from the
    // snapshot cache during a live outage (#TASK-2134).
    match state.channel_endpoints_fresh().await {
        Ok(endpoints) => Json(json!({
            "endpoints": endpoints.iter().map(channel_endpoint_response_value).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => thread_store_error_response(&error).into_response(),
    }
}

/// Uniform 500 body for store/projection failures at request boundaries.
fn thread_store_error_response(
    error: &garyx_router::ThreadStoreError,
) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "ok": false,
            "reason": "thread-store-error",
            "error": error.to_string(),
        })),
    )
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
pub async fn list_configured_bots(
    State(state): State<Arc<AppState>>,
    Query(params): Query<BotListParams>,
) -> axum::response::Response {
    let config = state.config_snapshot();
    let global_effective_agent_id =
        garyx_models::resolve_effective_default(&state.ops.custom_agents.snapshot().await)
            .map(|binding| binding.agent_id);
    let endpoints = if params.include_endpoints {
        match state.channel_endpoints_fresh().await {
            Ok(endpoints) => endpoints,
            Err(error) => return thread_store_error_response(&error).into_response(),
        }
    } else {
        Vec::new()
    };
    let mut bots = Vec::new();

    for account in configured_channel_accounts(&config.channels) {
        if !account.enabled {
            continue;
        }
        let root_behavior = channel_root_behavior(&state, &account.channel).await;
        let account_ui = if params.include_endpoints {
            resolve_account_ui_with_endpoints(
                &state,
                &account.channel,
                &account.account_id,
                &endpoints,
            )
            .await
        } else {
            None
        };
        let main_endpoint = if params.include_endpoints {
            resolve_main_endpoint_with_endpoints(
                &state,
                &account.channel,
                &account.account_id,
                &endpoints,
            )
            .await
        } else {
            None
        };
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
        let effective_agent_id = account
            .agent_id
            .clone()
            .or_else(|| global_effective_agent_id.clone());
        bots.push(json!({
            "channel": account.channel,
            "account_id": account.account_id,
            "display_name": display_name,
            "name": account.name.as_deref(),
            "enabled": account.enabled,
            "agent_id": account.agent_id.as_deref(),
            "effective_agent_id": effective_agent_id,
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

    Json(json!({ "bots": bots })).into_response()
}

/// GET /api/bot-consoles - list aggregated bot console summaries
pub async fn list_bot_consoles(
    State(state): State<Arc<AppState>>,
    Query(_params): Query<BotListParams>,
) -> axum::response::Response {
    let config = state.config_snapshot();
    let global_effective_agent_id =
        garyx_models::resolve_effective_default(&state.ops.custom_agents.snapshot().await)
            .map(|binding| binding.agent_id);
    let endpoints = match state.channel_endpoints_fresh().await {
        Ok(endpoints) => endpoints,
        Err(error) => return thread_store_error_response(&error).into_response(),
    };
    let mut groups = Vec::<Value>::new();
    let mut group_indexes = HashMap::<String, usize>::new();

    for account in configured_channel_accounts(&config.channels) {
        if !account.enabled {
            continue;
        }
        let id = format!("{}::{}", account.channel, account.account_id);
        let root_behavior = channel_root_behavior(&state, &account.channel).await;
        let account_endpoints = endpoints
            .iter()
            .filter(|endpoint| {
                endpoint.channel == account.channel && endpoint.account_id == account.account_id
            })
            .collect::<Vec<_>>();
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
            default_open_endpoint_from_projected_endpoints(&account_endpoints)
        };
        let default_open_thread_id = default_open_endpoint
            .as_ref()
            .and_then(|value| value.get("thread_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let main_endpoint_thread_id = main_endpoint
            .as_ref()
            .and_then(|endpoint| endpoint.thread_id.clone());
        let display_name = bot_display_name(account.name.as_deref(), &account.account_id);
        let effective_agent_id = account
            .agent_id
            .clone()
            .or_else(|| global_effective_agent_id.clone());
        group_indexes.insert(id.clone(), groups.len());
        groups.push(json!({
            "id": id,
            "channel": account.channel,
            "account_id": account.account_id,
            "display_name": display_name,
            "name": account.name.as_deref(),
            "title": bot_title(&account.channel, &account.account_id, account.name.as_deref()),
            "subtitle": bot_subtitle(&account.channel, &account.account_id),
            "agent_id": account.agent_id.as_deref(),
            "effective_agent_id": effective_agent_id,
            "workspace_dir": account.workspace_dir.as_deref(),
            "workspace_mode": public_workspace_mode(account.workspace_mode.as_deref()),
            "root_behavior": root_behavior,
            "endpoint_count": 0,
            "bound_endpoint_count": 0,
            "latest_activity": Value::Null,
            "status": "idle",
            "main_endpoint_status": if main_endpoint.is_some() { "resolved" } else { "unresolved" },
            "main_endpoint": main_endpoint.as_ref().map(ResolvedMainEndpoint::to_value),
            "main_endpoint_thread_id": main_endpoint_thread_id,
            "default_open_endpoint": default_open_endpoint,
            "default_open_thread_id": default_open_thread_id,
            "conversation_nodes": conversation_nodes_from_projected_endpoints(&account_endpoints),
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

    Json(json!({ "bots": groups })).into_response()
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
    let thread_count = match state.threads.thread_store.count_keys(None).await {
        Ok(count) => count,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response();
        }
    };
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
    .into_response()
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
