use std::collections::{HashMap, HashSet};

use serde_json::Value;
use tracing::{debug, info};

use crate::store::ThreadStore;

/// Record of an outbound message used for reply routing.
#[derive(Debug, Clone)]
pub struct OutboundMessageRecord {
    pub thread_id: String,
    pub channel: String,
    pub account_id: String,
    pub chat_id: String,
    pub thread_binding_key: Option<String>,
    pub message_id: String,
}

/// Composite key for the routing index: `(channel, account_id, chat_id, thread_binding_key, message_id)`.
type IndexKey = (String, String, String, String, String);

/// In-memory index for routing replies back to threads.
///
/// When the bot sends a message the message-id is recorded here.
/// When a user replies to that message we look up the id to find the thread.
pub struct MessageRoutingIndex {
    /// `(channel, account_id, chat_id, thread_binding_key, message_id)` -> `thread_id`
    index: HashMap<IndexKey, String>,
    /// Reverse index: `thread_id` -> set of index keys
    reverse_index: HashMap<String, HashSet<IndexKey>>,
    /// Maximum messages tracked per thread to prevent unbounded growth.
    max_messages_per_thread: usize,
}

impl MessageRoutingIndex {
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
            reverse_index: HashMap::new(),
            max_messages_per_thread: 100,
        }
    }

    /// Record an outbound message for reply routing.
    pub fn record_outbound(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
        message_id: &str,
    ) {
        let normalized = Self::normalize_message_id(message_id);
        if normalized.is_empty() {
            debug!(
                channel,
                account_id, "Skipping outbound routing record with empty message_id"
            );
            return;
        }

        let key: IndexKey = (
            channel.to_owned(),
            account_id.to_owned(),
            Self::normalize_chat_id(chat_id),
            Self::normalize_thread_binding_key(thread_binding_key),
            normalized.clone(),
        );
        self.index.insert(key.clone(), thread_id.to_owned());

        let entries = self.reverse_index.entry(thread_id.to_owned()).or_default();
        entries.insert(key);

        if entries.len() > self.max_messages_per_thread {
            self.prune_oldest_for_thread(thread_id);
        }

        debug!(
            "Recorded outbound message: {}:{}:{} -> {}",
            channel, account_id, normalized, thread_id
        );
    }

    /// Look up the thread id for a reply.
    pub fn lookup_thread_for_chat(
        &self,
        channel: &str,
        account_id: &str,
        chat_id: Option<&str>,
        thread_binding_key: Option<&str>,
        reply_to_message_id: &str,
    ) -> Option<&str> {
        let normalized = Self::normalize_message_id(reply_to_message_id);
        if normalized.is_empty() {
            return None;
        }

        let normalized_chat_id = chat_id.map(Self::normalize_chat_id);
        let normalized_thread_binding_key = Self::normalize_thread_binding_key(thread_binding_key);
        let thread_id = normalized_chat_id
            .as_ref()
            .and_then(|chat_id| {
                let scoped_key: IndexKey = (
                    channel.to_owned(),
                    account_id.to_owned(),
                    chat_id.clone(),
                    normalized_thread_binding_key.clone(),
                    normalized.clone(),
                );
                self.index.get(&scoped_key).map(|s| s.as_str()).or_else(|| {
                    let unscoped_key: IndexKey = (
                        channel.to_owned(),
                        account_id.to_owned(),
                        chat_id.clone(),
                        String::new(),
                        normalized.clone(),
                    );
                    self.index.get(&unscoped_key).map(|s| s.as_str())
                })
            })
            .or_else(|| {
                let legacy_key: IndexKey = (
                    channel.to_owned(),
                    account_id.to_owned(),
                    String::new(),
                    String::new(),
                    normalized.clone(),
                );
                self.index.get(&legacy_key).map(|s| s.as_str())
            });

        if let Some(sk) = thread_id {
            debug!(
                "Reply routing: {}:{}:{}:{}:{} -> {}",
                channel,
                account_id,
                normalized_chat_id.as_deref().unwrap_or(""),
                normalized_thread_binding_key,
                reply_to_message_id,
                sk
            );
        }
        thread_id
    }

    pub fn lookup_thread(
        &self,
        channel: &str,
        account_id: &str,
        reply_to_message_id: &str,
    ) -> Option<&str> {
        self.lookup_thread_for_chat(channel, account_id, None, None, reply_to_message_id)
    }

    /// Clear all entries for a thread.
    pub fn clear_thread(&mut self, thread_id: &str) {
        if let Some(keys) = self.reverse_index.remove(thread_id) {
            for key in &keys {
                self.index.remove(key);
            }
            debug!("Cleared routing index for thread {}", thread_id);
        }
    }

    pub fn clear_thread_chat(
        &mut self,
        thread_id: &str,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        thread_binding_key: Option<&str>,
    ) {
        let normalized_chat_id = Self::normalize_chat_id(chat_id);
        let normalized_thread_binding_key = Self::normalize_thread_binding_key(thread_binding_key);
        let clear_scoped_thread = !normalized_thread_binding_key.is_empty();
        let Some(keys) = self.reverse_index.get_mut(thread_id) else {
            return;
        };

        let to_remove: Vec<IndexKey> = keys
            .iter()
            .filter(
                |(key_channel, key_account_id, key_chat_id, key_thread_binding_key, _)| {
                    let matches_chat = if clear_scoped_thread {
                        key_chat_id == &normalized_chat_id
                    } else {
                        key_chat_id == &normalized_chat_id || key_chat_id.is_empty()
                    };
                    let matches_scope = if clear_scoped_thread {
                        key_thread_binding_key == &normalized_thread_binding_key
                    } else {
                        key_thread_binding_key.is_empty()
                    };
                    key_channel == channel
                        && key_account_id == account_id
                        && matches_chat
                        && matches_scope
                },
            )
            .cloned()
            .collect();

        for key in &to_remove {
            self.index.remove(key);
            keys.remove(key);
        }

        if keys.is_empty() {
            self.reverse_index.remove(thread_id);
        }

        if !to_remove.is_empty() {
            debug!(
                thread_id,
                channel,
                account_id,
                chat_id = normalized_chat_id,
                thread_binding_key = normalized_thread_binding_key,
                removed = to_remove.len(),
                "Cleared routing index entries for detached endpoint"
            );
        }
    }

    /// Clear all entries.
    pub fn clear_all(&mut self) {
        self.index.clear();
        self.reverse_index.clear();
        info!("Cleared all message routing entries");
    }

    /// Rebuild index from a thread store on startup.
    pub async fn rebuild_from_store(
        &mut self,
        thread_store: &dyn ThreadStore,
        channel: &str,
    ) -> usize {
        let mut count: usize = 0;
        let keys = thread_store.list_keys(None).await;

        for thread_id in &keys {
            let Some(session_data) = thread_store.get(thread_id).await else {
                continue;
            };

            let Some(outbound_ids) = session_data.get("outbound_message_ids") else {
                continue;
            };
            let Some(records) = outbound_ids.as_array() else {
                continue;
            };

            for record in records {
                if let Some(obj) = record.as_object() {
                    let rec_channel = obj
                        .get("channel")
                        .and_then(Value::as_str)
                        .unwrap_or(channel);
                    let rec_account = obj.get("account_id").and_then(Value::as_str).unwrap_or("");
                    let rec_chat_id = obj.get("chat_id").and_then(Value::as_str).unwrap_or("");
                    let rec_thread_binding_key = obj
                        .get("thread_binding_key")
                        .or_else(|| obj.get("thread_scope"))
                        .and_then(Value::as_str);
                    if let Some(rec_msg_id) = obj.get("message_id").and_then(Value::as_str) {
                        self.record_outbound(
                            thread_id,
                            rec_channel,
                            rec_account,
                            rec_chat_id,
                            rec_thread_binding_key,
                            rec_msg_id,
                        );
                        count += 1;
                    }
                }
            }
        }

        info!(
            "Rebuilt message routing index: {} entries from {} threads",
            count,
            keys.len()
        );
        count
    }

    /// Index statistics.
    pub fn get_stats(&self) -> MessageRoutingStats {
        MessageRoutingStats {
            total_entries: self.index.len(),
            threads_tracked: self.reverse_index.len(),
        }
    }

    // -- private helpers --

    fn normalize_message_id(message_id: &str) -> String {
        message_id.trim().to_owned()
    }

    fn normalize_chat_id(chat_id: &str) -> String {
        chat_id.trim().to_owned()
    }

    fn normalize_thread_binding_key(thread_binding_key: Option<&str>) -> String {
        thread_binding_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_owned()
    }

    fn prune_oldest_for_thread(&mut self, thread_id: &str) {
        let Some(entries) = self.reverse_index.get_mut(thread_id) else {
            return;
        };

        // Remove roughly half of the entries (no timestamp ordering, same as Python).
        let to_remove: Vec<IndexKey> = entries.iter().take(entries.len() / 2).cloned().collect();
        for key in &to_remove {
            self.index.remove(key);
            entries.remove(key);
        }

        debug!(
            "Pruned {} old entries for thread {}",
            to_remove.len(),
            thread_id
        );
    }
}

impl Default for MessageRoutingIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics returned by [`MessageRoutingIndex::get_stats`].
#[derive(Debug, Clone)]
pub struct MessageRoutingStats {
    pub total_entries: usize,
    pub threads_tracked: usize,
}

#[cfg(test)]
mod tests;
