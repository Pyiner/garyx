use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use garyx_models::provider::{
    ProviderRunOptions, QueuedUserInput, StreamBoundaryKind, StreamEvent,
};
use serde_json::json;

use garyx_models::codex_models::{
    CHATGPT_CODEX_BASE_URL, OPENAI_RESPONSES_BASE_URL, resolve_codex_auth,
};

use super::*;

struct FakeModelAdapter {
    responses: StdMutex<VecDeque<NativeModelResponse>>,
    requests: StdMutex<Vec<NativeModelRequest>>,
}

impl FakeModelAdapter {
    fn new(responses: Vec<NativeModelResponse>) -> Self {
        Self {
            responses: StdMutex::new(VecDeque::from(responses)),
            requests: StdMutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<NativeModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmAdapter for FakeModelAdapter {
    fn vendor(&self) -> ModelVendor {
        ModelVendor::OpenAi
    }

    async fn sample(
        &self,
        request: NativeModelRequest,
    ) -> Result<NativeModelResponse, AgentLoopError> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AgentLoopError::failed("fake model response exhausted"))
    }
}

fn options(workspace_dir: Option<String>) -> ProviderRunOptions {
    ProviderRunOptions {
        thread_id: "thread::native-test".to_owned(),
        message: "make the change".to_owned(),
        workspace_dir,
        images: None,
        metadata: HashMap::from([("bridge_run_id".to_owned(), json!("run-native-test"))]),
    }
}

async fn initialized_provider(
    config: GaryxNativeConfig,
    client: Arc<FakeModelAdapter>,
) -> GaryxNativeProvider {
    let mut provider = GaryxNativeProvider::with_model_adapter(config, client);
    provider.initialize().await.unwrap();
    provider
}

#[test]
fn http_response_body_enables_streaming_and_reasoning_effort() {
    let request = NativeModelRequest {
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
        options: LlmRequestOptions {
            reasoning_effort: Some("high".to_owned()),
            service_tier: Some("priority".to_owned()),
        },
        runtime: LlmRuntimeContext::default(),
    };

    let body = GptResponsesModelBackend::response_body(
        &request,
        vec![json!({ "role": "user", "content": "hello" })],
    );

    assert_eq!(body["model"], "gpt-test");
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
    assert_eq!(body["reasoning"]["effort"], "high");
    assert_eq!(body["service_tier"], "priority");
    assert_eq!(body["input"][0]["content"], "hello");
}

#[test]
fn streaming_completed_response_reuses_stream_output_items_when_final_output_is_empty() {
    let mut acc = ResponseStreamAccumulator::default();

    GptResponsesModelBackend::apply_stream_event(
        &mut acc,
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "reasoning",
                "summary": []
            }
        }),
    );
    GptResponsesModelBackend::apply_stream_event(
        &mut acc,
        json!({
            "type": "response.output_text.done",
            "text": "done"
        }),
    );
    GptResponsesModelBackend::apply_stream_event(
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
    GptResponsesModelBackend::apply_stream_event(
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
    GptResponsesModelBackend::apply_stream_event(
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

    let response = GptResponsesModelBackend::finalize_stream(acc).unwrap();
    assert_eq!(response.actual_model.as_deref(), Some("gpt-test-actual"));
    assert_eq!(response.input_tokens, 4);
    assert_eq!(response.output_tokens, 2);
    assert_eq!(response.outputs.len(), 2);
    assert!(matches!(&response.outputs[0], NativeModelOutput::Text(text) if text == "done"));
    assert!(matches!(
        &response.outputs[1],
        NativeModelOutput::ToolCall(call)
            if call.name == "read_file" && call.arguments["path"] == "AGENTS.md"
    ));
}

#[test]
fn streaming_text_delta_fallback_returns_text_when_completed_response_absent() {
    let mut acc = ResponseStreamAccumulator::default();

    GptResponsesModelBackend::apply_stream_event(
        &mut acc,
        json!({ "type": "response.output_text.delta", "delta": "hel" }),
    );
    GptResponsesModelBackend::apply_stream_event(
        &mut acc,
        json!({ "type": "response.output_text.delta", "delta": "lo" }),
    );

    let response = GptResponsesModelBackend::finalize_stream(acc).unwrap();
    assert_eq!(response.outputs.len(), 1);
    assert!(matches!(&response.outputs[0], NativeModelOutput::Text(text) if text == "hello"));
}

#[tokio::test]
async fn assistant_only_turn_streams_delta_and_persists_assistant_message() {
    let client = Arc::new(FakeModelAdapter::new(vec![NativeModelResponse {
        outputs: vec![NativeModelOutput::Text("done".to_owned())],
        input_tokens: 10,
        output_tokens: 2,
        actual_model: Some("gpt-test".to_owned()),
    }]));
    let provider = initialized_provider(GaryxNativeConfig::default(), client.clone()).await;
    let events = Arc::new(StdMutex::new(Vec::new()));
    let events_cb = events.clone();

    let result = provider
        .run_streaming(
            &options(None),
            Box::new(move |event| events_cb.lock().unwrap().push(event)),
        )
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(result.response, "done");
    assert_eq!(result.session_messages.len(), 1);
    assert_eq!(result.session_messages[0].text.as_deref(), Some("done"));
    assert_eq!(
        result
            .sdk_session_id
            .as_deref()
            .unwrap()
            .starts_with("garyx-native-"),
        true
    );
    let events = events.lock().unwrap().clone();
    assert!(matches!(events[0], StreamEvent::SessionBound { .. }));
    assert!(events.contains(&StreamEvent::Delta {
        text: "done".to_owned()
    }));
    assert!(events.contains(&StreamEvent::Done));

    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, "gpt-5.5");
    assert!(requests[0].messages.iter().any(|message| {
        message
            .text
            .as_deref()
            .is_some_and(|text| text.contains("make the change"))
    }));
}

#[tokio::test]
async fn tool_call_runs_and_follow_up_request_sees_tool_result() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("demo.txt");
    let client = Arc::new(FakeModelAdapter::new(vec![
        NativeModelResponse {
            outputs: vec![NativeModelOutput::ToolCall(NativeToolCall {
                id: "call-write".to_owned(),
                name: "write_file".to_owned(),
                arguments: json!({
                    "path": "demo.txt",
                    "content": "created by test"
                }),
            })],
            ..Default::default()
        },
        NativeModelResponse {
            outputs: vec![NativeModelOutput::Text("wrote file".to_owned())],
            ..Default::default()
        },
    ]));
    let provider = initialized_provider(GaryxNativeConfig::default(), client.clone()).await;

    let result = provider
        .run_streaming(
            &options(Some(temp.path().display().to_string())),
            Box::new(|_| {}),
        )
        .await
        .unwrap();

    assert_eq!(std::fs::read_to_string(target).unwrap(), "created by test");
    assert_eq!(result.response, "wrote file");
    assert_eq!(result.session_messages.len(), 3);
    assert_eq!(result.session_messages[0].role_str(), "tool_use");
    assert_eq!(result.session_messages[1].role_str(), "tool_result");
    assert_eq!(result.session_messages[2].role_str(), "assistant");

    let requests = client.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message.role_str() == "tool_result")
    );
}

#[tokio::test]
async fn queued_streaming_input_is_acknowledged_and_sampled_again() {
    let client = Arc::new(FakeModelAdapter::new(vec![
        NativeModelResponse {
            outputs: vec![NativeModelOutput::Text("first".to_owned())],
            ..Default::default()
        },
        NativeModelResponse {
            outputs: vec![NativeModelOutput::Text("second".to_owned())],
            ..Default::default()
        },
    ]));
    let provider = initialized_provider(GaryxNativeConfig::default(), client.clone()).await;
    provider
        .get_or_create_session("thread::native-test")
        .await
        .unwrap();
    assert!(
        provider
            .add_streaming_input(
                "thread::native-test",
                QueuedUserInput::text("follow up").with_pending_input_id("pending-1"),
            )
            .await
    );
    let events = Arc::new(StdMutex::new(Vec::new()));
    let events_cb = events.clone();

    let result = provider
        .run_streaming(
            &options(None),
            Box::new(move |event| events_cb.lock().unwrap().push(event)),
        )
        .await
        .unwrap();

    assert_eq!(result.response, "firstsecond");
    let events = events.lock().unwrap().clone();
    assert!(events.contains(&StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: Some("pending-1".to_owned()),
    }));
    let requests = client.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]
            .messages
            .iter()
            .any(|message| message.text.as_deref() == Some("follow up"))
    );
}

#[tokio::test]
async fn failed_model_request_clears_active_run() {
    let client = Arc::new(FakeModelAdapter::new(Vec::new()));
    let provider = initialized_provider(GaryxNativeConfig::default(), client).await;

    let result = provider
        .run_streaming(&options(None), Box::new(|_| {}))
        .await;

    assert!(result.is_err());
    assert!(
        !provider
            .active_runs
            .lock()
            .await
            .contains_key("run-native-test")
    );
}

#[test]
fn native_auth_prefers_codex_api_key_from_runtime_env() {
    let config = GaryxNativeConfig::default();
    let env = HashMap::from([("CODEX_API_KEY".to_owned(), "test-api-key".to_owned())]);

    let auth = resolve_codex_auth(&config, &env).unwrap();

    assert_eq!(auth.bearer_token, "test-api-key");
    assert_eq!(auth.base_url, OPENAI_RESPONSES_BASE_URL);
    assert_eq!(auth.account_id, None);
}

#[test]
fn native_auth_reads_chatgpt_token_from_codex_auth_file() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        serde_json::to_string(&json!({
            "tokens": {
                "access_token": "test-access-token",
                "refresh_token": "test-refresh-token",
                "account_id": "test-account",
                "id_token": {}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let config = GaryxNativeConfig {
        codex_home: temp.path().display().to_string(),
        ..Default::default()
    };

    let auth = resolve_codex_auth(&config, &HashMap::new()).unwrap();

    assert_eq!(auth.bearer_token, "test-access-token");
    assert_eq!(auth.base_url, CHATGPT_CODEX_BASE_URL);
    assert_eq!(auth.account_id.as_deref(), Some("test-account"));
}
