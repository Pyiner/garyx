//! Google Gemini adapter for the model-neutral agent loop.
//!
//! Authentication is injected through [`GoogleAuthProvider`]. The adapter uses
//! the public Gemini `generateContent` wire protocol and leaves host-specific
//! config and credential lookup outside this crate.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AgentLoopError, ConversationMessage, ConversationRole, LlmAdapter, LlmOutput, LlmRequest,
    LlmResponse, LlmRuntimeContext, LlmToolCall, ModelVendor, ToolDefinition,
};

pub const GOOGLE_GENERATIVE_AI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const GOOGLE_CODE_ASSIST_BASE_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleCredential {
    ApiKey(String),
    BearerToken(String),
    CodeAssistOAuth(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleAuth {
    pub credential: GoogleCredential,
    pub base_url: String,
}

impl GoogleAuth {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            credential: GoogleCredential::ApiKey(api_key.into()),
            base_url: GOOGLE_GENERATIVE_AI_BASE_URL.to_owned(),
        }
    }

    pub fn bearer_token(token: impl Into<String>) -> Self {
        Self {
            credential: GoogleCredential::BearerToken(token.into()),
            base_url: GOOGLE_GENERATIVE_AI_BASE_URL.to_owned(),
        }
    }

    pub fn code_assist_oauth(token: impl Into<String>) -> Self {
        Self {
            credential: GoogleCredential::CodeAssistOAuth(token.into()),
            base_url: GOOGLE_CODE_ASSIST_BASE_URL.to_owned(),
        }
    }
}

#[async_trait]
pub trait GoogleAuthProvider: Send + Sync {
    async fn resolve_auth(&self, runtime: &LlmRuntimeContext)
    -> Result<GoogleAuth, AgentLoopError>;
}

pub struct StaticGoogleAuthProvider {
    auth: GoogleAuth,
}

impl StaticGoogleAuthProvider {
    pub fn new(auth: GoogleAuth) -> Self {
        Self { auth }
    }
}

#[async_trait]
impl GoogleAuthProvider for StaticGoogleAuthProvider {
    async fn resolve_auth(
        &self,
        _runtime: &LlmRuntimeContext,
    ) -> Result<GoogleAuth, AgentLoopError> {
        Ok(self.auth.clone())
    }
}

pub fn generate_content_endpoint(base_url: &str, model: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with(":generateContent") {
        return trimmed.to_owned();
    }
    let model_path = if model.trim().starts_with("models/") {
        model.trim().to_owned()
    } else {
        format!("models/{}", model.trim())
    };
    format!("{trimmed}/{model_path}:generateContent")
}

pub fn code_assist_endpoint(base_url: &str, method: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with(&format!(":{method}")) {
        return trimmed.to_owned();
    }
    format!("{trimmed}:{method}")
}

pub struct GoogleGenerativeAiAdapter {
    auth_provider: Arc<dyn GoogleAuthProvider>,
    http: reqwest::Client,
}

impl GoogleGenerativeAiAdapter {
    pub fn new(auth_provider: Arc<dyn GoogleAuthProvider>) -> Self {
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

    fn tool_call_arguments(message: &ConversationMessage) -> Value {
        message
            .content
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| message.content.clone())
    }

    fn tool_result_response(message: &ConversationMessage) -> Value {
        let value = Self::provider_message_text(message)
            .map(Value::String)
            .unwrap_or_else(|| message.content.clone());
        if message.is_error.unwrap_or(false) {
            json!({ "error": value })
        } else {
            json!({ "output": value })
        }
    }

    fn push_content(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
        if parts.is_empty() {
            return;
        }
        if let Some(last) = contents.last_mut()
            && last.get("role").and_then(Value::as_str) == Some(role)
            && let Some(existing) = last.get_mut("parts").and_then(Value::as_array_mut)
        {
            existing.extend(parts);
            return;
        }
        contents.push(json!({
            "role": role,
            "parts": parts,
        }));
    }

    fn conversation_payload(request: &LlmRequest) -> (Option<String>, Vec<Value>) {
        let mut system_parts = Vec::<String>::new();
        if !request.instructions.trim().is_empty() {
            system_parts.push(request.instructions.trim().to_owned());
        }
        let mut contents = Vec::<Value>::new();
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
                    Self::push_content(&mut contents, "user", vec![json!({ "text": text })]);
                }
                ConversationRole::Assistant => {
                    let Some(text) = Self::provider_message_text(message) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    Self::push_content(&mut contents, "model", vec![json!({ "text": text })]);
                }
                ConversationRole::ToolUse => {
                    let name = Self::tool_call_name(message);
                    if name.is_empty() {
                        continue;
                    }
                    let mut function_call = json!({
                        "name": name,
                        "args": Self::tool_call_arguments(message),
                    });
                    if let Some(id) = message.tool_call_id.as_deref()
                        && !id.trim().is_empty()
                    {
                        function_call["id"] = Value::String(normalize_function_call_id(id));
                    }
                    let thought_signature = message
                        .metadata
                        .get("google_thought_signature")
                        .and_then(Value::as_str)
                        .and_then(normalize_non_empty);
                    let mut part = json!({ "functionCall": function_call });
                    if let Some(thought_signature) = thought_signature {
                        part["thoughtSignature"] = Value::String(thought_signature);
                    }
                    Self::push_content(&mut contents, "model", vec![part]);
                }
                ConversationRole::ToolResult => {
                    let name = message
                        .tool_name
                        .clone()
                        .unwrap_or_else(|| "tool_result".to_owned());
                    let mut function_response = json!({
                        "name": name,
                        "response": Self::tool_result_response(message),
                    });
                    if let Some(id) = message.tool_call_id.as_deref()
                        && !id.trim().is_empty()
                    {
                        function_response["id"] = Value::String(normalize_function_call_id(id));
                    }
                    Self::push_content(
                        &mut contents,
                        "user",
                        vec![json!({ "functionResponse": function_response })],
                    );
                }
            }
        }

        let system = (!system_parts.is_empty()).then(|| system_parts.join("\n\n"));
        (system, contents)
    }

    fn tool_schema(tool: &ToolDefinition) -> Value {
        json!({
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        })
    }

    pub fn request_body(request: &LlmRequest) -> Value {
        let (system, contents) = Self::conversation_payload(request);
        let mut body = json!({
            "contents": contents,
        });
        if let Some(system) = system {
            body["systemInstruction"] = json!({
                "parts": [{ "text": system }],
            });
        }
        if !request.tools.is_empty() {
            body["tools"] = json!([{
                "functionDeclarations": request.tools.iter().map(Self::tool_schema).collect::<Vec<_>>(),
            }]);
            body["toolConfig"] = json!({
                "functionCallingConfig": { "mode": "AUTO" },
            });
        }

        let mut generation_config = serde_json::Map::new();
        if let Some(effort) = request.options.reasoning_effort.as_deref()
            && let Some(thinking) = thinking_config(&request.model, effort)
        {
            generation_config.insert("thinkingConfig".to_owned(), thinking);
        }
        if !generation_config.is_empty() {
            body["generationConfig"] = Value::Object(generation_config);
        }
        body
    }

    pub fn parse_response(value: Value, requested_model: &str) -> LlmResponse {
        let mut outputs = Vec::new();
        if let Some(parts) = value
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
        {
            for part in parts {
                if part
                    .get("thought")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    continue;
                }
                if let Some(text) = part.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    outputs.push(LlmOutput::Text(text.to_owned()));
                }
                if let Some(function_call) = part.get("functionCall") {
                    let name = function_call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    if name.is_empty() {
                        continue;
                    }
                    let id = function_call
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                    let mut metadata = HashMap::new();
                    if let Some(thought_signature) =
                        part.get("thoughtSignature").and_then(Value::as_str)
                    {
                        metadata.insert(
                            "google_thought_signature".to_owned(),
                            Value::String(thought_signature.to_owned()),
                        );
                    }
                    outputs.push(LlmOutput::ToolCall(LlmToolCall {
                        id,
                        name,
                        arguments: function_call.get("args").cloned().unwrap_or(Value::Null),
                        metadata,
                    }));
                }
            }
        }

        let usage = value.get("usageMetadata").unwrap_or(&Value::Null);
        LlmResponse {
            outputs,
            input_tokens: usage
                .get("promptTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            output_tokens: usage
                .get("candidatesTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or_default()
                + usage
                    .get("thoughtsTokenCount")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            actual_model: value
                .get("modelVersion")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| Some(requested_model.to_owned())),
        }
    }

    pub fn code_assist_generate_content_body(
        request: &LlmRequest,
        project_id: &str,
        session_id: &str,
        user_prompt_id: &str,
    ) -> Value {
        let mut inner = Self::request_body(request);
        inner["session_id"] = Value::String(session_id.to_owned());
        json!({
            "model": request.model.clone(),
            "project": project_id,
            "user_prompt_id": user_prompt_id,
            "request": inner,
        })
    }

    fn code_assist_load_body(project_id: Option<&str>) -> Value {
        let mut body = json!({
            "metadata": code_assist_metadata(project_id),
            "mode": "HEALTH_CHECK",
        });
        if let Some(project_id) = project_id
            && !project_id.trim().is_empty()
        {
            body["cloudaicompanionProject"] = Value::String(project_id.trim().to_owned());
        }
        body
    }

    fn code_assist_project_from_load(
        value: &Value,
        preferred_project_id: Option<&str>,
    ) -> Option<String> {
        value
            .get("cloudaicompanionProject")
            .and_then(Value::as_str)
            .and_then(normalize_non_empty)
            .or_else(|| preferred_project_id.and_then(normalize_non_empty))
    }

    async fn resolve_code_assist_project(
        &self,
        token: &str,
        base_url: &str,
        runtime: &LlmRuntimeContext,
    ) -> Result<String, AgentLoopError> {
        let preferred_project_id = runtime_value(
            runtime,
            &[
                "GEMINI_CODE_ASSIST_PROJECT",
                "GOOGLE_CLOUD_PROJECT",
                "GOOGLE_CLOUD_PROJECT_ID",
            ],
        );
        let body = Self::code_assist_load_body(preferred_project_id.as_deref());
        let (status, text) = self
            .post_code_assist_json_with_retry(token, base_url, "loadCodeAssist", &body)
            .await?;
        if !status.is_success() {
            return Err(AgentLoopError::failed(format!(
                "Google Code Assist loadCodeAssist request failed with {status}: {text}"
            )));
        }
        let value = serde_json::from_str::<Value>(&text).map_err(|error| {
            AgentLoopError::failed(format!(
                "Google Code Assist loadCodeAssist response was invalid JSON: {error}; body={text}"
            ))
        })?;
        Self::code_assist_project_from_load(&value, preferred_project_id.as_deref()).ok_or_else(
            || {
                AgentLoopError::failed(
                    "Google Code Assist loadCodeAssist response did not include a project id",
                )
            },
        )
    }

    async fn sample_code_assist(
        &self,
        request: &LlmRequest,
        token: &str,
        base_url: &str,
    ) -> Result<LlmResponse, AgentLoopError> {
        let project_id = self
            .resolve_code_assist_project(token, base_url, &request.runtime)
            .await?;
        let session_id = code_assist_session_id(&request.runtime);
        let user_prompt_id = code_assist_user_prompt_id(&request.runtime);
        let body = Self::code_assist_generate_content_body(
            request,
            &project_id,
            &session_id,
            &user_prompt_id,
        );
        let (status, text) = self
            .post_code_assist_json_with_retry(token, base_url, "generateContent", &body)
            .await?;
        if !status.is_success() {
            return Err(AgentLoopError::failed(format!(
                "Google Code Assist generateContent request failed with {status}: {text}"
            )));
        }
        let value = serde_json::from_str::<Value>(&text).map_err(|error| {
            AgentLoopError::failed(format!(
                "Google Code Assist generateContent response was invalid JSON: {error}; body={text}"
            ))
        })?;
        Ok(Self::parse_response(
            value.get("response").cloned().unwrap_or(Value::Null),
            &request.model,
        ))
    }

    async fn post_code_assist_json_with_retry(
        &self,
        token: &str,
        base_url: &str,
        method: &str,
        body: &Value,
    ) -> Result<(reqwest::StatusCode, String), AgentLoopError> {
        const MAX_ATTEMPTS: usize = 3;
        let endpoint = code_assist_endpoint(base_url, method);
        let mut last_status = None;
        let mut last_text = String::new();

        for attempt in 1..=MAX_ATTEMPTS {
            let response = self
                .http
                .post(&endpoint)
                .bearer_auth(token)
                .json(body)
                .send()
                .await
                .map_err(|error| {
                    AgentLoopError::failed(format!(
                        "Google Code Assist {method} request failed: {error}"
                    ))
                })?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status.is_success() {
                return Ok((status, text));
            }
            if status != reqwest::StatusCode::TOO_MANY_REQUESTS || attempt == MAX_ATTEMPTS {
                return Ok((status, text));
            }

            let delay = code_assist_retry_delay(&text)
                .unwrap_or_else(|| Duration::from_secs(2_u64.pow((attempt - 1) as u32)));
            last_status = Some(status);
            last_text = text;
            tokio::time::sleep(delay.min(Duration::from_secs(75))).await;
        }

        Ok((
            last_status.unwrap_or(reqwest::StatusCode::TOO_MANY_REQUESTS),
            last_text,
        ))
    }
}

#[async_trait]
impl LlmAdapter for GoogleGenerativeAiAdapter {
    fn vendor(&self) -> ModelVendor {
        ModelVendor::Google
    }

    async fn sample(&self, request: LlmRequest) -> Result<LlmResponse, AgentLoopError> {
        let auth = self.auth_provider.resolve_auth(&request.runtime).await?;
        if let GoogleCredential::CodeAssistOAuth(token) = &auth.credential {
            return self
                .sample_code_assist(&request, token, &auth.base_url)
                .await;
        }

        let endpoint = generate_content_endpoint(&auth.base_url, &request.model);
        let body = Self::request_body(&request);
        let mut builder = self.http.post(endpoint).json(&body);
        match &auth.credential {
            GoogleCredential::ApiKey(api_key) => {
                builder = builder.query(&[("key", api_key.as_str())]);
            }
            GoogleCredential::BearerToken(token) => {
                builder = builder.bearer_auth(token);
            }
            GoogleCredential::CodeAssistOAuth(_) => {}
        }
        let response = builder.send().await.map_err(|error| {
            AgentLoopError::failed(format!("Google Gemini request failed: {error}"))
        })?;
        let status = response.status();
        let value = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AgentLoopError::failed(format!(
                "Google Gemini request failed with {status}: {value}"
            )));
        }
        let value = serde_json::from_str::<Value>(&value).map_err(|error| {
            AgentLoopError::failed(format!(
                "Google Gemini response was invalid JSON: {error}; body={value}"
            ))
        })?;
        Ok(Self::parse_response(value, &request.model))
    }
}

fn normalize_non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn runtime_value(runtime: &LlmRuntimeContext, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = runtime
            .env
            .get(*key)
            .map(String::as_str)
            .and_then(normalize_non_empty)
        {
            return Some(value);
        }
        if let Ok(value) = std::env::var(key)
            && let Some(value) = normalize_non_empty(&value)
        {
            return Some(value);
        }
    }
    None
}

fn code_assist_metadata(project_id: Option<&str>) -> Value {
    let mut metadata = json!({
        "ideType": "IDE_UNSPECIFIED",
        "platform": "PLATFORM_UNSPECIFIED",
        "pluginType": "GEMINI",
    });
    if let Some(project_id) = project_id.and_then(normalize_non_empty) {
        metadata["duetProject"] = Value::String(project_id);
    }
    metadata
}

fn stable_id(value: Option<&str>, prefix: &str) -> String {
    let normalized = value
        .and_then(normalize_non_empty)
        .map(|value| {
            value
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                        ch
                    } else {
                        '_'
                    }
                })
                .take(128)
                .collect::<String>()
        })
        .filter(|value| !value.is_empty());
    normalized.unwrap_or_else(|| format!("{prefix}_{}", Uuid::new_v4()))
}

fn code_assist_session_id(runtime: &LlmRuntimeContext) -> String {
    stable_id(
        runtime
            .metadata
            .get("sdk_session_id")
            .and_then(Value::as_str)
            .or_else(|| runtime.metadata.get("thread_id").and_then(Value::as_str)),
        "garyx_native_session",
    )
}

fn code_assist_user_prompt_id(runtime: &LlmRuntimeContext) -> String {
    stable_id(
        runtime
            .metadata
            .get("bridge_run_id")
            .and_then(Value::as_str)
            .or_else(|| {
                runtime
                    .metadata
                    .get("client_run_id")
                    .and_then(Value::as_str)
            })
            .or_else(|| runtime.metadata.get("run_id").and_then(Value::as_str)),
        "garyx_native_prompt",
    )
}

fn code_assist_retry_delay(body: &str) -> Option<Duration> {
    let value = serde_json::from_str::<Value>(body).ok()?;
    let message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)?;
    let seconds = message
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|pair| {
            (pair[1].trim_matches(|ch: char| !ch.is_ascii_alphabetic()) == "s")
                .then(|| {
                    pair[0]
                        .trim_matches(|ch: char| !ch.is_ascii_digit())
                        .parse::<u64>()
                        .ok()
                })
                .flatten()
        })
        .or_else(|| {
            message
                .split(|ch: char| !ch.is_ascii_digit())
                .find_map(|part| part.parse::<u64>().ok())
        })?;
    Some(Duration::from_secs(seconds.saturating_add(1)))
}

fn normalize_function_call_id(id: &str) -> String {
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
        "call_missing_id".to_owned()
    } else {
        normalized
    }
}

fn normalized_effort(value: &str) -> Option<&str> {
    match value.trim() {
        "" => None,
        "off" | "none" | "disabled" => Some("off"),
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" | "xhigh" | "max" => Some("high"),
        other => Some(other),
    }
}

fn is_gemini3_or_gemma4(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    id.starts_with("gemini-3") || id.starts_with("gemini-3.") || id.contains("gemma-4")
}

fn thinking_level(effort: &str, model: &str) -> &'static str {
    if effort == "minimal" {
        return "MINIMAL";
    }
    if model.to_ascii_lowercase().contains("pro") {
        match effort {
            "low" | "minimal" => "LOW",
            _ => "HIGH",
        }
    } else {
        match effort {
            "low" => "LOW",
            "medium" => "MEDIUM",
            _ => "HIGH",
        }
    }
}

fn thinking_budget(effort: &str, model: &str) -> i64 {
    let id = model.to_ascii_lowercase();
    if effort == "off" {
        return 0;
    }
    if id.contains("2.5-pro") {
        return match effort {
            "minimal" => 128,
            "low" => 2048,
            "medium" => 8192,
            _ => 32768,
        };
    }
    if id.contains("2.5-flash") {
        return match effort {
            "minimal" => 128,
            "low" => 2048,
            "medium" => 8192,
            _ => 24576,
        };
    }
    -1
}

fn thinking_config(model: &str, effort: &str) -> Option<Value> {
    let effort = normalized_effort(effort)?;
    if is_gemini3_or_gemma4(model) {
        return Some(json!({
            "thinkingLevel": if effort == "off" {
                "MINIMAL"
            } else {
                thinking_level(effort, model)
            },
        }));
    }
    Some(json!({
        "thinkingBudget": thinking_budget(effort, model),
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request(messages: Vec<ConversationMessage>) -> LlmRequest {
        LlmRequest {
            model: "gemini-3-flash-preview".to_owned(),
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
                reasoning_effort: Some("medium".to_owned()),
                service_tier: None,
            },
            runtime: LlmRuntimeContext::default(),
        }
    }

    #[test]
    fn request_body_maps_messages_tools_and_thinking_level() {
        let body = GoogleGenerativeAiAdapter::request_body(&request(vec![
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

        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "Act carefully.\n\nUse concise answers."
        );
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "inspect");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(
            body["contents"][1]["parts"][0]["functionCall"]["name"],
            "read_file"
        );
        assert_eq!(
            body["contents"][1]["parts"][0]["functionCall"]["args"]["path"],
            "README.md"
        );
        assert_eq!(body["contents"][2]["role"], "user");
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"]["name"],
            "read_file"
        );
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["parameters"]["required"][0],
            "path"
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "MEDIUM"
        );
    }

    #[test]
    fn gemini_25_reasoning_uses_budget() {
        let mut req = request(Vec::new());
        req.model = "gemini-2.5-pro".to_owned();
        req.options.reasoning_effort = Some("high".to_owned());

        let body = GoogleGenerativeAiAdapter::request_body(&req);

        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            32768
        );
    }

    #[test]
    fn response_parser_returns_text_tool_calls_and_usage() {
        let response = GoogleGenerativeAiAdapter::parse_response(
            json!({
                "modelVersion": "gemini-test-actual",
                "candidates": [{
                    "content": {
                        "parts": [
                            { "thought": true, "text": "internal" },
                            { "text": "checking" },
                            {
                                "thoughtSignature": "signature-test",
                                "functionCall": {
                                    "id": "call_1",
                                    "name": "read_file",
                                    "args": { "path": "AGENTS.md" }
                                }
                            }
                        ]
                    }
                }],
                "usageMetadata": {
                    "promptTokenCount": 4,
                    "candidatesTokenCount": 3,
                    "thoughtsTokenCount": 2
                }
            }),
            "gemini-test",
        );

        assert_eq!(response.actual_model.as_deref(), Some("gemini-test-actual"));
        assert_eq!(response.input_tokens, 4);
        assert_eq!(response.output_tokens, 5);
        assert!(matches!(&response.outputs[0], LlmOutput::Text(text) if text == "checking"));
        assert!(matches!(
            &response.outputs[1],
            LlmOutput::ToolCall(call)
                if call.id == "call_1"
                    && call.name == "read_file"
                    && call.arguments["path"] == "AGENTS.md"
                    && call.metadata["google_thought_signature"] == "signature-test"
        ));
    }

    #[test]
    fn request_body_replays_google_thought_signature_on_function_call_part() {
        let mut tool_use = ConversationMessage::tool_use(
            json!({
                "name": "read_file",
                "arguments": { "path": "README.md" }
            }),
            Some("call-read".to_owned()),
            Some("read_file".to_owned()),
        );
        tool_use.metadata.insert(
            "google_thought_signature".to_owned(),
            Value::String("signature-test".to_owned()),
        );

        let body = GoogleGenerativeAiAdapter::request_body(&request(vec![tool_use]));

        assert_eq!(
            body["contents"][0]["parts"][0]["functionCall"]["name"],
            "read_file"
        );
        assert_eq!(
            body["contents"][0]["parts"][0]["thoughtSignature"],
            "signature-test"
        );
    }

    #[test]
    fn code_assist_body_wraps_public_gemini_request() {
        let mut req = request(vec![ConversationMessage::user_text("inspect")]);
        req.runtime.metadata.insert(
            "sdk_session_id".to_owned(),
            Value::String("thread::test-session".to_owned()),
        );
        req.runtime.metadata.insert(
            "bridge_run_id".to_owned(),
            Value::String("run:test".to_owned()),
        );

        let body = GoogleGenerativeAiAdapter::code_assist_generate_content_body(
            &req,
            "test-project",
            &code_assist_session_id(&req.runtime),
            &code_assist_user_prompt_id(&req.runtime),
        );

        assert_eq!(body["model"], "gemini-3-flash-preview");
        assert_eq!(body["project"], "test-project");
        assert_eq!(body["user_prompt_id"], "run_test");
        assert_eq!(body["request"]["session_id"], "thread__test-session");
        assert_eq!(
            body["request"]["contents"][0]["parts"][0]["text"],
            "inspect"
        );
        assert_eq!(
            body["request"]["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "MEDIUM"
        );
    }

    #[test]
    fn code_assist_project_resolution_uses_load_response_or_preferred_project() {
        let load = json!({
            "cloudaicompanionProject": "resolved-project",
            "currentTier": { "id": "standard-tier" }
        });

        assert_eq!(
            GoogleGenerativeAiAdapter::code_assist_project_from_load(&load, Some("preferred"))
                .as_deref(),
            Some("resolved-project")
        );
        assert_eq!(
            GoogleGenerativeAiAdapter::code_assist_project_from_load(&json!({}), Some("preferred"))
                .as_deref(),
            Some("preferred")
        );
        assert!(
            GoogleGenerativeAiAdapter::code_assist_project_from_load(&json!({}), None).is_none()
        );
    }

    #[test]
    fn code_assist_retry_delay_reads_quota_reset_message() {
        let delay = code_assist_retry_delay(
            r#"{"error":{"message":"You have exhausted your capacity on this model. Your quota will reset after 55s."}}"#,
        )
        .unwrap();

        assert_eq!(delay, Duration::from_secs(56));
    }

    #[test]
    fn google_adapter_reports_vendor() {
        let auth_provider = Arc::new(StaticGoogleAuthProvider::new(GoogleAuth::new("test-key")));
        let adapter = GoogleGenerativeAiAdapter::new(auth_provider);

        assert_eq!(adapter.vendor(), ModelVendor::Google);
    }

    #[test]
    fn generate_content_endpoint_uses_models_prefix_once() {
        assert_eq!(
            generate_content_endpoint(GOOGLE_GENERATIVE_AI_BASE_URL, "gemini-test"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-test:generateContent"
        );
        assert_eq!(
            generate_content_endpoint(GOOGLE_GENERATIVE_AI_BASE_URL, "models/gemini-test"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-test:generateContent"
        );
    }

    #[test]
    fn code_assist_endpoint_appends_rpc_method() {
        assert_eq!(
            code_assist_endpoint(GOOGLE_CODE_ASSIST_BASE_URL, "generateContent"),
            "https://cloudcode-pa.googleapis.com/v1internal:generateContent"
        );
        assert_eq!(
            code_assist_endpoint(
                "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist",
                "loadCodeAssist",
            ),
            "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist"
        );
    }
}
