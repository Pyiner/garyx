use super::*;
use serde_json::json;
use tempfile::TempDir;

async fn make_store() -> (FileThreadStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = FileThreadStore::new(tmp.path()).await.unwrap();
    (store, tmp)
}

async fn seed_canonical(store: &FileThreadStore, key: &str, data: &Value) {
    tokio::fs::write(
        store.thread_file(key),
        serde_json::to_vec_pretty(data).unwrap(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_get_returns_none_for_missing_key() {
    let (store, _tmp) = make_store().await;
    assert_eq!(store.get("missing").await.unwrap(), None);
}

#[tokio::test]
async fn test_list_keys_with_prefix() {
    let (store, _tmp) = make_store().await;
    seed_canonical(&store, "agent1::main::u1", &json!({})).await;
    seed_canonical(&store, "agent1::main::u2", &json!({})).await;
    seed_canonical(&store, "agent2::main::u1", &json!({})).await;

    let all = store.list_keys(None).await.unwrap();
    assert_eq!(all.len(), 3);

    let mut filtered = store.list_keys(Some("agent1::")).await.unwrap();
    filtered.sort();
    assert_eq!(filtered, vec!["agent1::main::u1", "agent1::main::u2"]);
}

#[tokio::test]
async fn test_key_to_filename_roundtrip() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::user123";
    let path = store.thread_file(key);
    assert!(path.to_str().unwrap().contains("k_"));

    let stem = path.file_stem().unwrap().to_str().unwrap();
    assert_eq!(FileThreadStore::stem_to_key(stem), Some(key.to_owned()));
}

#[tokio::test]
async fn test_key_encoding_avoids_filename_collisions() {
    let (store, _tmp) = make_store().await;
    assert_ne!(store.thread_file("a:b"), store.thread_file("a/b"));
}

#[tokio::test]
async fn test_get_reads_legacy_filename() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::legacy";
    tokio::fs::write(store.legacy_thread_file(key), br#"{"v":"legacy"}"#)
        .await
        .unwrap();

    let loaded = store.get(key).await.unwrap().unwrap();
    assert_eq!(loaded["v"], "legacy");
}

#[tokio::test]
async fn test_get_reads_legacy_compat_filename() {
    let (store, _tmp) = make_store().await;
    let key = "main::main::legacy-safe";
    tokio::fs::write(
        store.legacy_compat_thread_file(key),
        br#"{"v":"legacy-safe"}"#,
    )
    .await
    .unwrap();

    let loaded = store.get(key).await.unwrap().unwrap();
    assert_eq!(loaded["v"], "legacy-safe");
}

#[tokio::test]
async fn test_cache_hit_preserves_deep_clone_isolation() {
    let (store, _tmp) = make_store().await;
    seed_canonical(&store, "cached", &json!({"arr": [1, 2, 3]})).await;

    let mut first = store.get("cached").await.unwrap().unwrap();
    first["arr"] = json!([99]);

    let second = store.get("cached").await.unwrap().unwrap();
    assert_eq!(second["arr"], json!([1, 2, 3]));
}

#[tokio::test]
async fn test_cache_invalidation_on_external_write() {
    let (store, _tmp) = make_store().await;
    seed_canonical(&store, "external", &json!({"v": 1})).await;
    let _ = store.get("external").await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    seed_canonical(&store, "external", &json!({"v": 2})).await;

    let loaded = store.get("external").await.unwrap().unwrap();
    assert_eq!(loaded["v"], 2);
}

#[tokio::test]
async fn test_cache_ttl_expiry() {
    let tmp = TempDir::new().unwrap();
    let store = FileThreadStore::with_options(
        tmp.path(),
        Duration::from_millis(100),
        1000,
        DEFAULT_MAX_CONCURRENT_OPS,
    )
    .await
    .unwrap();
    seed_canonical(&store, "ttl", &json!({"v": 1})).await;
    let _ = store.get("ttl").await.unwrap();

    tokio::time::sleep(Duration::from_millis(150)).await;

    let loaded = store.get("ttl").await.unwrap().unwrap();
    assert_eq!(loaded["v"], 1);
}

#[tokio::test]
async fn test_cache_evicts_oldest_entry() {
    let tmp = TempDir::new().unwrap();
    let store =
        FileThreadStore::with_options(tmp.path(), DEFAULT_CACHE_TTL, 1, DEFAULT_MAX_CONCURRENT_OPS)
            .await
            .unwrap();
    for key in ["first", "second", "third"] {
        seed_canonical(&store, key, &json!({"key": key})).await;
        store.get(key).await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    let cache = store.cache.lock().await;
    assert!(!cache.contains_key("first"));
    assert!(cache.contains_key("second"));
    assert!(cache.contains_key("third"));
}

#[tokio::test]
async fn test_get_ignores_historical_write_lock_file() {
    let tmp = TempDir::new().unwrap();
    let store =
        FileThreadStore::with_options(tmp.path(), Duration::ZERO, 1000, DEFAULT_MAX_CONCURRENT_OPS)
            .await
            .unwrap();
    seed_canonical(&store, "busy", &json!({"v": 1})).await;
    let lock_path = store.thread_file("busy").with_extension("lock");
    tokio::fs::write(&lock_path, b"").await.unwrap();

    let loaded = store.get("busy").await.unwrap().unwrap();
    assert_eq!(loaded["v"], 1);
    assert!(lock_path.exists(), "reads must not touch old lock files");
}
