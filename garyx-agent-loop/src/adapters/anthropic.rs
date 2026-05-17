//! Anthropic Messages adapter for the model-neutral agent loop.
//!
//! Authentication is injected through [`AnthropicAuthProvider`]. The adapter
//! only knows Anthropic's wire protocol; host applications own credential
//! lookup, config, and provider routing.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    AgentLoopError, ConversationMessage, ConversationRole, LlmAdapter, LlmOutput, LlmRequest,
    LlmResponse, LlmRuntimeContext, LlmToolCall, ModelVendor, ToolDefinition,
};

pub const ANTHROPIC_MESSAGES_BASE_URL: &str = "https://api.anthropic.com/v1";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: i64 = 8192;
const CLAUDE_CODE_VERSION: &str = "2.1.143";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnthropicCredential {
    ApiKey(String),
    BearerToken(String),
}

impl AnthropicCredential {
    pub fn is_bearer_token(&self) -> bool {
        matches!(self, Self::BearerToken(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicAuth {
    pub credential: AnthropicCredential,
    pub base_url: String,
    pub version: String,
    pub beta: Option<String>,
}

impl AnthropicAuth {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            credential: AnthropicCredential::ApiKey(api_key.into()),
            base_url: ANTHROPIC_MESSAGES_BASE_URL.to_owned(),
            version: ANTHROPIC_VERSION.to_owned(),
            beta: None,
        }
    }

    pub fn bearer_token(token: impl Into<String>) -> Self {
        Self {
            credential: AnthropicCredential::BearerToken(token.into()),
            base_url: ANTHROPIC_MESSAGES_BASE_URL.to_owned(),
            version: ANTHROPIC_VERSION.to_owned(),
            beta: None,
        }
    }
}

#[async_trait]
pub trait AnthropicAuthProvider: Send + Sync {
    async fn resolve_auth(
        &self,
        runtime: &LlmRuntimeContext,
    ) -> Result<AnthropicAuth, AgentLoopError>;
}

pub struct StaticAnthropicAuthProvider {
    auth: AnthropicAuth,
}

impl StaticAnthropicAuthProvider {
    pub fn new(auth: AnthropicAuth) -> Self {
        Self { auth }
    }
}

#[async_trait]
impl AnthropicAuthProvider for StaticAnthropicAuthProvider {
    async fn resolve_auth(
        &self,
        _runtime: &LlmRuntimeContext,
    ) -> Result<AnthropicAuth, AgentLoopError> {
        Ok(self.auth.clone())
    }
}

pub fn messages_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/messages") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/messages")
    }
}

pub struct AnthropicMessagesAdapter {
    auth_provider: Arc<dyn AnthropicAuthProvider>,
    http: reqwest::Client,
}

impl AnthropicMessagesAdapter {
    pub fn new(auth_provider: Arc<dyn AnthropicAuthProvider>) -> Self {
        Self {
            auth_provider,
            http: reqwest::Client::new(),
        }
    }

    fn provider_message_text(message: &ConversationMessage) -> Option<String> {
        message
            .text
            .clone()
            .or_else(|| message.content.as_str().map(ToOwned::to_owned))
            .or_else(|| {
                (!message.content.is_null())
                    .then(|| serde_json::to_string(&message.content).ok())
                    .flatten()
            })
    }

    fn tool_call_name(message: &ConversationMessage) -> String {
        message
            .tool_name
            .clone()
            .or_else(|| {
                message
                    .content
                    .get("name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_default()
    }

    fn tool_call_input(message: &ConversationMessage) -> Value {
        message
            .content
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| message.content.clone())
    }

    fn tool_result_content(message: &ConversationMessage) -> Value {
        Self::provider_message_text(message)
            .map(Value::String)
            .unwrap_or_else(|| message.content.clone())
    }

    fn push_message(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
        if blocks.is_empty() {
            return;
        }
        if let Some(last) = messages.last_mut()
            && last.get("role").and_then(Value::as_str) == Some(role)
            && let Some(content) = last.get_mut("content").and_then(Value::as_array_mut)
        {
            content.extend(blocks);
            return;
        }
        messages.push(json!({
            "role": role,
            "content": blocks,
        }));
    }

    fn conversation_payload(request: &LlmRequest) -> (Option<String>, Vec<Value>) {
        let mut system_parts = Vec::<String>::new();
        if !request.instructions.trim().is_empty() {
            system_parts.push(request.instructions.trim().to_owned());
        }

        let mut messages = Vec::<Value>::new();
        for message in &request.messages {
            match message.role {
                ConversationRole::System => {
                    if let Some(text) = Self::provider_message_text(message)
                        && !text.trim().is_empty()
                    {
                        system_parts.push(text);
                    }
                }
                ConversationRole::User => {
                    let Some(text) = Self::provider_message_text(message) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    Self::push_message(
                        &mut messages,
                        "user",
                        vec![json!({ "type": "text", "text": text })],
                    );
                }
                ConversationRole::Assistant => {
                    let Some(text) = Self::provider_message_text(message) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    Self::push_message(
                        &mut messages,
                        "assistant",
                        vec![json!({ "type": "text", "text": text })],
                    );
                }
                ConversationRole::ToolUse => {
                    let name = Self::tool_call_name(message);
                    if name.is_empty() {
                        continue;
                    }
                    let id = message
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| "toolu_garyx_missing_id".to_owned());
                    Self::push_message(
                        &mut messages,
                        "assistant",
                        vec![json!({
                            "type": "tool_use",
                            "id": normalize_tool_call_id(&id),
                            "name": name,
                            "input": Self::tool_call_input(message),
                        })],
                    );
                }
                ConversationRole::ToolResult => {
                    let Some(id) = message.tool_call_id.as_deref() else {
                        continue;
                    };
                    let mut block = json!({
                        "type": "tool_result",
                        "tool_use_id": normalize_tool_call_id(id),
                        "content": Self::tool_result_content(message),
                    });
                    if message.is_error.unwrap_or(false) {
                        block["is_error"] = Value::Bool(true);
                    }
                    Self::push_message(&mut messages, "user", vec![block]);
                }
            }
        }

        let system = (!system_parts.is_empty()).then(|| system_parts.join("\n\n"));
        (system, messages)
    }

    fn tool_schema(tool: &ToolDefinition) -> Value {
        json!({
            "name": tool.name,
            "description": tool.description,
            "input_schema": tool.parameters,
        })
    }

    pub fn request_body(request: &LlmRequest) -> Value {
        Self::request_body_with_identity(request, false)
    }

    fn request_body_with_identity(request: &LlmRequest, claude_code_identity: bool) -> Value {
        let (system, messages) = Self::conversation_payload(request);
        let mut body = json!({
            "model": request.model,
            "max_tokens": DEFAULT_MAX_TOKENS,
            "messages": messages,
        });
        let system = if claude_code_identity {
            Some(match system {
                Some(system) => {
                    format!("You are Claude Code, Anthropic's official CLI for Claude.\n\n{system}")
                }
                None => "You are Claude Code, Anthropic's official CLI for Claude.".to_owned(),
            })
        } else {
            system
        };
        if let Some(system) = system {
            body["system"] = Value::String(system);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(request.tools.iter().map(Self::tool_schema).collect());
            body["tool_choice"] = json!({ "type": "auto" });
        }
        if let Some(effort) = request.options.reasoning_effort.as_deref() {
            apply_reasoning_options(&mut body, &request.model, effort);
        }
        if let Some(service_tier) = request.options.service_tier.as_deref()
            && !service_tier.trim().is_empty()
        {
            body["service_tier"] = Value::String(service_tier.trim().to_owned());
        }
        body
    }

    pub fn parse_response(value: Value) -> LlmResponse {
        let mut outputs = Vec::new();
        if let Some(content) = value.get("content").and_then(Value::as_array) {
            for block in content {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            outputs.push(LlmOutput::Text(text.to_owned()));
                        }
                    }
                    Some("tool_use") => {
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if name.is_empty() {
                            continue;
                        }
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| "toolu_garyx_missing_id".to_owned());
                        outputs.push(LlmOutput::ToolCall(LlmToolCall {
                            id,
                            name,
                            arguments: block.get("input").cloned().unwrap_or(Value::Null),
                            metadata: Default::default(),
                        }));
                    }
                    _ => {}
                }
            }
        }

        let usage = value.get("usage").unwrap_or(&Value::Null);
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let cache_created = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        LlmResponse {
            outputs,
            input_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_i64)
                .unwrap_or_default()
                + cache_read
                + cache_created,
            output_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            actual_model: value
                .get("model")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }
    }
}

#[async_trait]
impl LlmAdapter for AnthropicMessagesAdapter {
    fn vendor(&self) -> ModelVendor {
        ModelVendor::Anthropic
    }

    async fn sample(&self, request: LlmRequest) -> Result<LlmResponse, AgentLoopError> {
        let auth = self.auth_provider.resolve_auth(&request.runtime).await?;
        let body = Self::request_body_with_identity(&request, auth.credential.is_bearer_token());
        let mut builder = self
            .http
            .post(messages_endpoint(&auth.base_url))
            .header("anthropic-version", auth.version)
            .json(&body);
        match auth.credential {
            AnthropicCredential::ApiKey(api_key) => {
                builder = builder.header("x-api-key", api_key);
            }
            AnthropicCredential::BearerToken(token) => {
                builder = builder
                    .bearer_auth(token)
                    .header("user-agent", format!("claude-cli/{CLAUDE_CODE_VERSION}"))
                    .header("x-app", "cli");
            }
        }
        if let Some(beta) = beta_header(auth.beta.as_deref(), body_has_claude_code_identity(&body))
        {
            builder = builder.header("anthropic-beta", beta);
        }
        let response = builder.send().await.map_err(|error| {
            AgentLoopError::failed(format!("Anthropic Messages request failed: {error}"))
        })?;
        let status = response.status();
        let value = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AgentLoopError::failed(format!(
                "Anthropic Messages request failed with {status}: {value}"
            )));
        }
        let value = serde_json::from_str::<Value>(&value).map_err(|error| {
            AgentLoopError::failed(format!(
                "Anthropic Messages response was invalid JSON: {error}; body={value}"
            ))
        })?;
        Ok(Self::parse_response(value))
    }
}

fn body_has_claude_code_identity(body: &Value) -> bool {
    body.get("system")
        .and_then(Value::as_str)
        .is_some_and(|system| system.starts_with("You are Claude Code,"))
}

fn beta_header(configured: Option<&str>, claude_code_oauth: bool) -> Option<String> {
    let mut values = Vec::<String>::new();
    if claude_code_oauth {
        values.push("claude-code-20250219".to_owned());
        values.push("oauth-2025-04-20".to_owned());
    }
    if let Some(configured) = configured {
        values.extend(
            configured
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        );
    }
    (!values.is_empty()).then(|| values.join(","))
}

fn normalize_tool_call_id(id: &str) -> String {
    let normalized: String = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if normalized.is_empty() {
        "toolu_garyx_missing_id".to_owned()
    } else {
        normalized
    }
}

fn supports_adaptive_thinking(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    id.contains("opus-4-6")
        || id.contains("opus-4.6")
        || id.contains("opus-4-7")
        || id.contains("opus-4.7")
        || id.contains("sonnet-4-6")
        || id.contains("sonnet-4.6")
}

fn normalized_effort(value: &str) -> Option<&str> {
    match value.trim() {
        "" => None,
        "off" | "none" | "disabled" => Some("off"),
        "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        other => Some(other),
    }
}

fn budget_for_effort(effort: &str) -> i64 {
    match effort {
        "low" => 1024,
        "medium" => 2048,
        "high" => 4096,
        "xhigh" | "max" => 6144,
        _ => 2048,
    }
}

fn apply_reasoning_options(body: &mut Value, model: &str, effort: &str) {
    let Some(effort) = normalized_effort(effort) else {
        return;
    };
    if effort == "off" {
        body["thinking"] = json!({ "type": "disabled" });
        return;
    }
    if supports_adaptive_thinking(model) {
        body["thinking"] = json!({
            "type": "adaptive",
            "display": "omitted",
        });
        body["output_config"] = json!({ "effort": effort });
    } else {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": budget_for_effort(effort),
            "display": "omitted",
        });
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request(messages: Vec<ConversationMessage>) -> LlmRequest {
        LlmRequest {
            model: "claude-sonnet-4-6".to_owned(),
            instructions: "Act carefully.".to_owned(),
            messages,
            tools: vec![ToolDefinition::function(
                "read_file",
                "Read a file.",
                json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            )],
            options: crate::LlmRequestOptions {
                reasoning_effort: Some("high".to_owned()),
                service_tier: Some("standard_only".to_owned()),
            },
            runtime: LlmRuntimeContext::default(),
        }
    }

    #[test]
    fn request_body_maps_messages_tools_and_reasoning() {
        let body = AnthropicMessagesAdapter::request_body(&request(vec![
            ConversationMessage::system_text("Use concise answers."),
            ConversationMessage::user_text("inspect"),
            ConversationMessage::tool_use(
                json!({
                    "name": "read_file",
                    "arguments": { "path": "README.md" }
                }),
                Some("call:read".to_owned()),
                Some("read_file".to_owned()),
            ),
            ConversationMessage::tool_result(
                json!({ "content": "hello" }),
                Some("call:read".to_owned()),
                Some("read_file".to_owned()),
                Some(false),
            ),
        ]));

        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(
            body["system"].as_str().unwrap(),
            "Act carefully.\n\nUse concise answers."
        );
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tool_choice"]["type"], "auto");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "inspect");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][1]["content"][0]["id"], "call_read");
        assert_eq!(
            body["messages"][1]["content"][0]["input"]["path"],
            "README.md"
        );
        assert_eq!(body["messages"][2]["role"], "user");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "omitted");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["service_tier"], "standard_only");
    }

    #[test]
    fn oauth_request_body_adds_claude_code_identity() {
        let body = AnthropicMessagesAdapter::request_body_with_identity(
            &request(vec![ConversationMessage::user_text("hello")]),
            true,
        );

        assert!(
            body["system"]
                .as_str()
                .unwrap()
                .starts_with("You are Claude Code, Anthropic's official CLI for Claude.")
        );
    }

    #[test]
    fn older_model_reasoning_uses_budget() {
        let mut req = request(Vec::new());
        req.model = "claude-3-5-sonnet-latest".to_owned();
        req.options.reasoning_effort = Some("medium".to_owned());

        let body = AnthropicMessagesAdapter::request_body(&req);

        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 2048);
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn response_parser_returns_text_tool_calls_and_usage() {
        let response = AnthropicMessagesAdapter::parse_response(json!({
            "model": "claude-test-actual",
            "content": [
                { "type": "text", "text": "checking" },
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "read_file",
                    "input": { "path": "AGENTS.md" }
                }
            ],
            "usage": {
                "input_tokens": 4,
                "cache_read_input_tokens": 2,
                "cache_creation_input_tokens": 1,
                "output_tokens": 3
            }
        }));

        assert_eq!(response.actual_model.as_deref(), Some("claude-test-actual"));
        assert_eq!(response.input_tokens, 7);
        assert_eq!(response.output_tokens, 3);
        assert!(matches!(&response.outputs[0], LlmOutput::Text(text) if text == "checking"));
        assert!(matches!(
            &response.outputs[1],
            LlmOutput::ToolCall(call)
                if call.id == "toolu_1"
                    && call.name == "read_file"
                    && call.arguments["path"] == "AGENTS.md"
        ));
    }

    #[test]
    fn anthropic_adapter_reports_vendor() {
        let auth_provider = Arc::new(StaticAnthropicAuthProvider::new(AnthropicAuth::new(
            "test-key",
        )));
        let adapter = AnthropicMessagesAdapter::new(auth_provider);

        assert_eq!(adapter.vendor(), ModelVendor::Anthropic);
    }

    #[test]
    fn oauth_beta_header_includes_required_features() {
        assert_eq!(
            beta_header(Some("test-beta"), true).as_deref(),
            Some("claude-code-20250219,oauth-2025-04-20,test-beta")
        );
    }
}
