use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;
use garyx_models::provider::{
    ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput, StreamEvent,
};
use serde_json::Value;

// ---------------------------------------------------------------------------
// BridgeError
// ---------------------------------------------------------------------------

/// Errors produced by the bridge layer.
#[derive(thiserror::Error, Debug, Clone)]
pub enum BridgeError {
    #[error("provider not ready")]
    ProviderNotReady,

    #[error("provider not found: {0}")]
    ProviderNotFound(String),

    #[error("run failed: {0}")]
    RunFailed(String),

    #[error("session error: {0}")]
    SessionError(String),

    #[error("timeout")]
    Timeout,

    #[error("internal error: {0}")]
    Internal(String),

    #[error("bridge overloaded: {0}")]
    Overloaded(String),
}

// ---------------------------------------------------------------------------
// Callback type alias
// ---------------------------------------------------------------------------

/// Streaming callback receives structured stream events.
pub type StreamCallback = Box<dyn Fn(StreamEvent) + Send + Sync>;

// ---------------------------------------------------------------------------
// AgentLoopProvider trait
// ---------------------------------------------------------------------------

/// Trait that all agent-loop providers must implement.
///
/// This is the Rust equivalent of `AgentLoopProvider` in
/// `src/garyx/agent_bridge/provider_protocol.py`.
#[async_trait]
pub trait AgentLoopProvider: Send + Sync {
    /// Return the provider type identifier.
    fn provider_type(&self) -> ProviderType;

    /// Whether the provider is ready to accept requests.
    fn is_ready(&self) -> bool;

    /// One-time initialization (connect to server, start subprocess, etc.).
    async fn initialize(&mut self) -> Result<(), BridgeError>;

    /// Graceful shutdown - release resources.
    async fn shutdown(&mut self) -> Result<(), BridgeError>;

    /// Run with streaming: `on_chunk(event)` is called for each
    /// response fragment.
    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError>;

    /// Abort a running request. Returns `true` if the abort was acted upon.
    async fn abort(&self, run_id: &str) -> bool {
        let _ = run_id;
        false
    }

    /// Whether this provider can accept additional user input while an
    /// existing streaming run is still active.
    fn supports_streaming_input(&self) -> bool {
        false
    }

    /// Queue a message for an existing Garyx thread.
    /// Returns `true` if the provider-side session exists and the message was queued.
    async fn add_streaming_input(&self, thread_id: &str, input: QueuedUserInput) -> bool {
        let _ = (thread_id, input);
        false
    }

    /// Interrupt a provider-side session for a Garyx thread gracefully.
    /// Returns `true` if the session was found and interrupted.
    async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        let _ = thread_id;
        false
    }

    /// Get or create a provider-native session ID for a Garyx thread.
    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError>;

    /// Clear / reset provider-side conversation history for a Garyx thread.
    async fn clear_session(&self, thread_id: &str) -> bool {
        let _ = thread_id;
        true
    }
}

/// Metadata attached to a provider for config-based deduplication.
#[derive(Debug, Clone)]
pub struct ProviderMeta {
    /// Deterministic key derived from the provider config (e.g.
    /// `"claude_code:a1b2c3d4"`).
    pub provider_key: String,
    /// Arbitrary metadata carried alongside the provider.
    pub extra: HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// ProviderHealth — per-provider health tracking
// ---------------------------------------------------------------------------

/// Health status for a single provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    /// Provider is functioning normally.
    Healthy,
    /// Provider has experienced recent failures but is still usable.
    Degraded,
    /// Provider is completely unavailable.
    Unavailable,
}

/// Tracks health metrics for a provider over a sliding window.
#[derive(Debug, Clone)]
pub struct ProviderHealth {
    pub provider_key: String,
    pub status: HealthStatus,
    pub total_runs: u64,
    pub successful_runs: u64,
    pub failed_runs: u64,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_success_time: Option<Instant>,
    pub last_failure_time: Option<Instant>,
    pub avg_latency_ms: f64,
}

impl ProviderHealth {
    /// Create a new healthy provider tracker.
    pub fn new(provider_key: impl Into<String>) -> Self {
        Self {
            provider_key: provider_key.into(),
            status: HealthStatus::Healthy,
            total_runs: 0,
            successful_runs: 0,
            failed_runs: 0,
            consecutive_failures: 0,
            last_error: None,
            last_success_time: None,
            last_failure_time: None,
            avg_latency_ms: 0.0,
        }
    }

    /// Record a successful run.
    pub fn record_success(&mut self, latency_ms: f64) {
        self.total_runs += 1;
        self.successful_runs += 1;
        self.consecutive_failures = 0;
        self.last_success_time = Some(Instant::now());
        self.update_latency(latency_ms);
        self.recompute_status();
    }

    /// Record a failed run.
    pub fn record_failure(&mut self, error: &str) {
        self.total_runs += 1;
        self.failed_runs += 1;
        self.consecutive_failures += 1;
        self.last_error = Some(error.to_owned());
        self.last_failure_time = Some(Instant::now());
        self.recompute_status();
    }

    /// Success rate as a fraction (0.0 to 1.0).
    pub fn success_rate(&self) -> f64 {
        if self.total_runs == 0 {
            return 1.0;
        }
        self.successful_runs as f64 / self.total_runs as f64
    }

    fn update_latency(&mut self, latency_ms: f64) {
        if self.successful_runs <= 1 {
            self.avg_latency_ms = latency_ms;
        } else {
            // Exponential moving average (alpha = 0.3)
            self.avg_latency_ms = 0.7 * self.avg_latency_ms + 0.3 * latency_ms;
        }
    }

    fn recompute_status(&mut self) {
        if self.consecutive_failures >= 5 {
            self.status = HealthStatus::Unavailable;
        } else if self.consecutive_failures >= 2 || self.success_rate() < 0.5 {
            self.status = HealthStatus::Degraded;
        } else {
            self.status = HealthStatus::Healthy;
        }
    }
}

#[cfg(test)]
mod tests;
