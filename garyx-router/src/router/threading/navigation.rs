use super::super::*;
use crate::store::ThreadStoreExt;
use chrono::Utc;
use serde_json::Value;

use crate::threads::default_thread_history_state_value;

impl MessageRouter {
    pub async fn latest_assistant_message_text_for_thread(
        &self,
        thread_id: &str,
    ) -> Option<String> {
        if let Some(history) = &self.thread_history
            && let Ok(Some(text)) = history
                .latest_message_text_for_role(thread_id, "assistant")
                .await
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // Binding-context helpers
    // ------------------------------------------------------------------

    pub fn build_binding_context_key(
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> String {
        format!("{channel}::{account_id}::{}", thread_binding_key.trim())
    }

    pub fn build_account_user_key(
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
        group_id: Option<&str>,
    ) -> String {
        let binding_key = if is_group {
            group_id.unwrap_or(from_id)
        } else {
            from_id
        };
        Self::build_binding_context_key(channel, account_id, binding_key)
    }

    // ------------------------------------------------------------------
    // Current-thread lookup / switching
    // ------------------------------------------------------------------

    pub fn get_current_thread_id_for_binding(
        &self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> Option<&str> {
        let binding_context_key =
            Self::build_binding_context_key(channel, account_id, thread_binding_key);
        self.thread_nav
            .binding_thread_map
            .get(&binding_context_key)
            .map(String::as_str)
    }

    pub fn get_current_thread_id_for_account(
        &self,
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
        group_id: Option<&str>,
    ) -> Option<&str> {
        let binding_key = if is_group {
            group_id.unwrap_or(from_id)
        } else {
            from_id
        };
        self.get_current_thread_id_for_binding(channel, account_id, binding_key)
    }

    pub fn reset_thread_for_binding(
        &mut self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> bool {
        let binding_context_key =
            Self::build_binding_context_key(channel, account_id, thread_binding_key);
        self.thread_nav
            .binding_thread_map
            .remove(&binding_context_key)
            .is_some()
    }

    /// Set the current thread for one binding context.
    pub fn switch_to_thread(&mut self, binding_context_key: &str, thread_id: &str) {
        self.thread_nav
            .binding_thread_map
            .insert(binding_context_key.to_owned(), thread_id.to_owned());
    }

    /// Remove every in-memory current-thread reference to a deleted thread.
    pub fn clear_thread_references(&mut self, thread_id: &str) {
        self.thread_nav
            .binding_thread_map
            .retain(|_, current| current != thread_id);
    }

    /// Incrementally remove every in-memory index entry pointing at one thread.
    ///
    /// Write-path replacement for the full `rebuild_thread_indexes` scan:
    /// deleting or archiving a thread only needs that thread's own references
    /// cleared. The full rebuild stays a startup-reconciliation repair and
    /// must not run on request paths (it stats every known thread on disk).
    pub fn purge_thread_from_indexes(&mut self, thread_id: &str) {
        self.clear_thread_references(thread_id);
        self.thread_nav
            .endpoint_thread_map
            .retain(|_, current| current != thread_id);
    }

    /// Incrementally drop one endpoint's binding and current-thread entries.
    pub fn purge_endpoint_binding(&mut self, endpoint_key: &str) {
        self.clear_binding_thread_context(endpoint_key);
    }

    pub(crate) fn clear_binding_thread_context(&mut self, binding_context_key: &str) {
        self.thread_nav
            .binding_thread_map
            .remove(binding_context_key);
        self.thread_nav
            .endpoint_thread_map
            .remove(binding_context_key);
    }

    pub fn clear_account_thread_context(
        &mut self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) {
        let binding_context_key =
            Self::build_binding_context_key(channel, account_id, thread_binding_key);
        self.clear_binding_thread_context(&binding_context_key);
    }

    /// Ensure a thread record exists with baseline metadata.
    pub async fn ensure_thread_entry(
        &self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
        label: Option<&str>,
    ) {
        let now = Utc::now().to_rfc3339();
        let mut thread_data = self
            .threads
            .get_logged(thread_id)
            .await
            .unwrap_or_else(|| serde_json::json!({}));
        let Some(obj) = thread_data.as_object_mut() else {
            return;
        };

        if obj.get("thread_id").and_then(Value::as_str).is_none() {
            obj.insert("thread_id".to_owned(), Value::String(thread_id.to_owned()));
        }
        if obj.get("channel").and_then(Value::as_str).is_none() {
            obj.insert("channel".to_owned(), Value::String(channel.to_owned()));
        }
        if obj.get("account_id").and_then(Value::as_str).is_none() {
            obj.insert(
                "account_id".to_owned(),
                Value::String(account_id.to_owned()),
            );
        }
        if obj
            .get("thread_binding_key")
            .and_then(Value::as_str)
            .is_none()
        {
            obj.insert(
                "thread_binding_key".to_owned(),
                Value::String(thread_binding_key.to_owned()),
            );
        }
        if obj.get("created_at").and_then(Value::as_str).is_none() {
            obj.insert("created_at".to_owned(), Value::String(now.clone()));
        }
        if obj.get("message_count").and_then(Value::as_i64).is_none() {
            obj.insert(
                "message_count".to_owned(),
                Value::Number(serde_json::Number::from(0)),
            );
        }
        if !obj.contains_key("history") {
            obj.insert("history".to_owned(), default_thread_history_state_value());
        }
        if !obj.contains_key("context") {
            obj.insert("context".to_owned(), Value::Object(serde_json::Map::new()));
        }
        if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
            obj.insert("label".to_owned(), Value::String(label.to_owned()));
        }

        obj.insert("updated_at".to_owned(), Value::String(now));
        self.threads.set_logged(thread_id, thread_data).await;
    }
}
