//! [`AgentTeamProvider`] — the meta-provider that turns a Team into a group
//! chat over per-sub-agent threads.
//!
//! This module implements the provider dispatch loop:
//!
//! 1. Resolve the team profile from `options.metadata["agent_team_id"]`.
//! 2. Load or hydrate the [`Group`] state for this thread.
//! 3. Plan the turn (mention parse → targets).
//! 4. For each target: ensure its child thread exists, build a combined
//!    message (catch-up envelopes + live turn), dispatch to the child's
//!    provider and forward the stream back.
//! 5. Advance the per-child catch-up offset, persist the group, and
//!    aggregate the child results into a single [`ProviderRunResult`].
//!
//! Dependencies are injected via two narrow traits, [`SubAgentDispatcher`]
//! and [`TeamProfileResolver`] (see `dispatcher.rs`). This keeps the
//! provider out of the bridge/gateway dependency graph and lets us unit
//! test the whole loop with in-process mocks.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDateTime};
use garyx_models::AgentTeamProfile;
use garyx_models::provider::{
    ProviderMessage, ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
};
use serde_json::Value;

use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

use super::dispatcher::{SubAgentDispatcher, TeamProfileResolver};
use super::planner::plan_turn;
use super::store::{Group, GroupStore};

/// Metadata key that the dispatch layer sets to identify the team that the
/// group thread is bound to.
const META_TEAM_ID: &str = "agent_team_id";

/// Metadata key carrying a pre-materialized snapshot of the group thread's
/// transcript (without the live user turn) that `AgentTeamProvider` slices
/// for catch-up. The gateway injects this; if missing we treat the transcript
/// as empty, which is the correct behavior for the very first turn.
const META_TRANSCRIPT_SNAPSHOT: &str = "group_transcript_snapshot";
const META_CLIENT_TIMESTAMP_LOCAL: &str = "client_timestamp_local";

/// One entry of the group-transcript snapshot metadata. Mirrors the shape
/// documented in the file header: `{ "agent_id": "...", "text": "...",
/// "at": "RFC3339 timestamp" }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptEntry {
    pub agent_id: String,
    pub text: String,
    pub at: String,
}

/// Parse `metadata["group_transcript_snapshot"]` into a vector of
/// [`TranscriptEntry`]. Missing, malformed, or non-array values yield an
/// empty vector — we never panic on bad metadata.
pub(crate) fn parse_group_transcript(metadata: &HashMap<String, Value>) -> Vec<TranscriptEntry> {
    let Some(value) = metadata.get(META_TRANSCRIPT_SNAPSHOT) else {
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let agent_id = obj.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
        let text = obj.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let at = obj.get("at").and_then(|v| v.as_str()).unwrap_or("");
        // An entry with no agent_id and no text is useless; skip it so we
        // don't emit empty envelopes.
        if agent_id.is_empty() && text.is_empty() {
            continue;
        }
        out.push(TranscriptEntry {
            agent_id: agent_id.to_owned(),
            text: text.to_owned(),
            at: at.to_owned(),
        });
    }
    out
}

/// Escape a value destined for an XML-style attribute (`from="..."`,
/// `at="..."`). We only need to neutralize the quote and the two
/// metacharacters that could produce structurally invalid tags; we leave
/// other bytes alone so the LLM still sees readable values.
fn escape_envelope_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '&' => out.push_str("&amp;"),
            other => out.push(other),
        }
    }
    out
}

/// Neutralize the only thing in the body that can break out of the
/// `<group_activity>` envelope: a literal closing tag. Replacing it with
/// the backslash-escaped form keeps the text human- and LLM-readable while
/// preventing a prior agent's (or user's) `</group_activity>` from
/// detaching the rest of the body into free-floating prompt text.
///
/// Chosen scope: only the closing tag, not `<` in general — agents and
/// users legitimately paste HTML/XML/code snippets, and blanket-escaping
/// all `<` destroys that fidelity for the downstream model.
fn sanitize_envelope_body(text: &str) -> String {
    text.replace("</group_activity>", r"<\/group_activity>")
}

/// Build the combined message delivered to a sub-agent for a single turn.
///
/// Format (see design doc §7.4):
/// ```text
/// <group_activity from="{agent_id}" at="{at}">
/// {text}
/// </group_activity>
///
/// <group_activity from="..." at="...">
/// ...
/// </group_activity>
///
/// {live_turn}
/// ```
///
/// - Each catch-up entry is wrapped in its own envelope.
/// - Envelopes are separated by a single blank line.
/// - `from` / `at` attribute values and envelope bodies are
///   escaped/sanitized so a prior turn cannot break out of its envelope
///   via a literal `"` in attributes or `</group_activity>` in the body.
/// - The live turn is appended AFTER the envelopes, separated by a blank
///   line, and is NOT wrapped — that's the caller's responsibility if the
///   message originated from a peer agent (the gateway pre-wraps routed
///   messages before handing them to this provider).
/// - If there are no catch-up entries, the output is just `live_turn`.
pub(crate) fn build_combined_message(catchup_slice: &[TranscriptEntry], live_turn: &str) -> String {
    if catchup_slice.is_empty() {
        return live_turn.to_owned();
    }
    let mut out = String::new();
    for (idx, entry) in catchup_slice.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "<group_activity from=\"{}\" at=\"{}\">\n{}\n</group_activity>",
            escape_envelope_attr(&entry.agent_id),
            escape_envelope_attr(&format_group_activity_timestamp(&entry.at)),
            sanitize_envelope_body(&entry.text),
        ));
    }
    if !live_turn.trim().is_empty() {
        out.push_str("\n\n");
        out.push_str(live_turn);
    }
    out
}

fn format_group_activity_timestamp(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    }
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(trimmed) {
        return timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
    }
    if let Ok(timestamp) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
    }
    trimmed.to_owned()
}

fn label_from_metadata(metadata: &HashMap<String, Value>, prefer_human_sender: bool) -> String {
    let display_name = metadata
        .get("agent_display_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let agent_id = metadata
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let from_id = metadata
        .get("from_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let internal_dispatch = metadata
        .get("internal_dispatch")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if prefer_human_sender && !internal_dispatch {
        return "user".to_owned();
    }

    agent_id
        .or(display_name)
        .or(from_id)
        .unwrap_or(if prefer_human_sender { "user" } else { "agent" })
        .to_owned()
}

fn current_turn_timestamp(metadata: &HashMap<String, Value>) -> String {
    if let Some(timestamp) = metadata
        .get(META_CLIENT_TIMESTAMP_LOCAL)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format_group_activity_timestamp(timestamp);
    }
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn current_turn_entry(message: &str, metadata: &HashMap<String, Value>) -> TranscriptEntry {
    TranscriptEntry {
        agent_id: label_from_metadata(metadata, true),
        text: message.to_owned(),
        at: current_turn_timestamp(metadata),
    }
}

fn assistant_turn_entry(agent_id: &str, text: &str) -> TranscriptEntry {
    TranscriptEntry {
        agent_id: agent_id.to_owned(),
        text: text.to_owned(),
        at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

fn assistant_group_message(agent_id: &str, text: &str) -> ProviderMessage {
    let mut metadata = HashMap::new();
    metadata.insert("agent_id".to_owned(), Value::String(agent_id.to_owned()));
    metadata.insert(
        "agent_display_name".to_owned(),
        Value::String(agent_id.to_owned()),
    );

    ProviderMessage {
        metadata,
        ..ProviderMessage::assistant_text(text)
    }
}

fn with_agent_speaker_metadata(mut message: ProviderMessage, agent_id: &str) -> ProviderMessage {
    message
        .metadata
        .entry("agent_id".to_owned())
        .or_insert_with(|| Value::String(agent_id.to_owned()));
    message
        .metadata
        .entry("agent_display_name".to_owned())
        .or_insert_with(|| Value::String(agent_id.to_owned()));
    message
}

/// Meta-provider that orchestrates a Team as a group chat.
///
/// Holds only the three narrow handles it actually needs. All persistence
/// and child-provider routing goes through the injected traits.
pub struct AgentTeamProvider {
    group_store: Arc<dyn GroupStore>,
    team_resolver: Arc<dyn TeamProfileResolver>,
    dispatcher: Arc<dyn SubAgentDispatcher>,
}

impl AgentTeamProvider {
    /// Construct a new provider with the given dependencies.
    pub fn new(
        group_store: Arc<dyn GroupStore>,
        team_resolver: Arc<dyn TeamProfileResolver>,
        dispatcher: Arc<dyn SubAgentDispatcher>,
    ) -> Self {
        Self {
            group_store,
            team_resolver,
            dispatcher,
        }
    }

    /// Read `agent_team_id` out of `metadata`. Returns an `Internal` error
    /// if missing — the dispatch layer is responsible for injecting this;
    /// its absence indicates a wiring bug at the caller, not a user error.
    fn extract_team_id(metadata: &HashMap<String, Value>) -> Result<String, BridgeError> {
        metadata
            .get(META_TEAM_ID)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned())
            .ok_or_else(|| {
                BridgeError::Internal(format!(
                    "{META_TEAM_ID} missing from ProviderRunOptions.metadata"
                ))
            })
    }

    async fn resolve_team(&self, team_id: &str) -> Result<AgentTeamProfile, BridgeError> {
        self.team_resolver
            .resolve_team(team_id)
            .await
            .ok_or_else(|| BridgeError::Internal(format!("team not found: {team_id}")))
    }
}

#[async_trait]
impl AgentLoopProvider for AgentTeamProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::AgentTeam
    }

    fn is_ready(&self) -> bool {
        true
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let started = Instant::now();

        // Step 1: resolve team profile from metadata.
        let team_id = Self::extract_team_id(&options.metadata)?;
        let team = self.resolve_team(&team_id).await?;

        // Step 2: load or hydrate Group state.
        let group_thread_id = options.thread_id.clone();
        let mut group = match self.group_store.load(&group_thread_id).await {
            Some(g) => g,
            None => Group::new(group_thread_id.clone(), team.team_id.clone()),
        };

        // Step 3: materialize the in-memory group transcript for this run.
        // The persisted snapshot omits the live inbound turn, so we append it
        // here before planning/routing. Every dispatched child then sees the
        // unread slice of the group transcript since its last read point.
        let mut transcript = parse_group_transcript(&options.metadata);
        transcript.push(current_turn_entry(&options.message, &options.metadata));

        // Step 4: plan the initial turn. Default no-mention => leader. Later
        // child-to-child routing only fires on explicit mentions in the child
        // reply; plain agent replies do not recursively default back to the
        // leader.
        let plan = plan_turn(&options.message, &team);
        let mut pending_targets: VecDeque<String> = plan.targets.into_iter().collect();

        // Step 5: dispatch targets, appending each child reply as a real group
        // transcript message and queuing any follow-on explicit mentions.
        let mut aggregated_response = String::new();
        let mut any_failure: Option<String> = None;
        let mut aggregate_tokens_in: i64 = 0;
        let mut aggregate_tokens_out: i64 = 0;
        let mut session_messages: Vec<ProviderMessage> = Vec::new();
        let mut dispatch_count = 0usize;
        const MAX_GROUP_DISPATCHES: usize = 16;

        // Wrap on_chunk in an Arc so each per-target forwarding closure can
        // hold its own cheap clone. StreamCallback is `Box<dyn Fn ...>`,
        // which we can share via Arc.
        let on_chunk_shared: Arc<StreamCallback> = Arc::new(on_chunk);

        while let Some(target_agent_id) = pending_targets.pop_front() {
            dispatch_count += 1;
            if dispatch_count > MAX_GROUP_DISPATCHES {
                any_failure.get_or_insert_with(|| {
                    format!(
                        "agent team routing exceeded {MAX_GROUP_DISPATCHES} dispatches in one turn"
                    )
                });
                break;
            }

            // 5a: ensure / reuse child thread.
            let child_thread_id = match group.child_thread(&target_agent_id) {
                Some(existing) => existing.to_owned(),
                None => {
                    let new_id = self
                        .dispatcher
                        .ensure_child_thread(
                            &group_thread_id,
                            &target_agent_id,
                            &team,
                            options.workspace_dir.as_deref(),
                        )
                        .await?;
                    group.record_child_thread(&target_agent_id, &new_id);
                    self.group_store.save(&group).await;
                    new_id
                }
            };

            // 5b+c: build combined message = unread group activity since the
            // child last read the transcript. Any one-time child-thread
            // identity/bootstrap context is injected by the outer dispatcher
            // on first wake-up, not here.
            let offset = group
                .catch_up_offset(&target_agent_id)
                .min(transcript.len());
            let unread_entries = transcript[offset..]
                .iter()
                .filter(|entry| entry.agent_id != target_agent_id)
                .cloned()
                .collect::<Vec<_>>();
            let combined_message = build_combined_message(&unread_entries, "");

            // 5d: forward the child's stream back to the caller. We prefix
            // only the FIRST delta of each assistant segment with
            // `[agent_id] ` so the live stream and persisted group transcript
            // keep attribution without duplicating the label on every chunk.
            let target_label = target_agent_id.clone();
            let forwarder = Arc::clone(&on_chunk_shared);
            let needs_prefix = Arc::new(AtomicBool::new(true));
            let forwarding_callback: StreamCallback = Box::new(move |event| match event {
                StreamEvent::Delta { text } => {
                    if text.is_empty() {
                        return;
                    }
                    let text = if needs_prefix.swap(false, Ordering::Relaxed) {
                        format!("[{}] {}", target_label, text)
                    } else {
                        text
                    };
                    forwarder(StreamEvent::Delta { text });
                }
                StreamEvent::Boundary {
                    kind,
                    pending_input_id,
                } => {
                    needs_prefix.store(true, Ordering::Relaxed);
                    forwarder(StreamEvent::Boundary {
                        kind,
                        pending_input_id,
                    });
                }
                StreamEvent::ToolUse { message } => {
                    needs_prefix.store(true, Ordering::Relaxed);
                    forwarder(StreamEvent::ToolUse {
                        message: with_agent_speaker_metadata(message, &target_label),
                    });
                }
                StreamEvent::ToolResult { message } => {
                    needs_prefix.store(true, Ordering::Relaxed);
                    forwarder(StreamEvent::ToolResult {
                        message: with_agent_speaker_metadata(message, &target_label),
                    });
                }
                StreamEvent::Done => {
                    // Suppress per-child Done; the provider emits one at the
                    // end of the whole group turn.
                }
                StreamEvent::ThreadTitleUpdated { .. } => {
                    // Child thread titles are managed on child threads; do
                    // not surface them as group-thread title updates.
                }
            });

            let mut child_options = options.clone();
            child_options.thread_id = child_thread_id.clone();
            child_options.message = combined_message;

            let child_completed_ok = match self
                .dispatcher
                .run_child_streaming(&child_thread_id, &child_options, forwarding_callback)
                .await
            {
                Ok(res) => {
                    if !res.response.trim().is_empty() {
                        if !aggregated_response.is_empty() {
                            aggregated_response.push('\n');
                        }
                        aggregated_response
                            .push_str(&format!("[{}] {}", target_agent_id, res.response));

                        transcript.push(assistant_turn_entry(&target_agent_id, &res.response));
                        session_messages
                            .push(assistant_group_message(&target_agent_id, &res.response));

                        let follow_up = plan_turn(&res.response, &team);
                        if follow_up.from_explicit_mentions {
                            pending_targets.extend(follow_up.targets);
                        }
                    }
                    aggregate_tokens_in += res.input_tokens;
                    aggregate_tokens_out += res.output_tokens;
                    if !res.success {
                        let label = res
                            .error
                            .clone()
                            .unwrap_or_else(|| format!("{target_agent_id} failed"));
                        any_failure.get_or_insert_with(|| format!("{target_agent_id}: {label}"));
                    }
                    true
                }
                Err(err) => {
                    any_failure.get_or_insert_with(|| format!("{target_agent_id}: {err}"));
                    false
                }
            };

            if child_completed_ok {
                group.advance_catch_up(&target_agent_id, transcript.len());
                self.group_store.save(&group).await;
            }
        }

        // Emit a single terminal Done once the whole group turn is done.
        on_chunk_shared(StreamEvent::Done);

        let run_id = format!("agent_team_run_{}", uuid::Uuid::new_v4());
        let duration_ms = started.elapsed().as_millis() as i64;
        Ok(ProviderRunResult {
            run_id,
            thread_id: group_thread_id,
            response: aggregated_response,
            session_messages,
            sdk_session_id: None,
            actual_model: Some("agent_team".to_owned()),
            thread_title: None,
            success: any_failure.is_none(),
            error: any_failure,
            input_tokens: aggregate_tokens_in,
            output_tokens: aggregate_tokens_out,
            cost: 0.0,
            duration_ms,
        })
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        // AgentTeam has no backend SDK session; the group thread id IS the
        // only identity we need.
        Ok(thread_id.to_owned())
    }

    async fn clear_session(&self, _thread_id: &str) -> bool {
        // No backend session to clear. Returning true matches the default,
        // spelled out explicitly for intent.
        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
