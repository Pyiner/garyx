mod delivery;
mod routing;

use std::collections::HashMap;

use garyx_models::routing::DeliveryContext;

use crate::message_routing::MessageRoutingIndex;

#[derive(Default)]
pub(super) struct ReplyRoutingState {
    pub(super) message_routing_index: MessageRoutingIndex,
}

#[derive(Default)]
pub(super) struct DeliveryContextState {
    pub(super) last_delivery: HashMap<String, DeliveryContext>,
    pub(super) last_delivery_order: Vec<String>,
}
