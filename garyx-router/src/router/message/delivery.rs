use super::super::*;
use crate::store::ThreadStoreExt;
use chrono::{DateTime, Utc};
use garyx_models::routing::{infer_delivery_target_id, infer_delivery_target_type};
use garyx_models::thread_logs::{ThreadLogEvent, is_canonical_thread_id};
use serde_json::{Value, json};
use std::collections::HashMap;
use tracing::{debug, warn};

use crate::threads::sync_endpoint_delivery_timestamp;

impl MessageRouter {
    fn delivery_binding_key(ctx: &DeliveryContext) -> &str {
        ctx.thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                let user_id = ctx.user_id.trim();
                (!user_id.is_empty()).then_some(user_id)
            })
            .unwrap_or_else(|| ctx.chat_id.trim())
    }

    fn normalize_thread_target(target: &str) -> &str {
        let trimmed = target.trim();
        if trimmed.starts_with("thread::") {
            return trimmed;
        }
        trimmed.strip_prefix("thread:").unwrap_or(trimmed)
    }

    // ------------------------------------------------------------------
    // Last delivery context
    // ------------------------------------------------------------------

    /// Record the last delivery context for a thread.
    ///
    /// Scheduled cron tasks use this to know where to send responses when
    /// there is no inbound message to reply to.
    pub fn set_last_delivery(&mut self, thread_id: &str, ctx: DeliveryContext) {
        self.delivery_ctx
            .last_delivery
            .insert(thread_id.to_owned(), ctx);
        self.delivery_ctx
            .last_delivery_order
            .retain(|k| k != thread_id);
        self.delivery_ctx
            .last_delivery_order
            .push(thread_id.to_owned());
    }

    /// Record and persist the last delivery context for a thread.
    pub async fn set_last_delivery_with_persistence(
        &mut self,
        thread_id: &str,
        ctx: DeliveryContext,
    ) {
        self.set_last_delivery_with_persistence_mode(thread_id, ctx, true)
            .await;
    }

    pub async fn set_last_delivery_with_known_thread_persistence(
        &mut self,
        thread_id: &str,
        ctx: DeliveryContext,
    ) {
        self.set_last_delivery_with_persistence_mode(thread_id, ctx, false)
            .await;
    }

    async fn set_last_delivery_with_persistence_mode(
        &mut self,
        thread_id: &str,
        ctx: DeliveryContext,
        sync_endpoint_timestamp: bool,
    ) {
        self.set_last_delivery(thread_id, ctx.clone());
        self.persist_delivery_context(thread_id, &ctx, sync_endpoint_timestamp)
            .await;
        let thread_id = ctx
            .thread_id
            .as_deref()
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
        if let Some(thread_id) = thread_id {
            self.record_thread_log(
                ThreadLogEvent::info(thread_id, "delivery", "delivery context updated")
                    .with_field("channel", json!(ctx.channel))
                    .with_field("account_id", json!(ctx.account_id))
                    .with_field("chat_id", json!(ctx.chat_id))
                    .with_field("user_id", json!(ctx.user_id)),
            )
            .await;
        }
    }

    async fn persist_delivery_context(
        &self,
        thread_id: &str,
        ctx: &DeliveryContext,
        sync_endpoint_timestamp: bool,
    ) {
        let Some(mut thread_data) = self.threads.get_logged(thread_id).await else {
            debug!(
                thread_id,
                "Thread missing; skipping delivery-context persistence"
            );
            return;
        };
        let Some(obj) = thread_data.as_object_mut() else {
            warn!(
                thread_id,
                "Thread payload is not an object; skipping delivery-context persistence"
            );
            return;
        };

        let delivery_value = match serde_json::to_value(ctx) {
            Ok(value) => value,
            Err(err) => {
                warn!(
                    thread_id,
                    error = %err,
                    "Failed to serialize delivery context for persistence"
                );
                return;
            }
        };

        obj.insert(
            "last_channel".to_owned(),
            Value::String(ctx.channel.clone()),
        );
        obj.insert("last_to".to_owned(), Value::String(ctx.chat_id.clone()));
        obj.insert(
            "last_account_id".to_owned(),
            Value::String(ctx.account_id.clone()),
        );
        if let Some(thread_id) = &ctx.thread_id {
            obj.insert(
                "last_thread_id".to_owned(),
                Value::String(thread_id.clone()),
            );
        } else {
            obj.remove("last_thread_id");
        }
        // Python compatibility keys.
        obj.insert("lastChannel".to_owned(), Value::String(ctx.channel.clone()));
        obj.insert("lastTo".to_owned(), Value::String(ctx.chat_id.clone()));
        obj.insert(
            "lastAccountId".to_owned(),
            Value::String(ctx.account_id.clone()),
        );
        if let Some(thread_id) = &ctx.thread_id {
            obj.insert("lastThreadId".to_owned(), Value::String(thread_id.clone()));
        } else {
            obj.remove("lastThreadId");
        }
        obj.insert("delivery_context".to_owned(), delivery_value);
        obj.insert(
            "lastUpdatedAt".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );

        self.threads.set_logged(thread_id, thread_data).await;
        if sync_endpoint_timestamp {
            let binding_key = Self::delivery_binding_key(ctx);
            let _ = sync_endpoint_delivery_timestamp(
                &self.threads,
                &ctx.channel,
                &ctx.account_id,
                binding_key,
                Some(&Utc::now().to_rfc3339()),
                thread_id,
            )
            .await;
        }
    }

    /// Get the last delivery context for a thread.
    pub fn get_last_delivery(&self, thread_id: &str) -> Option<&DeliveryContext> {
        self.delivery_ctx.last_delivery.get(thread_id)
    }

    pub fn clear_last_delivery(&mut self, thread_id: &str) {
        self.delivery_ctx.last_delivery.remove(thread_id);
        self.delivery_ctx
            .last_delivery_order
            .retain(|key| key != thread_id);
    }

    pub async fn clear_last_delivery_with_persistence(&mut self, thread_id: &str) {
        self.clear_last_delivery_with_persistence_mode(thread_id, true)
            .await;
    }

    async fn clear_last_delivery_with_persistence_mode(
        &mut self,
        thread_id: &str,
        sync_endpoint_timestamp: bool,
    ) {
        let existing_ctx = self.get_last_delivery(thread_id).cloned().or(self
            .threads
            .get_logged(thread_id)
            .await
            .and_then(|value| Self::extract_delivery_context_from_thread(&value)));
        self.clear_last_delivery(thread_id);

        let Some(mut thread_data) = self.threads.get_logged(thread_id).await else {
            if sync_endpoint_timestamp && let Some(ctx) = existing_ctx {
                let binding_key = Self::delivery_binding_key(&ctx);
                let _ = sync_endpoint_delivery_timestamp(
                    &self.threads,
                    &ctx.channel,
                    &ctx.account_id,
                    binding_key,
                    None,
                    thread_id,
                )
                .await;
            }
            return;
        };
        let Some(obj) = thread_data.as_object_mut() else {
            return;
        };

        for key in [
            "last_channel",
            "last_to",
            "last_account_id",
            "last_thread_id",
            "lastChannel",
            "lastTo",
            "lastAccountId",
            "lastThreadId",
            "delivery_context",
            "lastUpdatedAt",
        ] {
            obj.remove(key);
        }
        obj.insert(
            "updated_at".to_owned(),
            Value::String(Utc::now().to_rfc3339()),
        );
        self.threads.set_logged(thread_id, thread_data).await;
        if sync_endpoint_timestamp && let Some(ctx) = existing_ctx {
            let binding_key = Self::delivery_binding_key(&ctx);
            let _ = sync_endpoint_delivery_timestamp(
                &self.threads,
                &ctx.channel,
                &ctx.account_id,
                binding_key,
                None,
                thread_id,
            )
            .await;
        }
    }

    pub async fn clear_last_delivery_for_chat_with_known_thread_persistence(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
    ) {
        self.clear_last_delivery_for_chat_with_persistence_mode(
            thread_id,
            channel,
            account_id,
            chat_id,
            thread_binding_key,
            false,
        )
        .await;
    }

    async fn clear_last_delivery_for_chat_with_persistence_mode(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
        sync_endpoint_timestamp: bool,
    ) {
        let expected_binding_key = thread_binding_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(chat_id);
        let persisted_ctx = self
            .threads
            .get_logged(thread_id)
            .await
            .and_then(|value| Self::extract_delivery_context_from_thread(&value));
        let should_clear = self
            .get_last_delivery(thread_id)
            .cloned()
            .or(persisted_ctx)
            .is_some_and(|ctx| {
                ctx.channel == channel
                    && ctx.account_id == account_id
                    && ctx.chat_id == chat_id
                    && Self::delivery_binding_key(&ctx) == expected_binding_key
            });
        if should_clear {
            self.clear_last_delivery_with_persistence_mode(thread_id, sync_endpoint_timestamp)
                .await;
        }
    }

    /// Get the most recently updated delivery context.
    pub fn latest_delivery(&self) -> Option<(String, DeliveryContext)> {
        for key in self.delivery_ctx.last_delivery_order.iter().rev() {
            if let Some(ctx) = self.delivery_ctx.last_delivery.get(key) {
                return Some((key.clone(), ctx.clone()));
            }
        }
        None
    }

    /// Resolve a delivery target.
    ///
    /// - `"last"`: most recently updated delivery context.
    /// - `"thread:<key>"`: explicit thread key.
    /// - otherwise: treat the input as an explicit thread key.
    pub fn resolve_delivery_target(&self, target: &str) -> Option<(String, DeliveryContext)> {
        if target.trim().is_empty() || target == "last" {
            return self.latest_delivery();
        }

        let thread_id = Self::normalize_thread_target(target);
        self.get_last_delivery(thread_id)
            .cloned()
            .map(|ctx| (thread_id.to_owned(), ctx))
    }

    /// Resolve a delivery target from persisted thread data only.
    ///
    /// This does not mutate in-memory caches and is intended for
    /// lock-minimized recovery flows in outer services.
    pub async fn resolve_delivery_target_from_store(
        thread_store: Arc<dyn ThreadStore>,
        target: &str,
    ) -> Option<(String, DeliveryContext)> {
        let trimmed = target.trim();
        if !trimmed.is_empty() && trimmed != "last" {
            let thread_id = Self::normalize_thread_target(trimmed);
            if let Some(thread_data) = thread_store.get_logged(thread_id).await
                && let Some(ctx) = Self::extract_delivery_context_from_thread(&thread_data)
            {
                return Some((thread_id.to_owned(), ctx));
            }
            return None;
        }

        // "last" is a recency condition query: answered by the endpoint
        // projection (thread_meta delivery-context rows), never by a scan.
        let rows = match crate::endpoint_projection::channel_endpoint_projection_for(&thread_store)
            .delivery_contexts()
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "failed to read delivery context projection");
                return None;
            }
        };
        let mut best: Option<(String, DeliveryContext, Option<DateTime<Utc>>)> = None;
        for row in rows {
            let Some(ctx) = Self::delivery_context_from_projected_json(&row.context_json) else {
                continue;
            };
            let updated_at = Self::parse_delivery_updated_at(row.updated_at.as_deref());
            let thread_id = row.thread_id;
            match &best {
                None => best = Some((thread_id, ctx, updated_at)),
                Some((best_key, _, best_ts)) => {
                    let better = match (&updated_at, best_ts) {
                        (Some(a), Some(b)) => a > b || (a == b && thread_id > *best_key),
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                        (None, None) => thread_id > *best_key,
                    };
                    if better {
                        best = Some((thread_id, ctx, updated_at));
                    }
                }
            }
        }

        best.map(|(thread_id, ctx, _)| (thread_id, ctx))
    }

    /// Parse and normalize a projected delivery-context JSON payload through
    /// the same extractor the record-field path uses (target type/id
    /// inference, thread-id sanitizing, metadata copy).
    fn delivery_context_from_projected_json(context_json: &str) -> Option<DeliveryContext> {
        let parsed: Value = serde_json::from_str(context_json).ok()?;
        let wrapper = serde_json::json!({ "delivery_context": parsed });
        Self::extract_delivery_context(wrapper.as_object()?)
    }

    fn parse_delivery_updated_at(raw: Option<&str>) -> Option<DateTime<Utc>> {
        raw.and_then(|raw| {
            DateTime::parse_from_rfc3339(raw.trim())
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
    }

    fn sanitize_persisted_delivery_thread_id(
        channel: &str,
        chat_id: &str,
        thread_id: Option<String>,
    ) -> Option<String> {
        let thread_id = thread_id
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())?;
        if channel.eq_ignore_ascii_case("telegram") {
            if thread_id == chat_id {
                return None;
            }
            if thread_id.starts_with("thread::") {
                return None;
            }
            let is_numeric_topic = thread_id.parse::<i64>().is_ok();
            let composite_prefix = format!("{chat_id}_t");
            let is_composite_topic = thread_id
                .strip_prefix(&composite_prefix)
                .is_some_and(|suffix| !suffix.is_empty() && suffix.parse::<i64>().is_ok());
            if !is_numeric_topic && !is_composite_topic {
                return None;
            }
        }
        Some(thread_id)
    }

    pub(crate) fn extract_delivery_context(
        obj: &serde_json::Map<String, Value>,
    ) -> Option<DeliveryContext> {
        if let Some(delivery_obj) = obj.get("delivery_context").and_then(Value::as_object) {
            let channel = Self::value_as_string(delivery_obj.get("channel"))?;
            let account_id =
                Self::value_as_string(delivery_obj.get("account_id")).unwrap_or_default();
            let chat_id = Self::value_as_string(delivery_obj.get("chat_id"))?;
            let user_id = Self::value_as_string(delivery_obj.get("user_id"))
                .unwrap_or_else(|| chat_id.clone());
            let explicit_target_type =
                Self::value_as_string(delivery_obj.get("delivery_target_type"));
            let explicit_target_id = Self::value_as_string(delivery_obj.get("delivery_target_id"));
            let delivery_target_type = infer_delivery_target_type(
                &channel,
                explicit_target_type.as_deref(),
                explicit_target_id.as_deref(),
                &chat_id,
                &user_id,
            );
            let delivery_target_id = infer_delivery_target_id(
                &channel,
                Some(&delivery_target_type),
                explicit_target_id.as_deref(),
                &chat_id,
                &user_id,
            );
            let thread_id = Self::sanitize_persisted_delivery_thread_id(
                &channel,
                &chat_id,
                Self::value_as_string(delivery_obj.get("thread_id")),
            );
            let metadata = delivery_obj
                .get("metadata")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();

            return Some(DeliveryContext {
                channel,
                account_id,
                chat_id,
                user_id,
                delivery_target_type,
                delivery_target_id,
                thread_id,
                metadata,
            });
        }

        let channel = Self::value_as_string(obj.get("last_channel"))
            .or_else(|| Self::value_as_string(obj.get("lastChannel")))?;
        let chat_id = Self::value_as_string(obj.get("last_to"))
            .or_else(|| Self::value_as_string(obj.get("lastTo")))?;
        let account_id = Self::value_as_string(obj.get("last_account_id"))
            .or_else(|| Self::value_as_string(obj.get("lastAccountId")))
            .unwrap_or_default();
        let thread_id = Self::sanitize_persisted_delivery_thread_id(
            &channel,
            &chat_id,
            Self::value_as_string(obj.get("last_thread_id"))
                .or_else(|| Self::value_as_string(obj.get("lastThreadId"))),
        );
        let user_id = Self::value_as_string(obj.get("from_id"))
            .or_else(|| Self::value_as_string(obj.get("user_id")))
            .unwrap_or_else(|| chat_id.clone());
        let delivery_target_type =
            infer_delivery_target_type(&channel, None, None, &chat_id, &user_id);
        let delivery_target_id = infer_delivery_target_id(
            &channel,
            Some(&delivery_target_type),
            None,
            &chat_id,
            &user_id,
        );

        Some(DeliveryContext {
            channel,
            account_id,
            chat_id,
            user_id,
            delivery_target_type,
            delivery_target_id,
            thread_id,
            metadata: HashMap::new(),
        })
    }

    pub(crate) fn extract_delivery_context_from_thread(
        thread_data: &Value,
    ) -> Option<DeliveryContext> {
        thread_data
            .as_object()
            .and_then(Self::extract_delivery_context)
    }

    pub(crate) fn value_as_string(value: Option<&Value>) -> Option<String> {
        match value {
            Some(Value::String(s)) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            }
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
