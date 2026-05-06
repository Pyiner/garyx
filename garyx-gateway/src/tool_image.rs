use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use garyx_channels::generated_images::{
    GeneratedImageResult, build_image_generation_prompt, extract_image_generation_result,
    provider_message_item_type,
};
use garyx_models::local_paths::gary_home_dir;
use garyx_models::provider::{AgentRunRequest, ProviderMessage, ProviderType, StreamEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::server::AppState;

const DEFAULT_TIMEOUT_SECS: u64 = 600;
const MAX_TIMEOUT_SECS: u64 = 900;

#[derive(Debug, Deserialize)]
pub struct GenerateImageRequest {
    prompt: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct GenerateImageResponse {
    ok: bool,
    data_base64: String,
    bytes: usize,
    media_type: Option<String>,
    runtime_thread_id: String,
    run_id: String,
    extra_images_seen: bool,
}

#[derive(Debug)]
struct ImageToolRun {
    runtime_thread_id: String,
    run_id: String,
    image: GeneratedImageResult,
    extra_images_seen: bool,
}

#[derive(Debug)]
enum ToolImageError {
    InvalidRequest(String),
    Bridge(String),
    Timeout { timeout_secs: u64, run_id: String },
    Provider(String),
    Io(String),
}

impl ToolImageError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            Self::InvalidRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Bridge(message) => (StatusCode::BAD_GATEWAY, message),
            Self::Timeout {
                timeout_secs,
                run_id,
            } => (
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "timed out after {timeout_secs}s waiting for image generation run {run_id}"
                ),
            ),
            Self::Provider(message) | Self::Io(message) => (StatusCode::BAD_GATEWAY, message),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

fn normalized_timeout(raw: Option<u64>) -> Result<u64, ToolImageError> {
    let timeout = raw.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if timeout == 0 {
        return Err(ToolImageError::InvalidRequest(
            "timeout_secs must be greater than 0".to_owned(),
        ));
    }
    Ok(timeout.min(MAX_TIMEOUT_SECS))
}

async fn tool_workspace_dir(tool_name: &str) -> Result<String, ToolImageError> {
    #[cfg(test)]
    let root = std::env::var_os("GARYX_TEST_TOOL_WORKSPACE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| gary_home_dir().join("tool-workspaces"));
    #[cfg(not(test))]
    let root = gary_home_dir().join("tool-workspaces");
    let dir = root.join(tool_name);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| ToolImageError::Io(error.to_string()))?;
    Ok(dir.to_string_lossy().into_owned())
}

fn extract_image_from_tool_result_message(
    message: &ProviderMessage,
) -> Result<Option<GeneratedImageResult>, ToolImageError> {
    if provider_message_item_type(message) != Some("imageGeneration") {
        return Ok(None);
    }
    let result = message
        .content
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if result.is_empty() {
        return Ok(None);
    }
    extract_image_generation_result(message)
        .map(Some)
        .ok_or_else(|| {
            ToolImageError::Provider(
                "generated image payload was malformed or not valid base64".to_owned(),
            )
        })
}

fn extract_image_from_stream_event(
    event: &StreamEvent,
) -> Result<Option<GeneratedImageResult>, ToolImageError> {
    match event {
        StreamEvent::ToolResult { message } => extract_image_from_tool_result_message(message),
        _ => Ok(None),
    }
}

async fn run_image_tool(
    state: Arc<AppState>,
    prompt: String,
    timeout_secs: u64,
) -> Result<ImageToolRun, ToolImageError> {
    let workspace_dir = tool_workspace_dir("image").await?;
    let runtime_thread_id = format!("tool::image::{}", Uuid::new_v4());
    let run_id = format!("tool-run-{}", Uuid::new_v4());
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamEvent>();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        let _ = tx.send(event);
    });
    let metadata = HashMap::from([("source".to_owned(), json!("garyx_tool_image"))]);

    let request = AgentRunRequest::new(
        runtime_thread_id.clone(),
        build_image_generation_prompt(&prompt),
        run_id.clone(),
        "tool",
        "image",
        metadata,
    )
    .with_workspace_dir(Some(workspace_dir))
    .with_requested_provider(Some(ProviderType::CodexAppServer));

    state
        .integration
        .bridge
        .start_agent_run(request, Some(callback))
        .await
        .map_err(|error| ToolImageError::Bridge(error.to_string()))?;

    let deadline = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(deadline);
    let mut first_image: Option<GeneratedImageResult> = None;
    let mut extra_images_seen = false;

    loop {
        tokio::select! {
            _ = &mut deadline => {
                let _ = state.integration.bridge.abort_run(&run_id).await;
                return Err(ToolImageError::Timeout {
                    timeout_secs,
                    run_id,
                });
            }
            event = rx.recv() => {
                let Some(event) = event else {
                    break;
                };
                if let Some(image) = extract_image_from_stream_event(&event)? {
                    if first_image.is_some() {
                        extra_images_seen = true;
                    } else {
                        first_image = Some(image);
                    }
                }
                if matches!(event, StreamEvent::Done) {
                    break;
                }
            }
        }
    }

    let image = first_image.ok_or_else(|| {
        ToolImageError::Provider("Codex completed without generating an image".to_owned())
    })?;

    Ok(ImageToolRun {
        runtime_thread_id,
        run_id,
        image,
        extra_images_seen,
    })
}

pub async fn generate_image(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<GenerateImageRequest>,
) -> impl IntoResponse {
    let prompt = payload.prompt.trim();
    if prompt.is_empty() {
        return ToolImageError::InvalidRequest("prompt is required".to_owned()).into_response();
    }
    let timeout_secs = match normalized_timeout(payload.timeout_secs) {
        Ok(timeout_secs) => timeout_secs,
        Err(error) => return error.into_response(),
    };

    match run_image_tool(state, prompt.to_owned(), timeout_secs).await {
        Ok(run) => Json(GenerateImageResponse {
            ok: true,
            data_base64: BASE64.encode(&run.image.bytes),
            bytes: run.image.bytes.len(),
            media_type: run.image.media_type,
            runtime_thread_id: run.runtime_thread_id,
            run_id: run.run_id,
            extra_images_seen: run.extra_images_seen,
        })
        .into_response(),
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::body::{Body, to_bytes};
    use garyx_bridge::MultiProviderBridge;
    use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
    use garyx_models::config::GaryxConfig;
    use garyx_models::provider::{ProviderRunOptions, ProviderRunResult};
    use serde_json::json;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::route_graph::build_router;
    use crate::server::AppStateBuilder;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, OsStr::new(value));
            }
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = &self.previous {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[derive(Debug, Clone)]
    struct RecordedRun {
        message: String,
        metadata: HashMap<String, Value>,
        workspace_dir: Option<String>,
    }

    struct ImageProvider {
        ready: AtomicBool,
        runs: Mutex<Vec<RecordedRun>>,
    }

    impl ImageProvider {
        fn new() -> Self {
            Self {
                ready: AtomicBool::new(true),
                runs: Mutex::new(Vec::new()),
            }
        }

        fn runs(&self) -> Vec<RecordedRun> {
            self.runs.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl AgentLoopProvider for ImageProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::CodexAppServer
        }

        fn is_ready(&self) -> bool {
            self.ready.load(Ordering::Relaxed)
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            self.ready.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            self.ready.store(false, Ordering::Relaxed);
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            self.runs.lock().unwrap().push(RecordedRun {
                message: options.message.clone(),
                metadata: options.metadata.clone(),
                workspace_dir: options.workspace_dir.clone(),
            });
            on_chunk(StreamEvent::ToolResult {
                message: ProviderMessage::tool_result(
                    json!({
                        "type": "imageGeneration",
                        "id": "img_test",
                        "media_type": "image/png",
                        "result": "data:image/png;base64,aGVsbG8="
                    }),
                    Some("img_test".to_owned()),
                    Some("imageGeneration".to_owned()),
                    Some(false),
                )
                .with_metadata_value("item_type", json!("imageGeneration")),
            });
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "image-provider-run".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "generated".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                thread_title: None,
                success: true,
                error: None,
                input_tokens: 1,
                output_tokens: 1,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    #[tokio::test]
    async fn generate_image_route_invokes_codex_tool_prompt() {
        let workspace_root = tempdir().unwrap();
        let _workspace_env =
            ScopedEnvVar::set_path("GARYX_TEST_TOOL_WORKSPACE_ROOT", workspace_root.path());
        let provider = Arc::new(ImageProvider::new());
        let bridge = Arc::new(MultiProviderBridge::new());
        bridge
            .register_provider("codex-image-provider", provider.clone())
            .await;
        bridge
            .set_default_provider_key("codex-image-provider")
            .await;
        let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
            GaryxConfig::default(),
        ))
        .with_bridge(bridge)
        .build();
        let router = build_router(state);
        let response = router
            .oneshot(
                crate::test_support::authed_request()
                    .method("POST")
                    .uri("/api/tools/image")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "prompt": "make a tidy avatar", "timeout_secs": 5 }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["data_base64"], "aGVsbG8=");
        assert_eq!(payload["media_type"], "image/png");

        let runs = provider.runs();
        assert_eq!(runs.len(), 1);
        assert!(
            runs[0]
                .message
                .contains("You are being invoked by `garyx tool image`")
        );
        assert!(runs[0].message.contains("make a tidy avatar"));
        assert_eq!(runs[0].metadata["source"], "garyx_tool_image");
        let canonical_workspace_root = std::fs::canonicalize(workspace_root.path()).unwrap();
        assert!(
            runs[0]
                .workspace_dir
                .as_deref()
                .unwrap_or_default()
                .starts_with(&canonical_workspace_root.to_string_lossy().to_string())
        );
    }
}
