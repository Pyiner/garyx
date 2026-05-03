use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::sync::Mutex;

const LOCK_EX: i32 = 2;
const LOCK_UN: i32 = 8;

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

#[derive(Debug, thiserror::Error)]
pub enum TaskCounterError {
    #[error("counter I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("counter worker failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[async_trait]
pub trait TaskCounterStore: Send + Sync {
    async fn allocate(&self) -> Result<u64, TaskCounterError>;
    async fn peek(&self) -> Result<u64, TaskCounterError>;
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

    fn counter_path(&self) -> PathBuf {
        self.root.join("global.txt")
    }
}

#[async_trait]
impl TaskCounterStore for FileTaskCounterStore {
    async fn allocate(&self) -> Result<u64, TaskCounterError> {
        let path = self.counter_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::task::spawn_blocking(move || allocate_blocking(&path))
            .await?
            .map_err(TaskCounterError::from)
    }

    async fn peek(&self) -> Result<u64, TaskCounterError> {
        let path = self.counter_path();
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
    let next = current.checked_add(1).ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidData,
            format!("task counter overflow at {}", path.display()),
        )
    })?;
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
    trimmed.parse::<u64>().map(Some).map_err(|error| {
        Error::new(
            ErrorKind::InvalidData,
            format!("invalid task counter value '{trimmed}': {error}"),
        )
    })
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
        if *counter == 0 {
            *counter = 1;
        }
        let allocated = *counter;
        *counter = counter
            .checked_add(1)
            .ok_or_else(|| TaskCounterError::Io(std::io::ErrorKind::InvalidData.into()))?;
        Ok(allocated)
    }

    async fn peek(&self) -> Result<u64, TaskCounterError> {
        Ok((*self.counter.lock().await).max(1))
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

    #[tokio::test]
    async fn file_counter_allocates_contiguous_numbers() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(FileTaskCounterStore::new(temp.path()));
        let mut handles = Vec::new();
        for _ in 0..12 {
            let store = store.clone();
            handles.push(tokio::spawn(async move { store.allocate().await.unwrap() }));
        }
        let mut numbers = Vec::new();
        for handle in handles {
            numbers.push(handle.await.unwrap());
        }
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=12).collect::<Vec<_>>());
        assert_eq!(store.peek().await.unwrap(), 13);
    }

    #[tokio::test]
    async fn file_counter_rejects_corrupt_counter_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("task-counters/global.txt");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "not-a-number\n").await.unwrap();

        let store = FileTaskCounterStore::new(temp.path());
        let error = store.allocate().await.unwrap_err();
        assert!(matches!(error, TaskCounterError::Io(_)));
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "not-a-number\n"
        );
    }
}
