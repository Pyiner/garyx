use super::super::*;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashSet;

use crate::threads::{
    bindings_from_value, default_thread_history_state_value, is_thread_key, label_from_value,
};

impl MessageRouter {
    fn thread_matches_binding(
        thread_id: &str,
        thread_data: &Value,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> bool {
        if thread_id.starts_with("meta::") {
            return false;
        }

        if bindings_from_value(thread_data).into_iter().any(|binding| {
            binding.channel == channel
                && binding.account_id == account_id
                && binding.binding_key == thread_binding_key
        }) {
            return true;
        }

        let Some(obj) = thread_data.as_object() else {
            return false;
        };
        let legacy_binding_key = obj
            .get("thread_binding_key")
            .or_else(|| obj.get("from_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if legacy_binding_key != Some(thread_binding_key) {
            return false;
        }

        let legacy_channel_matches = obj
            .get("channel")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_none_or(|value| value == channel);
        let has_legacy_account =
            obj.get("account_id").is_some() || obj.get("origin_account_id").is_some();
        let legacy_account_matches = obj
            .get("account_id")
            .or_else(|| obj.get("origin_account_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .map(|value| value == account_id)
            .unwrap_or(false);
        let legacy_key_matches = thread_id.starts_with(&format!("{account_id}::"));

        legacy_channel_matches
            && if has_legacy_account {
                legacy_account_matches
            } else {
                legacy_key_matches
            }
    }

    pub async fn latest_message_text_for_thread(&self, thread_id: &str) -> Option<String> {
        if let Some(history) = &self.thread_history {
            if let Ok(Some(text)) = history.latest_message_text(thread_id).await {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
        None
    }

    pub async fn latest_assistant_message_text_for_thread(
        &self,
        thread_id: &str,
    ) -> Option<String> {
        if let Some(history) = &self.thread_history {
            if let Ok(Some(text)) = history
                .latest_message_text_for_role(thread_id, "assistant")
                .await
            {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
        None
    }

    fn summarize_thread_list_text(raw: &str, max_chars: usize) -> Option<String> {
        let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let trimmed = collapsed.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut summary = String::new();
        let mut count = 0usize;
        for ch in trimmed.chars() {
            if count >= max_chars {
                break;
            }
            summary.push(ch);
            count += 1;
        }

        if summary.is_empty() {
            return None;
        }
        if trimmed.chars().count() > max_chars {
            summary.push('…');
        }
        Some(summary)
    }

    async fn fallback_thread_list_label(
        &self,
        thread_id: &str,
        thread_data: &Value,
    ) -> Option<String> {
        if let Some(label) = label_from_value(thread_data) {
            return Some(label);
        }

        if let Some(messages) = thread_data.get("messages").and_then(Value::as_array) {
            for message in messages.iter().rev() {
                let content = message
                    .get("content")
                    .or_else(|| message.get("text"))
                    .and_then(Value::as_str);
                if let Some(summary) =
                    content.and_then(|value| Self::summarize_thread_list_text(value, 48))
                {
                    return Some(summary);
                }
            }
        }

        if let Some(history) = &self.thread_history {
            if let Ok(Some(text)) = history.latest_message_text(thread_id).await {
                if let Some(summary) = Self::summarize_thread_list_text(&text, 48) {
                    return Some(summary);
                }
            }
        }

        if is_thread_key(thread_id) {
            let suffix = thread_id.trim_start_matches("thread::");
            let short = suffix
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .take(8)
                .collect::<String>();
            if !short.is_empty() {
                return Some(format!("Thread {short}"));
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
    // Thread lookup / switching
    // ------------------------------------------------------------------

    pub fn get_current_thread_id_for_binding(
        &self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> Option<&str> {
        let binding_context_key =
            Self::build_binding_context_key(channel, account_id, thread_binding_key);
        if !self
            .thread_nav
            .binding_thread_history
            .contains_key(&binding_context_key)
        {
            return None;
        }
        self.thread_nav
            .binding_thread_map
            .get(&binding_context_key)
            .map(|s| s.as_str())
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

    /// Switch a binding context to a specific thread and record it in history.
    pub fn switch_to_thread(&mut self, binding_context_key: &str, thread_id: &str) {
        // Initialize history if needed.
        if !self
            .thread_nav
            .binding_thread_history
            .contains_key(binding_context_key)
        {
            self.thread_nav
                .binding_thread_history
                .insert(binding_context_key.to_owned(), Vec::new());
            self.thread_nav
                .binding_thread_index
                .insert(binding_context_key.to_owned(), 0);
        }

        let Some(history) = self
            .thread_nav
            .binding_thread_history
            .get_mut(binding_context_key)
        else {
            return;
        };

        // If we are not at the end of history, truncate forward history.
        let current_idx = *self
            .thread_nav
            .binding_thread_index
            .get(binding_context_key)
            .unwrap_or(&0);
        if !history.is_empty() && current_idx < history.len().saturating_sub(1) {
            history.truncate(current_idx + 1);
        }

        // Add to history if different from last entry.
        if history.last().is_none_or(|last| last != thread_id) {
            history.push(thread_id.to_owned());
            if history.len() > MAX_HISTORY {
                history.remove(0);
            }
        }

        // Update current index to end of history.
        let new_idx = history.len().saturating_sub(1);
        self.thread_nav
            .binding_thread_index
            .insert(binding_context_key.to_owned(), new_idx);

        // Update current thread.
        self.thread_nav
            .binding_thread_map
            .insert(binding_context_key.to_owned(), thread_id.to_owned());
    }

    /// Navigate to previous (`direction = -1`) or next (`direction = 1`) thread.
    ///
    /// Returns the new thread id if navigation succeeded, or `None` if
    /// at the boundary of history.
    pub fn navigate_thread(&mut self, binding_context_key: &str, direction: i32) -> Option<String> {
        let history = self
            .thread_nav
            .binding_thread_history
            .get(binding_context_key)?;
        if history.is_empty() {
            return None;
        }

        let current_idx = *self
            .thread_nav
            .binding_thread_index
            .get(binding_context_key)
            .unwrap_or(&0);
        let new_idx = if direction < 0 {
            if current_idx == 0 {
                return None; // already at oldest
            }
            current_idx - 1
        } else {
            let next = current_idx + 1;
            if next >= history.len() {
                return None; // already at newest
            }
            next
        };

        let target_key = history[new_idx].clone();
        self.thread_nav
            .binding_thread_map
            .insert(binding_context_key.to_owned(), target_key.clone());
        self.thread_nav
            .binding_thread_index
            .insert(binding_context_key.to_owned(), new_idx);

        Some(target_key)
    }

    /// Remove all in-memory switched-thread references to a deleted thread.
    pub fn clear_thread_references(&mut self, thread_id: &str) {
        self.thread_nav
            .binding_thread_map
            .retain(|_, current| current != thread_id);

        let binding_context_keys = self
            .thread_nav
            .binding_thread_history
            .keys()
            .cloned()
            .collect::<Vec<_>>();

        for binding_context_key in binding_context_keys {
            let Some(history) = self
                .thread_nav
                .binding_thread_history
                .get_mut(&binding_context_key)
            else {
                continue;
            };
            let current_idx = *self
                .thread_nav
                .binding_thread_index
                .get(&binding_context_key)
                .unwrap_or(&0);
            let removed_before_or_at_current = history
                .iter()
                .take(current_idx.saturating_add(1))
                .filter(|entry| entry.as_str() == thread_id)
                .count();
            history.retain(|entry| entry != thread_id);

            if history.is_empty() {
                self.thread_nav
                    .binding_thread_history
                    .remove(&binding_context_key);
                self.thread_nav
                    .binding_thread_index
                    .remove(&binding_context_key);
                self.thread_nav
                    .binding_thread_map
                    .remove(&binding_context_key);
                continue;
            }

            let adjusted_idx = current_idx
                .saturating_sub(removed_before_or_at_current)
                .min(history.len().saturating_sub(1));
            self.thread_nav
                .binding_thread_index
                .insert(binding_context_key.clone(), adjusted_idx);

            let current_thread = history[adjusted_idx].clone();
            self.thread_nav
                .binding_thread_map
                .entry(binding_context_key)
                .or_insert(current_thread);
        }
    }

    pub(crate) fn clear_binding_thread_context(&mut self, binding_context_key: &str) {
        self.thread_nav
            .binding_thread_map
            .remove(binding_context_key);
        self.thread_nav
            .binding_thread_history
            .remove(binding_context_key);
        self.thread_nav
            .binding_thread_index
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

    async fn list_binding_threads(
        &self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> Vec<(String, String)> {
        let mut user_threads = Vec::new();

        for key in self.threads.list_keys(None).await {
            let Some(thread_data) = self.threads.get(&key).await else {
                continue;
            };
            let Some(obj) = thread_data.as_object() else {
                continue;
            };
            if !Self::thread_matches_binding(
                &key,
                &thread_data,
                channel,
                account_id,
                thread_binding_key,
            ) {
                continue;
            }

            let sort_key = Self::value_as_string(obj.get("updated_at"))
                .or_else(|| Self::value_as_string(obj.get("created_at")))
                .unwrap_or_default();
            user_threads.push((key, sort_key));
        }

        user_threads.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        user_threads
    }

    async fn ensure_thread_history(
        &mut self,
        binding_context_key: &str,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) {
        let current_key = self
            .current_canonical_thread_for_binding(channel, account_id, thread_binding_key)
            .await;

        if let Some(history) = self
            .thread_nav
            .binding_thread_history
            .get(binding_context_key)
        {
            let current_idx = *self
                .thread_nav
                .binding_thread_index
                .get(binding_context_key)
                .unwrap_or(&usize::MAX);
            if !history.is_empty()
                && current_idx < history.len()
                && current_key
                    .as_ref()
                    .is_none_or(|current| history.get(current_idx) == Some(current))
            {
                return;
            }
        }

        let threads = self
            .list_binding_threads(channel, account_id, thread_binding_key)
            .await;
        if threads.is_empty() {
            return;
        }

        let mut rebuilt = Vec::new();
        let mut seen = HashSet::new();
        for (thread_id, _) in threads {
            if seen.insert(thread_id.clone()) {
                rebuilt.push(thread_id);
            }
        }
        if rebuilt.is_empty() {
            return;
        }

        if let Some(current) = current_key.as_ref() {
            if !seen.contains(current) {
                rebuilt.push(current.clone());
            }
        }

        self.thread_nav
            .binding_thread_history
            .insert(binding_context_key.to_owned(), rebuilt.clone());
        let idx = if let Some(current) = current_key {
            rebuilt
                .iter()
                .position(|item| item == &current)
                .unwrap_or(rebuilt.len().saturating_sub(1))
        } else {
            rebuilt.len().saturating_sub(1)
        };
        self.thread_nav
            .binding_thread_index
            .insert(binding_context_key.to_owned(), idx);
        if let Some(current_thread) = rebuilt.get(idx).cloned() {
            self.thread_nav
                .binding_thread_map
                .insert(binding_context_key.to_owned(), current_thread);
        }
    }

    /// Navigate threads after rebuilding history from persisted threads when needed.
    ///
    /// This mirrors Python behavior where `/threadprev` and `/threadnext`
    /// still work after router restart.
    pub(crate) async fn navigate_thread_with_rebuild(
        &mut self,
        binding_context_key: &str,
        navigation: NavigationContext<'_>,
        direction: i32,
    ) -> Option<String> {
        self.ensure_thread_history(
            binding_context_key,
            navigation.channel,
            navigation.account_id,
            navigation.thread_binding_key,
        )
        .await;
        self.navigate_thread(binding_context_key, direction)
    }

    /// List persisted threads for a user scoped by account.
    ///
    /// Returns newest-first, matching Python command UX.
    pub async fn list_user_threads_for_account(
        &self,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> Vec<ThreadListEntry> {
        let mut entries: Vec<ThreadListEntry> = Vec::new();

        for key in self.threads.list_keys(None).await {
            let Some(thread_data) = self.threads.get(&key).await else {
                continue;
            };
            let Some(obj) = thread_data.as_object() else {
                continue;
            };
            if !Self::thread_matches_binding(
                &key,
                &thread_data,
                channel,
                account_id,
                thread_binding_key,
            ) {
                continue;
            }

            let label = self.fallback_thread_list_label(&key, &thread_data).await;
            let updated_at = Self::value_as_string(obj.get("updated_at"))
                .or_else(|| Self::value_as_string(obj.get("created_at")));
            entries.push(ThreadListEntry {
                thread_id: key,
                label,
                updated_at,
            });
        }

        entries.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.thread_id.cmp(&b.thread_id))
        });
        entries
    }

    /// Ensure a thread record exists with baseline metadata.
    ///
    /// Used by channel-native `/new` commands so the newly created thread
    /// is immediately visible in `/threads` before any user message is sent.
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
            .get(thread_id)
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
        if !obj.contains_key("messages") {
            obj.insert("messages".to_owned(), Value::Array(Vec::new()));
        }
        if !obj.contains_key("history") {
            obj.insert("history".to_owned(), default_thread_history_state_value());
        }
        if !obj.contains_key("context") {
            obj.insert("context".to_owned(), Value::Object(serde_json::Map::new()));
        }
        if let Some(label) = label.map(str::trim).filter(|s| !s.is_empty()) {
            obj.insert("label".to_owned(), Value::String(label.to_owned()));
        }

        obj.insert("updated_at".to_owned(), Value::String(now));
        self.threads.set(thread_id, thread_data).await;
    }
}
