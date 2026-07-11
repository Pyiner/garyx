use crate::store::ThreadStoreExt;
use std::collections::HashMap;

use chrono::Utc;
use garyx_models::routing::{infer_delivery_target_id, infer_delivery_target_type};
use serde_json::Value;

use super::super::*;
use crate::threads::{
    ChannelBinding, ThreadEnsureOptions, default_agent_for_channel_account,
    default_workspace_for_channel_account, default_workspace_mode_for_channel_account,
    endpoint_key, new_thread_key, worktree_base_dir_for_config,
};
use crate::{EndpointBindingMutationError, EndpointDetachResult};

const INBOUND_FALLBACK_AGENT_ID: &str = "claude";

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
        let data = self.threads.get_logged(thread_id).await?;
        data.get("auto_recover_next_thread")
            .or_else(|| data.get("auto_recover_next_session"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && *s != thread_id)
            .map(|s| s.to_owned())
    }

    pub async fn bind_endpoint_runtime(
        &mut self,
        thread_id: &str,
        mut binding: ChannelBinding,
    ) -> Result<crate::EndpointBindResult, EndpointBindingMutationError> {
        let Some(mutator) = self.endpoint_binding_mutator() else {
            return Err(EndpointBindingMutationError::Unavailable);
        };
        let delivered_at = Utc::now().to_rfc3339();
        binding.last_delivery_at = Some(delivered_at);
        let result = mutator.bind_endpoint(thread_id, binding.clone()).await?;
        let previous_thread_id = result.previous_thread_id.clone();
        let binding = result.binding.clone();
        if let Some(previous_thread_id) = previous_thread_id.as_deref()
            && previous_thread_id != thread_id
        {
            self.clear_last_delivery_for_chat_with_known_thread_persistence(
                previous_thread_id,
                &binding.channel,
                &binding.account_id,
                &binding.chat_id,
                Some(&binding.binding_key),
            )
            .await;
        }
        self.set_last_delivery_with_known_thread_persistence(
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
        Ok(result)
    }

    pub async fn detach_endpoint_runtime(
        &mut self,
        endpoint_key: &str,
    ) -> Result<EndpointDetachResult, EndpointBindingMutationError> {
        let Some(mutator) = self.endpoint_binding_mutator() else {
            return Err(EndpointBindingMutationError::Unavailable);
        };
        let result = mutator.detach_endpoint(endpoint_key).await?;
        self.purge_endpoint_binding(endpoint_key);
        Ok(result)
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
        if !self.threads.exists_logged(&existing).await {
            return None;
        }
        Some(existing)
    }

    pub async fn resolve_canonical_thread_id(&mut self, thread_id: &str) -> Option<String> {
        let trimmed = thread_id.trim();
        if trimmed.is_empty() {
            return None;
        }
        if self.threads.exists_logged(trimmed).await {
            return Some(trimmed.to_owned());
        }
        None
    }

    pub(in crate::router) async fn endpoint_binding_from_inbound(
        &self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
        extra_metadata: &HashMap<String, Value>,
        fallback_display_label: Option<&str>,
    ) -> ChannelBinding {
        let endpoint = endpoint_key(channel, account_id, thread_binding_key);
        if let Some(mutator) = self.endpoint_binding_mutator() {
            match mutator.binding_for_endpoint(&endpoint).await {
                Ok(Some(owner)) => return owner.binding,
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(endpoint_key = endpoint, error = %error, "endpoint binding point lookup failed");
                }
            }
        }

        let chat_id = Self::resolve_chat_id_from_metadata(extra_metadata, thread_binding_key);
        let explicit_target_type = extra_metadata
            .get("delivery_target_type")
            .and_then(Value::as_str);
        let explicit_target_id = extra_metadata
            .get("delivery_target_id")
            .and_then(Value::as_str);
        let delivery_target_type = infer_delivery_target_type(
            channel,
            explicit_target_type,
            explicit_target_id,
            &chat_id,
            thread_binding_key,
        );
        let delivery_target_id = infer_delivery_target_id(
            channel,
            Some(&delivery_target_type),
            explicit_target_id,
            &chat_id,
            thread_binding_key,
        );
        ChannelBinding {
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            binding_key: thread_binding_key.to_owned(),
            chat_id,
            delivery_target_type,
            delivery_target_id,
            display_label: fallback_display_label
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| {
                    Self::resolve_display_label(
                        extra_metadata,
                        channel,
                        account_id,
                        thread_binding_key,
                    )
                }),
            last_inbound_at: Some(Utc::now().to_rfc3339()),
            last_delivery_at: None,
        }
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
        let workspace_mode =
            default_workspace_mode_for_channel_account(&self.config, channel, account_id);
        let options = ThreadEnsureOptions {
            label: Some(display_label.clone()),
            workspace_dir: default_workspace_for_channel_account(&self.config, channel, account_id),
            workspace_mode,
            worktree_base_dir: workspace_mode
                .is_worktree()
                .then(|| worktree_base_dir_for_config(&self.config)),
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
        let configured_agent_id = options.agent_id.clone();
        let (thread_id, _value) = match self.create_thread_with_options(options.clone()).await {
            Ok(created) => created,
            Err(error) => {
                let should_fallback = configured_agent_id
                    .as_deref()
                    .is_some_and(|agent_id| agent_id != INBOUND_FALLBACK_AGENT_ID);
                if should_fallback {
                    let mut fallback_options = options;
                    fallback_options.agent_id = Some(INBOUND_FALLBACK_AGENT_ID.to_owned());
                    match self.create_thread_with_options(fallback_options).await {
                        Ok(created) => {
                            tracing::warn!(
                                channel,
                                account_id,
                                configured_agent_id = configured_agent_id.as_deref().unwrap_or(""),
                                fallback_agent_id = INBOUND_FALLBACK_AGENT_ID,
                                error = %error,
                                "inbound thread creation failed; using fallback agent"
                            );
                            created
                        }
                        Err(fallback_error) => {
                            tracing::warn!(
                                channel,
                                account_id,
                                configured_agent_id = configured_agent_id.as_deref().unwrap_or(""),
                                fallback_agent_id = INBOUND_FALLBACK_AGENT_ID,
                                error = %error,
                                fallback_error = %fallback_error,
                                "inbound thread creation failed with configured and fallback agents"
                            );
                            return new_thread_key();
                        }
                    }
                } else {
                    tracing::warn!(
                        channel,
                        account_id,
                        configured_agent_id = configured_agent_id.as_deref().unwrap_or(""),
                        error = %error,
                        "inbound thread creation failed"
                    );
                    return new_thread_key();
                }
            }
        };

        let binding = self
            .endpoint_binding_from_inbound(
                channel,
                account_id,
                thread_binding_key,
                extra_metadata,
                Some(&display_label),
            )
            .await;
        if let Err(error) = self
            .bind_endpoint_runtime(&thread_id, binding.clone())
            .await
        {
            tracing::warn!(
                channel,
                account_id,
                thread_id = %thread_id,
                error = %error,
                "failed to persist inbound channel binding"
            );
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
            if self.threads.exists_logged(&redirect_key).await {
                tracing::info!(
                    original = %thread_id,
                    redirect = %redirect_key,
                    "auto-recovery redirect applied"
                );
                let binding = self
                    .endpoint_binding_from_inbound(
                        route.channel,
                        route.account_id,
                        route.thread_binding_key,
                        route.extra_metadata,
                        None,
                    )
                    .await;
                self.bind_endpoint_runtime(&redirect_key, binding.clone())
                    .await
                    .ok();
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
        if let Some(thread_id) = self.thread_nav.endpoint_thread_map.get(&key).cloned() {
            if self.threads.exists_logged(&thread_id).await {
                return Some(thread_id);
            }
            self.thread_nav.endpoint_thread_map.remove(&key);
        }

        let mutator = self.endpoint_binding_mutator()?;
        let owner = match mutator.binding_for_endpoint(&key).await {
            Ok(owner) => owner?,
            Err(error) => {
                tracing::warn!(endpoint_key = key, error = %error, "endpoint owner point lookup failed");
                return None;
            }
        };
        let thread_id = owner.thread_id;
        // Existence gate: a stale owner row must not resurrect a deleted
        // thread; a store failure degrades to unresolved (dispatch will
        // surface the outage when it touches the store).
        if !self.threads.exists_logged(&thread_id).await {
            return None;
        }
        self.thread_nav
            .endpoint_thread_map
            .insert(key, thread_id.clone());
        Some(thread_id)
    }
}
