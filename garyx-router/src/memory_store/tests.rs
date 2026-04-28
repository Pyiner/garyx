use super::*;
use serde_json::json;

#[tokio::test]
async fn test_basic_crud() {
    let store = InMemoryThreadStore::new();

    // Initially empty.
    assert_eq!(store.size().await, 0);
    assert!(!store.exists("k1").await);
    assert_eq!(store.get("k1").await, None);

    // Set and get.
    store.set("k1", json!({"hello": "world"})).await;
    assert!(store.exists("k1").await);
    assert_eq!(store.size().await, 1);
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
    assert_eq!(store.size().await, 0);
}

#[tokio::test]
async fn test_list_keys_with_prefix() {
    let store = InMemoryThreadStore::new();
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
    let store = InMemoryThreadStore::new();
    let result = store.update("missing", json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_clear() {
    let store = InMemoryThreadStore::new();
    store.set("a", json!(1)).await;
    store.set("b", json!(2)).await;
    assert_eq!(store.size().await, 2);
    store.clear().await;
    assert_eq!(store.size().await, 0);
}
