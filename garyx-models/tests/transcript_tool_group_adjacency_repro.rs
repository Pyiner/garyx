//! TASK-1021 — "two adjacent collapsed tool rows" investigation (server side).
//!
//! Symptom: an iOS transcript renders two collapsed tool rows back to back
//! (`[Used fileChange][Used fileChange]`) where an interstitial assistant
//! message should sit between them, and that assistant message vanishes.
//!
//! These tests pin down what the *server* reducer
//! (`garyx_models::transcript_render_state`) does, so we can decide whether the
//! bug lives in the reducer / live-frame derivation or further downstream
//! (the client frame application). They are pure, deterministic, no-UI tests.
//!
//! Synthetic structure mirrors the real failing sequence (assistant text,
//! tool group A, assistant boundary, interstitial assistant text, tool group B,
//! assistant boundary, assistant text, tool group C) without any real data.
//!
//! Findings encoded here:
//!   * `cold_reduce_*`            — full reduce keeps the interstitial assistant
//!     between the two tool groups; the groups are NEVER adjacent. (reducer OK)
//!   * `live_incremental_reduce_*`— reducing every growing prefix (the live
//!     per-frame model) also never produces adjacent tool groups, because the
//!     interstitial assistant always commits WITH text before its tool_use.
//!   * `empty_streaming_*`        — the only way to make the reducer collapse the
//!     two groups is to feed it an *empty streaming* interstitial assistant at
//!     the same frame the second tool_use exists. The reducer then filters the
//!     empty placeholder and the groups go adjacent — but this input is
//!     unreachable from the bridge (see doc), and a backfilled frame self-heals.

use garyx_models::transcript_render_state::{
    RenderActivityRow, RenderPlaceholderFilterReason, RenderRow, RenderStepItem,
    reduce_transcript_render_state,
};
use serde_json::{Value, json};

// ---------- record builders (synthetic; no real data) ----------

fn message_record(seq: u64, message: Value) -> Value {
    json!({
        "seq": seq,
        "thread_id": "thread::test",
        "run_id": "run::test",
        "timestamp": "2026-01-01T00:00:00Z",
        "message": message,
    })
}

fn control_record(seq: u64, kind: &str) -> Value {
    message_record(
        seq,
        json!({
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": { "kind": kind, "at": "2026-01-01T00:00:00Z" },
        }),
    )
}

fn user_record(seq: u64, text: &str) -> Value {
    message_record(
        seq,
        json!({
            "role": "user",
            "text": text,
            "timestamp": "2026-01-01T00:00:01Z",
            "metadata": { "origin_id": "00000000-0000-0000-0000-000000000001" },
        }),
    )
}

fn assistant_record(seq: u64, text: &str) -> Value {
    message_record(
        seq,
        json!({ "role": "assistant", "text": text, "timestamp": "2026-01-01T00:00:02Z" }),
    )
}

/// An assistant segment that is mid-stream: empty body + a streaming flag. This
/// is the *hypothetical* placeholder the original theory assumed the bridge
/// commits. See `empty_streaming_*` tests for why it is not actually reachable.
fn assistant_streaming_placeholder(seq: u64) -> Value {
    message_record(
        seq,
        json!({ "role": "assistant", "text": "", "streaming": true, "timestamp": "2026-01-01T00:00:02Z" }),
    )
}

fn tool_use_record(seq: u64, call_id: &str) -> Value {
    message_record(
        seq,
        json!({
            "role": "tool_use",
            "tool_use_id": call_id,
            "content": { "type": "commandExecution", "id": call_id, "tool": "fileChange" },
            "timestamp": "2026-01-01T00:00:03Z",
        }),
    )
}

fn tool_result_record(seq: u64, call_id: &str) -> Value {
    message_record(
        seq,
        json!({
            "role": "tool_result",
            "tool_use_id": call_id,
            "content": { "type": "tool_result", "id": call_id, "result": { "stdout": "ok" } },
            "timestamp": "2026-01-01T00:00:04Z",
        }),
    )
}

/// The full real-shaped sequence: the interstitial assistant (seq 7, the analog
/// of real seq 419) always carries text, exactly as it lands on disk.
fn real_shaped_records() -> Vec<Value> {
    vec![
        control_record(1, "run_start"),
        user_record(2, "Implement the projection."),
        assistant_record(3, "Segment one — adding the model."), // analog of 415
        tool_use_record(4, "call_a"),                           // group A
        tool_result_record(5, "call_a"),
        control_record(6, "assistant_boundary"), // analog of 418
        assistant_record(7, "Segment two — resolving destinations."), // analog of 419 (vanishes on iOS)
        tool_use_record(8, "call_b"),                                 // group B
        tool_result_record(9, "call_b"),
        control_record(10, "assistant_boundary"), // analog of 422
        assistant_record(11, "Segment three — wiring the panel."), // analog of 423
        tool_use_record(12, "call_c"),            // group C
        tool_result_record(13, "call_c"),
    ]
}

// ---------- snapshot inspection helpers ----------

/// Ordered step-item tags of the single user turn's single step.
/// "A" = assistant message step, "T:<id>" = tool group.
fn step_item_tags(records: &[Value]) -> Vec<String> {
    let snapshot = reduce_transcript_render_state(records.iter());
    let mut tags = Vec::new();
    for row in &snapshot.rows {
        let RenderRow::UserTurn(turn) = row;
        for activity in &turn.activity {
            match activity {
                RenderActivityRow::AssistantReply(reply) => {
                    tags.push(format!("A:{}", reply.message.seq));
                }
                RenderActivityRow::Step(step) => {
                    for item in &step.steps {
                        match item {
                            RenderStepItem::AssistantMessage(a) => {
                                tags.push(format!("A:{}", a.message.seq));
                            }
                            RenderStepItem::ToolGroup(g) => tags.push(format!("T:{}", g.id)),
                        }
                    }
                    if let Some(final_msg) = &step.final_message {
                        tags.push(format!("A:{}", final_msg.seq));
                    }
                }
            }
        }
    }
    tags
}

fn has_adjacent_tool_groups(tags: &[String]) -> bool {
    tags.windows(2)
        .any(|w| w[0].starts_with("T:") && w[1].starts_with("T:"))
}

fn assistant_seqs_present(tags: &[String]) -> Vec<u64> {
    tags.iter()
        .filter_map(|t| t.strip_prefix("A:").and_then(|s| s.parse::<u64>().ok()))
        .collect()
}

// ---------- 1. cold full reduce is correct ----------

#[test]
fn cold_reduce_keeps_interstitial_assistant_between_tool_groups() {
    let tags = step_item_tags(&real_shaped_records());

    // The interstitial assistant (seq 7, analog of 419) is rendered.
    assert!(
        assistant_seqs_present(&tags).contains(&7),
        "interstitial assistant seq 7 must be present in cold reduce; got {tags:?}"
    );
    // The two tool groups are never adjacent in the committed ledger.
    assert!(
        !has_adjacent_tool_groups(&tags),
        "cold reduce must not put two tool groups adjacent; got {tags:?}"
    );
}

// ---------- 2. live per-frame reduce is correct ----------

#[test]
fn live_incremental_reduce_never_swallows_interstitial_assistant() {
    let records = real_shaped_records();
    // Model the gateway's live frame derivation: reduce every growing prefix
    // (each committed record produces a frame derived from records <= its seq).
    for len in 1..=records.len() {
        let tags = step_item_tags(&records[..len]);
        assert!(
            !has_adjacent_tool_groups(&tags),
            "frame at prefix len={len} produced adjacent tool groups; got {tags:?}"
        );
    }

    // And the final frame equals the cold snapshot (no residual desync).
    assert_eq!(
        step_item_tags(&records),
        step_item_tags(&records[..records.len()]),
    );
}

// ---------- 3. the refuted hypothesis: empty-streaming placeholder ----------

#[test]
fn empty_streaming_interstitial_is_the_only_reducer_path_to_adjacency() {
    // Hypothesis under test: 419 is briefly an EMPTY streaming placeholder at the
    // same frame tool group B exists. Build exactly that frame.
    let collapsed = vec![
        control_record(1, "run_start"),
        user_record(2, "Implement the projection."),
        assistant_record(3, "Segment one."),
        tool_use_record(4, "call_a"),
        tool_result_record(5, "call_a"),
        control_record(6, "assistant_boundary"),
        assistant_streaming_placeholder(7), // <-- empty + streaming: filtered
        tool_use_record(8, "call_b"),
        tool_result_record(9, "call_b"),
    ];
    let snapshot = reduce_transcript_render_state(collapsed.iter());

    // The empty placeholder is filtered out...
    let filtered: Vec<u64> = snapshot
        .filtered_placeholders
        .iter()
        .filter(|p| p.reason == RenderPlaceholderFilterReason::EmptyStreamingAssistant)
        .map(|p| p.message.seq)
        .collect();
    assert_eq!(
        filtered,
        vec![7],
        "empty streaming interstitial must be filtered; got {filtered:?}"
    );

    // ...and that makes the two tool groups adjacent — the symptom, server side.
    let tags = step_item_tags(&collapsed);
    assert!(
        has_adjacent_tool_groups(&tags),
        "empty streaming interstitial should collapse the groups; got {tags:?}"
    );
    assert!(
        !assistant_seqs_present(&tags).contains(&7),
        "the filtered interstitial must be absent; got {tags:?}"
    );
}

#[test]
fn backfilling_the_interstitial_self_heals_the_next_frame() {
    // Same frame as above but the interstitial has now received its text and
    // dropped the streaming flag (the committed state). A re-reduce (the next
    // live frame / cold replay) restores the separation: the reducer cannot
    // persist the collapse once the body lands.
    let healed = vec![
        control_record(1, "run_start"),
        user_record(2, "Implement the projection."),
        assistant_record(3, "Segment one."),
        tool_use_record(4, "call_a"),
        tool_result_record(5, "call_a"),
        control_record(6, "assistant_boundary"),
        assistant_record(7, "Segment two — resolving destinations."), // backfilled
        tool_use_record(8, "call_b"),
        tool_result_record(9, "call_b"),
    ];
    let tags = step_item_tags(&healed);
    assert!(
        assistant_seqs_present(&tags).contains(&7),
        "backfilled interstitial must reappear; got {tags:?}"
    );
    assert!(
        !has_adjacent_tool_groups(&tags),
        "backfilled frame must separate the groups again; got {tags:?}"
    );
}
