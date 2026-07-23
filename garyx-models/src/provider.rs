use std::collections::HashMap;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    #[serde(alias = "claude", alias = "claude_tty", alias = "claude-tty")]
    ClaudeCode,
    CodexAppServer,
    /// TRAE CLI (`traex`) app-server backend. Forked from Codex and speaks the
    /// identical app-server JSON-RPC protocol, so it reuses the entire Codex
    /// provider pipeline with the `traex` binary.
    #[serde(
        rename = "traex",
        alias = "trae",
        alias = "trae_cli",
        alias = "traecli"
    )]
    Traex,
    /// Google Antigravity CLI (`agy`) provider, driven through the user's local
    /// OAuth-backed CLI session and transcript files.
    #[serde(rename = "antigravity", alias = "agy", alias = "antigravity_cli")]
    AntigravityCli,
    /// xAI Grok Build CLI provider, driven directly over ACP stdio.
    #[serde(rename = "grok_build", alias = "grok", alias = "grok-build")]
    GrokBuild,
}

impl ProviderType {
    pub fn as_slug(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::CodexAppServer => "codex_app_server",
            Self::Traex => "traex",
            Self::AntigravityCli => "antigravity",
            Self::GrokBuild => "grok_build",
        }
    }

    pub fn from_slug(value: &str) -> Option<Self> {
        match value.trim() {
            "claude" | "claude_code" | "claude-tty" | "claude_tty" => Some(Self::ClaudeCode),
            "codex" | "codex_app_server" => Some(Self::CodexAppServer),
            "traex" | "trae" | "trae_cli" | "traecli" => Some(Self::Traex),
            "antigravity" | "agy" | "antigravity_cli" => Some(Self::AntigravityCli),
            "grok" | "grok_build" | "grok-build" => Some(Self::GrokBuild),
            _ => None,
        }
    }
}

/// How the bridge absorbed a dispatched agent message.
///
/// A thread that is already streaming does not start a second concurrent run:
/// the message is queued into the active run as follow-up input, and every
/// downstream consumer (response subscriptions, run bookkeeping, automation
/// activity) must attribute the reply to `effective_run_id`, not the run id
/// the dispatch was requested with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentDispatchOutcome {
    /// A fresh run started under the requested run id.
    Started,
    /// The message was queued into the thread's active streaming run.
    QueuedToActiveRun {
        /// The run that will actually produce the reply.
        effective_run_id: String,
        /// The pending-input record the queued message was staged as.
        pending_input_id: String,
    },
}

impl AgentDispatchOutcome {
    /// The run id that owns the reply, when it differs from the requested one.
    pub fn effective_run_id(&self) -> Option<&str> {
        match self {
            Self::Started => None,
            Self::QueuedToActiveRun {
                effective_run_id, ..
            } => Some(effective_run_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentRunRequest {
    pub thread_id: String,
    pub message: String,
    pub run_id: String,
    pub channel: String,
    pub account_id: String,
    pub metadata: HashMap<String, Value>,
    pub images: Option<Vec<ImagePayload>>,
    pub workspace_dir: Option<String>,
    pub requested_provider: Option<ProviderType>,
}

impl AgentRunRequest {
    pub fn new(
        thread_id: impl Into<String>,
        message: impl Into<String>,
        run_id: impl Into<String>,
        channel: impl Into<String>,
        account_id: impl Into<String>,
        metadata: HashMap<String, Value>,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            message: message.into(),
            run_id: run_id.into(),
            channel: channel.into(),
            account_id: account_id.into(),
            metadata,
            images: None,
            workspace_dir: None,
            requested_provider: None,
        }
    }

    pub fn with_images(mut self, images: Option<Vec<ImagePayload>>) -> Self {
        self.images = images;
        self
    }

    pub fn with_workspace_dir(mut self, workspace_dir: Option<String>) -> Self {
        self.workspace_dir = workspace_dir;
        self
    }

    pub fn with_requested_provider(mut self, requested_provider: Option<ProviderType>) -> Self {
        self.requested_provider = requested_provider;
        self
    }
}

pub const ATTACHMENTS_METADATA_KEY: &str = "attachments";
pub const SDK_SESSION_ID_METADATA_KEY: &str = "sdk_session_id";
/// Per-thread provider overrides chosen at thread creation. Stored in thread
/// metadata under dedicated keys so they win over the agent profile at run
/// time without freezing the agent's own defaults into every thread.
pub const MODEL_OVERRIDE_METADATA_KEY: &str = "model_override";
pub const MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY: &str = "model_reasoning_effort_override";
pub const MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY: &str = "model_service_tier_override";
pub const MODEL_METADATA_KEY: &str = "model";
pub const MODEL_REASONING_EFFORT_METADATA_KEY: &str = "model_reasoning_effort";
pub const MODEL_SERVICE_TIER_METADATA_KEY: &str = "model_service_tier";
/// Inject the thread's model cells into run metadata (single-cell
/// semantics): `metadata.model`, `metadata.model_reasoning_effort`, and
/// `metadata.model_service_tier` on the thread record are THE per-thread
/// runtime cells — "what this thread actually runs". Legacy stored threads
/// may still carry the old dual-track `*_override` keys; those coalesce in
/// front of the cell until a write path migrates them away. Existing run
/// metadata always wins, so explicit request-level values keep priority and
/// agent-profile defaults applied afterwards with the same `or_insert`
/// semantics only fill keys neither the request nor the thread set.
pub fn merge_thread_model_cells(
    thread_data: &serde_json::Value,
    run_metadata: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    let Some(thread_metadata) = thread_data
        .get("metadata")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    let nonempty = |key: &str| {
        thread_metadata
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    for (run_key, legacy_override_key) in [
        (MODEL_METADATA_KEY, MODEL_OVERRIDE_METADATA_KEY),
        (
            MODEL_REASONING_EFFORT_METADATA_KEY,
            MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
        ),
        (
            MODEL_SERVICE_TIER_METADATA_KEY,
            MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY,
        ),
    ] {
        let Some(value) = nonempty(legacy_override_key).or_else(|| nonempty(run_key)) else {
            continue;
        };
        run_metadata
            .entry(run_key.to_owned())
            .or_insert_with(|| serde_json::Value::String(value.to_owned()));
    }
}

pub const SDK_SESSION_FORK_METADATA_KEY: &str = "sdk_session_fork";
pub const FORK_FROM_THREAD_ID_METADATA_KEY: &str = "fork_from_thread_id";
pub const FORK_FROM_SDK_SESSION_ID_METADATA_KEY: &str = "fork_from_sdk_session_id";
pub const FORK_FROM_PROVIDER_TYPE_METADATA_KEY: &str = "fork_from_provider_type";

/// Provider-to-channel streaming boundary markers.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StreamBoundaryKind {
    /// Indicates the upstream SDK acknowledged a queued user message and the
    /// next assistant output should start a fresh outbound segment.
    UserAck,
    /// Indicates the provider started a new assistant text segment and
    /// downstream adapters should finalize the current outbound segment first.
    AssistantSegment,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct QueuedUserInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_input_id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImagePayload>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<PromptAttachment>,
}

impl QueuedUserInput {
    pub fn text(message: impl Into<String>) -> Self {
        Self {
            pending_input_id: None,
            message: message.into(),
            images: Vec::new(),
            attachments: Vec::new(),
        }
    }

    pub fn with_pending_input_id(mut self, pending_input_id: impl Into<String>) -> Self {
        self.pending_input_id = Some(pending_input_id.into());
        self
    }

    pub fn with_attachments(mut self, attachments: Vec<PromptAttachment>) -> Self {
        self.attachments = attachments;
        self
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMessageRole {
    User,
    Assistant,
    System,
    ToolUse,
    ToolResult,
}

fn default_json_null() -> Value {
    Value::Null
}

/// Gary-internal normalized transcript message shared across providers,
/// persistence, API responses, and desktop clients.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProviderMessage {
    pub role: ProviderMessageRole,

    #[serde(default = "default_json_null", skip_serializing_if = "Value::is_null")]
    pub content: Value,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl ProviderMessage {
    pub fn user_text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            role: ProviderMessageRole::User,
            content: Value::String(text.clone()),
            text: Some(text),
            timestamp: None,
            metadata: HashMap::new(),
            tool_use_id: None,
            tool_name: None,
            is_error: None,
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            role: ProviderMessageRole::Assistant,
            content: Value::String(text.clone()),
            text: Some(text),
            timestamp: None,
            metadata: HashMap::new(),
            tool_use_id: None,
            tool_name: None,
            is_error: None,
        }
    }

    pub fn system_text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            role: ProviderMessageRole::System,
            content: Value::String(text.clone()),
            text: Some(text),
            timestamp: None,
            metadata: HashMap::new(),
            tool_use_id: None,
            tool_name: None,
            is_error: None,
        }
    }

    pub fn tool_use(
        content: Value,
        tool_use_id: Option<String>,
        tool_name: Option<String>,
    ) -> Self {
        Self {
            role: ProviderMessageRole::ToolUse,
            content,
            text: None,
            timestamp: None,
            metadata: HashMap::new(),
            tool_use_id,
            tool_name,
            is_error: None,
        }
    }

    pub fn tool_result(
        content: Value,
        tool_use_id: Option<String>,
        tool_name: Option<String>,
        is_error: Option<bool>,
    ) -> Self {
        Self {
            role: ProviderMessageRole::ToolResult,
            content,
            text: None,
            timestamp: None,
            metadata: HashMap::new(),
            tool_use_id,
            tool_name,
            is_error,
        }
    }

    pub fn with_timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.timestamp = Some(timestamp.into());
        self
    }

    pub fn with_metadata_value(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    pub fn role_str(&self) -> &'static str {
        match self.role {
            ProviderMessageRole::User => "user",
            ProviderMessageRole::Assistant => "assistant",
            ProviderMessageRole::System => "system",
            ProviderMessageRole::ToolUse => "tool_use",
            ProviderMessageRole::ToolResult => "tool_result",
        }
    }

    pub fn to_json_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Structured provider streaming event.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Provider-native session id became known for this Garyx thread.
    SessionBound { sdk_session_id: String },
    /// Incremental assistant text.
    Delta { text: String },
    /// Tool invocation started.
    ToolUse { message: ProviderMessage },
    /// Tool invocation finished.
    ToolResult { message: ProviderMessage },
    /// Non-text segment boundary marker.
    Boundary {
        kind: StreamBoundaryKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pending_input_id: Option<String>,
    },
    /// Garyx accepted and persisted a thread title update.
    ThreadTitleUpdated { title: String },
    /// Stream completion marker.
    Done,
}

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

/// Configuration for Claude Code SDK provider.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeCodeConfig {
    #[serde(default = "default_claude_provider_type")]
    pub provider_type: ProviderType,

    /// Claude Agent SDK executable selection. `native` uses the original
    /// Claude Code CLI; `cctty` uses Garyx's optional terminal wrapper.
    #[serde(default = "default_claude_cli_mode")]
    pub claude_cli_mode: String,

    /// Optional explicit executable path for the selected Claude SDK CLI mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_cli_path: Option<String>,

    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub model_reasoning_effort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<i64>,
    #[serde(default)]
    pub timeout_seconds: f64,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    #[serde(default = "crate::config::default_permission_mode")]
    pub permission_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default = "crate::config::default_mcp_base_url")]
    pub mcp_base_url: String,

    /// System prompt to pass to the Claude CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Tools to disallow in the Claude session.
    #[serde(default = "default_disallowed_tools")]
    pub disallowed_tools: Vec<String>,

    /// Max retries on transient errors.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Setting sources passed to the Claude CLI (`--setting-sources`).
    /// Default: `["user", "project", "local"]`.
    /// Set to empty vec to skip loading user settings (useful in tests).
    #[serde(default = "default_setting_sources")]
    pub setting_sources: Vec<String>,
}

fn default_claude_provider_type() -> ProviderType {
    ProviderType::ClaudeCode
}
pub fn default_claude_cli_mode() -> String {
    "native".to_owned()
}
fn default_disallowed_tools() -> Vec<String> {
    vec![
        "EnterPlanMode".to_owned(),
        "ExitPlanMode".to_owned(),
        "AskUserQuestion".to_owned(),
        // The harness `ScheduleWakeup` tool has no firing mechanism when the
        // Claude Code CLI is driven in single-turn (`claude -p`) mode: the
        // schedule is accepted but the process exits at turn end and nothing
        // wakes the agent back up. Agents should use the
        // `mcp__garyx__schedule_followup` MCP tool instead, which goes through
        // the gateway's cron-backed follow-up dispatch.
        "ScheduleWakeup".to_owned(),
    ]
}
fn default_max_retries() -> u32 {
    3
}
fn default_setting_sources() -> Vec<String> {
    vec!["user".to_owned(), "project".to_owned(), "local".to_owned()]
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            provider_type: ProviderType::ClaudeCode,
            claude_cli_mode: default_claude_cli_mode(),
            claude_cli_path: None,
            default_model: String::new(),
            model_reasoning_effort: String::new(),
            max_turns: None,
            timeout_seconds: 0.0,
            env: HashMap::new(),
            permission_mode: crate::config::default_permission_mode(),
            workspace_dir: None,
            mcp_base_url: crate::config::default_mcp_base_url(),
            system_prompt: None,
            disallowed_tools: default_disallowed_tools(),
            max_retries: default_max_retries(),
            setting_sources: default_setting_sources(),
        }
    }
}

/// Configuration for Codex app-server provider.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CodexAppServerConfig {
    #[serde(default = "default_codex_provider_type")]
    pub provider_type: ProviderType,

    #[serde(default)]
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<i64>,
    #[serde(default)]
    pub timeout_seconds: f64,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default = "crate::config::default_mcp_base_url")]
    pub mcp_base_url: String,
    #[serde(default)]
    pub codex_bin: String,
    #[serde(default = "default_approval_policy")]
    pub approval_policy: String,
    #[serde(default = "default_sandbox_mode")]
    pub sandbox_mode: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_reasoning_effort: String,
    #[serde(default)]
    pub model_service_tier: String,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: f64,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_seconds: f64,
    #[serde(default)]
    pub experimental_api: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

fn default_codex_provider_type() -> ProviderType {
    ProviderType::CodexAppServer
}
fn default_approval_policy() -> String {
    "never".to_owned()
}
fn default_sandbox_mode() -> String {
    "danger-full-access".to_owned()
}
fn default_request_timeout() -> f64 {
    300.0
}
fn default_startup_timeout() -> f64 {
    300.0
}

impl Default for CodexAppServerConfig {
    fn default() -> Self {
        Self {
            provider_type: ProviderType::CodexAppServer,
            default_model: String::new(),
            max_turns: None,
            timeout_seconds: 0.0,
            workspace_dir: None,
            mcp_base_url: crate::config::default_mcp_base_url(),
            codex_bin: String::new(),
            approval_policy: default_approval_policy(),
            sandbox_mode: default_sandbox_mode(),
            model: String::new(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
            request_timeout_seconds: default_request_timeout(),
            startup_timeout_seconds: default_startup_timeout(),
            experimental_api: false,
            env: HashMap::new(),
        }
    }
}

/// Configuration for Google Antigravity CLI provider.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AntigravityCliConfig {
    #[serde(default = "default_antigravity_provider_type")]
    pub provider_type: ProviderType,

    #[serde(default = "default_antigravity_model")]
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<i64>,
    #[serde(default)]
    pub timeout_seconds: f64,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub antigravity_bin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub antigravity_brain_root: String,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

fn default_antigravity_provider_type() -> ProviderType {
    ProviderType::AntigravityCli
}

pub fn default_antigravity_model() -> String {
    "Claude Opus 4.6 (Thinking)".to_owned()
}

impl Default for AntigravityCliConfig {
    fn default() -> Self {
        Self {
            provider_type: ProviderType::AntigravityCli,
            default_model: default_antigravity_model(),
            max_turns: None,
            timeout_seconds: 0.0,
            workspace_dir: None,
            antigravity_bin: String::new(),
            antigravity_brain_root: String::new(),
            model: String::new(),
            env: HashMap::new(),
        }
    }
}

/// Configuration for the native Grok Build ACP provider.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GrokBuildConfig {
    #[serde(default = "default_grok_provider_type")]
    pub provider_type: ProviderType,

    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_reasoning_effort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<i64>,
    #[serde(default)]
    pub timeout_seconds: f64,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub grok_bin: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

fn default_grok_provider_type() -> ProviderType {
    ProviderType::GrokBuild
}

impl Default for GrokBuildConfig {
    fn default() -> Self {
        Self {
            provider_type: ProviderType::GrokBuild,
            default_model: String::new(),
            model: String::new(),
            model_reasoning_effort: String::new(),
            max_turns: None,
            timeout_seconds: 0.0,
            workspace_dir: None,
            grok_bin: String::new(),
            env: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Image payload
// ---------------------------------------------------------------------------

/// An image payload attached to a provider run request.
///
/// Carries base64-encoded image data together with a MIME type so that
/// providers can forward images to the underlying model API.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ImagePayload {
    /// Best-effort original filename for the image attachment.
    #[serde(default)]
    pub name: String,

    /// Base64-encoded image data.
    #[serde(default)]
    pub data: String,

    /// MIME type (e.g. "image/png", "image/jpeg").
    #[serde(default = "default_image_media_type")]
    pub media_type: String,
}

fn default_image_media_type() -> String {
    "image/jpeg".to_owned()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FilePayload {
    /// Best-effort original filename for the file attachment.
    #[serde(default)]
    pub name: String,

    /// Base64-encoded file data.
    #[serde(default)]
    pub data: String,

    /// MIME type when known.
    #[serde(default)]
    pub media_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PromptAttachmentKind {
    Image,
    File,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PromptAttachment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    pub kind: PromptAttachmentKind,
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub media_type: String,
}

pub fn attachments_to_metadata_value(attachments: &[PromptAttachment]) -> Value {
    serde_json::to_value(attachments).unwrap_or_else(|_| Value::Array(Vec::new()))
}

pub fn attachments_from_metadata(metadata: &HashMap<String, Value>) -> Vec<PromptAttachment> {
    metadata
        .get(ATTACHMENTS_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<PromptAttachment>>(value).ok())
        .unwrap_or_default()
}

fn prompt_attachment_display_name(attachment: &PromptAttachment) -> String {
    let trimmed = attachment.name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }
    Path::new(&attachment.path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| attachment.path.clone())
}

fn prompt_attachment_instruction(attachment: &PromptAttachment) -> String {
    let label = prompt_attachment_display_name(attachment);
    match attachment.kind {
        PromptAttachmentKind::Image => format!(
            "Read this image file from disk: {} (name: {})",
            attachment.path, label
        ),
        PromptAttachmentKind::File => format!(
            "Read this file from disk: {} (name: {})",
            attachment.path, label
        ),
    }
}

pub fn build_prompt_message_with_attachments(
    message: &str,
    attachments: &[PromptAttachment],
) -> String {
    if attachments.is_empty() {
        return message.to_owned();
    }

    let attachment_lines = attachments
        .iter()
        .map(prompt_attachment_instruction)
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = message.trim();
    if trimmed.is_empty() {
        format!(
            "Use the attached local files as part of this turn.\n\n{}",
            attachment_lines
        )
    } else {
        format!(
            "{trimmed}\n\nUse the attached local files as part of this turn.\n\n{}",
            attachment_lines
        )
    }
}

fn attachment_content_block(attachment: &PromptAttachment) -> Value {
    let type_name = match attachment.kind {
        PromptAttachmentKind::Image => "image",
        PromptAttachmentKind::File => "file",
    };
    serde_json::json!({
        "type": type_name,
        "path": attachment.path,
        "name": prompt_attachment_display_name(attachment),
        "media_type": attachment.media_type,
    })
}

pub fn build_user_content_from_parts(
    user_message: &str,
    attachments: &[PromptAttachment],
    user_images: &[ImagePayload],
) -> Value {
    if attachments.is_empty() && user_images.is_empty() {
        return Value::String(user_message.to_owned());
    }

    let mut blocks = Vec::with_capacity(
        attachments.len() + user_images.len() + usize::from(!user_message.trim().is_empty()),
    );
    if !user_message.trim().is_empty() {
        blocks.push(serde_json::json!({
            "type": "text",
            "text": user_message,
        }));
    }

    if !attachments.is_empty() {
        blocks.extend(attachments.iter().map(attachment_content_block));
    } else {
        for (index, image) in user_images.iter().enumerate() {
            if image.data.trim().is_empty() {
                continue;
            }
            blocks.push(serde_json::json!({
                "type": "image",
                "name": sanitized_image_name_or_fallback(&image.name, index, &image.media_type),
                "source": {
                    "type": "base64",
                    "media_type": image.media_type,
                    "data": image.data,
                }
            }));
        }
    }

    if blocks.is_empty() {
        Value::String(user_message.to_owned())
    } else {
        Value::Array(blocks)
    }
}

pub fn file_attachments_from_paths(paths: &[String]) -> Vec<PromptAttachment> {
    paths
        .iter()
        .filter_map(|path| {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(PromptAttachment {
                attachment_id: None,
                kind: PromptAttachmentKind::File,
                path: trimmed.to_owned(),
                name: Path::new(trimmed)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| trimmed.to_owned()),
                media_type: String::new(),
            })
        })
        .collect()
}

fn image_extension_for_media_type(media_type: &str) -> &'static str {
    match media_type.trim() {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

fn sanitized_image_name_or_fallback(
    raw_name: &str,
    fallback_index: usize,
    media_type: &str,
) -> String {
    let trimmed = raw_name.trim();
    let candidate = if trimmed.is_empty() {
        String::new()
    } else {
        Path::new(trimmed)
            .file_name()
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    };
    if !candidate.trim().is_empty() {
        return candidate;
    }
    format!(
        "image-{}.{}",
        fallback_index + 1,
        image_extension_for_media_type(media_type)
    )
}

fn sanitized_file_name_or_fallback(raw_name: &str, fallback_index: usize) -> String {
    let trimmed = raw_name.trim();
    let candidate = if trimmed.is_empty() {
        String::new()
    } else {
        Path::new(trimmed)
            .file_name()
            .and_then(|value| value.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    };
    if !candidate.trim().is_empty() {
        return candidate;
    }
    format!("file-{}", fallback_index + 1)
}

pub fn stage_image_payloads_for_prompt(
    namespace: &str,
    images: &[ImagePayload],
) -> Vec<PromptAttachment> {
    if images.is_empty() {
        return Vec::new();
    }

    let root = std::env::temp_dir()
        .join(namespace)
        .join("prompt-attachments");
    if std::fs::create_dir_all(&root).is_err() {
        return Vec::new();
    }

    images
        .iter()
        .enumerate()
        .filter_map(|(index, image)| {
            let encoded = image.data.trim();
            if encoded.is_empty() {
                return None;
            }
            let bytes = BASE64.decode(encoded).ok()?;
            let name = sanitized_image_name_or_fallback(&image.name, index, &image.media_type);
            let path = root.join(format!("{}-{}", Uuid::new_v4(), name));
            std::fs::write(&path, bytes).ok()?;
            Some(PromptAttachment {
                attachment_id: None,
                kind: PromptAttachmentKind::Image,
                path: path.to_string_lossy().into_owned(),
                name,
                media_type: image.media_type.clone(),
            })
        })
        .collect()
}

pub fn stage_file_payloads_for_prompt(
    namespace: &str,
    files: &[FilePayload],
) -> Vec<PromptAttachment> {
    if files.is_empty() {
        return Vec::new();
    }

    let root = std::env::temp_dir()
        .join(namespace)
        .join("prompt-attachments");
    if std::fs::create_dir_all(&root).is_err() {
        return Vec::new();
    }

    files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            let encoded = file.data.trim();
            if encoded.is_empty() {
                return None;
            }
            let bytes = BASE64.decode(encoded).ok()?;
            let name = sanitized_file_name_or_fallback(&file.name, index);
            let path = root.join(format!("{}-{}", Uuid::new_v4(), name));
            std::fs::write(&path, bytes).ok()?;
            Some(PromptAttachment {
                attachment_id: None,
                kind: PromptAttachmentKind::File,
                path: path.to_string_lossy().into_owned(),
                name,
                media_type: file.media_type.clone(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Provider-facing run options / result (simplified vs agent.rs)
// ---------------------------------------------------------------------------

/// Options for starting a provider-level agent run.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProviderRunOptions {
    pub thread_id: String,
    pub message: String,

    /// Workspace directory override for this run.
    ///
    /// When set, providers should prefer this path over any config-level
    /// default workspace/cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,

    /// Image payloads to forward to the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImagePayload>>,

    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

/// Provider usage-quota / rate-limit context captured when a run terminates
/// because the provider's rolling quota was exhausted (e.g. Codex's 5-hour or
/// weekly ChatGPT-plan window). Sourced from the provider's own structured
/// signal — for Codex, the `account/rateLimits/updated` snapshot plus the
/// `usageLimitExceeded` error — so detection does not rely on error-text
/// matching. The gateway turns this into a `run_complete` control record and an
/// automatic resend scheduled at `reset_at`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderRateLimit {
    /// Provider identity, e.g. `codex`.
    pub provider: String,
    /// ISO 8601 timestamp when the exhausted window resets, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
    /// Which rolling window was exhausted: `primary` (5-hour session) or
    /// `secondary` (weekly allowance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    /// Percent of the exhausted window consumed (0-100), when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<i64>,
    /// Provider-reported reason classifier, e.g. `usage_limit_reached`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reached_type: Option<String>,
    /// Human-readable detail reported by the provider, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Result of a provider-level agent run.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProviderRunResult {
    pub run_id: String,
    pub thread_id: String,

    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub session_messages: Vec<ProviderMessage>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_title: Option<String>,

    #[serde(default = "crate::config::default_true")]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub duration_ms: i64,
}

#[cfg(test)]
mod tests;
