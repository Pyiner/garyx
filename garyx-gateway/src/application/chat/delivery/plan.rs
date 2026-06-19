use std::collections::HashSet;
use std::sync::Arc;

use garyx_channels::StreamingDispatchTarget;
use garyx_router::bindings_from_value;
use serde_json::Value;

use crate::server::AppState;

#[derive(Clone)]
pub(super) struct BoundThreadDeliveryTarget {
    pub(super) endpoint_identity: String,
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
            endpoint_identity: endpoint_key,
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
    let target_identity = target.endpoint_identity.trim();
    let streaming_identity = streaming_target.endpoint_identity.trim();
    !target_identity.is_empty() && target_identity == streaming_identity
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
