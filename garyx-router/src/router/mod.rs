use std::sync::Arc;

use crate::message_ledger::MessageLedgerStore;
pub(crate) use crate::message_routing::MessageRoutingIndex;
use crate::store::ThreadStore;
use crate::thread_history::ThreadHistoryRepository;
use garyx_models::config::GaryxConfig;
pub(crate) use garyx_models::provider::ImagePayload;
use garyx_models::provider::StreamEvent;
pub(crate) use garyx_models::routing::DeliveryContext;
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink, is_canonical_thread_id};
use garyx_models::{MessageLedgerEvent, MessageLedgerRecord};
use serde_json::Value;

mod command_catalog;
mod contracts;
mod inbound;
mod message;
mod run;
mod threading;

pub use command_catalog::{command_catalog_for_config, reserved_command_names};
pub use contracts::ThreadCreator;
pub use contracts::{
    AgentDispatcher, InboundRequest, InboundResult, InboundSink, ThreadListEntry,
    ThreadMessageRequest,
};
pub(crate) use contracts::{DispatchMetadataContext, NavigationContext, RouteContext};
pub use inbound::is_native_command_text;

#[cfg(test)]
mod tests;

/// Maximum number of threads kept in per-user navigation history.
const MAX_HISTORY: usize = 20;
/// Maximum number of outbound message records persisted per thread.
const MAX_OUTBOUND_MESSAGE_IDS: usize = 100;
pub const NATIVE_COMMAND_TEXT_METADATA_KEY: &str = "native_command_text";

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeThreadResult {
    reply_text: String,
    switched_thread: Option<String>,
}

// ---------------------------------------------------------------------------
// MessageRouter
// ---------------------------------------------------------------------------

/// Routes messages from channels to appropriate agent threads.
///
/// This is the Rust port of `garyx.sessions.routing.MessageRouter`.
/// It handles thread resolution, user-thread mapping, navigation,
/// reply-based routing, metadata enrichment, and optional dispatch
/// to a provider bridge.
pub struct MessageRouter {
    threads: Arc<dyn ThreadStore>,
    thread_history: Option<Arc<ThreadHistoryRepository>>,
    config: GaryxConfig,
    default_agent: String,
    thread_creator: Option<Arc<dyn ThreadCreator>>,
    inbound_sink: Option<Arc<dyn InboundSink>>,
    thread_nav: threading::ThreadNavigationState,
    reply_routing: message::ReplyRoutingState,
    delivery_ctx: message::DeliveryContextState,
    thread_logs: Option<Arc<dyn ThreadLogSink>>,
    message_ledger: Option<Arc<MessageLedgerStore>>,
}

impl MessageRouter {
    pub fn new(threads: Arc<dyn ThreadStore>, config: GaryxConfig) -> Self {
        let default_agent = config
            .agents
            .get("default")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_owned();

        Self {
            threads,
            thread_history: None,
            config,
            default_agent,
            thread_creator: None,
            inbound_sink: None,
            thread_nav: threading::ThreadNavigationState::default(),
            reply_routing: message::ReplyRoutingState::default(),
            delivery_ctx: message::DeliveryContextState::default(),
            thread_logs: None,
            message_ledger: None,
        }
    }

    pub fn set_thread_creator(&mut self, creator: Arc<dyn ThreadCreator>) {
        self.thread_creator = Some(creator);
    }

    pub fn set_inbound_sink(&mut self, sink: Arc<dyn InboundSink>) {
        self.inbound_sink = Some(sink);
    }

    pub async fn create_thread_with_options(
        &self,
        options: crate::ThreadEnsureOptions,
    ) -> Result<(String, Value), String> {
        if let Some(creator) = &self.thread_creator {
            creator.create_thread(self.threads.clone(), options).await
        } else {
            crate::create_thread_record(&self.threads, options).await
        }
    }

    pub fn set_thread_log_sink(&mut self, sink: Arc<dyn ThreadLogSink>) {
        self.thread_logs = Some(sink);
    }

    pub fn set_thread_history_repository(&mut self, history: Arc<ThreadHistoryRepository>) {
        self.thread_history = Some(history);
    }

    pub fn set_message_ledger_store(&mut self, store: Arc<MessageLedgerStore>) {
        self.message_ledger = Some(store);
    }

    pub fn command_catalog(
        &self,
        options: garyx_models::command_catalog::CommandCatalogOptions,
    ) -> garyx_models::command_catalog::CommandCatalog {
        command_catalog::command_catalog_for_config(&self.config, options)
    }

    fn thread_log_sink(&self) -> Option<Arc<dyn ThreadLogSink>> {
        self.thread_logs.clone()
    }

    async fn record_thread_log(&self, event: ThreadLogEvent) {
        if !is_canonical_thread_id(&event.thread_id) {
            return;
        }
        if let Some(sink) = self.thread_log_sink() {
            sink.record_event(event).await;
        }
    }

    pub async fn record_message_ledger_event(&self, event: MessageLedgerEvent) {
        if let Some(store) = &self.message_ledger {
            let _ = store.append_event(event).await;
        }
    }

    pub async fn list_message_ledger_events_for_thread(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Vec<MessageLedgerEvent> {
        let Some(store) = &self.message_ledger else {
            return Vec::new();
        };
        store
            .list_events_for_thread(thread_id, limit)
            .await
            .unwrap_or_default()
    }

    pub async fn list_message_ledger_records_for_bot(
        &self,
        bot_id: &str,
        limit: usize,
    ) -> Vec<MessageLedgerRecord> {
        let Some(store) = &self.message_ledger else {
            return Vec::new();
        };
        store
            .records_for_bot(bot_id, limit)
            .await
            .unwrap_or_default()
    }
}
