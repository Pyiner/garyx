use super::*;
use serde_json::json;
use tempfile::TempDir;

async fn make_store() -> (FileThreadStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = FileThreadStore::new(tmp.path()).await.unwrap();
    (store, tmp)
}

// ---------------------------------------------------------------
// Basic CRUD
// ---------------------------------------------------------------

#[tokio::test]
async fn test_basic_crud() {
    let (store, _tmp) = make_store().await;

    // Initially empty.
    assert_eq!(store.size().await.unwrap(), 0);
    assert!(!store.exists("k1").await);
    assert_eq!(store.get("k1").await, None);

    // Set and get.
    store.set("k1", json!({"hello": "world"})).await;
    assert!(store.exists("k1").await);
    assert_eq!(store.size().await.unwrap(), 1);
    let v = store.get("k1").await.unwrap();
    assert_eq!(v["hello"], "world");

    // Update.
    store.update("k1", json!({"foo": "bar"})).await.unwrap();
    let v = store.get("k1").await.unwrap();
    assert_eq!(v["hello"], "world");
    assert_eq!(v["foo"], "bar");

    // Delete.
    assert!(store.delete("k1").await);
    assert!(!store.delete("k1").await);
    assert_eq!(store.size().await.unwrap(), 0);
}

#[tokio::test]
async fn test_list_keys_with_prefix() {
    let (store, _tmp) = make_store().await;
    store.set("agent1::main::u1", json!({})).await;
    store.set("agent1::main::u2", json!({})).await;
    store.set("agent2::main::u1", json!({})).await;

    let all = store.list_keys(None).await;
    assert_eq!(all.len(), 3);

    let mut filtered = store.list_keys(Some("agent1::")).await;
    filtered.sort();
    assert_eq!(filtered, vec!["agent1::main::u1", "agent1::main::u2"]);
}

#[tokio::test]
async fn test_update_missing_key() {
    let (store, _tmp) = make_store().await;
    let result = store.update("missing", json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_clear() {
    let (store, _tmp) = make_store().await;
    store.set("a", json!(1)).await;
    store.set("b", json!(2)).await;
    assert_eq!(store.size().await.unwrap(), 2);
    let cleared = store.clear().await.unwrap();
    assert_eq!(cleared, 2);
    assert_eq!(store.size().await.unwrap(), 0);
}

// ---------------------------------------------------------------
// Key/filename conversion
// ---------------------------------------------------------------

#[tokio::test]
async fn test_key_to_filename_roundtrip() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::user123";
    let path = store.thread_file(key);
    assert!(path.to_str().unwrap().contains("k_"));

    // Roundtrip.
    let stem = path.file_stem().unwrap().to_str().unwrap();
    assert_eq!(FileThreadStore::stem_to_key(stem), Some(key.to_owned()));
}

#[tokio::test]
async fn test_key_encoding_avoids_filename_collisions() {
    let (store, _tmp) = make_store().await;
    let a = store.thread_file("a:b");
    let b = store.thread_file("a/b");
    assert_ne!(a, b);
}

#[tokio::test]
async fn test_get_reads_legacy_filename() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::legacy";
    let legacy_path = store.legacy_thread_file(key);
    tokio::fs::write(&legacy_path, br#"{"v":"legacy"}"#)
        .await
        .unwrap();

    let loaded = store.get(key).await.unwrap();
    assert_eq!(loaded["v"], "legacy");
}

#[tokio::test]
async fn test_get_reads_legacy_compat_filename() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::legacy-safe";
    let legacy_path = store.legacy_compat_thread_file(key);
    tokio::fs::write(&legacy_path, br#"{"v":"legacy-safe"}"#)
        .await
        .unwrap();

    let loaded = store.get(key).await.unwrap();
    assert_eq!(loaded["v"], "legacy-safe");
}

#[tokio::test]
async fn test_set_migrates_legacy_filename_to_modern_filename() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::legacy-migrate";
    let legacy_path = store.legacy_thread_file(key);
    let modern_path = store.thread_file(key);
    tokio::fs::write(&legacy_path, br#"{"v":"legacy"}"#)
        .await
        .unwrap();

    store.set(key, json!({"v":"modern"})).await;

    assert!(modern_path.exists());
    assert!(!legacy_path.exists());
    let loaded = store.get(key).await.unwrap();
    assert_eq!(loaded["v"], "modern");
}

#[tokio::test]
async fn test_set_migrates_legacy_compat_filename_to_canonical_directory() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::legacy-safe-migrate";
    let legacy_path = store.legacy_compat_thread_file(key);
    let modern_path = store.thread_file(key);
    tokio::fs::write(&legacy_path, br#"{"v":"legacy"}"#)
        .await
        .unwrap();

    store.set(key, json!({"v":"modern"})).await;

    assert!(modern_path.exists());
    assert!(!legacy_path.exists());
    let loaded = store.get(key).await.unwrap();
    assert_eq!(loaded["v"], "modern");
}

#[tokio::test]
async fn test_set_uses_legacy_lock_while_migrating_legacy_file() {
    let tmp = TempDir::new().unwrap();
    let store = FileThreadStore::with_options(
        tmp.path(),
        DEFAULT_CACHE_TTL,
        DEFAULT_CACHE_MAX_SIZE,
        Duration::from_millis(50),
        DEFAULT_MAX_CONCURRENT_OPS,
    )
    .await
    .unwrap();
    let key = "main::main::legacy-locked";
    let legacy_path = store.legacy_thread_file(key);
    let modern_path = store.thread_file(key);
    tokio::fs::write(&legacy_path, br#"{"v":"legacy"}"#)
        .await
        .unwrap();

    let legacy_lock_path = FileThreadStore::lock_file_for_path(&legacy_path);
    tokio::fs::write(&legacy_lock_path, b"").await.unwrap();

    store.set(key, json!({"v":"modern"})).await;

    assert!(legacy_path.exists());
    assert!(!modern_path.exists());
    tokio::fs::remove_file(&legacy_lock_path).await.unwrap();
    let loaded = store.get(key).await.unwrap();
    assert_eq!(loaded["v"], "legacy");
}

#[tokio::test]
async fn test_set_uses_legacy_compat_lock_while_migrating_legacy_compat_file() {
    let tmp = TempDir::new().unwrap();
    let store = FileThreadStore::with_options(
        tmp.path(),
        DEFAULT_CACHE_TTL,
        DEFAULT_CACHE_MAX_SIZE,
        Duration::from_millis(50),
        DEFAULT_MAX_CONCURRENT_OPS,
    )
    .await
    .unwrap();
    let key = "main::main::legacy-safe-locked";
    let legacy_path = store.legacy_compat_thread_file(key);
    let modern_path = store.thread_file(key);
    tokio::fs::write(&legacy_path, br#"{"v":"legacy"}"#)
        .await
        .unwrap();

    let legacy_lock_path = FileThreadStore::lock_file_for_path(&legacy_path);
    tokio::fs::write(&legacy_lock_path, b"").await.unwrap();

    store.set(key, json!({"v":"modern"})).await;

    assert!(legacy_path.exists());
    assert!(!modern_path.exists());
    tokio::fs::remove_file(&legacy_lock_path).await.unwrap();
    let loaded = store.get(key).await.unwrap();
    assert_eq!(loaded["v"], "legacy");
}

// ---------------------------------------------------------------
// Cache behaviour
// ---------------------------------------------------------------

#[tokio::test]
async fn test_cache_hit() {
    let (store, _tmp) = make_store().await;
    store.set("c1", json!({"val": 1})).await;

    // First read populates cache (or it was populated by set).
    let v1 = store.get("c1").await.unwrap();
    assert_eq!(v1["val"], 1);

    // Second read should come from cache.
    let v2 = store.get("c1").await.unwrap();
    assert_eq!(v2["val"], 1);
}

#[tokio::test]
async fn test_cache_invalidation_on_external_write() {
    let (store, _tmp) = make_store().await;
    store.set("ext", json!({"v": 1})).await;

    // Read to populate cache.
    let _ = store.get("ext").await;

    // Externally modify the file (simulate another process).
    let path = store.thread_file("ext");
    // Wait a tiny bit so mtime differs.
    tokio::time::sleep(Duration::from_millis(50)).await;
    tokio::fs::write(&path, serde_json::to_vec_pretty(&json!({"v": 2})).unwrap())
        .await
        .unwrap();

    // Next read should detect mtime change and return fresh data.
    let v = store.get("ext").await.unwrap();
    assert_eq!(v["v"], 2);
}

#[tokio::test]
async fn test_cache_ttl_expiry() {
    let tmp = TempDir::new().unwrap();
    let store = FileThreadStore::with_options(
        tmp.path(),
        Duration::from_millis(100), // very short TTL
        1000,
        DEFAULT_LOCK_TIMEOUT,
        DEFAULT_MAX_CONCURRENT_OPS,
    )
    .await
    .unwrap();

    store.set("ttl", json!({"v": 1})).await;
    let _ = store.get("ttl").await; // populate cache

    // Wait for TTL to expire.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Should still return data (reads from disk after cache expires).
    let v = store.get("ttl").await.unwrap();
    assert_eq!(v["v"], 1);
}

// ---------------------------------------------------------------
// Stale lock cleanup
// ---------------------------------------------------------------

#[tokio::test]
async fn test_stale_lock_cleanup() {
    let (store, _tmp) = make_store().await;

    // Create a fake stale lock file.
    let thread_path = store.thread_file("stale");
    let lock_path = FileThreadStore::lock_file_for_path(&thread_path);
    tokio::fs::write(&lock_path, b"").await.unwrap();

    // Backdate it.
    let old_time = SystemTime::now() - Duration::from_secs(60);
    filetime::set_file_mtime(&lock_path, filetime::FileTime::from_system_time(old_time)).unwrap();

    // The stale check should clean it up.
    store.check_stale_lock(&thread_path).await;
    assert!(!lock_path.exists());
}

// ---------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------

#[tokio::test]
async fn test_atomic_write_leaves_no_tmp() {
    let (store, _tmp) = make_store().await;
    store.set("atom", json!({"x": 42})).await;

    let tmp_path = store.thread_file("atom").with_extension("tmp");
    assert!(!tmp_path.exists(), "temp file should not remain on disk");
}

// ---------------------------------------------------------------
// Concurrent access
// ---------------------------------------------------------------

#[tokio::test]
async fn test_concurrent_writes() {
    let (store, _tmp) = make_store().await;
    let store = std::sync::Arc::new(store);

    let mut handles = Vec::new();
    for i in 0..20 {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            s.set("concurrent", json!({"writer": i})).await;
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    // Must have exactly one file, with data from one writer.
    assert!(store.exists("concurrent").await);
    let v = store.get("concurrent").await.unwrap();
    assert!(v["writer"].is_number());
}

#[tokio::test]
async fn test_concurrent_read_write() {
    let (store, _tmp) = make_store().await;
    let store = std::sync::Arc::new(store);

    store.set("rw", json!({"v": 0})).await;

    let mut handles = Vec::new();
    for i in 0..10 {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            s.set("rw", json!({"v": i})).await;
        }));
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            let _ = s.get("rw").await;
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    // Thread entry should exist and be valid JSON.
    let v = store.get("rw").await.unwrap();
    assert!(v["v"].is_number());
}

// ---------------------------------------------------------------
// Exists & delete edge cases
// ---------------------------------------------------------------

#[tokio::test]
async fn test_exists_after_delete() {
    let (store, _tmp) = make_store().await;
    store.set("del", json!({})).await;
    assert!(store.exists("del").await);
    store.delete("del").await;
    assert!(!store.exists("del").await);
}

#[tokio::test]
async fn test_get_returns_none_after_delete() {
    let (store, _tmp) = make_store().await;
    store.set("gd", json!({"a": 1})).await;
    assert!(store.get("gd").await.is_some());
    store.delete("gd").await;
    assert!(store.get("gd").await.is_none());
}

// ---------------------------------------------------------------
// Deep clone isolation
// ---------------------------------------------------------------

#[tokio::test]
async fn test_deep_clone_isolation() {
    let (store, _tmp) = make_store().await;
    store.set("iso", json!({"arr": [1, 2, 3]})).await;

    let mut v1 = store.get("iso").await.unwrap();
    // Mutate the returned value.
    v1["arr"] = json!([99]);

    // A subsequent get should return the original data.
    let v2 = store.get("iso").await.unwrap();
    assert_eq!(v2["arr"], json!([1, 2, 3]));
}

// ---------------------------------------------------------------
// Update merges only top-level keys
// ---------------------------------------------------------------

// ---------------------------------------------------------------
// Legacy team-chat scrub on load
// ---------------------------------------------------------------

#[tokio::test]
async fn test_get_scrubs_legacy_team_fields_and_repersists() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::scrub-me";
    let path = store.thread_file(key);
    // Write a fossil-bearing file directly to disk so `set()`'s own
    // scrub path doesn't interfere with the test.
    let fossil = serde_json::to_vec_pretty(&json!({
        "thread_id": key,
        "team_run_id": "run-xyz",
        "team_chat_messages": [
            {"role": "user", "content": "a"},
            {"role": "assistant", "content": "b"},
        ],
        "messages": [ {"role": "user", "content": "x"} ],
    }))
    .unwrap();
    tokio::fs::write(&path, &fossil).await.unwrap();

    // First read: scrub + re-persist.
    let v = store.get(key).await.unwrap();
    let obj = v.as_object().unwrap();
    assert!(!obj.contains_key("team_run_id"));
    assert!(!obj.contains_key("team_chat_messages"));
    let msgs = obj.get("messages").and_then(|v| v.as_array()).unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0]["content"], "x");
    assert_eq!(msgs[1]["content"], "a");
    assert_eq!(msgs[2]["content"], "b");

    // Disk should now be clean; read raw bytes to confirm scrub was
    // re-persisted rather than only applied in memory.
    let disk_bytes = tokio::fs::read(&path).await.unwrap();
    let disk: serde_json::Value = serde_json::from_slice(&disk_bytes).unwrap();
    let disk_obj = disk.as_object().unwrap();
    assert!(!disk_obj.contains_key("team_run_id"));
    assert!(!disk_obj.contains_key("team_chat_messages"));
}

#[tokio::test]
async fn test_update_preserves_existing_keys() {
    let (store, _tmp) = make_store().await;
    store.set("up", json!({"a": 1, "b": 2})).await;
    store.update("up", json!({"b": 20, "c": 3})).await.unwrap();

    let v = store.get("up").await.unwrap();
    assert_eq!(v["a"], 1);
    assert_eq!(v["b"], 20);
    assert_eq!(v["c"], 3);
}
