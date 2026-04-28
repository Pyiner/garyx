//! Narrow dependency-injection traits used by [`AgentTeamProvider`].
//!
//! The provider deliberately does **not** depend on the concrete
//! `MultiProviderBridge` or any gateway-side registry. Two reasons:
//!
//! 1. **Avoid circular deps.** `garyx-bridge` must not depend on
//!    `garyx-gateway` (the team registry lives in the gateway), and the
//!    bridge itself is the thing that owns and registers this provider —
//!    taking a direct `Arc<MultiProviderBridge>` would tie us back to it.
//! 2. **Testability.** Unit tests for the dispatch loop can implement
//!    these traits with simple mocks that record calls; no thread store,
//!    no child provider lifecycle, no registry boot required.
//!
//! Concrete gateway wiring composes `MultiProviderBridge`, `ThreadStore`,
//! and `AgentTeamStore` behind these traits.

use async_trait::async_trait;
use garyx_models::AgentTeamProfile;
use garyx_models::provider::{ProviderRunOptions, ProviderRunResult};

use crate::provider_trait::{BridgeError, StreamCallback};

/// Dispatches sub-agent work on behalf of an [`AgentTeamProvider`].
///
/// Implementations own the knowledge of:
/// - how to resolve a sub-agent's `provider_type` and create its thread,
/// - which provider to route a child-thread run to and how to drive it.
///
/// The provider only sees this narrow surface, which keeps it free of
/// direct dependencies on `MultiProviderBridge` / `ThreadStore` / gateway
/// types and makes it trivial to unit test with mocks.
#[async_trait]
pub trait SubAgentDispatcher: Send + Sync + 'static {
    /// Ensure a child thread bound to `child_agent_id` exists for
    /// `group_thread_id` and return its thread_id. Called lazily the first
    /// time the provider routes a turn to a given sub-agent for a given
    /// group.
    ///
    /// The implementation must:
    /// - resolve the child's `provider_type` via the agent registry,
    /// - allocate a thread id,
    /// - inherit `workspace_path` from `workspace_path` (parent's workspace),
    /// - persist the thread record.
    async fn ensure_child_thread(
        &self,
        group_thread_id: &str,
        child_agent_id: &str,
        team: &AgentTeamProfile,
        workspace_path: Option<&str>,
    ) -> Result<String, BridgeError>;

    /// Run a streaming turn against `child_thread_id`. The dispatcher looks
    /// up whatever provider is bound to that thread (via its persisted
    /// `provider_type`) and delegates `run_streaming` to it.
    ///
    /// `options.thread_id` is expected to already equal `child_thread_id`;
    /// the caller (the provider) sets this before invoking, so dispatcher
    /// implementations do not need to mutate `options`.
    async fn run_child_streaming(
        &self,
        child_thread_id: &str,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError>;
}

/// Resolves `team_id` to an [`AgentTeamProfile`].
///
/// The profile itself lives in a gateway-side registry (`AgentTeamStore`).
/// `garyx-bridge` must not depend on `garyx-gateway`, so we take the
/// resolver as a trait and let the gateway wire in a concrete
/// `AgentTeamStore`-backed implementation.
#[async_trait]
pub trait TeamProfileResolver: Send + Sync + 'static {
    /// Resolve `team_id` to its profile. Returns `None` if no such team is
    /// registered.
    async fn resolve_team(&self, team_id: &str) -> Option<AgentTeamProfile>;
}
