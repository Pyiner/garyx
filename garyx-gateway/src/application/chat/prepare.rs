use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, ImagePayload, ProviderType, attachments_to_metadata_value,
    stage_file_payloads_for_prompt, stage_image_payloads_for_prompt,
};
use garyx_models::thread_logs::ThreadLogEvent;
use garyx_router::{
    NATIVE_COMMAND_TEXT_METADATA_KEY, build_runtime_context_metadata, is_thread_key,
    update_thread_record, workspace_dir_from_value,
};
use serde_json::{Value, json};

use crate::agent_identity::{
    agent_runtime_metadata, build_group_transcript_snapshot, resolve_agent_reference_from_stores,
};
use crate::application::chat::contracts::ChatRequest;
use crate::chat_shared::record_api_thread_log;
use crate::managed_mcp_metadata::inject_managed_mcp_servers;
use crate::server::AppState;

const LEGACY_DEFAULT_THREAD_LABEL: &str = "Fresh Thread";
const PROMPT_THREAD_TITLE_SOURCE: &str = "garyx_prompt";

#[derive(Debug)]
pub(crate) enum ChatPreparationError {
    InvalidRequest(StatusCode, Json<Value>),
    ThreadUpdateConflict { thread_id: String, error: String },
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
    pub(crate) provider_metadata: HashMap<String, Value>,
}

#[derive(Debug)]
struct ResolvedChatTarget {
    thread_id: String,
    channel: String,
    account_id: String,
    from_id: String,
    metadata: HashMap<String, Value>,
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
) {
    let Some(mut thread_data) = state.threads.thread_store.get(thread_id).await else {
        return;
    };
    if thread_bound_provider_type(&thread_data).is_some() {
        return;
    }
    let Some(obj) = thread_data.as_object_mut() else {
        return;
    };
    obj.insert(
        "provider_type".to_owned(),
        serde_json::to_value(provider_type).unwrap_or(Value::Null),
    );
    obj.insert(
        "updated_at".to_owned(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    state.threads.thread_store.set(thread_id, thread_data).await;
}

pub(crate) async fn prepare_chat_request(
    state: &Arc<AppState>,
    mut req: ChatRequest,
) -> Result<PreparedChatRequest, ChatPreparationError> {
    let config = state.config_snapshot();
    let resolved_message = resolve_chat_message(&config, &mut req);
    let ResolvedChatTarget {
        thread_id,
        channel,
        account_id,
        from_id,
        metadata,
    } = resolve_chat_target(state, &req).await?;
    req.account_id = account_id;
    req.from_id = from_id;
    for (key, value) in metadata {
        req.metadata.insert(key, value);
    }
    let thread_data = state.threads.thread_store.get(&thread_id).await;
    let thread_provider_type = thread_data.as_ref().and_then(thread_bound_provider_type);
    let agent_reference = match thread_data
        .as_ref()
        .and_then(thread_bound_agent_id)
        .map(ToOwned::to_owned)
    {
        Some(agent_id) => {
            match resolve_agent_reference_from_stores(
                state.ops.custom_agents.as_ref(),
                state.ops.agent_teams.as_ref(),
                &agent_id,
            )
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
            req.metadata.entry(key).or_insert(value);
        }
        if reference.team().is_some()
            && let Some(thread_data) = thread_data.as_ref()
        {
            req.metadata
                .entry("group_transcript_snapshot".to_owned())
                .or_insert_with(|| build_group_transcript_snapshot(thread_data));
        }
    }
    req.provider_type = thread_provider_type.or_else(|| {
        agent_reference
            .as_ref()
            .map(|reference| reference.provider_type())
    });
    if let Some(provider_type) = req.provider_type.as_ref() {
        persist_thread_provider_type_if_missing(state, &thread_id, provider_type).await;
    }

    let mut staged_attachments = req.attachments.clone();
    staged_attachments.extend(stage_image_payloads_for_prompt(
        "garyx-gateway",
        &req.images,
    ));
    staged_attachments.extend(stage_file_payloads_for_prompt("garyx-gateway", &req.files));
    if !staged_attachments.is_empty() {
        req.metadata.insert(
            ATTACHMENTS_METADATA_KEY.to_owned(),
            attachments_to_metadata_value(&staged_attachments),
        );
    }

    persist_thread_label_if_missing(state, &thread_id, &resolved_message).await?;

    record_api_thread_log(
        state,
        ThreadLogEvent::info(&thread_id, "api", "chat request prepared")
            .with_field("workspace_path", json!(req.workspace_path.clone()))
            .with_field("wait_for_response", json!(req.wait_for_response)),
    )
    .await;

    persist_thread_workspace_if_missing(state, &thread_id, req.workspace_path.as_deref()).await?;
    let runtime_thread_data = state.threads.thread_store.get(&thread_id).await;
    let runtime_workspace_dir = req
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            runtime_thread_data
                .as_ref()
                .and_then(workspace_dir_from_value)
        });
    let runtime_context = build_runtime_context_metadata(
        &thread_id,
        runtime_thread_data.as_ref(),
        &req.metadata,
        &channel,
        &req.account_id,
        &req.from_id,
        runtime_workspace_dir.as_deref(),
    );
    req.metadata
        .insert("runtime_context".to_owned(), runtime_context);

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
        provider_metadata: req.provider_metadata,
    })
}

pub(crate) fn build_provider_run_metadata(
    config: &garyx_models::config::GaryxConfig,
    metadata: HashMap<String, Value>,
    provider_metadata: HashMap<String, Value>,
    channel: &str,
    account_id: &str,
    from_id: &str,
    run_id: &str,
) -> HashMap<String, Value> {
    let mut run_metadata = build_chat_metadata(metadata, channel, account_id, from_id, run_id);
    run_metadata.extend(provider_metadata);
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

fn prompt_derived_thread_label(message: &str) -> Option<String> {
    let summary = summarize_thread_label(message, 40);
    (!summary.is_empty()).then_some(summary)
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
) -> Result<(), ChatPreparationError> {
    let Some(next_label) = prompt_derived_thread_label(effective_message) else {
        return Ok(());
    };
    let Some(existing) = state.threads.thread_store.get(thread_id).await else {
        return Ok(());
    };
    if !should_autoname_thread(&existing) {
        return Ok(());
    }

    let Some(obj) = existing.as_object() else {
        return Err(ChatPreparationError::ThreadUpdateConflict {
            thread_id: thread_id.to_owned(),
            error: "thread payload is not an object".to_owned(),
        });
    };
    let mut next = Value::Object(obj.clone());
    if let Some(next_obj) = next.as_object_mut() {
        next_obj.insert("label".to_owned(), Value::String(next_label));
        next_obj.insert(
            "thread_title_source".to_owned(),
            Value::String(PROMPT_THREAD_TITLE_SOURCE.to_owned()),
        );
        next_obj.insert(
            "updated_at".to_owned(),
            Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }
    state.threads.thread_store.set(thread_id, next).await;
    Ok(())
}

async fn persist_thread_workspace_if_missing(
    state: &Arc<AppState>,
    thread_id: &str,
    workspace_path: Option<&str>,
) -> Result<(), ChatPreparationError> {
    let Some(workspace_path) = workspace_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if !is_thread_key(thread_id) {
        return Ok(());
    }

    let Some(existing) = state.threads.thread_store.get(thread_id).await else {
        return Ok(());
    };
    if workspace_dir_from_value(&existing).is_some() {
        return Ok(());
    }

    let updated = update_thread_record(
        &state.threads.thread_store,
        thread_id,
        None,
        Some(workspace_path.to_owned()),
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
    Ok(())
}

async fn resolve_chat_target(
    state: &Arc<AppState>,
    req: &ChatRequest,
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
        return Ok(ResolvedChatTarget {
            thread_id: trimmed.to_owned(),
            channel: "api".to_owned(),
            account_id: req.account_id.clone(),
            from_id: req.from_id.clone(),
            metadata: HashMap::new(),
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
        let Some(endpoint) =
            crate::routes::resolve_main_endpoint_by_bot(state, channel, account_id).await
        else {
            return Err(ChatPreparationError::InvalidRequest(
                StatusCode::NOT_FOUND,
                Json(json!({
                    "runId": "",
                    "threadId": Value::Null,
                    "response": Value::Null,
                    "error": format!("bot '{bot}' has no resolved main endpoint"),
                })),
            ));
        };
        if let Some(thread_id) = endpoint
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let metadata = endpoint_chat_metadata(&endpoint);
            return Ok(ResolvedChatTarget {
                thread_id: thread_id.to_owned(),
                channel: endpoint.channel,
                account_id: endpoint.account_id,
                from_id: endpoint.binding_key,
                metadata,
            });
        }

        let mut metadata = req.metadata.clone();
        let endpoint_metadata = endpoint_chat_metadata(&endpoint);
        metadata.extend(endpoint_metadata.clone());
        let mut router = state.threads.router.lock().await;
        let thread_id = router
            .resolve_or_create_inbound_thread(
                &endpoint.channel,
                &endpoint.account_id,
                &endpoint.binding_key,
                &metadata,
            )
            .await;
        return Ok(ResolvedChatTarget {
            thread_id,
            channel: endpoint.channel,
            account_id: endpoint.account_id,
            from_id: endpoint.binding_key,
            metadata: endpoint_metadata,
        });
    }

    let mut router = state.threads.router.lock().await;
    let thread_id = router
        .resolve_or_create_inbound_thread("api", &req.account_id, &req.from_id, &req.metadata)
        .await;
    Ok(ResolvedChatTarget {
        thread_id,
        channel: "api".to_owned(),
        account_id: req.account_id.clone(),
        from_id: req.from_id.clone(),
        metadata: HashMap::new(),
    })
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
