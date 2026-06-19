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

use garyx_models::provider::{ProviderMessage, StreamBoundaryKind, StreamEvent};
use garyx_models::transcript_kind::is_control_message;
use serde_json::{Map, Value};

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
