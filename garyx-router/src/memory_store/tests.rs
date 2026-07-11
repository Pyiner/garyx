use super::*;
use serde_json::json;

#[tokio::test]
async fn test_basic_crud() {
    let store = InMemoryThreadStore::new();

    // Initially empty.
    assert_eq!(store.size().await, 0);
    assert!(!store.exists("k1").await.unwrap());
    assert_eq!(store.get("k1").await.unwrap(), None);

    // Set and get.
    store.set("k1", json!({"hello": "world"})).await.unwrap();
    assert!(store.exists("k1").await.unwrap());
    assert_eq!(store.size().await, 1);
    let v = store.get("k1").await.unwrap().unwrap();
    assert_eq!(v["hello"], "world");

    // Update.
    store.update("k1", json!({"foo": "bar"})).await.unwrap();
    let v = store.get("k1").await.unwrap().unwrap();
    assert_eq!(v["hello"], "world");
    assert_eq!(v["foo"], "bar");

    // Delete.
    assert!(store.delete("k1").await.unwrap());
    assert!(!store.delete("k1").await.unwrap());
    assert_eq!(store.size().await, 0);
}

#[tokio::test]
async fn test_list_keys_with_prefix() {
    let store = InMemoryThreadStore::new();
    store.set("agent1::main::u1", json!({})).await.unwrap();
    store.set("agent1::main::u2", json!({})).await.unwrap();
    store.set("agent2::main::u1", json!({})).await.unwrap();

    let all = store.list_keys(None).await.unwrap();
    assert_eq!(all.len(), 3);

    let mut filtered = store.list_keys(Some("agent1::")).await.unwrap();
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
    store.set("a", json!(1)).await.unwrap();
    store.set("b", json!(2)).await.unwrap();
    assert_eq!(store.size().await, 2);
    store.clear().await;
    assert_eq!(store.size().await, 0);
}

/// The trait default for `update_many_atomic` must REFUSE before writing
/// anything (#TASK-2099 root final review): an API named atomic must never
/// partially commit, so a backend without a transactional implementation
/// gets an explicit unsupported error and zero writes — never a sequential
/// fallback that can stop halfway.
#[tokio::test]
async fn default_update_many_atomic_refuses_with_zero_writes() {
    /// Delegates reads/writes to an in-memory store but deliberately does
    /// NOT override `update_many_atomic`, exercising the trait default.
    struct NonAtomicStore {
        inner: InMemoryThreadStore,
    }

    #[async_trait::async_trait]
    impl ThreadStore for NonAtomicStore {
        async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
            self.inner.get(thread_id).await
        }
        async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
            self.inner.set(thread_id, data).await
        }
        async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.delete(thread_id).await
        }
        async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
            self.inner.list_keys(prefix).await
        }
        async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.exists(thread_id).await
        }
        async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
            self.inner.update(thread_id, updates).await
        }
    }

    let store = NonAtomicStore {
        inner: InMemoryThreadStore::new(),
    };
    store
        .set("thread::first", json!({"state": "before"}))
        .await
        .unwrap();
    store
        .set("thread::second", json!({"state": "before"}))
        .await
        .unwrap();

    let error = store
        .update_many_atomic(vec![
            crate::AtomicRecordMerge {
                thread_id: "thread::first".to_owned(),
                fields: json!({"state": "after"}),
                create_if_missing: false,
            },
            crate::AtomicRecordMerge {
                thread_id: "thread::second".to_owned(),
                fields: json!({"state": "after"}),
                create_if_missing: false,
            },
        ])
        .await
        .expect_err("the non-transactional default must refuse");
    assert!(
        matches!(error, ThreadStoreError::Backend(ref message)
            if message.contains("atomic multi-record")),
        "unexpected error: {error}"
    );

    for thread_id in ["thread::first", "thread::second"] {
        assert_eq!(
            store.get(thread_id).await.unwrap().unwrap()["state"],
            "before",
            "the refused mutation must not have written {thread_id}"
        );
    }
}
