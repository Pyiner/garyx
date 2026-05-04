use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use garyx_models::{
    BotThreadProblemSummary, MessageLedgerEvent, MessageLedgerRecord, MessageTerminalReason,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::file_store::thread_storage_file_name;

#[derive(Debug, thiserror::Error)]
pub enum MessageLedgerError {
    #[error("message ledger io error: {0}")]
    Io(String),
    #[error("message ledger parse error: {0}")]
    Parse(String),
}

#[derive(Debug)]
enum MessageLedgerMode {
    File {
        root_dir: PathBuf,
        io_lock: Mutex<()>,
    },
    Memory {
        events: Mutex<Vec<MessageLedgerEvent>>,
    },
}

#[derive(Debug)]
pub struct MessageLedgerStore {
    mode: MessageLedgerMode,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MessageLedgerLine {
    Session { version: u32, created_at: String },
    Event(Box<MessageLedgerEvent>),
}

impl MessageLedgerStore {
    pub async fn file(root_dir: impl AsRef<Path>) -> Result<Self, MessageLedgerError> {
        tokio::fs::create_dir_all(root_dir.as_ref())
            .await
            .map_err(|error| MessageLedgerError::Io(error.to_string()))?;
        Ok(Self {
            mode: MessageLedgerMode::File {
                root_dir: root_dir.as_ref().to_path_buf(),
                io_lock: Mutex::new(()),
            },
        })
    }

    pub fn memory() -> Self {
        Self {
            mode: MessageLedgerMode::Memory {
                events: Mutex::new(Vec::new()),
            },
        }
    }

    pub fn events_path(&self) -> Option<PathBuf> {
        match &self.mode {
            MessageLedgerMode::File { root_dir, .. } => {
                Some(root_dir.join(thread_storage_file_name("message_ledger_events", "jsonl")))
            }
            MessageLedgerMode::Memory { .. } => None,
        }
    }

    pub async fn append_event(&self, event: MessageLedgerEvent) -> Result<(), MessageLedgerError> {
        match &self.mode {
            MessageLedgerMode::File { io_lock, .. } => {
                let _guard = io_lock.lock().await;
                let path = self.events_path().ok_or_else(|| {
                    MessageLedgerError::Io("missing file-backed message ledger path".to_owned())
                })?;
                let existed = path.exists();
                let existing_len = if existed {
                    tokio::fs::metadata(&path)
                        .await
                        .map(|meta| meta.len())
                        .unwrap_or(0)
                } else {
                    0
                };

                let mut file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .await
                    .map_err(|error| MessageLedgerError::Io(error.to_string()))?;

                if !existed || existing_len == 0 {
                    let header = serde_json::to_string(&MessageLedgerLine::Session {
                        version: 1,
                        created_at: chrono::Utc::now().to_rfc3339(),
                    })
                    .map_err(|error| MessageLedgerError::Parse(error.to_string()))?;
                    file.write_all(header.as_bytes())
                        .await
                        .map_err(|error| MessageLedgerError::Io(error.to_string()))?;
                    file.write_all(b"\n")
                        .await
                        .map_err(|error| MessageLedgerError::Io(error.to_string()))?;
                }

                let encoded = serde_json::to_string(&MessageLedgerLine::Event(Box::new(event)))
                    .map_err(|error| MessageLedgerError::Parse(error.to_string()))?;
                file.write_all(encoded.as_bytes())
                    .await
                    .map_err(|error| MessageLedgerError::Io(error.to_string()))?;
                file.write_all(b"\n")
                    .await
                    .map_err(|error| MessageLedgerError::Io(error.to_string()))?;
                file.flush()
                    .await
                    .map_err(|error| MessageLedgerError::Io(error.to_string()))
            }
            MessageLedgerMode::Memory { events } => {
                events.lock().await.push(event);
                Ok(())
            }
        }
    }

    pub async fn list_events_for_thread(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<MessageLedgerEvent>, MessageLedgerError> {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() {
            return Ok(Vec::new());
        }
        let events = self.read_all_events().await?;
        Ok(limit_tail(
            events
                .into_iter()
                .filter(|event| {
                    event
                        .thread_id
                        .as_deref()
                        .is_some_and(|value| value == thread_id)
                })
                .collect(),
            limit,
        ))
    }

    pub async fn list_events_for_bot(
        &self,
        bot_id: &str,
        limit: usize,
    ) -> Result<Vec<MessageLedgerEvent>, MessageLedgerError> {
        let bot_id = bot_id.trim();
        if bot_id.is_empty() {
            return Ok(Vec::new());
        }
        let events = self.read_all_events().await?;
        Ok(limit_tail(
            events
                .into_iter()
                .filter(|event| event.bot_id == bot_id)
                .collect(),
            limit,
        ))
    }

    pub async fn records_for_thread(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<MessageLedgerRecord>, MessageLedgerError> {
        let events = self.list_events_for_thread(thread_id, usize::MAX).await?;
        Ok(limit_tail(fold_records(events), limit))
    }

    pub async fn records_for_bot(
        &self,
        bot_id: &str,
        limit: usize,
    ) -> Result<Vec<MessageLedgerRecord>, MessageLedgerError> {
        let events = self.list_events_for_bot(bot_id, usize::MAX).await?;
        Ok(limit_tail(fold_records(events), limit))
    }

    pub async fn problem_threads_for_bot(
        &self,
        bot_id: &str,
        limit: usize,
    ) -> Result<Vec<BotThreadProblemSummary>, MessageLedgerError> {
        let records = self.records_for_bot(bot_id, usize::MAX).await?;
        let mut by_thread: HashMap<String, BotThreadProblemSummary> = HashMap::new();
        for record in records.into_iter().filter(|record| record.is_problem()) {
            let Some(thread_id) = record.thread_id.clone() else {
                continue;
            };
            let entry =
                by_thread
                    .entry(thread_id.clone())
                    .or_insert_with(|| BotThreadProblemSummary {
                        bot_id: record.bot_id.clone(),
                        thread_id: thread_id.clone(),
                        last_status: record.status,
                        last_event_at: record.updated_at.clone(),
                        terminal_reason: record.terminal_reason,
                        last_text_excerpt: record.text_excerpt.clone(),
                        message_count: 0,
                    });
            entry.message_count += 1;
            if record.updated_at >= entry.last_event_at {
                entry.last_event_at = record.updated_at.clone();
                entry.last_status = record.status;
                entry.terminal_reason = record.terminal_reason;
                entry.last_text_excerpt = record.text_excerpt.clone();
            }
        }

        let mut summaries: Vec<_> = by_thread.into_values().collect();
        summaries.sort_by(|left, right| right.last_event_at.cmp(&left.last_event_at));
        summaries.truncate(limit);
        Ok(summaries)
    }

    async fn read_all_events(&self) -> Result<Vec<MessageLedgerEvent>, MessageLedgerError> {
        match &self.mode {
            MessageLedgerMode::File { io_lock, .. } => {
                let _guard = io_lock.lock().await;
                let Some(path) = self.events_path() else {
                    return Ok(Vec::new());
                };
                if !path.exists() {
                    return Ok(Vec::new());
                }
                let file = tokio::fs::File::open(&path)
                    .await
                    .map_err(|error| MessageLedgerError::Io(error.to_string()))?;
                let mut lines = BufReader::new(file).lines();
                let mut events = Vec::new();
                while let Some(line) = lines
                    .next_line()
                    .await
                    .map_err(|error| MessageLedgerError::Io(error.to_string()))?
                {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<MessageLedgerLine>(&line)
                        .map_err(|error| MessageLedgerError::Parse(error.to_string()))?
                    {
                        MessageLedgerLine::Session { .. } => {}
                        MessageLedgerLine::Event(event) => events.push(*event),
                    }
                }
                Ok(events)
            }
            MessageLedgerMode::Memory { events } => Ok(events.lock().await.clone()),
        }
    }
}

pub fn fold_records(events: Vec<MessageLedgerEvent>) -> Vec<MessageLedgerRecord> {
    let mut by_ledger: HashMap<String, MessageLedgerRecord> = HashMap::new();
    for event in events {
        let entry = by_ledger
            .entry(event.ledger_id.clone())
            .or_insert_with(|| MessageLedgerRecord::from_event(&event));
        entry.apply_event(&event);
    }

    let mut records: Vec<_> = by_ledger.into_values().collect();
    records.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
    records
}

fn limit_tail<T>(mut items: Vec<T>, limit: usize) -> Vec<T> {
    if limit == 0 || items.len() <= limit {
        return items;
    }
    items.drain(0..items.len() - limit);
    items
}

pub fn default_terminal_reason(record: &MessageLedgerRecord) -> MessageTerminalReason {
    record
        .terminal_reason
        .unwrap_or(MessageTerminalReason::None)
}

pub type SharedMessageLedgerStore = Arc<MessageLedgerStore>;

#[cfg(test)]
mod tests;
