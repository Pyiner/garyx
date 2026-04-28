use super::*;
use std::thread::sleep;
use std::time::Duration;
use tempfile::TempDir;

fn sample_group() -> Group {
    let mut g = Group::new("th::group-1", "team-alpha");
    g.record_child_thread("planner", "th::planner-abc");
    g.record_child_thread("coder", "th::coder-xyz");
    g.advance_catch_up("planner", 7);
    g.advance_catch_up("coder", 3);
    g
}

#[test]
fn group_new_initializes_empty_maps_and_timestamps() {
    let g = Group::new("th::g", "team-x");
    assert_eq!(g.group_thread_id, "th::g");
    assert_eq!(g.team_id, "team-x");
    assert!(g.child_threads.is_empty());
    assert!(g.catch_up_offsets.is_empty());
    // created_at == updated_at at construction time.
    assert_eq!(g.created_at, g.updated_at);
}

#[test]
fn record_child_thread_bumps_updated_at() {
    let mut g = Group::new("th::g", "team-x");
    let before = g.updated_at;
    sleep(Duration::from_millis(5));
    g.record_child_thread("planner", "th::p");
    assert!(g.updated_at >= before);
    assert_ne!(g.updated_at, before);
    assert_eq!(g.child_thread("planner"), Some("th::p"));
}

#[test]
fn advance_catch_up_bumps_updated_at() {
    let mut g = Group::new("th::g", "team-x");
    let before = g.updated_at;
    sleep(Duration::from_millis(5));
    g.advance_catch_up("planner", 42);
    assert!(g.updated_at >= before);
    assert_ne!(g.updated_at, before);
    assert_eq!(g.catch_up_offset("planner"), 42);
}

#[test]
fn catch_up_offset_defaults_to_zero_for_unknown_agent() {
    let g = Group::new("th::g", "team-x");
    assert_eq!(g.catch_up_offset("ghost"), 0);
}

#[test]
fn child_thread_returns_none_for_unknown_agent() {
    let g = Group::new("th::g", "team-x");
    assert_eq!(g.child_thread("ghost"), None);
}

#[tokio::test]
async fn file_store_save_load_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let store = FileGroupStore::new(tmp.path().to_path_buf());

    let g = sample_group();
    store.save(&g).await;

    // Drop in-memory cache to force disk read on next load.
    store.cache.write().await.clear();

    let loaded = store.load("th::group-1").await.expect("group should load");
    assert_eq!(loaded, g);
}

#[tokio::test]
async fn file_store_load_hits_cache_without_disk() {
    let tmp = TempDir::new().unwrap();
    let store = FileGroupStore::new(tmp.path().to_path_buf());

    let g = sample_group();
    store.save(&g).await;

    // Corrupt the on-disk file; a cache hit must still succeed.
    let path = store.file_path(&g.group_thread_id);
    std::fs::write(&path, b"not valid json").unwrap();

    let loaded = store.load("th::group-1").await.expect("cache hit");
    assert_eq!(loaded, g);
}

#[tokio::test]
async fn file_store_delete_removes_cache_and_file() {
    let tmp = TempDir::new().unwrap();
    let store = FileGroupStore::new(tmp.path().to_path_buf());

    let g = sample_group();
    store.save(&g).await;

    let path = store.file_path(&g.group_thread_id);
    assert!(path.exists(), "file should exist after save");
    assert!(store.cache.read().await.contains_key(&g.group_thread_id));

    store.delete(&g.group_thread_id).await;

    assert!(!path.exists(), "file should be gone after delete");
    assert!(!store.cache.read().await.contains_key(&g.group_thread_id));
    assert!(store.load(&g.group_thread_id).await.is_none());
}

#[tokio::test]
async fn file_store_load_missing_returns_none_without_error() {
    let tmp = TempDir::new().unwrap();
    let store = FileGroupStore::new(tmp.path().to_path_buf());

    assert!(store.load("th::never-saved").await.is_none());
}

#[tokio::test]
async fn file_store_two_ids_store_to_two_files() {
    let tmp = TempDir::new().unwrap();
    let store = FileGroupStore::new(tmp.path().to_path_buf());

    let mut g1 = Group::new("th::group-1", "team-alpha");
    g1.advance_catch_up("a", 1);
    let mut g2 = Group::new("th::group-2", "team-beta");
    g2.advance_catch_up("b", 2);

    store.save(&g1).await;
    store.save(&g2).await;

    let p1 = store.file_path("th::group-1");
    let p2 = store.file_path("th::group-2");
    assert_ne!(p1, p2);
    assert!(p1.exists());
    assert!(p2.exists());

    // Clear cache to force disk reads.
    store.cache.write().await.clear();

    let loaded1 = store.load("th::group-1").await.unwrap();
    let loaded2 = store.load("th::group-2").await.unwrap();
    assert_eq!(loaded1, g1);
    assert_eq!(loaded2, g2);
}

#[tokio::test]
async fn file_store_thread_id_with_colons_roundtrips() {
    let tmp = TempDir::new().unwrap();
    let store = FileGroupStore::new(tmp.path().to_path_buf());

    let id = "th::abc-123:nested";
    let g = Group::new(id, "team-colon");
    store.save(&g).await;

    // Sanitization is deterministic: saving and loading with the same
    // id must land on the same file.
    let path = store.file_path(id);
    assert!(path.exists(), "sanitized path must exist");
    // Filename must not contain raw colons.
    let filename = path.file_name().unwrap().to_string_lossy().to_string();
    assert!(!filename.contains(':'));

    // Clear cache to exercise the disk-backed path.
    store.cache.write().await.clear();

    let loaded = store.load(id).await.expect("load by original id");
    assert_eq!(loaded, g);
}

#[tokio::test]
async fn file_store_save_creates_base_dir_lazily() {
    let tmp = TempDir::new().unwrap();
    // Point at a sub-path that doesn't exist yet.
    let base = tmp.path().join("nested").join("agent-team-groups");
    assert!(!base.exists());

    let store = FileGroupStore::new(base.clone());
    store.save(&Group::new("th::lazy", "team-lazy")).await;

    assert!(base.exists(), "save must create the base dir");
    assert!(store.file_path("th::lazy").exists());
}

#[tokio::test]
async fn file_store_save_overwrites_existing_record() {
    let tmp = TempDir::new().unwrap();
    let store = FileGroupStore::new(tmp.path().to_path_buf());

    let mut g = Group::new("th::ow", "team-ow");
    g.advance_catch_up("a", 1);
    store.save(&g).await;

    g.advance_catch_up("a", 99);
    g.record_child_thread("a", "th::child-a");
    store.save(&g).await;

    store.cache.write().await.clear();
    let loaded = store.load("th::ow").await.unwrap();
    assert_eq!(loaded.catch_up_offset("a"), 99);
    assert_eq!(loaded.child_thread("a"), Some("th::child-a"));
}
