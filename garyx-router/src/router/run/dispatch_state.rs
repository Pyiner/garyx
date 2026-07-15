use crate::store::ThreadStoreExt;
use std::collections::HashMap;

use chrono::Utc;
use serde_json::Value;

use super::super::*;

impl MessageRouter {
    pub(super) async fn backfill_thread_context_if_missing(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
    ) {
        let Some(mut data) = self.threads.get_logged(thread_id).await else {
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
            self.threads.set_logged(thread_id, data).await;
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
