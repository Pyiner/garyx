use std::sync::Arc;

use garyx_models::routing::{
    DeliveryContext, infer_delivery_target_id, infer_delivery_target_type,
};
use garyx_router::{
    KnownChannelEndpoint, ThreadStore, agent_id_from_value, bindings_from_value,
    is_default_thread_list_hidden, is_thread_key, label_from_value, thread_kind_from_value,
    workspace_dir_from_value,
};
use serde_json::Value;
use tracing::warn;

use crate::garyx_db::{
    GaryxDbService, ThreadMessageRouteDraft, ThreadMetaDraft, ThreadMetaProjectionDraft,
    ThreadMetaProjectionSnapshot,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ThreadMetaProjectionBackfillStats {
    pub threads_scanned: usize,
    pub channel_endpoints: usize,
    pub message_routes: usize,
    pub last_delivery_contexts: usize,
}

pub(crate) async fn backfill_thread_meta_projection_if_incomplete(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> ThreadMetaProjectionBackfillStats {
    let thread_ids = thread_store.list_keys(Some("thread::")).await;
    match garyx_db.thread_meta_projection_is_current(thread_ids.len()) {
        Ok(true) => {
            return ThreadMetaProjectionBackfillStats::default();
        }
        Ok(false) => {}
        Err(error) => {
            warn!(error = %error, "failed to check thread meta projection before backfill");
            return ThreadMetaProjectionBackfillStats::default();
        }
    }

    backfill_thread_meta_projection(thread_ids, thread_store, garyx_db).await
}

pub(crate) async fn list_channel_endpoints_with_projection_backfill(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> Vec<KnownChannelEndpoint> {
    let should_backfill = match garyx_db.count_thread_channel_endpoints() {
        Ok(count) => count == 0,
        Err(error) => {
            warn!(error = %error, "failed to count channel endpoint projection before list");
            false
        }
    };
    if should_backfill {
        let thread_ids = thread_store.list_keys(Some("thread::")).await;
        let _ = backfill_thread_meta_projection(thread_ids, thread_store, garyx_db).await;
    }
    match garyx_db.list_thread_channel_endpoints() {
        Ok(endpoints) => endpoints,
        Err(error) => {
            warn!(error = %error, "failed to list channel endpoint projection");
            Vec::new()
        }
    }
}

async fn backfill_thread_meta_projection(
    thread_ids: Vec<String>,
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> ThreadMetaProjectionBackfillStats {
    let mut snapshot = ThreadMetaProjectionSnapshot::default();
    let mut stats = ThreadMetaProjectionBackfillStats::default();

    for thread_id in thread_ids {
        let Some(data) = thread_store.get(&thread_id).await else {
            continue;
        };
        let Some(draft) = thread_meta_projection_from_thread_data(&thread_id, &data) else {
            continue;
        };
        stats.threads_scanned += 1;
        stats.channel_endpoints += draft.channel_endpoints.len();
        stats.message_routes += draft.message_routes.len();
        if draft.thread_meta.last_delivery_context_json.is_some() {
            stats.last_delivery_contexts += 1;
        }
        snapshot.thread_meta.push(draft.thread_meta);
        snapshot.channel_endpoints.extend(draft.channel_endpoints);
        snapshot.message_routes.extend(draft.message_routes);
    }

    if let Err(error) = garyx_db.sync_thread_meta_projection_snapshot(snapshot) {
        warn!(error = %error, "failed to backfill thread meta projection");
        return ThreadMetaProjectionBackfillStats::default();
    }
    stats
}

pub(crate) fn thread_meta_projection_from_thread_data(
    thread_id: &str,
    data: &Value,
) -> Option<ThreadMetaProjectionDraft> {
    let thread_id = thread_id.trim();
    if !is_thread_key(thread_id) {
        return None;
    }

    let workspace_dir = workspace_dir_from_value(data);
    let thread_label = label_from_value(data);
    let thread_updated_at = string_field(data, "updated_at");
    let last_delivery = delivery_context_from_thread_data(data);
    let thread_meta = ThreadMetaDraft {
        thread_id: thread_id.to_owned(),
        workspace_dir: workspace_dir.clone(),
        thread_type: thread_kind_from_value(data).unwrap_or_else(|| "chat".to_owned()),
        thread_label: thread_label.clone(),
        agent_id: agent_id_from_value(data),
        provider_type: string_field(data, "provider_type"),
        updated_at: thread_updated_at.clone(),
        last_delivery_context_json: last_delivery
            .as_ref()
            .map(|(context_json, _)| context_json.clone()),
        last_delivery_updated_at: last_delivery.and_then(|(_, updated_at)| updated_at),
        default_list_hidden: is_default_thread_list_hidden(data),
    };
    let channel_endpoints = bindings_from_value(data)
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
                thread_id: Some(thread_id.to_owned()),
                thread_label: thread_label.clone(),
                workspace_dir: workspace_dir.clone(),
                thread_updated_at: thread_updated_at.clone(),
                last_inbound_at: binding.last_inbound_at,
                last_delivery_at: binding.last_delivery_at,
            }
        })
        .collect::<Vec<_>>();
    let message_routes = message_routes_from_thread_data(thread_id, data);

    Some(ThreadMetaProjectionDraft {
        thread_id: thread_id.to_owned(),
        thread_meta,
        channel_endpoints,
        message_routes,
    })
}

fn message_routes_from_thread_data(thread_id: &str, data: &Value) -> Vec<ThreadMessageRouteDraft> {
    let Some(records) = data.get("outbound_message_ids").and_then(Value::as_array) else {
        return Vec::new();
    };
    records
        .iter()
        .filter_map(|record| {
            let obj = record.as_object()?;
            let message_id = obj
                .get("message_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let channel = obj
                .get("channel")
                .and_then(Value::as_str)
                .or_else(|| data.get("channel").and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let account_id = obj
                .get("account_id")
                .and_then(Value::as_str)
                .or_else(|| data.get("account_id").and_then(Value::as_str))
                .map(str::trim)
                .unwrap_or_default();
            let chat_id = obj
                .get("chat_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let thread_binding_key = obj
                .get("thread_binding_key")
                .or_else(|| obj.get("thread_scope"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            Some(ThreadMessageRouteDraft {
                thread_id: thread_id.to_owned(),
                channel: channel.to_owned(),
                account_id: account_id.to_owned(),
                chat_id: chat_id.to_owned(),
                thread_binding_key,
                message_id: message_id.to_owned(),
            })
        })
        .collect()
}

fn delivery_context_from_thread_data(data: &Value) -> Option<(String, Option<String>)> {
    let updated_at = string_field(data, "lastUpdatedAt")
        .or_else(|| string_field(data, "updated_at"))
        .or_else(|| string_field(data, "last_updated_at"));
    if let Some(value) = data.get("delivery_context")
        && let Ok(delivery_context) = serde_json::from_value::<DeliveryContext>(value.clone())
    {
        let context_json = serde_json::to_string(&delivery_context).ok()?;
        return Some((context_json, updated_at));
    }

    let channel =
        string_field(data, "last_channel").or_else(|| string_field(data, "lastChannel"))?;
    let account_id =
        string_field(data, "last_account_id").or_else(|| string_field(data, "lastAccountId"))?;
    let chat_id = string_field(data, "last_to").or_else(|| string_field(data, "lastTo"))?;
    let delivery_thread_id =
        string_field(data, "last_thread_id").or_else(|| string_field(data, "lastThreadId"));
    let delivery_target_type = infer_delivery_target_type(&channel, None, None, &chat_id, &chat_id);
    let delivery_target_id = infer_delivery_target_id(&channel, None, None, &chat_id, &chat_id);
    let delivery_context = DeliveryContext {
        channel,
        account_id,
        chat_id: chat_id.clone(),
        user_id: chat_id,
        delivery_target_type,
        delivery_target_id,
        thread_id: delivery_thread_id,
        metadata: Default::default(),
    };
    let context_json = serde_json::to_string(&delivery_context).ok()?;
    Some((context_json, updated_at))
}

fn string_field(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
