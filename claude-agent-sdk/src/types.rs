use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::control::CanUseToolRequest;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type CanUseToolFuture = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'static>>;
pub type CanUseToolCallback =
    Arc<dyn Fn(CanUseToolRequest) -> CanUseToolFuture + Send + Sync + 'static>;

/// Provenance attached to user-role messages and their result frames.
///
/// Claude Code adds origin variants over time, so only the stable `kind`
/// discriminator is typed and all variant-specific fields are preserved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageOrigin {
    pub kind: String,
    #[serde(flatten)]
    pub metadata: HashMap<String, Value>,
}

impl MessageOrigin {
    pub fn is_task_notification(&self) -> bool {
        self.kind == "task-notification"
    }
}

// ---------------------------------------------------------------------------
// Permission modes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Auto,
    Plan,
    BypassPermissions,
    DontAsk,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::AcceptEdits => write!(f, "acceptEdits"),
            Self::Auto => write!(f, "auto"),
            Self::Plan => write!(f, "plan"),
            Self::BypassPermissions => write!(f, "bypassPermissions"),
            Self::DontAsk => write!(f, "dontAsk"),
        }
    }
}

// ---------------------------------------------------------------------------
// Content blocks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextBlock {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThinkingBlock {
    pub thinking: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolUseBlock {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultBlock {
    pub tool_use_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageBlock {
    pub source: ImageSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DocumentSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentBlock {
    pub source: DocumentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnknownContentBlock {
    pub block_type: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text(TextBlock),
    Image(ImageBlock),
    Document(DocumentBlock),
    Thinking(ThinkingBlock),
    ToolUse(ToolUseBlock),
    ToolResult(ToolResultBlock),
    Unknown(UnknownContentBlock),
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantMessageError {
    AuthenticationFailed,
    OauthOrgNotAllowed,
    BillingError,
    RateLimit,
    Overloaded,
    InvalidRequest,
    ModelNotFound,
    ServerError,
    MaxOutputTokens,
    /// Catch-all so error categories added by newer CLIs degrade to `Unknown`
    /// instead of dropping the classification entirely.
    #[serde(other)]
    Unknown,
}

impl AssistantMessageError {
    /// Stable snake_case label for logs and run error messages.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::AuthenticationFailed => "authentication_failed",
            Self::OauthOrgNotAllowed => "oauth_org_not_allowed",
            Self::BillingError => "billing_error",
            Self::RateLimit => "rate_limit",
            Self::Overloaded => "overloaded",
            Self::InvalidRequest => "invalid_request",
            Self::ModelNotFound => "model_not_found",
            Self::ServerError => "server_error",
            Self::MaxOutputTokens => "max_output_tokens",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserMessage {
    pub content: UserContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<MessageOrigin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AssistantMessageError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemMessage {
    pub subtype: String,
    /// The full raw data of the system message.
    pub data: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResultMessage {
    pub subtype: String,
    pub duration_ms: i64,
    pub duration_api_ms: i64,
    pub is_error: bool,
    pub num_turns: i64,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<MessageOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<HashMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<Value>,
    /// API stop reason for the final turn, when reported (open string — new
    /// values ship on the wire ahead of schema updates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Machine-readable terminal classification for the run (e.g.
    /// `completed`, `max_turns`, `blocking_limit`, `aborted_streaming`).
    /// Open string: newer CLIs add reasons ahead of schema updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<String>,
    /// HTTP status of the failing API request, when the run died on an API
    /// error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_error_status: Option<i64>,
    /// Human-readable error strings for error results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Tool invocations denied by permission evaluation during the run. Kept
    /// as raw JSON objects (`tool_name`, `tool_use_id`, `tool_input`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_denials: Vec<Value>,
    /// Per-model usage breakdown keyed by model id (tokens, cache reads and
    /// writes, cost, context window). Kept as raw JSON for tolerance.
    #[serde(
        rename = "modelUsage",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub model_usage: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub uuid: String,
    pub session_id: String,
    pub event: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    User(UserMessage),
    Assistant(AssistantMessage),
    System(SystemMessage),
    /// Boxed: the result frame carries the full terminal accounting and is
    /// much larger than the streaming variants exchanged per message.
    Result(Box<ResultMessage>),
    StreamEvent(StreamEvent),
}

impl Message {
    /// Returns `true` if this is a [`ResultMessage`].
    pub fn is_result(&self) -> bool {
        matches!(self, Self::Result(_))
    }

    pub fn as_result(&self) -> Option<&ResultMessage> {
        match self {
            Self::Result(r) => Some(r.as_ref()),
            _ => None,
        }
    }

    pub fn as_assistant(&self) -> Option<&AssistantMessage> {
        match self {
            Self::Assistant(a) => Some(a),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// MCP server configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

// ---------------------------------------------------------------------------
// Agent options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ClaudeAgentDefinition {
    pub description: String,
    pub prompt: String,
}

#[derive(Clone)]
pub struct ClaudeAgentOptions {
    pub agent: Option<String>,
    pub agents: HashMap<String, ClaudeAgentDefinition>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub mcp_servers: HashMap<String, McpServerConfig>,
    pub permission_mode: Option<PermissionMode>,
    pub continue_conversation: bool,
    pub resume: Option<String>,
    pub max_turns: Option<i64>,
    pub max_budget_usd: Option<f64>,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    pub permission_prompt_tool_name: Option<String>,
    pub cwd: Option<PathBuf>,
    pub cli_path: Option<PathBuf>,
    pub cli_prefix_args: Vec<String>,
    pub env: HashMap<String, String>,
    pub extra_args: HashMap<String, Option<String>>,
    pub max_buffer_size: Option<usize>,
    pub setting_sources: Option<Vec<String>>,
    pub include_partial_messages: bool,
    pub fork_session: bool,
    pub max_thinking_tokens: Option<i64>,
    pub output_format: Option<Value>,
    pub enable_file_checkpointing: bool,
    pub can_use_tool: Option<CanUseToolCallback>,
}

impl std::fmt::Debug for ClaudeAgentOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeAgentOptions")
            .field("agent", &self.agent)
            .field("agents", &self.agents)
            .field("system_prompt", &self.system_prompt)
            .field("append_system_prompt", &self.append_system_prompt)
            .field("mcp_servers", &self.mcp_servers)
            .field("permission_mode", &self.permission_mode)
            .field("continue_conversation", &self.continue_conversation)
            .field("resume", &self.resume)
            .field("max_turns", &self.max_turns)
            .field("max_budget_usd", &self.max_budget_usd)
            .field("allowed_tools", &self.allowed_tools)
            .field("disallowed_tools", &self.disallowed_tools)
            .field("model", &self.model)
            .field("fallback_model", &self.fallback_model)
            .field(
                "permission_prompt_tool_name",
                &self.permission_prompt_tool_name,
            )
            .field("cwd", &self.cwd)
            .field("cli_path", &self.cli_path)
            .field("cli_prefix_args", &self.cli_prefix_args)
            .field("env", &self.env)
            .field("extra_args", &self.extra_args)
            .field("max_buffer_size", &self.max_buffer_size)
            .field("setting_sources", &self.setting_sources)
            .field("include_partial_messages", &self.include_partial_messages)
            .field("fork_session", &self.fork_session)
            .field("max_thinking_tokens", &self.max_thinking_tokens)
            .field("output_format", &self.output_format)
            .field("enable_file_checkpointing", &self.enable_file_checkpointing)
            .field(
                "can_use_tool",
                &self.can_use_tool.as_ref().map(|_| "<handler>"),
            )
            .finish()
    }
}

impl Default for ClaudeAgentOptions {
    fn default() -> Self {
        Self {
            agent: None,
            agents: HashMap::new(),
            system_prompt: None,
            append_system_prompt: None,
            mcp_servers: HashMap::new(),
            permission_mode: None,
            continue_conversation: false,
            resume: None,
            max_turns: None,
            max_budget_usd: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            model: None,
            fallback_model: None,
            permission_prompt_tool_name: None,
            cwd: None,
            cli_path: None,
            cli_prefix_args: Vec::new(),
            env: HashMap::new(),
            extra_args: HashMap::new(),
            max_buffer_size: None,
            setting_sources: Some(vec![
                "user".to_string(),
                "project".to_string(),
                "local".to_string(),
            ]),
            include_partial_messages: false,
            fork_session: false,
            max_thinking_tokens: None,
            output_format: None,
            enable_file_checkpointing: false,
            can_use_tool: None,
        }
    }
}

impl ClaudeAgentOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert options into CLI arguments for the `claude` binary.
    pub fn to_cli_args(&self) -> Vec<String> {
        let mut args = vec![
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
        ];

        if !self.agents.is_empty() {
            let agents_json = self
                .agents
                .iter()
                .map(|(agent_id, definition)| {
                    (
                        agent_id.clone(),
                        serde_json::json!({
                            "description": definition.description,
                            "prompt": definition.prompt,
                        }),
                    )
                })
                .collect::<serde_json::Map<String, Value>>();
            args.push("--agents".into());
            args.push(Value::Object(agents_json).to_string());
        }

        if let Some(agent) = &self.agent {
            args.push("--agent".into());
            args.push(agent.clone());
        }

        match (&self.system_prompt, &self.append_system_prompt) {
            (Some(sp), _) => {
                args.push("--system-prompt".into());
                args.push(sp.clone());
            }
            (None, Some(append)) => {
                args.push("--append-system-prompt".into());
                args.push(append.clone());
            }
            (None, None) => {}
        }

        if !self.allowed_tools.is_empty() {
            args.push("--allowedTools".into());
            args.push(self.allowed_tools.join(","));
        }

        if let Some(max_turns) = self.max_turns {
            args.push("--max-turns".into());
            args.push(max_turns.to_string());
        }

        if let Some(budget) = self.max_budget_usd {
            args.push("--max-budget-usd".into());
            args.push(budget.to_string());
        }

        if !self.disallowed_tools.is_empty() {
            args.push("--disallowedTools".into());
            args.push(self.disallowed_tools.join(","));
        }

        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }

        if let Some(fallback) = &self.fallback_model {
            args.push("--fallback-model".into());
            args.push(fallback.clone());
        }

        if self.can_use_tool.is_some() {
            args.push("--permission-prompt-tool".into());
            args.push("stdio".into());
        } else if let Some(tool_name) = &self.permission_prompt_tool_name {
            args.push("--permission-prompt-tool".into());
            args.push(tool_name.clone());
        }

        if let Some(mode) = &self.permission_mode {
            args.push("--permission-mode".into());
            args.push(mode.to_string());
        }

        if self.continue_conversation {
            args.push("--continue".into());
        }

        if let Some(session) = &self.resume {
            args.push("--resume".into());
            args.push(session.clone());
        }

        // MCP servers
        if !self.mcp_servers.is_empty() {
            let servers_json = serde_json::json!({ "mcpServers": &self.mcp_servers });
            args.push("--mcp-config".into());
            args.push(servers_json.to_string());
        }

        if self.include_partial_messages {
            args.push("--include-partial-messages".into());
        }

        if self.fork_session {
            args.push("--fork-session".into());
        }

        if let Some(sources) = &self.setting_sources {
            args.push("--setting-sources".into());
            args.push(sources.join(","));
        }

        // Extra args
        for (flag, value) in &self.extra_args {
            match value {
                None => args.push(format!("--{flag}")),
                Some(v) => {
                    args.push(format!("--{flag}"));
                    args.push(v.clone());
                }
            }
        }

        if let Some(tokens) = self.max_thinking_tokens {
            args.push("--max-thinking-tokens".into());
            args.push(tokens.to_string());
        }

        // Output format / JSON schema
        if let Some(fmt) = &self.output_format
            && fmt.get("type").and_then(|t| t.as_str()) == Some("json_schema")
            && let Some(schema) = fmt.get("schema")
        {
            args.push("--json-schema".into());
            args.push(schema.to_string());
        }

        args
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
