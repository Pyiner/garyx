use std::collections::{BTreeMap, VecDeque};

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
pub struct RenderUserTurnRow {
    pub id: String,
    pub user: Option<RenderMessageRef>,
    pub activity: Vec<RenderActivityRow>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
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
    let mut visible_message_ids = Vec::new();
    let mut filtered_placeholders = Vec::new();
    let mut blocks = Vec::new();
    let mut current_tool_group = ToolGroupBuilder::default();

    for record in records {
        let Some(seq) = record_seq(record) else {
            continue;
        };
        let Some(message) = record_message(record) else {
            continue;
        };
        let role = normalized_role(message);
        let reference = message_ref(seq, &role);
        if is_control_message(message) {
            continue;
        }
        let tool_related = is_tool_related_message(&role, message);
        let kind = resolve_message_kind_for_object(&role, message, tool_related);
        match kind {
            "user_input" | "assistant_reply" => {
                current_tool_group.flush_into(&mut blocks);
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
                if !should_render_tool_trace(&role, message) {
                    continue;
                }
                visible_message_ids.push(reference.id.clone());
                current_tool_group.push_tool_message(ToolMessage {
                    reference,
                    timestamp: message_timestamp(record, message),
                    tool_use_id: tool_call_id(message),
                    is_result: is_tool_result_trace(&role, message),
                    is_error: message_bool(message, "is_error") || message_bool(message, "isError"),
                });
            }
            _ => {
                current_tool_group.flush_into(&mut blocks);
            }
        }
    }
    current_tool_group.flush_into(&mut blocks);
    apply_tool_group_statuses(&mut blocks, run_state);

    let rows = build_rows(&blocks, run_state);
    let (tail_activity, active_tool_group_id, progress_locus) =
        derive_tail_activity(blocks.last(), run_state);

    RenderSnapshot {
        based_on_seq,
        rows,
        tail_activity,
        active_tool_group_id,
        progress_locus,
        visible_message_ids,
        filtered_placeholders,
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
}

impl ToolGroupBuilder {
    fn push_tool_message(&mut self, message: ToolMessage) {
        if message.is_result {
            self.push_tool_result(message);
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

    fn push_tool_result(&mut self, message: ToolMessage) {
        if let Some(tool_use_id) = message.tool_use_id.as_deref() {
            if let Some(idx) = self.pending_by_id.remove(tool_use_id) {
                if let Some(entry) = self.entries.get_mut(idx) {
                    entry.absorb_result(message);
                    return;
                }
            }
        }

        while let Some(idx) = self.anonymous_pending.pop_front() {
            if let Some(entry) = self.entries.get_mut(idx) {
                if entry.is_pending() {
                    entry.absorb_result(message);
                    return;
                }
            }
        }

        if message.tool_use_id.is_none() && self.pending_by_id.len() == 1 {
            if let Some((tool_use_id, idx)) = self
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
        }

        self.entries.push(ToolEntryDraft::from_result(message));
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
        let entries = self
            .entries
            .drain(..)
            .map(|entry| RenderToolEntry {
                id: entry.id,
                tool_use_id: entry.tool_use_id,
                status: if entry.is_error {
                    RenderToolEntryStatus::Failed
                } else {
                    RenderToolEntryStatus::Completed
                },
                tool_use: entry.tool_use,
                tool_result: entry.tool_result,
            })
            .collect();
        blocks.push(RenderBlock::ToolGroup(RenderToolGroup {
            id: tool_group_id(first_tool_use_id.as_deref(), first_seq),
            status: RenderToolGroupStatus::Completed,
            entries,
            started_at,
            finished_at,
        }));
        self.pending_by_id.clear();
        self.anonymous_pending.clear();
    }
}

fn build_rows(blocks: &[RenderBlock], run_state: &TranscriptRunState) -> Vec<RenderRow> {
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
                false,
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
        true,
        run_state,
    );

    rows
}

fn flush_turn(
    rows: &mut Vec<RenderRow>,
    current_user: &mut Option<RenderMessageBlock>,
    current_blocks: &mut Vec<RenderBlock>,
    preceding_user_ts: &mut Option<String>,
    is_trailing_turn: bool,
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
    rows.push(RenderRow::UserTurn(RenderUserTurnRow {
        id,
        user: user.map(|block| block.reference),
        activity,
        started_at,
        finished_at,
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
    if blocks.len() == 1 {
        if let RenderBlock::Message(message) = &blocks[0] {
            if message.role == "assistant" {
                return vec![RenderActivityRow::AssistantReply(RenderAssistantReplyRow {
                    id: format!("assistant_reply:{}", message.reference.id),
                    message: message.reference.clone(),
                    streaming: message.streaming,
                })];
            }
        }
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
    if let Some(RenderBlock::ToolGroup(group)) = tail_block {
        if group.status == RenderToolGroupStatus::Active {
            return (
                RenderTailActivity::ToolActive,
                Some(group.id.clone()),
                RenderProgressLocus::ToolGroup,
            );
        }
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

fn should_render_tool_trace(_role: &str, message: &Map<String, Value>) -> bool {
    let reasoning = message
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("reasoning"));
    !reasoning
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

fn message_ref(seq: u64, role: &str) -> RenderMessageRef {
    RenderMessageRef {
        id: format!("seq:{seq}"),
        seq,
        role: role.to_owned(),
    }
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
}
