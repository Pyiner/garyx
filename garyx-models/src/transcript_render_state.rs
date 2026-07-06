use std::collections::{BTreeMap, HashMap, VecDeque};

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
    #[serde(rename = "visibleMessageIds")]
    pub visible_message_ids: Vec<String>,
    pub filtered_placeholders: Vec<RenderFilteredPlaceholder>,
    /// Present when the active run terminated because the provider's rolling
    /// usage quota was exhausted. Clients render a banner + live countdown to
    /// `reset_at` and surface whether an automatic resend is scheduled.
    #[serde(rename = "rateLimit", default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<RenderRateLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<RenderWindow>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderRow {
    UserTurn(RenderUserTurnRow),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderCapsuleCard {
    pub id: String,
    pub capsule_id: String,
    pub title: String,
    pub revision: i64,
    pub action: RenderCapsuleAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderCapsuleAction {
    Created,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderUserTurnRow {
    pub id: String,
    pub user: Option<RenderMessageRef>,
    pub activity: Vec<RenderActivityRow>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capsule_cards: Vec<RenderCapsuleCard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderActivityRow {
    AssistantReply(RenderAssistantReplyRow),
    Step(RenderStepRow),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderAssistantReplyRow {
    pub id: String,
    pub message: RenderMessageRef,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderStepRow {
    pub id: String,
    pub steps: Vec<RenderStepItem>,
    pub final_message: Option<RenderMessageRef>,
    pub running: bool,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderStepItem {
    AssistantMessage(RenderAssistantStep),
    ToolGroup(RenderToolGroup),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderAssistantStep {
    pub id: String,
    pub message: RenderMessageRef,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderToolGroup {
    pub id: String,
    pub status: RenderToolGroupStatus,
    pub entries: Vec<RenderToolEntry>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolGroupStatus {
    Active,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderToolEntry {
    pub id: String,
    pub tool_use_id: Option<String>,
    pub status: RenderToolEntryStatus,
    pub tool_use: Option<RenderMessageRef>,
    pub tool_result: Option<RenderMessageRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderToolEntryStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    let mut visible_message_ids = Vec::new();
    let mut filtered_placeholders = Vec::new();
    let mut blocks = Vec::new();
    let mut capsule_marks = Vec::new();
    let mut current_tool_group = ToolGroupBuilder::default();

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
                visible_message_ids.push(reference.id.clone());
                blocks.push(RenderBlock::Message(RenderMessageBlock {
                    reference,
                    role,
                    timestamp: message_timestamp(record, message),
                    streaming: message_streaming(message),
                }));
            }
            "tool_trace" => {
                visible_message_ids.push(reference.id.clone());
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
        derive_tail_activity(blocks.last(), run_state);
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
        visible_message_ids,
        filtered_placeholders,
        rate_limit,
        window: None,
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
        && group.status == RenderToolGroupStatus::Active
    {
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
            let RenderRow::UserTurn(turn) = row else {
                continue;
            };
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
            let RenderRow::UserTurn(turn) = row else {
                continue;
            };
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
                "text",
                "tools(1)", // call_b right where it happened
                "text",
                "tools(1)", // call_c right where it happened
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
        for row in &snapshot.rows {
            let RenderRow::UserTurn(turn) = row else {
                continue;
            };
            for activity in &turn.activity {
                let RenderActivityRow::Step(step) = activity else {
                    continue;
                };
                for item in &step.steps {
                    if let RenderStepItem::ToolGroup(group) = item {
                        groups.push(group.clone());
                    }
                }
            }
        }
        assert_eq!(groups.len(), 1, "exactly one group, got {}", groups.len());
        assert_eq!(groups[0].entries.len(), 1);
        assert!(
            groups[0].entries[0].tool_result.is_some(),
            "late result must be backfilled into the flushed entry",
        );
        assert_eq!(groups[0].entries[0].status, RenderToolEntryStatus::Completed);
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
        assert_eq!(snapshot.visible_message_ids, vec![user.id.clone()]);
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
        assert!(!snapshot.visible_message_ids.contains(&"seq:6".to_owned()));
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
        assert!(!snapshot.visible_message_ids.contains(&"seq:3".to_owned()));
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
}
