use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, error};

use crate::scrub::scrub_legacy_team_fields;
use crate::store::{ThreadStore, ThreadStoreError};

// Constants matching the Python implementation.
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(45);
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const STALE_LOCK_THRESHOLD: Duration = Duration::from_secs(30);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(25);
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

/// File-based thread storage with caching and locking.
///
/// Port of the Python `FileThreadStore` with identical semantics:
/// - JSON file storage per thread
/// - Lock-file based concurrency control with stale lock detection
/// - In-memory TTL cache validated against file mtime
/// - Atomic writes via temp-file + rename (POSIX)
/// - Semaphore-limited concurrent file operations
pub struct FileThreadStore {
    canonical_data_dir: PathBuf,
    legacy_data_dir: PathBuf,
    cache: Mutex<HashMap<String, CacheEntry>>,
    cache_ttl: Duration,
    cache_max_size: usize,
    lock_timeout: Duration,
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
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            semaphore: Semaphore::new(DEFAULT_MAX_CONCURRENT_OPS),
        })
    }

    /// Create a new store with custom parameters.
    pub async fn with_options(
        data_dir: impl AsRef<Path>,
        cache_ttl: Duration,
        cache_max_size: usize,
        lock_timeout: Duration,
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
            lock_timeout,
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

    /// Return the thread path whose lock should be used for writes.
    ///
    /// When only legacy data exists, keep using the legacy lock during the
    /// first write so legacy readers and updaters still serialize correctly
    /// while the data is migrated to the canonical location.
    fn resolve_write_lock_thread_file(&self, key: &str) -> PathBuf {
        let canonical = self.thread_file(key);
        if canonical.exists() {
            return canonical;
        }

        let legacy = self.legacy_thread_file(key);
        if legacy.exists() {
            return legacy;
        }

        let legacy_compat = self.legacy_compat_thread_file(key);
        if legacy_compat.exists() {
            legacy_compat
        } else {
            canonical
        }
    }

    fn lock_file_for_path(path: &Path) -> PathBuf {
        path.with_extension("lock")
    }

    fn decode_key(hex: &str) -> Option<String> {
        if hex.is_empty() || hex.len() % 2 != 0 {
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

    /// Check for and remove stale lock files (older than [`STALE_LOCK_THRESHOLD`]).
    async fn check_stale_lock(&self, thread_path: &Path) {
        let lock_path = Self::lock_file_for_path(thread_path);
        if let Ok(meta) = tokio::fs::metadata(&lock_path).await {
            if let Ok(modified) = meta.modified() {
                if let Ok(age) = SystemTime::now().duration_since(modified) {
                    if age > STALE_LOCK_THRESHOLD {
                        debug!(
                            thread_path = %thread_path.display(),
                            age_secs = age.as_secs(),
                            "removing stale lock"
                        );
                        let _ = tokio::fs::remove_file(&lock_path).await;
                    }
                }
            }
        }
    }

    /// Acquire a lock file, polling until timeout.
    ///
    /// Returns a [`LockGuard`] that removes the lock file on drop (async cleanup
    /// is handled by the caller via [`release_lock`]).
    async fn acquire_lock(&self, thread_path: &Path) -> Result<(), std::io::Error> {
        let lock_path = Self::lock_file_for_path(thread_path);
        let deadline = Instant::now() + self.lock_timeout;

        loop {
            // Try to create the lock file exclusively.
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!("lock timeout for path: {}", thread_path.display()),
                        ));
                    }
                    tokio::time::sleep(LOCK_POLL_INTERVAL).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Release a lock file.
    async fn release_lock(&self, thread_path: &Path) {
        let _ = tokio::fs::remove_file(Self::lock_file_for_path(thread_path)).await;
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

    /// Atomic write: write to a temp file then rename.
    async fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, data).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }

    fn storage_roots(&self) -> [&PathBuf; 2] {
        [&self.canonical_data_dir, &self.legacy_data_dir]
    }

    /// Remove all thread records and lock files.
    pub async fn clear(&self) -> std::io::Result<usize> {
        let mut count = 0usize;
        for root in self.storage_roots() {
            let mut entries = tokio::fs::read_dir(root).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "json" || ext == "lock" {
                        if ext == "json" {
                            count += 1;
                        }
                        let _ = tokio::fs::remove_file(&path).await;
                    }
                }
            }
        }
        self.cache.lock().await.clear();
        Ok(count)
    }

    /// Return the number of thread files on disk.
    pub async fn size(&self) -> std::io::Result<usize> {
        let mut count = 0usize;
        for root in self.storage_roots() {
            let mut entries = tokio::fs::read_dir(root).await?;
            while let Some(entry) = entries.next_entry().await? {
                if entry.path().extension().is_some_and(|ext| ext == "json") {
                    count += 1;
                }
            }
        }
        Ok(count)
    }
}

#[async_trait]
impl ThreadStore for FileThreadStore {
    async fn get(&self, thread_id: &str) -> Option<Value> {
        // Check cache first.
        {
            let mut cache = self.cache.lock().await;
            if let Some(entry) = cache.get(thread_id) {
                if entry.inserted_at.elapsed() <= self.cache_ttl {
                    // Quick TTL check passed, now validate mtime (needs fs access).
                    let mut data = Self::deep_clone(&entry.data);
                    let mtime = entry.mtime;
                    drop(cache);
                    // Validate mtime.
                    let path = self.resolve_thread_file(thread_id);
                    if let Some(disk_mtime) = Self::file_mtime(&path).await {
                        if disk_mtime == mtime {
                            // Defensive scrub on cache-hit: fresh inserts
                            // are already scrubbed, but an older cached
                            // entry from before this migration landed
                            // could still carry fossils.
                            let _ = scrub_legacy_team_fields(&mut data);
                            return Some(data);
                        }
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
            return None;
        }

        self.check_stale_lock(&path).await;

        let _permit = self.semaphore.acquire().await.ok()?;
        if let Err(e) = self.acquire_lock(&path).await {
            error!(thread_id, error = %e, "lock timeout on get");
            return None;
        }

        let result = async {
            let bytes = tokio::fs::read(&path).await.ok()?;
            let mut data: Value = serde_json::from_slice(&bytes).ok()?;

            // One-shot migration: strip legacy team-chat fossils and, if
            // anything was mutated, re-persist the cleaned record so
            // subsequent loads short-circuit. We already hold the lock
            // for this thread_id, so a direct atomic_write here is safe.
            let scrubbed = scrub_legacy_team_fields(&mut data);
            if scrubbed {
                // Write back to the canonical location. On a legacy
                // compat path the write still lands in the canonical
                // v2 directory, which matches the migration semantics
                // of `set()`.
                let canonical_path = self.thread_file(thread_id);
                match serde_json::to_vec_pretty(&data) {
                    Ok(bytes) => {
                        if let Err(e) = Self::atomic_write(&canonical_path, &bytes).await {
                            error!(thread_id, error = %e, "failed to re-persist scrubbed thread");
                        } else {
                            // If we just migrated out of a legacy
                            // location, best-effort drop the old file so
                            // the resolver stops preferring it.
                            let legacy_path = self.legacy_thread_file(thread_id);
                            if legacy_path != canonical_path && legacy_path.exists() {
                                let _ = tokio::fs::remove_file(&legacy_path).await;
                                let _ =
                                    tokio::fs::remove_file(Self::lock_file_for_path(&legacy_path))
                                        .await;
                            }
                            let legacy_compat_path = self.legacy_compat_thread_file(thread_id);
                            if legacy_compat_path != canonical_path && legacy_compat_path.exists() {
                                let _ = tokio::fs::remove_file(&legacy_compat_path).await;
                                let _ = tokio::fs::remove_file(Self::lock_file_for_path(
                                    &legacy_compat_path,
                                ))
                                .await;
                            }
                        }
                    }
                    Err(e) => {
                        error!(thread_id, error = %e, "failed to serialize scrubbed thread");
                    }
                }
            }

            // Use the canonical path for mtime / cache keying so a
            // just-migrated record's cache entry matches what the next
            // `get()` sees on disk.
            let cache_path = self.thread_file(thread_id);
            let mtime = Self::file_mtime(&cache_path)
                .await
                .or(Self::file_mtime(&path).await)?;

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

            Some(data)
        }
        .await;

        self.release_lock(&path).await;
        result
    }

    async fn set(&self, thread_id: &str, data: Value) {
        let path = self.thread_file(thread_id);
        let legacy_path = self.legacy_thread_file(thread_id);
        let legacy_compat_path = self.legacy_compat_thread_file(thread_id);
        let lock_thread_path = self.resolve_write_lock_thread_file(thread_id);

        self.check_stale_lock(&lock_thread_path).await;

        let _permit = match self.semaphore.acquire().await {
            Ok(permit) => permit,
            Err(e) => {
                error!(thread_id, error = %e, "semaphore closed on set");
                return;
            }
        };

        if let Err(e) = self.acquire_lock(&lock_thread_path).await {
            error!(thread_id, error = %e, "lock timeout on set");
            return;
        }

        let result = async {
            let bytes = serde_json::to_vec_pretty(&data)?;
            Self::atomic_write(&path, &bytes).await?;
            if legacy_path != path && legacy_path.exists() {
                let _ = tokio::fs::remove_file(&legacy_path).await;
                let _ = tokio::fs::remove_file(Self::lock_file_for_path(&legacy_path)).await;
            }
            if legacy_compat_path != path && legacy_compat_path.exists() {
                let _ = tokio::fs::remove_file(&legacy_compat_path).await;
                let _ = tokio::fs::remove_file(Self::lock_file_for_path(&legacy_compat_path)).await;
            }

            let mtime = Self::file_mtime(&path)
                .await
                .unwrap_or(SystemTime::UNIX_EPOCH);

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

            debug!(thread_id, "saved thread");
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await;

        self.release_lock(&lock_thread_path).await;

        if let Err(e) = result {
            error!(thread_id, error = %e, "failed to set thread");
        }
    }

    async fn delete(&self, thread_id: &str) -> bool {
        // Remove from cache first.
        self.cache.lock().await.remove(thread_id);

        let path = self.resolve_thread_file(thread_id);
        if !path.exists() {
            return false;
        }

        self.check_stale_lock(&path).await;

        let permit = self.semaphore.acquire().await;
        if permit.is_err() {
            return false;
        }

        if let Err(e) = self.acquire_lock(&path).await {
            error!(thread_id, error = %e, "lock timeout on delete");
            return false;
        }

        let ok = tokio::fs::remove_file(&path).await.is_ok();

        // Release lock and clean up the lock file.
        self.release_lock(&path).await;

        if ok {
            debug!(thread_id, "deleted thread");
        }
        ok
    }

    async fn list_keys(&self, prefix: Option<&str>) -> Vec<String> {
        let mut keys = Vec::new();
        let mut seen = HashSet::new();
        for root in self.storage_roots() {
            let mut entries = match tokio::fs::read_dir(root).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "json") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let Some(key) = Self::stem_to_key(stem) else {
                        continue;
                    };
                    if let Some(p) = prefix {
                        if !key.starts_with(p) {
                            continue;
                        }
                    }
                    if seen.insert(key.clone()) {
                        keys.push(key);
                    }
                }
            }
        }
        keys
    }

    async fn exists(&self, thread_id: &str) -> bool {
        // Fast path: check cache.
        {
            let cache = self.cache.lock().await;
            if cache.contains_key(thread_id) {
                return true;
            }
        }
        self.thread_file(thread_id).exists()
            || self.legacy_thread_file(thread_id).exists()
            || self.legacy_compat_thread_file(thread_id).exists()
    }

    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
        let path = self.resolve_thread_file(thread_id);

        self.check_stale_lock(&path).await;

        let _permit =
            self.semaphore.acquire().await.map_err(|_| {
                ThreadStoreError::NotFound(format!("semaphore error for {thread_id}"))
            })?;

        self.acquire_lock(&path).await.map_err(|e| {
            error!(thread_id, error = %e, "lock timeout on update");
            ThreadStoreError::NotFound(format!("lock timeout for {thread_id}"))
        })?;

        let result = async {
            if !path.exists() {
                return Err(ThreadStoreError::NotFound(thread_id.to_owned()));
            }

            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|_| ThreadStoreError::NotFound(thread_id.to_owned()))?;

            let mut data: Value = serde_json::from_slice(&bytes)
                .map_err(|_| ThreadStoreError::NotFound(thread_id.to_owned()))?;

            // Merge top-level keys.
            if let (Some(existing), Some(new_fields)) = (data.as_object_mut(), updates.as_object())
            {
                for (k, v) in new_fields {
                    existing.insert(k.clone(), v.clone());
                }
            }

            let out_bytes = serde_json::to_vec_pretty(&data)
                .map_err(|_| ThreadStoreError::NotFound(thread_id.to_owned()))?;
            Self::atomic_write(&path, &out_bytes)
                .await
                .map_err(|_| ThreadStoreError::NotFound(thread_id.to_owned()))?;

            let mtime = Self::file_mtime(&path)
                .await
                .unwrap_or(SystemTime::UNIX_EPOCH);

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

            Ok(())
        }
        .await;

        self.release_lock(&path).await;
        result
    }
}

#[cfg(test)]
mod tests;
