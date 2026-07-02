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

async fn initialized_provider_for(
    provider_type: ProviderType,
    default_model: &'static str,
    mut config: GaryxNativeConfig,
    client: Arc<FakeModelAdapter>,
) -> GaryxNativeProvider {
    config.provider_type = provider_type.clone();
    let mut provider =
        GaryxNativeProvider::with_model_adapter_for(provider_type, default_model, config, client);
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
    assert!(
        result
            .sdk_session_id
            .as_deref()
            .unwrap()
            .starts_with("garyx-native-")
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
async fn claude_model_backend_uses_backend_type_and_default_model() {
    let client = Arc::new(FakeModelAdapter::new(vec![NativeModelResponse {
        outputs: vec![NativeModelOutput::Text("done".to_owned())],
        ..Default::default()
    }]));
    let provider = initialized_provider_for(
        ProviderType::ClaudeLlm,
        DEFAULT_CLAUDE_MODEL,
        GaryxNativeConfig {
            provider_type: ProviderType::ClaudeLlm,
            default_model: String::new(),
            ..Default::default()
        },
        client.clone(),
    )
    .await;

    assert_eq!(provider.provider_type(), ProviderType::ClaudeLlm);
    provider
        .run_streaming(&options(None), Box::new(|_| {}))
        .await
        .unwrap();

    let requests = client.requests();
    assert_eq!(requests[0].model, DEFAULT_CLAUDE_MODEL);
}

#[tokio::test]
async fn gemini_model_backend_uses_backend_type_and_default_model() {
    let client = Arc::new(FakeModelAdapter::new(vec![NativeModelResponse {
        outputs: vec![NativeModelOutput::Text("done".to_owned())],
        ..Default::default()
    }]));
    let provider = initialized_provider_for(
        ProviderType::GeminiLlm,
        DEFAULT_GEMINI_MODEL,
        GaryxNativeConfig {
            provider_type: ProviderType::GeminiLlm,
            default_model: String::new(),
            ..Default::default()
        },
        client.clone(),
    )
    .await;

    assert_eq!(provider.provider_type(), ProviderType::GeminiLlm);
    provider
        .run_streaming(&options(None), Box::new(|_| {}))
        .await
        .unwrap();

    let requests = client.requests();
    assert_eq!(requests[0].model, DEFAULT_GEMINI_MODEL);
}

#[tokio::test]
async fn claude_auth_reads_runtime_env_and_config_base_url() {
    let provider = GaryxAnthropicAuthProvider {
        config: GaryxNativeConfig {
            base_url: "https://anthropic.example.test/v1".to_owned(),
            ..Default::default()
        },
    };
    let runtime = LlmRuntimeContext {
        env: HashMap::from([
            (
                "ANTHROPIC_API_KEY".to_owned(),
                "test-anthropic-key".to_owned(),
            ),
            ("ANTHROPIC_VERSION".to_owned(), "2099-01-01".to_owned()),
            ("ANTHROPIC_BETA".to_owned(), "test-beta".to_owned()),
        ]),
        metadata: HashMap::new(),
    };

    let auth = provider.resolve_auth(&runtime).await.unwrap();

    assert_eq!(
        auth.credential,
        AnthropicCredential::ApiKey("test-anthropic-key".to_owned())
    );
    assert_eq!(auth.base_url, "https://anthropic.example.test/v1");
    assert_eq!(auth.version, "2099-01-01");
    assert_eq!(auth.beta.as_deref(), Some("test-beta"));
}

#[tokio::test]
async fn claude_auth_can_use_claude_code_oauth_token() {
    let provider = GaryxAnthropicAuthProvider {
        config: GaryxNativeConfig::default(),
    };
    let runtime = LlmRuntimeContext {
        env: HashMap::from([(
            "CLAUDE_CODE_OAUTH_TOKEN".to_owned(),
            "test-oauth-token".to_owned(),
        )]),
        metadata: HashMap::new(),
    };

    let auth = provider.resolve_auth(&runtime).await.unwrap();

    assert_eq!(
        auth.credential,
        AnthropicCredential::BearerToken("test-oauth-token".to_owned())
    );
    assert_eq!(auth.base_url, ANTHROPIC_MESSAGES_BASE_URL);
}

#[tokio::test]
async fn gemini_auth_reads_runtime_env_and_config_base_url() {
    let provider = GaryxGoogleAuthProvider {
        config: GaryxNativeConfig {
            base_url: "https://gemini.example.test/v1beta".to_owned(),
            ..Default::default()
        },
    };
    let runtime = LlmRuntimeContext {
        env: HashMap::from([("GEMINI_API_KEY".to_owned(), "test-gemini-key".to_owned())]),
        metadata: HashMap::new(),
    };

    let auth = provider.resolve_auth(&runtime).await.unwrap();

    assert_eq!(
        auth.credential,
        GoogleCredential::ApiKey("test-gemini-key".to_owned())
    );
    assert_eq!(auth.base_url, "https://gemini.example.test/v1beta");
}

#[tokio::test]
async fn gemini_auth_can_use_runtime_oauth_token() {
    let provider = GaryxGoogleAuthProvider {
        config: GaryxNativeConfig::default(),
    };
    let runtime = LlmRuntimeContext {
        env: HashMap::from([(
            "GEMINI_OAUTH_ACCESS_TOKEN".to_owned(),
            "test-google-oauth-token".to_owned(),
        )]),
        metadata: HashMap::new(),
    };

    let auth = provider.resolve_auth(&runtime).await.unwrap();

    assert_eq!(
        auth.credential,
        GoogleCredential::CodeAssistOAuth("test-google-oauth-token".to_owned())
    );
    assert_eq!(auth.base_url, GOOGLE_CODE_ASSIST_BASE_URL);
}

#[tokio::test]
async fn gemini_auth_can_use_direct_generative_ai_bearer_token() {
    let provider = GaryxGoogleAuthProvider {
        config: GaryxNativeConfig::default(),
    };
    let runtime = LlmRuntimeContext {
        env: HashMap::from([(
            "GOOGLE_GENERATIVE_AI_ACCESS_TOKEN".to_owned(),
            "test-google-access-token".to_owned(),
        )]),
        metadata: HashMap::new(),
    };

    let auth = provider.resolve_auth(&runtime).await.unwrap();

    assert_eq!(
        auth.credential,
        GoogleCredential::BearerToken("test-google-access-token".to_owned())
    );
    assert_eq!(auth.base_url, GOOGLE_GENERATIVE_AI_BASE_URL);
}

#[tokio::test]
async fn gemini_oauth_auth_can_use_code_assist_endpoint_parts() {
    let provider = GaryxGoogleAuthProvider {
        config: GaryxNativeConfig::default(),
    };
    let runtime = LlmRuntimeContext {
        env: HashMap::from([
            (
                "GEMINI_OAUTH_ACCESS_TOKEN".to_owned(),
                "test-google-oauth-token".to_owned(),
            ),
            (
                "CODE_ASSIST_ENDPOINT".to_owned(),
                "https://codeassist.example.test".to_owned(),
            ),
            ("CODE_ASSIST_API_VERSION".to_owned(), "v1test".to_owned()),
        ]),
        metadata: HashMap::new(),
    };

    let auth = provider.resolve_auth(&runtime).await.unwrap();

    assert_eq!(
        auth.credential,
        GoogleCredential::CodeAssistOAuth("test-google-oauth-token".to_owned())
    );
    assert_eq!(auth.base_url, "https://codeassist.example.test/v1test");
}

#[tokio::test]
async fn gemini_auth_refreshes_expired_cli_oauth_cache() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("oauth_creds.json"),
        serde_json::to_string(&json!({
            "access_token": "expired-access-token",
            "expiry_date": 1,
            "refresh_token": "test-refresh-token",
            "token_type": "Bearer"
        }))
        .unwrap(),
    )
    .unwrap();
    let (token_url, captured_request) = oauth_token_test_server(
        json!({
            "access_token": "fresh-access-token",
            "expires_in": 3600,
            "token_type": "Bearer",
            "scope": "test-scope"
        })
        .to_string(),
    )
    .await;
    let provider = GaryxGoogleAuthProvider {
        config: GaryxNativeConfig::default(),
    };
    let runtime = LlmRuntimeContext {
        env: HashMap::from([
            (
                "GEMINI_CLI_HOME".to_owned(),
                temp.path().display().to_string(),
            ),
            ("GEMINI_OAUTH_TOKEN_URL".to_owned(), token_url),
            (
                "GEMINI_OAUTH_CLIENT_ID".to_owned(),
                "test-client-id".to_owned(),
            ),
            (
                "GEMINI_OAUTH_CLIENT_SECRET".to_owned(),
                "test-client-secret".to_owned(),
            ),
        ]),
        metadata: HashMap::new(),
    };

    let auth = provider.resolve_auth(&runtime).await.unwrap();

    assert_eq!(
        auth.credential,
        GoogleCredential::CodeAssistOAuth("fresh-access-token".to_owned())
    );
    let request = captured_request.lock().unwrap().clone();
    assert!(request.contains("grant_type=refresh_token"));
    assert!(request.contains("refresh_token=test-refresh-token"));
    assert!(request.contains("client_id=test-client-id"));
    assert!(request.contains("client_secret=test-client-secret"));
    let cache = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(temp.path().join("oauth_creds.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(cache["access_token"], "fresh-access-token");
    assert_eq!(cache["refresh_token"], "test-refresh-token");
    assert_eq!(cache["scope"], "test-scope");
    assert!(cache["expiry_date"].as_i64().unwrap() > 1);
}

async fn oauth_token_test_server(body: String) -> (String, Arc<StdMutex<String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let captured_request = Arc::new(StdMutex::new(String::new()));
    let captured = Arc::clone(&captured_request);
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let read = socket.read(&mut chunk).await.unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if http_request_is_complete(&buffer) {
                break;
            }
        }
        *captured.lock().unwrap() = String::from_utf8_lossy(&buffer).into_owned();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    (format!("http://{address}/token"), captured_request)
}

fn http_request_is_complete(buffer: &[u8]) -> bool {
    let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    buffer.len() >= header_end + 4 + content_length
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
                metadata: Default::default(),
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

#[tokio::test]
async fn run_streaming_keeps_started_run_model_when_defaults_reload_mid_run() {
    let client = Arc::new(FakeModelAdapter::new(vec![NativeModelResponse {
        outputs: vec![NativeModelOutput::Text("done".to_owned())],
        input_tokens: 1,
        output_tokens: 1,
        actual_model: None,
    }]));
    let mut config = GaryxNativeConfig::default();
    config.model = "gpt-old".to_owned();
    let provider = initialized_provider(config, client.clone()).await;

    // Pre-create the session and hold its lock so the run parks after it has
    // registered itself as active (and, with the fix, after it captured the
    // effective config) but before it can build the model request.
    let run_options = options(None);
    let session = provider.ensure_session(&run_options).await;
    let session_guard = session.lock().await;

    let run_future = provider.run_streaming(&run_options, Box::new(|_| {}));

    let orchestrate = async {
        // The run inserts into active_runs before it awaits the session lock;
        // once the run id is visible the config capture (which happens even
        // earlier) is guaranteed to have completed.
        for _ in 0..500 {
            if provider
                .active_runs
                .lock()
                .await
                .contains_key("run-native-test")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            provider
                .active_runs
                .lock()
                .await
                .contains_key("run-native-test"),
            "run never registered as active"
        );
        provider.update_model_defaults(&ProviderModelDefaults {
            model: "gpt-new".to_owned(),
            default_model: "gpt-new".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
        });
        drop(session_guard);
    };

    let (result, ()) = tokio::join!(run_future, orchestrate);
    let result = result.expect("run should succeed");
    assert!(result.success, "run failed: {:?}", result.error);

    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].model, "gpt-old",
        "an already-started run must keep the model captured at run start, \
         not pick up defaults reloaded mid-run"
    );
}

#[test]
fn build_exec_command_overlays_agent_env_without_leaking() {
    // Deterministic proof that the native shell tool (`exec_command`) injects
    // the agent's runtime env onto the spawned Command, closing the gap where
    // it previously inherited only the parent process environment.
    let mut env = HashMap::new();
    env.insert("TEST_AGENT_ENV_KEY".to_owned(), "test-value".to_owned());
    let cmd = build_exec_command("echo hello", Path::new("/Users/test"), &env);
    let std_cmd = cmd.as_std();

    let has_env = std_cmd.get_envs().any(|(key, value)| {
        key == std::ffi::OsStr::new("TEST_AGENT_ENV_KEY")
            && value == Some(std::ffi::OsStr::new("test-value"))
    });
    assert!(has_env, "agent env must reach the exec_command shell subprocess");

    // The env value must not leak into program/args (no-proactive-leak).
    assert!(!std_cmd.get_program().to_string_lossy().contains("test-value"));
    assert!(
        std_cmd
            .get_args()
            .all(|arg| !arg.to_string_lossy().contains("test-value"))
    );
}
