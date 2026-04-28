use std::collections::HashMap;

use chrono::Utc;
use serde_json::Value;

use super::super::*;

impl MessageRouter {
    pub(in crate::router) fn reply_chat_id(
        extra_metadata: &HashMap<String, Value>,
    ) -> Option<String> {
        extra_metadata
            .get("chat_id")
            .and_then(|value| match value {
                Value::String(value) => Some(value.trim().to_owned()),
                Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .filter(|value| !value.is_empty())
    }

    pub(super) async fn trim_thread_history(&self, thread_id: &str, limit: usize) -> usize {
        if limit == 0 {
            return 0;
        }

        let Some(mut thread_data) = self.threads.get(thread_id).await else {
            return 0;
        };
        let Some(obj) = thread_data.as_object_mut() else {
            return 0;
        };
        let Some(messages) = obj.get_mut("messages").and_then(|v| v.as_array_mut()) else {
            return 0;
        };

        if messages.len() <= limit {
            return 0;
        }

        let trimmed = messages.len() - limit;
        messages.drain(..trimmed);
        self.threads.set(thread_id, thread_data).await;
        trimmed
    }

    pub(super) async fn backfill_thread_context_if_missing(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
    ) {
        let Some(mut data) = self.threads.get(thread_id).await else {
            return;
        };
        let Some(obj) = data.as_object_mut() else {
            return;
        };

        let mut updated = false;
        if obj.get("channel").and_then(|v| v.as_str()).is_none() {
            obj.insert("channel".to_owned(), Value::String(channel.to_owned()));
            updated = true;
        }
        if obj.get("account_id").and_then(|v| v.as_str()).is_none() {
            obj.insert(
                "account_id".to_owned(),
                Value::String(account_id.to_owned()),
            );
            updated = true;
        }
        if obj.get("from_id").and_then(|v| v.as_str()).is_none() {
            obj.insert("from_id".to_owned(), Value::String(from_id.to_owned()));
            updated = true;
        }
        if obj.get("is_group").is_none() {
            obj.insert("is_group".to_owned(), Value::Bool(is_group));
            updated = true;
        }

        if updated {
            obj.insert(
                "updated_at".to_owned(),
                Value::String(Utc::now().to_rfc3339()),
            );
            self.threads.set(thread_id, data).await;
        }
    }

    pub(super) fn resolve_delivery_chat_id(
        extra_metadata: &HashMap<String, Value>,
        from_id: &str,
    ) -> String {
        extra_metadata
            .get("chat_id")
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| from_id.to_owned())
    }
}
