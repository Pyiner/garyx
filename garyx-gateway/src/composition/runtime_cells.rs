use garyx_channels::ChannelDispatcher;
use garyx_models::config::GaryxConfig;
use std::sync::{Arc, RwLock as StdRwLock};

/// Generic hot-swappable cell for runtime dependencies.
/// Backed by a standard RW lock with cloned Arc snapshots.
pub struct HotSwapCell<T: ?Sized> {
    inner: StdRwLock<Arc<T>>,
}

impl<T> HotSwapCell<T> {
    pub fn from_value(value: T) -> Self {
        Self {
            inner: StdRwLock::new(Arc::new(value)),
        }
    }
}

impl<T: ?Sized> HotSwapCell<T> {
    pub fn from_arc(value: Arc<T>) -> Self {
        Self {
            inner: StdRwLock::new(value),
        }
    }

    pub fn snapshot(&self) -> Arc<T> {
        match self.inner.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn replace_arc(&self, value: Arc<T>) {
        match self.inner.write() {
            Ok(mut guard) => *guard = value,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = value;
            }
        }
    }
}

/// Thread-safe runtime config cell for high-frequency read paths.
pub type LiveConfigCell = HotSwapCell<GaryxConfig>;

impl LiveConfigCell {
    pub fn new(config: GaryxConfig) -> Self {
        HotSwapCell::from_value(config)
    }

    pub fn replace(&self, config: GaryxConfig) {
        self.replace_arc(Arc::new(config));
    }
}

/// Thread-safe hot-swappable dispatcher slot used by runtime config reload.
pub type ChannelDispatcherCell = HotSwapCell<dyn ChannelDispatcher>;

impl ChannelDispatcherCell {
    pub fn new(dispatcher: Arc<dyn ChannelDispatcher>) -> Self {
        HotSwapCell::from_arc(dispatcher)
    }

    pub fn replace(&self, dispatcher: Arc<dyn ChannelDispatcher>) {
        self.replace_arc(dispatcher);
    }
}
