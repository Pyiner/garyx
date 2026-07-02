use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
#[cfg(test)]
use garyx_agent_loop::LlmOutput as NativeModelOutput;
use garyx_agent_loop::adapters::anthropic::{
    ANTHROPIC_MESSAGES_BASE_URL, ANTHROPIC_VERSION, AnthropicAuth, AnthropicAuthProvider,
    AnthropicCredential, AnthropicMessagesAdapter,
};
use garyx_agent_loop::adapters::google::{
    GOOGLE_CODE_ASSIST_BASE_URL, GOOGLE_GENERATIVE_AI_BASE_URL, GoogleAuth, GoogleAuthProvider,
    GoogleCredential, GoogleGenerativeAiAdapter,
};
#[cfg(test)]
use garyx_agent_loop::adapters::openai::OpenAiResponsesAdapter as GptResponsesModelBackend;
#[cfg(test)]
use garyx_agent_loop::adapters::openai::ResponseStreamAccumulator;
use garyx_agent_loop::adapters::openai::{OpenAiAuth, OpenAiAuthProvider, OpenAiResponsesAdapter};
use garyx_agent_loop::{
    AgentLoopError, AgentLoopEvent, AgentLoopRunRequest, AgentLoopSession, ConversationMessage,
    ConversationRole, LlmAdapter, LlmRequestOptions, LlmRuntimeContext,
    LlmToolCall as NativeToolCall, PendingUserInput, QueueMode, ToolDefinition, ToolExecution,
    ToolExecutor, run_agent_loop,
};
#[cfg(test)]
use garyx_agent_loop::{
    LlmRequest as NativeModelRequest, LlmResponse as NativeModelResponse, ModelVendor,
};
use garyx_models::codex_models::resolve_codex_auth;
use garyx_models::provider::{
    GaryxNativeConfig, ProviderMessage, ProviderMessageRole, ProviderRunOptions, ProviderRunResult,
    ProviderType, QueuedUserInput, StreamBoundaryKind, StreamEvent, attachments_from_metadata,
    build_prompt_message_with_attachments,
};
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::{
    compose_gary_instructions, prepend_initial_context_to_user_message, task_cli_env,
};
use crate::native_capabilities::{
    capability_instructions, capability_tool_schemas, is_capability_tool, run_capability_tool,
};
use crate::provider_trait::{
    AgentLoopProvider, BridgeError, ProviderModelDefaults, ProviderRuntimeSelection, StreamCallback,
};

pub(crate) const SESSION_MESSAGES_METADATA_KEY: &str = "garyx_session_messages";
const DEFAULT_REQUEST_TIMEOUT_SECS: f64 = 300.0;
const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GEMINI_CLI_OAUTH_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const DEFAULT_GPT_MODEL: &str = "gpt-5.5";
const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-4-6";
const DEFAULT_GEMINI_MODEL: &str = "gemini-3-flash-preview";

fn resolve_run_id(metadata: &HashMap<String, Value>) -> String {
    metadata
        .get("bridge_run_id")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("client_run_id").and_then(Value::as_str))
        .or_else(|| metadata.get("run_id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("run_{}", Uuid::new_v4()))
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn request_timeout(config: &GaryxNativeConfig) -> Duration {
    let timeout = if config.request_timeout_seconds > 0.0 {
        config.request_timeout_seconds
    } else if config.timeout_seconds > 0.0 {
        config.timeout_seconds
    } else {
        DEFAULT_REQUEST_TIMEOUT_SECS
    };
    Duration::from_secs_f64(timeout)
}

fn model_id(
    config: &GaryxNativeConfig,
    metadata: &HashMap<String, Value>,
    fallback: &str,
) -> String {
    normalize_non_empty(metadata.get("model").and_then(Value::as_str))
        .or_else(|| normalize_non_empty(Some(config.model.as_str())))
        .or_else(|| normalize_non_empty(Some(config.default_model.as_str())))
        .unwrap_or_else(|| fallback.to_owned())
}

fn metadata_string_map(metadata: &HashMap<String, Value>, key: &str) -> HashMap<String, String> {
    metadata
        .get(key)
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|value| (name.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_runtime_env(
    config: &GaryxNativeConfig,
    metadata: &HashMap<String, Value>,
) -> HashMap<String, String> {
    let mut env = config.env.clone();
    env.extend(task_cli_env(metadata));
    env.extend(metadata_string_map(metadata, "desktop_codex_env"));
    env.extend(metadata_string_map(metadata, "desktop_gpt_env"));
    env.extend(metadata_string_map(metadata, "desktop_claude_env"));
    env.extend(metadata_string_map(metadata, "desktop_anthropic_env"));
    env.extend(metadata_string_map(metadata, "desktop_gemini_env"));
    env.extend(metadata_string_map(metadata, "desktop_google_env"));
    env.extend(metadata_string_map(metadata, "desktop_garyx_native_env"));
    env
}

fn resolve_workspace_dir(config: &GaryxNativeConfig, options: &ProviderRunOptions) -> PathBuf {
    options
        .workspace_dir
        .as_ref()
        .or(config.workspace_dir.as_ref())
        .map(|value| PathBuf::from(shellexpand::tilde(value).as_ref()))
        .filter(|value| value.exists())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn truncate_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut clipped = value
        .chars()
        .take(limit.saturating_sub(20))
        .collect::<String>();
    clipped.push_str("\n[truncated]");
    clipped
}

fn persisted_session_messages(metadata: &HashMap<String, Value>) -> Vec<ProviderMessage> {
    metadata
        .get(SESSION_MESSAGES_METADATA_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<ProviderMessage>>(value).ok())
        .unwrap_or_default()
}

fn conversation_from_provider(message: ProviderMessage) -> ConversationMessage {
    let role = match message.role {
        ProviderMessageRole::User => ConversationRole::User,
        ProviderMessageRole::Assistant => ConversationRole::Assistant,
        ProviderMessageRole::System => ConversationRole::System,
        ProviderMessageRole::ToolUse => ConversationRole::ToolUse,
        ProviderMessageRole::ToolResult => ConversationRole::ToolResult,
    };
    ConversationMessage {
        role,
        content: message.content,
        text: message.text,
        timestamp: message.timestamp,
        metadata: message.metadata,
        tool_call_id: message.tool_use_id,
        tool_name: message.tool_name,
        is_error: message.is_error,
    }
}

fn provider_from_conversation(message: ConversationMessage) -> ProviderMessage {
    let role = match message.role {
        ConversationRole::User => ProviderMessageRole::User,
        ConversationRole::Assistant => ProviderMessageRole::Assistant,
        ConversationRole::System => ProviderMessageRole::System,
        ConversationRole::ToolUse => ProviderMessageRole::ToolUse,
        ConversationRole::ToolResult => ProviderMessageRole::ToolResult,
    };
    ProviderMessage {
        role,
        content: message.content,
        text: message.text,
        timestamp: message.timestamp,
        metadata: message.metadata,
        tool_use_id: message.tool_call_id,
        tool_name: message.tool_name,
        is_error: message.is_error,
    }
}

fn pending_from_queued(input: QueuedUserInput) -> PendingUserInput {
    PendingUserInput {
        pending_input_id: input.pending_input_id,
        message: input.message,
    }
}

struct GaryxOpenAiAuthProvider {
    config: GaryxNativeConfig,
}

#[async_trait]
impl OpenAiAuthProvider for GaryxOpenAiAuthProvider {
    async fn resolve_auth(
        &self,
        runtime: &LlmRuntimeContext,
    ) -> Result<OpenAiAuth, AgentLoopError> {
        let auth = resolve_codex_auth(&self.config, &runtime.env)
            .map_err(|error| AgentLoopError::failed(error.to_string()))?;
        Ok(OpenAiAuth {
            bearer_token: auth.bearer_token,
            base_url: auth.base_url,
            account_id: auth.account_id,
        })
    }
}

struct GaryxAnthropicAuthProvider {
    config: GaryxNativeConfig,
}

#[async_trait]
impl AnthropicAuthProvider for GaryxAnthropicAuthProvider {
    async fn resolve_auth(
        &self,
        runtime: &LlmRuntimeContext,
    ) -> Result<AnthropicAuth, AgentLoopError> {
        let credential = resolve_env_value(runtime, &["ANTHROPIC_API_KEY", "CLAUDE_API_KEY"])
            .map(AnthropicCredential::ApiKey)
            .or_else(|| {
                resolve_env_value(
                    runtime,
                    &[
                        "CLAUDE_CODE_OAUTH_TOKEN",
                        "ANTHROPIC_AUTH_TOKEN",
                        "CLAUDE_OAUTH_TOKEN",
                    ],
                )
                .map(AnthropicCredential::BearerToken)
            })
            .ok_or_else(|| {
                AgentLoopError::failed(
                    "missing ANTHROPIC_API_KEY, CLAUDE_API_KEY, or CLAUDE_CODE_OAUTH_TOKEN for Claude model provider",
                )
            })?;
        let base_url = normalize_non_empty(Some(self.config.base_url.as_str()))
            .or_else(|| resolve_env_value(runtime, &["ANTHROPIC_BASE_URL", "CLAUDE_BASE_URL"]))
            .unwrap_or_else(|| ANTHROPIC_MESSAGES_BASE_URL.to_owned());
        let version = resolve_env_value(runtime, &["ANTHROPIC_VERSION"])
            .unwrap_or_else(|| ANTHROPIC_VERSION.to_owned());
        let beta = resolve_env_value(runtime, &["ANTHROPIC_BETA"]);
        Ok(AnthropicAuth {
            credential,
            base_url,
            version,
            beta,
        })
    }
}

struct GaryxGoogleAuthProvider {
    config: GaryxNativeConfig,
}

#[async_trait]
impl GoogleAuthProvider for GaryxGoogleAuthProvider {
    async fn resolve_auth(
        &self,
        runtime: &LlmRuntimeContext,
    ) -> Result<GoogleAuth, AgentLoopError> {
        let credential = if let Some(api_key) =
            resolve_env_value(runtime, &["GEMINI_API_KEY", "GOOGLE_API_KEY"])
        {
            GoogleCredential::ApiKey(api_key)
        } else if let Some(access_token) =
            resolve_env_value(runtime, &["GOOGLE_GENERATIVE_AI_ACCESS_TOKEN"])
        {
            GoogleCredential::BearerToken(access_token)
        } else if let Some(access_token) = resolve_gemini_oauth_token(runtime).await? {
            GoogleCredential::CodeAssistOAuth(access_token)
        } else {
            return Err(AgentLoopError::failed(
                "missing GEMINI_API_KEY, GOOGLE_API_KEY, or valid Gemini OAuth token for Gemini model provider",
            ));
        };
        let base_url = match &credential {
            GoogleCredential::CodeAssistOAuth(_) => {
                normalize_non_empty(Some(self.config.base_url.as_str()))
                    .or_else(|| {
                        resolve_env_value(
                            runtime,
                            &[
                                "GEMINI_CODE_ASSIST_BASE_URL",
                                "GOOGLE_CODE_ASSIST_BASE_URL",
                                "CODE_ASSIST_BASE_URL",
                            ],
                        )
                    })
                    .or_else(|| code_assist_base_url(runtime))
                    .unwrap_or_else(|| GOOGLE_CODE_ASSIST_BASE_URL.to_owned())
            }
            _ => normalize_non_empty(Some(self.config.base_url.as_str()))
                .or_else(|| {
                    resolve_env_value(
                        runtime,
                        &[
                            "GEMINI_BASE_URL",
                            "GOOGLE_GENERATIVE_AI_BASE_URL",
                            "GOOGLE_API_BASE_URL",
                        ],
                    )
                })
                .unwrap_or_else(|| GOOGLE_GENERATIVE_AI_BASE_URL.to_owned()),
        };
        Ok(GoogleAuth {
            credential,
            base_url,
        })
    }
}

fn code_assist_base_url(runtime: &LlmRuntimeContext) -> Option<String> {
    let endpoint = resolve_env_value(runtime, &["CODE_ASSIST_ENDPOINT"])?;
    let version = resolve_env_value(runtime, &["CODE_ASSIST_API_VERSION"])
        .unwrap_or_else(|| "v1internal".to_owned());
    Some(format!(
        "{}/{}",
        endpoint.trim().trim_end_matches('/'),
        version.trim().trim_start_matches('/')
    ))
}

fn resolve_env_value(runtime: &LlmRuntimeContext, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = runtime
            .env
            .get(*key)
            .map(String::as_str)
            .and_then(|value| normalize_non_empty(Some(value)))
        {
            return Some(value);
        }
        if let Ok(value) = std::env::var(key)
            && let Some(value) = normalize_non_empty(Some(value.as_str()))
        {
            return Some(value);
        }
    }
    None
}

async fn resolve_gemini_oauth_token(
    runtime: &LlmRuntimeContext,
) -> Result<Option<String>, AgentLoopError> {
    if let Some(access_token) = resolve_env_value(
        runtime,
        &["GEMINI_OAUTH_ACCESS_TOKEN", "GOOGLE_OAUTH_ACCESS_TOKEN"],
    ) {
        return Ok(Some(access_token));
    }
    refresh_or_read_gemini_oauth_cache(runtime).await
}

async fn refresh_or_read_gemini_oauth_cache(
    runtime: &LlmRuntimeContext,
) -> Result<Option<String>, AgentLoopError> {
    let Some(path) = gemini_oauth_cache_path(runtime) else {
        return Ok(None);
    };
    let value = match std::fs::read_to_string(&path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let mut value = serde_json::from_str::<Value>(&value).map_err(|error| {
        AgentLoopError::failed(format!("Gemini OAuth cache is invalid JSON: {error}"))
    })?;
    if let Some(access_token) = valid_access_token(&value) {
        return Ok(Some(access_token));
    }
    let Some(refresh_token) = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .and_then(|value| normalize_non_empty(Some(value)))
    else {
        return Ok(None);
    };
    let refreshed = refresh_gemini_oauth_token(runtime, refresh_token).await?;
    merge_gemini_oauth_refresh(&mut value, &refreshed)?;
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).map_err(|error| {
            AgentLoopError::failed(format!("failed to serialize Gemini OAuth cache: {error}"))
        })?,
    )
    .map_err(|error| {
        AgentLoopError::failed(format!("failed to write Gemini OAuth cache: {error}"))
    })?;
    Ok(Some(refreshed.access_token))
}

fn gemini_oauth_cache_path(runtime: &LlmRuntimeContext) -> Option<PathBuf> {
    resolve_env_value(runtime, &["GEMINI_CLI_HOME"])
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".gemini"))
        })
        .map(|home| home.join("oauth_creds.json"))
}

fn valid_access_token(value: &Value) -> Option<String> {
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .and_then(|value| normalize_non_empty(Some(value)))?;
    let expiry_date = value.get("expiry_date").and_then(Value::as_i64)?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())?;
    (expiry_date > now_ms + 60_000).then_some(access_token)
}

#[derive(Debug)]
struct GeminiOauthRefresh {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    id_token: Option<String>,
    scope: Option<String>,
    token_type: Option<String>,
}

async fn refresh_gemini_oauth_token(
    runtime: &LlmRuntimeContext,
    refresh_token: String,
) -> Result<GeminiOauthRefresh, AgentLoopError> {
    let token_url = resolve_env_value(
        runtime,
        &["GEMINI_OAUTH_TOKEN_URL", "GOOGLE_OAUTH_TOKEN_URL"],
    )
    .unwrap_or_else(|| GOOGLE_OAUTH_TOKEN_URL.to_owned());
    let client_id = resolve_env_value(
        runtime,
        &["GEMINI_OAUTH_CLIENT_ID", "GOOGLE_OAUTH_CLIENT_ID"],
    )
    .unwrap_or_else(|| GEMINI_CLI_OAUTH_CLIENT_ID.to_owned());
    let client_secret = resolve_env_value(
        runtime,
        &["GEMINI_OAUTH_CLIENT_SECRET", "GOOGLE_OAUTH_CLIENT_SECRET"],
    )
    .ok_or_else(|| {
        AgentLoopError::failed(
            "Gemini OAuth cache is expired; set GEMINI_OAUTH_CLIENT_SECRET or refresh the Gemini CLI login",
        )
    })?;

    let response = reqwest::Client::new()
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
        ])
        .send()
        .await
        .map_err(|error| AgentLoopError::failed(format!("Gemini OAuth refresh failed: {error}")))?;
    let status = response.status();
    let text = response.text().await.map_err(|error| {
        AgentLoopError::failed(format!(
            "Gemini OAuth refresh response read failed: {error}"
        ))
    })?;
    if !status.is_success() {
        return Err(AgentLoopError::failed(format!(
            "Gemini OAuth refresh failed with {status}: {text}"
        )));
    }
    let value = serde_json::from_str::<Value>(&text).map_err(|error| {
        AgentLoopError::failed(format!(
            "Gemini OAuth refresh response was invalid JSON: {error}; body={text}"
        ))
    })?;
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .and_then(|value| normalize_non_empty(Some(value)))
        .ok_or_else(|| {
            AgentLoopError::failed("Gemini OAuth refresh response did not include access_token")
        })?;
    Ok(GeminiOauthRefresh {
        access_token,
        refresh_token: value
            .get("refresh_token")
            .and_then(Value::as_str)
            .and_then(|value| normalize_non_empty(Some(value))),
        expires_in: value.get("expires_in").and_then(Value::as_i64),
        id_token: value
            .get("id_token")
            .and_then(Value::as_str)
            .and_then(|value| normalize_non_empty(Some(value))),
        scope: value
            .get("scope")
            .and_then(Value::as_str)
            .and_then(|value| normalize_non_empty(Some(value))),
        token_type: value
            .get("token_type")
            .and_then(Value::as_str)
            .and_then(|value| normalize_non_empty(Some(value))),
    })
}

fn merge_gemini_oauth_refresh(
    value: &mut Value,
    refreshed: &GeminiOauthRefresh,
) -> Result<(), AgentLoopError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| AgentLoopError::failed("Gemini OAuth cache root must be an object"))?;
    object.insert(
        "access_token".to_owned(),
        Value::String(refreshed.access_token.clone()),
    );
    if let Some(refresh_token) = &refreshed.refresh_token {
        object.insert(
            "refresh_token".to_owned(),
            Value::String(refresh_token.clone()),
        );
    }
    if let Some(expires_in) = refreshed.expires_in {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| AgentLoopError::failed(format!("system clock error: {error}")))
            .and_then(|duration| {
                i64::try_from(duration.as_millis()).map_err(|error| {
                    AgentLoopError::failed(format!("system clock value overflow: {error}"))
                })
            })?;
        object.insert(
            "expiry_date".to_owned(),
            Value::Number(serde_json::Number::from(now_ms + expires_in * 1000)),
        );
    }
    if let Some(id_token) = &refreshed.id_token {
        object.insert("id_token".to_owned(), Value::String(id_token.clone()));
    }
    if let Some(scope) = &refreshed.scope {
        object.insert("scope".to_owned(), Value::String(scope.clone()));
    }
    if let Some(token_type) = &refreshed.token_type {
        object.insert("token_type".to_owned(), Value::String(token_type.clone()));
    }
    Ok(())
}

pub struct GaryxNativeProvider {
    config: GaryxNativeConfig,
    /// Hot-reloadable model defaults. Config reloads reconcile onto the live
    /// provider instance (the provider key excludes model defaults to keep
    /// thread affinity stable), so model resolution must read these instead
    /// of the frozen `config` fields.
    model_defaults: std::sync::RwLock<ProviderModelDefaults>,
    provider_type: ProviderType,
    default_model: &'static str,
    ready: Mutex<bool>,
    sessions: Mutex<HashMap<String, Arc<Mutex<AgentLoopSession>>>>,
    active_runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    model_adapter: Arc<dyn LlmAdapter>,
}

impl GaryxNativeProvider {
    pub fn new(config: GaryxNativeConfig) -> Self {
        Self::new_gpt(config)
    }

    pub fn new_gpt(config: GaryxNativeConfig) -> Self {
        let auth_provider = Arc::new(GaryxOpenAiAuthProvider {
            config: config.clone(),
        });
        let model_adapter = Arc::new(OpenAiResponsesAdapter::new(auth_provider));
        Self::with_model_adapter_for(ProviderType::Gpt, DEFAULT_GPT_MODEL, config, model_adapter)
    }

    pub fn new_claude(config: GaryxNativeConfig) -> Self {
        let auth_provider = Arc::new(GaryxAnthropicAuthProvider {
            config: config.clone(),
        });
        let model_adapter = Arc::new(AnthropicMessagesAdapter::new(auth_provider));
        Self::with_model_adapter_for(
            ProviderType::ClaudeLlm,
            DEFAULT_CLAUDE_MODEL,
            config,
            model_adapter,
        )
    }

    pub fn new_gemini(config: GaryxNativeConfig) -> Self {
        let auth_provider = Arc::new(GaryxGoogleAuthProvider {
            config: config.clone(),
        });
        let model_adapter = Arc::new(GoogleGenerativeAiAdapter::new(auth_provider));
        Self::with_model_adapter_for(
            ProviderType::GeminiLlm,
            DEFAULT_GEMINI_MODEL,
            config,
            model_adapter,
        )
    }

    #[cfg(test)]
    pub(crate) fn with_model_adapter(
        config: GaryxNativeConfig,
        model_adapter: Arc<dyn LlmAdapter>,
    ) -> Self {
        Self::with_model_adapter_for(ProviderType::Gpt, DEFAULT_GPT_MODEL, config, model_adapter)
    }

    pub(crate) fn with_model_adapter_for(
        provider_type: ProviderType,
        default_model: &'static str,
        config: GaryxNativeConfig,
        model_adapter: Arc<dyn LlmAdapter>,
    ) -> Self {
        let model_defaults = std::sync::RwLock::new(ProviderModelDefaults {
            model: config.model.clone(),
            default_model: config.default_model.clone(),
            model_reasoning_effort: config.model_reasoning_effort.clone(),
            model_service_tier: config.model_service_tier.clone(),
        });
        Self {
            config,
            model_defaults,
            provider_type,
            default_model,
            ready: Mutex::new(false),
            sessions: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            model_adapter,
        }
    }

    /// Clone the frozen config with the hot-reloadable model defaults
    /// overlaid, so model resolution observes the latest reloaded defaults.
    fn effective_config(&self) -> GaryxNativeConfig {
        let defaults = self
            .model_defaults
            .read()
            .expect("native model defaults lock poisoned")
            .clone();
        let mut config = self.config.clone();
        config.model = if defaults.model.is_empty() {
            defaults.default_model.clone()
        } else {
            defaults.model.clone()
        };
        config.default_model = defaults.default_model;
        config.model_reasoning_effort = defaults.model_reasoning_effort;
        config.model_service_tier = defaults.model_service_tier;
        config
    }

    async fn ensure_session(&self, options: &ProviderRunOptions) -> Arc<Mutex<AgentLoopSession>> {
        let restored_sid = options
            .metadata
            .get("sdk_session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .entry(options.thread_id.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(AgentLoopSession::new(
                    restored_sid
                        .clone()
                        .unwrap_or_else(|| format!("garyx-native-{}", Uuid::new_v4())),
                )))
            })
            .clone();
        drop(sessions);

        let persisted = persisted_session_messages(&options.metadata);
        if !persisted.is_empty() {
            let mut state = session.lock().await;
            if state.messages.is_empty() {
                state.messages = persisted
                    .into_iter()
                    .map(conversation_from_provider)
                    .collect();
            }
            if let Some(sid) = restored_sid
                && state.sdk_session_id != sid
            {
                state.sdk_session_id = sid;
            }
        }
        session
    }

    fn instructions(&self, options: &ProviderRunOptions, workspace_dir: &Path) -> String {
        let mut parts = Vec::new();
        let runtime_system_prompt = options
            .metadata
            .get("system_prompt")
            .and_then(Value::as_str);
        parts.push(compose_gary_instructions(
            runtime_system_prompt,
            options.workspace_dir.as_deref().map(Path::new),
            options
                .metadata
                .get("automation_id")
                .and_then(Value::as_str),
        ));
        let native_capabilities = capability_instructions(workspace_dir, &options.metadata);
        if !native_capabilities.trim().is_empty() {
            parts.push(native_capabilities);
        }
        parts.join("\n\n")
    }

    fn tool_schemas(
        workspace_dir: &Path,
        metadata: &HashMap<String, Value>,
    ) -> Vec<ToolDefinition> {
        let mut tools = vec![
            ToolDefinition::function(
                "exec_command",
                "Run a shell command in the active workspace.",
                json!({
                    "type": "object",
                    "properties": {
                        "cmd": { "type": "string" },
                        "timeout_seconds": { "type": "number" }
                    },
                    "required": ["cmd"],
                    "additionalProperties": false
                }),
            ),
            ToolDefinition::function(
                "read_file",
                "Read a UTF-8 text file.",
                json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            ),
            ToolDefinition::function(
                "write_file",
                "Write a UTF-8 text file.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            ),
            ToolDefinition::function(
                "list_dir",
                "List files and directories.",
                json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "additionalProperties": false
                }),
            ),
        ];
        tools.extend(capability_tool_schemas(workspace_dir, metadata));
        tools
    }

    async fn run_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
        metadata: &HashMap<String, Value>,
    ) -> (Value, bool) {
        let result = match call.name.as_str() {
            "exec_command" => self.exec_command_tool(call, workspace_dir).await,
            "read_file" => self.read_file_tool(call, workspace_dir).await,
            "write_file" => self.write_file_tool(call, workspace_dir).await,
            "list_dir" => self.list_dir_tool(call, workspace_dir).await,
            name if is_capability_tool(name) => {
                let runtime_env = resolve_runtime_env(&self.config, metadata);
                run_capability_tool(call, workspace_dir, metadata, &runtime_env).await
            }
            _ => Err(format!("unknown tool '{}'", call.name)),
        };
        match result {
            Ok(value) => (value, false),
            Err(error) => (json!({ "error": error }), true),
        }
    }

    async fn exec_command_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let cmd = call
            .arguments
            .get("cmd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing cmd".to_owned())?;
        let timeout = call
            .arguments
            .get("timeout_seconds")
            .and_then(Value::as_f64)
            .filter(|value| *value > 0.0)
            .unwrap_or(60.0)
            .min(900.0);
        let output = tokio::time::timeout(
            Duration::from_secs_f64(timeout),
            Command::new("zsh")
                .arg("-lc")
                .arg(cmd)
                .current_dir(workspace_dir)
                .output(),
        )
        .await
        .map_err(|_| format!("command timed out after {timeout:.0}s"))?
        .map_err(|error| format!("failed to run command: {error}"))?;
        Ok(json!({
            "status": output.status.code(),
            "success": output.status.success(),
            "stdout": truncate_text(&String::from_utf8_lossy(&output.stdout), MAX_TOOL_OUTPUT_CHARS),
            "stderr": truncate_text(&String::from_utf8_lossy(&output.stderr), MAX_TOOL_OUTPUT_CHARS),
        }))
    }

    async fn read_file_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let path = resolve_tool_path(workspace_dir, &call.arguments)?;
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Ok(json!({
            "path": path.display().to_string(),
            "content": truncate_text(&contents, MAX_TOOL_OUTPUT_CHARS),
        }))
    }

    async fn write_file_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let path = resolve_tool_path(workspace_dir, &call.arguments)?;
        let content = call
            .arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing content".to_owned())?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        Ok(json!({
            "path": path.display().to_string(),
            "bytes": content.len(),
        }))
    }

    async fn list_dir_tool(
        &self,
        call: &NativeToolCall,
        workspace_dir: &Path,
    ) -> Result<Value, String> {
        let path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .map(|value| path_from_arg(workspace_dir, value))
            .unwrap_or_else(|| workspace_dir.to_path_buf());
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|error| format!("failed to list {}: {error}", path.display()))?;
        let mut values = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?
        {
            let file_type = entry.file_type().await.ok();
            values.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "kind": if file_type.as_ref().is_some_and(|value| value.is_dir()) { "dir" } else { "file" },
            }));
        }
        Ok(json!({
            "path": path.display().to_string(),
            "entries": values,
        }))
    }
}

fn resolve_tool_path(workspace_dir: &Path, arguments: &Value) -> Result<PathBuf, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing path".to_owned())?;
    Ok(path_from_arg(workspace_dir, path))
}

fn path_from_arg(workspace_dir: &Path, path: &str) -> PathBuf {
    let expanded = PathBuf::from(shellexpand::tilde(path).as_ref());
    if expanded.is_absolute() {
        expanded
    } else {
        workspace_dir.join(expanded)
    }
}

struct BridgeToolExecutor<'a> {
    provider: &'a GaryxNativeProvider,
    workspace_dir: PathBuf,
    metadata: &'a HashMap<String, Value>,
}

#[async_trait]
impl ToolExecutor for BridgeToolExecutor<'_> {
    async fn execute_tool(&self, call: &NativeToolCall) -> ToolExecution {
        let (content, is_error) = self
            .provider
            .run_tool(call, &self.workspace_dir, self.metadata)
            .await;
        ToolExecution {
            content,
            is_error,
            terminate: false,
        }
    }
}

fn map_loop_error(error: AgentLoopError) -> BridgeError {
    match error {
        AgentLoopError::Timeout => BridgeError::Timeout,
        AgentLoopError::Failed(message) => BridgeError::RunFailed(message),
    }
}

#[async_trait]
impl AgentLoopProvider for GaryxNativeProvider {
    fn provider_type(&self) -> ProviderType {
        // The provider type is the selected model backend. The in-process
        // native loop is the execution engine behind this backend.
        self.provider_type.clone()
    }

    fn is_ready(&self) -> bool {
        self.ready.try_lock().map(|value| *value).unwrap_or(false)
    }

    fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        let effective_config = self.effective_config();
        ProviderRuntimeSelection {
            model: Some(model_id(
                &effective_config,
                &options.metadata,
                self.default_model,
            )),
            model_reasoning_effort: normalize_non_empty(
                options
                    .metadata
                    .get("model_reasoning_effort")
                    .and_then(Value::as_str)
                    .or(Some(effective_config.model_reasoning_effort.as_str())),
            ),
            model_service_tier: normalize_non_empty(
                options
                    .metadata
                    .get("model_service_tier")
                    .and_then(Value::as_str)
                    .or(Some(effective_config.model_service_tier.as_str())),
            ),
        }
    }

    fn update_model_defaults(&self, defaults: &ProviderModelDefaults) {
        *self
            .model_defaults
            .write()
            .expect("native model defaults lock poisoned") = defaults.clone();
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        *self.ready.lock().await = true;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        self.sessions.lock().await.clear();
        self.active_runs.lock().await.clear();
        *self.ready.lock().await = false;
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        let start = Instant::now();
        let run_id = resolve_run_id(&options.metadata);
        let workspace_dir = resolve_workspace_dir(&self.config, options);
        let session = self.ensure_session(options).await;
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_runs
            .lock()
            .await
            .insert(run_id.clone(), cancel.clone());

        let sdk_session_id = {
            let mut state = session.lock().await;
            state.interrupted = false;
            if !options.message.trim().is_empty() {
                let attachments = attachments_from_metadata(&options.metadata);
                let message = build_prompt_message_with_attachments(&options.message, &attachments);
                let message =
                    prepend_initial_context_to_user_message(&message, &options.metadata, true);
                state.messages.push(ConversationMessage::user_text(message));
            }
            state.sdk_session_id.clone()
        };
        let effective_config = self.effective_config();
        let request = AgentLoopRunRequest {
            model: model_id(&effective_config, &options.metadata, self.default_model),
            instructions: self.instructions(options, &workspace_dir),
            tools: Self::tool_schemas(&workspace_dir, &options.metadata),
            options: LlmRequestOptions {
                reasoning_effort: normalize_non_empty(
                    options
                        .metadata
                        .get("model_reasoning_effort")
                        .and_then(Value::as_str)
                        .or(Some(effective_config.model_reasoning_effort.as_str())),
                ),
                service_tier: normalize_non_empty(
                    options
                        .metadata
                        .get("model_service_tier")
                        .and_then(Value::as_str)
                        .or(Some(effective_config.model_service_tier.as_str())),
                ),
            },
            runtime: LlmRuntimeContext {
                env: resolve_runtime_env(&self.config, &options.metadata),
                metadata: options.metadata.clone(),
            },
            request_timeout: request_timeout(&self.config),
            max_tool_iterations: self.config.max_tool_iterations,
            max_turns: self
                .config
                .max_turns
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0),
            queue_mode: QueueMode::All,
            compaction: None,
        };
        let tool_executor = BridgeToolExecutor {
            provider: self,
            workspace_dir,
            metadata: &options.metadata,
        };
        let mut emitted_sdk_session_id = sdk_session_id.clone();
        let outcome = run_agent_loop(
            session,
            self.model_adapter.as_ref(),
            &tool_executor,
            request,
            cancel,
            |event| match event {
                AgentLoopEvent::SessionBound { sdk_session_id } => {
                    emitted_sdk_session_id = sdk_session_id.clone();
                    on_chunk(StreamEvent::SessionBound { sdk_session_id });
                }
                AgentLoopEvent::Delta { text } => on_chunk(StreamEvent::Delta { text }),
                AgentLoopEvent::ToolUse { message } => on_chunk(StreamEvent::ToolUse {
                    message: provider_from_conversation(message),
                }),
                AgentLoopEvent::ToolResult { message } => on_chunk(StreamEvent::ToolResult {
                    message: provider_from_conversation(message),
                }),
                AgentLoopEvent::UserAck { pending_input_id } => on_chunk(StreamEvent::Boundary {
                    kind: StreamBoundaryKind::UserAck,
                    pending_input_id,
                }),
                AgentLoopEvent::Done => on_chunk(StreamEvent::Done),
                _ => {}
            },
        )
        .await
        .map_err(map_loop_error);

        self.active_runs.lock().await.remove(&run_id);
        let outcome = outcome?;

        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response: outcome.response,
            session_messages: outcome
                .session_messages
                .into_iter()
                .map(provider_from_conversation)
                .collect(),
            sdk_session_id: Some(emitted_sdk_session_id),
            actual_model: outcome.actual_model,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: outcome.input_tokens,
            output_tokens: outcome.output_tokens,
            cost: 0.0,
            duration_ms: start.elapsed().as_millis() as i64,
        })
    }

    async fn abort(&self, run_id: &str) -> bool {
        self.active_runs
            .lock()
            .await
            .get(run_id)
            .map(|flag| {
                flag.store(true, Ordering::Relaxed);
                true
            })
            .unwrap_or(false)
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, thread_id: &str, input: QueuedUserInput) -> bool {
        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(thread_id).cloned()
        };
        let Some(session) = session else {
            return false;
        };
        session
            .lock()
            .await
            .pending_inputs
            .push_back(pending_from_queued(input));
        true
    }

    async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        let session = {
            let sessions = self.sessions.lock().await;
            sessions.get(thread_id).cloned()
        };
        let Some(session) = session else {
            return false;
        };
        session.lock().await.interrupted = true;
        true
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .entry(thread_id.to_owned())
            .or_insert_with(|| {
                Arc::new(Mutex::new(AgentLoopSession::new(format!(
                    "garyx-native-{}",
                    Uuid::new_v4()
                ))))
            })
            .clone();
        Ok(session.lock().await.sdk_session_id.clone())
    }

    async fn clear_session(&self, thread_id: &str) -> bool {
        self.sessions.lock().await.remove(thread_id).is_some()
    }
}

#[cfg(test)]
mod tests;
