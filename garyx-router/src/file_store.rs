use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};
use tracing::debug;

use crate::store::ThreadStoreError;

// Constants matching the Python implementation.
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(45);
const DEFAULT_MAX_CONCURRENT_OPS: usize = 50;
const DEFAULT_CACHE_MAX_SIZE: usize = 1000;

pub fn encode_thread_storage_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len() * 2);
    for b in key.as_bytes() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub fn thread_storage_file_name(key: &str, extension: &str) -> String {
    let encoded = encode_thread_storage_key(key);
    if extension.is_empty() {
        format!("k_{encoded}")
    } else {
        format!("k_{encoded}.{extension}")
    }
}

/// Cached thread entry with mtime tracking for validation.
struct CacheEntry {
    data: Value,
    mtime: SystemTime,
    inserted_at: Instant,
}

/// Read-only access to the legacy file-based thread archive.
///
/// Retains the archive's historical read semantics:
/// - JSON file storage per record
/// - In-memory TTL cache validated against file mtime
/// - Semaphore-limited concurrent file operations
pub struct FileThreadStore {
    canonical_data_dir: PathBuf,
    legacy_data_dir: PathBuf,
    cache: Mutex<HashMap<String, CacheEntry>>,
    cache_ttl: Duration,
    cache_max_size: usize,
    semaphore: Semaphore,
}

impl FileThreadStore {
    /// Create a new file-based thread store.
    ///
    /// `data_dir` is the base directory; new thread records are stored in
    /// `data_dir/threads/` and older `data_dir/sessions/` data remains readable.
    pub async fn new(data_dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let threads_dir = data_dir.as_ref().join("threads");
        let sessions_dir = data_dir.as_ref().join("sessions");
        tokio::fs::create_dir_all(&threads_dir).await?;
        tokio::fs::create_dir_all(&sessions_dir).await?;
        Ok(Self {
            canonical_data_dir: threads_dir,
            legacy_data_dir: sessions_dir,
            cache: Mutex::new(HashMap::new()),
            cache_ttl: DEFAULT_CACHE_TTL,
            cache_max_size: DEFAULT_CACHE_MAX_SIZE,
            semaphore: Semaphore::new(DEFAULT_MAX_CONCURRENT_OPS),
        })
    }

    /// Create a new store with custom parameters.
    #[cfg(test)]
    pub async fn with_options(
        data_dir: impl AsRef<Path>,
        cache_ttl: Duration,
        cache_max_size: usize,
        max_concurrent_ops: usize,
    ) -> std::io::Result<Self> {
        let threads_dir = data_dir.as_ref().join("threads");
        let sessions_dir = data_dir.as_ref().join("sessions");
        tokio::fs::create_dir_all(&threads_dir).await?;
        tokio::fs::create_dir_all(&sessions_dir).await?;
        Ok(Self {
            canonical_data_dir: threads_dir,
            legacy_data_dir: sessions_dir,
            cache: Mutex::new(HashMap::new()),
            cache_ttl,
            cache_max_size,
            semaphore: Semaphore::new(max_concurrent_ops),
        })
    }

    /// Convert a thread key to a safe filename (v2).
    ///
    /// Uses hex-encoded UTF-8 bytes to guarantee reversibility and avoid
    /// collisions (for example `:` vs `/`).
    fn thread_file(&self, key: &str) -> PathBuf {
        self.canonical_data_dir
            .join(thread_storage_file_name(key, "json"))
    }

    /// Legacy filename mapping retained for backward-compatibility reads.
    fn legacy_thread_file(&self, key: &str) -> PathBuf {
        self.legacy_data_dir
            .join(thread_storage_file_name(key, "json"))
    }

    /// Legacy pre-v2 filename mapping retained for backward-compatibility reads.
    fn legacy_compat_thread_file(&self, key: &str) -> PathBuf {
        let safe = key.replace("::", "__").replace([':', '/'], "_");
        self.legacy_data_dir.join(format!("{safe}.json"))
    }

    /// Return the best on-disk file path for a given key.
    ///
    /// Prefers canonical v2 encoded files; falls back to legacy directory files
    /// and finally legacy pre-v2 filenames when only older data exists.
    fn resolve_thread_file(&self, key: &str) -> PathBuf {
        let canonical = self.thread_file(key);
        if canonical.exists() {
            return canonical;
        }

        let legacy = self.legacy_thread_file(key);
        if legacy.exists() {
            return legacy;
        }

        self.legacy_compat_thread_file(key)
    }

    fn decode_key(hex: &str) -> Option<String> {
        if hex.is_empty() || !hex.len().is_multiple_of(2) {
            return None;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        let mut i = 0;
        while i < hex.len() {
            let chunk = &hex[i..i + 2];
            let b = u8::from_str_radix(chunk, 16).ok()?;
            bytes.push(b);
            i += 2;
        }
        String::from_utf8(bytes)
            .map_err(|e| tracing::warn!(hex, error = %e, "hex-decoded bytes are not valid UTF-8"))
            .ok()
    }

    /// Convert a filename stem back to a thread key.
    fn stem_to_key(stem: &str) -> Option<String> {
        if let Some(encoded) = stem.strip_prefix("k_") {
            return Self::decode_key(encoded);
        }
        Some(stem.replace("__", "::"))
    }

    /// Get file modification time, or `None` if the file does not exist.
    async fn file_mtime(path: &Path) -> Option<SystemTime> {
        tokio::fs::metadata(path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
    }

    /// Evict entries that exceed `cache_max_size`, removing oldest first.
    fn evict_if_needed(cache: &mut HashMap<String, CacheEntry>, max: usize) {
        if cache.len() <= max {
            return;
        }
        // Find the oldest entry and remove it.
        let to_remove = cache.len() - max;
        let mut entries: Vec<(&String, &CacheEntry)> = cache.iter().collect();
        entries.sort_by_key(|(_, e)| e.inserted_at);
        let keys: Vec<String> = entries
            .into_iter()
            .take(to_remove)
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys {
            cache.remove(&k);
        }
    }

    /// Deep-clone a `Value` via serde round-trip. This ensures the returned
    /// value is fully independent of the cached copy.
    fn deep_clone(v: &Value) -> Value {
        v.clone()
    }

    fn storage_roots(&self) -> [&PathBuf; 2] {
        [&self.canonical_data_dir, &self.legacy_data_dir]
    }

    pub async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
        // Check cache first.
        {
            let mut cache = self.cache.lock().await;
            if let Some(entry) = cache.get(thread_id) {
                if entry.inserted_at.elapsed() <= self.cache_ttl {
                    // Quick TTL check passed, now validate mtime (needs fs access).
                    let data = Self::deep_clone(&entry.data);
                    let mtime = entry.mtime;
                    drop(cache);
                    // Validate mtime.
                    let path = self.resolve_thread_file(thread_id);
                    if let Some(disk_mtime) = Self::file_mtime(&path).await
                        && disk_mtime == mtime
                    {
                        return Ok(Some(data));
                    }
                    // Invalidate stale cache entry.
                    let mut cache = self.cache.lock().await;
                    cache.remove(thread_id);
                    debug!(thread_id, "cache invalidated (mtime mismatch)");
                } else {
                    cache.remove(thread_id);
                }
            }
        }

        let path = self.resolve_thread_file(thread_id);
        if !path.exists() {
            return Ok(None);
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|error| ThreadStoreError::Backend(format!("semaphore closed: {error}")))?;

        // Reads never consult historical per-record lock files. The archive is
        // read only during boot import while the lifecycle lock is held.
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| ThreadStoreError::Backend(format!("read failed: {error}")))?;
        let data: Value =
            serde_json::from_slice(&bytes).map_err(|error| ThreadStoreError::Serialization {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            })?;

        // Prefer the canonical path for cache keying, while still accepting a
        // record read from an older storage location.
        let cache_path = self.thread_file(thread_id);
        let Some(mtime) = Self::file_mtime(&cache_path)
            .await
            .or(Self::file_mtime(&path).await)
        else {
            // Deleted between read and stat: treat as a consistent read
            // without caching.
            return Ok(Some(data));
        };

        let mut cache = self.cache.lock().await;
        Self::evict_if_needed(&mut cache, self.cache_max_size);
        cache.insert(
            thread_id.to_owned(),
            CacheEntry {
                data: Self::deep_clone(&data),
                mtime,
                inserted_at: Instant::now(),
            },
        );

        Ok(Some(data))
    }

    pub async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
        let mut keys = Vec::new();
        let mut seen = HashSet::new();
        for root in self.storage_roots() {
            let mut entries = match tokio::fs::read_dir(root).await {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(ThreadStoreError::Backend(format!(
                        "list failed for {}: {error}",
                        root.display()
                    )));
                }
            };

            loop {
                match entries.next_entry().await {
                    Ok(Some(entry)) => {
                        let path = entry.path();
                        if path.extension().is_none_or(|ext| ext != "json") {
                            continue;
                        }
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            let Some(key) = Self::stem_to_key(stem) else {
                                continue;
                            };
                            if let Some(p) = prefix
                                && !key.starts_with(p)
                            {
                                continue;
                            }
                            if seen.insert(key.clone()) {
                                keys.push(key);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        return Err(ThreadStoreError::Backend(format!(
                            "list failed for {}: {error}",
                            root.display()
                        )));
                    }
                }
            }
        }
        Ok(keys)
    }
}

#[cfg(test)]
mod tests;
