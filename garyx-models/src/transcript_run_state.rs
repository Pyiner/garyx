use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::transcript_kind::{
    is_tool_related_message, is_tool_result_trace, resolve_message_kind_for_object, tool_call_id,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRunActivity {
    #[default]
    Idle,
    Thinking,
    UsingTool,
    Reconciling,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptRewriteRange {
    pub notice_seq: Option<u64>,
    pub start_seq: u64,
    pub end_seq: u64,
}

/// Provider usage-quota / rate-limit context for a terminated run.
///
/// Surfaced when a run ended because the provider's rolling quota was
/// exhausted (e.g. Codex's 5-hour ChatGPT-plan window). Carries the
/// authoritative reset time so clients can render a live countdown, and a flag
/// indicating the gateway has scheduled an automatic resend at reset time.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptRateLimit {
    /// Provider identity, e.g. `codex`.
    pub provider: Option<String>,
    /// ISO 8601 timestamp when the exhausted window resets, when known.
    pub reset_at: Option<String>,
    /// Which rolling window was exhausted: `primary` (e.g. the 5-hour session)
    /// or `secondary` (e.g. the weekly allowance).
    pub window: Option<String>,
    /// Human-readable detail reported by the provider, when available.
    pub message: Option<String>,
    /// Whether the gateway scheduled an automatic resend at reset time.
    pub will_auto_resend: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptRunState {
    pub busy: bool,
    pub active_run_id: Option<String>,
    pub activity: TranscriptRunActivity,
    pub terminal_status: Option<String>,
    pub last_user_ack_seq: Option<u64>,
    pub last_user_ack_pending_input_id: Option<String>,
    pub title: Option<String>,
    pub rewrite_ranges: Vec<TranscriptRewriteRange>,
    pub last_transcript_reset_seq: Option<u64>,
    /// Set when the active run terminated due to provider quota exhaustion.
    /// Cleared whenever a fresh run starts or the run is interrupted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<TranscriptRateLimit>,
    #[serde(skip)]
    pending_tool_call_ids: BTreeMap<String, usize>,
    #[serde(skip)]
    pending_anonymous_tool_call_count: usize,
}

pub fn reduce_transcript_run_state<'a>(
    records: impl IntoIterator<Item = &'a Value>,
) -> TranscriptRunState {
    let mut state = TranscriptRunState::default();
    for record in records {
        apply_transcript_record(&mut state, record);
    }
    state
}

pub fn apply_transcript_record(state: &mut TranscriptRunState, record: &Value) {
    let seq = record.get("seq").and_then(Value::as_u64);
    let Some(message) = record.get("message").and_then(Value::as_object) else {
        return;
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .trim()
        .to_ascii_lowercase();
    let tool_related = is_tool_related_message(&role, message);
    let kind = resolve_message_kind_for_object(&role, message, tool_related);

    match kind {
        "control" => apply_control_record(state, seq, message.get("control")),
        "tool_trace" if state.busy && state.activity != TranscriptRunActivity::Reconciling => {
            apply_tool_trace_record(state, &role, message);
        }
        "assistant_reply" | "user_input"
            if state.busy && state.activity != TranscriptRunActivity::Reconciling =>
        {
            state.activity = activity_for_pending_tools(state);
        }
        _ => {}
    }
}

fn apply_control_record(state: &mut TranscriptRunState, seq: Option<u64>, control: Option<&Value>) {
    let Some(control) = control.and_then(Value::as_object) else {
        return;
    };
    let Some(kind) = control.get("kind").and_then(Value::as_str) else {
        return;
    };

    match kind {
        "run_start" => {
            state.busy = true;
            state.active_run_id = control_string(control.get("run_id"));
            state.terminal_status = None;
            state.rate_limit = None;
            clear_pending_tools(state);
            state.activity = TranscriptRunActivity::Thinking;
        }
        "user_ack" => {
            state.last_user_ack_seq = seq;
            state.last_user_ack_pending_input_id = control_string(control.get("pending_input_id"))
                .or_else(|| control_string(control.get("pendingInputId")));
        }
        "assistant_boundary"
            if state.busy && state.activity != TranscriptRunActivity::Reconciling =>
        {
            state.activity = activity_for_pending_tools(state);
        }
        "done" if state.busy => {
            clear_pending_tools(state);
            state.activity = TranscriptRunActivity::Reconciling;
        }
        "run_complete" => {
            state.busy = false;
            state.active_run_id = None;
            clear_pending_tools(state);
            state.activity = TranscriptRunActivity::Idle;
            state.terminal_status =
                control_string(control.get("status")).or_else(|| Some("completed".to_owned()));
            state.rate_limit = parse_rate_limit(control.get("rate_limit"));
        }
        "run_interrupted" | "interrupt_confirmed" => {
            state.busy = false;
            state.active_run_id = None;
            clear_pending_tools(state);
            state.activity = TranscriptRunActivity::Idle;
            state.terminal_status = Some("interrupted".to_owned());
            state.rate_limit = None;
        }
        "thread_title_updated" => {
            state.title = control_string(control.get("title"));
        }
        "transcript_reset" => {
            state.last_transcript_reset_seq = seq;
        }
        "range_rewrite" => {
            let start_seq = control_u64(control.get("start_seq")).or(seq).unwrap_or(0);
            let end_seq = control_u64(control.get("end_seq")).unwrap_or(start_seq);
            state.rewrite_ranges.push(TranscriptRewriteRange {
                notice_seq: seq,
                start_seq,
                end_seq,
            });
        }
        _ => {}
    }
}

fn parse_rate_limit(value: Option<&Value>) -> Option<TranscriptRateLimit> {
    let object = value.and_then(Value::as_object)?;
    Some(TranscriptRateLimit {
        provider: control_string(object.get("provider")),
        reset_at: control_string(object.get("reset_at"))
            .or_else(|| control_string(object.get("resetAt"))),
        window: control_string(object.get("window")),
        message: control_string(object.get("message")),
        will_auto_resend: object
            .get("will_auto_resend")
            .or_else(|| object.get("willAutoResend"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn control_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn control_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value.as_u64().or_else(|| {
            value
                .as_str()
                .and_then(|text| text.trim().parse::<u64>().ok())
        })
    })
}

fn apply_tool_trace_record(
    state: &mut TranscriptRunState,
    role: &str,
    message: &serde_json::Map<String, Value>,
) {
    if is_tool_result_trace(role, message) {
        mark_tool_result(state, tool_call_id(message));
    } else {
        mark_tool_use(state, tool_call_id(message));
    }
    state.activity = activity_for_pending_tools(state);
}

fn mark_tool_use(state: &mut TranscriptRunState, tool_call_id: Option<String>) {
    if let Some(tool_call_id) = tool_call_id {
        *state.pending_tool_call_ids.entry(tool_call_id).or_insert(0) += 1;
    } else {
        state.pending_anonymous_tool_call_count += 1;
    }
}

fn mark_tool_result(state: &mut TranscriptRunState, tool_call_id: Option<String>) {
    if let Some(tool_call_id) = tool_call_id {
        let _ = decrement_pending_tool_id(state, &tool_call_id);
        return;
    }
    if state.pending_anonymous_tool_call_count > 0 {
        state.pending_anonymous_tool_call_count -= 1;
    } else if state.pending_tool_call_ids.len() == 1 {
        state.pending_tool_call_ids.clear();
    }
}

fn decrement_pending_tool_id(state: &mut TranscriptRunState, tool_call_id: &str) -> bool {
    let Some(count) = state.pending_tool_call_ids.get_mut(tool_call_id) else {
        return false;
    };
    *count -= 1;
    if *count == 0 {
        state.pending_tool_call_ids.remove(tool_call_id);
    }
    true
}

fn activity_for_pending_tools(state: &TranscriptRunState) -> TranscriptRunActivity {
    if state.pending_anonymous_tool_call_count > 0 || !state.pending_tool_call_ids.is_empty() {
        TranscriptRunActivity::UsingTool
    } else {
        TranscriptRunActivity::Thinking
    }
}

fn clear_pending_tools(state: &mut TranscriptRunState) {
    state.pending_tool_call_ids.clear();
    state.pending_anonymous_tool_call_count = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, json};

    fn parse_jsonl(raw: &str) -> Vec<Value> {
        raw.lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                (!trimmed.is_empty()).then(|| serde_json::from_str(trimmed).unwrap())
            })
            .collect()
    }

    fn control_record(seq: u64, event: &Value) -> Value {
        let event_type = event.get("type").and_then(Value::as_str).unwrap();
        let thread_id = field_string(event, "thread_id")
            .or_else(|| field_string(event, "threadId"))
            .unwrap_or_else(|| "thread::fixture".to_owned());
        let run_id = field_string(event, "run_id")
            .or_else(|| field_string(event, "runId"))
            .unwrap_or_else(|| "run::fixture".to_owned());
        let mut control = Map::new();
        control.insert("kind".to_owned(), json!(event_type));
        control.insert("thread_id".to_owned(), json!(thread_id));
        control.insert("run_id".to_owned(), json!(run_id));
        control.insert("at".to_owned(), json!("2026-06-18T12:00:00Z"));
        if let Some(value) = event
            .get("pending_input_id")
            .or_else(|| event.get("pendingInputId"))
        {
            control.insert("pending_input_id".to_owned(), value.clone());
        }
        if let Some(value) = event.get("duration_ms").or_else(|| event.get("durationMs")) {
            control.insert("duration_ms".to_owned(), value.clone());
        }
        if let Some(value) = event.get("title") {
            control.insert("title".to_owned(), value.clone());
        }
        json!({
            "seq": seq,
            "thread_id": thread_id,
            "run_id": run_id,
            "timestamp": "2026-06-18T12:00:00Z",
            "message": {
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": Value::Object(control),
            }
        })
    }

    fn field_string(event: &Value, key: &str) -> Option<String> {
        event
            .get(key)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    #[test]
    fn lifecycle_fixture_replays_to_idle_terminal_state() {
        let events = parse_jsonl(include_str!(
            "../../test-fixtures/stream-sync/stream-lifecycle.jsonl"
        ));
        let mut next_seq = 1_u64;
        let mut records = Vec::new();
        for event in &events {
            match event.get("type").and_then(Value::as_str) {
                Some("committed_message") => records.push(event.clone()),
                Some("run_start" | "done" | "run_complete") => {
                    records.push(control_record(next_seq, event));
                    next_seq += 1;
                }
                _ => {}
            }
        }

        let state = reduce_transcript_run_state(&records);
        assert!(!state.busy);
        assert_eq!(state.activity, TranscriptRunActivity::Idle);
        assert_eq!(state.terminal_status.as_deref(), Some("completed"));
        assert_eq!(state.active_run_id, None);
    }

    #[test]
    fn user_ack_fixture_replays_ack_position_and_tool_activity() {
        let events = parse_jsonl(include_str!(
            "../../test-fixtures/stream-sync/stream-events-with-user-ack.jsonl"
        ));
        let records: Vec<Value> = events
            .into_iter()
            .filter(|event| {
                event
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|event_type| event_type == "committed_message")
            })
            .collect();

        let tool_state = reduce_transcript_run_state(
            records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 3),
        );
        assert!(tool_state.busy);
        assert_eq!(tool_state.activity, TranscriptRunActivity::UsingTool);

        let reconciling_state = reduce_transcript_run_state(
            records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 7),
        );
        assert!(reconciling_state.busy);
        assert_eq!(
            reconciling_state.activity,
            TranscriptRunActivity::Reconciling
        );
        assert_eq!(
            reconciling_state.last_user_ack_pending_input_id.as_deref(),
            Some("pending-fixture-followup")
        );
        assert!(reconciling_state.last_user_ack_seq.is_some());

        let state = reduce_transcript_run_state(&records);
        assert!(!state.busy);
        assert_eq!(state.activity, TranscriptRunActivity::Idle);
        assert_eq!(state.terminal_status.as_deref(), Some("completed"));
        assert_eq!(
            state.last_user_ack_pending_input_id.as_deref(),
            Some("pending-fixture-followup")
        );
        assert!(state.last_user_ack_seq.is_some());
    }

    #[test]
    fn multi_tool_lull_fixture_replays_finished_tool_gap_as_thinking() {
        let records = parse_jsonl(include_str!(
            "../../test-fixtures/stream-sync/multi-tool-lull.jsonl"
        ));
        let committed_records: Vec<Value> = records
            .into_iter()
            .filter(|event| {
                event
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|event_type| event_type == "committed_message")
            })
            .collect();

        let first_tool_lull = reduce_transcript_run_state(
            committed_records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 4),
        );
        assert!(first_tool_lull.busy);
        assert_eq!(first_tool_lull.activity, TranscriptRunActivity::Thinking);

        let second_tool_running = reduce_transcript_run_state(
            committed_records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 5),
        );
        assert!(second_tool_running.busy);
        assert_eq!(
            second_tool_running.activity,
            TranscriptRunActivity::UsingTool
        );

        let final_tool_lull = reduce_transcript_run_state(
            committed_records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 6),
        );
        assert!(final_tool_lull.busy);
        assert_eq!(final_tool_lull.activity, TranscriptRunActivity::Thinking);
    }

    #[test]
    fn parallel_tool_lull_fixture_waits_for_all_results_before_thinking() {
        let records = parse_jsonl(include_str!(
            "../../test-fixtures/stream-sync/parallel-tool-lull.jsonl"
        ));
        let committed_records: Vec<Value> = records
            .into_iter()
            .filter(|event| {
                event
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|event_type| event_type == "committed_message")
            })
            .collect();

        let both_tools_running = reduce_transcript_run_state(
            committed_records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 4),
        );
        assert!(both_tools_running.busy);
        assert_eq!(
            both_tools_running.activity,
            TranscriptRunActivity::UsingTool
        );

        let one_tool_still_running = reduce_transcript_run_state(
            committed_records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 5),
        );
        assert!(one_tool_still_running.busy);
        assert_eq!(
            one_tool_still_running.activity,
            TranscriptRunActivity::UsingTool
        );

        let all_tools_finished = reduce_transcript_run_state(
            committed_records
                .iter()
                .filter(|record| record.get("seq").and_then(Value::as_u64).unwrap_or(0) <= 6),
        );
        assert!(all_tools_finished.busy);
        assert_eq!(all_tools_finished.activity, TranscriptRunActivity::Thinking);
    }

    #[test]
    fn rewrite_controls_surface_replay_invalidation_windows() {
        let records = vec![
            control_record(
                1,
                &json!({
                    "type": "run_start",
                    "threadId": "thread::fixture-rewrite",
                    "runId": "run::fixture-rewrite",
                }),
            ),
            json!({
                "seq": 2,
                "thread_id": "thread::fixture-rewrite",
                "run_id": "run::fixture-rewrite",
                "timestamp": "2026-06-18T12:00:00Z",
                "message": {
                    "role": "system",
                    "kind": "control",
                    "internal": true,
                    "internal_kind": "control",
                    "control": {
                        "kind": "range_rewrite",
                        "start_seq": 1,
                        "end_seq": 1,
                    }
                }
            }),
            json!({
                "seq": 3,
                "thread_id": "thread::fixture-rewrite",
                "run_id": "run::fixture-rewrite",
                "timestamp": "2026-06-18T12:00:00Z",
                "message": {
                    "role": "system",
                    "kind": "control",
                    "internal": true,
                    "internal_kind": "control",
                    "control": {
                        "kind": "transcript_reset",
                    }
                }
            }),
        ];

        let state = reduce_transcript_run_state(&records);
        assert_eq!(
            state.rewrite_ranges,
            vec![TranscriptRewriteRange {
                notice_seq: Some(2),
                start_seq: 1,
                end_seq: 1,
            }]
        );
        assert_eq!(state.last_transcript_reset_seq, Some(3));
    }
}
