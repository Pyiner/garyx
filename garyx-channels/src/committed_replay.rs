//! Map persisted committed transcript records back into the provider
//! [`StreamEvent`] sequence channel/plugin consumers already understand.
//!
//! Block 5 of the unified message-sync work switches channel consumers from
//! draining the bridge's live `external_callback` to reading the durable
//! per-thread committed stream (`committed_message{seq}`, content + control).
//! The consumers themselves (telegram/discord/feishu/weixin senders and the
//! subprocess plugin host) are unchanged: they still take a [`StreamEvent`]
//! sequence and run it through [`crate::plugin_tools`]. Only the *source*
//! changes. This module is the pure, testable seam between a committed record's
//! `message` value and that [`StreamEvent`] sequence.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use garyx_models::transcript_kind::is_control_message;
use garyx_router::ThreadTranscriptRecord;
use serde_json::{Map, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Cap on records pulled during the terminal reconcile. The reconcile only
/// fetches the tail beyond what was forwarded live, so this is generous; it only
/// matters as an upper bound when the live stream observed nothing (extreme lag)
/// and the whole run must be recovered from the durable tail.
const COMMITTED_REPLAY_RECONCILE_CAP: usize = 1024;

/// Map a single committed transcript record `message` payload to the
/// [`StreamEvent`]s a channel consumer would have observed live.
///
/// Input is the `message` object carried by a `committed_message{seq}` event
/// (equivalently a `ThreadTranscriptRecord.message`). Returns zero or more
/// events; records that carry no channel-stream meaning (user input echoes,
/// run lifecycle controls) map to an empty vec.
///
/// The granularity is whatever the durable record holds — a finalized assistant
/// segment maps to one [`StreamEvent::Delta`] carrying the whole text rather
/// than a token-by-token stream. The channel send policy in
/// [`crate::plugin_tools`] merges these via
/// [`crate::streaming_core::merge_stream_text`], which is snapshot-aware, so the
/// final rendered message is equivalent to the live token stream.
pub fn committed_record_to_stream_events(message: &Value) -> Vec<StreamEvent> {
    let Some(object) = message.as_object() else {
        return Vec::new();
    };

    if is_control_message(object) {
        return control_record_to_stream_events(object);
    }

    let role = object
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match role {
        "assistant" => match committed_assistant_text(object) {
            Some(text) => vec![StreamEvent::Delta { text }],
            None => Vec::new(),
        },
        "tool_use" => ProviderMessage::from_value(message)
            .map(|message| vec![StreamEvent::ToolUse { message }])
            .unwrap_or_default(),
        "tool_result" => ProviderMessage::from_value(message)
            .map(|message| vec![StreamEvent::ToolResult { message }])
            .unwrap_or_default(),
        // User input is never echoed back into the channel stream, and
        // system/other content records carry no channel-visible event.
        _ => Vec::new(),
    }
}

/// Map a `kind=control` record to the channel-visible [`StreamEvent`]s.
///
/// Run lifecycle facts (`run_start`, `run_complete`) intentionally map to
/// nothing: they are not content-stream events, and the replay adapter consumes
/// them as terminal triggers instead of forwarding them. Mapping `run_complete`
/// to [`StreamEvent::Done`] here would double-flush alongside the `done`
/// control.
fn control_record_to_stream_events(object: &Map<String, Value>) -> Vec<StreamEvent> {
    let Some(control) = object.get("control").and_then(Value::as_object) else {
        return Vec::new();
    };
    let kind = control
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let pending_input_id = control
        .get("pending_input_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    match kind {
        "assistant_boundary" => vec![StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id,
        }],
        "user_ack" => vec![StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id,
        }],
        "done" => vec![StreamEvent::Done],
        "thread_title_updated" => control
            .get("title")
            .and_then(Value::as_str)
            .map(|title| {
                vec![StreamEvent::ThreadTitleUpdated {
                    title: title.to_owned(),
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Finalized assistant text carried by a committed content record.
///
/// Prefers the explicit `text` field (what the provider streamed), falling back
/// to a string `content`. Tool-carrying assistant records with no plain text map
/// to nothing, so they never produce an empty channel delta.
fn committed_assistant_text(object: &Map<String, Value>) -> Option<String> {
    let non_empty = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|text| !text.is_empty())
    };
    non_empty(object.get("text")).or_else(|| non_empty(object.get("content")))
}

/// Reads the durable transcript tail for the terminal reconcile.
///
/// Abstracts the bridge so the replay adapter is testable without a live
/// provider. Returns the thread's records with `seq > after_seq`.
#[async_trait]
pub trait CommittedTailReader: Send + Sync {
    async fn records_after_seq(&self, thread_id: &str, after_seq: u64)
    -> Vec<ThreadTranscriptRecord>;
}

#[async_trait]
impl CommittedTailReader for MultiProviderBridge {
    async fn records_after_seq(
        &self,
        thread_id: &str,
        after_seq: u64,
    ) -> Vec<ThreadTranscriptRecord> {
        let Some(history) = self.thread_history().await else {
            return Vec::new();
        };
        history
            .transcript_store()
            .records_after_seq(thread_id, after_seq, COMMITTED_REPLAY_RECONCILE_CAP)
            .await
            .unwrap_or_default()
    }
}

/// What a single gateway-bus line yields for the run being replayed.
#[derive(Debug, Default, PartialEq)]
struct BusOutcome {
    events: Vec<StreamEvent>,
    terminal: bool,
}

/// Pure per-run reduction of the gateway committed/lifecycle bus into the
/// `StreamEvent` sequence a channel consumer expects.
///
/// Live phase: each `committed_message{seq}` for this run maps through
/// [`committed_record_to_stream_events`], deduped so an exact same-seq re-emit
/// is dropped while a same-seq *overwrite* (changed payload) still flows (the
/// snapshot-aware merge in `plugin_tools` keeps that idempotent for text).
///
/// Terminal phase: a `run_complete`/`run_error` for this run triggers a one-shot
/// transcript reconcile — any tail records the live broadcast dropped are
/// replayed, and a synthetic [`StreamEvent::Done`] is emitted if the run ended
/// without a `done` control (interrupts/errors) so the consumer still flushes
/// the final whole message.
struct CommittedReplayState {
    run_id: String,
    thread_id: Option<String>,
    /// `seq -> last forwarded message payload`, mirroring the per-thread SSE
    /// forward-dedup so exact duplicates are dropped but overwrites still flow.
    forwarded: HashMap<u64, String>,
    last_emitted_seq: u64,
    done_emitted: bool,
}

impl CommittedReplayState {
    fn new(run_id: String) -> Self {
        Self {
            run_id,
            thread_id: None,
            forwarded: HashMap::new(),
            last_emitted_seq: 0,
            done_emitted: false,
        }
    }

    fn on_bus_message(&mut self, raw: &str) -> BusOutcome {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return BusOutcome::default();
        };
        let Some(object) = value.as_object() else {
            return BusOutcome::default();
        };
        if object.get("run_id").and_then(Value::as_str) != Some(self.run_id.as_str()) {
            return BusOutcome::default();
        }
        match object.get("type").and_then(Value::as_str) {
            Some("committed_message") => {
                self.capture_thread_id(object);
                let Some(message) = object.get("message") else {
                    return BusOutcome::default();
                };
                let seq = object.get("seq").and_then(Value::as_u64).unwrap_or(0);
                let payload = message.to_string();
                if self.forwarded.get(&seq).is_some_and(|prev| prev == &payload) {
                    return BusOutcome::default();
                }
                self.forwarded.insert(seq, payload);
                self.last_emitted_seq = self.last_emitted_seq.max(seq);
                let events = committed_record_to_stream_events(message);
                self.note_done(&events);
                BusOutcome {
                    events,
                    terminal: false,
                }
            }
            Some("run_complete") | Some("run_error") => {
                self.capture_thread_id(object);
                BusOutcome {
                    events: Vec::new(),
                    terminal: true,
                }
            }
            _ => BusOutcome::default(),
        }
    }

    /// Terminal reconcile: replay any durable tail records past what was
    /// forwarded live, then synthesize `Done` if the run never emitted one.
    fn reconcile_events(&mut self, tail: &[ThreadTranscriptRecord]) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        for record in tail {
            if record.run_id.as_deref() != Some(self.run_id.as_str()) {
                continue;
            }
            // `records_after_seq` already returns seq > last_emitted_seq; the
            // guard keeps the reconcile append-only even if a caller passes a
            // looser cursor, so a record is never forwarded twice.
            if record.seq <= self.last_emitted_seq {
                continue;
            }
            self.last_emitted_seq = record.seq;
            let mapped = committed_record_to_stream_events(&record.message);
            self.note_done(&mapped);
            events.extend(mapped);
        }
        if !self.done_emitted {
            self.done_emitted = true;
            events.push(StreamEvent::Done);
        }
        events
    }

    fn capture_thread_id(&mut self, object: &Map<String, Value>) {
        if self.thread_id.is_some() {
            return;
        }
        if let Some(thread_id) = object
            .get("thread_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            self.thread_id = Some(thread_id.to_owned());
        }
    }

    fn note_done(&mut self, events: &[StreamEvent]) {
        if events.iter().any(|event| matches!(event, StreamEvent::Done)) {
            self.done_emitted = true;
        }
    }
}

/// Drive a channel consumer from the durable per-thread committed stream.
///
/// `rx` must already be subscribed (before the run is dispatched, so the first
/// committed record is not missed). Every `committed_message{seq}` for `run_id`
/// is mapped to the `StreamEvent` sequence the consumer already understands and
/// handed to `consumer`; the consumer (telegram/discord/feishu/weixin sender or
/// the subprocess plugin host) is unchanged. On `run_complete`/`run_error` the
/// adapter runs a one-shot transcript reconcile through `reader` to backfill any
/// dropped tail and flush the final whole message, then returns.
pub fn spawn_committed_channel_replay(
    rx: broadcast::Receiver<String>,
    reader: Arc<dyn CommittedTailReader>,
    run_id: String,
    consumer: Arc<dyn Fn(StreamEvent) + Send + Sync>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = rx;
        let mut state = CommittedReplayState::new(run_id);
        loop {
            match rx.recv().await {
                Ok(raw) => {
                    let outcome = state.on_bus_message(&raw);
                    for event in outcome.events {
                        consumer(event);
                    }
                    if outcome.terminal {
                        finish_replay(&mut state, reader.as_ref(), consumer.as_ref()).await;
                        break;
                    }
                }
                // A slow consumer dropped events; the terminal reconcile backfills
                // the tail from the durable transcript, so keep draining.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                // The bus closed (shutdown); reconcile what we can and stop.
                Err(broadcast::error::RecvError::Closed) => {
                    finish_replay(&mut state, reader.as_ref(), consumer.as_ref()).await;
                    break;
                }
            }
        }
    })
}

async fn finish_replay(
    state: &mut CommittedReplayState,
    reader: &dyn CommittedTailReader,
    consumer: &(dyn Fn(StreamEvent) + Send + Sync),
) {
    let tail = match state.thread_id.as_deref() {
        Some(thread_id) => reader.records_after_seq(thread_id, state.last_emitted_seq).await,
        None => Vec::new(),
    };
    for event in state.reconcile_events(&tail) {
        consumer(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_tools::{
        PluginStreamSendDecision, PluginStreamSendPolicy, PluginStreamSendState,
    };
    use serde_json::json;
    use std::time::Instant;

    /// Pull the `message` payloads out of a transcript-shaped fixture
    /// (`{seq, thread_id, run_id, timestamp, message}` per line).
    fn transcript_messages(raw: &str) -> Vec<Value> {
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let record: Value = serde_json::from_str(line).expect("fixture line is json");
                record
                    .get("message")
                    .cloned()
                    .expect("transcript record carries a message")
            })
            .collect()
    }

    /// Map an ordered list of committed record messages to the flattened
    /// `StreamEvent` sequence, as the replay adapter would feed a consumer.
    fn replay(messages: &[Value]) -> Vec<StreamEvent> {
        messages
            .iter()
            .flat_map(committed_record_to_stream_events)
            .collect()
    }

    fn assert_public_fixture(raw: &str) {
        assert!(
            raw.match_indices("/Users/")
                .all(|(offset, _)| raw[offset..].starts_with("/Users/test")),
            "fixture must use synthetic local user paths"
        );
        assert!(
            !raw.contains('@'),
            "fixture must not contain email-like personal identifiers"
        );
    }

    #[test]
    fn content_records_map_to_delta_and_tool_events() {
        let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-tool.jsonl");
        assert_public_fixture(raw);
        let messages = transcript_messages(raw);
        let events = replay(&messages);

        assert_eq!(
            events,
            vec![
                StreamEvent::Delta {
                    text: "I will inspect the fixture directory and report the file count."
                        .to_owned()
                },
                StreamEvent::ToolUse {
                    message: ProviderMessage::from_value(&messages[2]).unwrap()
                },
                StreamEvent::ToolResult {
                    message: ProviderMessage::from_value(&messages[3]).unwrap()
                },
                StreamEvent::Delta {
                    text: "The fixture directory is present and contains the expected files."
                        .to_owned()
                },
            ],
            "user input is skipped; assistant/tool records map 1:1 in seq order"
        );
    }

    #[test]
    fn tool_events_preserve_tool_identity_for_placeholder_rendering() {
        let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-tool.jsonl");
        let messages = transcript_messages(raw);
        let StreamEvent::ToolUse { message } = &replay(&messages)[1] else {
            panic!("second event should be a tool_use");
        };
        assert_eq!(message.tool_name.as_deref(), Some("commandExecution"));
        assert_eq!(message.tool_use_id.as_deref(), Some("call_fixture_ls"));
    }

    #[test]
    fn control_records_map_to_boundary_and_done() {
        let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-control.jsonl");
        assert_public_fixture(raw);
        let messages = transcript_messages(raw);
        let events = replay(&messages);

        assert_eq!(
            events,
            vec![
                // seq1 run_start control + seq2 user input both drop out.
                StreamEvent::Delta {
                    text: "Looking into the fixtures now.".to_owned()
                },
                StreamEvent::ToolUse {
                    message: ProviderMessage::from_value(&messages[3]).unwrap()
                },
                StreamEvent::ToolResult {
                    message: ProviderMessage::from_value(&messages[4]).unwrap()
                },
                StreamEvent::Boundary {
                    kind: StreamBoundaryKind::AssistantSegment,
                    pending_input_id: None,
                },
                StreamEvent::Delta {
                    text: "All fixture checks passed.".to_owned()
                },
                StreamEvent::Done,
            ],
            "run lifecycle controls drop; boundary/done map to stream events"
        );
    }

    #[test]
    fn user_ack_control_preserves_pending_input_id() {
        let message = RunControlMessage::new("user_ack")
            .with("pending_input_id", json!("pending-followup-1"))
            .build();
        assert_eq!(
            committed_record_to_stream_events(&message),
            vec![StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: Some("pending-followup-1".to_owned()),
            }]
        );
    }

    #[test]
    fn run_lifecycle_controls_produce_no_channel_events() {
        for kind in ["run_start", "run_complete"] {
            let message = RunControlMessage::new(kind).build();
            assert!(
                committed_record_to_stream_events(&message).is_empty(),
                "{kind} is a lifecycle fact, not a channel stream event"
            );
        }
    }

    #[test]
    fn thread_title_control_maps_to_title_update() {
        let message = RunControlMessage::new("thread_title_updated")
            .with("title", json!("Fixture thread"))
            .build();
        assert_eq!(
            committed_record_to_stream_events(&message),
            vec![StreamEvent::ThreadTitleUpdated {
                title: "Fixture thread".to_owned()
            }]
        );
    }

    #[test]
    fn user_and_system_content_records_are_skipped() {
        let user = json!({"role": "user", "content": "hi", "text": "hi"});
        let system = json!({"role": "system", "content": "boot"});
        assert!(committed_record_to_stream_events(&user).is_empty());
        assert!(committed_record_to_stream_events(&system).is_empty());
    }

    #[test]
    fn empty_assistant_record_produces_no_delta() {
        let empty = json!({"role": "assistant", "content": "", "text": ""});
        assert!(committed_record_to_stream_events(&empty).is_empty());
        let tool_only = json!({"role": "assistant", "content": {"tool_use_id": "x"}});
        assert!(committed_record_to_stream_events(&tool_only).is_empty());
    }

    /// End-to-end equivalence at the pure-function level: replaying the real
    /// transcript through the same `plugin_tools` send state both channel
    /// families use yields the full assistant text, segment-separated. This is
    /// the "渠道消息等价" invariant without opening a channel.
    #[test]
    fn replayed_events_render_equivalent_final_text() {
        let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-control.jsonl");
        let messages = transcript_messages(raw);
        let events = replay(&messages);

        for policy in [
            PluginStreamSendPolicy::telegram_like(),
            PluginStreamSendPolicy::buffered_until_tool_or_done(),
        ] {
            let mut state = PluginStreamSendState::new(policy);
            let now = Instant::now();
            let mut final_text = None;
            for event in &events {
                match event {
                    StreamEvent::Delta { text } => {
                        let _ = state.on_delta(text, now);
                    }
                    StreamEvent::ToolUse { message } => {
                        let _ = state.on_tool_call(message, now);
                    }
                    StreamEvent::Boundary { kind, .. } => state.apply_boundary(kind.clone()),
                    StreamEvent::Done => {
                        if let PluginStreamSendDecision::FlushNow { content_text } =
                            state.on_done(now)
                        {
                            final_text = Some(content_text);
                        }
                    }
                    StreamEvent::ToolResult { .. }
                    | StreamEvent::ThreadTitleUpdated { .. }
                    | StreamEvent::SessionBound { .. } => {}
                }
            }

            let final_text = final_text.expect("done should flush accumulated text");
            assert!(
                final_text.contains("Looking into the fixtures now."),
                "first assistant segment must survive: {final_text:?}"
            );
            assert!(
                final_text.contains("All fixture checks passed."),
                "second assistant segment must survive: {final_text:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Replay adapter (committed bus -> consumer) tests
    // -----------------------------------------------------------------------

    const FIXTURE_THREAD: &str = "thread::fixture-stream-sync-control";
    const FIXTURE_RUN: &str = "run::fixture-control";

    /// Wrap each transcript fixture record into the `committed_message{seq}`
    /// gateway-bus line the persistence worker emits after the write flush.
    fn committed_bus_lines(raw: &str) -> Vec<String> {
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let record: Value = serde_json::from_str(line).unwrap();
                json!({
                    "type": "committed_message",
                    "thread_id": record["thread_id"],
                    "run_id": record["run_id"],
                    "seq": record["seq"],
                    "message": record["message"],
                })
                .to_string()
            })
            .collect()
    }

    fn run_lifecycle_line(kind: &str) -> String {
        json!({
            "type": kind,
            "thread_id": FIXTURE_THREAD,
            "run_id": FIXTURE_RUN,
            "duration_ms": 1234,
        })
        .to_string()
    }

    fn transcript_record(seq: u64, run_id: &str, message: Value) -> ThreadTranscriptRecord {
        ThreadTranscriptRecord {
            seq,
            thread_id: FIXTURE_THREAD.to_owned(),
            run_id: Some(run_id.to_owned()),
            timestamp: "2026-06-18T12:30:04Z".to_owned(),
            message,
        }
    }

    fn done_control_message() -> Value {
        RunControlMessage::new("done").build()
    }

    struct MockTailReader {
        records: Vec<ThreadTranscriptRecord>,
    }

    #[async_trait]
    impl CommittedTailReader for MockTailReader {
        async fn records_after_seq(
            &self,
            _thread_id: &str,
            after_seq: u64,
        ) -> Vec<ThreadTranscriptRecord> {
            self.records
                .iter()
                .filter(|record| record.seq > after_seq)
                .cloned()
                .collect()
        }
    }

    /// Drain a list of bus lines through the pure live reduction.
    fn drive_live(run_id: &str, lines: &[String]) -> (Vec<StreamEvent>, bool, CommittedReplayState) {
        let mut state = CommittedReplayState::new(run_id.to_owned());
        let mut events = Vec::new();
        let mut terminal = false;
        for line in lines {
            let outcome = state.on_bus_message(line);
            events.extend(outcome.events);
            terminal |= outcome.terminal;
        }
        (events, terminal, state)
    }

    #[test]
    fn live_bus_replay_matches_direct_mapping_and_detects_terminal() {
        let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-control.jsonl");
        let mut lines = committed_bus_lines(raw);
        lines.push(run_lifecycle_line("run_complete"));

        let (events, terminal, state) = drive_live(FIXTURE_RUN, &lines);

        assert!(terminal, "run_complete must terminate the replay");
        assert!(state.done_emitted, "the done control sets done_emitted");
        assert_eq!(
            events,
            replay(&transcript_messages(raw)),
            "the bus reduction equals the direct per-record mapping"
        );
    }

    #[test]
    fn committed_messages_for_other_runs_are_ignored() {
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let other = json!({
            "type": "committed_message",
            "thread_id": FIXTURE_THREAD,
            "run_id": "run::someone-else",
            "seq": 1,
            "message": {"role": "assistant", "text": "not mine"},
        })
        .to_string();
        assert_eq!(state.on_bus_message(&other), BusOutcome::default());
        assert_eq!(state.last_emitted_seq, 0);
    }

    #[test]
    fn exact_duplicate_seq_is_deduped_but_overwrite_flows() {
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let line = |text: &str| {
            json!({
                "type": "committed_message",
                "thread_id": FIXTURE_THREAD,
                "run_id": FIXTURE_RUN,
                "seq": 3,
                "message": {"role": "assistant", "text": text},
            })
            .to_string()
        };

        assert_eq!(
            state.on_bus_message(&line("Hello")).events,
            vec![StreamEvent::Delta {
                text: "Hello".to_owned()
            }]
        );
        assert!(
            state.on_bus_message(&line("Hello")).events.is_empty(),
            "an exact same-seq re-emit is deduped"
        );
        assert_eq!(
            state.on_bus_message(&line("Hello world")).events,
            vec![StreamEvent::Delta {
                text: "Hello world".to_owned()
            }],
            "a same-seq overwrite still flows (merge keeps it idempotent)"
        );
    }

    #[test]
    fn terminal_reconcile_backfills_dropped_tail_and_synthesizes_done() {
        // Live only saw the first segment; the rest of the run was dropped.
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let _ = state.on_bus_message(
            &json!({
                "type": "committed_message",
                "thread_id": FIXTURE_THREAD,
                "run_id": FIXTURE_RUN,
                "seq": 3,
                "message": {"role": "assistant", "text": "Looking into the fixtures now."},
            })
            .to_string(),
        );
        assert_eq!(state.last_emitted_seq, 3);

        // The durable tail has a later assistant segment but no done control
        // (an interrupt/error run).
        let tail = vec![transcript_record(
            7,
            FIXTURE_RUN,
            json!({"role": "assistant", "text": "All fixture checks passed."}),
        )];
        assert_eq!(
            state.reconcile_events(&tail),
            vec![
                StreamEvent::Delta {
                    text: "All fixture checks passed.".to_owned()
                },
                StreamEvent::Done,
            ],
            "the dropped tail is replayed and Done is synthesized"
        );
    }

    #[test]
    fn terminal_reconcile_with_done_in_tail_does_not_double_emit_done() {
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let tail = vec![
            transcript_record(1, FIXTURE_RUN, json!({"role": "assistant", "text": "hi"})),
            transcript_record(2, FIXTURE_RUN, done_control_message()),
        ];
        assert_eq!(
            state.reconcile_events(&tail),
            vec![
                StreamEvent::Delta {
                    text: "hi".to_owned()
                },
                StreamEvent::Done,
            ]
        );
        assert!(
            state.reconcile_events(&[]).is_empty(),
            "Done is not re-emitted once observed"
        );
    }

    #[test]
    fn reconcile_skips_records_from_other_runs() {
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let tail = vec![
            transcript_record(5, "run::other", json!({"role": "assistant", "text": "theirs"})),
            transcript_record(6, FIXTURE_RUN, json!({"role": "assistant", "text": "mine"})),
        ];
        assert_eq!(
            state.reconcile_events(&tail),
            vec![
                StreamEvent::Delta {
                    text: "mine".to_owned()
                },
                StreamEvent::Done,
            ]
        );
    }

    #[tokio::test]
    async fn spawn_replays_live_and_recovers_dropped_done_on_terminal() {
        use std::sync::Mutex as StdMutex;

        let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-control.jsonl");
        let bus_lines = committed_bus_lines(raw);

        let (tx, rx) = broadcast::channel(64);
        let collected: Arc<StdMutex<Vec<StreamEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = {
            let collected = collected.clone();
            Arc::new(move |event| collected.lock().unwrap().push(event))
        };
        // The durable transcript still has the seq-8 done control the live
        // broadcast will "drop" below.
        let reader = Arc::new(MockTailReader {
            records: vec![transcript_record(8, FIXTURE_RUN, done_control_message())],
        });

        let handle =
            spawn_committed_channel_replay(rx, reader, FIXTURE_RUN.to_owned(), consumer);

        // Emit seq 1..7 (drop the seq-8 done control), then the run_complete
        // lifecycle event that terminates and triggers the reconcile.
        for line in bus_lines.iter().take(7) {
            tx.send(line.clone()).unwrap();
        }
        tx.send(run_lifecycle_line("run_complete")).unwrap();

        handle.await.unwrap();

        let events = collected.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::Done))
                .count(),
            1,
            "exactly one Done, recovered from the durable tail after the drop"
        );
        assert!(
            events.contains(&StreamEvent::Delta {
                text: "All fixture checks passed.".to_owned()
            }),
            "the final assistant segment is delivered: {events:?}"
        );
    }

    /// Minimal builder for `RunControlRecord`-shaped committed messages, mirroring
    /// `garyx-bridge`'s `RunControlRecord::new` output so the mapper is tested
    /// against the real on-store control shape.
    struct RunControlMessage {
        control: Map<String, Value>,
    }

    impl RunControlMessage {
        fn new(kind: &str) -> Self {
            let mut control = Map::new();
            control.insert("kind".to_owned(), json!(kind));
            control.insert("thread_id".to_owned(), json!("thread::fixture-control"));
            control.insert("run_id".to_owned(), json!("run::fixture-control"));
            control.insert("at".to_owned(), json!("2026-06-18T12:00:00Z"));
            Self { control }
        }

        fn with(mut self, key: &str, value: Value) -> Self {
            self.control.insert(key.to_owned(), value);
            self
        }

        fn build(self) -> Value {
            json!({
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": Value::Object(self.control),
            })
        }
    }
}
