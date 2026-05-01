use crate::execution::{
    ElevatedLevel, ExecAsk, ExecHost, ExecSecurity, ReasoningLevel, ResponseUsage,
};
use crate::routing::DeliveryContext;
use crate::task::ThreadTask;
use crate::threading::{GroupActivation, QueueDrop, QueueMode, SendPolicy, ThreadTokenUsage};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderRuntimeState {
    pub session_file: Option<String>,
    pub sdk_session_id: Option<String>,
    pub provider_override: Option<String>,
    pub auth_profile_override: Option<String>,
    pub verbose_level: Option<String>,
    pub reasoning_level: Option<ReasoningLevel>,
    pub elevated_level: Option<ElevatedLevel>,
    pub max_turns: Option<i64>,
    pub exec_host: Option<ExecHost>,
    pub exec_security: Option<ExecSecurity>,
    pub exec_ask: Option<ExecAsk>,
    pub exec_node: Option<String>,
    pub response_usage: Option<ResponseUsage>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadRoutingState {
    pub group_activation: Option<GroupActivation>,
    pub group_activation_needs_system_intro: bool,
    pub group_channel: Option<String>,
    pub group_id: Option<String>,
    pub space: Option<String>,
    pub channel: Option<String>,
    pub last_channel: Option<String>,
    pub last_to: Option<String>,
    pub last_account_id: Option<String>,
    pub last_thread_id: Option<String>,
    pub delivery_context: Option<DeliveryContext>,
    pub send_policy: Option<SendPolicy>,
    pub spawned_by: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadQueueState {
    pub queue_mode: Option<QueueMode>,
    pub queue_debounce_ms: Option<i64>,
    pub queue_cap: Option<i64>,
    pub queue_drop: Option<QueueDrop>,
    pub system_sent: bool,
    pub aborted_last_run: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadUsageState {
    pub token_usage: ThreadTokenUsage,
    pub compaction_count: i64,
}

pub const THREAD_HISTORY_SOURCE_TRANSCRIPT_V1: &str = "transcript_v1";

fn default_thread_history_source() -> String {
    THREAD_HISTORY_SOURCE_TRANSCRIPT_V1.to_owned()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveRunSnapshot {
    pub run_id: Option<String>,
    pub provider_key: Option<String>,
    pub assistant_response: Option<String>,
    pub messages: Vec<HashMap<String, Value>>,
    pub pending_user_inputs: Vec<HashMap<String, Value>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreadHistoryState {
    pub source: String,
    pub transcript_file: Option<String>,
    pub message_count: usize,
    pub snapshot_limit: usize,
    pub snapshot_truncated: bool,
    pub last_message_at: Option<DateTime<Utc>>,
    pub recent_committed_run_ids: Vec<String>,
    pub active_run_snapshot: Option<ActiveRunSnapshot>,
}

impl Default for ThreadHistoryState {
    fn default() -> Self {
        Self {
            source: default_thread_history_source(),
            transcript_file: None,
            message_count: 0,
            snapshot_limit: 0,
            snapshot_truncated: false,
            last_message_at: None,
            recent_committed_run_ids: Vec::new(),
            active_run_snapshot: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThreadRecordView<'a> {
    pub thread_id: &'a str,
    pub agent_id: &'a str,
    pub label: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub messages: &'a [HashMap<String, Value>],
    pub metadata: &'a HashMap<String, Value>,
    pub provider_runtime: ProviderRuntimeState,
    pub routing: ThreadRoutingState,
    pub queue: ThreadQueueState,
    pub usage: ThreadUsageState,
    pub history: ThreadHistoryState,
    pub task: Option<&'a ThreadTask>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThreadRecord {
    pub thread_id: String,
    pub agent_id: String,
    pub label: Option<String>,
    pub display_name: Option<String>,
    pub subject: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub chat_type: Option<crate::threading::ChatType>,
    pub origin: Option<crate::threading::ThreadOrigin>,
    pub messages: Vec<HashMap<String, Value>>,
    pub metadata: HashMap<String, Value>,
    pub provider_runtime: ProviderRuntimeState,
    pub routing: ThreadRoutingState,
    pub queue: ThreadQueueState,
    pub usage: ThreadUsageState,
    pub history: ThreadHistoryState,
    pub task: Option<ThreadTask>,
}
