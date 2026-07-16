mod delivery;
mod routing;

use std::collections::HashMap;

use garyx_models::routing::DeliveryContext;

#[derive(Default)]
pub(super) struct DeliveryContextState {
    pub(super) last_delivery: HashMap<String, DeliveryContext>,
    pub(super) last_delivery_order: Vec<String>,
}
