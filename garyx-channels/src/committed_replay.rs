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

use std::sync::Arc;

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use garyx_models::transcript_kind::is_control_message;
use garyx_router::{THREAD_TRANSCRIPT_REPLAY_CAP, ThreadTranscriptRecord};
use serde_json::{Map, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[derive(Debug, thiserror::Error)]
pub enum CommittedReplayError {
    #[error("gateway committed event bus is not wired")]
    MissingEventBus,
}

pub struct CommittedReplaySubscription {
    callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    task: Option<JoinHandle<()>>,
}

impl CommittedReplaySubscription {
    fn new(callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>, task: JoinHandle<()>) -> Self {
        Self {
            callback,
            task: Some(task),
        }
    }

    pub fn callback(&self) -> Option<Arc<dyn Fn(StreamEvent) + Send + Sync>> {
        self.callback.clone()
    }

    pub fn detach(mut self) {
        self.task.take();
    }

    pub fn abort(mut self) -> Option<JoinHandle<()>> {
        let task = self.task.take();
        if let Some(task) = task.as_ref() {
            task.abort();
        }
        task
    }
}

impl Drop for CommittedReplaySubscription {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

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
    async fn records_after_seq(
        &self,
        thread_id: &str,
        after_seq: u64,
    ) -> Vec<ThreadTranscriptRecord>;

    async fn records_for_run_after_seq(
        &self,
        thread_id: &str,
        run_id: &str,
        after_seq: u64,
    ) -> Vec<ThreadTranscriptRecord>;
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
            .records_after_seq(thread_id, after_seq, THREAD_TRANSCRIPT_REPLAY_CAP)
            .await
            .unwrap_or_default()
    }

    async fn records_for_run_after_seq(
        &self,
        thread_id: &str,
        run_id: &str,
        after_seq: u64,
    ) -> Vec<ThreadTranscriptRecord> {
        let Some(history) = self.thread_history().await else {
            return Vec::new();
        };
        history
            .transcript_store()
            .records_for_run_after_seq(thread_id, run_id, after_seq, THREAD_TRANSCRIPT_REPLAY_CAP)
            .await
            .unwrap_or_default()
    }
}

/// What a single committed/lifecycle bus line means for the run being replayed.
#[derive(Debug, PartialEq)]
enum BusSignal {
    /// Forward these events to the consumer now.
    Deliver(Vec<StreamEvent>),
    /// A committed row arrived ahead of the contiguous frontier (a broadcast
    /// drop). It must not be forwarded out of order; the durable transcript fills
    /// the hole in order instead. The async loop owns the I/O.
    GapFill,
    /// The run ended (`run_complete`/`run_error`): backfill the tail and stop.
    Terminal,
    /// Nothing to do for this line.
    Ignore,
}

/// Pure per-run reduction of the gateway committed/lifecycle bus into the
/// `StreamEvent` sequence a channel consumer expects.
///
/// The cursor is a **contiguous** frontier (highest in-order delivered seq), not
/// a high-water mark. A committed record is forwarded only when it is the next
/// contiguous seq; a hole (a broadcast drop) is never forwarded out of order,
/// because re-feeding a missed middle row after later rows would corrupt the
/// merged text. Holes are backfilled in order from the durable jsonl — which is
/// gapless and authoritative — on a broadcast `Lagged` (a drop always raises
/// one) and once more at terminal, so the final whole message stays complete and
/// ordered ("不丢、顺序对"). A same-seq overwrite (terminal tail rewrite) still
/// flows; an exact duplicate is dropped.
///
/// The first record is normally trusted as the run's start (the adapter
/// subscribes before dispatch, so with no drop the first row it sees is the
/// run's first emit). But if a drop happens *before* the frontier is
/// established, the first visible row may not be the start, so the frontier is
/// instead rebuilt from cursor 0 against the durable transcript once the
/// thread id is known — never trusting an arbitrary first-visible seq after an
/// initial lag.
struct CommittedReplayState {
    run_id: String,
    thread_id: Option<String>,
    /// Highest contiguously-delivered seq, or `None` before the first record.
    frontier: Option<u64>,
    /// Payload last delivered at `frontier`, to tell a same-seq overwrite (flow)
    /// from an exact duplicate (drop).
    frontier_payload: Option<String>,
    /// A broadcast drop happened before the frontier was established, so the
    /// first visible row cannot be trusted as the run's start.
    lagged_before_frontier: bool,
    done_emitted: bool,
}

impl CommittedReplayState {
    fn new(run_id: String) -> Self {
        Self::with_initial_thread_id(run_id, None)
    }

    fn with_thread_id(run_id: String, thread_id: String) -> Self {
        Self::with_initial_thread_id(run_id, Some(thread_id))
    }

    fn with_initial_thread_id(run_id: String, thread_id: Option<String>) -> Self {
        Self {
            run_id,
            thread_id,
            frontier: None,
            frontier_payload: None,
            lagged_before_frontier: false,
            done_emitted: false,
        }
    }

    fn on_bus_message(&mut self, raw: &str) -> BusSignal {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return BusSignal::Ignore;
        };
        let Some(object) = value.as_object() else {
            return BusSignal::Ignore;
        };
        match object.get("type").and_then(Value::as_str) {
            Some("committed_message") => {
                if !self.accepts_committed_object(object) {
                    return BusSignal::Ignore;
                }
                self.capture_thread_id(object);
                let Some(message) = object.get("message") else {
                    return BusSignal::Ignore;
                };
                if is_terminal_control_message(message) {
                    return BusSignal::Terminal;
                }
                let seq = object.get("seq").and_then(Value::as_u64).unwrap_or(0);
                match self.frontier {
                    // After an initial lag the first visible row may not be the
                    // run's start, so rebuild the frontier from the durable
                    // transcript (cursor 0) instead of trusting this seq.
                    None if self.lagged_before_frontier => BusSignal::GapFill,
                    // First record with no prior drop: trust it as the run's
                    // start. Or the next contiguous one. Deliver in order.
                    None => BusSignal::Deliver(self.deliver_record(seq, message)),
                    Some(frontier) if seq == frontier + 1 => {
                        BusSignal::Deliver(self.deliver_record(seq, message))
                    }
                    // Same-seq overwrite (terminal tail rewrite / snapshot growth):
                    // flow it when the payload changed, drop an exact duplicate.
                    Some(frontier) if seq == frontier => {
                        let payload = message.to_string();
                        if self.frontier_payload.as_deref() == Some(payload.as_str()) {
                            BusSignal::Ignore
                        } else {
                            BusSignal::Deliver(self.deliver_record(seq, message))
                        }
                    }
                    // Already delivered: an older duplicate, drop it.
                    Some(frontier) if seq < frontier => BusSignal::Ignore,
                    // A hole ahead of the frontier: recover from the durable
                    // transcript rather than forwarding out of order.
                    Some(_) => BusSignal::GapFill,
                }
            }
            _ => BusSignal::Ignore,
        }
    }

    /// Deliver durable records past the contiguous frontier, in seq order.
    ///
    /// Input is the gapless `records_after_seq` tail, so a hole left by a dropped
    /// broadcast row is recovered in its correct position. Run-mismatched and
    /// already-delivered rows are skipped, so it is safe to call repeatedly
    /// (`Lagged`, then terminal).
    fn reconcile_from_records(&mut self, records: &[ThreadTranscriptRecord]) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        for record in records {
            if record.run_id.as_deref() != Some(self.run_id.as_str())
                && !is_thread_scoped_control_message(&record.message)
            {
                continue;
            }
            if self.frontier.is_some_and(|frontier| record.seq <= frontier) {
                continue;
            }
            events.extend(self.deliver_record(record.seq, &record.message));
        }
        events
    }

    /// Map one record, advance the frontier, and remember whether `done` passed.
    fn deliver_record(&mut self, seq: u64, message: &Value) -> Vec<StreamEvent> {
        self.frontier = Some(seq);
        self.frontier_payload = Some(message.to_string());
        let events = committed_record_to_stream_events(message);
        self.note_done(&events);
        events
    }

    /// A synthetic terminal `Done` for runs that ended without a `done` control
    /// (interrupts/errors), so the consumer still flushes the final whole message.
    fn synthetic_done(&mut self) -> Option<StreamEvent> {
        if self.done_emitted {
            None
        } else {
            self.done_emitted = true;
            Some(StreamEvent::Done)
        }
    }

    /// Cursor for the next durable read: returns records with `seq > read_cursor`.
    fn read_cursor(&self) -> u64 {
        self.frontier.unwrap_or(0)
    }

    /// Record that the broadcast dropped events. While the frontier is not yet
    /// established this poisons the first-visible-row shortcut, so the start is
    /// rebuilt from the durable transcript instead.
    fn note_lag(&mut self) {
        if self.frontier.is_none() {
            self.lagged_before_frontier = true;
        }
    }

    fn needs_run_scoped_page(&self, tail: &[ThreadTranscriptRecord]) -> bool {
        if self.frontier.is_none() && self.lagged_before_frontier {
            return true;
        }
        let Some(first) = tail.first() else {
            return false;
        };
        let cursor = self.read_cursor();
        cursor > 0 && first.seq > cursor + 1
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

    fn accepts_committed_object(&self, object: &Map<String, Value>) -> bool {
        if object.get("run_id").and_then(Value::as_str) == Some(self.run_id.as_str()) {
            return true;
        }
        if object.get("run_id").is_some() {
            return false;
        }
        if !object
            .get("message")
            .is_some_and(is_thread_scoped_control_message)
        {
            return false;
        }
        self.thread_id.as_deref().is_none_or(|thread_id| {
            object.get("thread_id").and_then(Value::as_str) == Some(thread_id)
        })
    }

    fn note_done(&mut self, events: &[StreamEvent]) {
        if events
            .iter()
            .any(|event| matches!(event, StreamEvent::Done))
        {
            self.done_emitted = true;
        }
    }
}

fn is_thread_scoped_control_message(message: &Value) -> bool {
    message
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
        .is_some()
}

fn control_kind(message: &Value) -> Option<&str> {
    message
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
}

fn is_terminal_control_message(message: &Value) -> bool {
    matches!(control_kind(message), Some("run_complete" | "run_error"))
}

/// Drive a channel consumer from the durable per-thread committed stream.
///
/// `rx` must already be subscribed (before the run is dispatched, so the first
/// committed record is not missed). Every `committed_message{seq}` for `run_id`
/// is mapped to the `StreamEvent` sequence the consumer already understands and
/// handed to `consumer`; the consumer (telegram/discord/feishu/weixin sender or
/// the subprocess plugin host) is unchanged. Holes from a lagging broadcast and
/// the run's tail are backfilled in order from the durable transcript via
/// `reader`; the adapter returns on `run_complete`/`run_error`.
pub fn spawn_committed_channel_replay(
    rx: broadcast::Receiver<String>,
    reader: Arc<dyn CommittedTailReader>,
    run_id: String,
    consumer: Arc<dyn Fn(StreamEvent) + Send + Sync>,
) -> JoinHandle<()> {
    spawn_committed_channel_replay_with_state(
        rx,
        reader,
        CommittedReplayState::new(run_id),
        consumer,
    )
}

/// Drive a channel consumer from the durable committed stream when the
/// canonical thread id is known before dispatch.
///
/// This keeps the legacy inbound-channel subscription API unchanged while
/// letting bound delivery recover an initial broadcast lag immediately, before a
/// matching committed bus line arrives.
pub fn spawn_committed_channel_replay_for_thread(
    rx: broadcast::Receiver<String>,
    reader: Arc<dyn CommittedTailReader>,
    run_id: String,
    thread_id: String,
    consumer: Arc<dyn Fn(StreamEvent) + Send + Sync>,
) -> JoinHandle<()> {
    spawn_committed_channel_replay_with_state(
        rx,
        reader,
        CommittedReplayState::with_thread_id(run_id, thread_id),
        consumer,
    )
}

fn spawn_committed_channel_replay_with_state(
    rx: broadcast::Receiver<String>,
    reader: Arc<dyn CommittedTailReader>,
    initial_state: CommittedReplayState,
    consumer: Arc<dyn Fn(StreamEvent) + Send + Sync>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = rx;
        let mut state = initial_state;
        loop {
            match rx.recv().await {
                Ok(raw) => match state.on_bus_message(&raw) {
                    BusSignal::Deliver(events) => {
                        for event in events {
                            consumer(event);
                        }
                    }
                    BusSignal::GapFill => {
                        backfill(&mut state, reader.as_ref(), consumer.as_ref()).await;
                    }
                    BusSignal::Terminal => {
                        backfill(&mut state, reader.as_ref(), consumer.as_ref()).await;
                        if let Some(done) = state.synthetic_done() {
                            consumer(done);
                        }
                        break;
                    }
                    BusSignal::Ignore => {}
                },
                // A drop always raises `Lagged` before the receiver resumes. Note
                // it (so an initial lag does not let an arbitrary first-visible
                // row become the frontier) and pull the durable transcript to fill
                // any of this run's missed rows in order, then keep draining live.
                // The transcript read is a no-op until the thread id is known; the
                // lag note ensures the first committed message then rebuilds from
                // cursor 0 instead.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    state.note_lag();
                    backfill(&mut state, reader.as_ref(), consumer.as_ref()).await;
                }
                // The bus closed (shutdown); reconcile what we can and stop.
                Err(broadcast::error::RecvError::Closed) => {
                    backfill(&mut state, reader.as_ref(), consumer.as_ref()).await;
                    if let Some(done) = state.synthetic_done() {
                        consumer(done);
                    }
                    break;
                }
            }
        }
    })
}

/// Read the durable transcript from the contiguous frontier and deliver any
/// run-matching rows past it, in order. A no-op until the thread id is known.
async fn backfill(
    state: &mut CommittedReplayState,
    reader: &dyn CommittedTailReader,
    consumer: &(dyn Fn(StreamEvent) + Send + Sync),
) {
    let Some(thread_id) = state.thread_id.clone() else {
        return;
    };
    let records = reader
        .records_after_seq(&thread_id, state.read_cursor())
        .await;
    if state.needs_run_scoped_page(&records) {
        backfill_run_pages(state, reader, consumer, &thread_id).await;
        return;
    }
    for event in state.reconcile_from_records(&records) {
        consumer(event);
    }
}

async fn backfill_run_pages(
    state: &mut CommittedReplayState,
    reader: &dyn CommittedTailReader,
    consumer: &(dyn Fn(StreamEvent) + Send + Sync),
    thread_id: &str,
) {
    loop {
        let cursor = state.read_cursor();
        let page = reader
            .records_for_run_after_seq(thread_id, &state.run_id, cursor)
            .await;
        if page.is_empty() {
            break;
        }
        for event in state.reconcile_from_records(&page) {
            consumer(event);
        }
        if page.len() < THREAD_TRANSCRIPT_REPLAY_CAP || state.read_cursor() == cursor {
            break;
        }
    }
}

/// Wire a channel `consumer` to the committed stream and decide what callback to
/// hand `route_and_dispatch`.
///
/// This subscribes BEFORE dispatch and spawns [`spawn_committed_channel_replay`]
/// to drive `consumer` from the durable committed stream, returning `None` so
/// the bridge does not also drive `consumer` with the live `external_callback`.
/// Missing bus wiring is a configuration error: without the committed bus there
/// is no secondary content source to fall back to.
///
/// Call this immediately before `route_and_dispatch` so no committed record is
/// missed between subscribe and the run's first emit.
///
/// `pub(crate)` on purpose: the only production consumer is
/// [`crate::inbound::InboundPipeline`]. Downstream crates must dispatch
/// through the shared pipeline instead of re-rolling subscribe-before-
/// dispatch by hand; thread-scoped gateway delivery uses the still-public
/// [`committed_callback_for_thread`].
pub(crate) async fn committed_callback(
    bridge: &Arc<MultiProviderBridge>,
    run_id: &str,
    consumer: Arc<dyn Fn(StreamEvent) + Send + Sync>,
) -> Result<CommittedReplaySubscription, CommittedReplayError> {
    let Some(rx) = bridge.subscribe_events().await else {
        return Err(CommittedReplayError::MissingEventBus);
    };
    let task = spawn_committed_channel_replay(rx, bridge.clone(), run_id.to_owned(), consumer);
    Ok(CommittedReplaySubscription::new(None, task))
}

/// Wire a channel `consumer` to the committed stream when the canonical thread
/// id is already known before the run starts.
///
/// Gateway bound delivery uses this path. Inbound channel dispatch should keep
/// using [`committed_callback`], because those call sites subscribe before
/// routing resolves the canonical thread.
pub async fn committed_callback_for_thread(
    bridge: &Arc<MultiProviderBridge>,
    thread_id: &str,
    run_id: &str,
    consumer: Arc<dyn Fn(StreamEvent) + Send + Sync>,
) -> Result<CommittedReplaySubscription, CommittedReplayError> {
    let Some(rx) = bridge.subscribe_events().await else {
        return Err(CommittedReplayError::MissingEventBus);
    };
    let task = spawn_committed_channel_replay_for_thread(
        rx,
        bridge.clone(),
        run_id.to_owned(),
        thread_id.to_owned(),
        consumer,
    );
    Ok(CommittedReplaySubscription::new(None, task))
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

    #[tokio::test]
    async fn committed_callback_fails_closed_without_event_bus() {
        let bridge = Arc::new(MultiProviderBridge::new());
        let consumer = Arc::new(|_event: StreamEvent| {});

        let error = match committed_callback(&bridge, "run-missing-bus", consumer).await {
            Ok(_) => panic!("committed replay requires a configured event bus"),
            Err(error) => error,
        };

        assert!(matches!(error, CommittedReplayError::MissingEventBus));
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

    fn committed_line(seq: u64, message: Value) -> String {
        json!({
            "type": "committed_message",
            "thread_id": FIXTURE_THREAD,
            "run_id": FIXTURE_RUN,
            "seq": seq,
            "message": message,
        })
        .to_string()
    }

    fn committed_line_for_run(seq: u64, run_id: &str, message: Value) -> String {
        json!({
            "type": "committed_message",
            "thread_id": FIXTURE_THREAD,
            "run_id": run_id,
            "seq": seq,
            "message": message,
        })
        .to_string()
    }

    fn thread_scoped_control_line(seq: u64, thread_id: &str, kind: &str) -> String {
        json!({
            "type": "committed_message",
            "thread_id": thread_id,
            "seq": seq,
            "message": RunControlMessage::new(kind).build(),
        })
        .to_string()
    }

    fn run_lifecycle_line(seq: u64, kind: &str) -> String {
        committed_line(
            seq,
            RunControlMessage::new(kind)
                .with("status", json!("completed"))
                .with("duration_ms", json!(1234))
                .build(),
        )
    }

    fn assistant(text: &str) -> Value {
        json!({"role": "assistant", "text": text})
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

        async fn records_for_run_after_seq(
            &self,
            _thread_id: &str,
            run_id: &str,
            after_seq: u64,
        ) -> Vec<ThreadTranscriptRecord> {
            self.records
                .iter()
                .filter(|record| {
                    record.seq > after_seq
                        && (record.run_id.as_deref() == Some(run_id)
                            || (record.run_id.is_none()
                                && is_thread_scoped_control_message(&record.message)))
                })
                .cloned()
                .collect()
        }
    }

    /// Drain bus lines through the pure live reduction, collecting forwarded
    /// events and which signals were seen.
    fn drive_live(
        run_id: &str,
        lines: &[String],
    ) -> (Vec<StreamEvent>, bool, bool, CommittedReplayState) {
        let mut state = CommittedReplayState::new(run_id.to_owned());
        let mut events = Vec::new();
        let mut terminal = false;
        let mut gap = false;
        for line in lines {
            match state.on_bus_message(line) {
                BusSignal::Deliver(forwarded) => events.extend(forwarded),
                BusSignal::Terminal => terminal = true,
                BusSignal::GapFill => gap = true,
                BusSignal::Ignore => {}
            }
        }
        (events, terminal, gap, state)
    }

    #[test]
    fn contiguous_live_replay_matches_direct_mapping_and_detects_terminal() {
        let raw = include_str!("../../test-fixtures/stream-sync/transcript-with-control.jsonl");
        let mut lines = committed_bus_lines(raw);
        lines.push(run_lifecycle_line(9, "run_complete"));

        let (events, terminal, gap, state) = drive_live(FIXTURE_RUN, &lines);

        assert!(terminal, "run_complete must terminate the replay");
        assert!(!gap, "a contiguous stream never needs a gap fill");
        assert!(state.done_emitted, "the done control sets done_emitted");
        assert_eq!(
            events,
            replay(&transcript_messages(raw)),
            "the contiguous reduction equals the direct per-record mapping"
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
        assert_eq!(state.on_bus_message(&other), BusSignal::Ignore);
        assert_eq!(state.frontier, None);
    }

    #[test]
    fn same_seq_overwrite_flows_but_exact_duplicate_is_dropped() {
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());

        assert_eq!(
            state.on_bus_message(&committed_line(3, assistant("Hello"))),
            BusSignal::Deliver(vec![StreamEvent::Delta {
                text: "Hello".to_owned()
            }])
        );
        assert_eq!(
            state.on_bus_message(&committed_line(3, assistant("Hello"))),
            BusSignal::Ignore,
            "an exact same-seq re-emit is dropped"
        );
        assert_eq!(
            state.on_bus_message(&committed_line(3, assistant("Hello world"))),
            BusSignal::Deliver(vec![StreamEvent::Delta {
                text: "Hello world".to_owned()
            }]),
            "a same-seq overwrite still flows (merge keeps it idempotent)"
        );
    }

    #[test]
    fn dropped_middle_row_is_recovered_in_order() {
        // Reviewer regression (#TASK-852): live delivers seq 3, the broadcast
        // drops seq 4, then live sees seq 5 — a hole. The old high-water cursor
        // advanced past seq 4 and lost it forever. The contiguous frontier leaves
        // the hole for the gapless durable transcript to fill in its correct
        // position.
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        assert_eq!(
            state.on_bus_message(&committed_line(3, assistant("first "))),
            BusSignal::Deliver(vec![StreamEvent::Delta {
                text: "first ".to_owned()
            }])
        );
        assert_eq!(
            state.on_bus_message(&committed_line(5, assistant("third "))),
            BusSignal::GapFill,
            "a row ahead of the frontier is never forwarded out of order"
        );
        assert_eq!(
            state.frontier,
            Some(3),
            "the hole did not advance the frontier"
        );

        // The durable tail past the frontier (seq 3) is gapless: seq 4 then seq 5.
        let tail = vec![
            transcript_record(4, FIXTURE_RUN, assistant("second ")),
            transcript_record(5, FIXTURE_RUN, assistant("third ")),
        ];
        assert_eq!(
            state.reconcile_from_records(&tail),
            vec![
                StreamEvent::Delta {
                    text: "second ".to_owned()
                },
                StreamEvent::Delta {
                    text: "third ".to_owned()
                },
            ],
            "the missed middle row is recovered in its correct position"
        );
    }

    #[test]
    fn reconcile_skips_already_delivered_and_other_runs() {
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let _ = state.on_bus_message(&committed_line(4, assistant("a")));
        assert_eq!(state.frontier, Some(4));

        let tail = vec![
            // Already delivered (<= frontier): skip.
            transcript_record(4, FIXTURE_RUN, assistant("a")),
            // Another run on the same thread: skip.
            transcript_record(5, "run::other", assistant("theirs")),
            // New row for this run: deliver.
            transcript_record(6, FIXTURE_RUN, assistant("mine")),
        ];
        assert_eq!(
            state.reconcile_from_records(&tail),
            vec![StreamEvent::Delta {
                text: "mine".to_owned()
            }]
        );
    }

    #[test]
    fn terminal_synthesizes_done_when_run_ends_without_a_done_control() {
        // Interrupt/error: the tail has more content but no `done` control.
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let _ = state.on_bus_message(&committed_line(3, assistant("partial")));

        let tail = vec![transcript_record(4, FIXTURE_RUN, assistant("final tail"))];
        let mut events = state.reconcile_from_records(&tail);
        events.extend(state.synthetic_done());

        assert_eq!(
            events,
            vec![
                StreamEvent::Delta {
                    text: "final tail".to_owned()
                },
                StreamEvent::Done,
            ],
            "the dropped tail is recovered and Done is synthesized"
        );
        assert!(
            state.synthetic_done().is_none(),
            "Done is synthesized at most once"
        );
    }

    #[test]
    fn done_control_in_backfill_suppresses_synthetic_done() {
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let tail = vec![
            transcript_record(1, FIXTURE_RUN, assistant("hi")),
            transcript_record(2, FIXTURE_RUN, done_control_message()),
        ];
        assert_eq!(
            state.reconcile_from_records(&tail),
            vec![
                StreamEvent::Delta {
                    text: "hi".to_owned()
                },
                StreamEvent::Done,
            ]
        );
        assert!(
            state.synthetic_done().is_none(),
            "a done control observed in the backfill suppresses the synthetic Done"
        );
    }

    #[tokio::test]
    async fn spawn_recovers_dropped_middle_row_via_gap_fill_in_order() {
        use std::sync::Mutex as StdMutex;

        let (tx, rx) = broadcast::channel(64);
        let collected: Arc<StdMutex<Vec<StreamEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = {
            let collected = collected.clone();
            Arc::new(move |event| collected.lock().unwrap().push(event))
        };
        // The durable transcript holds all three segments in order.
        let reader = Arc::new(MockTailReader {
            records: vec![
                transcript_record(1, FIXTURE_RUN, assistant("first ")),
                transcript_record(2, FIXTURE_RUN, assistant("second ")),
                transcript_record(3, FIXTURE_RUN, assistant("third ")),
            ],
        });

        let handle = spawn_committed_channel_replay(rx, reader, FIXTURE_RUN.to_owned(), consumer);

        // Live delivers seq 1, the broadcast drops seq 2, live sees seq 3 (a
        // hole), then the run completes.
        tx.send(committed_line(1, assistant("first "))).unwrap();
        tx.send(committed_line(3, assistant("third "))).unwrap();
        tx.send(run_lifecycle_line(4, "run_complete")).unwrap();

        handle.await.unwrap();

        let events = collected.lock().unwrap().clone();
        let deltas: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Delta { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            deltas,
            vec!["first ", "second ", "third "],
            "the dropped middle row is recovered in order: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::Done))
                .count(),
            1,
            "exactly one Done"
        );
    }

    #[tokio::test]
    async fn spawn_recovers_dropped_tail_done_on_terminal() {
        use std::sync::Mutex as StdMutex;

        let (tx, rx) = broadcast::channel(64);
        let collected: Arc<StdMutex<Vec<StreamEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = {
            let collected = collected.clone();
            Arc::new(move |event| collected.lock().unwrap().push(event))
        };
        // Durable transcript: the assistant reply plus a seq-2 done control the
        // live broadcast will "drop".
        let reader = Arc::new(MockTailReader {
            records: vec![
                transcript_record(1, FIXTURE_RUN, assistant("only reply")),
                transcript_record(2, FIXTURE_RUN, done_control_message()),
            ],
        });

        let handle = spawn_committed_channel_replay(rx, reader, FIXTURE_RUN.to_owned(), consumer);

        // Live sees the reply but the broadcast drops the done control; the
        // run_complete lifecycle event triggers the terminal backfill.
        tx.send(committed_line(1, assistant("only reply"))).unwrap();
        tx.send(run_lifecycle_line(3, "run_complete")).unwrap();

        handle.await.unwrap();

        let events = collected.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                StreamEvent::Delta {
                    text: "only reply".to_owned()
                },
                StreamEvent::Done,
            ],
            "the dropped tail done is recovered from the transcript, not double-emitted"
        );
    }

    #[tokio::test]
    async fn spawn_synthesizes_done_for_interrupted_run_without_done_control() {
        use std::sync::Mutex as StdMutex;

        let (tx, rx) = broadcast::channel(64);
        let collected: Arc<StdMutex<Vec<StreamEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = {
            let collected = collected.clone();
            Arc::new(move |event| collected.lock().unwrap().push(event))
        };
        let reader = Arc::new(MockTailReader {
            records: vec![transcript_record(
                1,
                FIXTURE_RUN,
                assistant("partial reply"),
            )],
        });

        let handle = spawn_committed_channel_replay(rx, reader, FIXTURE_RUN.to_owned(), consumer);

        tx.send(committed_line(1, assistant("partial reply")))
            .unwrap();
        tx.send(run_lifecycle_line(2, "run_complete")).unwrap();

        handle.await.unwrap();

        let events = collected.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![
                StreamEvent::Delta {
                    text: "partial reply".to_owned()
                },
                StreamEvent::Done,
            ],
            "interrupted/error runs without done still flush buffered channel text"
        );
    }

    #[tokio::test]
    async fn replay_subscription_abort_stops_task_before_terminal_arrives() {
        let (_tx, rx) = broadcast::channel(64);
        let reader = Arc::new(MockTailReader {
            records: Vec::new(),
        });
        let consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(|_event| {});
        let task = spawn_committed_channel_replay(rx, reader, FIXTURE_RUN.to_owned(), consumer);
        let subscription = CommittedReplaySubscription::new(None, task);

        let task = subscription.abort().expect("subscription owns replay task");
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("aborted replay task should finish promptly")
            .expect_err("aborted task should be cancelled");

        assert!(error.is_cancelled());
    }

    #[test]
    fn first_visible_row_after_initial_lag_is_not_trusted_as_start() {
        // Reviewer regression R2 (#TASK-853): a drop before the frontier is
        // established (before thread_id is even known) must not let the first
        // visible row become the contiguous start, or earlier rows are lost.
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        state.note_lag();
        assert!(state.lagged_before_frontier);

        // seq 3 is the first row the receiver sees after the lag — not the run's
        // start. It must defer to a durable rebuild, not seed the frontier.
        assert_eq!(
            state.on_bus_message(&committed_line(3, assistant("third "))),
            BusSignal::GapFill,
            "an arbitrary first-visible row after an initial lag is not trusted"
        );
        assert_eq!(state.frontier, None, "the frontier is not seeded from it");
        assert_eq!(state.read_cursor(), 0, "the rebuild reads from cursor 0");

        // The transcript rebuild then delivers the whole run in order.
        let tail = vec![
            transcript_record(1, FIXTURE_RUN, assistant("first ")),
            transcript_record(2, FIXTURE_RUN, assistant("second ")),
            transcript_record(3, FIXTURE_RUN, assistant("third ")),
        ];
        assert_eq!(
            state.reconcile_from_records(&tail),
            vec![
                StreamEvent::Delta {
                    text: "first ".to_owned()
                },
                StreamEvent::Delta {
                    text: "second ".to_owned()
                },
                StreamEvent::Delta {
                    text: "third ".to_owned()
                },
            ],
            "the dropped initial rows are recovered in order from cursor 0"
        );
    }

    #[test]
    fn lag_after_frontier_is_established_does_not_force_a_full_rebuild() {
        // A drop *after* the start is a normal mid-run gap: the frontier already
        // anchors the in-order position, so only the tail past it is refilled.
        let mut state = CommittedReplayState::new(FIXTURE_RUN.to_owned());
        let _ = state.on_bus_message(&committed_line(1, assistant("first ")));
        assert_eq!(state.frontier, Some(1));
        state.note_lag();
        assert!(
            !state.lagged_before_frontier,
            "a lag once the frontier exists is an ordinary gap, not an unsafe start"
        );
        assert_eq!(
            state.read_cursor(),
            1,
            "the refill reads past the frontier, not from 0"
        );
    }

    #[tokio::test]
    async fn spawn_recovers_initial_lag_before_thread_id_via_full_rebuild() {
        use std::sync::Mutex as StdMutex;

        // A small buffer so publishing before the task drains forces a real
        // initial Lagged with the thread id still unknown.
        let (tx, rx) = broadcast::channel(2);
        let collected: Arc<StdMutex<Vec<StreamEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = {
            let collected = collected.clone();
            Arc::new(move |event| collected.lock().unwrap().push(event))
        };
        let reader = Arc::new(MockTailReader {
            records: vec![
                transcript_record(1, FIXTURE_RUN, assistant("first ")),
                transcript_record(2, FIXTURE_RUN, assistant("second ")),
                transcript_record(3, FIXTURE_RUN, assistant("third ")),
            ],
        });

        let handle = spawn_committed_channel_replay(rx, reader, FIXTURE_RUN.to_owned(), consumer);

        // Publish before the task polls: the cap-2 broadcast drops seq 1 and 2,
        // so the receiver's first poll is Lagged (thread id unknown), then it
        // sees only seq 3 and the run_complete.
        tx.send(committed_line(1, assistant("first "))).unwrap();
        tx.send(committed_line(2, assistant("second "))).unwrap();
        tx.send(committed_line(3, assistant("third "))).unwrap();
        tx.send(run_lifecycle_line(4, "run_complete")).unwrap();

        handle.await.unwrap();

        let events = collected.lock().unwrap().clone();
        let deltas: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Delta { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            deltas,
            vec!["first ", "second ", "third "],
            "the initial-lag drop is rebuilt from cursor 0 in order: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::Done))
                .count(),
            1,
            "exactly one Done"
        );
    }

    #[tokio::test]
    async fn spawn_backfills_initial_lag_before_matching_row_reaches_terminal() {
        use std::sync::Mutex as StdMutex;
        use std::time::Duration;

        let (tx, rx) = broadcast::channel(1);
        let collected: Arc<StdMutex<Vec<StreamEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let consumer: Arc<dyn Fn(StreamEvent) + Send + Sync> = {
            let collected = collected.clone();
            Arc::new(move |event| collected.lock().unwrap().push(event))
        };
        let reader = Arc::new(MockTailReader {
            records: vec![
                transcript_record(1, FIXTURE_RUN, assistant("first ")),
                transcript_record(2, FIXTURE_RUN, assistant("second ")),
            ],
        });

        let handle = spawn_committed_channel_replay_for_thread(
            rx,
            reader,
            FIXTURE_RUN.to_owned(),
            FIXTURE_THREAD.to_owned(),
            consumer,
        );

        // The target run's rows were already committed, but the receiver lags
        // before it has observed any matching row. The retained bus line belongs
        // to another run, so waiting for the next matching row would defer the
        // durable catch-up until terminal.
        tx.send(committed_line(1, assistant("first "))).unwrap();
        tx.send(committed_line(2, assistant("second "))).unwrap();
        tx.send(committed_line_for_run(
            3,
            "run::other",
            assistant("other run"),
        ))
        .unwrap();

        tokio::time::sleep(Duration::from_millis(25)).await;

        let deltas: Vec<String> = collected
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Delta { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            deltas,
            vec!["first ", "second "],
            "initial lag must backfill the target run before terminal"
        );

        tx.send(run_lifecycle_line(4, "run_complete")).unwrap();
        handle.await.unwrap();

        let events = collected.lock().unwrap().clone();
        let final_deltas: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::Delta { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            final_deltas,
            vec!["first ", "second "],
            "terminal backfill must not duplicate already recovered deltas"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::Done))
                .count(),
            1,
            "terminal still emits exactly one Done"
        );
    }

    #[test]
    fn seeded_thread_id_filters_thread_scoped_controls_by_thread() {
        let mut state =
            CommittedReplayState::with_thread_id(FIXTURE_RUN.to_owned(), FIXTURE_THREAD.to_owned());

        assert_eq!(
            state.on_bus_message(&thread_scoped_control_line(1, FIXTURE_THREAD, "user_ack")),
            BusSignal::Deliver(vec![StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: None,
            }]),
            "same-thread runless controls still flow for bound delivery"
        );
        assert_eq!(
            state.on_bus_message(&thread_scoped_control_line(2, "thread::other", "user_ack")),
            BusSignal::Ignore,
            "seeded bound replay rejects runless controls from other threads"
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
            control.insert("thread_id".to_owned(), json!(FIXTURE_THREAD));
            control.insert("run_id".to_owned(), json!(FIXTURE_RUN));
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
