//! Group transcript snapshots for agent-team prompts.
//!
//! When a team-bound thread dispatches a run, the member agent's prompt
//! carries a `group_transcript_snapshot`: a compact `[{agent_id, text, at}]`
//! view of the recent conversation. This module is the single
//! implementation (#TASK-1864 batch 1 deduplicated the former
//! router/gateway copies) and reads from the committed transcript tail —
//! the only session source since Batch 2's import backfilled every
//! pre-transcript thread.

use serde_json::{Value, json};
use tracing::warn;

use crate::thread_history::ThreadHistoryRepository;

/// Window for the group snapshot, matching the legacy `messages` snapshot
/// cap (bridge `MAX_SESSION_MESSAGES`) it replaces.
pub const GROUP_TRANSCRIPT_SNAPSHOT_LIMIT: usize = 100;

fn message_actor_label(object: &serde_json::Map<String, Value>) -> Option<String> {
    let metadata = object.get("metadata").and_then(Value::as_object);
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();

    let agent_display_name = metadata
        .and_then(|fields| fields.get("agent_display_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let agent_id = metadata
        .and_then(|fields| fields.get("agent_id"))
        .and_then(Value::as_str)
        .or_else(|| object.get("agent_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let from_id = metadata
        .and_then(|fields| fields.get("from_id"))
        .and_then(Value::as_str)
        .or_else(|| object.get("from_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let internal_dispatch = metadata
        .and_then(|fields| fields.get("internal_dispatch"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    match role {
        "assistant" => agent_id.or(agent_display_name),
        "user" if internal_dispatch => agent_id.or(agent_display_name).or(from_id),
        "user" => Some("user".to_owned()),
        _ => agent_id.or(agent_display_name).or(from_id),
    }
}

/// Map ordered provider-session messages into `[{agent_id, text, at}]`
/// snapshot entries.
pub fn group_transcript_snapshot_from_messages<'a>(
    messages: impl IntoIterator<Item = &'a Value>,
) -> Value {
    let mut entries = Vec::new();
    for message in messages {
        let Some(object) = message.as_object() else {
            continue;
        };
        let agent_id = message_actor_label(object).unwrap_or_default();
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| object.get("content").and_then(Value::as_str))
            .unwrap_or("");
        if agent_id.is_empty() && text.is_empty() {
            continue;
        }
        let at = object
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("");
        entries.push(json!({
            "agent_id": agent_id,
            "text": text,
            "at": at,
        }));
    }
    Value::Array(entries)
}

/// Legacy snapshot built from the thread record's `messages` array.
/// Fallback only: pre-transcript threads keep a `messages` snapshot but
/// have no transcript file (#TASK-1864 batch 1); deleted after Batch 2's
/// import backfills those transcripts.
pub fn build_group_transcript_snapshot(thread_data: &Value) -> Value {
    let Some(messages) = thread_data.get("messages").and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    group_transcript_snapshot_from_messages(messages.iter())
}

/// Group snapshot from the committed transcript tail (control records
/// skipped) — the only session source (#TASK-1864 closing batch).
pub async fn build_group_transcript_snapshot_from_history(
    history: &ThreadHistoryRepository,
    thread_id: &str,
) -> Value {
    match history
        .provider_session_tail(thread_id, GROUP_TRANSCRIPT_SNAPSHOT_LIMIT)
        .await
    {
        Ok(messages) => group_transcript_snapshot_from_messages(messages.iter()),
        Err(error) => {
            warn!(
                thread_id = %thread_id,
                error = %error,
                "failed to read transcript tail for group snapshot"
            );
            Value::Array(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::memory_store::InMemoryThreadStore;
    use crate::thread_history::ThreadTranscriptStore;

    fn fixture_thread_messages() -> Vec<Value> {
        vec![
            json!({
                "role": "user",
                "text": "@[Coder](coder) please help",
                "timestamp": "t0",
                "metadata": {
                    "agent_id": "team::demo",
                    "from_id": "alice"
                }
            }),
            json!({
                "role": "assistant",
                "text": "On it.",
                "timestamp": "t1",
                "metadata": {
                    "agent_id": "coder",
                    "agent_display_name": "Coder"
                }
            }),
            json!({
                "role": "user",
                "text": "@[Reviewer](reviewer) take a look",
                "timestamp": "t2",
                "metadata": {
                    "internal_dispatch": true,
                    "agent_id": "planner",
                    "agent_display_name": "Planner"
                }
            }),
        ]
    }

    fn expected_snapshot() -> Value {
        json!([
            {"agent_id": "user", "text": "@[Coder](coder) please help", "at": "t0"},
            {"agent_id": "coder", "text": "On it.", "at": "t1"},
            {"agent_id": "planner", "text": "@[Reviewer](reviewer) take a look", "at": "t2"}
        ])
    }

    #[test]
    fn group_transcript_snapshot_uses_user_label_for_human_turns() {
        let thread_data = json!({ "messages": fixture_thread_messages() });
        assert_eq!(build_group_transcript_snapshot(&thread_data), expected_snapshot());
    }

    #[tokio::test]
    async fn history_snapshot_matches_legacy_snapshot_for_same_content() {
        // Snapshot-vs-rebuild oracle: the same messages served through the
        // transcript produce exactly the legacy record snapshot, with an
        // interleaved control record skipped.
        let history = ThreadHistoryRepository::new(
            Arc::new(InMemoryThreadStore::new()),
            Arc::new(ThreadTranscriptStore::memory()),
        );
        let thread_id = "thread::group-snapshot";
        let mut transcript_rows = fixture_thread_messages();
        transcript_rows.insert(
            2,
            json!({
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": {"kind": "run_complete", "run_id": "run-1"},
            }),
        );
        history
            .transcript_store()
            .append_committed_messages(thread_id, Some("run-1"), &transcript_rows)
            .await
            .expect("append transcript");

        let snapshot =
            build_group_transcript_snapshot_from_history(&history, thread_id).await;
        assert_eq!(snapshot, expected_snapshot());
    }

}
