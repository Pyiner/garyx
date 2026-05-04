use std::collections::HashMap;
use std::sync::Arc;

use garyx_core::apply_custom_slash_command;
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, AgentRunRequest, ProviderType, attachments_from_metadata,
    attachments_to_metadata_value, file_attachments_from_paths, stage_image_payloads_for_prompt,
};
use garyx_models::thread_logs::{ThreadLogEvent, is_canonical_thread_id};
use serde_json::{Value, json};
use tracing::info;

use super::super::*;
use super::planning::{DispatchContext, DispatchPlan};

fn message_actor_label(object: &serde_json::Map<String, Value>) -> Option<String> {
    let metadata = object.get("metadata").and_then(Value::as_object);
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();

    let agent_display_name = metadata
        .and_then(|fields| fields.get("agent_display_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let agent_id = metadata
        .and_then(|fields| fields.get("agent_id"))
        .and_then(Value::as_str)
        .or_else(|| object.get("agent_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let from_id = metadata
        .and_then(|fields| fields.get("from_id"))
        .and_then(Value::as_str)
        .or_else(|| object.get("from_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let internal_dispatch = metadata
        .and_then(|fields| fields.get("internal_dispatch"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    match role {
        "assistant" => agent_id.or(agent_display_name),
        "user" if internal_dispatch => agent_id.or(agent_display_name).or(from_id),
        "user" => Some("user".to_owned()),
        _ => agent_id.or(agent_display_name).or(from_id),
    }
}

fn build_group_transcript_snapshot(thread_data: &Value) -> Value {
    let Some(messages) = thread_data.get("messages").and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    let mut entries = Vec::with_capacity(messages.len());
    for message in messages {
        let Some(object) = message.as_object() else {
            continue;
        };
        let agent_id = message_actor_label(object).unwrap_or_default();
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| object.get("content").and_then(Value::as_str))
            .unwrap_or("");
        if agent_id.is_empty() && text.is_empty() {
            continue;
        }
        let at = object
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("");
        entries.push(json!({
            "agent_id": agent_id,
            "text": text,
            "at": at,
        }));
    }
    Value::Array(entries)
}

impl MessageRouter {
    fn apply_custom_command_transform_fields(
        &mut self,
        channel: &str,
        account_id: &str,
        from_id: &str,
        message: &mut String,
        extra_metadata: &mut HashMap<String, Value>,
        dispatch_target: Option<&str>,
    ) {
        let custom_command_text = extra_metadata
            .get(NATIVE_COMMAND_TEXT_METADATA_KEY)
            .and_then(Value::as_str)
            .unwrap_or(message.as_str())
            .to_owned();
        if let Some(transformed) =
            apply_custom_slash_command(&self.config, &custom_command_text, message, extra_metadata)
        {
            *message = transformed;
            if let Some(thread_id) = dispatch_target {
                info!(
                    channel,
                    account_id,
                    from_id,
                    command = %custom_command_text,
                    thread_id,
                    "custom slash command transformed before direct thread dispatch"
                );
            } else {
                info!(
                    channel,
                    account_id,
                    from_id,
                    command = %custom_command_text,
                    "custom slash command transformed before dispatch"
                );
            }
        }
    }

    pub(super) fn apply_custom_command_transform(
        &mut self,
        request: &mut InboundRequest,
        dispatch_target: Option<&str>,
    ) {
        self.apply_custom_command_transform_fields(
            &request.channel,
            &request.account_id,
            &request.from_id,
            &mut request.message,
            &mut request.extra_metadata,
            dispatch_target,
        );
    }

    pub(super) fn apply_custom_thread_message_transform(
        &mut self,
        context: &mut DispatchContext,
        dispatch_target: Option<&str>,
    ) {
        self.apply_custom_command_transform_fields(
            &context.channel,
            &context.account_id,
            &context.from_id,
            &mut context.message,
            &mut context.extra_metadata,
            dispatch_target,
        );
    }

    pub(super) async fn execute_dispatch_plan(
        &mut self,
        plan: DispatchPlan,
        dispatcher: &dyn AgentDispatcher,
        response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
        dispatch_message: &'static str,
    ) -> Result<InboundResult, String> {
        let DispatchPlan {
            context,
            thread_id,
            message_metadata,
            mut dispatch_metadata,
            delivery_context,
            images,
            file_paths,
        } = plan;

        let mut prompt_attachments = attachments_from_metadata(&dispatch_metadata);
        prompt_attachments.extend(file_attachments_from_paths(&file_paths));
        prompt_attachments.extend(stage_image_payloads_for_prompt(
            "garyx-router",
            images.as_deref().unwrap_or_default(),
        ));
        if !prompt_attachments.is_empty() {
            dispatch_metadata.insert(
                ATTACHMENTS_METADATA_KEY.to_owned(),
                attachments_to_metadata_value(&prompt_attachments),
            );
        }

        let thread_record = self.threads.get(&thread_id).await;
        let thread_workspace_dir = thread_record
            .as_ref()
            .and_then(crate::workspace_dir_from_value);
        if let Some(thread_record) = thread_record.as_ref() {
            for (key, value) in crate::thread_metadata_from_value(thread_record) {
                dispatch_metadata.entry(key).or_insert(value);
            }
            if dispatch_metadata.contains_key("agent_team_id") {
                dispatch_metadata
                    .entry("group_transcript_snapshot".to_owned())
                    .or_insert_with(|| build_group_transcript_snapshot(thread_record));
            }
        }

        dispatch_metadata.insert(
            "resolved_thread_id".to_owned(),
            Value::String(thread_id.clone()),
        );
        if let Some(workspace_dir) = thread_workspace_dir.as_ref() {
            dispatch_metadata.insert(
                "workspace_dir".to_owned(),
                Value::String(workspace_dir.clone()),
            );
        }
        dispatch_metadata.insert(
            "runtime_context".to_owned(),
            crate::build_runtime_context_metadata(
                &thread_id,
                thread_record.as_ref(),
                &dispatch_metadata,
                &context.channel,
                &context.account_id,
                &context.from_id,
                thread_workspace_dir.as_deref(),
            ),
        );
        let requested_provider = dispatch_metadata
            .get("requested_provider_type")
            .and_then(Value::as_str)
            .and_then(|value| match value {
                "claude_code" => Some(ProviderType::ClaudeCode),
                "codex_app_server" => Some(ProviderType::CodexAppServer),
                "gemini_cli" => Some(ProviderType::GeminiCli),
                "agent_team" => Some(ProviderType::AgentTeam),
                _ => None,
            });

        self.set_last_delivery_with_persistence(&thread_id, delivery_context)
            .await;

        if is_canonical_thread_id(&thread_id) {
            self.record_thread_log(
                ThreadLogEvent::info(thread_id.clone(), "dispatch", dispatch_message)
                    .with_run_id(context.run_id.clone())
                    .with_field("channel", json!(context.channel))
                    .with_field("account_id", json!(context.account_id))
                    .with_field("from_id", json!(context.from_id))
                    .with_field(
                        "has_images",
                        json!(images.as_ref().is_some_and(|items| !items.is_empty())),
                    ),
            )
            .await;
        }

        dispatcher
            .dispatch(
                AgentRunRequest::new(
                    &thread_id,
                    &context.message,
                    &context.run_id,
                    &context.channel,
                    &context.account_id,
                    dispatch_metadata,
                )
                .with_images(images)
                .with_workspace_dir(thread_workspace_dir)
                .with_requested_provider(requested_provider),
                response_callback,
            )
            .await?;

        Ok(InboundResult {
            thread_id,
            metadata: message_metadata,
            local_reply: None,
        })
    }
}
