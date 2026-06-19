use std::collections::HashSet;
use std::sync::Arc;

use garyx_channels::StreamingDispatchTarget;
use garyx_router::bindings_from_value;
use serde_json::Value;

use crate::server::AppState;

#[derive(Clone)]
pub(super) struct BoundThreadDeliveryTarget {
    pub(super) endpoint_key: String,
    pub(super) channel: String,
    pub(super) account_id: String,
    pub(super) chat_id: String,
    pub(super) delivery_target_type: String,
    pub(super) delivery_target_id: String,
    pub(super) thread_id: Option<String>,
}

pub(super) fn bound_thread_delivery_targets(value: &Value) -> Vec<BoundThreadDeliveryTarget> {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();

    for binding in bindings_from_value(value) {
        let channel = binding.channel.trim();
        let account_id = binding.account_id.trim();
        if channel.is_empty() || account_id.is_empty() {
            continue;
        }
        if channel.eq_ignore_ascii_case("api") {
            continue;
        }

        let chat_id = binding.chat_id.trim().to_owned();
        let binding_key = binding.binding_key.trim().to_owned();
        let resolved_chat_id = if chat_id.is_empty() {
            binding_key.clone()
        } else {
            chat_id
        };
        if resolved_chat_id.is_empty() {
            continue;
        }

        let endpoint_key = binding.endpoint_key();
        if !seen.insert(endpoint_key.clone()) {
            continue;
        }

        let thread_id =
            crate::routes::binding_delivery_thread_id(&binding.binding_key, &binding.chat_id);

        targets.push(BoundThreadDeliveryTarget {
            endpoint_key,
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            chat_id: resolved_chat_id,
            delivery_target_type: binding.resolved_delivery_target_type(),
            delivery_target_id: binding.resolved_delivery_target_id(),
            thread_id,
        });
    }

    targets
}

pub(super) async fn snapshot_bound_thread_delivery_targets(
    state: &Arc<AppState>,
    thread_id: &str,
) -> Vec<BoundThreadDeliveryTarget> {
    state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .map(|session_data| bound_thread_delivery_targets(&session_data))
        .unwrap_or_default()
}

fn same_delivery_target(
    target: &BoundThreadDeliveryTarget,
    streaming_target: &StreamingDispatchTarget,
) -> bool {
    if !target
        .channel
        .trim()
        .eq_ignore_ascii_case(streaming_target.channel.trim())
        || target.account_id.trim() != streaming_target.account_id.trim()
    {
        return false;
    }

    let target_delivery_id = target.delivery_target_id.trim();
    let streaming_delivery_id = streaming_target.delivery_target_id.trim();
    if !target_delivery_id.is_empty()
        && !streaming_delivery_id.is_empty()
        && target.delivery_target_type.trim() == streaming_target.delivery_target_type.trim()
        && target_delivery_id == streaming_delivery_id
    {
        return true;
    }

    let target_chat_id = target.chat_id.trim();
    let streaming_chat_id = streaming_target.chat_id.trim();
    !target_chat_id.is_empty()
        && !streaming_chat_id.is_empty()
        && target_chat_id == streaming_chat_id
}

pub(super) fn targets_except_streaming_target(
    targets: &[BoundThreadDeliveryTarget],
    streaming_target: &StreamingDispatchTarget,
) -> Vec<BoundThreadDeliveryTarget> {
    targets
        .iter()
        .filter(|target| !same_delivery_target(target, streaming_target))
        .cloned()
        .collect()
}
