use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use garyx_models::AgentBindingError;
use garyx_models::messages::MessageMetadata;
use garyx_models::provider::{AgentDispatchOutcome, ImagePayload, StreamEvent};
use serde_json::Value;

use crate::{AdmittedRun, ThreadEnsureOptions, ThreadStore};

#[derive(Debug, thiserror::Error)]
pub enum ThreadCreationError {
    #[error(transparent)]
    AgentBinding(#[from] AgentBindingError),
    #[error("thread store backend failed: {0}")]
    Storage(String),
    #[error("{0}")]
    Other(String),
}

impl From<String> for ThreadCreationError {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}

impl ThreadCreationError {
    pub(crate) fn from_record_creation_error(error: String) -> Self {
        if error.starts_with("workspace_mode=worktree") {
            Self::Other(error)
        } else {
            Self::Storage(error)
        }
    }
}

#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        run: AdmittedRun,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<AgentDispatchOutcome, String>;
}

#[async_trait]
pub trait ThreadCreator: Send + Sync {
    async fn create_thread(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        options: ThreadEnsureOptions,
    ) -> Result<(String, Value), ThreadCreationError>;
}

pub struct InboundRequest {
    pub channel: String,
    pub account_id: String,
    pub from_id: String,
    pub is_group: bool,
    pub thread_binding_key: String,
    pub message: String,
    pub run_id: String,
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
    /// How the bridge absorbed the dispatch. `None` when no bridge dispatch
    /// happened (e.g. the message was answered by a local command).
    pub dispatch_outcome: Option<AgentDispatchOutcome>,
}

#[derive(Clone, Copy)]
pub(crate) struct RouteContext<'a> {
    pub(crate) channel: &'a str,
    pub(crate) account_id: &'a str,
    pub(crate) thread_binding_key: &'a str,
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
}
