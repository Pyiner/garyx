use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::execution::{
    ElevatedLevel, ExecAsk, ExecHost, ExecSecurity, ReasoningLevel, ResponseUsage,
};
pub use crate::routing::DeliveryContext;
pub use crate::threading::ThreadOrigin as SessionOrigin;
pub use crate::threading::ThreadTokenUsage as SessionTokenUsage;
pub use crate::threading::{ChatType, GroupActivation, QueueDrop, QueueMode, SendPolicy};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Supporting structs
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// SessionEntry
// ---------------------------------------------------------------------------

/// Complete session entry with all metadata.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionEntry {
    // === Identity ===
    #[serde(default, alias = "session_id")]
    pub thread_id: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,

    // === Timestamps ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,

    // === Chat Type & Origin ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_type: Option<crate::threading::ChatType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::threading::ThreadOrigin>,

    // === Session file ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_session_id: Option<String>,

    // === AI Configuration Overrides ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbose_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<crate::execution::ReasoningLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevated_level: Option<crate::execution::ElevatedLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<i64>,

    // === Execution Settings ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_host: Option<crate::execution::ExecHost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_security: Option<crate::execution::ExecSecurity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_ask: Option<crate::execution::ExecAsk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_usage: Option<crate::execution::ResponseUsage>,

    // === Group Settings ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_activation: Option<crate::threading::GroupActivation>,
    #[serde(default)]
    pub group_activation_needs_system_intro: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,

    // === Routing Context ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_context: Option<DeliveryContext>,

    // === Message Queuing ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_mode: Option<crate::threading::QueueMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_debounce_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_cap: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_drop: Option<crate::threading::QueueDrop>,

    // === Policy & Control ===
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_policy: Option<crate::threading::SendPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_by: Option<String>,

    // === Transcript & Storage ===
    #[serde(default)]
    pub system_sent: bool,
    #[serde(default)]
    pub aborted_last_run: bool,

    // === Token & Memory Management ===
    #[serde(default)]
    pub token_usage: crate::threading::ThreadTokenUsage,
    #[serde(default)]
    pub compaction_count: i64,

    // === Message history ===
    #[serde(default)]
    pub messages: Vec<HashMap<String, Value>>,

    // === Custom metadata ===
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

fn default_agent_id() -> String {
    "default".to_owned()
}

impl Default for SessionEntry {
    fn default() -> Self {
        Self {
            thread_id: String::new(),
            agent_id: default_agent_id(),
            label: None,
            display_name: None,
            subject: None,
            created_at: None,
            updated_at: None,
            chat_type: None,
            origin: None,
            session_file: None,
            sdk_session_id: None,
            provider_override: None,
            auth_profile_override: None,
            verbose_level: None,
            reasoning_level: None,
            elevated_level: None,
            max_turns: None,
            exec_host: None,
            exec_security: None,
            exec_ask: None,
            exec_node: None,
            response_usage: None,
            group_activation: None,
            group_activation_needs_system_intro: false,
            group_channel: None,
            group_id: None,
            space: None,
            channel: None,
            last_channel: None,
            last_to: None,
            last_account_id: None,
            last_thread_id: None,
            delivery_context: None,
            queue_mode: None,
            queue_debounce_ms: None,
            queue_cap: None,
            queue_drop: None,
            send_policy: None,
            spawned_by: None,
            system_sent: false,
            aborted_last_run: false,
            token_usage: SessionTokenUsage::default(),
            compaction_count: 0,
            messages: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

impl SessionEntry {
    pub fn new_thread(thread_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            agent_id: agent_id.into(),
            ..Self::default()
        }
    }

    pub fn to_thread_record(&self) -> crate::thread_record::ThreadRecord {
        crate::thread_record::ThreadRecord::from(self)
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub fn thread_record_view(&self) -> crate::thread_record::ThreadRecordView<'_> {
        crate::thread_record::ThreadRecordView {
            thread_id: self.thread_id(),
            agent_id: &self.agent_id,
            label: self.label.as_deref(),
            display_name: self.display_name.as_deref(),
            subject: self.subject.as_deref(),
            messages: &self.messages,
            metadata: &self.metadata,
            provider_runtime: self.provider_runtime_state(),
            routing: self.thread_routing_state(),
            queue: self.thread_queue_state(),
            usage: self.thread_usage_state(),
            history: crate::thread_record::ThreadHistoryState::default(),
        }
    }

    pub fn provider_runtime_state(&self) -> crate::thread_record::ProviderRuntimeState {
        crate::thread_record::ProviderRuntimeState {
            session_file: self.session_file.clone(),
            sdk_session_id: self.sdk_session_id.clone(),
            provider_override: self.provider_override.clone(),
            auth_profile_override: self.auth_profile_override.clone(),
            verbose_level: self.verbose_level.clone(),
            reasoning_level: self.reasoning_level.clone(),
            elevated_level: self.elevated_level.clone(),
            max_turns: self.max_turns,
            exec_host: self.exec_host.clone(),
            exec_security: self.exec_security.clone(),
            exec_ask: self.exec_ask.clone(),
            exec_node: self.exec_node.clone(),
            response_usage: self.response_usage.clone(),
        }
    }

    pub fn thread_routing_state(&self) -> crate::thread_record::ThreadRoutingState {
        crate::thread_record::ThreadRoutingState {
            group_activation: self.group_activation.clone(),
            group_activation_needs_system_intro: self.group_activation_needs_system_intro,
            group_channel: self.group_channel.clone(),
            group_id: self.group_id.clone(),
            space: self.space.clone(),
            channel: self.channel.clone(),
            last_channel: self.last_channel.clone(),
            last_to: self.last_to.clone(),
            last_account_id: self.last_account_id.clone(),
            last_thread_id: self.last_thread_id.clone(),
            delivery_context: self.delivery_context.clone(),
            send_policy: self.send_policy.clone(),
            spawned_by: self.spawned_by.clone(),
        }
    }

    pub fn thread_queue_state(&self) -> crate::thread_record::ThreadQueueState {
        crate::thread_record::ThreadQueueState {
            queue_mode: self.queue_mode.clone(),
            queue_debounce_ms: self.queue_debounce_ms,
            queue_cap: self.queue_cap,
            queue_drop: self.queue_drop.clone(),
            system_sent: self.system_sent,
            aborted_last_run: self.aborted_last_run,
        }
    }

    pub fn thread_usage_state(&self) -> crate::thread_record::ThreadUsageState {
        crate::thread_record::ThreadUsageState {
            token_usage: self.token_usage.clone(),
            compaction_count: self.compaction_count,
        }
    }

    /// Update token usage statistics.
    pub fn update_usage(&mut self, input_tokens: i64, output_tokens: i64, cost_usd: f64) {
        self.token_usage.input_tokens += input_tokens;
        self.token_usage.output_tokens += output_tokens;
        self.token_usage.total_tokens += input_tokens + output_tokens;
        self.token_usage.total_cost_usd += cost_usd;
    }

    /// Check if this is a group/channel session.
    pub fn is_group_session(&self) -> bool {
        matches!(
            self.chat_type,
            Some(ChatType::Group) | Some(ChatType::Channel)
        )
    }

    /// Check if this is a direct/DM session.
    pub fn is_direct_session(&self) -> bool {
        matches!(self.chat_type, Some(ChatType::Direct) | None)
    }

    /// Check if sending is allowed for this session.
    pub fn can_send(&self) -> bool {
        match self.send_policy {
            None => true,
            Some(SendPolicy::Allow) => true,
            Some(SendPolicy::Deny) => false,
        }
    }

    /// Update the updated_at timestamp to now.
    pub fn touch(&mut self) {
        self.updated_at = Some(Utc::now());
    }
}

impl From<&SessionEntry> for crate::thread_record::ThreadRecord {
    fn from(value: &SessionEntry) -> Self {
        Self {
            thread_id: value.thread_id.clone(),
            agent_id: value.agent_id.clone(),
            label: value.label.clone(),
            display_name: value.display_name.clone(),
            subject: value.subject.clone(),
            created_at: value.created_at,
            updated_at: value.updated_at,
            chat_type: value.chat_type.clone(),
            origin: value.origin.clone(),
            messages: value.messages.clone(),
            metadata: value.metadata.clone(),
            provider_runtime: value.provider_runtime_state(),
            routing: value.thread_routing_state(),
            queue: value.thread_queue_state(),
            usage: value.thread_usage_state(),
            history: crate::thread_record::ThreadHistoryState::default(),
        }
    }
}

impl From<crate::thread_record::ThreadRecord> for SessionEntry {
    fn from(value: crate::thread_record::ThreadRecord) -> Self {
        Self {
            thread_id: value.thread_id,
            agent_id: value.agent_id,
            label: value.label,
            display_name: value.display_name,
            subject: value.subject,
            created_at: value.created_at,
            updated_at: value.updated_at,
            chat_type: value.chat_type,
            origin: value.origin,
            session_file: value.provider_runtime.session_file,
            sdk_session_id: value.provider_runtime.sdk_session_id,
            provider_override: value.provider_runtime.provider_override,
            auth_profile_override: value.provider_runtime.auth_profile_override,
            verbose_level: value.provider_runtime.verbose_level,
            reasoning_level: value.provider_runtime.reasoning_level,
            elevated_level: value.provider_runtime.elevated_level,
            max_turns: value.provider_runtime.max_turns,
            exec_host: value.provider_runtime.exec_host,
            exec_security: value.provider_runtime.exec_security,
            exec_ask: value.provider_runtime.exec_ask,
            exec_node: value.provider_runtime.exec_node,
            response_usage: value.provider_runtime.response_usage,
            group_activation: value.routing.group_activation,
            group_activation_needs_system_intro: value.routing.group_activation_needs_system_intro,
            group_channel: value.routing.group_channel,
            group_id: value.routing.group_id,
            space: value.routing.space,
            channel: value.routing.channel,
            last_channel: value.routing.last_channel,
            last_to: value.routing.last_to,
            last_account_id: value.routing.last_account_id,
            last_thread_id: value.routing.last_thread_id,
            delivery_context: value.routing.delivery_context,
            queue_mode: value.queue.queue_mode,
            queue_debounce_ms: value.queue.queue_debounce_ms,
            queue_cap: value.queue.queue_cap,
            queue_drop: value.queue.queue_drop,
            send_policy: value.routing.send_policy,
            spawned_by: value.routing.spawned_by,
            system_sent: value.queue.system_sent,
            aborted_last_run: value.queue.aborted_last_run,
            token_usage: value.usage.token_usage,
            compaction_count: value.usage.compaction_count,
            messages: value.messages,
            metadata: value.metadata,
        }
    }
}

#[cfg(test)]
mod tests;
