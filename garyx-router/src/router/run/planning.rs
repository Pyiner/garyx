use std::collections::HashMap;

use garyx_core::annotate_slash_command_metadata;
use garyx_models::messages::MessageMetadata;
use garyx_models::thread_logs::{ThreadLogEvent, is_canonical_thread_id};
use garyx_models::{MessageLedgerEvent, MessageLifecycleStatus};
use serde_json::{Value, json};
use tracing::debug;

use super::super::*;
use crate::bindings_from_value;
use garyx_models::routing::{
    DELIVERY_TARGET_TYPE_CHAT_ID, infer_delivery_target_id, infer_delivery_target_type,
};

pub(super) struct DispatchContext {
    pub(super) channel: String,
    pub(super) account_id: String,
    pub(super) from_id: String,
    pub(super) is_group: bool,
    pub(super) thread_binding_key: String,
    pub(super) message: String,
    pub(super) run_id: String,
    pub(super) reply_to_message_id: Option<String>,
    pub(super) extra_metadata: HashMap<String, Value>,
    pub(super) images: Vec<ImagePayload>,
    pub(super) file_paths: Vec<String>,
}

impl DispatchContext {
    pub(super) fn route_context(&self) -> RouteContext<'_> {
        RouteContext {
            channel: &self.channel,
            account_id: &self.account_id,
            thread_binding_key: &self.thread_binding_key,
            reply_to_message_id: self.reply_to_message_id.as_deref(),
            extra_metadata: &self.extra_metadata,
        }
    }
}

impl From<InboundRequest> for DispatchContext {
    fn from(request: InboundRequest) -> Self {
        Self {
            channel: request.channel,
            account_id: request.account_id,
            from_id: request.from_id,
            is_group: request.is_group,
            thread_binding_key: request.thread_binding_key,
            message: request.message,
            run_id: request.run_id,
            reply_to_message_id: request.reply_to_message_id,
            extra_metadata: request.extra_metadata,
            images: request.images,
            file_paths: request.file_paths,
        }
    }
}

pub(super) struct DispatchPlan {
    pub(super) context: DispatchContext,
    pub(super) thread_id: String,
    pub(super) message_metadata: MessageMetadata,
    pub(super) dispatch_metadata: HashMap<String, Value>,
    pub(super) delivery_context: DeliveryContext,
    pub(super) images: Option<Vec<ImagePayload>>,
    pub(super) file_paths: Vec<String>,
}

impl MessageRouter {
    fn thread_string_field(thread: &Value, key: &str) -> Option<String> {
        thread
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn thread_bool_field(thread: &Value, key: &str) -> Option<bool> {
        thread.get(key).and_then(Value::as_bool)
    }

    fn resolve_bound_thread_binding_key(
        thread_data: &Value,
        channel: &str,
        account_id: &str,
        fallback: &str,
    ) -> String {
        let fallback = fallback.trim();
        if let Some(binding) = bindings_from_value(thread_data)
            .into_iter()
            .find(|binding| binding.channel == channel && binding.account_id == account_id)
            .or_else(|| bindings_from_value(thread_data).into_iter().next())
            && !binding.binding_key.trim().is_empty()
        {
            return binding.binding_key;
        }

        Self::thread_string_field(thread_data, "thread_binding_key")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| fallback.to_owned())
    }

    fn sanitize_delivery_thread_id(
        delivery_context: Option<&DeliveryContext>,
        thread_binding_key: &str,
        is_group: bool,
    ) -> Option<String> {
        let thread_binding_key = thread_binding_key.trim();
        let delivery_thread_id = delivery_context
            .and_then(|ctx| ctx.thread_id.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if !is_group && delivery_thread_id == thread_binding_key {
            return None;
        }
        Some(delivery_thread_id.to_owned())
    }

    pub(super) async fn build_thread_dispatch_context(
        &self,
        thread_id: &str,
        request: ThreadMessageRequest,
    ) -> Result<DispatchContext, String> {
        let thread_data = self
            .threads
            .get(thread_id)
            .await
            .ok_or_else(|| format!("thread not found: {thread_id}"))?;
        let delivery_context = self
            .resolve_delivery_target(thread_id)
            .map(|(_, ctx)| ctx)
            .or_else(|| Self::extract_delivery_context_from_thread(&thread_data));

        let channel = Self::thread_string_field(&thread_data, "channel")
            .or_else(|| delivery_context.as_ref().map(|ctx| ctx.channel.clone()))
            .unwrap_or_else(|| "api".to_owned());
        let account_id = Self::thread_string_field(&thread_data, "account_id")
            .or_else(|| delivery_context.as_ref().map(|ctx| ctx.account_id.clone()))
            .unwrap_or_else(|| "main".to_owned());
        let from_id = Self::thread_string_field(&thread_data, "from_id")
            .or_else(|| delivery_context.as_ref().map(|ctx| ctx.user_id.clone()))
            .or_else(|| delivery_context.as_ref().map(|ctx| ctx.chat_id.clone()))
            .unwrap_or_else(|| "loop".to_owned());
        let is_group = Self::thread_bool_field(&thread_data, "is_group").unwrap_or(false);
        let fallback_binding_key = if is_group {
            delivery_context
                .as_ref()
                .map(|ctx| ctx.chat_id.clone())
                .unwrap_or_else(|| from_id.clone())
        } else {
            from_id.clone()
        };
        let thread_binding_key = Self::resolve_bound_thread_binding_key(
            &thread_data,
            &channel,
            &account_id,
            &fallback_binding_key,
        );
        let delivery_thread_id = Self::sanitize_delivery_thread_id(
            delivery_context.as_ref(),
            &thread_binding_key,
            is_group,
        );

        let mut extra_metadata = request.extra_metadata;
        if let Some(delivery) = &delivery_context {
            extra_metadata
                .entry("chat_id".to_owned())
                .or_insert_with(|| Value::String(delivery.chat_id.clone()));
            if let Some(thread_id) = delivery_thread_id.as_ref() {
                extra_metadata
                    .entry("thread_id".to_owned())
                    .or_insert_with(|| Value::String(thread_id.clone()));
            }
        }
        extra_metadata
            .entry("delivery_thread_id".to_owned())
            .or_insert_with(|| match delivery_thread_id {
                Some(thread_id) => Value::String(thread_id),
                None => Value::Null,
            });

        Ok(DispatchContext {
            channel,
            account_id,
            from_id,
            is_group,
            thread_binding_key,
            message: request.message,
            run_id: request.run_id,
            reply_to_message_id: None,
            extra_metadata,
            images: request.images,
            file_paths: request.file_paths,
        })
    }

    fn explicit_delivery_thread_id(
        extra_metadata: &HashMap<String, Value>,
    ) -> Option<Option<String>> {
        for key in ["delivery_thread_id", "thread_scope"] {
            let Some(value) = extra_metadata.get(key) else {
                continue;
            };
            return Some(match value {
                Value::String(value) => {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                }
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            });
        }
        None
    }

    fn build_dispatch_metadata(
        mut metadata: HashMap<String, Value>,
        dispatch: DispatchMetadataContext<'_>,
        reply_routed: bool,
    ) -> HashMap<String, Value> {
        metadata.insert(
            "channel".to_owned(),
            Value::String(dispatch.navigation.channel.to_owned()),
        );
        metadata.insert(
            "account_id".to_owned(),
            Value::String(dispatch.navigation.account_id.to_owned()),
        );
        metadata.insert(
            "from_id".to_owned(),
            Value::String(dispatch.from_id.to_owned()),
        );
        metadata.insert("is_group".to_owned(), Value::Bool(dispatch.is_group));
        metadata.insert(
            "thread_binding_key".to_owned(),
            Value::String(dispatch.navigation.thread_binding_key.to_owned()),
        );
        if reply_routed {
            if let Some(reply_to_message_id) = dispatch.reply_to_message_id {
                metadata.insert(
                    "reply_to_message_id".to_owned(),
                    Value::String(reply_to_message_id.to_owned()),
                );
            }
            metadata.insert("is_reply_routed".to_owned(), Value::Bool(true));
        }
        metadata
    }

    fn build_delivery_context(
        channel: &str,
        account_id: &str,
        chat_id: String,
        user_id: &str,
        thread_id: Option<String>,
    ) -> DeliveryContext {
        let delivery_target_type = infer_delivery_target_type(
            channel,
            Some(DELIVERY_TARGET_TYPE_CHAT_ID),
            Some(&chat_id),
            &chat_id,
            user_id,
        );
        let delivery_target_id = infer_delivery_target_id(
            channel,
            Some(&delivery_target_type),
            Some(&chat_id),
            &chat_id,
            user_id,
        );
        DeliveryContext {
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            chat_id,
            user_id: user_id.to_owned(),
            delivery_target_type,
            delivery_target_id,
            thread_id,
            metadata: HashMap::new(),
        }
    }

    fn normalize_images(images: Vec<ImagePayload>) -> Option<Vec<ImagePayload>> {
        if images.is_empty() {
            None
        } else {
            Some(images)
        }
    }

    pub(super) async fn build_dispatch_plan_for_thread(
        &mut self,
        mut context: DispatchContext,
        thread_id: String,
        reply_routed: bool,
    ) -> DispatchPlan {
        let thread_binding_key = context.thread_binding_key.clone();
        let thread_binding_key_ref = thread_binding_key.as_str();
        let reply_to_message_id = context.reply_to_message_id.clone();
        let reply_to_message_id_ref = reply_to_message_id.as_deref();

        if is_canonical_thread_id(&thread_id) {
            if let Some(reply_to_message_id) = reply_to_message_id_ref {
                let message = if reply_routed {
                    "reply route matched existing thread"
                } else {
                    "reply route missed; falling back to thread resolution"
                };
                self.record_thread_log(
                    ThreadLogEvent::info(thread_id.clone(), "routing", message)
                        .with_run_id(context.run_id.clone())
                        .with_field("reply_to_message_id", json!(reply_to_message_id)),
                )
                .await;
            }

            self.record_thread_log(
                ThreadLogEvent::info(thread_id.clone(), "routing", "resolved inbound thread")
                    .with_run_id(context.run_id.clone())
                    .with_field("channel", json!(context.channel))
                    .with_field("account_id", json!(context.account_id))
                    .with_field("from_id", json!(context.from_id))
                    .with_field("is_group", json!(context.is_group))
                    .with_field("thread_binding_key", json!(thread_binding_key_ref))
                    .with_field("reply_routed", json!(reply_routed)),
            )
            .await;
        }

        self.record_message_ledger_event(MessageLedgerEvent {
            ledger_id: dispatch_ledger_id(&context),
            bot_id: format!("{}:{}", context.channel, context.account_id),
            status: MessageLifecycleStatus::ThreadResolved,
            created_at: chrono::Utc::now().to_rfc3339(),
            thread_id: Some(thread_id.clone()),
            run_id: Some(context.run_id.clone()),
            channel: Some(context.channel.clone()),
            account_id: Some(context.account_id.clone()),
            chat_id: context
                .extra_metadata
                .get("chat_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            from_id: Some(context.from_id.clone()),
            native_message_id: native_message_id(&context.extra_metadata),
            text_excerpt: Some(context.message.chars().take(200).collect()),
            terminal_reason: None,
            reply_message_id: None,
            metadata: json!({
                "source": "router_thread_resolution",
                "reply_routed": reply_routed,
                "thread_binding_key": thread_binding_key_ref,
            }),
        })
        .await;

        self.apply_history_limit_if_needed(&thread_id, &context.extra_metadata)
            .await;
        self.backfill_thread_context_if_missing(
            &thread_id,
            &context.channel,
            &context.account_id,
            &context.from_id,
            context.is_group,
        )
        .await;

        let mut message_metadata = Self::enrich_metadata(
            &context.channel,
            &context.account_id,
            &context.from_id,
            context.is_group,
            Some(thread_binding_key_ref),
            &thread_id,
        );
        annotate_slash_command_metadata(&mut message_metadata, &context.extra_metadata);
        let delivery_chat_id =
            Self::resolve_delivery_chat_id(&context.extra_metadata, &context.from_id);
        let dispatch_context = DispatchMetadataContext {
            navigation: NavigationContext {
                channel: &context.channel,
                account_id: &context.account_id,
                thread_binding_key: thread_binding_key_ref,
            },
            from_id: &context.from_id,
            is_group: context.is_group,
            reply_to_message_id: reply_to_message_id_ref,
        };

        let delivery_thread_id = match Self::explicit_delivery_thread_id(&context.extra_metadata) {
            Some(value) => value,
            None => context.is_group.then(|| thread_binding_key.clone()),
        };
        if let Some(thread_id) = delivery_thread_id.as_ref() {
            context
                .extra_metadata
                .entry("thread_id".to_owned())
                .or_insert_with(|| Value::String(thread_id.clone()));
            context
                .extra_metadata
                .entry("delivery_thread_id".to_owned())
                .or_insert_with(|| Value::String(thread_id.clone()));
        }
        let dispatch_metadata = Self::build_dispatch_metadata(
            std::mem::take(&mut context.extra_metadata),
            dispatch_context,
            reply_routed,
        );
        let delivery_context = Self::build_delivery_context(
            &context.channel,
            &context.account_id,
            delivery_chat_id,
            &context.from_id,
            delivery_thread_id,
        );
        let images = Self::normalize_images(std::mem::take(&mut context.images));
        let file_paths = std::mem::take(&mut context.file_paths);

        DispatchPlan {
            context,
            thread_id,
            message_metadata,
            dispatch_metadata,
            delivery_context,
            images,
            file_paths,
        }
    }

    pub(super) async fn build_dispatch_plan(&mut self, context: DispatchContext) -> DispatchPlan {
        let route = context.route_context();
        let (thread_id, reply_routed) = self.resolve_thread_for_request(route).await;
        let thread_id = self
            .apply_auto_recovery_if_needed(route, thread_id, reply_routed)
            .await;

        self.build_dispatch_plan_for_thread(context, thread_id, reply_routed)
            .await
    }

    async fn apply_history_limit_if_needed(
        &self,
        thread_id: &str,
        extra_metadata: &HashMap<String, Value>,
    ) {
        if let Some(limit) = extra_metadata
            .get("thread_history_limit")
            .or_else(|| extra_metadata.get("history_limit"))
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0)
            .map(|v| v as usize)
        {
            let trimmed = self.trim_thread_history(thread_id, limit).await;
            if trimmed > 0 {
                debug!(
                    thread_id = %thread_id,
                    thread_history_limit = limit,
                    trimmed,
                    "trimmed thread history before dispatch"
                );
            }
        }
    }
}

fn native_message_id(extra_metadata: &HashMap<String, Value>) -> Option<String> {
    extra_metadata
        .get("native_message_id")
        .or_else(|| extra_metadata.get("message_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn dispatch_ledger_id(context: &DispatchContext) -> String {
    if let Some(native_message_id) = native_message_id(&context.extra_metadata) {
        let chat_id = context
            .extra_metadata
            .get("chat_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown-chat");
        return format!(
            "{}:{}:{}",
            format_args!("{}:{}", context.channel, context.account_id),
            chat_id,
            native_message_id
        );
    }
    format!("run:{}", context.run_id)
}
