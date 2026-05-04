use std::collections::HashMap;

use chrono::Utc;
use garyx_models::routing::{
    DELIVERY_TARGET_TYPE_CHAT_ID, infer_delivery_target_id, infer_delivery_target_type,
};
use serde_json::Value;

use super::super::*;
use crate::threads::{
    ChannelBinding, ThreadEnsureOptions, ThreadIndexStats, bind_endpoint_to_thread,
    default_agent_for_channel_account, default_workspace_for_channel_account, endpoint_key,
    label_from_value, list_known_channel_endpoints, new_thread_key,
};

fn delivery_thread_scope_from_binding(binding: &ChannelBinding) -> Option<String> {
    let binding_key = binding.binding_key.trim();
    if binding_key.is_empty() {
        return None;
    }
    let chat_id = binding.chat_id.trim();
    if binding.channel.eq_ignore_ascii_case("telegram") && binding_key == chat_id {
        return None;
    }
    Some(binding_key.to_owned())
}

impl MessageRouter {
    pub async fn check_auto_recovery(&self, thread_id: &str) -> Option<String> {
        let data = self.threads.get(thread_id).await?;
        data.get("auto_recover_next_thread")
            .or_else(|| data.get("auto_recover_next_session"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && *s != thread_id)
            .map(|s| s.to_owned())
    }

    pub async fn bind_endpoint_runtime(
        &mut self,
        thread_id: &str,
        binding: ChannelBinding,
    ) -> Result<Option<String>, String> {
        let previous_thread_id =
            bind_endpoint_to_thread(&self.threads, thread_id, binding.clone()).await?;
        if let Some(previous_thread_id) = previous_thread_id.as_deref()
            && previous_thread_id != thread_id
        {
            self.clear_last_delivery_for_chat_with_persistence(
                previous_thread_id,
                &binding.channel,
                &binding.account_id,
                &binding.chat_id,
                Some(&binding.binding_key),
            )
            .await;
        }
        self.set_last_delivery_with_persistence(
            thread_id,
            garyx_models::routing::DeliveryContext {
                channel: binding.channel.clone(),
                account_id: binding.account_id.clone(),
                chat_id: binding.chat_id.clone(),
                user_id: binding.chat_id.clone(),
                delivery_target_type: binding.resolved_delivery_target_type(),
                delivery_target_id: binding.resolved_delivery_target_id(),
                thread_id: delivery_thread_scope_from_binding(&binding),
                metadata: Default::default(),
            },
        )
        .await;
        self.thread_nav
            .endpoint_thread_map
            .insert(binding.endpoint_key(), thread_id.to_owned());
        Ok(previous_thread_id)
    }

    pub(in crate::router) async fn current_canonical_thread_for_binding(
        &mut self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> Option<String> {
        if let Some(bound_thread) = self
            .resolve_endpoint_thread_id(channel, account_id, thread_binding_key)
            .await
        {
            return Some(bound_thread);
        }

        let existing = self
            .get_current_thread_id_for_binding(channel, account_id, thread_binding_key)
            .map(str::to_owned)?;
        if !self.threads.exists(&existing).await {
            return None;
        }
        Some(existing)
    }

    pub async fn rebuild_thread_indexes(&mut self) -> ThreadIndexStats {
        let stale_threads = self
            .thread_nav
            .binding_thread_map
            .values()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        for thread_id in stale_threads {
            if crate::threads::is_thread_key(&thread_id) && !self.threads.exists(&thread_id).await {
                self.clear_thread_references(&thread_id);
            }
        }

        self.thread_nav.endpoint_thread_map.clear();

        let mut stats = ThreadIndexStats::default();
        for endpoint in list_known_channel_endpoints(&self.threads).await {
            if let Some(thread_id) = endpoint.thread_id {
                self.thread_nav
                    .endpoint_thread_map
                    .insert(endpoint.endpoint_key.clone(), thread_id);
                stats.endpoint_bindings += 1;
            } else {
                self.clear_binding_thread_context(&endpoint.endpoint_key);
            }
        }

        stats
    }

    pub async fn resolve_canonical_thread_id(&mut self, thread_id: &str) -> Option<String> {
        let trimmed = thread_id.trim();
        if trimmed.is_empty() {
            return None;
        }
        if self.threads.exists(trimmed).await {
            return Some(trimmed.to_owned());
        }
        None
    }

    pub(in crate::router) async fn endpoint_binding_for_thread(
        &self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
        preferred_thread_id: Option<&str>,
    ) -> Option<ChannelBinding> {
        let endpoint = endpoint_key(channel, account_id, thread_binding_key);
        if let Some(existing) = list_known_channel_endpoints(&self.threads)
            .await
            .into_iter()
            .find(|candidate| candidate.endpoint_key == endpoint)
        {
            return Some(ChannelBinding {
                channel: existing.channel,
                account_id: existing.account_id,
                binding_key: existing.binding_key,
                chat_id: existing.chat_id,
                delivery_target_type: existing.delivery_target_type,
                delivery_target_id: existing.delivery_target_id,
                display_label: existing.display_label,
                last_inbound_at: existing.last_inbound_at,
                last_delivery_at: existing.last_delivery_at,
            });
        }

        let display_label = match preferred_thread_id {
            Some(key) => self
                .threads
                .get(key)
                .await
                .and_then(|value| label_from_value(&value))
                .unwrap_or_else(|| thread_binding_key.to_owned()),
            None => thread_binding_key.to_owned(),
        };

        Some(ChannelBinding {
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            binding_key: thread_binding_key.to_owned(),
            chat_id: String::new(),
            delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
            delivery_target_id: String::new(),
            display_label,
            last_inbound_at: Some(Utc::now().to_rfc3339()),
            last_delivery_at: None,
        })
    }

    fn resolve_display_label(
        extra_metadata: &HashMap<String, Value>,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> String {
        for field in [
            "display_label",
            "sender_name",
            "sender_display_name",
            "sender_username",
            "sender_first_name",
        ] {
            if let Some(value) = extra_metadata
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return value.to_owned();
            }
        }
        format!("{channel}/{account_id}/{thread_binding_key}")
    }

    fn resolve_chat_id_from_metadata(
        extra_metadata: &HashMap<String, Value>,
        thread_binding_key: &str,
    ) -> String {
        extra_metadata
            .get("chat_id")
            .and_then(|value| match value {
                Value::String(value) => Some(value.clone()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| thread_binding_key.to_owned())
    }

    pub async fn resolve_or_create_inbound_thread(
        &mut self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
        extra_metadata: &HashMap<String, Value>,
    ) -> String {
        if let Some(existing) = self
            .current_canonical_thread_for_binding(channel, account_id, thread_binding_key)
            .await
        {
            return existing;
        }

        let display_label =
            Self::resolve_display_label(extra_metadata, channel, account_id, thread_binding_key);
        let options = ThreadEnsureOptions {
            label: Some(display_label.clone()),
            workspace_dir: default_workspace_for_channel_account(&self.config, channel, account_id),
            agent_id: default_agent_for_channel_account(&self.config, channel, account_id),
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: Some(channel.to_owned()),
            origin_account_id: Some(account_id.to_owned()),
            origin_from_id: None,
            is_group: None,
        };
        let Ok((thread_id, _value)) = self.create_thread_with_options(options).await else {
            return new_thread_key();
        };

        let resolved_chat_id =
            Self::resolve_chat_id_from_metadata(extra_metadata, thread_binding_key);
        let binding = ChannelBinding {
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            binding_key: thread_binding_key.to_owned(),
            chat_id: resolved_chat_id.clone(),
            delivery_target_type: infer_delivery_target_type(
                channel,
                Some(DELIVERY_TARGET_TYPE_CHAT_ID),
                Some(&resolved_chat_id),
                &resolved_chat_id,
                &resolved_chat_id,
            ),
            delivery_target_id: infer_delivery_target_id(
                channel,
                Some(DELIVERY_TARGET_TYPE_CHAT_ID),
                Some(&resolved_chat_id),
                &resolved_chat_id,
                &resolved_chat_id,
            ),
            display_label,
            last_inbound_at: Some(Utc::now().to_rfc3339()),
            last_delivery_at: None,
        };
        if self
            .bind_endpoint_runtime(&thread_id, binding.clone())
            .await
            .is_err()
        {
            return thread_id;
        }

        thread_id
    }

    pub(in crate::router) async fn resolve_thread_for_request(
        &mut self,
        route: RouteContext<'_>,
    ) -> (String, bool) {
        if let Some(reply_id) = route.reply_to_message_id {
            let chat_id = Self::reply_chat_id(route.extra_metadata);
            if let Some(reply_thread) = self.resolve_reply_thread_for_chat(
                route.channel,
                route.account_id,
                chat_id.as_deref(),
                Some(route.thread_binding_key),
                reply_id,
            ) {
                let reply_thread = reply_thread.to_owned();
                if Self::is_scheduled_thread(&reply_thread) {
                    return (reply_thread, true);
                }
                if let Some(canonical) = self.resolve_canonical_thread_id(&reply_thread).await {
                    return (canonical, true);
                }
            }
        }

        (
            self.resolve_or_create_inbound_thread(
                route.channel,
                route.account_id,
                route.thread_binding_key,
                route.extra_metadata,
            )
            .await,
            false,
        )
    }

    pub(in crate::router) async fn apply_auto_recovery_if_needed(
        &mut self,
        route: RouteContext<'_>,
        mut thread_id: String,
        reply_routed: bool,
    ) -> String {
        let binding_context_key = Self::build_binding_context_key(
            route.channel,
            route.account_id,
            route.thread_binding_key,
        );
        if reply_routed && Self::is_scheduled_thread(&thread_id) {
            self.switch_to_thread(&binding_context_key, &thread_id);
        }
        if Self::is_scheduled_thread(&thread_id) {
            return thread_id;
        }

        if let Some(redirect_key) = self.check_auto_recovery(&thread_id).await {
            if self.threads.exists(&redirect_key).await {
                tracing::info!(
                    original = %thread_id,
                    redirect = %redirect_key,
                    "auto-recovery redirect applied"
                );
                if let Some(binding) = self
                    .endpoint_binding_for_thread(
                        route.channel,
                        route.account_id,
                        route.thread_binding_key,
                        Some(&redirect_key),
                    )
                    .await
                {
                    self.bind_endpoint_runtime(&redirect_key, binding.clone())
                        .await
                        .ok();
                }
                self.switch_to_thread(&binding_context_key, &redirect_key);
                thread_id = redirect_key;
            } else {
                tracing::debug!(
                    original = %thread_id,
                    redirect = %redirect_key,
                    "auto-recovery target missing; ignored redirect"
                );
            }
        }

        thread_id
    }

    pub async fn resolve_endpoint_thread_id(
        &mut self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> Option<String> {
        let key = endpoint_key(channel, account_id, thread_binding_key);
        self.thread_nav.endpoint_thread_map.get(&key).cloned()
    }
}
