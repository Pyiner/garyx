use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use antigravity_sdk::{
    AntigravityClient, AntigravityClientConfig, AntigravityError, AntigravityEvent,
    AntigravityRunRequest, ApprovalCallback, ApprovalDecision, ApprovalFuture,
};
use async_trait::async_trait;
use garyx_models::local_paths::home_dir;
use garyx_models::provider::{
    AntigravityCliConfig, PromptAttachment, ProviderMessage, ProviderMessageRole,
    ProviderRunOptions, ProviderRunResult, ProviderType, SDK_SESSION_FORK_METADATA_KEY,
    StreamEvent, attachments_from_metadata, build_prompt_message_with_attachments,
    default_antigravity_model, stage_image_payloads_for_prompt,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::gary_prompt::{compose_gary_instructions, prepend_initial_context_to_user_message};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_common::{
    metadata_bool, normalize_non_empty, resolve_uuid_run_id as resolve_run_id, runtime_env_overlay,
};
use crate::provider_trait::{
    BridgeError, ClearSessionOutcome, ProviderModelDefaults, ProviderRuntime,
    ProviderRuntimeSelection, StreamCallback,
};

const DEFAULT_REQUEST_TIMEOUT_SECS: f64 = 300.0;

fn resolve_runtime_antigravity_env(
    config: &AntigravityCliConfig,
    metadata: &HashMap<String, Value>,
) -> HashMap<String, String> {
    runtime_env_overlay(&config.env, metadata, "desktop_antigravity_env")
}

fn antigravity_bin(config: &AntigravityCliConfig) -> &str {
    let trimmed = config.antigravity_bin.trim();
    if trimmed.is_empty() { "agy" } else { trimmed }
}

fn model_id(config: &AntigravityCliConfig, metadata: &HashMap<String, Value>) -> String {
    normalize_non_empty(metadata.get("model").and_then(Value::as_str))
        .or_else(|| normalize_non_empty(Some(config.model.as_str())))
        .or_else(|| normalize_non_empty(Some(config.default_model.as_str())))
        .unwrap_or_else(default_antigravity_model)
}

fn request_timeout(config: &AntigravityCliConfig) -> Duration {
    let timeout = if config.timeout_seconds > 0.0 {
        config.timeout_seconds
    } else {
        DEFAULT_REQUEST_TIMEOUT_SECS
    };
    Duration::from_secs_f64(timeout)
}

fn resolve_workspace_dir(
    config: &AntigravityCliConfig,
    options: &ProviderRunOptions,
) -> Option<PathBuf> {
    options
        .workspace_dir
        .as_ref()
        .or(config.workspace_dir.as_ref())
        .map(|value| PathBuf::from(shellexpand::tilde(value).as_ref()))
        .filter(|value| value.exists())
        .or_else(|| std::env::current_dir().ok())
}

fn configured_brain_root(config: &AntigravityCliConfig) -> Option<PathBuf> {
    normalize_non_empty(Some(config.antigravity_brain_root.as_str()))
        .map(|value| PathBuf::from(shellexpand::tilde(&value).as_ref()))
        .or_else(|| {
            home_dir().map(|home| home.join(".gemini").join("antigravity-cli").join("brain"))
        })
}

fn run_log_path() -> PathBuf {
    let dir = std::env::temp_dir().join("garyx-antigravity");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("run-{}.log", Uuid::new_v4()))
}

fn build_prompt_text(options: &ProviderRunOptions, include_instructions: bool) -> String {
    let mut attachments = attachments_from_metadata(&options.metadata);
    if attachments.is_empty() {
        attachments.extend(stage_image_payloads_for_prompt(
            "garyx-antigravity",
            options.images.as_deref().unwrap_or_default(),
        ));
    }
    build_prompt_text_from_attachments(options, include_instructions, &attachments)
}

fn build_prompt_text_from_attachments(
    options: &ProviderRunOptions,
    include_instructions: bool,
    attachments: &[PromptAttachment],
) -> String {
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message =
        prepend_initial_context_to_user_message(&message, &options.metadata, include_instructions);
    let user_message = build_prompt_message_with_attachments(&message, attachments);
    if !include_instructions {
        return user_message;
    }

    let runtime_system_prompt = options
        .metadata
        .get("system_prompt")
        .and_then(Value::as_str);
    let instructions = compose_gary_instructions(runtime_system_prompt);

    if user_message.trim().is_empty() {
        format!("<system_instructions>\n{instructions}\n</system_instructions>")
    } else {
        format!(
            "<system_instructions>\n{instructions}\n</system_instructions>\n\n<user_request>\n{user_message}\n</user_request>"
        )
    }
}

/// Garyx's current Antigravity product policy. The SDK requires this callback
/// and has no fallback approval decision of its own.
fn garyx_approval_callback() -> ApprovalCallback {
    Arc::new(|_| Box::pin(async { Ok(ApprovalDecision::BypassPermissions) }) as ApprovalFuture)
}

fn provider_message_timestamp(created_at: Option<&str>) -> String {
    created_at
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
}

fn source_metadata() -> Value {
    json!("antigravity")
}

fn append_antigravity_assistant_session_message(
    session_messages: &mut Vec<ProviderMessage>,
    delta: &str,
    created_at: Option<&str>,
    reasoning: Option<&str>,
) {
    if delta.is_empty() {
        return;
    }
    let can_append = session_messages.last().is_some_and(|message| {
        message.role == ProviderMessageRole::Assistant
            && message.metadata.get("source").and_then(Value::as_str) == Some("antigravity")
    });
    if can_append {
        if let Some(last) = session_messages.last_mut() {
            let mut text = last.text.clone().unwrap_or_default();
            text.push_str(delta);
            last.text = Some(text.clone());
            last.content = Value::String(text);
            if let Some(reasoning) = reasoning.filter(|value| !value.trim().is_empty()) {
                let current = last
                    .metadata
                    .get("provider_reasoning")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let joined = if current.is_empty() {
                    reasoning.to_owned()
                } else {
                    format!("{current}\n\n{reasoning}")
                };
                last.metadata
                    .insert("provider_reasoning".to_owned(), Value::String(joined));
            }
        }
        return;
    }

    let mut entry = ProviderMessage::assistant_text(delta)
        .with_timestamp(provider_message_timestamp(created_at))
        .with_metadata_value("source", source_metadata());
    if let Some(reasoning) = reasoning.filter(|value| !value.trim().is_empty()) {
        entry = entry.with_metadata_value("provider_reasoning", json!(reasoning));
    }
    session_messages.push(entry);
}

#[derive(Default)]
struct EventMapper {
    response: String,
    session_messages: Vec<ProviderMessage>,
}

impl EventMapper {
    fn apply(&mut self, event: AntigravityEvent, on_chunk: &StreamCallback) {
        match event {
            AntigravityEvent::SessionBound { conversation_id } => {
                on_chunk(StreamEvent::SessionBound {
                    sdk_session_id: conversation_id,
                });
            }
            AntigravityEvent::AssistantDelta {
                text,
                reasoning,
                created_at,
                ..
            } => {
                self.response.push_str(&text);
                on_chunk(StreamEvent::Delta { text: text.clone() });
                append_antigravity_assistant_session_message(
                    &mut self.session_messages,
                    &text,
                    created_at.as_deref(),
                    reasoning.as_deref(),
                );
            }
            AntigravityEvent::ToolUse {
                tool_use_id,
                name,
                input,
                created_at,
                ..
            } => {
                let content = json!({
                    "name": name,
                    "args": input,
                });
                let message = ProviderMessage::tool_use(content, Some(tool_use_id), Some(name))
                    .with_timestamp(provider_message_timestamp(created_at.as_deref()))
                    .with_metadata_value("source", source_metadata());
                on_chunk(StreamEvent::ToolUse {
                    message: message.clone(),
                });
                self.session_messages.push(message);
            }
            AntigravityEvent::ToolResult {
                tool_use_id,
                name,
                content,
                is_error,
                created_at,
                ..
            } => {
                let message = ProviderMessage::tool_result(
                    content,
                    tool_use_id,
                    Some(name),
                    is_error.then_some(true),
                )
                .with_timestamp(provider_message_timestamp(created_at.as_deref()))
                .with_metadata_value("source", source_metadata());
                on_chunk(StreamEvent::ToolResult {
                    message: message.clone(),
                });
                self.session_messages.push(message);
            }
            // The legacy adapter recorded transcript errors in the final run
            // result but did not emit a Garyx stream event for a bare error.
            AntigravityEvent::Error { .. } => {}
        }
    }
}

struct RunThreadRegistration<'a> {
    run_id: String,
    map: &'a Mutex<HashMap<String, String>>,
}

impl<'a> RunThreadRegistration<'a> {
    fn new(map: &'a Mutex<HashMap<String, String>>, run_id: &str, thread_id: &str) -> Self {
        map.lock()
            .expect("antigravity run map lock poisoned")
            .insert(run_id.to_owned(), thread_id.to_owned());
        Self {
            run_id: run_id.to_owned(),
            map,
        }
    }
}

impl Drop for RunThreadRegistration<'_> {
    fn drop(&mut self) {
        self.map
            .lock()
            .expect("antigravity run map lock poisoned")
            .remove(&self.run_id);
    }
}

struct RunOnceResult {
    result: ProviderRunResult,
    invalid_conversation: bool,
}

enum RunOnceError {
    InvalidConversation(String),
    Bridge(BridgeError),
}

fn map_sdk_run_error(error: AntigravityError) -> RunOnceError {
    match error {
        AntigravityError::InvalidConversation(message) => {
            RunOnceError::InvalidConversation(message)
        }
        AntigravityError::Timeout => RunOnceError::Bridge(BridgeError::Timeout),
        AntigravityError::Spawn(message) => RunOnceError::Bridge(BridgeError::Internal(format!(
            "failed to spawn antigravity CLI: {message}"
        ))),
        AntigravityError::Transport(message) => {
            RunOnceError::Bridge(BridgeError::Internal(message))
        }
        other => RunOnceError::Bridge(BridgeError::RunFailed(other.to_string())),
    }
}

pub struct AntigravityCliProvider {
    config: AntigravityCliConfig,
    /// Hot-reloadable model defaults. Config reloads reconcile onto the live
    /// provider instance (the provider key excludes model defaults to keep
    /// thread affinity stable), so model resolution must read these instead
    /// of the frozen `config` fields.
    model_defaults: std::sync::RwLock<ProviderModelDefaults>,
    session_map: Mutex<HashMap<String, String>>,
    run_session_map: Mutex<HashMap<String, String>>,
    client: AntigravityClient,
    ready: bool,
}

impl AntigravityCliProvider {
    pub fn new(config: AntigravityCliConfig) -> Self {
        let model_defaults = std::sync::RwLock::new(ProviderModelDefaults {
            model: config.model.clone(),
            default_model: config.default_model.clone(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
        });
        let client = AntigravityClient::new(AntigravityClientConfig::new(
            antigravity_bin(&config),
            configured_brain_root(&config).unwrap_or_default(),
        ));
        Self {
            config,
            model_defaults,
            session_map: Mutex::new(HashMap::new()),
            run_session_map: Mutex::new(HashMap::new()),
            client,
            ready: false,
        }
    }

    /// Clone the frozen config with the hot-reloadable model defaults
    /// overlaid, so model resolution observes the latest reloaded defaults.
    fn effective_config(&self) -> AntigravityCliConfig {
        let defaults = self
            .model_defaults
            .read()
            .expect("antigravity model defaults lock poisoned")
            .clone();
        let mut config = self.config.clone();
        config.model = if defaults.model.is_empty() {
            defaults.default_model.clone()
        } else {
            defaults.model.clone()
        };
        config.default_model = defaults.default_model;
        config
    }

    async fn run_once(
        &self,
        options: &ProviderRunOptions,
        run_id: &str,
        session_id: Option<&str>,
        on_chunk: &StreamCallback,
    ) -> Result<RunOnceResult, RunOnceError> {
        let workspace_dir = resolve_workspace_dir(&self.config, options).ok_or_else(|| {
            RunOnceError::Bridge(BridgeError::RunFailed(
                "antigravity workspace directory is unavailable".to_owned(),
            ))
        })?;
        if configured_brain_root(&self.config).is_none() {
            return Err(RunOnceError::Bridge(BridgeError::RunFailed(
                "antigravity brain root is unavailable".to_owned(),
            )));
        }
        let model = model_id(&self.effective_config(), &options.metadata);
        let prompt = build_prompt_text(options, session_id.is_none());
        let request = AntigravityRunRequest {
            run_id: run_id.to_owned(),
            prompt,
            discovery_text: options.message.clone(),
            model: model.clone(),
            conversation_id: session_id.map(ToOwned::to_owned),
            workspace_dir,
            log_path: run_log_path(),
            env: resolve_runtime_antigravity_env(&self.config, &options.metadata),
            print_timeout: request_timeout(&self.config),
            approval_callback: garyx_approval_callback(),
        };

        let _run_registration =
            RunThreadRegistration::new(&self.run_session_map, run_id, &options.thread_id);
        let mapper = Mutex::new(EventMapper::default());
        let event_callback = |event: AntigravityEvent| {
            if let AntigravityEvent::SessionBound { conversation_id } = &event {
                self.session_map
                    .lock()
                    .expect("antigravity session map lock poisoned")
                    .insert(options.thread_id.clone(), conversation_id.clone());
            }
            mapper
                .lock()
                .expect("antigravity event mapper lock poisoned")
                .apply(event, on_chunk);
        };
        let outcome = self
            .client
            .execute(request, &event_callback)
            .await
            .map_err(map_sdk_run_error)?;
        let mapper = mapper
            .into_inner()
            .expect("antigravity event mapper lock poisoned");
        let invalid_conversation = outcome
            .failure
            .as_ref()
            .is_some_and(|failure| failure.is_invalid_conversation());
        let error = outcome.failure.map(|failure| failure.message);

        on_chunk(StreamEvent::Done);

        Ok(RunOnceResult {
            result: ProviderRunResult {
                run_id: run_id.to_owned(),
                thread_id: options.thread_id.clone(),
                response: mapper.response,
                session_messages: mapper.session_messages,
                sdk_session_id: Some(outcome.conversation_id),
                actual_model: Some(model),
                thread_title: None,
                success: outcome.success,
                error,
                input_tokens: 0,
                output_tokens: 0,
                cost: 0.0,
                duration_ms: outcome.duration.as_millis() as i64,
            },
            invalid_conversation,
        })
    }

    async fn retry_without_session(
        &self,
        options: &ProviderRunOptions,
        run_id: &str,
        on_chunk: &StreamCallback,
    ) -> Result<RunOnceResult, BridgeError> {
        self.session_map
            .lock()
            .expect("antigravity session map lock poisoned")
            .remove(&options.thread_id);
        self.run_once(options, run_id, None, on_chunk)
            .await
            .map_err(|error| match error {
                RunOnceError::InvalidConversation(message) => BridgeError::RunFailed(message),
                RunOnceError::Bridge(error) => error,
            })
    }
}

#[async_trait]
impl ProviderRuntime for AntigravityCliProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::AntigravityCli
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        ProviderRuntimeSelection {
            model: Some(model_id(&self.effective_config(), &options.metadata)),
            model_reasoning_effort: None,
            model_service_tier: None,
        }
    }

    fn update_model_defaults(&self, defaults: &ProviderModelDefaults) {
        *self
            .model_defaults
            .write()
            .expect("antigravity model defaults lock poisoned") = defaults.clone();
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        if self.ready {
            return Ok(());
        }
        match self.client.probe().await {
            Ok(()) => {
                self.ready = true;
                Ok(())
            }
            Err(AntigravityError::NotReady(_)) => Err(BridgeError::ProviderNotReady),
            Err(AntigravityError::Spawn(message)) => Err(BridgeError::Internal(format!(
                "failed to invoke antigravity CLI: {message}"
            ))),
            Err(error) => Err(BridgeError::Internal(error.to_string())),
        }
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        self.client.shutdown().await;
        self.run_session_map
            .lock()
            .expect("antigravity run map lock poisoned")
            .clear();
        self.session_map
            .lock()
            .expect("antigravity session map lock poisoned")
            .clear();
        self.ready = false;
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        if !self.ready {
            return Err(BridgeError::ProviderNotReady);
        }

        if metadata_bool(&options.metadata, SDK_SESSION_FORK_METADATA_KEY) {
            return Err(BridgeError::SessionError(
                "antigravity provider does not support sdk session fork".to_owned(),
            ));
        }

        let run_id = resolve_run_id(&options.metadata);
        let session_id = self
            .session_map
            .lock()
            .expect("antigravity session map lock poisoned")
            .get(&options.thread_id)
            .cloned()
            .or_else(|| {
                normalize_non_empty(
                    options
                        .metadata
                        .get("sdk_session_id")
                        .and_then(Value::as_str),
                )
            });

        let (mut attempt, retried) = match self
            .run_once(options, &run_id, session_id.as_deref(), &on_chunk)
            .await
        {
            Ok(attempt) => (attempt, false),
            Err(RunOnceError::InvalidConversation(_)) if session_id.is_some() => (
                self.retry_without_session(options, &run_id, &on_chunk)
                    .await?,
                true,
            ),
            Err(RunOnceError::InvalidConversation(message)) => {
                return Err(BridgeError::RunFailed(message));
            }
            Err(RunOnceError::Bridge(error)) => return Err(error),
        };

        if !retried && attempt.invalid_conversation && session_id.is_some() {
            attempt = self
                .retry_without_session(options, &run_id, &on_chunk)
                .await?;
        }

        Ok(attempt.result)
    }

    async fn abort(&self, run_id: &str) -> bool {
        self.run_session_map
            .lock()
            .expect("antigravity run map lock poisoned")
            .remove(run_id);
        self.client.abort(run_id).await
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        Ok(self
            .session_map
            .lock()
            .expect("antigravity session map lock poisoned")
            .get(thread_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn clear_session(&self, thread_id: &str) -> ClearSessionOutcome {
        let active_run_ids = self
            .run_session_map
            .lock()
            .expect("antigravity run map lock poisoned")
            .iter()
            .filter(|(_, mapped_thread_id)| mapped_thread_id.as_str() == thread_id)
            .map(|(run_id, _)| run_id.clone())
            .collect::<Vec<_>>();
        for run_id in active_run_ids {
            let _ = self.abort(&run_id).await;
        }
        if self
            .session_map
            .lock()
            .expect("antigravity session map lock poisoned")
            .remove(thread_id)
            .is_some()
        {
            ClearSessionOutcome::Cleared
        } else {
            ClearSessionOutcome::AlreadyAbsent
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    use super::*;

    #[test]
    fn resolve_runtime_antigravity_env_overlays_agent_and_run_env() {
        let mut config = AntigravityCliConfig::default();
        config
            .env
            .insert("TEST_AGENT_ENV_KEY".to_owned(), "test-value".to_owned());
        config.env.insert(
            "GARYX_THREAD_ID".to_owned(),
            "agent-should-not-win".to_owned(),
        );
        config
            .env
            .insert("SYNTHETIC_PRECEDENCE".to_owned(), "agent-value".to_owned());
        let metadata = HashMap::from([
            ("agent_id".to_owned(), serde_json::json!("antigravity")),
            (
                "runtime_context".to_owned(),
                serde_json::json!({ "thread_id": "thread::synthetic-task" }),
            ),
            (
                "desktop_antigravity_env".to_owned(),
                serde_json::json!({ "SYNTHETIC_PRECEDENCE": "run-value" }),
            ),
        ]);

        let env = resolve_runtime_antigravity_env(&config, &metadata);

        assert_eq!(
            env.get("TEST_AGENT_ENV_KEY").map(String::as_str),
            Some("test-value")
        );
        assert_eq!(
            env.get("GARYX_THREAD_ID").map(String::as_str),
            Some("thread::synthetic-task")
        );
        assert_eq!(
            env.get("SYNTHETIC_PRECEDENCE").map(String::as_str),
            Some("run-value")
        );
    }

    #[test]
    fn sdk_events_map_to_garyx_stream_and_session_messages() {
        let events = StdArc::new(StdMutex::new(Vec::new()));
        let events_for_callback = StdArc::clone(&events);
        let callback: StreamCallback = Box::new(move |event| {
            events_for_callback.lock().unwrap().push(event);
        });
        let mut mapper = EventMapper::default();
        mapper.apply(
            AntigravityEvent::ToolUse {
                step_index: 2,
                tool_use_id: "antigravity-tool-2-0".to_owned(),
                name: "RUN_COMMAND".to_owned(),
                input: json!({"command": "pwd"}),
                created_at: Some("2026-01-01T00:00:00Z".to_owned()),
            },
            &callback,
        );
        mapper.apply(
            AntigravityEvent::ToolResult {
                step_index: 3,
                tool_use_id: Some("antigravity-tool-2-0".to_owned()),
                name: "RUN_COMMAND".to_owned(),
                content: json!({"type": "RUN_COMMAND", "content": "stdout"}),
                is_error: false,
                created_at: Some("2026-01-01T00:00:01Z".to_owned()),
            },
            &callback,
        );
        mapper.apply(
            AntigravityEvent::AssistantDelta {
                step_index: 4,
                text: "done".to_owned(),
                reasoning: Some("checking".to_owned()),
                created_at: Some("2026-01-01T00:00:02Z".to_owned()),
            },
            &callback,
        );
        mapper.apply(
            AntigravityEvent::Error {
                step_index: 5,
                message: "not streamed".to_owned(),
                created_at: None,
            },
            &callback,
        );

        assert_eq!(mapper.response, "done");
        let events = events.lock().unwrap();
        assert_eq!(
            events.len(),
            3,
            "bare SDK errors must not become stream rows"
        );
        assert!(matches!(events[0], StreamEvent::ToolUse { .. }));
        assert!(matches!(events[1], StreamEvent::ToolResult { .. }));
        assert!(matches!(events[2], StreamEvent::Delta { .. }));
        let normal_result = mapper
            .session_messages
            .iter()
            .find(|message| message.role == ProviderMessageRole::ToolResult)
            .expect("tool result");
        assert_eq!(normal_result.is_error, None);
        let assistant = mapper
            .session_messages
            .iter()
            .find(|message| message.role == ProviderMessageRole::Assistant)
            .expect("assistant message");
        assert_eq!(
            assistant
                .metadata
                .get("provider_reasoning")
                .and_then(Value::as_str),
            Some("checking")
        );
    }

    #[tokio::test]
    async fn run_streaming_tails_fake_antigravity_through_sdk() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir_all(&workspace_dir).expect("workspace");
        let brain_root = temp
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("brain");
        let conversations_dir = temp
            .path()
            .join(".gemini")
            .join("antigravity-cli")
            .join("conversations");
        fs::create_dir_all(&brain_root).expect("brain");
        fs::create_dir_all(&conversations_dir).expect("conversations");
        let script_path = temp.path().join("fake-agy.py");
        let script = r#"#!/usr/bin/env python3
import json
import os
import sys
import time

if len(sys.argv) > 1 and sys.argv[1] == "models":
    print("Synthetic Model")
    sys.exit(0)
if "--dangerously-skip-permissions" not in sys.argv:
    print("missing caller approval", file=sys.stderr)
    sys.exit(9)

brain = os.environ["FAKE_AGY_BRAIN_ROOT"]
expected_thread = os.environ["EXPECTED_GARYX_THREAD_ID"]
if os.environ.get("GARYX_THREAD_ID") != expected_thread:
    print("wrong per-run identity", file=sys.stderr)
    sys.exit(8)
conversation = None
prompt = ""
for index, arg in enumerate(sys.argv):
    if arg == "--conversation":
        conversation = sys.argv[index + 1]
    if arg == "-p":
        prompt = sys.argv[index + 1]
if not conversation:
    conversation = "synthetic-session"
if conversation == "synthetic-stale-session":
    print("conversation not found", file=sys.stderr)
    sys.exit(7)

base = os.path.dirname(brain)
os.makedirs(os.path.join(base, "conversations"), exist_ok=True)
open(os.path.join(base, "conversations", conversation + ".db"), "a").close()
logs = os.path.join(brain, conversation, ".system_generated", "logs")
os.makedirs(logs, exist_ok=True)
path = os.path.join(logs, "transcript.jsonl")
mode = "a" if os.path.exists(path) else "w"
with open(path, mode) as transcript:
    transcript.write(json.dumps({"type":"USER_INPUT","step_index":1 if mode == "w" else 4,"content":prompt}) + "\n")
    transcript.flush()
    time.sleep(0.1)
    transcript.write(json.dumps({"type":"PLANNER_RESPONSE","step_index":2 if mode == "w" else 5,"content":"hello from agy"}) + "\n")
    transcript.flush()
sys.exit(0)
"#;
        fs::write(&script_path, script).expect("script");
        let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("chmod");

        let thread_id = "thread::synthetic-antigravity";
        let mut provider = AntigravityCliProvider::new(AntigravityCliConfig {
            antigravity_bin: script_path.to_string_lossy().to_string(),
            antigravity_brain_root: brain_root.to_string_lossy().to_string(),
            workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
            timeout_seconds: 5.0,
            env: HashMap::from([
                (
                    "FAKE_AGY_BRAIN_ROOT".to_owned(),
                    brain_root.to_string_lossy().to_string(),
                ),
                ("EXPECTED_GARYX_THREAD_ID".to_owned(), thread_id.to_owned()),
            ]),
            ..Default::default()
        });
        provider.initialize().await.expect("initialize");

        let events = StdArc::new(StdMutex::new(Vec::new()));
        let events_for_callback = StdArc::clone(&events);
        let callback: StreamCallback = Box::new(move |event| {
            events_for_callback.lock().unwrap().push(event);
        });
        let result = provider
            .run_streaming(
                &ProviderRunOptions {
                    thread_id: thread_id.to_owned(),
                    message: "say hi".to_owned(),
                    workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                    images: None,
                    metadata: HashMap::from([
                        (
                            "runtime_context".to_owned(),
                            json!({ "thread_id": thread_id }),
                        ),
                        (
                            "sdk_session_id".to_owned(),
                            json!("synthetic-stale-session"),
                        ),
                    ]),
                },
                callback,
            )
            .await
            .expect("run");

        assert!(result.success, "unexpected error: {:?}", result.error);
        assert_eq!(result.sdk_session_id.as_deref(), Some("synthetic-session"));
        assert_eq!(result.response, "hello from agy");
        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            StreamEvent::SessionBound { sdk_session_id } if sdk_session_id == "synthetic-session"
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, StreamEvent::Done))
        );
    }
}
