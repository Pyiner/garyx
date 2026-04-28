use tracing::info;

use garyx_models::routing::{
    DELIVERY_TARGET_TYPE_CHAT_ID, DeliveryContext, infer_delivery_target_id,
    infer_delivery_target_type,
};
use garyx_models::{MessageLedgerEvent, MessageLifecycleStatus};
use serde_json::{Value, json};

use super::super::inbound::NativeCommand;
use super::super::*;

impl MessageRouter {
    pub(super) async fn handle_local_command(
        &mut self,
        request: &InboundRequest,
        command: NativeCommand,
    ) -> Result<InboundResult, String> {
        let thread_binding_key = request.thread_binding_key.as_str();
        let local = match command {
            NativeCommand::Thread(thread_command) => {
                self.execute_native_thread_command(
                    &request.channel,
                    &request.account_id,
                    &request.from_id,
                    request.is_group,
                    thread_binding_key,
                    thread_command,
                )
                .await?
            }
            NativeCommand::Loop => {
                let current_thread = self
                    .resolve_or_create_inbound_thread(
                        &request.channel,
                        &request.account_id,
                        thread_binding_key,
                        &request.extra_metadata,
                    )
                    .await;

                let (reply_text, switched) = self.toggle_loop_mode(&current_thread).await;
                NativeThreadResult {
                    reply_text,
                    switched_thread: Some(switched),
                }
            }
        };

        let thread_id = if let Some(switched_thread) = local.switched_thread {
            switched_thread
        } else {
            self.resolve_or_create_inbound_thread(
                &request.channel,
                &request.account_id,
                thread_binding_key,
                &request.extra_metadata,
            )
            .await
        };

        let delivery_chat_id =
            Self::resolve_delivery_chat_id(&request.extra_metadata, &request.from_id);
        let delivery_target_type = infer_delivery_target_type(
            &request.channel,
            Some(DELIVERY_TARGET_TYPE_CHAT_ID),
            Some(&delivery_chat_id),
            &delivery_chat_id,
            &request.from_id,
        );
        let delivery_target_id = infer_delivery_target_id(
            &request.channel,
            Some(&delivery_target_type),
            Some(&delivery_chat_id),
            &delivery_chat_id,
            &request.from_id,
        );
        self.set_last_delivery_with_persistence(
            &thread_id,
            DeliveryContext {
                channel: request.channel.clone(),
                account_id: request.account_id.clone(),
                chat_id: delivery_chat_id,
                user_id: request.from_id.clone(),
                delivery_target_type,
                delivery_target_id,
                thread_id: Some(thread_binding_key.to_owned()),
                metadata: Default::default(),
            },
        )
        .await;

        let message_metadata = Self::enrich_metadata(
            &request.channel,
            &request.account_id,
            &request.from_id,
            request.is_group,
            Some(thread_binding_key),
            &thread_id,
        );
        info!(
            channel = %request.channel,
            account_id = %request.account_id,
            from_id = %request.from_id,
            command = crate::router::inbound::InboundCommandClassifier::name(command),
            "native command handled in router"
        );
        self.record_message_ledger_event(MessageLedgerEvent {
            ledger_id: format!("run:{}", request.run_id),
            bot_id: format!("{}:{}", request.channel, request.account_id),
            status: MessageLifecycleStatus::ThreadResolved,
            created_at: chrono::Utc::now().to_rfc3339(),
            thread_id: Some(thread_id.clone()),
            run_id: Some(request.run_id.clone()),
            channel: Some(request.channel.clone()),
            account_id: Some(request.account_id.clone()),
            chat_id: request
                .extra_metadata
                .get("chat_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            from_id: Some(request.from_id.clone()),
            native_message_id: request
                .extra_metadata
                .get("message_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            text_excerpt: Some(request.message.chars().take(200).collect()),
            terminal_reason: None,
            reply_message_id: None,
            metadata: json!({
                "source": "router_local_command",
                "command": crate::router::inbound::InboundCommandClassifier::name(command),
            }),
        })
        .await;
        Ok(InboundResult {
            thread_id,
            metadata: message_metadata,
            local_reply: Some(local.reply_text),
        })
    }
}
