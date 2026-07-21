use async_trait::async_trait;

use crate::ChannelBinding;
use crate::store::ChannelBindingsMergeAuthority;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointDeliveryTimestampResult {
    Applied,
    Unchanged,
    OwnerChanged { current_holder: Option<String> },
    NotFound,
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
    /// Capability to construct binding-carrying `AtomicRecordMerge`
    /// entries (`AtomicRecordMerge::channel_bindings_merge`).
    ///
    /// Provided for every implementor and not meaningfully overridable:
    /// [`ChannelBindingsMergeAuthority`] has no public constructor, so an
    /// override could only return a value obtained from another mutator.
    /// Implementing this trait IS the declaration of being the serialized
    /// endpoint-binding mutator; ordinary `ThreadStore` callers have no
    /// path to the capability. Fixtures that inject binding state without
    /// being a mutator use the `test-seams`-gated
    /// `ChannelBindingsMergeAuthority::test_authority` seam instead.
    fn binding_merge_authority(&self) -> ChannelBindingsMergeAuthority {
        ChannelBindingsMergeAuthority::mutator_provided()
    }

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

    async fn sync_delivery_timestamp(
        &self,
        _endpoint_key: &str,
        _expected_holder_thread_id: &str,
        _last_delivery_at: Option<String>,
    ) -> Result<EndpointDeliveryTimestampResult, EndpointBindingMutationError> {
        Err(EndpointBindingMutationError::Unavailable)
    }
}
