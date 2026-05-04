//! Integration tests for [`AgentTeamProvider`].
//!
//! These tests layer a real `AgentLoopProvider` (the in-module
//! [`RecordingProvider`]) underneath a test-only [`RecordingDispatcher`],
//! so the full stack — provider → dispatcher → child `AgentLoopProvider` —
//! is exercised end-to-end against a real [`FileGroupStore`] writing to a
//! [`TempDir`].
//!
//! The existing `provider::tests` module uses a flat `MockDispatcher` and
//! never goes through an `AgentLoopProvider`; these tests intentionally
//! complement (not duplicate) that coverage by proving that child turns
//! really do drive a sub-provider's `run_streaming` and that its responses
//! propagate back to the aggregated [`ProviderRunResult`].
//!
//! Scenarios covered:
//!
//! 1. Default route to leader creates the leader's child thread, dispatches
//!    the raw live turn, emits exactly one terminal `Done`, and surfaces a
//!    successful [`ProviderRunResult`].
//! 2. Explicit `@[Coder](coder)` routes only to coder — leader is never
//!    dispatched.
//! 3. Re-mentioning the same sub-agent reuses the previously allocated child
//!    thread; `ensure_child_thread` fires exactly once across both turns.
//! 4. Catch-up math: after the child has already seen `n` entries, a later
//!    turn with a snapshot of length `m > n` must deliver exactly
//!    `transcript[n..m]` as envelopes plus the new live turn — and never
//!    re-deliver the pre-offset entries.
//! 5. Multi-mention fans out to both targets, aggregates their responses and
//!    token counts, and still emits exactly one terminal `Done`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Mutex as TokioMutex;

use garyx_models::AgentTeamProfile;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent};

use super::dispatcher::{SubAgentDispatcher, TeamProfileResolver};
use super::provider::AgentTeamProvider;
use super::store::{FileGroupStore, GroupStore};
use crate::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};

// ---------------------------------------------------------------------------
// Metadata key mirrors — the provider hides these behind private consts, so
// we re-declare them here rather than poking at provider internals. If the
// provider's keys ever change, these tests will fail the scenario assertions
// loudly, which is the desired signal.
// ---------------------------------------------------------------------------

const META_TEAM_ID: &str = "agent_team_id";
const META_TRANSCRIPT_SNAPSHOT: &str = "group_transcript_snapshot";

// ---------------------------------------------------------------------------
// Test-only `TeamProfileResolver` stub.
// ---------------------------------------------------------------------------

struct StubResolver {
    team: AgentTeamProfile,
}

#[async_trait]
impl TeamProfileResolver for StubResolver {
    async fn resolve_team(&self, team_id: &str) -> Option<AgentTeamProfile> {
        if team_id == self.team.team_id {
            Some(self.team.clone())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// RecordingProvider — a real `AgentLoopProvider` that records every call and
// emits one `Delta` + one `Done` per run. One instance per sub-agent so the
// responses are distinguishable ("resp:{agent_id}:{counter}").
//
// Pattern adapted from `garyx-gateway/src/internal_inbound.rs` tests.
// ---------------------------------------------------------------------------

/// One recorded invocation of [`RecordingProvider::run_streaming`]. The
/// `metadata` field is captured for parity with the reference pattern in
/// `internal_inbound.rs` tests and to aid debugging when assertions fail,
/// even when a given test does not assert on it directly.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct RecordedCall {
    thread_id: String,
    message: String,
    metadata: HashMap<String, Value>,
}

struct RecordingProvider {
    /// Which sub-agent this provider is "playing as". Used to tag responses
    /// and deltas.
    agent_id: String,
    calls: StdMutex<Vec<RecordedCall>>,
    /// Per-instance monotonic counter so successive runs produce distinct
    /// response strings ("resp:coder:1", "resp:coder:2", …).
    counter: StdMutex<u32>,
}

impl RecordingProvider {
    fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            calls: StdMutex::new(Vec::new()),
            counter: StdMutex::new(0),
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl AgentLoopProvider for RecordingProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
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
        let next_counter = {
            let mut guard = self.counter.lock().unwrap();
            *guard += 1;
            *guard
        };

        self.calls.lock().unwrap().push(RecordedCall {
            thread_id: options.thread_id.clone(),
            message: options.message.clone(),
            metadata: options.metadata.clone(),
        });

        let response_body = format!("resp:{}:{}", self.agent_id, next_counter);
        on_chunk(StreamEvent::Delta {
            text: response_body.clone(),
        });
        on_chunk(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id: format!("rec-run-{}-{}", self.agent_id, next_counter),
            thread_id: options.thread_id.clone(),
            response: response_body,
            session_messages: Vec::new(),
            sdk_session_id: None,
            actual_model: None,
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 2,
            cost: 0.0,
            duration_ms: 0,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }
}

// ---------------------------------------------------------------------------
// RecordingDispatcher — routes child runs to a per-agent RecordingProvider.
// Derives the child_agent_id from the child_thread_id using the allocation
// convention `th::child-{agent_id}-{NNNN}`.
// ---------------------------------------------------------------------------

struct RecordingDispatcher {
    /// agent_id → provider pretending to be that sub-agent.
    providers: HashMap<String, Arc<RecordingProvider>>,
    /// (group_thread_id, child_agent_id, workspace_path) tuples, in call order.
    ensure_calls: TokioMutex<Vec<(String, String, Option<String>)>>,
    /// agent_id → allocated child_thread_id.
    allocated: StdMutex<HashMap<String, String>>,
    /// Monotonic allocator for unique thread ids.
    next_id: StdMutex<u32>,
}

impl RecordingDispatcher {
    fn new(providers: HashMap<String, Arc<RecordingProvider>>) -> Self {
        Self {
            providers,
            ensure_calls: TokioMutex::new(Vec::new()),
            allocated: StdMutex::new(HashMap::new()),
            next_id: StdMutex::new(0),
        }
    }

    /// Strip `"th::child-"` prefix and the trailing `"-NNNN"` counter to
    /// recover the child_agent_id. Mirrors the allocation format below.
    fn agent_id_from_thread(child_thread_id: &str) -> Option<String> {
        let tail = child_thread_id.strip_prefix("th::child-")?;
        // Trailing "-NNNN" — drop from the last '-' onward.
        let dash = tail.rfind('-')?;
        Some(tail[..dash].to_owned())
    }
}

#[async_trait]
impl SubAgentDispatcher for RecordingDispatcher {
    async fn ensure_child_thread(
        &self,
        group_thread_id: &str,
        child_agent_id: &str,
        _team: &AgentTeamProfile,
        workspace_path: Option<&str>,
    ) -> Result<String, BridgeError> {
        self.ensure_calls.lock().await.push((
            group_thread_id.to_owned(),
            child_agent_id.to_owned(),
            workspace_path.map(str::to_owned),
        ));

        // Reuse if we've already allocated for this agent — the provider is
        // *supposed* to cache in `Group`, but we defend against double-calls
        // so the error surface is clear if the provider ever regresses.
        {
            let allocated = self.allocated.lock().unwrap();
            if let Some(existing) = allocated.get(child_agent_id) {
                return Ok(existing.clone());
            }
        }

        let counter = {
            let mut guard = self.next_id.lock().unwrap();
            *guard += 1;
            *guard
        };
        let new_id = format!("th::child-{}-{:04}", child_agent_id, counter);
        self.allocated
            .lock()
            .unwrap()
            .insert(child_agent_id.to_owned(), new_id.clone());
        Ok(new_id)
    }

    async fn run_child_streaming(
        &self,
        child_thread_id: &str,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let agent_id = Self::agent_id_from_thread(child_thread_id).ok_or_else(|| {
            BridgeError::Internal(format!(
                "cannot derive agent_id from child_thread_id {child_thread_id}"
            ))
        })?;

        let provider = self.providers.get(&agent_id).ok_or_else(|| {
            BridgeError::Internal(format!(
                "no RecordingProvider registered for agent_id {agent_id}"
            ))
        })?;

        provider.run_streaming(options, on_chunk).await
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn make_team(team_id: &str, leader: &str, members: &[&str]) -> AgentTeamProfile {
    AgentTeamProfile {
        team_id: team_id.to_owned(),
        display_name: "Demo".to_owned(),
        leader_agent_id: leader.to_owned(),
        member_agent_ids: members.iter().map(|s| (*s).to_owned()).collect(),
        workflow_text: String::new(),
        created_at: "2026-04-19T00:00:00Z".to_owned(),
        updated_at: "2026-04-19T00:00:00Z".to_owned(),
    }
}

fn base_options(thread_id: &str, message: &str, team_id: &str) -> ProviderRunOptions {
    let mut metadata = HashMap::new();
    metadata.insert(META_TEAM_ID.to_owned(), json!(team_id));
    ProviderRunOptions {
        thread_id: thread_id.to_owned(),
        message: message.to_owned(),
        workspace_dir: Some("/workspace".to_owned()),
        images: None,
        metadata,
    }
}

fn recording_callback() -> (StreamCallback, Arc<StdMutex<Vec<StreamEvent>>>) {
    let log: Arc<StdMutex<Vec<StreamEvent>>> = Arc::new(StdMutex::new(Vec::new()));
    let log_for_cb = Arc::clone(&log);
    let cb: StreamCallback = Box::new(move |event| {
        log_for_cb.lock().unwrap().push(event);
    });
    (cb, log)
}

/// Count `StreamEvent::Done` occurrences in a captured log.
fn count_done(events: &[StreamEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, StreamEvent::Done))
        .count()
}

/// Collect the text of every `Delta` event in a captured log.
fn collect_deltas(events: &[StreamEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Delta { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Build an [`AgentTeamProvider`] wired up with a real [`FileGroupStore`]
/// under `tmp`, a stubbed resolver, and a [`RecordingDispatcher`] that
/// routes to one [`RecordingProvider`] per agent_id listed in `agent_ids`.
fn build_rig(
    team: AgentTeamProfile,
    agent_ids: &[&str],
    tmp: &TempDir,
) -> (
    AgentTeamProvider,
    Arc<RecordingDispatcher>,
    HashMap<String, Arc<RecordingProvider>>,
    Arc<FileGroupStore>,
) {
    let mut providers: HashMap<String, Arc<RecordingProvider>> = HashMap::new();
    for id in agent_ids {
        providers.insert((*id).to_owned(), Arc::new(RecordingProvider::new(*id)));
    }

    let group_store = Arc::new(FileGroupStore::new(tmp.path().to_path_buf()));
    let resolver = Arc::new(StubResolver { team });
    let dispatcher = Arc::new(RecordingDispatcher::new(providers.clone()));

    let provider = AgentTeamProvider::new(
        Arc::clone(&group_store) as Arc<dyn GroupStore>,
        resolver,
        Arc::clone(&dispatcher) as Arc<dyn SubAgentDispatcher>,
    );
    (provider, dispatcher, providers, group_store)
}

// ---------------------------------------------------------------------------
// 1. Default route to leader creates child thread on first message.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_route_creates_leader_child_and_dispatches_group_activity() {
    let tmp = TempDir::new().unwrap();
    let team = make_team("team::demo", "leader", &["coder"]);
    let (provider, dispatcher, providers, store) = build_rig(team, &["leader", "coder"], &tmp);

    let options = base_options("th::group", "hi team", "team::demo");
    let (cb, log) = recording_callback();

    let result = provider.run_streaming(&options, cb).await.unwrap();

    // Aggregate result reflects the leader's single recorded run.
    assert!(result.success);
    assert_eq!(result.error, None);
    assert_eq!(result.response, "[leader] resp:leader:1");
    assert_eq!(result.input_tokens, 1);
    assert_eq!(result.output_tokens, 2);

    // Exactly one ensure_child_thread call, for the leader, carrying the
    // parent workspace path.
    let ensures = dispatcher.ensure_calls.lock().await.clone();
    assert_eq!(ensures.len(), 1);
    assert_eq!(ensures[0].0, "th::group");
    assert_eq!(ensures[0].1, "leader");
    assert_eq!(ensures[0].2.as_deref(), Some("/workspace"));

    // The underlying RecordingProvider for the leader sees only the unread
    // group transcript slice. One-time team bootstrap context is injected by
    // the outer gateway dispatcher on first wake-up, not by the bridge.
    let leader_calls = providers.get("leader").unwrap().calls();
    assert_eq!(leader_calls.len(), 1);
    let msg = &leader_calls[0].message;
    assert!(
        msg.starts_with("<group_activity from=\"user\""),
        "leader must receive unread group activity, got:\n{msg}"
    );
    assert!(
        msg.contains("<group_activity from=\"user\""),
        "current live turn must be delivered as group activity, msg:\n{msg}"
    );
    assert!(leader_calls[0].thread_id.starts_with("th::child-leader-"));

    // The coder provider was never touched.
    assert_eq!(providers.get("coder").unwrap().calls().len(), 0);

    // Stream observer saw exactly one terminal Done.
    let events = log.lock().unwrap().clone();
    assert_eq!(count_done(&events), 1);
    // Terminal event is Done (no deltas after it).
    assert_eq!(events.last(), Some(&StreamEvent::Done));

    // Delta was prefixed with "[leader] ".
    let deltas = collect_deltas(&events);
    assert_eq!(deltas, vec!["[leader] resp:leader:1".to_owned()]);

    // Group persisted: the leader's child thread and its advanced offset.
    let persisted = store.load("th::group").await.expect("group must persist");
    assert_eq!(persisted.team_id, "team::demo");
    assert_eq!(
        persisted.child_thread("leader").map(str::to_owned),
        Some(leader_calls[0].thread_id.clone())
    );
    // Offset advances to the full transcript length observed by the leader:
    // current user turn + leader reply.
    assert_eq!(persisted.catch_up_offset("leader"), 2);
}

// ---------------------------------------------------------------------------
// 2. Explicit `@[Coder](coder)` routes only to coder.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn explicit_mention_dispatches_only_to_mentioned_agent() {
    let tmp = TempDir::new().unwrap();
    let team = make_team("team::demo", "leader", &["coder", "planner"]);
    let (provider, dispatcher, providers, _store) =
        build_rig(team, &["leader", "coder", "planner"], &tmp);

    let options = base_options("th::group", "@[Coder](coder) ship it", "team::demo");
    let (cb, log) = recording_callback();

    let result = provider.run_streaming(&options, cb).await.unwrap();

    assert!(result.success);
    assert_eq!(result.response, "[coder] resp:coder:1");

    // Exactly one ensure, for coder — leader is never allocated.
    let ensures = dispatcher.ensure_calls.lock().await.clone();
    assert_eq!(ensures.len(), 1);
    assert_eq!(ensures[0].1, "coder");

    // Only coder ran; leader and planner untouched.
    assert_eq!(providers.get("coder").unwrap().calls().len(), 1);
    assert_eq!(providers.get("leader").unwrap().calls().len(), 0);
    assert_eq!(providers.get("planner").unwrap().calls().len(), 0);

    // Exactly one terminal Done; delta prefixed with "[coder] ".
    let events = log.lock().unwrap().clone();
    assert_eq!(count_done(&events), 1);
    assert_eq!(
        collect_deltas(&events),
        vec!["[coder] resp:coder:1".to_owned()]
    );
}

// ---------------------------------------------------------------------------
// 3. Second @ to same sub-agent reuses existing child thread.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_mention_reuses_existing_child_thread() {
    let tmp = TempDir::new().unwrap();
    let team = make_team("team::demo", "leader", &["coder"]);
    let (provider, dispatcher, providers, store) = build_rig(team, &["leader", "coder"], &tmp);

    let opts1 = base_options("th::group", "@[Coder](coder) first", "team::demo");
    let (cb1, _log1) = recording_callback();
    let r1 = provider.run_streaming(&opts1, cb1).await.unwrap();
    assert!(r1.success);

    let opts2 = base_options("th::group", "@[Coder](coder) second", "team::demo");
    let (cb2, _log2) = recording_callback();
    let r2 = provider.run_streaming(&opts2, cb2).await.unwrap();
    assert!(r2.success);

    // ensure_child_thread was called exactly once across both turns.
    let ensures = dispatcher.ensure_calls.lock().await.clone();
    assert_eq!(
        ensures.len(),
        1,
        "ensure_child_thread must only fire on the first dispatch, got: {ensures:?}"
    );
    assert_eq!(ensures[0].1, "coder");

    // Both runs landed on the same child thread id.
    let coder_calls = providers.get("coder").unwrap().calls();
    assert_eq!(coder_calls.len(), 2);
    assert_eq!(coder_calls[0].thread_id, coder_calls[1].thread_id);

    // Counter visible in the aggregated responses confirms ordering.
    assert_eq!(r1.response, "[coder] resp:coder:1");
    assert_eq!(r2.response, "[coder] resp:coder:2");

    // Persisted Group reflects the single allocation.
    let persisted = store.load("th::group").await.unwrap();
    assert_eq!(
        persisted.child_thread("coder").map(str::to_owned),
        Some(coder_calls[0].thread_id.clone())
    );
}

// ---------------------------------------------------------------------------
// 4. Catch-up after intervening turns.
//
// Turn 1: `@[Coder](coder) first` with empty snapshot → coder sees the
//         current user turn as one group_activity envelope. Offset advances
//         past both the user turn and coder reply.
// Turn 2: (hand-crafted) `@[Coder](coder) next` with snapshot of 4 entries.
//         Coder's pre-offset is 2, so it sees only entries 2..4 plus the
//         current live user turn. After, offset advances to the full
//         in-memory transcript length for that dispatch.
//
// Rationale: this test simulates the snapshot by hand, so the cleanest proof
// of catch-up math is to jump an offset delta of exactly 3+ in one step and
// assert the envelope slice is precisely `[offset..len)`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn catch_up_delivers_only_new_entries_and_advances_offset() {
    let tmp = TempDir::new().unwrap();
    let team = make_team("team::demo", "leader", &["coder"]);
    let (provider, _dispatcher, providers, store) = build_rig(team, &["leader", "coder"], &tmp);

    // Turn 1: empty transcript snapshot → current user turn arrives as one
    // group_activity envelope.
    let opts1 = base_options("th::group", "@[Coder](coder) first", "team::demo");
    let (cb1, log1) = recording_callback();
    provider.run_streaming(&opts1, cb1).await.unwrap();

    {
        let coder_calls = providers.get("coder").unwrap().calls();
        assert_eq!(coder_calls.len(), 1);
        let msg = &coder_calls[0].message;
        assert!(
            msg.starts_with("<group_activity from=\"user\""),
            "current unread activity must come first, got:\n{msg}"
        );
        assert!(
            msg.contains("<group_activity from=\"user\"") && msg.contains("@[Coder](coder) first"),
            "current user turn must be wrapped as group activity, msg:\n{msg}"
        );

        let events = log1.lock().unwrap().clone();
        assert_eq!(count_done(&events), 1);
    }

    // Offset advances to 2: current user turn + coder reply.
    let g_after_1 = store.load("th::group").await.unwrap();
    assert_eq!(g_after_1.catch_up_offset("coder"), 2);

    // Turn 2: hand-craft a snapshot of 4 entries. Coder's offset = 2, so only
    // entries 2..3 plus the current user turn are unread.
    let mut opts2 = base_options("th::group", "@[Coder](coder) next", "team::demo");
    let snapshot = json!([
        {"agent_id": "user",   "text": "entry-0", "at": "t0"},
        {"agent_id": "leader", "text": "entry-1", "at": "t1"},
        {"agent_id": "user",   "text": "entry-2", "at": "t2"},
        {"agent_id": "leader", "text": "entry-3", "at": "t3"},
    ]);
    opts2
        .metadata
        .insert(META_TRANSCRIPT_SNAPSHOT.to_owned(), snapshot);
    let (cb2, _log2) = recording_callback();
    provider.run_streaming(&opts2, cb2).await.unwrap();

    let coder_calls = providers.get("coder").unwrap().calls();
    assert_eq!(coder_calls.len(), 2);
    let msg2 = &coder_calls[1].message;

    // Only entries 2 and 3 remain unread from the persisted snapshot.
    for idx in 2..4 {
        assert!(
            msg2.contains(&format!("entry-{idx}")),
            "catch-up message must include entry-{idx}:\n{msg2}"
        );
    }
    for idx in 0..2 {
        assert!(
            !msg2.contains(&format!("entry-{idx}")),
            "already-read entry-{idx} must not be re-delivered:\n{msg2}"
        );
    }
    // Envelope structure: from="..." at="..." for each entry.
    assert!(
        msg2.contains("<group_activity from=\"user\" at=\"t2\">"),
        "envelope format mismatch:\n{msg2}"
    );
    // Current live turn is also wrapped as group activity.
    assert!(
        msg2.contains("@[Coder](coder) next"),
        "current turn must be present in the unread group activity batch:\n{msg2}"
    );

    // Offset advanced to the full transcript length seen by the child:
    // 4 snapshot entries + current turn + coder reply.
    let g_after_2 = store.load("th::group").await.unwrap();
    assert_eq!(g_after_2.catch_up_offset("coder"), 6);

    // Third turn: snapshot grows to 7. Coder's offset = 6, so only the newest
    // unread snapshot entry plus the current turn should appear.
    let mut opts3 = base_options("th::group", "@[Coder](coder) again", "team::demo");
    let snapshot3 = json!([
        {"agent_id": "user",   "text": "entry-0", "at": "t0"},
        {"agent_id": "leader", "text": "entry-1", "at": "t1"},
        {"agent_id": "user",   "text": "entry-2", "at": "t2"},
        {"agent_id": "leader", "text": "entry-3", "at": "t3"},
        {"agent_id": "coder",  "text": "entry-4", "at": "t4"},
        {"agent_id": "user",   "text": "entry-5", "at": "t5"},
        {"agent_id": "leader", "text": "entry-6", "at": "t6"},
    ]);
    opts3
        .metadata
        .insert(META_TRANSCRIPT_SNAPSHOT.to_owned(), snapshot3);
    let (cb3, _) = recording_callback();
    provider.run_streaming(&opts3, cb3).await.unwrap();

    let coder_calls = providers.get("coder").unwrap().calls();
    assert_eq!(coder_calls.len(), 3);
    let msg3 = &coder_calls[2].message;

    // Only entry-6 remains unread from the persisted snapshot.
    {
        let idx = 6;
        assert!(
            msg3.contains(&format!("entry-{idx}")),
            "turn-3 message must include entry-{idx}:\n{msg3}"
        );
    }
    for idx in 0..=5 {
        assert!(
            !msg3.contains(&format!("entry-{idx}")),
            "turn-3 message must not re-deliver entry-{idx}:\n{msg3}"
        );
    }
    assert!(msg3.contains("@[Coder](coder) again"));

    let pos6 = msg3.find("entry-6").unwrap();
    let pos_live = msg3.find("@[Coder](coder) again").unwrap();
    assert!(
        pos6 < pos_live,
        "unread snapshot entries must precede the live turn"
    );

    // Offset advanced to the full in-memory transcript length:
    // 7 snapshot entries + current turn + coder reply.
    let g_after_3 = store.load("th::group").await.unwrap();
    assert_eq!(g_after_3.catch_up_offset("coder"), 9);
}

// ---------------------------------------------------------------------------
// 5. Multi-mention fans out and aggregates.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_mention_fans_out_and_aggregates_results() {
    let tmp = TempDir::new().unwrap();
    let team = make_team("team::demo", "leader", &["a", "b"]);
    let (provider, dispatcher, providers, store) = build_rig(team, &["leader", "a", "b"], &tmp);

    let options = base_options("th::group", "@[A](a) @[B](b) do it", "team::demo");
    let (cb, log) = recording_callback();
    let result = provider.run_streaming(&options, cb).await.unwrap();

    assert!(result.success);
    assert_eq!(result.error, None);

    // Both ensure_child_thread calls fired, in document order.
    let ensures = dispatcher.ensure_calls.lock().await.clone();
    assert_eq!(ensures.len(), 2);
    assert_eq!(ensures[0].1, "a");
    assert_eq!(ensures[1].1, "b");

    // Each underlying RecordingProvider ran exactly once.
    let a_calls = providers.get("a").unwrap().calls();
    let b_calls = providers.get("b").unwrap().calls();
    assert_eq!(a_calls.len(), 1);
    assert_eq!(b_calls.len(), 1);
    assert!(a_calls[0].thread_id.starts_with("th::child-a-"));
    assert!(b_calls[0].thread_id.starts_with("th::child-b-"));

    // Aggregated response contains BOTH child responses.
    assert!(
        result.response.contains("resp:a:1") && result.response.contains("resp:b:1"),
        "aggregated response missing a child contribution: {:?}",
        result.response
    );

    // Token counts summed across children (1 + 1 in; 2 + 2 out).
    assert_eq!(result.input_tokens, 2);
    assert_eq!(result.output_tokens, 4);

    // Stream observer saw EXACTLY one terminal Done (per-child Done
    // suppressed by the provider).
    let events = log.lock().unwrap().clone();
    assert_eq!(
        count_done(&events),
        1,
        "provider must emit exactly one terminal Done, got events: {events:?}"
    );
    // Both prefixed deltas present, in dispatch order.
    let deltas = collect_deltas(&events);
    assert_eq!(
        deltas,
        vec!["[a] resp:a:1".to_owned(), "[b] resp:b:1".to_owned()]
    );

    // Both child threads persisted under the Group.
    let persisted = store.load("th::group").await.unwrap();
    assert_eq!(
        persisted.child_thread("a").map(str::to_owned),
        Some(a_calls[0].thread_id.clone())
    );
    assert_eq!(
        persisted.child_thread("b").map(str::to_owned),
        Some(b_calls[0].thread_id.clone())
    );
}
