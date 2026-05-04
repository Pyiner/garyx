use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text(TextBlock),
    Image(ImageBlock),
    Thinking(ThinkingBlock),
    ToolUse(ToolUseBlock),
    ToolResult(ToolResultBlock),
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantMessageError {
    AuthenticationFailed,
    BillingError,
    RateLimit,
    InvalidRequest,
    ServerError,
    MaxOutputTokens,
    Unknown,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultMessage {
    pub subtype: String,
    pub duration_ms: i64,
    pub duration_api_ms: i64,
    pub is_error: bool,
    pub num_turns: i64,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<HashMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<Value>,
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
    Result(ResultMessage),
    StreamEvent(StreamEvent),
}

impl Message {
    /// Returns `true` if this is a [`ResultMessage`].
    pub fn is_result(&self) -> bool {
        matches!(self, Self::Result(_))
    }

    pub fn as_result(&self) -> Option<&ResultMessage> {
        match self {
            Self::Result(r) => Some(r),
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

#[derive(Debug, Clone)]
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
    pub env: HashMap<String, String>,
    pub extra_args: HashMap<String, Option<String>>,
    pub max_buffer_size: Option<usize>,
    pub setting_sources: Option<Vec<String>>,
    pub include_partial_messages: bool,
    pub fork_session: bool,
    pub max_thinking_tokens: Option<i64>,
    pub output_format: Option<Value>,
    pub enable_file_checkpointing: bool,
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

        // System prompt
        match (&self.system_prompt, &self.append_system_prompt) {
            (Some(sp), _) => {
                args.push("--system-prompt".into());
                args.push(sp.clone());
            }
            (None, Some(append)) => {
                args.push("--append-system-prompt".into());
                args.push(append.clone());
            }
            (None, None) => {
                args.push("--system-prompt".into());
                args.push(String::new());
            }
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

        if let Some(tool_name) = &self.permission_prompt_tool_name {
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
