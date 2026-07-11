use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use garyx_models::config::GaryxConfig;
use garyx_models::local_paths::gary_home_dir;
use garyx_models::provider::ProviderType;
use garyx_models::routing::{
    default_delivery_target_type, infer_delivery_target_id, infer_delivery_target_type,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::store::ThreadStoreError;
use crate::{DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, ThreadStore};
use crate::{WorkspaceMode, prepare_thread_worktree};

pub const THREAD_KEY_PREFIX: &str = "thread::";
pub const KNOWN_CHANNEL_ENDPOINTS_KEY: &str = "meta::known_channel_endpoints";
const EXPLICIT_THREAD_TITLE_SOURCE: &str = "explicit";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChannelBinding {
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default, alias = "peer_id")]
    pub binding_key: String,
    #[serde(default)]
    pub chat_id: String,
    #[serde(default = "default_delivery_target_type")]
    pub delivery_target_type: String,
    #[serde(default)]
    pub delivery_target_id: String,
    #[serde(default)]
    pub display_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_inbound_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivery_at: Option<String>,
}

impl ChannelBinding {
    pub fn endpoint_key(&self) -> String {
        endpoint_key(&self.channel, &self.account_id, &self.binding_key)
    }

    pub fn resolved_delivery_target_type(&self) -> String {
        infer_delivery_target_type(
            &self.channel,
            Some(&self.delivery_target_type),
            Some(&self.delivery_target_id),
            &self.chat_id,
            &self.binding_key,
        )
    }

    pub fn resolved_delivery_target_id(&self) -> String {
        infer_delivery_target_id(
            &self.channel,
            Some(&self.delivery_target_type),
            Some(&self.delivery_target_id),
            &self.chat_id,
            &self.binding_key,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KnownChannelEndpoint {
    #[serde(default)]
    pub endpoint_key: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default, alias = "peer_id")]
    pub binding_key: String,
    #[serde(default)]
    pub chat_id: String,
    #[serde(default = "default_delivery_target_type")]
    pub delivery_target_type: String,
    #[serde(default)]
    pub delivery_target_id: String,
    #[serde(default)]
    pub display_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_inbound_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivery_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ThreadEnsureOptions {
    pub label: Option<String>,
    pub workspace_dir: Option<String>,
    pub workspace_mode: WorkspaceMode,
    pub worktree_base_dir: Option<PathBuf>,
    pub agent_id: Option<String>,
    pub metadata: HashMap<String, Value>,
    pub provider_type: Option<ProviderType>,
    pub sdk_session_id: Option<String>,
    pub thread_kind: Option<String>,
    pub origin_channel: Option<String>,
    pub origin_account_id: Option<String>,
    pub origin_from_id: Option<String>,
    pub is_group: Option<bool>,
}

pub fn new_thread_key() -> String {
    format!("{THREAD_KEY_PREFIX}{}", Uuid::new_v4())
}

pub fn is_thread_key(key: &str) -> bool {
    key.trim().starts_with(THREAD_KEY_PREFIX)
}

/// True when a thread record matches a channel binding, either through
/// `channel_bindings` or the retained legacy top-level fields (binding key
/// from `thread_binding_key`/`from_id`, channel absent-or-equal, account
/// from `account_id`/`origin_account_id`, or the `{account_id}::` thread-id
/// prefix for records without any account field). Shared by the scan
/// endpoint projection; the SQL projection mirrors these semantics over
/// the `thread_meta` legacy binding columns.
pub(crate) fn thread_matches_binding(
    thread_id: &str,
    thread_data: &Value,
    channel: &str,
    account_id: &str,
    thread_binding_key: &str,
) -> bool {
    if thread_id.starts_with("meta::") {
        return false;
    }

    if bindings_from_value(thread_data).into_iter().any(|binding| {
        binding.channel == channel
            && binding.account_id == account_id
            && binding.binding_key == thread_binding_key
    }) {
        return true;
    }

    let Some(obj) = thread_data.as_object() else {
        return false;
    };
    let legacy_binding_key = obj
        .get("thread_binding_key")
        .or_else(|| obj.get("from_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if legacy_binding_key != Some(thread_binding_key) {
        return false;
    }

    let legacy_channel_matches = obj
        .get("channel")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(|value| value == channel);
    let has_legacy_account =
        obj.get("account_id").is_some() || obj.get("origin_account_id").is_some();
    let legacy_account_matches = obj
        .get("account_id")
        .or_else(|| obj.get("origin_account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|value| value == account_id)
        .unwrap_or(false);
    let legacy_key_matches = thread_id.starts_with(&format!("{account_id}::"));

    legacy_channel_matches
        && if has_legacy_account {
            legacy_account_matches
        } else {
            legacy_key_matches
        }
}

/// The legacy top-level binding fields a record carries, for the
/// `thread_meta` projection columns: (binding_key, channel, account_id,
/// has_account_field).
pub fn legacy_binding_fields_from_value(
    value: &Value,
) -> (Option<String>, Option<String>, Option<String>, bool) {
    let Some(obj) = value.as_object() else {
        return (None, None, None, false);
    };
    let binding_key = obj
        .get("thread_binding_key")
        .or_else(|| obj.get("from_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let channel = obj
        .get("channel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let has_account = obj.get("account_id").is_some() || obj.get("origin_account_id").is_some();
    let account_id = obj
        .get("account_id")
        .or_else(|| obj.get("origin_account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(ToOwned::to_owned);
    (binding_key, channel, account_id, has_account)
}

pub fn endpoint_key(channel: &str, account_id: &str, binding_key: &str) -> String {
    format!("{channel}::{account_id}::{}", binding_key.trim())
}

pub fn normalize_workspace_dir(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_sdk_session_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn default_thread_history_state_value() -> Value {
    json!({
        "source": "transcript_v1",
        "message_count": 0,
        "snapshot_limit": DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT,
        "snapshot_truncated": false,
        "recent_committed_run_ids": [],
    })
}

pub fn bindings_from_value(value: &Value) -> Vec<ChannelBinding> {
    value
        .get("channel_bindings")
        .and_then(Value::as_array)
        .map(|bindings| {
            bindings
                .iter()
                .filter_map(|binding| {
                    let mut normalized = binding.clone();
                    if let Some(obj) = normalized.as_object_mut()
                    {
                        if obj.get("binding_key").is_none() {
                            let legacy_binding_key = obj
                                .get("thread_scope")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .or_else(|| {
                                    obj.get("peer_id")
                                        .and_then(Value::as_str)
                                        .map(str::trim)
                                        .filter(|value| !value.is_empty())
                                });
                            if let Some(legacy_binding_key) = legacy_binding_key {
                                obj.insert(
                                    "binding_key".to_owned(),
                                    Value::String(legacy_binding_key.to_owned()),
                                );
                            }
                        }
                        obj.remove("peer_id");
                        obj.remove("thread_scope");
                    }
                    serde_json::from_value::<ChannelBinding>(normalized)
                        .map_err(|e| tracing::warn!(raw = %binding, error = %e, "failed to parse channel binding"))
                        .ok()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn thread_string_field(value: &Value, primary: &str, fallback: &str) -> Option<String> {
    value
        .get(primary)
        .or_else(|| value.get(fallback))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn is_internal_binding_channel(channel: &str) -> bool {
    channel.trim().eq_ignore_ascii_case("api")
}

pub fn validate_thread_accepts_bot_binding(
    thread_id: &str,
    thread_data: &Value,
    channel: &str,
    account_id: &str,
) -> Result<(), String> {
    if is_internal_binding_channel(channel) {
        return Ok(());
    }
    let bot_id = format!("{channel}:{account_id}");
    let thread_channel = thread_string_field(thread_data, "channel", "origin_channel");
    let thread_account_id = thread_string_field(thread_data, "account_id", "origin_account_id");

    if let Some(owner_channel) = thread_channel.as_deref()
        && owner_channel != channel
        && !is_internal_binding_channel(owner_channel)
    {
        return Err(format!(
            "cannot bind bot '{bot_id}' to thread '{thread_id}': thread belongs to channel '{owner_channel}'"
        ));
    }

    if thread_channel.as_deref() == Some(channel)
        && let Some(owner_account_id) = thread_account_id.as_deref()
        && owner_account_id != account_id
    {
        return Err(format!(
            "cannot bind bot '{bot_id}' to thread '{thread_id}': thread belongs to bot '{channel}:{owner_account_id}'"
        ));
    }

    for binding in bindings_from_value(thread_data) {
        if is_internal_binding_channel(&binding.channel) {
            continue;
        }
        if binding.channel != channel {
            return Err(format!(
                "cannot bind bot '{bot_id}' to thread '{thread_id}': thread is already bound to channel '{}'",
                binding.channel
            ));
        }
        if binding.account_id != account_id {
            return Err(format!(
                "cannot bind bot '{bot_id}' to thread '{thread_id}': thread is already bound to bot '{}:{}'",
                binding.channel, binding.account_id
            ));
        }
    }

    Ok(())
}

pub async fn upsert_known_channel_endpoint(
    store: &Arc<dyn ThreadStore>,
    binding: &ChannelBinding,
) -> Result<(), String> {
    let mut value = store
        .get(KNOWN_CHANNEL_ENDPOINTS_KEY)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| Value::Object(Map::new()));
    upsert_binding(&mut value, binding.clone());
    store
        .set(KNOWN_CHANNEL_ENDPOINTS_KEY, value)
        .await
        .map_err(|error| error.to_string())
}

pub fn workspace_dir_from_value(value: &Value) -> Option<String> {
    value
        .get("workspace_dir")
        .and_then(Value::as_str)
        .and_then(|workspace| normalize_workspace_dir(Some(workspace)))
}

pub fn agent_id_from_value(value: &Value) -> Option<String> {
    value
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn thread_metadata_from_value(value: &Value) -> HashMap<String, Value> {
    value
        .get("metadata")
        .and_then(Value::as_object)
        .map(|metadata| {
            metadata
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn thread_kind_from_value(value: &Value) -> Option<String> {
    value
        .get("thread_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn is_hidden_thread_value(value: &Value) -> bool {
    value.get("hidden").and_then(Value::as_bool) == Some(true)
}

pub fn is_default_thread_list_hidden(value: &Value) -> bool {
    is_hidden_thread_value(value)
}

pub fn label_from_value(value: &Value) -> Option<String> {
    value
        .get("label")
        .or_else(|| value.get("display_name"))
        .or_else(|| value.get("subject"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_updated_at(raw: Option<&str>) -> Option<DateTime<Utc>> {
    raw.and_then(|value| {
        DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|timestamp| timestamp.with_timezone(&Utc))
    })
}

pub(crate) fn value_updated_at(value: &Value) -> Option<String> {
    value
        .get("updated_at")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(crate) fn is_preferred_thread_binding(
    candidate_thread_id: &str,
    candidate_updated_at: Option<&str>,
    current_thread_id: &str,
    current_updated_at: Option<&str>,
) -> bool {
    let candidate_ts = parse_updated_at(candidate_updated_at);
    let current_ts = parse_updated_at(current_updated_at);

    match (candidate_ts, current_ts) {
        (Some(candidate), Some(current)) if candidate != current => return candidate > current,
        (Some(_), None) => return true,
        (None, Some(_)) => return false,
        _ => {}
    }

    let candidate_raw = candidate_updated_at.unwrap_or_default();
    let current_raw = current_updated_at.unwrap_or_default();
    if candidate_raw != current_raw {
        return candidate_raw > current_raw;
    }

    candidate_thread_id > current_thread_id
}

pub fn upsert_thread_fields(value: &mut Value, thread_id: &str, options: &ThreadEnsureOptions) {
    let now = Utc::now().to_rfc3339();
    let Some(obj) = ensure_object(value) else {
        return;
    };

    obj.insert("thread_id".to_owned(), Value::String(thread_id.to_owned()));
    if obj.get("created_at").and_then(Value::as_str).is_none() {
        obj.insert("created_at".to_owned(), Value::String(now.clone()));
    }
    obj.insert("updated_at".to_owned(), Value::String(now));

    if let Some(label) = options
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        obj.insert("label".to_owned(), Value::String(label.to_owned()));
    }

    if let Some(workspace_dir) = normalize_workspace_dir(options.workspace_dir.as_deref()) {
        obj.insert("workspace_dir".to_owned(), Value::String(workspace_dir));
    } else if obj.get("workspace_dir").is_none() {
        obj.insert("workspace_dir".to_owned(), Value::Null);
    }

    if let Some(agent_id) = options
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        obj.insert("agent_id".to_owned(), Value::String(agent_id.to_owned()));
    }

    if let Some(provider_type) = options.provider_type.as_ref() {
        obj.insert(
            "provider_type".to_owned(),
            serde_json::to_value(provider_type).unwrap_or(Value::Null),
        );
    }

    if let Some(sdk_session_id) = normalize_sdk_session_id(options.sdk_session_id.as_deref()) {
        obj.insert("sdk_session_id".to_owned(), Value::String(sdk_session_id));
    }

    if !options.metadata.is_empty() {
        if !obj.get("metadata").is_some_and(Value::is_object) {
            obj.insert("metadata".to_owned(), Value::Object(Map::new()));
        }
        let mut mirrored_top_level_fields = Vec::new();
        if let Some(metadata_obj) = obj.get_mut("metadata").and_then(Value::as_object_mut) {
            for (key, entry_value) in &options.metadata {
                let trimmed_key = key.trim();
                if trimmed_key.is_empty() {
                    continue;
                }
                metadata_obj.insert(trimmed_key.to_owned(), entry_value.clone());
                if matches!(
                    trimmed_key,
                    "source"
                        | "hidden"
                        | "automation_id"
                        | "automation_thread_mode"
                        | "exclude_from_recent"
                ) {
                    mirrored_top_level_fields.push((trimmed_key.to_owned(), entry_value.clone()));
                }
            }
        }
        for (key, value) in mirrored_top_level_fields {
            obj.insert(key, value);
        }
    }

    if let Some(thread_kind) = options
        .thread_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        obj.insert(
            "thread_kind".to_owned(),
            Value::String(thread_kind.to_owned()),
        );
    }

    if let Some(channel) = options.origin_channel.as_deref()
        && obj.get("channel").and_then(Value::as_str).is_none()
    {
        obj.insert("channel".to_owned(), Value::String(channel.to_owned()));
    }
    if let Some(account_id) = options.origin_account_id.as_deref()
        && obj.get("account_id").and_then(Value::as_str).is_none()
    {
        obj.insert(
            "account_id".to_owned(),
            Value::String(account_id.to_owned()),
        );
    }
    if let Some(from_id) = options.origin_from_id.as_deref()
        && obj.get("from_id").and_then(Value::as_str).is_none()
    {
        obj.insert("from_id".to_owned(), Value::String(from_id.to_owned()));
    }
    if let Some(is_group) = options.is_group
        && obj.get("is_group").is_none()
    {
        obj.insert("is_group".to_owned(), Value::Bool(is_group));
    }

    if obj.get("message_count").and_then(Value::as_i64).is_none() {
        obj.insert(
            "message_count".to_owned(),
            Value::Number(serde_json::Number::from(0)),
        );
    }
    if !obj.contains_key("history") {
        obj.insert("history".to_owned(), default_thread_history_state_value());
    }
    if !obj.contains_key("channel_bindings") {
        obj.insert("channel_bindings".to_owned(), Value::Array(Vec::new()));
    }
}

pub fn upsert_binding(value: &mut Value, binding: ChannelBinding) -> bool {
    let Some(obj) = ensure_object(value) else {
        return false;
    };
    let now = Utc::now().to_rfc3339();

    let bindings = obj
        .entry("channel_bindings".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));

    let Some(items) = bindings.as_array_mut() else {
        *bindings = Value::Array(Vec::new());
        return upsert_binding(value, binding);
    };

    let endpoint_key = binding.endpoint_key();
    let binding_value = serde_json::to_value(binding).unwrap_or(Value::Null);
    if let Some(slot) = items.iter_mut().find(|item| {
        serde_json::from_value::<ChannelBinding>((*item).clone())
            .ok()
            .map(|existing| existing.endpoint_key() == endpoint_key)
            .unwrap_or(false)
    }) {
        *slot = binding_value;
    } else {
        items.push(binding_value);
    }

    obj.insert("updated_at".to_owned(), Value::String(now));
    true
}

pub fn remove_binding(value: &mut Value, endpoint_key_to_remove: &str) -> bool {
    let Some(obj) = ensure_object(value) else {
        return false;
    };
    let Some(items) = obj
        .get_mut("channel_bindings")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };

    let original_len = items.len();
    items.retain(|item| {
        serde_json::from_value::<ChannelBinding>(item.clone())
            .ok()
            .map(|binding| binding.endpoint_key() != endpoint_key_to_remove)
            .unwrap_or(true)
    });
    if items.len() != original_len {
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
        return true;
    }
    false
}

pub async fn create_thread_record(
    store: &Arc<dyn ThreadStore>,
    mut options: ThreadEnsureOptions,
) -> Result<(String, Value), String> {
    let thread_id = new_thread_key();
    let worktree = if options.workspace_mode.is_worktree() {
        let workspace_dir = options.workspace_dir.clone().ok_or_else(|| {
            "workspace_mode=worktree requires workspace_dir to be a git repository root".to_owned()
        })?;
        let prepared = prepare_thread_worktree(
            &thread_id,
            &workspace_dir,
            options.worktree_base_dir.as_deref(),
        )
        .await
        .map_err(|error| format!("workspace_mode=worktree failed: {error}"))?;
        options.workspace_dir = Some(prepared.worktree_dir.clone());
        Some(prepared.metadata)
    } else {
        None
    };
    let mut value = Value::Object(Map::new());
    upsert_thread_fields(&mut value, &thread_id, &options);
    if let Some(worktree) = worktree
        && let Some(obj) = value.as_object_mut()
    {
        obj.insert("worktree".to_owned(), worktree);
    }
    store
        .set(&thread_id, value.clone())
        .await
        .map_err(|error| error.to_string())?;
    Ok((thread_id, value))
}

pub async fn update_thread_record(
    store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    label: Option<String>,
    workspace_dir: Option<String>,
) -> Result<Value, String> {
    let Some(mut value) = store
        .get(thread_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err(format!("thread not found: {thread_id}"));
    };
    let existing_workspace_dir = workspace_dir_from_value(&value);
    let Some(obj) = ensure_object(&mut value) else {
        return Err(format!("thread payload is not an object: {thread_id}"));
    };

    if let Some(label) = label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        obj.insert("label".to_owned(), Value::String(label.to_owned()));
        obj.insert(
            "thread_title_source".to_owned(),
            Value::String(EXPLICIT_THREAD_TITLE_SOURCE.to_owned()),
        );
        obj.remove("provider_thread_title");
    }
    if let Some(workspace_dir_input) = workspace_dir {
        let requested_workspace_dir = normalize_workspace_dir(Some(workspace_dir_input.as_str()));
        match (existing_workspace_dir.as_deref(), requested_workspace_dir) {
            (Some(existing), Some(requested)) if existing != requested => {
                return Err(format!(
                    "thread workspace_dir is immutable; create a new thread to use {requested}"
                ));
            }
            (Some(_), None) => {
                return Err(
                    "thread workspace_dir is immutable; cannot clear workspace_dir".to_owned(),
                );
            }
            (Some(_), Some(_)) => {}
            (None, Some(requested)) => {
                obj.insert("workspace_dir".to_owned(), Value::String(requested));
            }
            (None, None) => {
                obj.insert("workspace_dir".to_owned(), Value::Null);
            }
        }
    }
    obj.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
    );
    store
        .set(thread_id, value.clone())
        .await
        .map_err(|error| error.to_string())?;
    Ok(value)
}

pub async fn delete_thread_record(
    store: &Arc<dyn ThreadStore>,
    thread_id: &str,
) -> Result<(), String> {
    let Some(value) = store
        .get(thread_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err(format!("thread not found: {thread_id}"));
    };
    if !bindings_from_value(&value).is_empty() {
        return Err("cannot delete thread with active channel bindings".to_owned());
    }
    if !store
        .delete(thread_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!("thread not found: {thread_id}"));
    }
    Ok(())
}

/// Update `last_delivery_at` on one endpoint's binding with point reads: the
/// known-endpoints registry plus the binding's holder thread. Steady state,
/// an endpoint binding lives on exactly one thread and every caller operates
/// on it, so there is nothing for a store scan to find — the previous
/// `list_keys` walk ran on every run delivery.
pub async fn sync_endpoint_delivery_timestamp(
    store: &Arc<dyn ThreadStore>,
    channel: &str,
    account_id: &str,
    binding_key: &str,
    last_delivery_at: Option<&str>,
    holder_thread_id: &str,
) -> Result<(), String> {
    let target_key = endpoint_key(channel, account_id, binding_key);

    for key in [KNOWN_CHANNEL_ENDPOINTS_KEY, holder_thread_id] {
        if key != KNOWN_CHANNEL_ENDPOINTS_KEY && !is_thread_key(key) {
            continue;
        }
        let Some(mut value) = store.get(key).await.map_err(|error| error.to_string())? else {
            continue;
        };
        let Some(obj) = ensure_object(&mut value) else {
            continue;
        };
        let Some(items) = obj
            .get_mut("channel_bindings")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };

        let mut changed = false;
        for item in items.iter_mut() {
            let Ok(mut binding) = serde_json::from_value::<ChannelBinding>(item.clone()) else {
                continue;
            };
            if binding.endpoint_key() != target_key {
                continue;
            }
            binding.last_delivery_at = last_delivery_at.map(ToOwned::to_owned);
            *item = serde_json::to_value(binding).unwrap_or(Value::Null);
            changed = true;
        }

        if changed {
            obj.insert(
                "updated_at".to_owned(),
                Value::String(Utc::now().to_rfc3339()),
            );
            store
                .set(key, value)
                .await
                .map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

/// Endpoints recorded in the channel binding registry, without touching any
/// thread records. Cheap: reads a single store key.
pub async fn list_registry_channel_endpoints(
    store: &Arc<dyn ThreadStore>,
) -> Result<Vec<KnownChannelEndpoint>, ThreadStoreError> {
    let registry = match store.get(KNOWN_CHANNEL_ENDPOINTS_KEY).await {
        Ok(Some(registry)) => registry,
        Ok(None) | Err(ThreadStoreError::NotFound(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(bindings_from_value(&registry)
        .into_iter()
        .map(|binding| {
            let endpoint_key = binding.endpoint_key();
            let delivery_target_type = binding.resolved_delivery_target_type();
            let delivery_target_id = binding.resolved_delivery_target_id();
            KnownChannelEndpoint {
                endpoint_key,
                channel: binding.channel,
                account_id: binding.account_id,
                binding_key: binding.binding_key,
                chat_id: binding.chat_id,
                delivery_target_type,
                delivery_target_id,
                display_label: binding.display_label,
                thread_id: None,
                thread_label: None,
                workspace_dir: None,
                thread_updated_at: None,
                last_inbound_at: binding.last_inbound_at,
                last_delivery_at: binding.last_delivery_at,
            }
        })
        .collect())
}

/// Registry + projection endpoint listing. Store or projection failures
/// propagate as `Err` so request boundaries surface them as errors
/// instead of an empty listing (#TASK-2128); fire-and-forget callers
/// must opt into degradation explicitly.
pub async fn list_known_channel_endpoints(
    store: &Arc<dyn ThreadStore>,
) -> Result<Vec<KnownChannelEndpoint>, ThreadStoreError> {
    let mut endpoints = HashMap::new();

    for endpoint in list_registry_channel_endpoints(store).await? {
        endpoints.insert(endpoint.endpoint_key.clone(), endpoint);
    }

    let projected = crate::endpoint_projection::channel_endpoint_projection_for(store)
        .endpoints()
        .await
        .map_err(ThreadStoreError::Backend)?;
    for candidate in projected {
        let should_replace = endpoints
            .get(&candidate.endpoint_key)
            .is_none_or(|current| {
                current.thread_id.is_none()
                    || is_preferred_thread_binding(
                        candidate.thread_id.as_deref().unwrap_or_default(),
                        candidate.thread_updated_at.as_deref(),
                        current.thread_id.as_deref().unwrap_or_default(),
                        current.thread_updated_at.as_deref(),
                    )
            });
        if should_replace {
            endpoints.insert(candidate.endpoint_key.clone(), candidate);
        }
    }
    let mut endpoints: Vec<_> = endpoints.into_values().collect();
    endpoints.sort_by(|left, right| left.endpoint_key.cmp(&right.endpoint_key));
    Ok(endpoints)
}

pub fn default_workspace_for_channel_account(
    config: &GaryxConfig,
    channel: &str,
    account_id: &str,
) -> Option<String> {
    match channel {
        "api" => config
            .channels
            .api
            .accounts
            .get(account_id)
            .and_then(|account| normalize_workspace_dir(account.workspace_dir.as_deref())),
        _ => config
            .channels
            .plugins
            .get(channel)
            .and_then(|plugin| plugin.accounts.get(account_id))
            .and_then(|account| normalize_workspace_dir(account.workspace_dir.as_deref())),
    }
}

pub fn default_agent_for_channel_account(
    config: &GaryxConfig,
    channel: &str,
    account_id: &str,
) -> Option<String> {
    match channel {
        "api" => config
            .channels
            .api
            .accounts
            .get(account_id)
            .map(|account| account.agent_id.trim().to_owned()),
        _ => config
            .channels
            .plugins
            .get(channel)
            .and_then(|plugin| plugin.accounts.get(account_id))
            .and_then(|account| account.agent_id.as_deref())
            .map(str::trim)
            .map(str::to_owned),
    }
    .filter(|value| !value.is_empty())
}

pub fn default_workspace_mode_for_channel_account(
    config: &GaryxConfig,
    channel: &str,
    account_id: &str,
) -> WorkspaceMode {
    let configured = match channel {
        "api" => config
            .channels
            .api
            .accounts
            .get(account_id)
            .and_then(|account| account.workspace_mode.as_deref()),
        _ => config
            .channels
            .plugins
            .get(channel)
            .and_then(|plugin| plugin.accounts.get(account_id))
            .and_then(|account| account.workspace_mode.as_deref()),
    };

    match configured
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("worktree") => WorkspaceMode::Worktree,
        _ => WorkspaceMode::Local,
    }
}

pub fn worktree_base_dir_for_config(config: &GaryxConfig) -> PathBuf {
    config
        .sessions
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(gary_home_dir)
        .join("worktrees")
}

fn ensure_object(value: &mut Value) -> Option<&mut Map<String, Value>> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut()
}

#[cfg(test)]
mod tests;
