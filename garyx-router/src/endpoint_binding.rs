use async_trait::async_trait;

use crate::ChannelBinding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointBindingOwner {
    pub thread_id: String,
    pub binding: ChannelBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointBindResult {
    pub thread_id: String,
    pub previous_thread_id: Option<String>,
    pub binding: ChannelBinding,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointDetachResult {
    pub previous_thread_id: Option<String>,
    pub binding: Option<ChannelBinding>,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointBindingMutationError {
    #[error("endpoint binding mutator is unavailable")]
    Unavailable,
    #[error("thread not found: {0}")]
    TargetNotFound(String),
    #[error("thread is archived: {0}")]
    TargetArchived(String),
    #[error("thread lifecycle mutation is in progress: {0}")]
    ThreadLifecycleInProgress(String),
    #[error("projected endpoint owner is unavailable: {0}")]
    PreviousOwnerUnavailable(String),
    #[error("{0}")]
    Incompatible(String),
    #[error("endpoint projection lookup failed: {0}")]
    Projection(String),
    #[error("failed to update thread '{thread_id}': {message}")]
    WriteFailed { thread_id: String, message: String },
}

#[async_trait]
pub trait EndpointBindingMutator: Send + Sync {
    async fn binding_for_endpoint(
        &self,
        endpoint_key: &str,
    ) -> Result<Option<EndpointBindingOwner>, EndpointBindingMutationError>;

    async fn bind_endpoint(
        &self,
        target_thread_id: &str,
        binding: ChannelBinding,
    ) -> Result<EndpointBindResult, EndpointBindingMutationError>;

    async fn detach_endpoint(
        &self,
        endpoint_key: &str,
    ) -> Result<EndpointDetachResult, EndpointBindingMutationError>;
}
