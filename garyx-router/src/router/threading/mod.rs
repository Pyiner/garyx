mod navigation;
mod threads;

use std::collections::HashMap;

#[derive(Default)]
pub(super) struct ThreadNavigationState {
    pub(super) binding_thread_map: HashMap<String, String>,
    pub(super) endpoint_thread_map: HashMap<String, String>,
}
