use caseless::Caseless;
use chrono::DateTime;
use garyx_models::routing::{
    DeliveryContext, infer_delivery_target_id, infer_delivery_target_type,
};

use garyx_router::{
    KnownChannelEndpoint, agent_id_from_value, bindings_from_value, history_message_count,
    is_default_thread_list_hidden, is_thread_key, label_from_value, workspace_dir_from_value,
};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use crate::garyx_db::{ThreadMetaDraft, ThreadMetaProjectionDraft};
use crate::thread_runtime::selected_model_cells_from_thread_value;
use crate::thread_type::thread_summary_type_from_record;

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
    let agent_id = agent_id_from_value(data);
    let sort_updated_at_us =
        summary_sort_updated_at_us(thread_updated_at.as_deref(), created_at.as_deref());
    let search_text = summary_search_text(
        thread_label.as_deref(),
        workspace_dir.as_deref(),
        agent_id.as_deref(),
        last_message_preview.as_deref(),
    );
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
        agent_id,
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
        sort_updated_at_us,
        search_text,
    };
    let channel_endpoints = channel_endpoints_from_thread_data(thread_id, data);
    Some(ThreadMetaProjectionDraft {
        thread_id: thread_id.to_owned(),
        thread_meta,
        channel_endpoints,
    })
}

pub(crate) fn normalize_for_search(value: &str) -> String {
    value.nfkc().default_case_fold().collect()
}

fn summary_sort_updated_at_us(updated_at: Option<&str>, created_at: Option<&str>) -> i64 {
    updated_at
        .and_then(parse_rfc3339_micros)
        .or_else(|| created_at.and_then(parse_rfc3339_micros))
        .unwrap_or(0)
}

fn parse_rfc3339_micros(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|timestamp| timestamp.timestamp_micros())
}

fn summary_search_text(
    title: Option<&str>,
    workspace_dir: Option<&str>,
    agent_id: Option<&str>,
    last_message_preview: Option<&str>,
) -> String {
    normalize_for_search(&format!(
        "{}\n{}\n{}\n{}",
        title.unwrap_or_default(),
        workspace_dir.unwrap_or_default(),
        agent_id.unwrap_or_default(),
        last_message_preview.unwrap_or_default(),
    ))
}

/// Channel endpoint rows for one thread record: one row per binding the
/// record currently holds, written by the same-transaction projection
/// derivation on every record write.
pub(crate) fn channel_endpoints_from_thread_data(
    thread_id: &str,
    data: &Value,
) -> Vec<KnownChannelEndpoint> {
    let thread_id = thread_id.trim();
    let thread_label = label_from_value(data);
    let workspace_dir = workspace_dir_from_value(data);
    let thread_updated_at = data
        .get("updated_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    bindings_from_value(data)
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
        .collect::<Vec<_>>()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_for_search_obeys_nfkc_and_default_case_fold_contract() {
        assert_eq!(
            normalize_for_search("Cafe\u{301}"),
            normalize_for_search("CAFÉ")
        );
        assert_eq!(normalize_for_search("Straße ẞ"), "strasse ss");
        assert_eq!(normalize_for_search("Σςσ"), "σσσ");
        assert_eq!(normalize_for_search("％＿＼"), "%_\\");
        assert_eq!(normalize_for_search("left\0RIGHT"), "left\0right");
    }

    #[test]
    fn summary_projection_derives_sort_fallback_and_four_field_search_text() {
        let data = json!({
            "label": "Straße",
            "workspace_dir": "/workspace/Équipe",
            "agent_id": "Σς",
            "updated_at": "not-a-timestamp",
            "created_at": "2026-07-17T01:02:03.456789+00:00",
            "last_assistant_preview": "％＿＼\0Tail"
        });
        let projected = thread_meta_projection_from_thread_data_with_active_run(
            "thread::summary-derived",
            &data,
            None,
        )
        .expect("summary projection")
        .thread_meta;
        assert_eq!(
            projected.sort_updated_at_us,
            DateTime::parse_from_rfc3339("2026-07-17T01:02:03.456789Z")
                .unwrap()
                .timestamp_micros()
        );
        assert_eq!(
            projected.search_text,
            normalize_for_search("Straße\n/workspace/Équipe\nΣς\n％＿＼\0Tail")
        );

        let missing = thread_meta_projection_from_thread_data_with_active_run(
            "thread::summary-missing-time",
            &json!({}),
            None,
        )
        .unwrap();
        assert_eq!(missing.thread_meta.sort_updated_at_us, 0);
    }
}
