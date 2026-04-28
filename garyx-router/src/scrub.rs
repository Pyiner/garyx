//! One-shot migration: scrub legacy team-chat fossils from on-disk
//! `ThreadRecord`s.
//!
//! Baseline `5d9be37` thread records may still carry fields from the prior
//! team-chat iteration (`TeamRun` / `team_chat_messages[]` / etc.). This
//! module provides pure helpers that strip those fields and merge any
//! orphaned chat turns back into the canonical `messages[]` list, so no
//! content is lost and no downstream code needs to know the fossils ever
//! existed.

use std::path::Path;

use serde_json::Value;
use tracing::info;

/// Every legacy field that must be stripped from a thread record. Some of
/// these are deleted outright; `team_chat_messages` is merged into
/// `messages[]` first (see [`scrub_legacy_team_fields`]).
const LEGACY_TEAM_FIELDS: &[&str] = &[
    "team_run_id",
    "team_chat_mode",
    "team_chat_messages",
    "is_team_chat",
    "leader_agent_id",
    "member_agent_id",
    "leader_thread_id",
    "member_thread_ids",
    "team_members",
    "is_group_chat_team_thread",
];

/// Strip every legacy team-chat field from `doc` in-place.
///
/// Returns `true` if the document was mutated (and therefore should be
/// re-persisted by the caller). Returns `false` on a clean document,
/// making the function idempotent: feeding the same already-scrubbed
/// document in a second time is a no-op.
///
/// Merge semantics for `team_chat_messages`:
/// - Entries are appended to the end of `messages[]` in their existing
///   insertion order, so they retain their original chronological position
///   relative to each other.
/// - If `messages` is missing or not an array, it is created as a fresh
///   array containing the legacy turns.
/// - If `messages` already exists as an array, the legacy turns are
///   appended after any existing entries.
/// - An empty or missing `team_chat_messages` contributes no entries but
///   the key itself is still removed if present.
///
/// Emits a single `info`-level log line per thread whose id is discoverable
/// from `doc["thread_id"]` (falls back to `doc["id"]`). Threads without
/// either key still get scrubbed, just without an id in the log line.
pub fn scrub_legacy_team_fields(doc: &mut Value) -> bool {
    let Some(obj) = doc.as_object_mut() else {
        return false;
    };

    // Fast path: nothing to do.
    if !LEGACY_TEAM_FIELDS.iter().any(|f| obj.contains_key(*f)) {
        return false;
    }

    // Pull out legacy messages first so we can merge them before the
    // bulk delete pass.
    let legacy_messages = obj.remove("team_chat_messages");

    if let Some(legacy) = legacy_messages {
        if let Value::Array(legacy_arr) = legacy {
            if !legacy_arr.is_empty() {
                match obj.get_mut("messages") {
                    Some(Value::Array(existing)) => {
                        existing.extend(legacy_arr);
                    }
                    _ => {
                        obj.insert("messages".to_owned(), Value::Array(legacy_arr));
                    }
                }
            }
        }
        // Non-array / empty legacy `team_chat_messages` => just drop it;
        // the remove() above has already done so.
    }

    for field in LEGACY_TEAM_FIELDS {
        if *field == "team_chat_messages" {
            // Already handled above.
            continue;
        }
        obj.remove(*field);
    }

    // Discover a reasonable id for the log line; don't fail the scrub
    // just because the doc is oddly shaped.
    let thread_id = obj
        .get("thread_id")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("id").and_then(|v| v.as_str()))
        .unwrap_or("<unknown>");
    info!(thread_id, "scrubbed legacy team fields from thread");

    true
}

/// Best-effort removal of the legacy `~/.garyx/team_runs/` directory.
///
/// Nothing in the new regime reads `TeamRun` state, so the entire
/// directory is removed. Missing directories are not an error; any I/O
/// failure is logged and swallowed so a dirty disk can't block startup.
///
/// This helper deliberately does NOT run on its own — the orchestrator
/// wires the call site in phase 1.
pub fn cleanup_legacy_team_runs_dir(data_dir: &Path) {
    let target = data_dir.join("team_runs");
    match std::fs::remove_dir_all(&target) {
        Ok(()) => {
            info!(path = %target.display(), "removed legacy team_runs directory");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Nothing to do — the common path once the migration has run.
        }
        Err(e) => {
            tracing::warn!(
                path = %target.display(),
                error = %e,
                "failed to remove legacy team_runs directory"
            );
        }
    }
}

#[cfg(test)]
mod scrub_tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn clean_thread_is_noop() {
        let mut doc = json!({
            "thread_id": "th::clean",
            "agent_id": "planner",
            "messages": [ {"role": "user", "content": "hi"} ],
        });
        let before = doc.clone();
        let mutated = scrub_legacy_team_fields(&mut doc);
        assert!(!mutated);
        assert_eq!(doc, before);
    }

    #[test]
    fn team_run_id_only_is_removed() {
        let mut doc = json!({
            "thread_id": "th::only-run-id",
            "team_run_id": "run-123",
            "messages": [],
        });
        let mutated = scrub_legacy_team_fields(&mut doc);
        assert!(mutated);
        let obj = doc.as_object().unwrap();
        assert!(!obj.contains_key("team_run_id"));
        assert!(obj.contains_key("messages"));
    }

    #[test]
    fn team_chat_messages_appended_after_existing_messages() {
        let mut doc = json!({
            "thread_id": "th::merge",
            "messages": [
                {"role": "user",      "content": "x"},
                {"role": "assistant", "content": "y"},
            ],
            "team_chat_messages": [
                {"role": "user",      "content": "a"},
                {"role": "assistant", "content": "b"},
                {"role": "user",      "content": "c"},
            ],
        });
        let mutated = scrub_legacy_team_fields(&mut doc);
        assert!(mutated);

        let obj = doc.as_object().unwrap();
        assert!(!obj.contains_key("team_chat_messages"));
        let msgs = obj.get("messages").and_then(|v| v.as_array()).unwrap();
        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[0]["content"], "x");
        assert_eq!(msgs[1]["content"], "y");
        assert_eq!(msgs[2]["content"], "a");
        assert_eq!(msgs[3]["content"], "b");
        assert_eq!(msgs[4]["content"], "c");
    }

    #[test]
    fn team_chat_messages_creates_messages_when_missing() {
        let mut doc = json!({
            "thread_id": "th::no-messages",
            "team_chat_messages": [
                {"role": "user",      "content": "a"},
                {"role": "assistant", "content": "b"},
            ],
        });
        let mutated = scrub_legacy_team_fields(&mut doc);
        assert!(mutated);

        let obj = doc.as_object().unwrap();
        assert!(!obj.contains_key("team_chat_messages"));
        let msgs = obj.get("messages").and_then(|v| v.as_array()).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["content"], "a");
        assert_eq!(msgs[1]["content"], "b");
    }

    #[test]
    fn idempotent_second_pass_is_noop() {
        let mut doc = json!({
            "thread_id": "th::idempotent",
            "team_run_id": "run-xyz",
            "team_chat_messages": [ {"role": "user", "content": "hi"} ],
            "messages": [],
        });
        assert!(scrub_legacy_team_fields(&mut doc));
        // Snapshot the post-scrub state and re-run.
        let snapshot = doc.clone();
        assert!(!scrub_legacy_team_fields(&mut doc));
        assert_eq!(doc, snapshot);
    }

    #[test]
    fn all_legacy_fields_are_removed() {
        let mut doc = json!({
            "thread_id": "th::all",
            "agent_id": "planner",
            "workspace_path": "/proj/foo",
            "messages": [ {"role": "user", "content": "keep-me"} ],

            // Every field in LEGACY_TEAM_FIELDS.
            "team_run_id": "run-1",
            "team_chat_mode": "leader-follower",
            "team_chat_messages": [ {"role": "user", "content": "legacy"} ],
            "is_team_chat": true,
            "leader_agent_id": "leader",
            "member_agent_id": "member",
            "leader_thread_id": "th::leader",
            "member_thread_ids": ["th::m1", "th::m2"],
            "team_members": [{"agent_id": "coder"}],
            "is_group_chat_team_thread": true,
        });
        let mutated = scrub_legacy_team_fields(&mut doc);
        assert!(mutated);

        let obj = doc.as_object().unwrap();
        for field in LEGACY_TEAM_FIELDS {
            assert!(
                !obj.contains_key(*field),
                "field {field} should have been removed"
            );
        }

        // Non-legacy fields are preserved.
        assert_eq!(obj["agent_id"], "planner");
        assert_eq!(obj["workspace_path"], "/proj/foo");

        // Legacy messages were merged onto the end.
        let msgs = obj.get("messages").and_then(|v| v.as_array()).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["content"], "keep-me");
        assert_eq!(msgs[1]["content"], "legacy");
    }

    #[test]
    fn empty_team_chat_messages_is_dropped_without_touching_messages() {
        let mut doc = json!({
            "thread_id": "th::empty-legacy",
            "messages": [ {"role": "user", "content": "x"} ],
            "team_chat_messages": [],
        });
        let mutated = scrub_legacy_team_fields(&mut doc);
        assert!(mutated);
        let obj = doc.as_object().unwrap();
        assert!(!obj.contains_key("team_chat_messages"));
        let msgs = obj.get("messages").and_then(|v| v.as_array()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"], "x");
    }

    #[test]
    fn non_object_doc_is_noop() {
        let mut doc = json!([1, 2, 3]);
        let mutated = scrub_legacy_team_fields(&mut doc);
        assert!(!mutated);
        assert_eq!(doc, json!([1, 2, 3]));
    }

    #[test]
    fn cleanup_legacy_team_runs_dir_removes_directory() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let runs = data_dir.join("team_runs");
        std::fs::create_dir_all(&runs).unwrap();
        std::fs::write(runs.join("a.json"), br#"{"run_id": "a"}"#).unwrap();
        std::fs::write(runs.join("b.json"), br#"{"run_id": "b"}"#).unwrap();
        assert!(runs.exists());

        cleanup_legacy_team_runs_dir(data_dir);

        assert!(!runs.exists(), "team_runs directory should be gone");
    }

    #[test]
    fn cleanup_legacy_team_runs_dir_missing_is_ok() {
        let tmp = TempDir::new().unwrap();
        // No team_runs subdir — should still be a no-op, not a panic.
        cleanup_legacy_team_runs_dir(tmp.path());
    }
}
