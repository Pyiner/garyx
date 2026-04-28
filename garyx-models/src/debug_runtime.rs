use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageLifecycleStatus {
    Received,
    Filtered,
    ThreadResolved,
    RunStarted,
    RunStreaming,
    ReplySent,
    ReplyFailed,
    RunInterrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageTerminalReason {
    None,
    PolicyFiltered,
    RoutingRejected,
    ProviderError,
    ToolError,
    SelfRestart,
    ShutdownDuringRun,
    ReplyDispatchFailed,
    UnknownInterruption,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageLedgerEvent {
    pub ledger_id: String,
    pub bot_id: String,
    pub status: MessageLifecycleStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<MessageTerminalReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_message_id: Option<String>,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageLedgerRecord {
    pub ledger_id: String,
    pub bot_id: String,
    pub status: MessageLifecycleStatus,
    pub first_seen_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<MessageTerminalReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_message_id: Option<String>,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotThreadDebugSummary {
    pub bot_id: String,
    pub thread_id: String,
    pub last_status: MessageLifecycleStatus,
    pub last_event_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<MessageTerminalReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_text_excerpt: Option<String>,
    pub message_count: usize,
}

impl MessageLedgerRecord {
    pub fn from_event(event: &MessageLedgerEvent) -> Self {
        Self {
            ledger_id: event.ledger_id.clone(),
            bot_id: event.bot_id.clone(),
            status: event.status,
            first_seen_at: event.created_at.clone(),
            updated_at: event.created_at.clone(),
            thread_id: trim_non_empty(event.thread_id.as_deref()),
            run_id: trim_non_empty(event.run_id.as_deref()),
            channel: trim_non_empty(event.channel.as_deref()),
            account_id: trim_non_empty(event.account_id.as_deref()),
            chat_id: trim_non_empty(event.chat_id.as_deref()),
            from_id: trim_non_empty(event.from_id.as_deref()),
            native_message_id: trim_non_empty(event.native_message_id.as_deref()),
            text_excerpt: trim_non_empty(event.text_excerpt.as_deref()),
            terminal_reason: event.terminal_reason,
            reply_message_id: trim_non_empty(event.reply_message_id.as_deref()),
            metadata: normalize_metadata(&event.metadata),
        }
    }

    pub fn apply_event(&mut self, event: &MessageLedgerEvent) {
        self.bot_id = event.bot_id.clone();
        self.status = event.status;
        self.updated_at = event.created_at.clone();
        assign_if_some(&mut self.thread_id, event.thread_id.as_deref());
        assign_if_some(&mut self.run_id, event.run_id.as_deref());
        assign_if_some(&mut self.channel, event.channel.as_deref());
        assign_if_some(&mut self.account_id, event.account_id.as_deref());
        assign_if_some(&mut self.chat_id, event.chat_id.as_deref());
        assign_if_some(&mut self.from_id, event.from_id.as_deref());
        assign_if_some(
            &mut self.native_message_id,
            event.native_message_id.as_deref(),
        );
        assign_if_some(&mut self.text_excerpt, event.text_excerpt.as_deref());
        if let Some(reason) = event.terminal_reason {
            self.terminal_reason = Some(reason);
        }
        assign_if_some(
            &mut self.reply_message_id,
            event.reply_message_id.as_deref(),
        );
        merge_metadata(&mut self.metadata, &event.metadata);
    }

    pub fn is_problem(&self) -> bool {
        matches!(
            self.status,
            MessageLifecycleStatus::ReplyFailed | MessageLifecycleStatus::RunInterrupted
        ) || !matches!(
            self.terminal_reason.unwrap_or(MessageTerminalReason::None),
            MessageTerminalReason::None
        )
    }
}

fn default_metadata() -> Value {
    Value::Object(Default::default())
}

fn trim_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_metadata(value: &Value) -> Value {
    if value.is_object() {
        value.clone()
    } else {
        default_metadata()
    }
}

fn assign_if_some(slot: &mut Option<String>, next: Option<&str>) {
    if let Some(value) = trim_non_empty(next) {
        *slot = Some(value);
    }
}

fn merge_metadata(target: &mut Value, patch: &Value) {
    if !patch.is_object() {
        return;
    }
    if !target.is_object() {
        *target = default_metadata();
    }
    if let (Some(target_map), Some(patch_map)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch_map {
            target_map.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests;
