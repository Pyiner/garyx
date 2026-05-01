use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use garyx_models::TaskScope;
use tokio::sync::Mutex;

const LOCK_EX: i32 = 2;
const LOCK_UN: i32 = 8;

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

#[derive(Debug, thiserror::Error)]
pub enum TaskCounterError {
    #[error("invalid task scope: {0}")]
    InvalidScope(String),
    #[error("counter I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("counter worker failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[async_trait]
pub trait TaskCounterStore: Send + Sync {
    async fn allocate(&self, scope: &TaskScope) -> Result<u64, TaskCounterError>;
    async fn peek(&self, scope: &TaskScope) -> Result<u64, TaskCounterError>;
}

pub struct FileTaskCounterStore {
    root: PathBuf,
}

impl FileTaskCounterStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            root: data_dir.as_ref().join("task-counters"),
        }
    }

    fn counter_path(&self, scope: &TaskScope) -> Result<PathBuf, TaskCounterError> {
        validate_scope_part(&scope.channel)?;
        validate_scope_part(&scope.account_id)?;
        Ok(self
            .root
            .join(&scope.channel)
            .join(format!("{}.txt", scope.account_id)))
    }
}

#[async_trait]
impl TaskCounterStore for FileTaskCounterStore {
    async fn allocate(&self, scope: &TaskScope) -> Result<u64, TaskCounterError> {
        let path = self.counter_path(scope)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::task::spawn_blocking(move || allocate_blocking(&path))
            .await?
            .map_err(TaskCounterError::from)
    }

    async fn peek(&self, scope: &TaskScope) -> Result<u64, TaskCounterError> {
        let path = self.counter_path(scope)?;
        tokio::task::spawn_blocking(move || peek_blocking(&path))
            .await?
            .map_err(TaskCounterError::from)
    }
}

fn allocate_blocking(path: &Path) -> std::io::Result<u64> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)?;
    let _guard = FlockGuard::lock(&file)?;
    let current = read_counter(&mut file)?.unwrap_or(1);
    let next = current.saturating_add(1);
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    write!(file, "{next}\n")?;
    file.sync_all()?;
    Ok(current)
}

fn peek_blocking(path: &Path) -> std::io::Result<u64> {
    if !path.exists() {
        return Ok(1);
    }
    let mut file = std::fs::OpenOptions::new().read(true).open(path)?;
    Ok(read_counter(&mut file)?.unwrap_or(1))
}

fn read_counter(file: &mut std::fs::File) -> std::io::Result<Option<u64>> {
    file.seek(SeekFrom::Start(0))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(trimmed.parse::<u64>().ok())
}

struct FlockGuard {
    fd: i32,
}

impl FlockGuard {
    fn lock(file: &std::fs::File) -> std::io::Result<Self> {
        let fd = file.as_raw_fd();
        let rc = unsafe { flock(fd, LOCK_EX) };
        if rc == 0 {
            Ok(Self { fd })
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

impl Drop for FlockGuard {
    fn drop(&mut self) {
        let _ = unsafe { flock(self.fd, LOCK_UN) };
    }
}

#[derive(Default)]
pub struct InMemoryTaskCounterStore {
    counters: Mutex<HashMap<TaskScope, u64>>,
}

impl InMemoryTaskCounterStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TaskCounterStore for InMemoryTaskCounterStore {
    async fn allocate(&self, scope: &TaskScope) -> Result<u64, TaskCounterError> {
        validate_scope_part(&scope.channel)?;
        validate_scope_part(&scope.account_id)?;
        let mut counters = self.counters.lock().await;
        let current = counters.entry(scope.clone()).or_insert(1);
        let allocated = *current;
        *current = current.saturating_add(1);
        Ok(allocated)
    }

    async fn peek(&self, scope: &TaskScope) -> Result<u64, TaskCounterError> {
        validate_scope_part(&scope.channel)?;
        validate_scope_part(&scope.account_id)?;
        Ok(*self.counters.lock().await.get(scope).unwrap_or(&1))
    }
}

fn validate_scope_part(value: &str) -> Result<(), TaskCounterError> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        return Err(TaskCounterError::InvalidScope(value.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn in_memory_counter_allocates_contiguous_numbers() {
        let store = Arc::new(InMemoryTaskCounterStore::new());
        let scope = TaskScope::new("telegram", "main");
        let mut handles = Vec::new();
        for _ in 0..20 {
            let store = store.clone();
            let scope = scope.clone();
            handles.push(tokio::spawn(async move {
                store.allocate(&scope).await.unwrap()
            }));
        }
        let mut numbers = Vec::new();
        for handle in handles {
            numbers.push(handle.await.unwrap());
        }
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=20).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn file_counter_allocates_contiguous_numbers() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(FileTaskCounterStore::new(temp.path()));
        let scope = TaskScope::new("telegram", "main");
        let mut handles = Vec::new();
        for _ in 0..12 {
            let store = store.clone();
            let scope = scope.clone();
            handles.push(tokio::spawn(async move {
                store.allocate(&scope).await.unwrap()
            }));
        }
        let mut numbers = Vec::new();
        for handle in handles {
            numbers.push(handle.await.unwrap());
        }
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=12).collect::<Vec<_>>());
        assert_eq!(store.peek(&scope).await.unwrap(), 13);
    }
}
