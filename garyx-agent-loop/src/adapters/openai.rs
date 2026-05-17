//! OpenAI Responses adapter for the model-neutral agent loop.
//!
//! Authentication is injected through [`OpenAiAuthProvider`] so host
//! applications can use API keys, OAuth tokens, enterprise gateways, or any
//! other credential source without coupling the adapter to the host's config
//! model.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    AgentLoopError, ConversationMessage, LlmAdapter, LlmOutput, LlmRequest, LlmResponse,
    LlmRuntimeContext, LlmToolCall, ModelVendor, ToolDefinition,
};

pub const OPENAI_RESPONSES_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiAuth {
    pub bearer_token: String,
    pub base_url: String,
    pub account_id: Option<String>,
}

impl OpenAiAuth {
    pub fn new(bearer_token: impl Into<String>) -> Self {
        Self {
            bearer_token: bearer_token.into(),
            base_url: OPENAI_RESPONSES_BASE_URL.to_owned(),
            account_id: None,
        }
    }
}

#[async_trait]
pub trait OpenAiAuthProvider: Send + Sync {
    async fn resolve_auth(&self, runtime: &LlmRuntimeContext)
    -> Result<OpenAiAuth, AgentLoopError>;
}

pub struct StaticOpenAiAuthProvider {
    auth: OpenAiAuth,
}

impl StaticOpenAiAuthProvider {
    pub fn new(auth: OpenAiAuth) -> Self {
        Self { auth }
    }
}

#[async_trait]
impl OpenAiAuthProvider for StaticOpenAiAuthProvider {
    async fn resolve_auth(
        &self,
        _runtime: &LlmRuntimeContext,
    ) -> Result<OpenAiAuth, AgentLoopError> {
        Ok(self.auth.clone())
    }
}

pub fn responses_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/responses")
    }
}

#[derive(Default)]
pub struct ResponseStreamAccumulator {
    completed_response: Option<Value>,
    output_items: Vec<Value>,
    text: String,
    error: Option<String>,
}

/// OpenAI-compatible Responses adapter used by Garyx's GPT provider.
///
/// It supports both direct OpenAI API-key auth and ChatGPT Codex auth resolved
/// through the same Codex auth files used by the Codex CLI.
pub struct OpenAiResponsesAdapter {
    auth_provider: Arc<dyn OpenAiAuthProvider>,
    http: reqwest::Client,
}

impl OpenAiResponsesAdapter {
    pub fn new(auth_provider: Arc<dyn OpenAiAuthProvider>) -> Self {
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

    pub fn message_input(message: &ConversationMessage) -> Option<Value> {
        match message.role_str() {
            "user" => Some(json!({
                "role": "user",
                "content": Self::provider_message_text(message).unwrap_or_default(),
            })),
            "assistant" => Some(json!({
                "role": "assistant",
                "content": Self::provider_message_text(message).unwrap_or_default(),
            })),
            "system" => Some(json!({
                "role": "system",
                "content": Self::provider_message_text(message).unwrap_or_default(),
            })),
            "tool_use" => {
                let call_id = message
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                Some(json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": Self::tool_call_name(message),
                    "arguments": Self::tool_call_arguments(message),
                }))
            }
            "tool_result" => {
                let call_id = message
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                Some(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": Self::provider_message_text(message).unwrap_or_else(|| message.content.to_string()),
                }))
            }
            _ => None,
        }
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

    fn tool_call_arguments(message: &ConversationMessage) -> String {
        let arguments = message.content.get("arguments").unwrap_or(&message.content);
        match arguments.as_str() {
            Some(text) => text.to_owned(),
            None => arguments.to_string(),
        }
    }

    fn tool_schema(tool: &ToolDefinition) -> Value {
        json!({
            "type": "function",
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        })
    }

    pub fn response_body(request: &LlmRequest, input: Vec<Value>) -> Value {
        let mut body = json!({
            "model": request.model,
            "instructions": request.instructions,
            "input": input,
            "tools": request.tools.iter().map(Self::tool_schema).collect::<Vec<_>>(),
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stream": true,
            "store": false,
        });
        if let Some(effort) = request.options.reasoning_effort.as_deref()
            && !effort.trim().is_empty()
        {
            body["reasoning"] = json!({ "effort": effort.trim() });
        }
        if let Some(service_tier) = request.options.service_tier.as_deref()
            && !service_tier.trim().is_empty()
        {
            body["service_tier"] = json!(service_tier.trim());
        }
        body
    }

    pub fn parse_response(value: Value) -> LlmResponse {
        let mut outputs = Vec::new();
        if let Some(items) = value.get("output").and_then(Value::as_array) {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        if let Some(content) = item.get("content").and_then(Value::as_array) {
                            for block in content {
                                if let Some(text) = block
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .or_else(|| block.get("output_text").and_then(Value::as_str))
                                    && !text.is_empty()
                                {
                                    outputs.push(LlmOutput::Text(text.to_owned()));
                                }
                            }
                        }
                    }
                    Some("function_call") => {
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        if name.is_empty() {
                            continue;
                        }
                        let id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .or_else(|| item.get("id").and_then(Value::as_str))
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("call_{}", Uuid::new_v4()));
                        let arguments = item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .and_then(|text| serde_json::from_str::<Value>(text).ok())
                            .unwrap_or_else(|| {
                                item.get("arguments").cloned().unwrap_or(Value::Null)
                            });
                        outputs.push(LlmOutput::ToolCall(LlmToolCall {
                            id,
                            name,
                            arguments,
                        }));
                    }
                    _ => {}
                }
            }
        }

        let usage = value.get("usage").unwrap_or(&Value::Null);
        LlmResponse {
            outputs,
            input_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
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

    pub fn apply_stream_event(acc: &mut ResponseStreamAccumulator, event: Value) {
        match event.get("type").and_then(Value::as_str) {
            Some("response.completed") => {
                acc.completed_response = event.get("response").cloned();
            }
            Some("response.failed") | Some("response.incomplete") => {
                acc.error = event
                    .get("response")
                    .and_then(|response| response.get("error"))
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .or_else(|| {
                        event
                            .get("response")
                            .and_then(|response| response.get("incomplete_details"))
                            .and_then(|details| details.get("reason"))
                            .and_then(Value::as_str)
                    })
                    .map(ToOwned::to_owned)
                    .or_else(|| Some(event.to_string()));
            }
            Some("response.output_item.done") => {
                if let Some(item) = event.get("item") {
                    acc.output_items.push(item.clone());
                }
            }
            Some("response.output_text.done") => {
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    acc.text = text.to_owned();
                }
            }
            Some("response.output_text.delta") | Some("response.refusal.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    acc.text.push_str(delta);
                }
            }
            Some("response.refusal.done") => {
                if let Some(refusal) = event.get("refusal").and_then(Value::as_str) {
                    acc.text = refusal.to_owned();
                }
            }
            Some("error") => {
                acc.error = event
                    .get("message")
                    .and_then(Value::as_str)
                    .or_else(|| event.get("error").and_then(Value::as_str))
                    .or_else(|| {
                        event
                            .get("error")
                            .and_then(|error| error.get("message"))
                            .and_then(Value::as_str)
                    })
                    .map(ToOwned::to_owned)
                    .or_else(|| Some(event.to_string()));
            }
            _ => {}
        }
    }

    pub fn finalize_stream(acc: ResponseStreamAccumulator) -> Result<LlmResponse, AgentLoopError> {
        if let Some(error) = acc.error {
            return Err(AgentLoopError::failed(format!(
                "OpenAI Responses stream failed: {error}"
            )));
        }
        if let Some(mut response) = acc.completed_response {
            let completed_output_empty = response
                .get("output")
                .and_then(Value::as_array)
                .map(Vec::is_empty)
                .unwrap_or(true);
            if completed_output_empty && !acc.output_items.is_empty() {
                response["output"] = Value::Array(acc.output_items);
            }
            let mut parsed = Self::parse_response(response);
            if parsed.outputs.is_empty() && !acc.text.is_empty() {
                parsed.outputs.push(LlmOutput::Text(acc.text));
            }
            return Ok(parsed);
        }
        if !acc.output_items.is_empty() {
            let mut parsed = Self::parse_response(json!({ "output": acc.output_items }));
            if parsed.outputs.is_empty() && !acc.text.is_empty() {
                parsed.outputs.push(LlmOutput::Text(acc.text));
            }
            return Ok(parsed);
        }
        if !acc.text.is_empty() {
            return Ok(LlmResponse {
                outputs: vec![LlmOutput::Text(acc.text)],
                ..Default::default()
            });
        }
        Err(AgentLoopError::failed(
            "OpenAI Responses stream completed without output",
        ))
    }

    fn process_sse_data(
        acc: &mut ResponseStreamAccumulator,
        data: &str,
    ) -> Result<bool, AgentLoopError> {
        let trimmed = data.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        if trimmed == "[DONE]" {
            return Ok(true);
        }
        let event = serde_json::from_str::<Value>(trimmed).map_err(|error| {
            AgentLoopError::failed(format!(
                "OpenAI Responses stream event was invalid JSON: {error}"
            ))
        })?;
        Self::apply_stream_event(acc, event);
        Ok(false)
    }

    async fn parse_streaming_response(
        mut response: reqwest::Response,
    ) -> Result<LlmResponse, AgentLoopError> {
        let mut acc = ResponseStreamAccumulator::default();
        let mut pending = Vec::<u8>::new();
        let mut event_data = String::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            AgentLoopError::failed(format!("OpenAI Responses stream failed: {error}"))
        })? {
            pending.extend_from_slice(&chunk);
            while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
                let mut line = pending.drain(..=newline).collect::<Vec<_>>();
                line.pop();
                if line.ends_with(b"\r") {
                    line.pop();
                }
                let line = std::str::from_utf8(&line).map_err(|error| {
                    AgentLoopError::failed(format!(
                        "OpenAI Responses stream line was invalid UTF-8: {error}"
                    ))
                })?;
                if line.is_empty() {
                    if !event_data.is_empty() {
                        if Self::process_sse_data(&mut acc, &event_data)? {
                            return Self::finalize_stream(acc);
                        }
                        event_data.clear();
                    }
                    continue;
                }
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.strip_prefix(' ').unwrap_or(data);
                    if !event_data.is_empty() {
                        event_data.push('\n');
                    }
                    event_data.push_str(data);
                }
            }
        }
        if !pending.is_empty() {
            if pending.ends_with(b"\r") {
                pending.pop();
            }
            let line = std::str::from_utf8(&pending).map_err(|error| {
                AgentLoopError::failed(format!(
                    "OpenAI Responses stream line was invalid UTF-8: {error}"
                ))
            })?;
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.strip_prefix(' ').unwrap_or(data);
                if !event_data.is_empty() {
                    event_data.push('\n');
                }
                event_data.push_str(data);
            }
        }
        if !event_data.is_empty() {
            Self::process_sse_data(&mut acc, &event_data)?;
        }
        Self::finalize_stream(acc)
    }
}

#[async_trait]
impl LlmAdapter for OpenAiResponsesAdapter {
    fn vendor(&self) -> ModelVendor {
        ModelVendor::OpenAi
    }

    async fn sample(&self, request: LlmRequest) -> Result<LlmResponse, AgentLoopError> {
        let auth = self.auth_provider.resolve_auth(&request.runtime).await?;
        let mut input = Vec::new();
        for message in &request.messages {
            if let Some(item) = Self::message_input(message) {
                input.push(item);
            }
        }

        let body = Self::response_body(&request, input);
        let mut builder = self
            .http
            .post(responses_endpoint(&auth.base_url))
            .bearer_auth(auth.bearer_token)
            .json(&body);
        if let Some(account_id) = auth.account_id.as_deref()
            && !account_id.trim().is_empty()
        {
            builder = builder.header("ChatGPT-Account-ID", account_id);
        }
        let response = builder.send().await.map_err(|error| {
            AgentLoopError::failed(format!("OpenAI Responses request failed: {error}"))
        })?;
        let status = response.status();
        if !status.is_success() {
            let value = response.text().await.unwrap_or_default();
            return Err(AgentLoopError::failed(format!(
                "OpenAI Responses request failed with {status}: {value}"
            )));
        }
        Self::parse_streaming_response(response).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn response_body_enables_streaming_and_reasoning_effort() {
        let request = LlmRequest {
            model: "gpt-test".to_owned(),
            instructions: "Act carefully.".to_owned(),
            messages: Vec::new(),
            tools: vec![ToolDefinition::function(
                "read_file",
                "Read a file.",
                json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            )],
            options: crate::LlmRequestOptions {
                reasoning_effort: Some("high".to_owned()),
                service_tier: Some("priority".to_owned()),
            },
            runtime: LlmRuntimeContext::default(),
        };

        let body = OpenAiResponsesAdapter::response_body(
            &request,
            vec![json!({ "role": "user", "content": "hello" })],
        );

        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["service_tier"], "priority");
        assert_eq!(body["input"][0]["content"], "hello");
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tools"][0]["description"], "Read a file.");
    }

    #[test]
    fn message_input_replays_tool_call_arguments_without_garyx_wrapper() {
        let message = ConversationMessage::tool_use(
            json!({
                "name": "write_file",
                "arguments": {
                    "path": "demo.txt",
                    "content": "hello"
                }
            }),
            Some("call-1".to_owned()),
            Some("write_file".to_owned()),
        );

        let input = OpenAiResponsesAdapter::message_input(&message).unwrap();

        assert_eq!(input["type"], "function_call");
        assert_eq!(input["call_id"], "call-1");
        assert_eq!(input["name"], "write_file");
        let arguments =
            serde_json::from_str::<Value>(input["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments["path"], "demo.txt");
        assert_eq!(arguments["content"], "hello");
        assert!(arguments.get("name").is_none());
    }

    #[test]
    fn completed_response_reuses_stream_output_items_when_final_output_is_empty() {
        let mut acc = ResponseStreamAccumulator::default();

        OpenAiResponsesAdapter::apply_stream_event(
            &mut acc,
            json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "reasoning",
                    "summary": []
                }
            }),
        );
        OpenAiResponsesAdapter::apply_stream_event(
            &mut acc,
            json!({
                "type": "response.output_text.done",
                "text": "done"
            }),
        );
        OpenAiResponsesAdapter::apply_stream_event(
            &mut acc,
            json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "done" }
                    ]
                }
            }),
        );
        OpenAiResponsesAdapter::apply_stream_event(
            &mut acc,
            json!({
                "type": "response.output_item.done",
                "item": {
                    "type": "function_call",
                    "call_id": "call-1",
                    "name": "read_file",
                    "arguments": "{\"path\":\"AGENTS.md\"}"
                }
            }),
        );
        OpenAiResponsesAdapter::apply_stream_event(
            &mut acc,
            json!({
                "type": "response.completed",
                "response": {
                    "model": "gpt-test-actual",
                    "output": [],
                    "usage": {
                        "input_tokens": 4,
                        "output_tokens": 2
                    }
                }
            }),
        );

        let response = OpenAiResponsesAdapter::finalize_stream(acc).unwrap();
        assert_eq!(response.actual_model.as_deref(), Some("gpt-test-actual"));
        assert_eq!(response.input_tokens, 4);
        assert_eq!(response.output_tokens, 2);
        assert_eq!(response.outputs.len(), 2);
        assert!(matches!(&response.outputs[0], LlmOutput::Text(text) if text == "done"));
        assert!(matches!(
            &response.outputs[1],
            LlmOutput::ToolCall(call)
                if call.name == "read_file" && call.arguments["path"] == "AGENTS.md"
        ));
    }

    #[test]
    fn text_delta_fallback_returns_text_when_completed_response_absent() {
        let mut acc = ResponseStreamAccumulator::default();

        OpenAiResponsesAdapter::apply_stream_event(
            &mut acc,
            json!({ "type": "response.output_text.delta", "delta": "hel" }),
        );
        OpenAiResponsesAdapter::apply_stream_event(
            &mut acc,
            json!({ "type": "response.output_text.delta", "delta": "lo" }),
        );

        let response = OpenAiResponsesAdapter::finalize_stream(acc).unwrap();
        assert_eq!(response.outputs.len(), 1);
        assert!(matches!(&response.outputs[0], LlmOutput::Text(text) if text == "hello"));
    }

    #[test]
    fn openai_adapter_reports_vendor() {
        let auth_provider = Arc::new(StaticOpenAiAuthProvider::new(OpenAiAuth::new("test-key")));
        let adapter = OpenAiResponsesAdapter::new(auth_provider);

        assert_eq!(adapter.vendor(), ModelVendor::OpenAi);
    }

    #[test]
    fn responses_endpoint_appends_only_once() {
        assert_eq!(
            responses_endpoint(OPENAI_RESPONSES_BASE_URL),
            "https://api.openai.com/v1/responses"
        );

        assert_eq!(
            responses_endpoint("https://api.openai.com/v1/responses"),
            "https://api.openai.com/v1/responses"
        );
    }
}
