use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};

/// Shared event stream infrastructure (broadcast + replay + drop accounting).
pub struct EventStreamHub {
    tx: broadcast::Sender<String>,
    history: Arc<Mutex<VecDeque<String>>>,
    drops: std::sync::atomic::AtomicU64,
}

impl EventStreamHub {
    pub fn new(tx: broadcast::Sender<String>) -> Arc<Self> {
        let hub = Arc::new(Self {
            tx,
            history: Arc::new(Mutex::new(VecDeque::new())),
            drops: std::sync::atomic::AtomicU64::new(0),
        });
        spawn_event_history_recorder(hub.clone());
        hub
    }

    pub fn sender(&self) -> broadcast::Sender<String> {
        self.tx.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub async fn history_len(&self) -> usize {
        self.history.lock().await.len()
    }

    pub async fn history_snapshot(&self, limit: usize) -> Vec<String> {
        let history = self.history.lock().await;
        let start = history.len().saturating_sub(limit);
        history.iter().skip(start).cloned().collect()
    }

    pub fn dropped_count(&self) -> u64 {
        self.drops.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn record_drop(&self) {
        self.drops
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

const EVENT_HISTORY_MAX: usize = 256;

fn spawn_event_history_recorder(hub: Arc<EventStreamHub>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    let mut rx = hub.tx.subscribe();
    handle.spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let mut history = hub.history.lock().await;
                    history.push_back(event);
                    while history.len() > EVENT_HISTORY_MAX {
                        history.pop_front();
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
