use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::transcript_kind::{
    is_control_message, is_tool_related_message, is_tool_result_trace,
    resolve_message_kind_for_object, tool_call_id,
};
use crate::transcript_run_state::{
    TranscriptRunActivity, TranscriptRunState, reduce_transcript_run_state,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderSnapshot {
    pub based_on_seq: u64,
    pub rows: Vec<RenderRow>,
    #[serde(rename = "tailActivity")]
    pub tail_activity: RenderTailActivity,
    #[serde(rename = "activeToolGroupId")]
    pub active_tool_group_id: Option<String>,
    pub progress_locus: RenderProgressLocus,
    pub filtered_placeholders: Vec<RenderFilteredPlaceholder>,
    /// Present when the active run terminated because the provider's rolling
    /// usage quota was exhausted. Clients render a banner + live countdown to
    /// `reset_at` and surface whether an automatic resend is scheduled.
    #[serde(rename = "rateLimit", default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RenderRateLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<RenderWindow>,
    /// Combined hash over the per-row structural hashes in `rows` order
    /// (#TASK-1956 knife 1). The server is the only hasher: clients treat
    /// this as an opaque token and compare it by equality against
    /// `RenderDelta.from_rows_hash` to keep the delta chain honest. In Rust
    /// this is a `u64`; on the wire it is a decimal STRING because u64
    /// exceeds JavaScript's 2^53 safe-integer range. `None` (absent on the
    /// wire) on connections that did not declare `render_mode=delta`, which
    /// keeps undeclared frames byte-identical.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rows_hash_token"
    )]
    pub rows_hash: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderWindow {
    pub floor_seq: u64,
    pub has_more_above: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderRateLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(rename = "resetAt", default, skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "willAutoResend", default)]
    pub will_auto_resend: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderTailActivity {
    None,
    Thinking,
    AssistantStreaming,
    ToolActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderProgressLocus {
    None,
    Tail,
    ToolGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderRow {
    UserTurn(RenderUserTurnRow),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderCapsuleCard {
    pub id: String,
    pub capsule_id: String,
    pub title: String,
    pub revision: i64,
    pub action: RenderCapsuleAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderCapsuleAction {
    Created,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderUserTurnRow {
    pub id: String,
    pub user: Option<RenderMessageRef>,
    pub activity: Vec<RenderActivityRow>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capsule_cards: Vec<RenderCapsuleCard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderActivityRow {
    AssistantReply(RenderAssistantReplyRow),
    Step(RenderStepRow),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderAssistantReplyRow {
    pub id: String,
    pub message: RenderMessageRef,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderStepRow {
    pub id: String,
    pub steps: Vec<RenderStepItem>,
    pub final_message: Option<RenderMessageRef>,
    pub running: bool,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderStepItem {
    AssistantMessage(RenderAssistantStep),
    ToolGroup(RenderToolGroup),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderAssistantStep {
    pub id: String,
    pub message: RenderMessageRef,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderToolGroup {
    pub id: String,
    pub status: RenderToolGroupStatus,
    pub entries: Vec<RenderToolEntry>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolGroupStatus {
    Active,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderToolEntry {
    pub id: String,
    pub tool_use_id: Option<String>,
    pub status: RenderToolEntryStatus,
    pub tool_use: Option<RenderMessageRef>,
    pub tool_result: Option<RenderMessageRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolEntryStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderMessageRef {
    pub id: String,
    pub seq: u64,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderFilteredPlaceholder {
    pub message: RenderMessageRef,
    pub reason: RenderPlaceholderFilterReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderPlaceholderFilterReason {
    EmptyStreamingAssistant,
}

/// Serde codec for `RenderSnapshot.rows_hash`: `u64` in Rust, decimal
/// string on the wire (u64 exceeds JS's 2^53 safe-integer range; the
/// contract type is an opaque string token).
mod rows_hash_token {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        value.map(|hash| hash.to_string()).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        let token = Option::<String>::deserialize(deserializer)?;
        token
            .map(|token| token.parse::<u64>().map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// Incremental live-frame payload (#TASK-1956 knife 1). Scalar fields are
/// always sent whole; rows travel as the full id order plus the bodies of
/// new/changed rows only. `from_rows_hash`/`rows_hash` chain consecutive
/// frames: the server is the only hasher, clients compare the opaque
/// tokens by equality and take the gap path on any mismatch. Serde naming
/// is aligned with `RenderSnapshot` field for field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderDelta {
    /// The client must hold the snapshot at this seq...
    pub from_seq: u64,
    /// ...with exactly this rows content (drift tripwire).
    pub from_rows_hash: String,
    pub based_on_seq: u64,
    /// Combined rows hash AFTER applying this delta; the client stores it
    /// as its new chain token on accept.
    pub rows_hash: String,
    /// Full row id sequence: re-order is unambiguous.
    pub row_order: Vec<String>,
    /// Full bodies for new/changed rows only.
    pub upsert_rows: Vec<RenderRow>,
    #[serde(rename = "tailActivity")]
    pub tail_activity: RenderTailActivity,
    #[serde(rename = "activeToolGroupId")]
    pub active_tool_group_id: Option<String>,
    pub progress_locus: RenderProgressLocus,
    #[serde(rename = "rateLimit", default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RenderRateLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<RenderWindow>,
    pub filtered_placeholders: Vec<RenderFilteredPlaceholder>,
}

/// Why `apply_render_delta` rejected a delta. Every variant is a protocol
/// violation on the receiving side: the consumer discards the frame and
/// enters its existing gap path (reconnect + authoritative refetch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderDeltaError {
    /// `delta.from_seq` does not match the held snapshot's `based_on_seq`.
    FromSeqMismatch {
        delta_from_seq: u64,
        prev_based_on_seq: u64,
    },
    /// `delta.from_rows_hash` does not match the held rows-hash token:
    /// the delta base drifted (same-seq drops, guard interactions, bugs).
    FromRowsHashMismatch {
        delta_from_rows_hash: String,
        prev_rows_hash: String,
    },
    /// An `upsert_rows` entry's id appears more than once.
    DuplicateUpsertRow { row_id: String },
    /// An `upsert_rows` entry's id is absent from `row_order`: stray
    /// upserts are a producer/consumer disagreement, not ignorable padding.
    UnexpectedUpsertRow { row_id: String },
    /// A `row_order` id resolves to neither `upsert_rows` nor the held
    /// snapshot's rows.
    MissingRow { row_id: String },
    /// The reassembled rows do not hash to `delta.rows_hash`: the chain is
    /// broken even though every id resolved.
    RowsHashMismatch {
        delta_rows_hash: String,
        reassembled_rows_hash: String,
    },
}

impl std::fmt::Display for RenderDeltaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FromSeqMismatch {
                delta_from_seq,
                prev_based_on_seq,
            } => write!(
                f,
                "render delta from_seq {delta_from_seq} does not match held snapshot seq {prev_based_on_seq}"
            ),
            Self::FromRowsHashMismatch {
                delta_from_rows_hash,
                prev_rows_hash,
            } => write!(
                f,
                "render delta from_rows_hash {delta_from_rows_hash} does not match held rows hash {prev_rows_hash}"
            ),
            Self::DuplicateUpsertRow { row_id } => {
                write!(f, "render delta upsert row id {row_id} appears more than once")
            }
            Self::UnexpectedUpsertRow { row_id } => {
                write!(
                    f,
                    "render delta upsert row id {row_id} is absent from row_order"
                )
            }
            Self::MissingRow { row_id } => {
                write!(
                    f,
                    "render delta row id {row_id} missing from upsert rows and held snapshot"
                )
            }
            Self::RowsHashMismatch {
                delta_rows_hash,
                reassembled_rows_hash,
            } => write!(
                f,
                "render delta rows_hash {delta_rows_hash} does not match reassembled rows hash {reassembled_rows_hash}"
            ),
        }
    }
}

impl std::error::Error for RenderDeltaError {}

/// Per-row structural hashes (keyed by row id) plus the combined rows hash
/// for one snapshot's rows. This is what a delta-mode connection caches
/// per frame instead of the full previous snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRowsDigest {
    pub rows_hash: u64,
    pub row_hashes: HashMap<String, u64>,
}

pub fn render_row_id(row: &RenderRow) -> &str {
    match row {
        RenderRow::UserTurn(turn) => &turn.id,
    }
}

/// Structural hash of one row. The algorithm is a server implementation
/// detail: tokens never leave one server process's lifetime (every new
/// connection reseeds from a full frame), so cross-version stability is
/// not required — only in-process determinism.
pub fn render_row_hash(row: &RenderRow) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    row.hash(&mut hasher);
    hasher.finish()
}

/// One pass over `rows`: per-row hashes by id plus the combined hash over
/// the per-row hashes in row order (row count included, so truncation and
/// reorder both change the token).
pub fn render_rows_digest(rows: &[RenderRow]) -> RenderRowsDigest {
    let mut combined = std::collections::hash_map::DefaultHasher::new();
    let mut row_hashes = HashMap::with_capacity(rows.len());
    for row in rows {
        let row_hash = render_row_hash(row);
        row_hash.hash(&mut combined);
        row_hashes.insert(render_row_id(row).to_owned(), row_hash);
    }
    rows.len().hash(&mut combined);
    RenderRowsDigest {
        rows_hash: combined.finish(),
        row_hashes,
    }
}

/// Diff two full snapshots into a delta (oracle side; the gateway's live
/// loop uses [`derive_render_delta_from_base`] so it only has to cache row
/// hashes, never the previous snapshot's bodies).
pub fn derive_render_delta(prev: &RenderSnapshot, next: &RenderSnapshot) -> RenderDelta {
    let prev_digest = render_rows_digest(&prev.rows);
    derive_render_delta_from_base(
        prev.based_on_seq,
        prev_digest.rows_hash,
        &prev_digest.row_hashes,
        next,
        render_rows_digest(&next.rows).rows_hash,
    )
}

/// Diff `next` against a previously-sent frame known only by its seq,
/// combined rows hash, and per-row hashes. `next_rows_hash` must be the
/// combined hash of `next.rows` (see [`render_rows_digest`]).
pub fn derive_render_delta_from_base(
    from_seq: u64,
    from_rows_hash: u64,
    from_row_hashes: &HashMap<String, u64>,
    next: &RenderSnapshot,
    next_rows_hash: u64,
) -> RenderDelta {
    let mut row_order = Vec::with_capacity(next.rows.len());
    let mut upsert_rows = Vec::new();
    for row in &next.rows {
        let row_id = render_row_id(row);
        row_order.push(row_id.to_owned());
        if from_row_hashes.get(row_id) != Some(&render_row_hash(row)) {
            upsert_rows.push(row.clone());
        }
    }
    RenderDelta {
        from_seq,
        from_rows_hash: from_rows_hash.to_string(),
        based_on_seq: next.based_on_seq,
        rows_hash: next_rows_hash.to_string(),
        row_order,
        upsert_rows,
        tail_activity: next.tail_activity,
        active_tool_group_id: next.active_tool_group_id.clone(),
        progress_locus: next.progress_locus,
        rate_limit: next.rate_limit.clone(),
        window: next.window,
        filtered_placeholders: next.filtered_placeholders.clone(),
    }
}

/// Reassemble the next full snapshot from the held one plus a delta —
/// the reference client algorithm and the oracle's other side.
///
/// Validation order matches the client contract: seq base, rows-hash
/// chain token, row-id completeness, then the reassembled-rows hash
/// tripwire. The held token is `prev.rows_hash` when present (clients
/// never hash; they store the last accepted token); a `prev` without the
/// token is hashed locally, which only the server-side oracle does.
pub fn apply_render_delta(
    prev: &RenderSnapshot,
    delta: &RenderDelta,
) -> Result<RenderSnapshot, RenderDeltaError> {
    if delta.from_seq != prev.based_on_seq {
        return Err(RenderDeltaError::FromSeqMismatch {
            delta_from_seq: delta.from_seq,
            prev_based_on_seq: prev.based_on_seq,
        });
    }
    let prev_rows_hash = prev
        .rows_hash
        .unwrap_or_else(|| render_rows_digest(&prev.rows).rows_hash)
        .to_string();
    if delta.from_rows_hash != prev_rows_hash {
        return Err(RenderDeltaError::FromRowsHashMismatch {
            delta_from_rows_hash: delta.from_rows_hash.clone(),
            prev_rows_hash,
        });
    }
    let mut upsert_by_id: HashMap<&str, &RenderRow> =
        HashMap::with_capacity(delta.upsert_rows.len());
    for row in &delta.upsert_rows {
        let row_id = render_row_id(row);
        if upsert_by_id.insert(row_id, row).is_some() {
            return Err(RenderDeltaError::DuplicateUpsertRow {
                row_id: row_id.to_owned(),
            });
        }
        // Every upsert must be referenced by row_order: a stray upsert is a
        // producer/consumer disagreement, not ignorable padding.
        if !delta.row_order.iter().any(|ordered| ordered == row_id) {
            return Err(RenderDeltaError::UnexpectedUpsertRow {
                row_id: row_id.to_owned(),
            });
        }
    }
    let prev_by_id: HashMap<&str, &RenderRow> = prev
        .rows
        .iter()
        .map(|row| (render_row_id(row), row))
        .collect();
    let mut rows = Vec::with_capacity(delta.row_order.len());
    for row_id in &delta.row_order {
        let row = upsert_by_id
            .get(row_id.as_str())
            .or_else(|| prev_by_id.get(row_id.as_str()))
            .copied()
            .ok_or_else(|| RenderDeltaError::MissingRow {
                row_id: row_id.clone(),
            })?;
        rows.push(row.clone());
    }
    let reassembled_rows_hash = render_rows_digest(&rows).rows_hash;
    if delta.rows_hash != reassembled_rows_hash.to_string() {
        return Err(RenderDeltaError::RowsHashMismatch {
            delta_rows_hash: delta.rows_hash.clone(),
            reassembled_rows_hash: reassembled_rows_hash.to_string(),
        });
    }
    Ok(RenderSnapshot {
        based_on_seq: delta.based_on_seq,
        rows,
        tail_activity: delta.tail_activity,
        active_tool_group_id: delta.active_tool_group_id.clone(),
        progress_locus: delta.progress_locus,
        filtered_placeholders: delta.filtered_placeholders.clone(),
        rate_limit: delta.rate_limit.clone(),
        window: delta.window,
        rows_hash: Some(reassembled_rows_hash),
    })
}

pub fn reduce_transcript_render_state<'a>(
    records: impl IntoIterator<Item = &'a Value>,
) -> RenderSnapshot {
    let records = records.into_iter().collect::<Vec<_>>();
    let run_state = reduce_transcript_run_state(records.iter().copied());
    reduce_transcript_render_state_with_run_state(records.iter().copied(), &run_state)
}

pub fn final_assistant_text_from_render_records<'a>(
    records: impl IntoIterator<Item = &'a Value>,
) -> Option<String> {
    let records = records.into_iter().collect::<Vec<_>>();
    let snapshot = reduce_transcript_render_state(records.iter().copied());
    let final_message = latest_final_assistant_ref(&snapshot)?;
    let message = records
        .iter()
        .find(|record| record_seq(record) == Some(final_message.seq))
        .and_then(|record| record_message(record))?;
    assistant_visible_text(message)
}

pub fn reduce_transcript_render_state_with_run_state<'a>(
    records: impl IntoIterator<Item = &'a Value>,
    run_state: &TranscriptRunState,
) -> RenderSnapshot {
    let records = records.into_iter().collect::<Vec<_>>();
    let based_on_seq = records
        .iter()
        .filter_map(|record| record_seq(record))
        .max()
        .unwrap_or(0);
    let latest_capsules = latest_capsules_by_id(records.iter().copied());
    let mut filtered_placeholders = Vec::new();
    let mut blocks = Vec::new();
    let mut capsule_marks = Vec::new();
    let mut current_tool_group = ToolGroupBuilder::default();
    let mut latest_run_start_seq = 0u64;

    for record in records {
        let Some(seq) = record_seq(record) else {
            continue;
        };
        let Some(message) = record_message(record) else {
            continue;
        };
        let role = normalized_role(message);
        let reference = message_ref(seq, &role, message);
        if is_control_message(message) {
            if control_kind(message) == Some("run_start") {
                latest_run_start_seq = seq;
            }
            if let Some(mark) = capsule_mark_from_message(seq, message) {
                current_tool_group.flush_boundary_into(&mut blocks);
                capsule_marks.push(mark);
            }
            continue;
        }
        let tool_related = is_tool_related_message(&role, message);
        let kind = resolve_message_kind_for_object(&role, message, tool_related);
        match kind {
            "user_input" | "assistant_reply" => {
                current_tool_group.flush_boundary_into(&mut blocks);
                if kind == "assistant_reply" && is_empty_streaming_assistant(message) {
                    filtered_placeholders.push(RenderFilteredPlaceholder {
                        message: reference,
                        reason: RenderPlaceholderFilterReason::EmptyStreamingAssistant,
                    });
                    continue;
                }
                blocks.push(RenderBlock::Message(RenderMessageBlock {
                    reference,
                    role,
                    timestamp: message_timestamp(record, message),
                    streaming: message_streaming(message),
                }));
            }
            "tool_trace" => {
                current_tool_group.push_tool_message(
                    ToolMessage {
                        reference,
                        timestamp: message_timestamp(record, message),
                        tool_use_id: tool_call_id(message),
                        is_result: is_tool_result_trace(&role, message),
                        is_error: message_bool(message, "is_error")
                            || message_bool(message, "isError"),
                    },
                    &mut blocks,
                );
            }
            _ => {
                current_tool_group.flush_boundary_into(&mut blocks);
            }
        }
    }
    current_tool_group.flush_into(&mut blocks);
    apply_tool_group_statuses(&mut blocks, run_state);

    let rows = build_rows(&blocks, &capsule_marks, &latest_capsules, run_state);
    let (tail_activity, active_tool_group_id, progress_locus) =
        derive_tail_activity(blocks.last(), run_state, latest_run_start_seq);
    let rate_limit = run_state.rate_limit.as_ref().map(|limit| RenderRateLimit {
        provider: limit.provider.clone(),
        reset_at: limit.reset_at.clone(),
        window: limit.window.clone(),
        message: limit.message.clone(),
        will_auto_resend: limit.will_auto_resend,
    });

    RenderSnapshot {
        based_on_seq,
        rows,
        tail_activity,
        active_tool_group_id,
        progress_locus,
        filtered_placeholders,
        rate_limit,
        window: None,
        rows_hash: None,
    }
}

#[derive(Debug, Clone)]
enum RenderBlock {
    Message(RenderMessageBlock),
    ToolGroup(RenderToolGroup),
}

impl RenderBlock {
    fn timestamp(&self) -> Option<&str> {
        match self {
            Self::Message(block) => block.timestamp.as_deref(),
            Self::ToolGroup(group) => group.started_at.as_deref(),
        }
    }

    fn is_user(&self) -> bool {
        matches!(self, Self::Message(block) if block.role == "user")
    }

    fn streaming_assistant(&self) -> bool {
        matches!(
            self,
            Self::Message(RenderMessageBlock {
                role,
                streaming: true,
                ..
            }) if role == "assistant"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapsuleMark {
    seq: u64,
    capsule_id: String,
    title: String,
    revision: i64,
    action: RenderCapsuleAction,
}

#[derive(Debug, Clone)]
struct RenderMessageBlock {
    reference: RenderMessageRef,
    role: String,
    timestamp: Option<String>,
    streaming: bool,
}

#[derive(Debug, Clone)]
struct ToolMessage {
    reference: RenderMessageRef,
    timestamp: Option<String>,
    tool_use_id: Option<String>,
    is_result: bool,
    is_error: bool,
}

#[derive(Debug, Clone)]
struct ToolEntryDraft {
    id: String,
    tool_use_id: Option<String>,
    tool_use: Option<RenderMessageRef>,
    tool_result: Option<RenderMessageRef>,
    first_seq: u64,
    first_timestamp: Option<String>,
    last_timestamp: Option<String>,
    is_error: bool,
}

impl ToolEntryDraft {
    fn from_use(message: ToolMessage) -> Self {
        let seq = message.reference.seq;
        let id = tool_entry_id(message.tool_use_id.as_deref(), seq);
        Self {
            id,
            tool_use_id: message.tool_use_id,
            tool_use: Some(message.reference),
            tool_result: None,
            first_seq: seq,
            first_timestamp: message.timestamp.clone(),
            last_timestamp: message.timestamp,
            is_error: false,
        }
    }

    fn from_result(message: ToolMessage) -> Self {
        let seq = message.reference.seq;
        let id = tool_entry_id(message.tool_use_id.as_deref(), seq);
        Self {
            id,
            tool_use_id: message.tool_use_id,
            tool_use: None,
            tool_result: Some(message.reference),
            first_seq: seq,
            first_timestamp: message.timestamp.clone(),
            last_timestamp: message.timestamp,
            is_error: message.is_error,
        }
    }

    fn absorb_result(&mut self, message: ToolMessage) {
        if self.tool_use_id.is_none() {
            self.tool_use_id = message.tool_use_id;
            self.id = tool_entry_id(self.tool_use_id.as_deref(), self.first_seq);
        }
        self.tool_result = Some(message.reference);
        self.last_timestamp = message.timestamp.or_else(|| self.last_timestamp.clone());
        self.is_error = message.is_error;
    }

    fn is_pending(&self) -> bool {
        self.tool_use.is_some() && self.tool_result.is_none()
    }
}

#[derive(Debug, Default)]
struct ToolGroupBuilder {
    entries: Vec<ToolEntryDraft>,
    pending_by_id: BTreeMap<String, usize>,
    anonymous_pending: VecDeque<usize>,
    /// Pending calls that were already flushed at a narrative boundary,
    /// addressed as (block index, entry index) into the emitted blocks. A
    /// late tool_result repairs the flushed entry in place instead of
    /// opening a second group (#TASK-1603) — and the boundary flush itself
    /// stays unconditional, so an orphan call can never hold the group
    /// open and pool every later tool at the end of the thread.
    flushed_pending_by_id: BTreeMap<String, (usize, usize)>,
}

impl ToolGroupBuilder {
    fn push_tool_message(&mut self, message: ToolMessage, blocks: &mut Vec<RenderBlock>) {
        if message.is_result {
            self.push_tool_result(message, blocks);
        } else {
            self.push_tool_use(message);
        }
    }

    fn push_tool_use(&mut self, message: ToolMessage) {
        let tool_use_id = message.tool_use_id.clone();
        let entry = ToolEntryDraft::from_use(message);
        let idx = self.entries.len();
        self.entries.push(entry);
        if let Some(tool_use_id) = tool_use_id {
            self.pending_by_id.insert(tool_use_id, idx);
        } else {
            self.anonymous_pending.push_back(idx);
        }
    }

    fn push_tool_result(&mut self, message: ToolMessage, blocks: &mut Vec<RenderBlock>) {
        if let Some(tool_use_id) = message.tool_use_id.as_deref()
            && let Some(idx) = self.pending_by_id.remove(tool_use_id)
            && let Some(entry) = self.entries.get_mut(idx)
        {
            entry.absorb_result(message);
            return;
        }

        if let Some(tool_use_id) = message.tool_use_id.as_deref()
            && let Some((block_idx, entry_idx)) = self.flushed_pending_by_id.remove(tool_use_id)
            && let Some(RenderBlock::ToolGroup(group)) = blocks.get_mut(block_idx)
            && let Some(entry) = group.entries.get_mut(entry_idx)
        {
            entry.tool_result = Some(message.reference);
            entry.status = if message.is_error {
                RenderToolEntryStatus::Failed
            } else {
                RenderToolEntryStatus::Completed
            };
            if message.timestamp.is_some() {
                group.finished_at = message.timestamp;
            }
            return;
        }

        while let Some(idx) = self.anonymous_pending.pop_front() {
            if let Some(entry) = self.entries.get_mut(idx)
                && entry.is_pending()
            {
                entry.absorb_result(message);
                return;
            }
        }

        if message.tool_use_id.is_none()
            && self.pending_by_id.len() == 1
            && let Some((tool_use_id, idx)) = self
                .pending_by_id
                .iter()
                .next()
                .map(|(tool_use_id, idx)| (tool_use_id.clone(), *idx))
        {
            self.pending_by_id.remove(&tool_use_id);
            if let Some(entry) = self.entries.get_mut(idx) {
                entry.absorb_result(message);
                return;
            }
        }

        self.entries.push(ToolEntryDraft::from_result(message));
    }

    /// Narrative-boundary flush (#TASK-1603 follow-up): the group flushes
    /// unconditionally so tool rows stay at their actual position in the
    /// narration. Calls still waiting for a result are flushed too and
    /// registered in `flushed_pending_by_id`; a late tool_result repairs
    /// the emitted entry in place (see `push_tool_result`), which keeps
    /// group ids unique without letting one orphan call hold the group
    /// open and pool every later tool at the end of the thread.
    fn flush_boundary_into(&mut self, blocks: &mut Vec<RenderBlock>) {
        self.flush_into(blocks);
    }

    fn flush_into(&mut self, blocks: &mut Vec<RenderBlock>) {
        if self.entries.is_empty() {
            return;
        }
        let first_seq = self
            .entries
            .iter()
            .map(|entry| entry.first_seq)
            .min()
            .unwrap_or(0);
        let first_tool_use_id = self
            .entries
            .iter()
            .find_map(|entry| entry.tool_use_id.clone());
        let started_at = self
            .entries
            .iter()
            .find_map(|entry| entry.first_timestamp.clone());
        let finished_at = self
            .entries
            .iter()
            .rev()
            .find_map(|entry| entry.last_timestamp.clone());
        let mut pending_ids = Vec::new();
        let entries: Vec<RenderToolEntry> = self
            .entries
            .drain(..)
            .enumerate()
            .map(|(entry_idx, entry)| {
                if entry.is_pending()
                    && let Some(tool_use_id) = entry.tool_use_id.clone()
                {
                    pending_ids.push((tool_use_id, entry_idx));
                }
                RenderToolEntry {
                    id: entry.id,
                    tool_use_id: entry.tool_use_id,
                    status: if entry.is_error {
                        RenderToolEntryStatus::Failed
                    } else {
                        RenderToolEntryStatus::Completed
                    },
                    tool_use: entry.tool_use,
                    tool_result: entry.tool_result,
                }
            })
            .collect();
        let block_idx = blocks.len();
        blocks.push(RenderBlock::ToolGroup(RenderToolGroup {
            id: tool_group_id(first_tool_use_id.as_deref(), first_seq),
            status: RenderToolGroupStatus::Completed,
            entries,
            started_at,
            finished_at,
        }));
        for (tool_use_id, entry_idx) in pending_ids {
            self.flushed_pending_by_id
                .insert(tool_use_id, (block_idx, entry_idx));
        }
        self.pending_by_id.clear();
        self.anonymous_pending.clear();
    }
}

fn latest_capsules_by_id<'a>(
    records: impl IntoIterator<Item = &'a Value>,
) -> HashMap<String, CapsuleMark> {
    let mut latest = HashMap::<String, CapsuleMark>::new();
    for record in records {
        let Some(seq) = record_seq(record) else {
            continue;
        };
        let Some(message) = record_message(record) else {
            continue;
        };
        let Some(mark) = capsule_mark_from_message(seq, message) else {
            continue;
        };
        latest
            .entry(mark.capsule_id.clone())
            .and_modify(|existing| {
                if mark.revision > existing.revision
                    || (mark.revision == existing.revision && mark.seq > existing.seq)
                {
                    *existing = mark.clone();
                }
            })
            .or_insert(mark);
    }
    latest
}

fn control_kind(message: &Map<String, Value>) -> Option<&str> {
    message
        .get("control")
        .and_then(Value::as_object)
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
}

fn capsule_mark_from_message(seq: u64, message: &Map<String, Value>) -> Option<CapsuleMark> {
    let control = message.get("control").and_then(Value::as_object)?;
    if control.get("kind").and_then(Value::as_str) != Some("capsule_attached") {
        return None;
    }
    let capsule_id = control
        .get("capsule_id")
        .or_else(|| control.get("capsuleId"))
        .or_else(|| control.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let revision = control
        .get("revision")
        .and_then(Value::as_i64)
        .filter(|revision| *revision >= 0)?;
    let title = control
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let action = match control
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("created")
    {
        "created" => RenderCapsuleAction::Created,
        "updated" => RenderCapsuleAction::Updated,
        _ => return None,
    };
    Some(CapsuleMark {
        seq,
        capsule_id,
        title,
        revision,
        action,
    })
}

fn capsule_cards_for_turn(
    turn_start_seq: Option<u64>,
    turn_end_seq: Option<u64>,
    capsule_marks: &[CapsuleMark],
    latest_capsules: &HashMap<String, CapsuleMark>,
) -> Vec<RenderCapsuleCard> {
    let Some(turn_start_seq) = turn_start_seq else {
        return Vec::new();
    };
    let mut by_capsule = HashMap::<String, CapsuleMark>::new();
    let mut first_seq_by_capsule = HashMap::<String, u64>::new();
    for mark in capsule_marks.iter().filter(|mark| {
        mark.seq >= turn_start_seq && turn_end_seq.is_none_or(|end_seq| mark.seq < end_seq)
    }) {
        first_seq_by_capsule
            .entry(mark.capsule_id.clone())
            .or_insert(mark.seq);
        by_capsule
            .entry(mark.capsule_id.clone())
            .and_modify(|existing| {
                if mark.revision > existing.revision
                    || (mark.revision == existing.revision && mark.seq > existing.seq)
                {
                    *existing = mark.clone();
                }
            })
            .or_insert_with(|| mark.clone());
    }

    let mut cards = by_capsule.into_iter().collect::<Vec<_>>();
    cards.sort_by_key(|(capsule_id, mark)| {
        (
            first_seq_by_capsule
                .get(capsule_id)
                .copied()
                .unwrap_or(mark.seq),
            mark.seq,
        )
    });
    cards
        .into_iter()
        .map(|(capsule_id, mark)| {
            let latest = latest_capsules.get(&capsule_id).unwrap_or(&mark);
            RenderCapsuleCard {
                id: format!("capsule_card:{capsule_id}"),
                capsule_id,
                title: latest.title.clone(),
                revision: latest.revision,
                action: mark.action,
            }
        })
        .collect()
}

fn block_first_seq(block: &RenderBlock) -> Option<u64> {
    match block {
        RenderBlock::Message(message) => Some(message.reference.seq),
        RenderBlock::ToolGroup(group) => group
            .entries
            .iter()
            .flat_map(|entry| [entry.tool_use.as_ref(), entry.tool_result.as_ref()])
            .flatten()
            .map(|reference| reference.seq)
            .min(),
    }
}

fn build_rows(
    blocks: &[RenderBlock],
    capsule_marks: &[CapsuleMark],
    latest_capsules: &HashMap<String, CapsuleMark>,
    run_state: &TranscriptRunState,
) -> Vec<RenderRow> {
    let mut rows = Vec::new();
    let mut current_user: Option<RenderMessageBlock> = None;
    let mut current_blocks: Vec<RenderBlock> = Vec::new();
    let mut preceding_user_ts: Option<String> = None;

    for block in blocks {
        if block.is_user() {
            flush_turn(
                &mut rows,
                &mut current_user,
                &mut current_blocks,
                &mut preceding_user_ts,
                block_first_seq(block),
                false,
                capsule_marks,
                latest_capsules,
                run_state,
            );
            if let RenderBlock::Message(message) = block {
                preceding_user_ts = message.timestamp.clone();
                current_user = Some(message.clone());
            }
            continue;
        }
        current_blocks.push(block.clone());
    }

    flush_turn(
        &mut rows,
        &mut current_user,
        &mut current_blocks,
        &mut preceding_user_ts,
        None,
        true,
        capsule_marks,
        latest_capsules,
        run_state,
    );

    rows
}

fn flush_turn(
    rows: &mut Vec<RenderRow>,
    current_user: &mut Option<RenderMessageBlock>,
    current_blocks: &mut Vec<RenderBlock>,
    preceding_user_ts: &mut Option<String>,
    turn_end_seq: Option<u64>,
    is_trailing_turn: bool,
    capsule_marks: &[CapsuleMark],
    latest_capsules: &HashMap<String, CapsuleMark>,
    run_state: &TranscriptRunState,
) {
    let activity = build_activity_rows(
        current_blocks,
        preceding_user_ts.clone(),
        is_trailing_turn,
        run_state,
    );
    if current_user.is_none() && activity.is_empty() {
        current_blocks.clear();
        *preceding_user_ts = None;
        return;
    }

    let user = current_user.take();
    let started_at = user
        .as_ref()
        .and_then(|block| block.timestamp.clone())
        .or_else(|| first_activity_timestamp(&activity));
    let running = activity.iter().any(activity_running);
    let finished_at = if running {
        None
    } else {
        last_activity_timestamp(&activity)
    };
    let id = if let Some(user) = &user {
        format!("user_turn:{}", user.reference.id)
    } else {
        let first_id = activity
            .first()
            .map(activity_id)
            .unwrap_or("empty-orphan")
            .to_owned();
        format!("user_turn:orphan:{first_id}")
    };
    let turn_start_seq = user
        .as_ref()
        .map(|block| block.reference.seq)
        .or_else(|| current_blocks.iter().filter_map(block_first_seq).min());
    let capsule_cards = if is_trailing_turn && run_state.busy {
        Vec::new()
    } else {
        capsule_cards_for_turn(turn_start_seq, turn_end_seq, capsule_marks, latest_capsules)
    };
    rows.push(RenderRow::UserTurn(RenderUserTurnRow {
        id,
        user: user.map(|block| block.reference),
        activity,
        started_at,
        finished_at,
        capsule_cards,
    }));
    current_blocks.clear();
    *preceding_user_ts = None;
}

fn build_activity_rows(
    blocks: &[RenderBlock],
    preceding_user_ts: Option<String>,
    is_trailing_turn: bool,
    run_state: &TranscriptRunState,
) -> Vec<RenderActivityRow> {
    if blocks.is_empty() {
        return Vec::new();
    }
    if blocks.len() == 1
        && let RenderBlock::Message(message) = &blocks[0]
        && message.role == "assistant"
    {
        return vec![RenderActivityRow::AssistantReply(RenderAssistantReplyRow {
            id: format!("assistant_reply:{}", message.reference.id),
            message: message.reference.clone(),
            streaming: message.streaming,
        })];
    }

    let mut step_items = blocks.iter().map(step_item_from_block).collect::<Vec<_>>();
    let defer_final = run_state.busy && is_trailing_turn;
    let final_message = if defer_final {
        None
    } else {
        take_final_assistant(&mut step_items)
    };
    let running = (run_state.busy && is_trailing_turn) || step_items.iter().any(step_item_running);
    let started_at = preceding_user_ts.or_else(|| {
        blocks
            .iter()
            .find_map(|block| block.timestamp().map(ToOwned::to_owned))
    });
    let finished_at = if running {
        None
    } else {
        final_message
            .as_ref()
            .and_then(|message| message_timestamp_from_ref(blocks, message))
            .or_else(|| step_items.iter().rev().find_map(step_item_timestamp))
    };
    let id = step_items
        .first()
        .map(step_item_id)
        .or_else(|| final_message.as_ref().map(|message| message.id.as_str()))
        .unwrap_or("empty");
    vec![RenderActivityRow::Step(RenderStepRow {
        id: format!("step:{id}"),
        steps: step_items,
        final_message,
        running,
        started_at,
        finished_at,
    })]
}

fn step_item_from_block(block: &RenderBlock) -> RenderStepItem {
    match block {
        RenderBlock::Message(message) => RenderStepItem::AssistantMessage(RenderAssistantStep {
            id: format!("assistant_step:{}", message.reference.id),
            message: message.reference.clone(),
            streaming: message.streaming,
        }),
        RenderBlock::ToolGroup(group) => RenderStepItem::ToolGroup(group.clone()),
    }
}

fn take_final_assistant(items: &mut Vec<RenderStepItem>) -> Option<RenderMessageRef> {
    let Some(RenderStepItem::AssistantMessage(RenderAssistantStep {
        message,
        streaming: false,
        ..
    })) = items.last()
    else {
        return None;
    };
    let message = message.clone();
    items.pop();
    Some(message)
}

fn derive_tail_activity(
    tail_block: Option<&RenderBlock>,
    run_state: &TranscriptRunState,
    latest_run_start_seq: u64,
) -> (RenderTailActivity, Option<String>, RenderProgressLocus) {
    if !run_state.busy {
        return (RenderTailActivity::None, None, RenderProgressLocus::None);
    }
    if tail_block.is_some_and(RenderBlock::streaming_assistant) {
        return (
            RenderTailActivity::AssistantStreaming,
            None,
            RenderProgressLocus::Tail,
        );
    }
    if let Some(RenderBlock::ToolGroup(group)) = tail_block
        && tool_group_first_seq(group).is_some_and(|first| first > latest_run_start_seq)
    {
        // Busy run with a CURRENT-run tool group at the tail: stay anchored
        // on the group even in the gap between a tool_result and the next
        // call. Falling back to Thinking for that ~150ms gap made the
        // indicator flicker on every consecutive tool call (user report);
        // the group stays the progress locus until narration text lands or
        // the run stops being busy. A group from a PREVIOUS run (first seq
        // before the latest run_start) must not light up when a fresh run
        // begins before its first body row commits (review #TASK-1706).
        return (
            RenderTailActivity::ToolActive,
            Some(group.id.clone()),
            RenderProgressLocus::ToolGroup,
        );
    }
    (
        RenderTailActivity::Thinking,
        None,
        RenderProgressLocus::Tail,
    )
}

fn tool_group_first_seq(group: &RenderToolGroup) -> Option<u64> {
    group
        .entries
        .iter()
        .filter_map(|entry| {
            let use_seq = entry.tool_use.as_ref().map(|reference| reference.seq);
            let result_seq = entry.tool_result.as_ref().map(|reference| reference.seq);
            match (use_seq, result_seq) {
                (Some(u), Some(r)) => Some(u.min(r)),
                (Some(u), None) => Some(u),
                (None, Some(r)) => Some(r),
                (None, None) => None,
            }
        })
        .min()
}

fn apply_tool_group_statuses(blocks: &mut [RenderBlock], run_state: &TranscriptRunState) {
    let tail_index = blocks.len().checked_sub(1);
    for (index, block) in blocks.iter_mut().enumerate() {
        let RenderBlock::ToolGroup(group) = block else {
            continue;
        };
        let active = run_state.busy
            && run_state.activity == TranscriptRunActivity::UsingTool
            && Some(index) == tail_index
            && group
                .entries
                .iter()
                .any(|entry| entry.tool_use.is_some() && entry.tool_result.is_none());
        group.status = if active {
            RenderToolGroupStatus::Active
        } else {
            RenderToolGroupStatus::Completed
        };
        for entry in &mut group.entries {
            let failed = entry.status == RenderToolEntryStatus::Failed;
            entry.status = if failed {
                RenderToolEntryStatus::Failed
            } else if active && entry.tool_use.is_some() && entry.tool_result.is_none() {
                RenderToolEntryStatus::Running
            } else {
                RenderToolEntryStatus::Completed
            };
        }
    }
}

fn record_seq(record: &Value) -> Option<u64> {
    record.get("seq").and_then(Value::as_u64)
}

fn record_message(record: &Value) -> Option<&Map<String, Value>> {
    record.get("message").and_then(Value::as_object)
}

fn normalized_role(message: &Map<String, Value>) -> String {
    message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .trim()
        .to_ascii_lowercase()
}

fn message_ref(seq: u64, role: &str, message: &Map<String, Value>) -> RenderMessageRef {
    let id = if role == "user" {
        origin_id_of(message)
            .map(|origin_id| format!("origin:{origin_id}"))
            .unwrap_or_else(|| format!("seq:{seq}"))
    } else {
        format!("seq:{seq}")
    };
    RenderMessageRef {
        id,
        seq,
        role: role.to_owned(),
    }
}

fn origin_id_of(message: &Map<String, Value>) -> Option<String> {
    message
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("origin_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn message_timestamp(record: &Value, message: &Map<String, Value>) -> Option<String> {
    message
        .get("timestamp")
        .and_then(Value::as_str)
        .or_else(|| record.get("timestamp").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn message_streaming(message: &Map<String, Value>) -> bool {
    message_bool(message, "pending")
        || message_bool(message, "streaming")
        || message_bool(message, "is_streaming")
        || message_bool(message, "isStreaming")
}

fn message_bool(message: &Map<String, Value>, key: &str) -> bool {
    message.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn is_empty_streaming_assistant(message: &Map<String, Value>) -> bool {
    normalized_role(message) == "assistant"
        && message_streaming(message)
        && !message_has_visible_text(message)
        && !message_has_attachments(message)
}

fn message_has_visible_text(message: &Map<String, Value>) -> bool {
    message
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || value_has_visible_text(message.get("content"))
}

fn value_has_visible_text(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(items)) => items.iter().any(|item| value_has_visible_text(Some(item))),
        Some(Value::Object(object)) => object.iter().any(|(key, item)| {
            let key = key.trim().to_ascii_lowercase();
            if matches!(key.as_str(), "type" | "kind" | "id" | "status") {
                return false;
            }
            value_has_visible_text(Some(item))
        }),
        _ => false,
    }
}

fn message_has_attachments(message: &Map<String, Value>) -> bool {
    value_has_array_items(message.get("attachments"))
        || message
            .get("metadata")
            .and_then(Value::as_object)
            .is_some_and(|metadata| value_has_array_items(metadata.get("attachments")))
}

fn value_has_array_items(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

fn tool_group_id(tool_use_id: Option<&str>, first_seq: u64) -> String {
    match tool_use_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(tool_use_id) => format!("tool_group:{tool_use_id}"),
        None => format!("tool_group:seq:{first_seq}"),
    }
}

fn tool_entry_id(tool_use_id: Option<&str>, first_seq: u64) -> String {
    match tool_use_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(tool_use_id) => format!("tool_entry:{tool_use_id}"),
        None => format!("tool_entry:seq:{first_seq}"),
    }
}

fn activity_running(activity: &RenderActivityRow) -> bool {
    matches!(
        activity,
        RenderActivityRow::Step(RenderStepRow { running: true, .. })
            | RenderActivityRow::AssistantReply(RenderAssistantReplyRow {
                streaming: true,
                ..
            })
    )
}

fn activity_id(activity: &RenderActivityRow) -> &str {
    match activity {
        RenderActivityRow::AssistantReply(row) => &row.id,
        RenderActivityRow::Step(row) => &row.id,
    }
}

fn first_activity_timestamp(activity: &[RenderActivityRow]) -> Option<String> {
    activity.iter().find_map(|row| match row {
        RenderActivityRow::AssistantReply(_) => None,
        RenderActivityRow::Step(row) => row.started_at.clone(),
    })
}

fn last_activity_timestamp(activity: &[RenderActivityRow]) -> Option<String> {
    activity.iter().rev().find_map(|row| match row {
        RenderActivityRow::AssistantReply(_) => None,
        RenderActivityRow::Step(row) => row.finished_at.clone(),
    })
}

fn step_item_running(item: &RenderStepItem) -> bool {
    match item {
        RenderStepItem::AssistantMessage(step) => step.streaming,
        RenderStepItem::ToolGroup(group) => group.status == RenderToolGroupStatus::Active,
    }
}

fn step_item_id(item: &RenderStepItem) -> &str {
    match item {
        RenderStepItem::AssistantMessage(step) => &step.id,
        RenderStepItem::ToolGroup(group) => &group.id,
    }
}

fn step_item_timestamp(item: &RenderStepItem) -> Option<String> {
    match item {
        RenderStepItem::AssistantMessage(_) => None,
        RenderStepItem::ToolGroup(group) => group.finished_at.clone(),
    }
}

fn latest_final_assistant_ref(snapshot: &RenderSnapshot) -> Option<&RenderMessageRef> {
    for row in snapshot.rows.iter().rev() {
        let RenderRow::UserTurn(turn) = row;
        for activity in turn.activity.iter().rev() {
            match activity {
                RenderActivityRow::AssistantReply(reply)
                    if !reply.streaming && reply.message.role == "assistant" =>
                {
                    return Some(&reply.message);
                }
                RenderActivityRow::Step(step) => {
                    if let Some(message) = step
                        .final_message
                        .as_ref()
                        .filter(|message| message.role == "assistant")
                    {
                        return Some(message);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn assistant_visible_text(message: &Map<String, Value>) -> Option<String> {
    if normalized_role(message) != "assistant" {
        return None;
    }
    if let Some(text) = message
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_owned());
    }
    visible_text_from_value(message.get("content"))
}

fn visible_text_from_value(value: Option<&Value>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(value) = value {
        collect_visible_text(value, &mut parts, 0);
    }
    let text = parts.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn collect_visible_text(value: &Value, parts: &mut Vec<String>, depth: usize) {
    if depth > 32 {
        return;
    }
    match value {
        Value::String(text) => push_visible_text_part(parts, text),
        Value::Array(items) => {
            for item in items {
                collect_visible_text(item, parts, depth + 1);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                push_visible_text_part(parts, text);
            }
            for key in ["content", "parts", "items"] {
                if let Some(value) = object.get(key) {
                    collect_visible_text(value, parts, depth + 1);
                }
            }
        }
        _ => {}
    }
}

fn push_visible_text_part(parts: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_owned());
    }
}

fn message_timestamp_from_ref(
    blocks: &[RenderBlock],
    reference: &RenderMessageRef,
) -> Option<String> {
    blocks.iter().find_map(|block| match block {
        RenderBlock::Message(message) if message.reference.id == reference.id => {
            message.timestamp.clone()
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    #[derive(Debug, Deserialize)]
    struct RenderFixture {
        cases: Vec<RenderCase>,
    }

    #[derive(Debug, Deserialize)]
    struct RenderCase {
        name: String,
        source: Option<RenderCaseSource>,
        #[serde(default)]
        records: Vec<Value>,
        run_state: Option<RunStateOverride>,
        expected_run_state: Option<RunStateExpectation>,
        expected: Value,
    }

    #[derive(Debug, Deserialize)]
    struct RenderCaseSource {
        fixture: String,
        min_seq: Option<u64>,
        max_seq: Option<u64>,
    }

    #[derive(Debug, Deserialize)]
    struct RunStateOverride {
        busy: bool,
        activity: TranscriptRunActivity,
    }

    #[derive(Debug, Deserialize)]
    struct RunStateExpectation {
        busy: bool,
        activity: TranscriptRunActivity,
    }

    #[test]
    fn render_state_fixture_cases_match_expected_snapshots() {
        let fixture = load_render_fixture();
        for case in fixture.cases {
            let records = records_for_case(&case);
            let mut run_state = reduce_transcript_run_state(&records);
            if let Some(expected_run_state) = &case.expected_run_state {
                assert_eq!(
                    run_state.busy, expected_run_state.busy,
                    "{} run_state.busy",
                    case.name
                );
                assert_eq!(
                    run_state.activity, expected_run_state.activity,
                    "{} run_state.activity",
                    case.name
                );
            }
            if let Some(override_state) = &case.run_state {
                run_state.busy = override_state.busy;
                run_state.activity = override_state.activity;
            }
            let actual = reduce_transcript_render_state_with_run_state(&records, &run_state);
            let actual = serde_json::to_value(actual).expect("serialize render snapshot");
            assert_eq!(actual, case.expected, "{}", case.name);
        }
    }

    #[test]
    fn tool_group_ids_stay_unique_when_text_interleaves_use_and_result() {
        // #TASK-1603: an assistant text record between a tool_use and its
        // tool_result used to flush the open group early; the late result
        // then opened a SECOND group with the same tool_use_id-derived id,
        // and React warned about duplicate `tool_group:call_*` keys.
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Run a tool", "00000000-0000-0000-0000-000000000001"),
            message_record(
                3,
                json!({
                    "role": "tool_use",
                    "tool_use_id": "call_a",
                    "text": "",
                    "timestamp": "2026-01-01T00:00:03Z"
                }),
            ),
            assistant_record(4, "Narrating between use and result"),
            message_record(
                5,
                json!({
                    "role": "tool_result",
                    "tool_use_id": "call_a",
                    "text": "done",
                    "timestamp": "2026-01-01T00:00:05Z"
                }),
            ),
            assistant_record(6, "Final answer"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let mut group_ids = Vec::new();
        for row in &snapshot.rows {
            let turn = expect_user_turn(row);
            for activity in &turn.activity {
                let RenderActivityRow::Step(step) = activity else {
                    continue;
                };
                for item in &step.steps {
                    if let RenderStepItem::ToolGroup(group) = item {
                        group_ids.push(group.id.clone());
                    }
                }
            }
        }
        let mut deduped = group_ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            group_ids.len(),
            "tool_group ids must be unique per snapshot, got {group_ids:?}"
        );
    }

    /// Tail-activity stability (user report: the thinking indicator
    /// flickers between tool calls). In a busy run whose latest activity
    /// is still tool use, the ~150ms gap after a tool_result commits (tail
    /// group complete, next tool_use not yet committed) must NOT flash
    /// back to Thinking — the indicator stays on the tail tool group.
    #[test]
    fn completed_tail_tool_group_keeps_tool_active_between_calls() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Run tools", "00000000-0000-0000-0000-000000000004"),
            tool_use_record(3, "call_gap", "Bash"),
            tool_result_record(4, "call_gap", false),
        ];
        let run_state = reduce_transcript_run_state(&records);
        assert!(
            run_state.busy,
            "run_start without run_end keeps the run busy"
        );
        // The run-state reducer legitimately reports Thinking after a
        // tool_result; the render layer still anchors the tail on the tool
        // group so the indicator does not flicker through the gap.
        let snapshot = reduce_transcript_render_state_with_run_state(records.iter(), &run_state);
        assert_eq!(
            snapshot.tail_activity,
            RenderTailActivity::ToolActive,
            "gap between tool calls must not flash thinking",
        );
        assert!(
            snapshot.active_tool_group_id.is_some(),
            "the indicator stays anchored on the tail tool group",
        );
    }

    /// Review #TASK-1706: a FRESH run_start arriving before its first body
    /// row commits must not reactivate the previous run's completed tail
    /// tool group — the anchor only applies to groups born after the
    /// latest run_start.
    #[test]
    fn fresh_run_start_does_not_reactivate_previous_runs_tool_group() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "First ask", "00000000-0000-0000-0000-000000000005"),
            tool_use_record(3, "call_old", "Bash"),
            tool_result_record(4, "call_old", false),
            control_record(5, "run_end"),
            // New run begins; its user row has not committed yet.
            control_record(6, "run_start"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        assert_eq!(
            snapshot.tail_activity,
            RenderTailActivity::Thinking,
            "a previous run's completed group must not light up on a fresh run",
        );
        assert_eq!(snapshot.active_tool_group_id, None);
    }

    #[test]
    fn orphan_tool_use_does_not_drag_later_tools_to_the_tail() {
        // User report (Mac + iOS): a tool_use that never receives a result
        // (cancelled / steered call) kept the boundary flush suppressed
        // forever, so every later tool call was absorbed into that one open
        // group, which only flushed at end-of-records — ALL tool rows
        // rendered pooled at the bottom of the thread instead of
        // interleaved with the narration at their actual positions.
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Do things", "00000000-0000-0000-0000-000000000002"),
            tool_use_record(3, "call_orphan", "Bash"), // never gets a result
            assistant_record(4, "First narration"),
            tool_use_record(5, "call_b", "Read"),
            tool_result_record(6, "call_b", false),
            assistant_record(7, "Second narration"),
            tool_use_record(8, "call_c", "Bash"),
            tool_result_record(9, "call_c", false),
            assistant_record(10, "Final answer"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let mut sequence = Vec::new();
        for row in &snapshot.rows {
            let turn = expect_user_turn(row);
            for activity in &turn.activity {
                match activity {
                    RenderActivityRow::AssistantReply(_) => sequence.push("reply".to_owned()),
                    RenderActivityRow::Step(step) => {
                        for item in &step.steps {
                            match item {
                                RenderStepItem::AssistantMessage(_) => {
                                    sequence.push("text".to_owned());
                                }
                                RenderStepItem::ToolGroup(group) => {
                                    sequence.push(format!("tools({})", group.entries.len()));
                                }
                            }
                        }
                    }
                }
            }
        }
        // run_start with no run_end keeps the run busy, so the trailing
        // assistant text stays a step (final-answer placement defers).
        assert_eq!(
            sequence,
            vec![
                "tools(1)", // orphan call stays at its own position
                "text", "tools(1)", // call_b right where it happened
                "text", "tools(1)", // call_c right where it happened
                "text",
            ],
            "tool groups must interleave with narration, got {sequence:?}",
        );
    }

    #[test]
    fn late_tool_result_backfills_the_already_flushed_group() {
        // #TASK-1603 follow-up: with boundary flushes restored, a result
        // arriving after narration must repair the entry inside the group
        // that already flushed (same id, no duplicate group) instead of
        // opening a second group or pooling at the tail.
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Run a tool", "00000000-0000-0000-0000-000000000003"),
            tool_use_record(3, "call_late", "Bash"),
            assistant_record(4, "Narrating between use and result"),
            tool_result_record(5, "call_late", false),
            assistant_record(6, "Final answer"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let mut groups = Vec::new();
        let mut sequence = Vec::new();
        for row in &snapshot.rows {
            let turn = expect_user_turn(row);
            for activity in &turn.activity {
                let RenderActivityRow::Step(step) = activity else {
                    continue;
                };
                for item in &step.steps {
                    match item {
                        RenderStepItem::AssistantMessage(_) => sequence.push("text"),
                        RenderStepItem::ToolGroup(group) => {
                            sequence.push("tools");
                            groups.push(group.clone());
                        }
                    }
                }
            }
        }
        assert_eq!(groups.len(), 1, "exactly one group, got {}", groups.len());
        // The group flushed at the narration boundary, so it must sit
        // BEFORE the text (review #TASK-1680: without this position check
        // the test also passed on the pre-backfill reducer, which held the
        // group open and emitted it after the narration).
        assert_eq!(
            sequence.first().copied(),
            Some("tools"),
            "flushed group must precede the narration, got {sequence:?}",
        );
        assert_eq!(groups[0].entries.len(), 1);
        assert!(
            groups[0].entries[0].tool_result.is_some(),
            "late result must be backfilled into the flushed entry",
        );
        assert_eq!(
            groups[0].entries[0].status,
            RenderToolEntryStatus::Completed
        );
    }

    #[test]
    fn first_message_origin_id_drives_user_row_identity() {
        let records = vec![
            control_record(1, "run_start"),
            message_record(
                2,
                json!({
                    "role": "user",
                    "text": "Start.",
                    "timestamp": "2026-01-01T00:00:01Z",
                    "metadata": {
                        "origin_id": "00000000-0000-0000-0000-000000000001"
                    }
                }),
            ),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let row = expect_user_turn(&snapshot.rows[0]);
        let user = row.user.as_ref().expect("user ref");
        assert_eq!(
            row.id,
            "user_turn:origin:00000000-0000-0000-0000-000000000001"
        );
        assert_eq!(user.id, "origin:00000000-0000-0000-0000-000000000001");
        assert_eq!(user.seq, 2);
        assert_eq!(row_ref_ids(&snapshot), vec![user.id.clone()]);
    }

    #[test]
    fn queued_mid_stream_attribution_stays_physical_seq_order() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Root", "00000000-0000-0000-0000-000000000001"),
            assistant_record(3, "Before queued 1"),
            assistant_record(4, "Before queued 2"),
            user_record(5, "Queued", "00000000-0000-0000-0000-000000000002"),
            user_ack_record(6, "queued-input-1"),
            assistant_record(7, "After queued"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        assert_eq!(snapshot.rows.len(), 2);

        let first = expect_user_turn(&snapshot.rows[0]);
        assert_eq!(
            first.id,
            "user_turn:origin:00000000-0000-0000-0000-000000000001"
        );
        let first_step = expect_step(&first.activity[0]);
        assert_eq!(first_step.steps.len(), 1);
        match &first_step.steps[0] {
            RenderStepItem::AssistantMessage(message) => assert_eq!(message.message.seq, 3),
            other => panic!("expected assistant step, got {other:?}"),
        }
        assert_eq!(
            first_step
                .final_message
                .as_ref()
                .expect("final before ack")
                .seq,
            4
        );

        let second = expect_user_turn(&snapshot.rows[1]);
        assert_eq!(
            second.id,
            "user_turn:origin:00000000-0000-0000-0000-000000000002"
        );
        let reply = expect_assistant_reply(&second.activity[0]);
        assert_eq!(reply.message.seq, 7);
        assert!(
            !row_ref_ids(&snapshot).contains(&"seq:6".to_owned()),
            "user_ack control record must not surface as a rendered message"
        );
    }

    #[test]
    fn two_queued_user_rows_keep_origin_order() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Root", "00000000-0000-0000-0000-000000000001"),
            assistant_record(3, "Before queued"),
            user_record(4, "Queued 1", "00000000-0000-0000-0000-000000000002"),
            user_ack_record(5, "queued-input-1"),
            assistant_record(6, "After queued 1"),
            user_record(7, "Queued 2", "00000000-0000-0000-0000-000000000003"),
            user_ack_record(8, "queued-input-2"),
            assistant_record(9, "After queued 2"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let row_ids = snapshot
            .rows
            .iter()
            .map(|row| expect_user_turn(row).id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            row_ids,
            vec![
                "user_turn:origin:00000000-0000-0000-0000-000000000001",
                "user_turn:origin:00000000-0000-0000-0000-000000000002",
                "user_turn:origin:00000000-0000-0000-0000-000000000003",
            ]
        );
    }

    #[test]
    fn missing_origin_id_keeps_seq_fallback() {
        let records = vec![
            control_record(1, "run_start"),
            message_record(
                2,
                json!({
                    "role": "user",
                    "text": "No origin.",
                    "timestamp": "2026-01-01T00:00:01Z"
                }),
            ),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let row = expect_user_turn(&snapshot.rows[0]);
        let user = row.user.as_ref().expect("user ref");
        assert_eq!(row.id, "user_turn:seq:2");
        assert_eq!(user.id, "seq:2");
        assert_eq!(user.seq, 2);
    }

    #[test]
    fn cold_replay_equals_live_for_origin_user_rows() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Root", "00000000-0000-0000-0000-000000000001"),
            assistant_record(3, "Before queued"),
            user_record(4, "Queued", "00000000-0000-0000-0000-000000000002"),
            user_ack_record(5, "queued-input-1"),
            assistant_record(6, "After queued"),
            control_record(7, "run_complete"),
        ];
        let cold = reduce_transcript_render_state(&records);
        let live = (1..=records.len())
            .map(|len| reduce_transcript_render_state(&records[..len]))
            .next_back()
            .expect("live final snapshot");

        assert_eq!(live, cold);
    }

    #[test]
    fn user_ack_run_state_is_unchanged_by_origin_id() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Root", "00000000-0000-0000-0000-000000000001"),
            user_ack_record(3, "queued-input-1"),
        ];

        let run_state = reduce_transcript_run_state(&records);
        let snapshot = reduce_transcript_render_state_with_run_state(&records, &run_state);

        assert_eq!(run_state.last_user_ack_seq, Some(3));
        assert_eq!(
            run_state.last_user_ack_pending_input_id.as_deref(),
            Some("queued-input-1")
        );
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(
            expect_user_turn(&snapshot.rows[0]).id,
            "user_turn:origin:00000000-0000-0000-0000-000000000001"
        );
        assert!(
            !row_ref_ids(&snapshot).contains(&"seq:3".to_owned()),
            "user_ack control record must not surface as a rendered message"
        );
    }

    #[test]
    fn final_assistant_text_from_render_records_uses_final_segment() {
        let records = vec![
            control_record(1, "run_start"),
            message_record(2, json!({"role": "user", "content": "finish the task"})),
            message_record(
                3,
                json!({
                    "role": "assistant",
                    "content": "polling review status",
                    "text": "polling review status"
                }),
            ),
            control_record(4, "assistant_boundary"),
            message_record(
                5,
                json!({
                    "role": "assistant",
                    "content": "final implementation summary",
                    "text": "final implementation summary"
                }),
            ),
            control_record(6, "run_complete"),
        ];

        assert_eq!(
            final_assistant_text_from_render_records(&records).as_deref(),
            Some("final implementation summary")
        );
    }

    #[test]
    fn final_assistant_text_from_render_records_keeps_single_reply() {
        let records = vec![
            control_record(1, "run_start"),
            message_record(2, json!({"role": "user", "content": "finish the task"})),
            message_record(
                3,
                json!({
                    "role": "assistant",
                    "content": "single final summary",
                    "text": "single final summary"
                }),
            ),
            control_record(4, "run_complete"),
        ];

        assert_eq!(
            final_assistant_text_from_render_records(&records).as_deref(),
            Some("single final summary")
        );
    }

    #[test]
    fn final_assistant_text_from_render_records_returns_none_without_assistant() {
        let records = vec![
            control_record(1, "run_start"),
            message_record(2, json!({"role": "user", "content": "finish the task"})),
            control_record(3, "run_complete"),
        ];

        assert_eq!(final_assistant_text_from_render_records(&records), None);
    }

    #[test]
    fn final_assistant_text_from_render_records_requires_terminal_control_for_steps() {
        let records = vec![
            control_record(1, "run_start"),
            message_record(2, json!({"role": "user", "content": "finish the task"})),
            message_record(
                3,
                json!({
                    "role": "assistant",
                    "content": "polling review status",
                    "text": "polling review status"
                }),
            ),
            control_record(4, "assistant_boundary"),
            message_record(
                5,
                json!({
                    "role": "assistant",
                    "content": "final implementation summary",
                    "text": "final implementation summary"
                }),
            ),
        ];

        assert_eq!(final_assistant_text_from_render_records(&records), None);
    }

    #[test]
    fn capsule_card_after_final_for_create() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Create capsule", "00000000-0000-0000-0000-000000000101"),
            tool_use_record(3, "toolu-capsule-1", "mcp__garyx__capsule_create"),
            tool_result_record(4, "toolu-capsule-1", false),
            capsule_attached_record(
                5,
                "01900000-0000-7000-8000-000000000101",
                "Created Capsule",
                1,
                "created",
            ),
            assistant_record(6, "Final answer"),
            control_record(7, "run_complete"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let row = expect_user_turn(&snapshot.rows[0]);
        let step = expect_step(&row.activity[0]);

        assert_eq!(
            step.final_message.as_ref().map(|message| message.seq),
            Some(6)
        );
        assert_eq!(row.capsule_cards.len(), 1);
        assert_eq!(
            row.capsule_cards[0].id,
            "capsule_card:01900000-0000-7000-8000-000000000101"
        );
        assert_eq!(
            row.capsule_cards[0].capsule_id,
            "01900000-0000-7000-8000-000000000101"
        );
        assert_eq!(row.capsule_cards[0].title, "Created Capsule");
        assert_eq!(row.capsule_cards[0].revision, 1);
        assert_eq!(row.capsule_cards[0].action, RenderCapsuleAction::Created);
    }

    #[test]
    fn capsule_card_waits_until_not_busy() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Create capsule", "00000000-0000-0000-0000-000000000102"),
            tool_use_record(3, "toolu-capsule-1", "mcp__garyx__capsule_create"),
            tool_result_record(4, "toolu-capsule-1", false),
            capsule_attached_record(
                5,
                "01900000-0000-7000-8000-000000000102",
                "Busy Hidden Capsule",
                1,
                "created",
            ),
        ];
        let mut busy = TranscriptRunState::default();
        busy.busy = true;
        busy.activity = TranscriptRunActivity::Thinking;

        let busy_snapshot = reduce_transcript_render_state_with_run_state(&records, &busy);
        assert!(
            expect_user_turn(&busy_snapshot.rows[0])
                .capsule_cards
                .is_empty()
        );

        let idle_snapshot =
            reduce_transcript_render_state_with_run_state(&records, &TranscriptRunState::default());
        assert_eq!(
            expect_user_turn(&idle_snapshot.rows[0]).capsule_cards.len(),
            1
        );
    }

    #[test]
    fn same_run_create_then_update_dedupes_to_latest_revision() {
        let records = vec![
            user_record(
                1,
                "Create and update",
                "00000000-0000-0000-0000-000000000103",
            ),
            tool_use_record(2, "toolu-create", "mcp__garyx__capsule_create"),
            tool_result_record(3, "toolu-create", false),
            capsule_attached_record(
                4,
                "01900000-0000-7000-8000-000000000103",
                "Draft Capsule",
                1,
                "created",
            ),
            tool_use_record(5, "toolu-update", "mcp__garyx__capsule_update"),
            tool_result_record(6, "toolu-update", false),
            capsule_attached_record(
                7,
                "01900000-0000-7000-8000-000000000103",
                "Updated Capsule",
                2,
                "updated",
            ),
            assistant_record(8, "Final answer"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let cards = &expect_user_turn(&snapshot.rows[0]).capsule_cards;

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].capsule_id, "01900000-0000-7000-8000-000000000103");
        assert_eq!(cards[0].title, "Updated Capsule");
        assert_eq!(cards[0].revision, 2);
        assert_eq!(cards[0].action, RenderCapsuleAction::Updated);
    }

    #[test]
    fn multiple_capsules_order_by_first_mark_seq() {
        let records = vec![
            user_record(1, "Create two", "00000000-0000-0000-0000-000000000104"),
            capsule_attached_record(
                2,
                "01900000-0000-7000-8000-000000000201",
                "First Capsule",
                1,
                "created",
            ),
            capsule_attached_record(
                3,
                "01900000-0000-7000-8000-000000000202",
                "Second Capsule",
                1,
                "created",
            ),
            capsule_attached_record(
                4,
                "01900000-0000-7000-8000-000000000201",
                "First Capsule Updated",
                2,
                "updated",
            ),
            assistant_record(5, "Done"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let cards = &expect_user_turn(&snapshot.rows[0]).capsule_cards;

        assert_eq!(
            cards
                .iter()
                .map(|card| card.capsule_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "01900000-0000-7000-8000-000000000201",
                "01900000-0000-7000-8000-000000000202",
            ]
        );
        assert_eq!(cards[0].revision, 2);
        assert_eq!(cards[0].action, RenderCapsuleAction::Updated);
    }

    #[test]
    fn later_run_update_bumps_revision_on_all_cards() {
        let capsule_id = "01900000-0000-7000-8000-000000000301";
        let records = vec![
            user_record(1, "Create", "00000000-0000-0000-0000-000000000301"),
            capsule_attached_record(2, capsule_id, "Original Capsule", 1, "created"),
            assistant_record(3, "Created"),
            user_record(4, "Update", "00000000-0000-0000-0000-000000000302"),
            capsule_attached_record(5, capsule_id, "Latest Capsule", 2, "updated"),
            assistant_record(6, "Updated"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let first = expect_user_turn(&snapshot.rows[0]);
        let second = expect_user_turn(&snapshot.rows[1]);

        assert_eq!(first.capsule_cards.len(), 1);
        assert_eq!(first.capsule_cards[0].revision, 2);
        assert_eq!(first.capsule_cards[0].title, "Latest Capsule");
        assert_eq!(first.capsule_cards[0].action, RenderCapsuleAction::Created);
        assert_eq!(second.capsule_cards[0].revision, 2);
        assert_eq!(second.capsule_cards[0].action, RenderCapsuleAction::Updated);
    }

    #[test]
    fn marker_below_render_floor_omits_card() {
        let records = vec![
            user_record(10, "Create", "00000000-0000-0000-0000-000000000401"),
            assistant_record(12, "Created"),
        ];

        let snapshot = reduce_transcript_render_state(&records);

        assert!(expect_user_turn(&snapshot.rows[0]).capsule_cards.is_empty());
    }

    #[test]
    fn non_capsule_control_does_not_emit_card() {
        let records = vec![
            user_record(1, "No capsule", "00000000-0000-0000-0000-000000000501"),
            control_record(2, "thread_title_updated"),
            assistant_record(3, "Done"),
        ];

        let snapshot = reduce_transcript_render_state(&records);

        assert!(expect_user_turn(&snapshot.rows[0]).capsule_cards.is_empty());
    }

    #[test]
    fn capsule_mark_does_not_break_tail_activity_or_tool_group_status() {
        let records = vec![
            control_record(1, "run_start"),
            user_record(2, "Create", "00000000-0000-0000-0000-000000000601"),
            tool_use_record(3, "toolu-pending", "mcp__garyx__capsule_create"),
            capsule_attached_record(
                4,
                "01900000-0000-7000-8000-000000000601",
                "Pending Capsule",
                1,
                "created",
            ),
        ];
        let run_state = reduce_transcript_run_state(&records);

        let snapshot = reduce_transcript_render_state_with_run_state(&records, &run_state);
        let row = expect_user_turn(&snapshot.rows[0]);
        let step = expect_step(&row.activity[0]);

        assert!(
            row.capsule_cards.is_empty(),
            "busy trailing turn must hide cards"
        );
        assert_eq!(snapshot.tail_activity, RenderTailActivity::ToolActive);
        assert_eq!(snapshot.progress_locus, RenderProgressLocus::ToolGroup);
        assert_eq!(
            snapshot.active_tool_group_id.as_deref(),
            Some("tool_group:toolu-pending")
        );
        match &step.steps[0] {
            RenderStepItem::ToolGroup(group) => {
                assert_eq!(group.status, RenderToolGroupStatus::Active);
                assert_eq!(group.entries[0].status, RenderToolEntryStatus::Running);
            }
            other => panic!("expected tool group, got {other:?}"),
        }
    }

    #[test]
    fn rate_limited_run_complete_surfaces_render_rate_limit() {
        let records = vec![
            control_record(1, "run_start"),
            message_record(2, json!({"role": "user", "content": "hi"})),
            message_record(
                3,
                json!({
                    "role": "system",
                    "kind": "control",
                    "internal": true,
                    "internal_kind": "control",
                    "control": {
                        "kind": "run_complete",
                        "thread_id": "thread::render-final",
                        "run_id": "run::render-final",
                        "at": "2026-01-01T00:00:00Z",
                        "status": "rate_limited",
                        "error": "usageLimitExceeded",
                        "rate_limit": {
                            "provider": "codex",
                            "reset_at": "2026-01-01T05:00:00Z",
                            "window": "primary",
                            "will_auto_resend": true,
                            "message": "You've hit your usage limit."
                        }
                    }
                }),
            ),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        let rate_limit = snapshot.rate_limit.expect("rate limit surfaced");
        assert_eq!(rate_limit.provider.as_deref(), Some("codex"));
        assert_eq!(rate_limit.reset_at.as_deref(), Some("2026-01-01T05:00:00Z"));
        assert_eq!(rate_limit.window.as_deref(), Some("primary"));
        assert!(rate_limit.will_auto_resend);
        assert_eq!(
            rate_limit.message.as_deref(),
            Some("You've hit your usage limit.")
        );
    }

    #[test]
    fn fresh_run_start_clears_prior_render_rate_limit() {
        let rate_limited = json!({
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": {
                "kind": "run_complete",
                "thread_id": "thread::render-final",
                "run_id": "run::render-final",
                "at": "2026-01-01T00:00:00Z",
                "status": "rate_limited",
                "rate_limit": { "provider": "codex", "reset_at": "2026-01-01T05:00:00Z", "will_auto_resend": true }
            }
        });
        let records = vec![
            control_record(1, "run_start"),
            message_record(2, json!({"role": "user", "content": "hi"})),
            message_record(3, rate_limited),
            control_record(4, "run_start"),
        ];

        let snapshot = reduce_transcript_render_state(&records);
        assert!(snapshot.rate_limit.is_none());
    }

    // ---- render delta (#TASK-1956 knife 1) ----

    /// A stream that exercises every row mutation class the delta path
    /// must encode: new rows (user turns), in-place tail mutation
    /// (assistant/tool activity growing inside the open turn), and
    /// untouched rows (the finished first turn while the second runs).
    fn delta_oracle_records() -> Vec<Value> {
        vec![
            control_record(1, "run_start"),
            user_record(2, "First ask", "00000000-0000-0000-0000-00000000d001"),
            assistant_record(3, "Let me check"),
            tool_use_record(4, "call_delta_a", "Bash"),
            tool_result_record(5, "call_delta_a", false),
            assistant_record(6, "First answer"),
            control_record(7, "run_end"),
            control_record(8, "run_start"),
            user_record(9, "Second ask", "00000000-0000-0000-0000-00000000d002"),
            tool_use_record(10, "call_delta_b", "Read"),
            tool_result_record(11, "call_delta_b", true),
            assistant_record(12, "Second answer"),
            control_record(13, "run_end"),
        ]
    }

    /// What a delta reassembly is expected to produce for a snapshot the
    /// reducer built directly: same rows and scalars, `rows_hash` stamped
    /// (the chain token).
    fn delta_expected(mut snapshot: RenderSnapshot) -> RenderSnapshot {
        snapshot.rows_hash = Some(render_rows_digest(&snapshot.rows).rows_hash);
        snapshot
    }

    /// Structural oracle over real captured record streams and the
    /// synthetic mutation stream: at every seq,
    /// `apply_render_delta(prev, derive_render_delta(prev, next))` must
    /// equal the snapshot the reducer derives directly, and the
    /// `rows_hash` token chain must stay connected frame to frame.
    #[test]
    fn delta_oracle_apply_matches_direct_snapshot_at_every_seq() {
        let mut streams = vec![("synthetic-mutations".to_owned(), delta_oracle_records())];
        for fixture in [
            "stream-sync/transcript-with-control.jsonl",
            "stream-sync/transcript-with-tool.jsonl",
            "stream-sync/multi-tool-lull.jsonl",
            "stream-sync/parallel-tool-lull.jsonl",
            "stream-sync/stream-events-with-user-ack.jsonl",
        ] {
            let path = fixture_root().join(fixture);
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            let records = raw
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>();
            streams.push((fixture.to_owned(), records));
        }

        for (name, records) in streams {
            let mut held = delta_expected(reduce_transcript_render_state(&records[..1]));
            for upto in 2..=records.len() {
                let next = reduce_transcript_render_state(&records[..upto]);
                let delta = derive_render_delta(
                    &reduce_transcript_render_state(&records[..upto - 1]),
                    &next,
                );
                // Chain continuity: the delta must depart from exactly the
                // token the held snapshot carries.
                assert_eq!(
                    Some(delta.from_rows_hash.clone()),
                    held.rows_hash.map(|hash| hash.to_string()),
                    "{name}: rows_hash chain broke entering step {upto}"
                );
                held = apply_render_delta(&held, &delta).unwrap_or_else(|error| {
                    panic!("{name}: delta rejected at step {upto}: {error}")
                });
                assert_eq!(
                    held,
                    delta_expected(next),
                    "{name}: reassembly diverged at step {upto}"
                );
            }
        }
    }

    /// The wire minimality claim: a commit that only touches the open
    /// turn re-sends that one row, not the finished turns before it.
    #[test]
    fn delta_upserts_only_changed_rows() {
        let records = delta_oracle_records();
        // Seq 12 appends assistant text inside the second turn; the first
        // turn's row is byte-identical and must not travel.
        let prev = reduce_transcript_render_state(&records[..11]);
        let next = reduce_transcript_render_state(&records[..12]);
        assert_eq!(next.rows.len(), 2, "fixture should hold two user turns");
        let delta = derive_render_delta(&prev, &next);
        assert_eq!(
            delta
                .upsert_rows
                .iter()
                .map(|row| render_row_id(row).to_owned())
                .collect::<Vec<_>>(),
            vec![render_row_id(&next.rows[1]).to_owned()],
            "only the mutated open turn may be re-sent"
        );
        assert_eq!(delta.row_order.len(), 2, "row_order always travels whole");
    }

    #[test]
    fn rows_hash_serializes_as_decimal_string_token() {
        let records = delta_oracle_records();
        let mut snapshot = reduce_transcript_render_state(&records);
        assert!(
            !serde_json::to_value(&snapshot)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("rows_hash"),
            "undeclared connections must stay byte-identical: no rows_hash key"
        );
        // u64::MAX exceeds JS's 2^53 safe-integer range: the token must be
        // a STRING on the wire and survive a roundtrip exactly.
        snapshot.rows_hash = Some(u64::MAX);
        let value = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            value.get("rows_hash").and_then(Value::as_str),
            Some("18446744073709551615")
        );
        let back: RenderSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(back.rows_hash, Some(u64::MAX));
    }

    #[test]
    fn render_delta_wire_names_align_with_render_snapshot() {
        let records = delta_oracle_records();
        let prev = reduce_transcript_render_state(&records[..9]);
        let next = reduce_transcript_render_state(&records);
        let delta = derive_render_delta(&prev, &next);
        let value = serde_json::to_value(&delta).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        // Scalar fields use RenderSnapshot's exact serde names; rateLimit
        // and window are absent here because both are None.
        assert_eq!(
            keys,
            BTreeSet::from(
                [
                    "from_seq",
                    "from_rows_hash",
                    "based_on_seq",
                    "rows_hash",
                    "row_order",
                    "upsert_rows",
                    "tailActivity",
                    "activeToolGroupId",
                    "progress_locus",
                    "filtered_placeholders",
                ]
                .map(str::to_owned)
            )
        );
        let back: RenderDelta = serde_json::from_value(value).unwrap();
        assert_eq!(back, delta);
    }

    /// Gap tripwire: a delta departing from the wrong seq must be
    /// rejected (the receiver discards it and takes its gap path).
    #[test]
    fn apply_render_delta_rejects_from_seq_mismatch() {
        let records = delta_oracle_records();
        let prev = reduce_transcript_render_state(&records[..9]);
        let next = reduce_transcript_render_state(&records[..10]);
        let mut delta = derive_render_delta(&prev, &next);
        delta.from_seq += 1;
        assert_eq!(
            apply_render_delta(&prev, &delta),
            Err(RenderDeltaError::FromSeqMismatch {
                delta_from_seq: delta.from_seq,
                prev_based_on_seq: prev.based_on_seq,
            })
        );
    }

    /// Drift tripwire: same seq but different held rows content (same-seq
    /// drops, guard interactions, future bugs) must be an explicit exit,
    /// never a silent mis-render.
    #[test]
    fn apply_render_delta_rejects_from_rows_hash_mismatch() {
        let records = delta_oracle_records();
        let prev = reduce_transcript_render_state(&records[..9]);
        let next = reduce_transcript_render_state(&records[..10]);
        let delta = derive_render_delta(&prev, &next);
        // The held snapshot drifted: same seq, structurally different rows
        // (a different origin id yields a different user-turn row).
        let mut drifted_records = records[..9].to_vec();
        drifted_records[8] = user_record(
            9,
            "Second ask, drifted",
            "00000000-0000-0000-0000-00000000dead",
        );
        let drifted = reduce_transcript_render_state(&drifted_records);
        assert_eq!(drifted.based_on_seq, prev.based_on_seq);
        let error = apply_render_delta(&drifted, &delta).unwrap_err();
        assert!(
            matches!(error, RenderDeltaError::FromRowsHashMismatch { .. }),
            "expected FromRowsHashMismatch, got {error:?}"
        );
    }

    /// Protocol-violation tripwire: every id in `row_order` must resolve
    /// from `upsert_rows` or the held snapshot.
    #[test]
    fn apply_render_delta_rejects_missing_row_id() {
        let records = delta_oracle_records();
        let prev = reduce_transcript_render_state(&records[..9]);
        let next = reduce_transcript_render_state(&records[..10]);
        let mut delta = derive_render_delta(&prev, &next);
        delta.row_order.push("row-from-nowhere".to_owned());
        assert_eq!(
            apply_render_delta(&prev, &delta),
            Err(RenderDeltaError::MissingRow {
                row_id: "row-from-nowhere".to_owned(),
            })
        );
    }

    /// Protocol-violation tripwire (#TASK-2032 finding 1): an upsert whose
    /// id is absent from `row_order` is a producer/consumer disagreement
    /// and must be rejected, not silently ignored.
    #[test]
    fn apply_render_delta_rejects_upsert_outside_row_order() {
        let records = delta_oracle_records();
        let prev = reduce_transcript_render_state(&records[..9]);
        let next = reduce_transcript_render_state(&records[..10]);
        let mut delta = derive_render_delta(&prev, &next);
        let mut stray = next.rows[0].clone();
        let RenderRow::UserTurn(row) = &mut stray;
        row.id = "row-outside-order".to_owned();
        delta.upsert_rows.push(stray);
        assert_eq!(
            apply_render_delta(&prev, &delta),
            Err(RenderDeltaError::UnexpectedUpsertRow {
                row_id: "row-outside-order".to_owned(),
            })
        );
    }

    /// Duplicate upsert ids are equally malformed: the receiver must not
    /// pick a winner silently.
    #[test]
    fn apply_render_delta_rejects_duplicate_upsert_ids() {
        let records = delta_oracle_records();
        let prev = reduce_transcript_render_state(&records[..9]);
        let next = reduce_transcript_render_state(&records[..10]);
        let mut delta = derive_render_delta(&prev, &next);
        assert!(!delta.upsert_rows.is_empty(), "fixture must change a row");
        let duplicate = delta.upsert_rows[0].clone();
        delta.upsert_rows.push(duplicate);
        let error = apply_render_delta(&prev, &delta).unwrap_err();
        assert!(
            matches!(error, RenderDeltaError::DuplicateUpsertRow { .. }),
            "expected DuplicateUpsertRow, got {error:?}"
        );
    }

    /// Chain tripwire: if the reassembled rows do not hash to the token
    /// the delta declares (a dropped upsert, a tampered body), the frame
    /// is rejected even though every id resolved.
    #[test]
    fn apply_render_delta_rejects_reassembled_rows_hash_mismatch() {
        let records = delta_oracle_records();
        let prev = reduce_transcript_render_state(&records[..11]);
        let next = reduce_transcript_render_state(&records[..12]);
        let mut delta = derive_render_delta(&prev, &next);
        // Simulate a diff bug: the changed row's body is dropped from the
        // wire, so the receiver falls back to its stale held body.
        assert_eq!(delta.upsert_rows.len(), 1, "fixture must change one row");
        delta.upsert_rows.clear();
        let error = apply_render_delta(&prev, &delta).unwrap_err();
        assert!(
            matches!(error, RenderDeltaError::RowsHashMismatch { .. }),
            "expected RowsHashMismatch, got {error:?}"
        );
    }

    /// The digest treats order and length as content: reorder and
    /// truncation must both change the combined token.
    #[test]
    fn render_rows_digest_detects_reorder_and_truncation() {
        let records = delta_oracle_records();
        let snapshot = reduce_transcript_render_state(&records);
        assert_eq!(snapshot.rows.len(), 2, "fixture should hold two user turns");
        let forward = render_rows_digest(&snapshot.rows);
        assert_eq!(
            forward.rows_hash,
            render_rows_digest(&snapshot.rows).rows_hash,
            "digest must be deterministic"
        );
        let reversed = vec![snapshot.rows[1].clone(), snapshot.rows[0].clone()];
        assert_ne!(forward.rows_hash, render_rows_digest(&reversed).rows_hash);
        assert_ne!(
            forward.rows_hash,
            render_rows_digest(&snapshot.rows[..1]).rows_hash
        );
    }

    fn load_render_fixture() -> RenderFixture {
        let path = fixture_root().join("render-layer/render-state-cases.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
    }

    fn records_for_case(case: &RenderCase) -> Vec<Value> {
        if let Some(source) = &case.source {
            let path = fixture_root().join(&source.fixture);
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            return raw
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    (!trimmed.is_empty()).then(|| serde_json::from_str::<Value>(trimmed).unwrap())
                })
                .filter(|record| {
                    let seq = record_seq(record).unwrap_or(0);
                    source.min_seq.is_none_or(|min_seq| seq >= min_seq)
                        && source.max_seq.is_none_or(|max_seq| seq <= max_seq)
                })
                .collect();
        }
        case.records.clone()
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .join("test-fixtures")
    }

    fn message_record(seq: u64, message: Value) -> Value {
        json!({
            "seq": seq,
            "thread_id": "thread::render-final",
            "run_id": "run::render-final",
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
                "control": {
                    "kind": kind,
                    "thread_id": "thread::render-final",
                    "run_id": "run::render-final",
                    "at": "2026-01-01T00:00:00Z",
                }
            }),
        )
    }

    fn user_record(seq: u64, text: &str, origin_id: &str) -> Value {
        message_record(
            seq,
            json!({
                "role": "user",
                "text": text,
                "timestamp": "2026-01-01T00:00:01Z",
                "metadata": {
                    "origin_id": origin_id
                }
            }),
        )
    }

    fn assistant_record(seq: u64, text: &str) -> Value {
        message_record(
            seq,
            json!({
                "role": "assistant",
                "text": text,
                "timestamp": "2026-01-01T00:00:02Z"
            }),
        )
    }

    fn user_ack_record(seq: u64, pending_input_id: &str) -> Value {
        message_record(
            seq,
            json!({
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": {
                    "kind": "user_ack",
                    "thread_id": "thread::render-final",
                    "run_id": "run::render-final",
                    "pending_input_id": pending_input_id,
                    "at": "2026-01-01T00:00:00Z"
                }
            }),
        )
    }

    fn tool_use_record(seq: u64, tool_use_id: &str, tool_name: &str) -> Value {
        message_record(
            seq,
            json!({
                "role": "tool_use",
                "content": {
                    "tool": tool_name,
                    "input": {}
                },
                "tool_use_id": tool_use_id,
                "tool_name": tool_name,
                "timestamp": "2026-01-01T00:00:02Z"
            }),
        )
    }

    fn tool_result_record(seq: u64, tool_use_id: &str, is_error: bool) -> Value {
        message_record(
            seq,
            json!({
                "role": "tool_result",
                "content": {
                    "result": "ok"
                },
                "tool_use_id": tool_use_id,
                "is_error": is_error,
                "timestamp": "2026-01-01T00:00:03Z"
            }),
        )
    }

    fn capsule_attached_record(
        seq: u64,
        capsule_id: &str,
        title: &str,
        revision: i64,
        action: &str,
    ) -> Value {
        message_record(
            seq,
            json!({
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": {
                    "kind": "capsule_attached",
                    "thread_id": "thread::render-final",
                    "run_id": "run::render-final",
                    "at": "2026-01-01T00:00:00Z",
                    "capsule_id": capsule_id,
                    "revision": revision,
                    "action": action,
                    "title": title
                }
            }),
        )
    }

    fn expect_user_turn(row: &RenderRow) -> &RenderUserTurnRow {
        match row {
            RenderRow::UserTurn(row) => row,
        }
    }

    fn expect_assistant_reply(activity: &RenderActivityRow) -> &RenderAssistantReplyRow {
        match activity {
            RenderActivityRow::AssistantReply(row) => row,
            other => panic!("expected assistant reply, got {other:?}"),
        }
    }

    fn expect_step(activity: &RenderActivityRow) -> &RenderStepRow {
        match activity {
            RenderActivityRow::Step(row) => row,
            other => panic!("expected step, got {other:?}"),
        }
    }

    /// Every message-ref id the snapshot's row tree references — the
    /// "which messages does this snapshot render" oracle.
    fn row_ref_ids(snapshot: &RenderSnapshot) -> Vec<String> {
        let mut ids = Vec::new();
        let mut push = |reference: Option<&RenderMessageRef>| {
            if let Some(reference) = reference {
                ids.push(reference.id.clone());
            }
        };
        for row in &snapshot.rows {
            let RenderRow::UserTurn(row) = row;
            push(row.user.as_ref());
            for activity in &row.activity {
                match activity {
                    RenderActivityRow::AssistantReply(reply) => push(Some(&reply.message)),
                    RenderActivityRow::Step(step) => {
                        for item in &step.steps {
                            match item {
                                RenderStepItem::AssistantMessage(message) => {
                                    push(Some(&message.message));
                                }
                                RenderStepItem::ToolGroup(group) => {
                                    for entry in &group.entries {
                                        push(entry.tool_use.as_ref());
                                        push(entry.tool_result.as_ref());
                                    }
                                }
                            }
                        }
                        push(step.final_message.as_ref());
                    }
                }
            }
        }
        ids
    }
}
