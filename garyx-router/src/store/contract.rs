//! Executable contract for [`ThreadStore`] backends.
//!
//! Two core invariants — tombstone write rejection and the protected
//! `channel_bindings` field on `set` — must run inside each backend's own
//! write lock or transaction, so the trait cannot make them structural for
//! every implementation. Instead the crate that owns the trait publishes
//! the contract as runnable assertions: every backend and every delegating
//! wrapper must call [`run_thread_store_contract`] and
//! [`run_patch_and_protected_field_contract`] from its test suite
//! (#TASK-1864 batch 2 established the suite; this module is its
//! trait-crate home).
//!
//! The functions panic on the first violation, so they slot directly into
//! `#[tokio::test]` bodies.

use std::collections::BTreeSet;

use serde_json::{Map, json};

use super::{
    ThreadPatchResult, ThreadRecordPatch, ThreadStore, ThreadStoreError, ThreadTerminalState,
};
use crate::AtomicRecordMerge;

fn label_patch(value: &str) -> ThreadRecordPatch {
    let mut fields = Map::new();
    fields.insert("label".to_owned(), json!(value));
    ThreadRecordPatch::new(fields, BTreeSet::new()).expect("label-only patch is valid")
}

/// Point operations, key listing/counting, and the terminal-tombstone
/// write fence.
pub async fn run_thread_store_contract(store: &dyn ThreadStore) {
    // Missing key: absent, not an error.
    assert_eq!(store.get("thread::missing").await.expect("get"), None);
    assert!(!store.exists("thread::missing").await.expect("exists"));
    assert!(!store.delete("thread::missing").await.expect("delete"));
    assert!(
        matches!(
            store.patch("thread::missing", label_patch("x")).await,
            Err(ThreadStoreError::NotFound(_))
        ),
        "patch of a missing thread must be NotFound"
    );

    // Round trip.
    store
        .set(
            "thread::alpha",
            json!({"thread_id": "thread::alpha", "label": "first"}),
        )
        .await
        .expect("set");
    let read = store
        .get("thread::alpha")
        .await
        .expect("get")
        .expect("read back");
    assert_eq!(read["label"], "first");
    assert!(store.exists("thread::alpha").await.expect("exists"));

    // Overwrite replaces the whole value.
    store
        .set(
            "thread::alpha",
            json!({"thread_id": "thread::alpha", "generation": 2}),
        )
        .await
        .expect("set v2");
    let read = store
        .get("thread::alpha")
        .await
        .expect("get")
        .expect("read v2");
    assert_eq!(read["generation"], 2);
    assert!(read.get("label").is_none(), "set is a full replace");

    // Patch merges allowlisted top-level fields and preserves the rest.
    let observed = read;
    let mut desired = observed.clone();
    desired["label"] = json!("merged");
    let patch =
        ThreadRecordPatch::from_diff(&observed, &desired, &["label"]).expect("diff patch builds");
    assert_eq!(
        store.patch("thread::alpha", patch).await.expect("patch"),
        ThreadPatchResult::Applied
    );
    let read = store
        .get("thread::alpha")
        .await
        .expect("get")
        .expect("read merged");
    assert_eq!(read["generation"], 2);
    assert_eq!(read["label"], "merged");

    // Non-thread keys are ordinary records.
    store
        .set("meta::known_channel_endpoints", json!({"endpoints": []}))
        .await
        .expect("set registry");
    store
        .set("cron::job-1", json!({"schedule": "daily"}))
        .await
        .expect("set cron");

    // list_keys / count_keys: all + prefix.
    let mut all = store.list_keys(None).await.expect("list");
    all.sort();
    assert_eq!(
        all,
        vec![
            "cron::job-1".to_owned(),
            "meta::known_channel_endpoints".to_owned(),
            "thread::alpha".to_owned(),
        ]
    );
    let mut threads = store
        .list_keys(Some("thread::"))
        .await
        .expect("list prefix");
    threads.sort();
    assert_eq!(threads, vec!["thread::alpha".to_owned()]);
    assert_eq!(store.count_keys(None).await.expect("count"), 3);
    assert_eq!(
        store
            .count_keys(Some("thread::"))
            .await
            .expect("count prefix"),
        1
    );

    // Delete records the durable tombstone; every write shape is fenced.
    assert!(store.delete("thread::alpha").await.expect("delete"));
    assert!(!store.delete("thread::alpha").await.expect("re-delete"));
    assert_eq!(store.get("thread::alpha").await.expect("get"), None);
    assert!(!store.exists("thread::alpha").await.expect("exists"));
    assert_eq!(
        store
            .terminal_state("thread::alpha")
            .await
            .expect("terminal state"),
        Some(ThreadTerminalState::Deleted)
    );
    assert!(matches!(
        store
            .set("thread::alpha", json!({"thread_id": "thread::alpha"}))
            .await,
        Err(ThreadStoreError::Archived(_))
    ));
    assert!(matches!(
        store
            .patch("thread::alpha", label_patch("resurrected"))
            .await,
        Err(ThreadStoreError::Archived(_))
    ));
    assert!(matches!(
        store
            .update_many_atomic(vec![AtomicRecordMerge {
                thread_id: "thread::alpha".to_owned(),
                fields: json!({"label": "resurrected"}),
                create_if_missing: false,
            }])
            .await,
        Err(ThreadStoreError::Archived(_))
    ));
}

/// Patch witness semantics and the protected `channel_bindings` field.
pub async fn run_patch_and_protected_field_contract(store: &dyn ThreadStore) {
    let thread_id = "thread::patch-contract";
    let binding = json!({
        "channel": "telegram",
        "account_id": "main",
        "binding_key": "1000000001",
        "chat_id": "1000000001",
        "last_delivery_at": "2026-07-20T00:00:00Z"
    });
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "before",
                "history": {"message_count": 1},
                "channel_bindings": [binding]
            }),
        )
        .await
        .expect("seed record");

    let observed = store.get(thread_id).await.expect("get").expect("seeded");
    let mut desired = observed.clone();
    desired["label"] = json!("after");
    let patch =
        ThreadRecordPatch::from_diff(&observed, &desired, &["label"]).expect("diff patch builds");
    assert_eq!(
        store.patch(thread_id, patch).await.expect("patch"),
        ThreadPatchResult::Applied
    );
    let patched = store.get(thread_id).await.expect("get").expect("patched");
    assert_eq!(patched["label"], "after");
    assert_eq!(patched["history"]["message_count"], 1);
    assert_eq!(patched["channel_bindings"], observed["channel_bindings"]);

    let unchanged =
        ThreadRecordPatch::from_diff(&patched, &patched, &["label"]).expect("empty diff builds");
    assert_eq!(
        store.patch(thread_id, unchanged).await.expect("patch"),
        ThreadPatchResult::Unchanged
    );

    // A full replace that moves binding metadata is rejected under the
    // backend's own write guard.
    let mut changed_binding = patched.clone();
    changed_binding["channel_bindings"][0]["last_delivery_at"] = json!("2026-07-20T00:01:00Z");
    assert!(matches!(
        store.set(thread_id, changed_binding).await,
        Err(ThreadStoreError::ProtectedFieldConflict { .. })
    ));

    // The patch witness refuses protected fields at construction, so no
    // unvalidated merge shape can reach storage at all.
    let mut illegal_patch = Map::new();
    illegal_patch.insert("channel_bindings".to_owned(), json!([]));
    assert!(matches!(
        ThreadRecordPatch::new(illegal_patch, BTreeSet::new()),
        Err(ThreadStoreError::InvalidPatch(_))
    ));

    let mut unexpected = patched.clone();
    unexpected["history"]["message_count"] = json!(2);
    assert!(matches!(
        ThreadRecordPatch::from_diff(&patched, &unexpected, &["label"]),
        Err(ThreadStoreError::InvalidPatch(_))
    ));
}
