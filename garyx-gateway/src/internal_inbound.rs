use std::collections::HashMap;
use std::sync::Arc;

use garyx_models::MessageLifecycleStatus;
use garyx_models::provider::ProviderType;
use garyx_models::routing::DeliveryContext;
use garyx_router::{ChannelBinding, MessageRouter, ThreadMessageRequest};
use serde_json::Value;

use crate::chat_delivery::build_bound_response_callback;
use crate::server::AppState;

#[derive(Default)]
pub(crate) struct InternalDispatchOptions {
    pub(crate) extra_metadata: HashMap<String, Value>,
    pub(crate) file_paths: Vec<String>,
    pub(crate) requested_provider: Option<ProviderType>,
}

fn thread_string_field(thread: &Value, key: &str) -> Option<String> {
    thread
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn delivery_streaming_target(
    target_thread_id: &str,
    delivery: &DeliveryContext,
) -> garyx_channels::StreamingDispatchTarget {
    garyx_channels::StreamingDispatchTarget {
        target_thread_id: target_thread_id.to_owned(),
        channel: delivery.channel.clone(),
        account_id: delivery.account_id.clone(),
        chat_id: delivery.chat_id.clone(),
        delivery_target_type: delivery.delivery_target_type.clone(),
        delivery_target_id: delivery.delivery_target_id.clone(),
        thread_id: delivery.thread_id.clone(),
    }
}

fn missing_thread_binding(thread: &Value, delivery: &DeliveryContext) -> Option<ChannelBinding> {
    let bindings = thread.get("channel_bindings")?.as_array()?;
    if !bindings.is_empty() {
        return None;
    }

    let channel = thread_string_field(thread, "channel")?;
    let account_id = thread_string_field(thread, "account_id")?;
    let is_group = thread
        .get("is_group")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let binding_key = if is_group {
        delivery.thread_id.clone()?
    } else {
        thread_string_field(thread, "thread_binding_key")
            .or_else(|| thread_string_field(thread, "from_id"))
            .or_else(|| Some(delivery.user_id.clone()))
            .or_else(|| Some(delivery.chat_id.clone()))?
    };
    let display_label = thread
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&binding_key)
        .to_owned();

    Some(ChannelBinding {
        channel,
        account_id,
        binding_key,
        chat_id: delivery.chat_id.clone(),
        delivery_target_type: delivery.delivery_target_type.clone(),
        delivery_target_id: delivery.delivery_target_id.clone(),
        display_label,
        last_inbound_at: None,
        last_delivery_at: None,
    })
}

pub(crate) async fn dispatch_internal_message_to_thread(
    state: &Arc<AppState>,
    target_thread_id: &str,
    run_id: &str,
    message: &str,
    options: InternalDispatchOptions,
) -> Result<(), String> {
    let InternalDispatchOptions {
        mut extra_metadata,
        file_paths,
        requested_provider,
    } = options;
    let thread_data = state
        .threads
        .thread_store
        .get(target_thread_id)
        .await
        .ok_or_else(|| format!("thread not found: {target_thread_id}"))?;
    let delivery_context = MessageRouter::resolve_delivery_target_from_store(
        state.threads.thread_store.clone(),
        target_thread_id,
    )
    .await
    .map(|(_, ctx)| ctx);

    if let Some(binding) = delivery_context
        .as_ref()
        .and_then(|delivery| missing_thread_binding(&thread_data, delivery))
    {
        let mut router = state.threads.router.lock().await;
        let _ = router
            .bind_endpoint_runtime(target_thread_id, binding)
            .await;
    }

    let channel = thread_string_field(&thread_data, "channel")
        .or_else(|| delivery_context.as_ref().map(|ctx| ctx.channel.clone()))
        .unwrap_or_else(|| "api".to_owned());
    let account_id = thread_string_field(&thread_data, "account_id")
        .or_else(|| delivery_context.as_ref().map(|ctx| ctx.account_id.clone()))
        .unwrap_or_else(|| "main".to_owned());
    let from_id = thread_string_field(&thread_data, "from_id")
        .or_else(|| delivery_context.as_ref().map(|ctx| ctx.user_id.clone()))
        .or_else(|| delivery_context.as_ref().map(|ctx| ctx.chat_id.clone()))
        .unwrap_or_else(|| "loop".to_owned());

    extra_metadata.insert("internal_dispatch".to_owned(), Value::Bool(true));
    if let Some(requested_provider) = requested_provider.as_ref() {
        let provider_value = match requested_provider {
            ProviderType::ClaudeCode => "claude_code",
            ProviderType::CodexAppServer => "codex_app_server",
            ProviderType::GeminiCli => "gemini_cli",
            ProviderType::AgentTeam => "agent_team",
        };
        extra_metadata.insert(
            "requested_provider_type".to_owned(),
            Value::String(provider_value.to_owned()),
        );
    }
    let response_callback = build_bound_response_callback(
        state,
        target_thread_id,
        run_id,
        delivery_context
            .as_ref()
            .map(|delivery| delivery_streaming_target(target_thread_id, delivery)),
    )
    .await;

    crate::runtime_diagnostics::record_message_ledger_event(
        state,
        MessageLifecycleStatus::RunStarted,
        crate::runtime_diagnostics::RuntimeDiagnosticContext {
            thread_id: Some(target_thread_id.to_owned()),
            run_id: Some(run_id.to_owned()),
            channel: Some(channel.clone()),
            account_id: Some(account_id.clone()),
            from_id: Some(from_id.clone()),
            chat_id: extra_metadata
                .get("chat_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| delivery_context.as_ref().map(|ctx| ctx.chat_id.clone())),
            text_excerpt: Some(message.chars().take(200).collect()),
            metadata: Some(serde_json::json!({
                "source": "internal_inbound",
            })),
            ..Default::default()
        },
    )
    .await;

    let result = {
        let mut router = state.threads.router.lock().await;
        router
            .dispatch_message_to_thread(
                target_thread_id,
                ThreadMessageRequest {
                    message: message.to_owned(),
                    run_id: run_id.to_owned(),
                    extra_metadata,
                    images: Vec::new(),
                    file_paths,
                },
                state.integration.bridge.as_ref(),
                response_callback,
            )
            .await?
    };

    if result.local_reply.is_some() {
        return Err("internal thread dispatch unexpectedly handled locally".to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests;
