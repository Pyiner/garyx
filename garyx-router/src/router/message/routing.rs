use super::super::*;
use chrono::Utc;
use garyx_models::messages::MessageMetadata;
use garyx_models::thread_logs::{ThreadLogEvent, is_canonical_thread_id};
use serde_json::{Value, json};
use std::collections::HashMap;
use tracing::{debug, info, warn};

impl MessageRouter {
    // ------------------------------------------------------------------
    // Agent / route resolution (simplified until garyx-core is ready)
    // ------------------------------------------------------------------

    /// Resolve which agent should handle messages for a given channel context.
    ///
    /// This is a simplified version that returns the default agent.
    /// Full binding-based resolution will be available once `garyx_core::route_resolver`
    /// is implemented.
    pub fn resolve_agent_for_channel(
        &self,
        _channel: &str,
        _account_id: &str,
        _from_id: Option<&str>,
        _is_group: bool,
    ) -> &str {
        &self.default_agent
    }

    // ------------------------------------------------------------------
    // Inbound message routing (thread resolution only, no dispatch)
    // ------------------------------------------------------------------

    /// Resolve the current thread for an inbound message.
    ///
    /// 1. Builds account-scoped user_key from channel/account_id/from_id/is_group/group_id
    /// 2. If user has a switched thread, returns that
    /// 3. Otherwise returns a fresh ordinary thread id
    pub fn resolve_inbound_thread(
        &mut self,
        channel: &str,
        account_id: &str,
        from_id: &str,
        _is_group: bool,
        thread_binding_key: Option<&str>,
    ) -> String {
        let binding_key = thread_binding_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| from_id.trim());
        if binding_key.is_empty() {
            debug!(
                channel,
                account_id, "Resolved inbound thread without binding key -> fresh thread"
            );
            return crate::threads::new_thread_key();
        }
        if let Some(thread_id) =
            self.get_current_thread_id_for_binding(channel, account_id, binding_key)
        {
            info!("Using switched thread: {}", thread_id);
            return thread_id.to_owned();
        }

        debug!(
            "Resolved inbound thread: {}:{} -> fresh thread",
            channel, binding_key
        );
        crate::threads::new_thread_key()
    }

    /// Look up a thread id for a reply message via the message routing index.
    pub fn resolve_reply_thread(
        &self,
        channel: &str,
        account_id: &str,
        reply_to_message_id: &str,
    ) -> Option<&str> {
        self.reply_routing.message_routing_index.lookup_thread(
            channel,
            account_id,
            reply_to_message_id,
        )
    }

    pub fn resolve_reply_thread_for_chat(
        &self,
        channel: &str,
        account_id: &str,
        chat_id: Option<&str>,
        thread_binding_key: Option<&str>,
        reply_to_message_id: &str,
    ) -> Option<&str> {
        self.reply_routing
            .message_routing_index
            .lookup_thread_for_chat(
                channel,
                account_id,
                chat_id,
                thread_binding_key,
                reply_to_message_id,
            )
    }

    // ------------------------------------------------------------------
    // Message routing index delegation
    // ------------------------------------------------------------------

    /// Record an outbound message for reply routing.
    pub fn record_outbound_message(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        message_id: &str,
    ) {
        self.record_outbound_message_for_chat(thread_id, channel, account_id, "", None, message_id);
    }

    pub fn record_outbound_message_for_chat(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
        message_id: &str,
    ) {
        self.reply_routing.message_routing_index.record_outbound(
            thread_id,
            channel,
            account_id,
            chat_id,
            thread_binding_key,
            message_id,
        );
    }

    /// Record an outbound message for reply routing and persist it to the thread.
    pub async fn record_outbound_message_with_persistence(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
        message_id: &str,
    ) {
        self.record_outbound_message_with_thread_log(
            thread_id,
            channel,
            account_id,
            chat_id,
            thread_binding_key,
            message_id,
            None,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_outbound_message_with_thread_log(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
        message_id: &str,
        thread_log_id: Option<&str>,
    ) {
        self.record_outbound_message_for_chat(
            thread_id,
            channel,
            account_id,
            chat_id,
            thread_binding_key,
            message_id,
        );
        self.persist_outbound_message_id(
            thread_id,
            channel,
            account_id,
            chat_id,
            thread_binding_key,
            message_id,
        )
        .await;
        let target_thread_id = thread_log_id
            .map(str::trim)
            .filter(|value| is_canonical_thread_id(value))
            .map(ToOwned::to_owned)
            .or_else(|| {
                let trimmed = thread_id.trim();
                if is_canonical_thread_id(trimmed) {
                    Some(trimmed.to_owned())
                } else {
                    None
                }
            });
        if let Some(thread_id) = target_thread_id {
            self.record_thread_log(
                ThreadLogEvent::info(&thread_id, "delivery", "outbound message delivered")
                    .with_field("channel", json!(channel))
                    .with_field("account_id", json!(account_id))
                    .with_field("chat_id", json!(chat_id))
                    .with_field("message_id", json!(message_id))
                    .with_field("thread_id", json!(thread_id)),
            )
            .await;
        }
    }

    async fn persist_outbound_message_id(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
        message_id: &str,
    ) {
        let normalized_message_id = message_id.trim();
        if normalized_message_id.is_empty() {
            debug!(
                thread_id,
                channel, account_id, "Skipping outbound message persistence with empty message_id"
            );
            return;
        }

        let Some(mut thread_data) = self.threads.get(thread_id).await else {
            debug!(
                thread_id,
                channel, account_id, "Thread missing; skipping outbound message persistence"
            );
            return;
        };
        let Some(obj) = thread_data.as_object_mut() else {
            warn!(
                thread_id,
                channel,
                account_id,
                "Thread payload is not an object; skipping outbound message persistence"
            );
            return;
        };

        let mut records = obj
            .get("outbound_message_ids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        records.push(serde_json::json!({
            "channel": channel,
            "account_id": account_id,
            "chat_id": chat_id,
            "thread_binding_key": thread_binding_key,
            "message_id": normalized_message_id,
            "timestamp": Utc::now().to_rfc3339(),
        }));
        if records.len() > MAX_OUTBOUND_MESSAGE_IDS {
            let keep_from = records.len() - MAX_OUTBOUND_MESSAGE_IDS;
            records = records.split_off(keep_from);
        }

        obj.insert("outbound_message_ids".to_owned(), Value::Array(records));
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
        self.threads.set(thread_id, thread_data).await;
    }

    /// Rebuild the message routing index from the thread store.
    pub async fn rebuild_routing_index(&mut self, channel: &str) -> usize {
        self.reply_routing
            .message_routing_index
            .rebuild_from_store(self.threads.as_ref(), channel)
            .await
    }

    /// Get a reference to the message routing index.
    pub fn message_routing_index(&self) -> &MessageRoutingIndex {
        &self.reply_routing.message_routing_index
    }

    /// Get a mutable reference to the message routing index.
    pub fn message_routing_index_mut(&mut self) -> &mut MessageRoutingIndex {
        &mut self.reply_routing.message_routing_index
    }

    pub fn clear_reply_routing_for_chat(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
    ) {
        self.reply_routing.message_routing_index.clear_thread_chat(
            thread_id,
            channel,
            account_id,
            chat_id,
            thread_binding_key,
        );
    }

    pub async fn clear_reply_routing_for_chat_with_persistence(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
    ) {
        self.clear_reply_routing_for_chat(
            thread_id,
            channel,
            account_id,
            chat_id,
            thread_binding_key,
        );

        let Some(mut thread_data) = self.threads.get(thread_id).await else {
            return;
        };
        let Some(obj) = thread_data.as_object_mut() else {
            return;
        };
        let Some(records) = obj
            .get_mut("outbound_message_ids")
            .and_then(Value::as_array_mut)
        else {
            return;
        };

        let normalized_scope = thread_binding_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        let clear_scoped_thread = !normalized_scope.is_empty();

        let original_len = records.len();
        records.retain(|record| {
            let Some(item) = record.as_object() else {
                return true;
            };
            let matches_channel = item
                .get("channel")
                .and_then(Value::as_str)
                .is_some_and(|value| value == channel);
            let matches_account = item
                .get("account_id")
                .and_then(Value::as_str)
                .is_some_and(|value| value == account_id);
            let item_chat = item
                .get("chat_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let matches_chat = if clear_scoped_thread {
                item_chat == chat_id
            } else {
                item_chat == chat_id || item_chat.is_empty()
            };
            let item_scope = item
                .get("thread_binding_key")
                .or_else(|| item.get("thread_scope"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let matches_scope = if clear_scoped_thread {
                item_scope == normalized_scope
            } else {
                item_scope.is_empty()
            };

            !(matches_channel && matches_account && matches_chat && matches_scope)
        });

        if records.len() == original_len {
            return;
        }

        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
        self.threads.set(thread_id, thread_data).await;
    }

    /// Clone the underlying thread store handle.
    pub fn thread_store(&self) -> Arc<dyn ThreadStore> {
        self.threads.clone()
    }

    // ------------------------------------------------------------------
    // Metadata enrichment
    // ------------------------------------------------------------------

    /// Build a [`MessageMetadata`] from the inbound message context.
    ///
    /// This is called internally by `route_and_dispatch` but is also
    /// available for channel handlers that need the metadata without
    /// full dispatch.
    pub fn enrich_metadata(
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
        thread_id: Option<&str>,
        resolved_thread_id: &str,
    ) -> MessageMetadata {
        MessageMetadata {
            channel: Some(channel.to_owned()),
            account_id: Some(account_id.to_owned()),
            from_id: Some(from_id.to_owned()),
            is_group,
            thread_id: thread_id.map(|s| s.to_owned()),
            resolved_thread_id: Some(resolved_thread_id.to_owned()),
            extra: HashMap::new(),
        }
    }
}
