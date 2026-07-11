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

    #[error("session parse unsupported block: {0}")]
    SessionParseUnsupportedBlock(String),

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderRuntimeSelection {
    pub model: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub model_service_tier: Option<String>,
}

impl ProviderRuntimeSelection {
    pub fn from_metadata(metadata: &HashMap<String, Value>) -> Self {
        Self {
            model: runtime_metadata_string(metadata, "model"),
            model_reasoning_effort: runtime_metadata_string(metadata, "model_reasoning_effort"),
            model_service_tier: runtime_metadata_string(metadata, "model_service_tier"),
        }
    }
}

fn runtime_metadata_string(metadata: &HashMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Model-default fields of an `AgentProviderConfig` that hot-apply onto a
/// live provider instance during a config reload.
///
/// Provider keys intentionally exclude model defaults (see
/// `compute_provider_key`) so thread affinity and persisted SDK session ids
/// stay stable across default-model edits. Reconciling a reload therefore
/// must not recreate the provider; instead these fields are pushed onto the
/// existing instance via [`ProviderRuntime::update_model_defaults`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderModelDefaults {
    pub model: String,
    pub default_model: String,
    pub model_reasoning_effort: String,
    pub model_service_tier: String,
}

impl From<&garyx_models::config::AgentProviderConfig> for ProviderModelDefaults {
    fn from(agent_cfg: &garyx_models::config::AgentProviderConfig) -> Self {
        Self {
            model: agent_cfg.model.clone(),
            default_model: agent_cfg.default_model.clone(),
            model_reasoning_effort: agent_cfg.model_reasoning_effort.clone(),
            model_service_tier: agent_cfg.model_service_tier.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderRuntime trait
// ---------------------------------------------------------------------------

/// Common runtime contract implemented by every provider adapter.
#[async_trait]
pub trait ProviderRuntime: Send + Sync {
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

    /// Resolve the runtime values Garyx will request from the provider for a run.
    /// Providers with config-level defaults should override this so bridge-level
    /// snapshotting observes the same values as the provider request builder.
    fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        ProviderRuntimeSelection::from_metadata(&options.metadata)
    }

    /// Hot-apply reloaded model defaults onto this live provider instance.
    ///
    /// Called when a config reload reconciles onto an already-registered
    /// provider key. Only model-default resolution for future runs may change;
    /// active runs, sessions, and thread affinity are untouched. Providers
    /// without config-level model defaults keep the no-op default.
    fn update_model_defaults(&self, defaults: &ProviderModelDefaults) {
        let _ = defaults;
    }

    /// Abort a running request. Returns `true` if the abort was acted upon.
    async fn abort(&self, run_id: &str) -> bool {
        let _ = run_id;
        false
    }

    /// Consume any provider quota / rate-limit context staged for `thread_id`
    /// when its most recent run terminated because the provider's rolling usage
    /// quota was exhausted. Returns `None` for providers without quota tracking,
    /// or when the last run did not hit a quota limit. The value is consumed
    /// (taken) so it is reported against exactly one terminal run.
    async fn take_rate_limit(
        &self,
        thread_id: &str,
    ) -> Option<garyx_models::provider::ProviderRateLimit> {
        let _ = thread_id;
        None
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
