
use garyx_models::routing::{
    DeliveryContext, infer_delivery_target_id, infer_delivery_target_type,
};
use std::sync::Arc;

use garyx_router::{
    KnownChannelEndpoint, ThreadStore, agent_id_from_value, bindings_from_value,
    history_message_count, is_default_thread_list_hidden, is_thread_key, label_from_value,
    list_registry_channel_endpoints, workspace_dir_from_value,
};
use serde_json::Value;
use tracing::warn;

use crate::garyx_db::{
    GaryxDbService, ThreadMessageRouteDraft, ThreadMetaDraft, ThreadMetaProjectionDraft,
};
use crate::thread_runtime::selected_model_cells_from_thread_value;
use crate::thread_type::thread_summary_type_from_record;



/// Channel endpoints for gateway sync: the projection rows (bound
/// endpoints, derived in the same transaction as every record write)
/// merged with the known-endpoint registry (channels the gateway has
/// seen, kept even when unbound). The former read-time backfill gate is
/// retired (#TASK-1864 closing batch).
pub(crate) async fn list_channel_endpoints_with_registry(
    thread_store: &Arc<dyn ThreadStore>,
    garyx_db: &GaryxDbService,
) -> Vec<KnownChannelEndpoint> {
    let projected = match garyx_db.list_thread_channel_endpoints() {
        Ok(rows) => rows,
        Err(error) => {
            warn!(error = %error, "failed to list channel endpoint projection");
            Vec::new()
        }
    };
    let known = list_registry_channel_endpoints(thread_store).await;
    merge_projected_and_known_channel_endpoints(projected, known)
}

fn merge_projected_and_known_channel_endpoints(
    mut projected: Vec<KnownChannelEndpoint>,
    known: Vec<KnownChannelEndpoint>,
) -> Vec<KnownChannelEndpoint> {
    for endpoint in known {
        match projected
            .iter_mut()
            .find(|candidate| candidate.endpoint_key == endpoint.endpoint_key)
        {
            Some(existing) if existing.thread_id.is_none() && endpoint.thread_id.is_some() => {
                *existing = endpoint;
            }
            Some(_) => {}
            None => projected.push(endpoint),
        }
    }
    projected.sort_by(|left, right| left.endpoint_key.cmp(&right.endpoint_key));
    projected
}

pub(crate) fn thread_meta_projection_from_thread_data_with_active_run(
    thread_id: &str,
    data: &Value,
    active_run_id: Option<String>,
) -> Option<ThreadMetaProjectionDraft> {
    let thread_id = thread_id.trim();
    if !is_thread_key(thread_id) {
        return None;
    }

    let workspace_dir = workspace_dir_from_value(data);
    let thread_label = label_from_value(data);
    let created_at = string_field(data, "created_at").or_else(|| string_field(data, "_created_at"));
    let thread_updated_at = string_field(data, "updated_at");
    let message_count = history_message_count(data).min(u32::MAX as usize) as u32;
    let last_user_message = last_message_preview_for_role(data, "user");
    let last_assistant_message = last_message_preview_for_role(data, "assistant");
    let last_message_preview = last_assistant_message
        .clone()
        .or_else(|| last_user_message.clone());
    let recent_run_id = data
        .get("history")
        .and_then(|history| history.get("recent_committed_run_ids"))
        .and_then(Value::as_array)
        .and_then(|entries| entries.last())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let worktree_json = data
        .get("worktree")
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::to_string(value).ok());
    let last_delivery = delivery_context_from_thread_data(data);
    // Runtime-summary columns (list fast path): same extraction as the live
    // summary builder so `/api/threads` no longer re-reads the full thread
    // record per row.
    let (selected_model, selected_model_reasoning_effort, selected_model_service_tier) =
        selected_model_cells_from_thread_value(data);
    let thread_meta = ThreadMetaDraft {
        thread_id: thread_id.to_owned(),
        workspace_dir: workspace_dir.clone(),
        thread_type: thread_summary_type_from_record(data),
        thread_label: thread_label.clone(),
        agent_id: agent_id_from_value(data),
        provider_type: string_field(data, "provider_type"),
        provider_key: string_field(data, "provider_key"),
        selected_model,
        selected_model_reasoning_effort,
        selected_model_service_tier,
        sdk_session_id: string_field(data, "sdk_session_id"),
        created_at,
        updated_at: thread_updated_at.clone(),
        message_count,
        last_user_message,
        last_assistant_message,
        last_message_preview,
        recent_run_id,
        active_run_id,
        worktree_json,
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

fn last_message_preview_for_role(data: &Value, role: &str) -> Option<String> {
    // Write-time preview fields are the source (#TASK-1864 batch 1).
    if let Some(preview) = garyx_models::message_preview::preview_field_for_role(role)
        .and_then(|field| data.get(field))
        .and_then(Value::as_str)
    {
        return Some(preview.to_owned());
    }
    None
}
