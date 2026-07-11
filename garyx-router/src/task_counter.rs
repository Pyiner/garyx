use async_trait::async_trait;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum TaskCounterError {
    #[error("counter backend failed: {0}")]
    Backend(String),
}

/// Allocates globally unique, strictly increasing task numbers.
///
/// The production implementation is SQLite-backed (gateway): one
/// transaction bumps a single counter row while flooring it against the
/// task projection's `MAX(number)`, so a number is never handed out
/// twice — even across restarts or after manual database surgery.
#[async_trait]
pub trait TaskCounterStore: Send + Sync {
    async fn allocate(&self) -> Result<u64, TaskCounterError>;
}

#[derive(Default)]
pub struct InMemoryTaskCounterStore {
    counter: Mutex<u64>,
}

impl InMemoryTaskCounterStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TaskCounterStore for InMemoryTaskCounterStore {
    async fn allocate(&self) -> Result<u64, TaskCounterError> {
        let mut counter = self.counter.lock().await;
        *counter = counter
            .checked_add(1)
            .ok_or_else(|| TaskCounterError::Backend("task counter overflow".to_owned()))?;
        Ok(*counter)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn in_memory_counter_allocates_contiguous_numbers() {
        let store = Arc::new(InMemoryTaskCounterStore::new());
        let mut handles = Vec::new();
        for _ in 0..20 {
            let store = store.clone();
            handles.push(tokio::spawn(async move { store.allocate().await.unwrap() }));
        }
        let mut numbers = Vec::new();
        for handle in handles {
            numbers.push(handle.await.unwrap());
        }
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=20).collect::<Vec<_>>());
    }
}
