use garyx_router::ThreadStoreExt;
use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, ImagePayload, ProviderType, attachments_to_metadata_value,
    merge_thread_model_cells, stage_file_payloads_for_prompt, stage_image_payloads_for_prompt,
};
use garyx_models::routing::DELIVERY_TARGET_TYPE_CHAT_ID;
use garyx_models::thread_logs::ThreadLogEvent;
use garyx_models::{SERVER_OWNED_AGENT_METADATA_KEYS, strip_server_owned_agent_metadata};
use garyx_router::{
    ChannelBinding, EndpointBindingMutator, NATIVE_COMMAND_TEXT_METADATA_KEY, ThreadCreationError,
    ThreadRecordPatch, WorkspaceMode, build_runtime_context_metadata,
    default_agent_for_channel_account, default_workspace_for_channel_account,
    default_workspace_mode_for_channel_account, endpoint_key, is_thread_key,
    normalize_workspace_dir, update_thread_record, workspace_dir_from_value,
};
use serde_json::{Value, json};

use crate::agent_identity::{agent_runtime_metadata, resolve_agent_reference_from_stores};
use crate::application::chat::contracts::ChatRequest;
use crate::chat_shared::record_api_thread_log;
use crate::garyx_db::PromptAttachmentClaim;
use crate::managed_mcp_metadata::inject_managed_mcp_servers;
use crate::prompt_attachment_lifecycle::PromptAttachmentLifecycleError;
use crate::server::AppState;
use crate::workspace_mode::ensure_implicit_thread_workspace_for_config;

const LEGACY_DEFAULT_THREAD_LABEL: &str = "Fresh Thread";
const PROMPT_THREAD_TITLE_SOURCE: &str = "garyx_prompt";
const PROVIDER_TYPE_PATCH_FIELDS: &[&str] = &["provider_type", "updated_at"];

#[derive(Debug)]
pub(crate) enum ChatPreparationError {
    InvalidRequest(StatusCode, Json<Value>),
    ThreadUpdateConflict {
        thread_id: String,
        error: String,
    },
    /// Thread store read/write failed (backend/serialization/archived).
    Storage {
        thread_id: String,
        error: String,
    },
}

fn storage_error(
    thread_id: &str,
) -> impl Fn(garyx_router::ThreadStoreError) -> ChatPreparationError {
    let thread_id = thread_id.to_owned();
    move |error| ChatPreparationError::Storage {
        thread_id: thread_id.clone(),
        error: error.to_string(),
    }
}

fn implicit_thread_creation_error(error: ThreadCreationError) -> ChatPreparationError {
    match error {
        ThreadCreationError::AgentBinding(error) => ChatPreparationError::InvalidRequest(
            StatusCode::BAD_REQUEST,
            Json(json!({
                "runId": "",
                "threadId": Value::Null,
                "response": Value::Null,
                "error": error.to_string(),
            })),
        ),
        ThreadCreationError::Other(error)
            if error.starts_with("unknown agent_id:")
                || error.starts_with("agent_id is not standalone:")
                || error.starts_with("workspace_mode=worktree") =>
        {
            ChatPreparationError::InvalidRequest(
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "runId": "",
                    "threadId": Value::Null,
                    "response": Value::Null,
                    "error": error,
                })),
            )
        }
        ThreadCreationError::Storage(error) | ThreadCreationError::Other(error) => {
            ChatPreparationError::Storage {
                thread_id: String::new(),
                error,
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedChatRequest {
    pub(crate) thread_id: String,
    pub(crate) effective_message: String,
    pub(crate) channel: String,
    pub(crate) account_id: String,
    pub(crate) from_id: String,
    pub(crate) workspace_path: Option<String>,
    pub(crate) provider_type: Option<ProviderType>,
    pub(crate) images: Vec<ImagePayload>,
    pub(crate) metadata: HashMap<String, Value>,
    pub(crate) managed_attachment_claims: Vec<PromptAttachmentClaim>,
    pub(crate) record_patch: ThreadRecordPatch,
    pub(crate) binding_plan: Option<ChannelBinding>,
    pub(crate) cache_changes_after_commit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatPreparationMode {
    Immediate,
    DeferredAdmission,
}

#[derive(Debug)]
struct ResolvedChatTarget {
    thread_id: String,
    channel: String,
    account_id: String,
    from_id: String,
    metadata: HashMap<String, Value>,
    thread_cache_maybe_stale: bool,
    binding_plan: Option<ChannelBinding>,
}

/// Pure routing result used before a correlated threadless chat obtains its
/// durable, thread-keyed admission identity. Existing owners are only read;
/// the create variant carries all inputs required by the atomic create path
/// and performs no record publication itself.
pub(crate) enum ThreadlessCorrelationTarget {
    Existing { thread_id: String },
    Create(ImplicitThreadCreatePlan),
}

pub(crate) struct ImplicitThreadCreatePlan {
    pub(crate) channel: String,
    pub(crate) account_id: String,
    pub(crate) from_id: String,
    pub(crate) binding: ChannelBinding,
    pub(crate) label: String,
    pub(crate) workspace_dir: Option<String>,
    pub(crate) workspace_mode: WorkspaceMode,
    pub(crate) agent_id: Option<String>,
    pub(crate) dispatch_metadata: HashMap<String, Value>,
}

pub(crate) async fn resolve_threadless_correlation_target(
    state: &Arc<AppState>,
    req: &ChatRequest,
) -> Result<ThreadlessCorrelationTarget, ChatPreparationError> {
    let config = state.config_snapshot();
    if let Some(bot) = req
        .bot
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Some((channel, account_id)) = bot.split_once(':') else {
            return Err(ChatPreparationError::InvalidRequest(
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "runId": "",
                    "threadId": Value::Null,
                    "response": Value::Null,
                    "error": "bot must be `channel:account_id`",
                })),
            ));
        };
        let endpoint =
            match crate::routes::resolve_main_endpoint_by_bot(state, channel, account_id).await {
                Ok(Some(endpoint)) => endpoint,
                Ok(None) => {
                    return Err(ChatPreparationError::InvalidRequest(
                        StatusCode::NOT_FOUND,
                        Json(json!({
                            "runId": "",
                            "threadId": Value::Null,
                            "response": Value::Null,
                            "error": format!("bot '{bot}' has no resolved main endpoint"),
                        })),
                    ));
                }
                Err(error) => {
                    return Err(ChatPreparationError::Storage {
                        thread_id: String::new(),
                        error: error.to_string(),
                    });
                }
            };
        if let Some(thread_id) = endpoint
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && state
                .threads
                .thread_store
                .get(thread_id)
                .await
                .map_err(storage_error(thread_id))?
                .is_some()
        {
            return Ok(ThreadlessCorrelationTarget::Existing {
                thread_id: thread_id.to_owned(),
            });
        }

        let mut binding = endpoint.to_binding();
        binding.last_inbound_at = Some(chrono::Utc::now().to_rfc3339());
        let workspace_mode = default_workspace_mode_for_channel_account(
            &config,
            &endpoint.channel,
            &endpoint.account_id,
        );
        return Ok(ThreadlessCorrelationTarget::Create(
            ImplicitThreadCreatePlan {
                channel: endpoint.channel.clone(),
                account_id: endpoint.account_id.clone(),
                from_id: endpoint.binding_key.clone(),
                label: binding.display_label.clone(),
                workspace_dir: normalize_workspace_dir(req.workspace_path.as_deref()).or_else(
                    || {
                        default_workspace_for_channel_account(
                            &config,
                            &endpoint.channel,
                            &endpoint.account_id,
                        )
                    },
                ),
                workspace_mode,
                agent_id: default_agent_for_channel_account(
                    &config,
                    &endpoint.channel,
                    &endpoint.account_id,
                ),
                dispatch_metadata: endpoint_chat_metadata(&endpoint),
                binding,
            },
        ));
    }

    let channel = "api";
    let account_id = req.account_id.trim();
    let from_id = req.from_id.trim();
    if account_id.is_empty() || from_id.is_empty() {
        return Err(ChatPreparationError::InvalidRequest(
            StatusCode::BAD_REQUEST,
            Json(json!({
                "runId": "",
                "threadId": Value::Null,
                "response": Value::Null,
                "error": "accountId and fromId must be non-empty",
            })),
        ));
    }
    let canonical_endpoint_key = endpoint_key(channel, account_id, from_id);
    let owner = state
        .ops
        .endpoint_binding_mutator
        .binding_for_endpoint(&canonical_endpoint_key)
        .await
        .map_err(|error| ChatPreparationError::Storage {
            thread_id: String::new(),
            error: error.to_string(),
        })?;
    if let Some(owner) = owner
        && state
            .threads
            .thread_store
            .get(&owner.thread_id)
            .await
            .map_err(storage_error(&owner.thread_id))?
            .is_some()
    {
        return Ok(ThreadlessCorrelationTarget::Existing {
            thread_id: owner.thread_id,
        });
    }

    let workspace_mode = default_workspace_mode_for_channel_account(&config, channel, account_id);
    let label = format!("{channel}/{account_id}/{from_id}");
    Ok(ThreadlessCorrelationTarget::Create(
        ImplicitThreadCreatePlan {
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            from_id: from_id.to_owned(),
            binding: ChannelBinding {
                channel: channel.to_owned(),
                account_id: account_id.to_owned(),
                binding_key: from_id.to_owned(),
                chat_id: from_id.to_owned(),
                delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
                delivery_target_id: from_id.to_owned(),
                display_label: label.clone(),
                last_inbound_at: Some(chrono::Utc::now().to_rfc3339()),
                last_delivery_at: None,
            },
            label,
            workspace_dir: normalize_workspace_dir(req.workspace_path.as_deref())
                .or_else(|| default_workspace_for_channel_account(&config, channel, account_id)),
            workspace_mode,
            agent_id: default_agent_for_channel_account(&config, channel, account_id),
            dispatch_metadata: HashMap::new(),
        },
    ))
}

fn thread_bound_agent_id(thread_data: &Value) -> Option<&str> {
    thread_data
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn thread_bound_provider_type(thread_data: &Value) -> Option<ProviderType> {
    let raw = thread_data.get("provider_type")?.clone();
    serde_json::from_value(raw.clone())
        .map_err(|e| tracing::debug!(raw = %raw, error = %e, "failed to parse thread-bound provider_type"))
        .ok()
}

async fn persist_thread_provider_type_if_missing(
    state: &Arc<AppState>,
    thread_id: &str,
    provider_type: &ProviderType,
) -> bool {
    let Some(mut thread_data) = state.threads.thread_store.get_logged(thread_id).await else {
        return false;
    };
    let observed = thread_data.clone();
    if thread_bound_provider_type(&thread_data).is_some() {
        return false;
    }
    let Some(obj) = thread_data.as_object_mut() else {
        return false;
    };
    obj.insert(
        "provider_type".to_owned(),
        serde_json::to_value(provider_type).unwrap_or(Value::Null),
    );
    obj.insert(
        "updated_at".to_owned(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    let patch =
        match ThreadRecordPatch::from_diff(&observed, &thread_data, PROVIDER_TYPE_PATCH_FIELDS) {
            Ok(patch) => patch,
            Err(error) => {
                tracing::warn!(thread_id, error = %error, "invalid provider-type patch");
                return false;
            }
        };
    match state.threads.thread_store.patch(thread_id, patch).await {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(thread_id, error = %error, "provider-type patch did not persist");
            false
        }
    }
}

pub(crate) async fn prepare_chat_request(
    state: &Arc<AppState>,
    req: ChatRequest,
) -> Result<PreparedChatRequest, ChatPreparationError> {
    prepare_chat_request_with_mode(state, req, ChatPreparationMode::Immediate).await
}

pub(crate) async fn prepare_durable_chat_request(
    state: &Arc<AppState>,
    req: ChatRequest,
) -> Result<PreparedChatRequest, ChatPreparationError> {
    prepare_chat_request_with_mode(state, req, ChatPreparationMode::DeferredAdmission).await
}

async fn prepare_chat_request_with_mode(
    state: &Arc<AppState>,
    mut req: ChatRequest,
    mode: ChatPreparationMode,
) -> Result<PreparedChatRequest, ChatPreparationError> {
    strip_server_owned_agent_metadata(&mut req.metadata);
    let config = state.config_snapshot();
    let resolved_message = resolve_chat_message(&config, &mut req);
    let ResolvedChatTarget {
        thread_id,
        channel,
        account_id,
        from_id,
        metadata,
        thread_cache_maybe_stale,
        binding_plan,
    } = resolve_chat_target(state, &req, mode).await?;
    let mut thread_cache_maybe_stale = thread_cache_maybe_stale;
    req.account_id = account_id;
    req.from_id = from_id;
    for (key, value) in metadata {
        req.metadata.insert(key, value);
    }
    if let Some(client_intent_id) = req
        .client_intent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        req.metadata.insert(
            "client_intent_id".to_owned(),
            Value::String(client_intent_id.to_owned()),
        );
    }
    let thread_data = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .map_err(storage_error(&thread_id))?
        .ok_or_else(|| {
            ChatPreparationError::InvalidRequest(
                StatusCode::NOT_FOUND,
                Json(json!({"threadId": thread_id, "error": "thread not found"})),
            )
        })?;
    let observed_thread_data = thread_data.clone();
    let mut desired_thread_data = thread_data;
    merge_thread_model_cells(&observed_thread_data, &mut req.metadata);
    let thread_provider_type = thread_bound_provider_type(&observed_thread_data);
    let agent_reference = match thread_bound_agent_id(&observed_thread_data).map(ToOwned::to_owned)
    {
        Some(agent_id) => {
            match resolve_agent_reference_from_stores(state.ops.custom_agents.as_ref(), &agent_id)
                .await
            {
                Ok(reference) => Some(reference),
                Err(error) => {
                    tracing::warn!(
                        thread_id = %thread_id,
                        agent_id = %agent_id,
                        error = %error,
                        "failed to resolve thread-bound agent before chat run"
                    );
                    None
                }
            }
        }
        None => None,
    };
    if let Some(reference) = agent_reference.as_ref() {
        for (key, value) in agent_runtime_metadata(reference) {
            if SERVER_OWNED_AGENT_METADATA_KEYS.contains(&key.as_str()) {
                req.metadata.insert(key, value);
            } else {
                req.metadata.entry(key).or_insert(value);
            }
        }
    }
    req.provider_type = thread_provider_type.clone().or_else(|| {
        agent_reference
            .as_ref()
            .map(|reference| reference.provider_type())
    });
    if let Some(provider_type) = req.provider_type.as_ref() {
        match mode {
            ChatPreparationMode::Immediate => {
                thread_cache_maybe_stale |=
                    persist_thread_provider_type_if_missing(state, &thread_id, provider_type).await;
            }
            ChatPreparationMode::DeferredAdmission if thread_provider_type.is_none() => {
                let object = desired_thread_data.as_object_mut().ok_or_else(|| {
                    ChatPreparationError::ThreadUpdateConflict {
                        thread_id: thread_id.clone(),
                        error: "thread payload is not an object".to_owned(),
                    }
                })?;
                object.insert(
                    "provider_type".to_owned(),
                    serde_json::to_value(provider_type).unwrap_or(Value::Null),
                );
                object.insert(
                    "updated_at".to_owned(),
                    Value::String(chrono::Utc::now().to_rfc3339()),
                );
            }
            ChatPreparationMode::DeferredAdmission => {}
        }
    }

    let mut staged_attachments = req.attachments.clone();
    staged_attachments.extend(stage_image_payloads_for_prompt(
        "garyx-gateway",
        &req.images,
    ));
    staged_attachments.extend(stage_file_payloads_for_prompt("garyx-gateway", &req.files));
    let scope = req
        .idempotency_scope
        .as_ref()
        .map(|scope| (scope.identity.as_str(), scope.epoch))
        .unwrap_or(("__legacy_api__", 0));
    let managed_attachment_claims = state
        .ops
        .prompt_attachments
        .prepare_claims(scope, &mut staged_attachments)
        .await
        .map_err(|error| match error {
            PromptAttachmentLifecycleError::Invalid(message) => {
                ChatPreparationError::InvalidRequest(
                    StatusCode::BAD_REQUEST,
                    Json(json!({"threadId": thread_id, "error": message})),
                )
            }
            PromptAttachmentLifecycleError::Conflict(message) => {
                ChatPreparationError::InvalidRequest(
                    StatusCode::CONFLICT,
                    Json(json!({"threadId": thread_id, "error": message})),
                )
            }
            PromptAttachmentLifecycleError::Storage(error) => ChatPreparationError::Storage {
                thread_id: thread_id.clone(),
                error,
            },
        })?;
    if !staged_attachments.is_empty() {
        req.metadata.insert(
            ATTACHMENTS_METADATA_KEY.to_owned(),
            attachments_to_metadata_value(&staged_attachments),
        );
    }

    match mode {
        ChatPreparationMode::Immediate => {
            if persist_thread_label_if_missing(state, &thread_id, &resolved_message)
                .await?
                .is_some()
            {
                thread_cache_maybe_stale = true;
            }
        }
        ChatPreparationMode::DeferredAdmission => {
            if let Some(next_label) = prompt_derived_thread_label(&resolved_message)
                && should_autoname_thread(&desired_thread_data)
            {
                let object = desired_thread_data.as_object_mut().ok_or_else(|| {
                    ChatPreparationError::ThreadUpdateConflict {
                        thread_id: thread_id.clone(),
                        error: "thread payload is not an object".to_owned(),
                    }
                })?;
                object.insert("label".to_owned(), Value::String(next_label));
                object.insert(
                    "thread_title_source".to_owned(),
                    Value::String(PROMPT_THREAD_TITLE_SOURCE.to_owned()),
                );
                object.insert(
                    "updated_at".to_owned(),
                    Value::String(chrono::Utc::now().to_rfc3339()),
                );
            }
        }
    }

    let runtime_workspace = match mode {
        ChatPreparationMode::Immediate => {
            resolve_runtime_workspace_dir(state, &thread_id, req.workspace_path.as_deref()).await?
        }
        ChatPreparationMode::DeferredAdmission => {
            resolve_runtime_workspace_dir_deferred(
                state,
                &thread_id,
                req.workspace_path.as_deref(),
                &mut desired_thread_data,
            )
            .await?
        }
    };
    thread_cache_maybe_stale |= runtime_workspace.thread_cache_maybe_stale;
    req.workspace_path = runtime_workspace.workspace_dir;

    record_api_thread_log(
        state,
        ThreadLogEvent::info(&thread_id, "api", "chat request prepared")
            .with_field("workspace_path", json!(req.workspace_path.clone()))
            .with_field("wait_for_response", json!(req.wait_for_response)),
    )
    .await;

    if thread_cache_maybe_stale {
        state.invalidate_gateway_sync_caches().await;
    }
    let runtime_thread_data = match mode {
        ChatPreparationMode::Immediate => state
            .threads
            .thread_store
            .get(&thread_id)
            .await
            .map_err(storage_error(&thread_id))?,
        ChatPreparationMode::DeferredAdmission => Some(desired_thread_data.clone()),
    };
    let runtime_context = build_runtime_context_metadata(
        &thread_id,
        runtime_thread_data.as_ref(),
        &req.metadata,
        &channel,
        &req.account_id,
        &req.from_id,
        req.workspace_path.as_deref(),
    );
    req.metadata
        .insert("runtime_context".to_owned(), runtime_context);

    let record_patch = match mode {
        ChatPreparationMode::Immediate => ThreadRecordPatch::default(),
        ChatPreparationMode::DeferredAdmission => ThreadRecordPatch::from_diff(
            &observed_thread_data,
            &desired_thread_data,
            &[
                "provider_type",
                "label",
                "thread_title_source",
                "workspace_dir",
                "updated_at",
            ],
        )
        .map_err(storage_error(&thread_id))?,
    };
    let cache_changes_after_commit = mode == ChatPreparationMode::DeferredAdmission
        && (!record_patch.is_empty() || binding_plan.is_some());

    Ok(PreparedChatRequest {
        thread_id,
        effective_message: resolved_message,
        channel,
        account_id: req.account_id,
        from_id: req.from_id,
        workspace_path: req.workspace_path,
        provider_type: req.provider_type,
        images: req.images,
        metadata: req.metadata,
        managed_attachment_claims,
        record_patch,
        binding_plan,
        cache_changes_after_commit,
    })
}

pub(crate) fn build_provider_run_metadata(
    config: &garyx_models::config::GaryxConfig,
    metadata: HashMap<String, Value>,
    channel: &str,
    account_id: &str,
    from_id: &str,
    run_id: &str,
) -> HashMap<String, Value> {
    let mut run_metadata = build_chat_metadata(metadata, channel, account_id, from_id, run_id);
    let gateway_auth_token = config.gateway.auth_token.trim();
    if !gateway_auth_token.is_empty() {
        run_metadata.insert(
            "garyx_mcp_auth_token".to_owned(),
            Value::String(gateway_auth_token.to_owned()),
        );
    }
    inject_managed_mcp_servers(&config.mcp_servers, &mut run_metadata);
    run_metadata
}

fn resolve_chat_message(
    config: &garyx_models::config::GaryxConfig,
    req: &mut ChatRequest,
) -> String {
    let command_text = req
        .metadata
        .get(NATIVE_COMMAND_TEXT_METADATA_KEY)
        .and_then(Value::as_str)
        .unwrap_or(&req.message)
        .to_owned();
    garyx_core::apply_custom_slash_command(config, &command_text, &req.message, &mut req.metadata)
        .unwrap_or_else(|| req.message.clone())
}

fn summarize_thread_label(value: &str, limit: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= limit {
        return normalized;
    }
    let mut truncated = String::new();
    for ch in normalized.chars().take(limit - 1) {
        truncated.push(ch);
    }
    format!("{}…", truncated.trim_end())
}

pub(crate) fn prompt_derived_thread_label(message: &str) -> Option<String> {
    let summary = summarize_thread_label(message, 40);
    (!summary.is_empty()).then_some(summary)
}

struct RuntimeWorkspaceResolution {
    workspace_dir: Option<String>,
    thread_cache_maybe_stale: bool,
}

async fn resolve_runtime_workspace_dir(
    state: &Arc<AppState>,
    thread_id: &str,
    requested_workspace_path: Option<&str>,
) -> Result<RuntimeWorkspaceResolution, ChatPreparationError> {
    let requested_workspace_dir = normalize_workspace_dir(requested_workspace_path);
    let existing_thread = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .map_err(storage_error(thread_id))?;
    let existing_workspace_dir = existing_thread.as_ref().and_then(workspace_dir_from_value);

    if let (Some(existing), Some(requested)) = (
        existing_workspace_dir.as_deref(),
        requested_workspace_dir.as_deref(),
    ) && existing != requested
    {
        return Err(ChatPreparationError::ThreadUpdateConflict {
            thread_id: thread_id.to_owned(),
            error: format!(
                "thread workspace_dir is immutable; create a new thread to use {requested}"
            ),
        });
    }

    if let Some(existing) = existing_workspace_dir {
        return Ok(RuntimeWorkspaceResolution {
            workspace_dir: Some(existing),
            thread_cache_maybe_stale: false,
        });
    }

    let workspace_dir = match requested_workspace_dir {
        Some(workspace_dir) => workspace_dir,
        None => ensure_implicit_thread_workspace_for_config(&state.config_snapshot(), thread_id)
            .await
            .map_err(|error| ChatPreparationError::ThreadUpdateConflict {
                thread_id: thread_id.to_owned(),
                error,
            })?,
    };

    let mut thread_cache_maybe_stale = false;
    if is_thread_key(thread_id) && existing_thread.is_some() {
        let updated = update_thread_record(
            &state.threads.thread_store,
            thread_id,
            None,
            Some(workspace_dir.clone()),
        )
        .await
        .map_err(|error| ChatPreparationError::ThreadUpdateConflict {
            thread_id: thread_id.to_owned(),
            error,
        })?;
        state
            .integration
            .bridge
            .set_thread_workspace_binding(thread_id, workspace_dir_from_value(&updated))
            .await;
        thread_cache_maybe_stale = true;
    }

    Ok(RuntimeWorkspaceResolution {
        workspace_dir: Some(workspace_dir),
        thread_cache_maybe_stale,
    })
}

async fn resolve_runtime_workspace_dir_deferred(
    state: &Arc<AppState>,
    thread_id: &str,
    requested_workspace_path: Option<&str>,
    desired_thread_data: &mut Value,
) -> Result<RuntimeWorkspaceResolution, ChatPreparationError> {
    let requested_workspace_dir = normalize_workspace_dir(requested_workspace_path);
    let existing_workspace_dir = workspace_dir_from_value(desired_thread_data);
    if let (Some(existing), Some(requested)) = (
        existing_workspace_dir.as_deref(),
        requested_workspace_dir.as_deref(),
    ) && existing != requested
    {
        return Err(ChatPreparationError::ThreadUpdateConflict {
            thread_id: thread_id.to_owned(),
            error: format!(
                "thread workspace_dir is immutable; create a new thread to use {requested}"
            ),
        });
    }
    if let Some(existing) = existing_workspace_dir {
        return Ok(RuntimeWorkspaceResolution {
            workspace_dir: Some(existing),
            thread_cache_maybe_stale: false,
        });
    }

    let workspace_dir = match requested_workspace_dir {
        Some(workspace_dir) => workspace_dir,
        None => ensure_implicit_thread_workspace_for_config(&state.config_snapshot(), thread_id)
            .await
            .map_err(|error| ChatPreparationError::ThreadUpdateConflict {
                thread_id: thread_id.to_owned(),
                error,
            })?,
    };
    let object = desired_thread_data.as_object_mut().ok_or_else(|| {
        ChatPreparationError::ThreadUpdateConflict {
            thread_id: thread_id.to_owned(),
            error: "thread payload is not an object".to_owned(),
        }
    })?;
    object.insert(
        "workspace_dir".to_owned(),
        Value::String(workspace_dir.clone()),
    );
    object.insert(
        "updated_at".to_owned(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    Ok(RuntimeWorkspaceResolution {
        workspace_dir: Some(workspace_dir),
        thread_cache_maybe_stale: false,
    })
}

fn should_autoname_thread(existing: &Value) -> bool {
    let Some(label) = existing.get("label").and_then(Value::as_str) else {
        return true;
    };
    let trimmed = label.trim();
    trimmed.is_empty()
        || trimmed == LEGACY_DEFAULT_THREAD_LABEL
        || api_route_placeholder_label(existing).as_deref() == Some(trimmed)
}

fn api_route_placeholder_label(existing: &Value) -> Option<String> {
    let channel = existing.get("channel").and_then(Value::as_str)?.trim();
    let account_id = existing.get("account_id").and_then(Value::as_str)?.trim();
    let from_id = existing.get("from_id").and_then(Value::as_str)?.trim();
    if channel != "api" || account_id.is_empty() || from_id.is_empty() {
        return None;
    }
    Some(format!("{channel}/{account_id}/{from_id}"))
}

async fn persist_thread_label_if_missing(
    state: &Arc<AppState>,
    thread_id: &str,
    effective_message: &str,
) -> Result<Option<String>, ChatPreparationError> {
    let Some(next_label) = prompt_derived_thread_label(effective_message) else {
        return Ok(None);
    };
    let Some(existing) = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .map_err(storage_error(thread_id))?
    else {
        return Ok(None);
    };
    if !should_autoname_thread(&existing) {
        return Ok(None);
    }

    let Some(obj) = existing.as_object() else {
        return Err(ChatPreparationError::ThreadUpdateConflict {
            thread_id: thread_id.to_owned(),
            error: "thread payload is not an object".to_owned(),
        });
    };
    let mut next = Value::Object(obj.clone());
    if let Some(next_obj) = next.as_object_mut() {
        next_obj.insert("label".to_owned(), Value::String(next_label.clone()));
        next_obj.insert(
            "thread_title_source".to_owned(),
            Value::String(PROMPT_THREAD_TITLE_SOURCE.to_owned()),
        );
        next_obj.insert(
            "updated_at".to_owned(),
            Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }
    let patch = ThreadRecordPatch::from_diff(
        &existing,
        &next,
        &["label", "thread_title_source", "updated_at"],
    )
    .map_err(storage_error(thread_id))?;
    state
        .threads
        .thread_store
        .patch(thread_id, patch)
        .await
        .map_err(storage_error(thread_id))?;
    Ok(Some(next_label))
}

async fn resolve_chat_target(
    state: &Arc<AppState>,
    req: &ChatRequest,
    mode: ChatPreparationMode,
) -> Result<ResolvedChatTarget, ChatPreparationError> {
    if let Some(key) = req.thread_id.as_deref() {
        let trimmed = key.trim();
        if req
            .bot
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            return Err(ChatPreparationError::InvalidRequest(
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "runId": "",
                    "threadId": trimmed,
                    "response": Value::Null,
                    "error": "threadId and bot are mutually exclusive",
                })),
            ));
        }
        if trimmed.is_empty() || !is_thread_key(trimmed) {
            return Err(ChatPreparationError::InvalidRequest(
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "runId": "",
                    "threadId": trimmed,
                    "response": Value::Null,
                    "error": "threadId must be a canonical thread id",
                })),
            ));
        }
        let archived_thread_id = trimmed.to_owned();
        match state
            .ops
            .garyx_db
            .run_blocking(move |db| db.is_thread_archived(&archived_thread_id))
            .await
        {
            Ok(true) => {
                return Err(ChatPreparationError::InvalidRequest(
                    StatusCode::GONE,
                    Json(json!({
                        "runId": "",
                        "threadId": trimmed,
                        "response": Value::Null,
                        "error": "thread is archived",
                    })),
                ));
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(thread_id = trimmed, error = %error, "failed to check archived thread before chat start");
            }
        }
        match state.threads.thread_store.get(trimmed).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(ChatPreparationError::InvalidRequest(
                    StatusCode::NOT_FOUND,
                    Json(json!({
                        "runId": "",
                        "threadId": trimmed,
                        "response": Value::Null,
                        "error": "thread not found",
                    })),
                ));
            }
            Err(error) => return Err(storage_error(trimmed)(error)),
        }
        let (thread_cache_maybe_stale, binding_plan) = match mode {
            ChatPreparationMode::Immediate => (
                persist_explicit_api_thread_binding(
                    state,
                    trimmed,
                    req.account_id.as_str(),
                    req.from_id.as_str(),
                )
                .await?,
                None,
            ),
            ChatPreparationMode::DeferredAdmission => (
                false,
                explicit_api_thread_binding(req.account_id.as_str(), req.from_id.as_str()),
            ),
        };
        return Ok(ResolvedChatTarget {
            thread_id: trimmed.to_owned(),
            channel: "api".to_owned(),
            account_id: req.account_id.clone(),
            from_id: req.from_id.clone(),
            metadata: HashMap::new(),
            thread_cache_maybe_stale,
            binding_plan,
        });
    }

    if let Some(bot) = req
        .bot
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let Some((channel, account_id)) = bot.split_once(':') else {
            return Err(ChatPreparationError::InvalidRequest(
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "runId": "",
                    "threadId": Value::Null,
                    "response": Value::Null,
                    "error": "bot must be `channel:account_id`",
                })),
            ));
        };
        let endpoint =
            match crate::routes::resolve_main_endpoint_by_bot(state, channel, account_id).await {
                Ok(Some(endpoint)) => endpoint,
                Ok(None) => {
                    return Err(ChatPreparationError::InvalidRequest(
                        StatusCode::NOT_FOUND,
                        Json(json!({
                            "runId": "",
                            "threadId": Value::Null,
                            "response": Value::Null,
                            "error": format!("bot '{bot}' has no resolved main endpoint"),
                        })),
                    ));
                }
                Err(error) => {
                    return Err(ChatPreparationError::InvalidRequest(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "runId": "",
                            "threadId": Value::Null,
                            "response": Value::Null,
                            "error": format!("thread store error: {error}"),
                        })),
                    ));
                }
            };
        if let Some(thread_id) = endpoint
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match state.threads.thread_store.get(thread_id).await {
                Ok(Some(_)) => {
                    let metadata = endpoint_chat_metadata(&endpoint);
                    return Ok(ResolvedChatTarget {
                        thread_id: thread_id.to_owned(),
                        channel: endpoint.channel,
                        account_id: endpoint.account_id,
                        from_id: endpoint.binding_key,
                        metadata,
                        thread_cache_maybe_stale: false,
                        binding_plan: None,
                    });
                }
                Ok(None) => {
                    tracing::debug!(
                        thread_id,
                        bot,
                        "bot endpoint snapshot was stale; resolving as first contact"
                    );
                }
                Err(error) => return Err(storage_error(thread_id)(error)),
            }
        }

        if mode == ChatPreparationMode::DeferredAdmission {
            return Err(ChatPreparationError::ThreadUpdateConflict {
                thread_id: endpoint.thread_id.unwrap_or_default(),
                error: "bot endpoint owner changed before durable admission".to_owned(),
            });
        }
        let mut metadata = req.metadata.clone();
        let endpoint_metadata = endpoint_chat_metadata(&endpoint);
        metadata.extend(endpoint_metadata.clone());
        let thread_id = {
            let mut router = state.threads.router.lock().await;
            router
                .resolve_or_create_inbound_thread_typed(
                    &endpoint.channel,
                    &endpoint.account_id,
                    &endpoint.binding_key,
                    &metadata,
                )
                .await
                .map_err(implicit_thread_creation_error)?
        };
        return Ok(ResolvedChatTarget {
            thread_id,
            channel: endpoint.channel,
            account_id: endpoint.account_id,
            from_id: endpoint.binding_key,
            metadata: endpoint_metadata,
            thread_cache_maybe_stale: true,
            binding_plan: None,
        });
    }

    if mode == ChatPreparationMode::DeferredAdmission {
        return Err(ChatPreparationError::ThreadUpdateConflict {
            thread_id: String::new(),
            error: "API endpoint owner changed before durable admission".to_owned(),
        });
    }
    let thread_id = {
        let mut router = state.threads.router.lock().await;
        router
            .resolve_or_create_inbound_thread_typed(
                "api",
                &req.account_id,
                &req.from_id,
                &req.metadata,
            )
            .await
            .map_err(implicit_thread_creation_error)?
    };
    Ok(ResolvedChatTarget {
        thread_id,
        channel: "api".to_owned(),
        account_id: req.account_id.clone(),
        from_id: req.from_id.clone(),
        metadata: HashMap::new(),
        thread_cache_maybe_stale: true,
        binding_plan: None,
    })
}

fn explicit_api_thread_binding(account_id: &str, from_id: &str) -> Option<ChannelBinding> {
    let account_id = account_id.trim();
    let from_id = from_id.trim();
    if account_id.is_empty() || from_id.is_empty() {
        return None;
    }
    Some(ChannelBinding {
        channel: "api".to_owned(),
        account_id: account_id.to_owned(),
        binding_key: from_id.to_owned(),
        chat_id: from_id.to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        delivery_target_id: from_id.to_owned(),
        display_label: format!("api/{account_id}/{from_id}"),
        last_inbound_at: Some(chrono::Utc::now().to_rfc3339()),
        last_delivery_at: None,
    })
}

async fn persist_explicit_api_thread_binding(
    state: &Arc<AppState>,
    thread_id: &str,
    account_id: &str,
    from_id: &str,
) -> Result<bool, ChatPreparationError> {
    let Some(binding) = explicit_api_thread_binding(account_id, from_id) else {
        return Ok(false);
    };
    if state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .map_err(storage_error(thread_id))?
        .is_none()
    {
        return Ok(false);
    }

    state
        .threads
        .router
        .lock()
        .await
        .bind_endpoint_runtime(thread_id, binding)
        .await
        .map_err(|error| ChatPreparationError::ThreadUpdateConflict {
            thread_id: thread_id.to_owned(),
            error: error.to_string(),
        })?;
    Ok(true)
}

fn endpoint_chat_metadata(
    endpoint: &crate::routes::ResolvedMainEndpoint,
) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();
    metadata.insert(
        "chat_id".to_owned(),
        Value::String(endpoint.chat_id.clone()),
    );
    metadata.insert(
        "display_label".to_owned(),
        Value::String(endpoint.display_label.clone()),
    );
    metadata.insert(
        "thread_binding_key".to_owned(),
        Value::String(endpoint.binding_key.clone()),
    );
    metadata.insert(
        "delivery_target_type".to_owned(),
        Value::String(endpoint.delivery_target_type.clone()),
    );
    metadata.insert(
        "delivery_target_id".to_owned(),
        Value::String(endpoint.delivery_target_id.clone()),
    );
    metadata.insert(
        "delivery_thread_id".to_owned(),
        endpoint
            .delivery_thread_id
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    metadata
}

fn build_chat_metadata(
    mut metadata: HashMap<String, Value>,
    channel: &str,
    account_id: &str,
    from_id: &str,
    run_id: &str,
) -> HashMap<String, Value> {
    metadata.insert("channel".to_owned(), Value::String(channel.to_owned()));
    metadata.insert(
        "account_id".to_owned(),
        Value::String(account_id.to_owned()),
    );
    metadata.insert("from_id".to_owned(), Value::String(from_id.to_owned()));
    metadata.insert("is_group".to_owned(), Value::Bool(false));
    metadata.insert("client_run_id".to_owned(), Value::String(run_id.to_owned()));
    metadata
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod patch_allowlist_contract {
    //! The patch-field allowlist is a reviewed durable contract for an
    //! existing-record writer (retired source-scan guard, now pinned by
    //! direct import): growing it means auditing what a concurrent
    //! whole-record write could clobber.

    /// Behavioral half of the writer contract: the provider-type writer must
    /// stay a field-scoped patch within its allowlist and never regress to a
    /// whole-record `set` (which would clobber concurrently written fields).
    #[tokio::test]
    async fn provider_type_writer_patches_within_allowlist_never_sets() {
        use crate::composition::app_bootstrap::AppStateBuilder;
        use garyx_models::config::GaryxConfig;
        use garyx_router::test_seams::PatchSpyThreadStore;
        use serde_json::json;
        use std::sync::Arc;

        let spy = PatchSpyThreadStore::seeded(
            "thread::provider-type-writer",
            json!({"concurrent_marker": "survives"}),
        );
        let state = AppStateBuilder::new(GaryxConfig::default())
            .with_thread_store(spy.clone() as Arc<dyn garyx_router::ThreadStore>)
            .build();
        assert!(
            super::persist_thread_provider_type_if_missing(
                &state,
                "thread::provider-type-writer",
                &garyx_models::provider::ProviderType::ClaudeCode,
            )
            .await
        );

        assert!(
            spy.set_thread_ids().is_empty(),
            "provider-type writer must never issue a whole-record set"
        );
        let patches = spy.patched_field_sets();
        assert!(!patches.is_empty(), "writer must persist via patch");
        for fields in &patches {
            for field in fields {
                assert!(
                    super::PROVIDER_TYPE_PATCH_FIELDS.contains(&field.as_str()),
                    "patched field {field} outside the reviewed allowlist"
                );
            }
        }
        let record = spy.record("thread::provider-type-writer").expect("record");
        assert_eq!(record["concurrent_marker"], json!("survives"));
        assert!(record["provider_type"].is_string());
    }

    #[test]
    fn provider_type_patch_allowlist_is_the_reviewed_contract() {
        assert_eq!(
            super::PROVIDER_TYPE_PATCH_FIELDS,
            &["provider_type", "updated_at"]
        );
    }
}
