use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use garyx_models::messages::MessageMetadata;
use garyx_models::provider::{AgentRunRequest, ImagePayload, StreamEvent};
use serde_json::Value;

use crate::{ThreadEnsureOptions, ThreadStore};

#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        request: AgentRunRequest,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<(), String>;
}

#[async_trait]
pub trait ThreadCreator: Send + Sync {
    async fn create_thread(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        options: ThreadEnsureOptions,
    ) -> Result<(String, Value), String>;
}

#[async_trait]
pub trait InboundSink: Send + Sync {
    async fn try_handle(&self, request: &InboundRequest) -> Option<Result<InboundResult, String>>;
}

pub struct InboundRequest {
    pub channel: String,
    pub account_id: String,
    pub from_id: String,
    pub is_group: bool,
    pub thread_binding_key: String,
    pub message: String,
    pub run_id: String,
    pub reply_to_message_id: Option<String>,
    pub images: Vec<ImagePayload>,
    pub extra_metadata: HashMap<String, Value>,
    /// Local file paths for non-image attachments (documents, voice, video, etc.)
    /// downloaded to disk by the channel handler.
    pub file_paths: Vec<String>,
}

pub struct ThreadMessageRequest {
    pub message: String,
    pub run_id: String,
    pub extra_metadata: HashMap<String, Value>,
    pub images: Vec<ImagePayload>,
    /// Local file paths for non-image attachments (documents, voice, video, etc.)
    /// downloaded to disk by the caller.
    pub file_paths: Vec<String>,
}

#[derive(Debug)]
pub struct InboundResult {
    pub thread_id: String,
    pub metadata: MessageMetadata,
    pub local_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadListEntry {
    pub thread_id: String,
    pub label: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) struct RouteContext<'a> {
    pub(crate) channel: &'a str,
    pub(crate) account_id: &'a str,
    pub(crate) thread_binding_key: &'a str,
    pub(crate) reply_to_message_id: Option<&'a str>,
    pub(crate) extra_metadata: &'a HashMap<String, Value>,
}

#[derive(Clone, Copy)]
pub(crate) struct NavigationContext<'a> {
    pub(crate) channel: &'a str,
    pub(crate) account_id: &'a str,
    pub(crate) thread_binding_key: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) struct DispatchMetadataContext<'a> {
    pub(crate) navigation: NavigationContext<'a>,
    pub(crate) from_id: &'a str,
    pub(crate) is_group: bool,
    pub(crate) reply_to_message_id: Option<&'a str>,
}
