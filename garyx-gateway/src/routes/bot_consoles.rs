//! Configured-bot listing and bot console tree assembly.

use super::*;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EndpointConversationDetails {
    pub(super) kind: &'static str,
    pub(super) label: String,
}

#[derive(Clone)]
pub(super) struct ConfiguredChannelAccount {
    channel: String,
    account_id: String,
    enabled: bool,
    name: Option<String>,
    agent_id: Option<String>,
    workspace_dir: Option<String>,
    workspace_mode: Option<String>,
}

struct ResolvedConfiguredBot {
    account: ConfiguredChannelAccount,
    root_behavior: &'static str,
    effective_agent_id: Option<String>,
    main_endpoint: Option<ResolvedMainEndpoint>,
    plugin: Option<Arc<dyn garyx_channels::plugin::ChannelPlugin>>,
}

struct ResolvedBotSnapshot {
    endpoints: Vec<garyx_router::KnownChannelEndpoint>,
    bots: Vec<ResolvedConfiguredBot>,
}

pub(super) fn public_workspace_mode(value: Option<&str>) -> &'static str {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("worktree") => "worktree",
        _ => "local",
    }
}

pub(super) fn configured_channel_accounts(
    channels: &ChannelsConfig,
) -> Vec<ConfiguredChannelAccount> {
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

pub(super) fn default_endpoint_conversation_label(
    endpoint: &garyx_router::KnownChannelEndpoint,
) -> String {
    trimmed_nonempty(Some(&endpoint.display_label))
        .or_else(|| trimmed_nonempty(endpoint.thread_label.as_deref()))
        .or_else(|| trimmed_nonempty(Some(&endpoint.chat_id)))
        .or_else(|| trimmed_nonempty(Some(&endpoint.binding_key)))
        .unwrap_or_else(|| "Conversation".to_owned())
}

pub(super) fn endpoint_scope(endpoint: &garyx_router::KnownChannelEndpoint) -> Option<&str> {
    let binding_key = endpoint.binding_key.trim();
    let chat_id = endpoint.chat_id.trim();
    if binding_key.is_empty() || binding_key == chat_id {
        None
    } else {
        Some(binding_key)
    }
}

pub(super) fn endpoint_conversation_kind(
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

pub(super) fn endpoint_conversation_details(
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

pub(super) fn resolved_main_endpoint_conversation_details(
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

pub(super) fn channel_endpoint_response_value(
    endpoint: &garyx_router::KnownChannelEndpoint,
) -> Value {
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

pub(super) fn sort_channel_endpoint_values_by_identity(items: &mut [Value]) {
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

pub(super) fn plugin_conversation_endpoint_value(
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

pub(super) fn bot_display_name(name: Option<&str>, account_id: &str) -> String {
    name.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
    .unwrap_or_else(|| account_id.to_owned())
}

pub(super) fn bot_title(_channel: &str, account_id: &str, name: Option<&str>) -> String {
    bot_display_name(name, account_id)
}

pub(super) fn bot_subtitle(channel_label: &str, account_id: &str) -> String {
    format!("{channel_label} Bot · {account_id}")
}

pub(super) async fn channel_plugin_for(
    state: &Arc<AppState>,
    channel: &str,
) -> Option<Arc<dyn garyx_channels::plugin::ChannelPlugin>> {
    let manager = state.channel_plugin_manager();

    {
        let guard = manager.lock().await;
        guard.plugin(channel)
    }
}

pub(super) fn account_root_behavior_value(
    behavior: garyx_channels::plugin_host::AccountRootBehavior,
) -> &'static str {
    match behavior {
        garyx_channels::plugin_host::AccountRootBehavior::OpenDefault => "open_default",
        garyx_channels::plugin_host::AccountRootBehavior::ExpandOnly => "expand_only",
    }
}

pub(super) async fn resolve_main_endpoint_with_endpoints(
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

/// Canonical fresh account resolution for both bot listing routes.
///
/// Route assembly may adapt presentation fields, but it must never resolve a
/// main endpoint independently of this snapshot.
async fn resolve_bot_snapshot(
    state: &Arc<AppState>,
) -> Result<ResolvedBotSnapshot, garyx_router::ThreadStoreError> {
    let config = state.config_snapshot();
    let global_effective_agent_id =
        garyx_models::resolve_effective_default(&state.ops.custom_agents.snapshot().await)
            .map(|binding| binding.agent_id);
    let endpoints = state.channel_endpoints_fresh().await?;
    let mut bots = Vec::new();

    for account in configured_channel_accounts(&config.channels) {
        if !account.enabled {
            continue;
        }
        let plugin = channel_plugin_for(state, &account.channel).await;
        let root_behavior = plugin
            .as_ref()
            .map(|plugin| account_root_behavior_value(plugin.account_root_behavior()))
            .unwrap_or("open_default");
        let main_endpoint = match plugin.as_ref() {
            Some(plugin) => plugin
                .resolve_main_endpoint(&account.account_id, &endpoints)
                .await
                .map(Into::into),
            None => None,
        };
        let effective_agent_id = account
            .agent_id
            .clone()
            .or_else(|| global_effective_agent_id.clone());
        bots.push(ResolvedConfiguredBot {
            account,
            root_behavior,
            effective_agent_id,
            main_endpoint,
            plugin,
        });
    }

    Ok(ResolvedBotSnapshot { endpoints, bots })
}

pub(super) fn resolve_default_open_endpoint_from_account_ui(
    account_ui: Option<&PluginAccountUi>,
    endpoints: &[garyx_router::KnownChannelEndpoint],
) -> Option<Value> {
    let endpoint_key = account_ui.and_then(|ui| ui.default_open_endpoint_key.as_deref())?;
    let endpoint = endpoints
        .iter()
        .find(|candidate| candidate.endpoint_key == endpoint_key)?;
    Some(channel_endpoint_response_value(endpoint))
}

pub(super) fn endpoint_activity(endpoint: &garyx_router::KnownChannelEndpoint) -> Option<&str> {
    endpoint
        .last_inbound_at
        .as_deref()
        .or(endpoint.last_delivery_at.as_deref())
        .or(endpoint.thread_updated_at.as_deref())
}

pub(super) fn default_open_endpoint_from_projected_endpoints(
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

pub(super) fn conversation_nodes_from_projected_endpoints(
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

pub(super) async fn resolve_main_endpoint_by_key(
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

/// GET /api/configured-bots - list all configured channel bot accounts from config
pub async fn list_configured_bots(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let ResolvedBotSnapshot { endpoints, bots } = match resolve_bot_snapshot(&state).await {
        Ok(snapshot) => snapshot,
        Err(error) => return thread_store_error_response(&error).into_response(),
    };
    let mut values = Vec::new();

    for bot in bots {
        let ResolvedConfiguredBot {
            account,
            root_behavior,
            effective_agent_id,
            main_endpoint,
            plugin,
        } = bot;
        let plugin_endpoints = endpoints
            .iter()
            .filter(|endpoint| {
                endpoint.channel == account.channel && endpoint.account_id == account.account_id
            })
            .map(plugin_conversation_endpoint_value)
            .collect::<Vec<_>>();
        let account_ui = match plugin {
            Some(plugin) => {
                plugin
                    .resolve_account_ui(&account.account_id, &plugin_endpoints)
                    .await
            }
            None => None,
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
        values.push(json!({
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

    Json(json!({ "bots": values })).into_response()
}

/// GET /api/bot-consoles - list aggregated bot console summaries
pub async fn list_bot_consoles(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let ResolvedBotSnapshot { endpoints, bots } = match resolve_bot_snapshot(&state).await {
        Ok(snapshot) => snapshot,
        Err(error) => return thread_store_error_response(&error).into_response(),
    };
    let mut groups = Vec::<Value>::new();
    let mut group_indexes = HashMap::<String, usize>::new();

    for bot in bots {
        let ResolvedConfiguredBot {
            account,
            root_behavior,
            effective_agent_id,
            main_endpoint,
            plugin: _,
        } = bot;
        let id = format!("{}::{}", account.channel, account.account_id);
        let account_endpoints = endpoints
            .iter()
            .filter(|endpoint| {
                endpoint.channel == account.channel && endpoint.account_id == account.account_id
            })
            .collect::<Vec<_>>();
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
