use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use garyx_models::ThreadHistoryBackend;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::conversation_index::ConversationIndexManager;
use crate::file_store::thread_storage_file_name;
use crate::store::ThreadStore;

pub const DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT: usize = 100;
pub const RECENT_COMMITTED_RUN_IDS_LIMIT: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum ThreadHistoryError {
    #[error("thread not found: {0}")]
    ThreadNotFound(String),
    #[error("missing transcript for thread: {0}")]
    MissingTranscript(String),
    #[error("transcript io error for thread {thread_id}: {message}")]
    TranscriptIo { thread_id: String, message: String },
    #[error("invalid transcript for thread {thread_id}: {message}")]
    InvalidTranscript { thread_id: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadTranscriptRecord {
    pub seq: u64,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub timestamp: String,
    pub message: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptAppendResult {
    pub total_messages: usize,
    pub last_message_at: Option<String>,
    pub transcript_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ThreadHistorySnapshot {
    pub thread_id: String,
    pub thread_data: Value,
    pub committed_messages: Vec<Value>,
    pub overlay_messages: Vec<Value>,
    pub total_committed_messages: usize,
}

impl ThreadHistorySnapshot {
    pub fn combined_messages(&self) -> Vec<Value> {
        let mut messages =
            Vec::with_capacity(self.committed_messages.len() + self.overlay_messages.len());
        messages.extend(self.committed_messages.clone());
        messages.extend(self.overlay_messages.clone());
        messages
    }

    pub fn total_messages(&self) -> usize {
        self.total_committed_messages + self.overlay_messages.len()
    }
}

#[derive(Debug)]
enum TranscriptStoreMode {
    File {
        root_dir: PathBuf,
        io_lock: Mutex<()>,
    },
    Memory {
        records: Mutex<HashMap<String, Vec<ThreadTranscriptRecord>>>,
    },
}

#[derive(Debug)]
pub struct ThreadTranscriptStore {
    mode: TranscriptStoreMode,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TranscriptLine {
    Session {
        version: u32,
        thread_id: String,
        created_at: String,
    },
    Message {
        seq: u64,
        thread_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        timestamp: String,
        message: Value,
    },
}

impl ThreadTranscriptStore {
    pub async fn file(root_dir: impl AsRef<Path>) -> std::io::Result<Self> {
        tokio::fs::create_dir_all(root_dir.as_ref()).await?;
        Ok(Self {
            mode: TranscriptStoreMode::File {
                root_dir: root_dir.as_ref().to_path_buf(),
                io_lock: Mutex::new(()),
            },
        })
    }

    pub fn memory() -> Self {
        Self {
            mode: TranscriptStoreMode::Memory {
                records: Mutex::new(HashMap::new()),
            },
        }
    }

    pub fn transcript_path(&self, thread_id: &str) -> Option<PathBuf> {
        match &self.mode {
            TranscriptStoreMode::File { root_dir, .. } => {
                Some(root_dir.join(thread_storage_file_name(thread_id, "jsonl")))
            }
            TranscriptStoreMode::Memory { .. } => None,
        }
    }

    pub async fn exists(&self, thread_id: &str) -> bool {
        match &self.mode {
            TranscriptStoreMode::File { .. } => self
                .transcript_path(thread_id)
                .is_some_and(|path| path.exists()),
            TranscriptStoreMode::Memory { records } => records
                .lock()
                .await
                .get(thread_id)
                .is_some_and(|entries| !entries.is_empty()),
        }
    }

    pub async fn append_committed_messages(
        &self,
        thread_id: &str,
        run_id: Option<&str>,
        messages: &[Value],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { io_lock, .. } => {
                let _guard = io_lock.lock().await;
                let path = self.transcript_path(thread_id).ok_or_else(|| {
                    ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: "missing transcript path".to_owned(),
                    }
                })?;
                let mut existing = self.read_records_from_path(thread_id, &path).await?;
                let mut next_seq = existing.last().map(|record| record.seq + 1).unwrap_or(1);
                let trimmed_run_id = trim_non_empty(run_id);
                let mut appended = Vec::with_capacity(messages.len());
                for message in messages {
                    let record = ThreadTranscriptRecord {
                        seq: next_seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
                        timestamp: message_timestamp(message)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: message.clone(),
                    };
                    next_seq += 1;
                    appended.push(record.clone());
                    existing.push(record);
                }

                if appended.is_empty() && path.exists() {
                    return Ok(TranscriptAppendResult {
                        total_messages: existing.len(),
                        last_message_at: existing.last().map(|record| record.timestamp.clone()),
                        transcript_file: Some(path),
                    });
                }

                let mut file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .await
                    .map_err(|error| ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    })?;

                if !path.exists()
                    || tokio::fs::metadata(&path)
                        .await
                        .map(|meta| meta.len())
                        .unwrap_or(0)
                        == 0
                {
                    let header = serde_json::to_string(&TranscriptLine::Session {
                        version: 1,
                        thread_id: thread_id.to_owned(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                    })
                    .map_err(|error| {
                        ThreadHistoryError::InvalidTranscript {
                            thread_id: thread_id.to_owned(),
                            message: error.to_string(),
                        }
                    })?;
                    file.write_all(header.as_bytes()).await.map_err(|error| {
                        ThreadHistoryError::TranscriptIo {
                            thread_id: thread_id.to_owned(),
                            message: error.to_string(),
                        }
                    })?;
                    file.write_all(b"\n").await.map_err(|error| {
                        ThreadHistoryError::TranscriptIo {
                            thread_id: thread_id.to_owned(),
                            message: error.to_string(),
                        }
                    })?;
                }

                for record in &appended {
                    let line = serde_json::to_string(&TranscriptLine::from(record.clone()))
                        .map_err(|error| ThreadHistoryError::InvalidTranscript {
                            thread_id: thread_id.to_owned(),
                            message: error.to_string(),
                        })?;
                    file.write_all(line.as_bytes()).await.map_err(|error| {
                        ThreadHistoryError::TranscriptIo {
                            thread_id: thread_id.to_owned(),
                            message: error.to_string(),
                        }
                    })?;
                    file.write_all(b"\n").await.map_err(|error| {
                        ThreadHistoryError::TranscriptIo {
                            thread_id: thread_id.to_owned(),
                            message: error.to_string(),
                        }
                    })?;
                }
                file.flush()
                    .await
                    .map_err(|error| ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    })?;

                Ok(TranscriptAppendResult {
                    total_messages: existing.len(),
                    last_message_at: existing.last().map(|record| record.timestamp.clone()),
                    transcript_file: Some(path),
                })
            }
            TranscriptStoreMode::Memory { records } => {
                let trimmed_run_id = trim_non_empty(run_id);
                let mut guard = records.lock().await;
                let entries = guard.entry(thread_id.to_owned()).or_default();
                let mut next_seq = entries.last().map(|record| record.seq + 1).unwrap_or(1);
                for message in messages {
                    entries.push(ThreadTranscriptRecord {
                        seq: next_seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
                        timestamp: message_timestamp(message)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: message.clone(),
                    });
                    next_seq += 1;
                }
                Ok(TranscriptAppendResult {
                    total_messages: entries.len(),
                    last_message_at: entries.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                })
            }
        }
    }

    pub async fn rewrite_from_messages(
        &self,
        thread_id: &str,
        messages: &[Value],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { io_lock, .. } => {
                let _guard = io_lock.lock().await;
                let path = self.transcript_path(thread_id).ok_or_else(|| {
                    ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: "missing transcript path".to_owned(),
                    }
                })?;
                let mut lines = Vec::with_capacity(messages.len() + 1);
                lines.push(
                    serde_json::to_string(&TranscriptLine::Session {
                        version: 1,
                        thread_id: thread_id.to_owned(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                    })
                    .map_err(|error| {
                        ThreadHistoryError::InvalidTranscript {
                            thread_id: thread_id.to_owned(),
                            message: error.to_string(),
                        }
                    })?,
                );
                let mut last_message_at = None;
                for (idx, message) in messages.iter().enumerate() {
                    let record = ThreadTranscriptRecord {
                        seq: idx as u64 + 1,
                        thread_id: thread_id.to_owned(),
                        run_id: extract_run_id(message),
                        timestamp: message_timestamp(message)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: message.clone(),
                    };
                    last_message_at = Some(record.timestamp.clone());
                    lines.push(
                        serde_json::to_string(&TranscriptLine::from(record)).map_err(|error| {
                            ThreadHistoryError::InvalidTranscript {
                                thread_id: thread_id.to_owned(),
                                message: error.to_string(),
                            }
                        })?,
                    );
                }
                let payload = format!("{}\n", lines.join("\n"));
                tokio::fs::write(&path, payload).await.map_err(|error| {
                    ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    }
                })?;
                Ok(TranscriptAppendResult {
                    total_messages: messages.len(),
                    last_message_at,
                    transcript_file: Some(path),
                })
            }
            TranscriptStoreMode::Memory { records } => {
                let mut guard = records.lock().await;
                let entries = guard.entry(thread_id.to_owned()).or_default();
                entries.clear();
                for (idx, message) in messages.iter().enumerate() {
                    entries.push(ThreadTranscriptRecord {
                        seq: idx as u64 + 1,
                        thread_id: thread_id.to_owned(),
                        run_id: extract_run_id(message),
                        timestamp: message_timestamp(message)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: message.clone(),
                    });
                }
                Ok(TranscriptAppendResult {
                    total_messages: entries.len(),
                    last_message_at: entries.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                })
            }
        }
    }

    pub async fn tail(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let start = records.len().saturating_sub(limit);
        Ok(records[start..]
            .iter()
            .map(|record| record.message.clone())
            .collect())
    }

    pub async fn message_count(&self, thread_id: &str) -> Result<usize, ThreadHistoryError> {
        Ok(self.read_records(thread_id).await?.len())
    }

    pub async fn records(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        self.read_records(thread_id).await
    }

    pub async fn find_latest_for_run(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        if trimmed_run_id.is_empty() {
            return Ok(Vec::new());
        }
        let records = self.read_records(thread_id).await?;
        let mut matches = Vec::new();
        let mut collecting = false;
        for record in records.iter().rev() {
            match record.run_id.as_deref() {
                Some(candidate) if candidate == trimmed_run_id => {
                    collecting = true;
                    matches.push(record.message.clone());
                }
                _ if collecting => break,
                _ => {}
            }
        }
        matches.reverse();
        Ok(matches)
    }

    pub async fn find_latest_text_for_role(
        &self,
        thread_id: &str,
        role: &str,
    ) -> Result<Option<String>, ThreadHistoryError> {
        let trimmed_role = role.trim();
        if trimmed_role.is_empty() {
            return Ok(None);
        }
        let records = self.read_records(thread_id).await?;
        for record in records.iter().rev() {
            if message_role(&record.message) == Some(trimmed_role) {
                if let Some(text) = message_text(&record.message)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Ok(Some(text.to_owned()));
                }
            }
        }
        Ok(None)
    }

    pub async fn delete(&self, thread_id: &str) -> Result<(), ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { io_lock, .. } => {
                let _guard = io_lock.lock().await;
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(());
                };
                match tokio::fs::remove_file(&path).await {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    }),
                }
            }
            TranscriptStoreMode::Memory { records } => {
                records.lock().await.remove(thread_id);
                Ok(())
            }
        }
    }

    async fn read_records(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                self.read_records_from_path(thread_id, &path).await
            }
            TranscriptStoreMode::Memory { records } => Ok(records
                .lock()
                .await
                .get(thread_id)
                .cloned()
                .unwrap_or_default()),
        }
    }

    async fn read_records_from_path(
        &self,
        thread_id: &str,
        path: &Path,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = tokio::fs::read_to_string(path).await.map_err(|error| {
            ThreadHistoryError::TranscriptIo {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            }
        })?;
        let mut records = Vec::new();
        for (line_no, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed = serde_json::from_str::<TranscriptLine>(line).map_err(|error| {
                ThreadHistoryError::InvalidTranscript {
                    thread_id: thread_id.to_owned(),
                    message: format!("line {}: {}", line_no + 1, error),
                }
            })?;
            if let TranscriptLine::Message {
                seq,
                thread_id,
                run_id,
                timestamp,
                message,
            } = parsed
            {
                records.push(ThreadTranscriptRecord {
                    seq,
                    thread_id,
                    run_id,
                    timestamp,
                    message,
                });
            }
        }
        Ok(records)
    }
}

impl Default for ThreadTranscriptStore {
    fn default() -> Self {
        Self::memory()
    }
}

impl From<ThreadTranscriptRecord> for TranscriptLine {
    fn from(value: ThreadTranscriptRecord) -> Self {
        Self::Message {
            seq: value.seq,
            thread_id: value.thread_id,
            run_id: value.run_id,
            timestamp: value.timestamp,
            message: value.message,
        }
    }
}

pub struct ThreadHistoryRepository {
    thread_store: Arc<dyn ThreadStore>,
    transcript_store: Arc<ThreadTranscriptStore>,
    backend: ThreadHistoryBackend,
    conversation_index: Option<Arc<ConversationIndexManager>>,
}

impl ThreadHistoryRepository {
    pub fn new(
        thread_store: Arc<dyn ThreadStore>,
        transcript_store: Arc<ThreadTranscriptStore>,
        backend: ThreadHistoryBackend,
    ) -> Self {
        Self {
            thread_store,
            transcript_store,
            backend,
            conversation_index: None,
        }
    }

    pub fn with_conversation_index(
        mut self,
        conversation_index: Arc<ConversationIndexManager>,
    ) -> Self {
        self.conversation_index = Some(conversation_index);
        self
    }

    pub fn transcript_store(&self) -> Arc<ThreadTranscriptStore> {
        self.transcript_store.clone()
    }

    pub fn backend(&self) -> ThreadHistoryBackend {
        self.backend.clone()
    }

    pub fn conversation_index(&self) -> Option<Arc<ConversationIndexManager>> {
        self.conversation_index.clone()
    }

    pub fn update_conversation_index_config(
        &self,
        config: garyx_models::config::ConversationIndexConfig,
    ) {
        if let Some(index) = &self.conversation_index {
            index.update_config(config);
        }
    }

    pub fn enqueue_conversation_index_for_thread(&self, thread_id: &str) {
        if let Some(index) = &self.conversation_index {
            index.enqueue_thread(thread_id);
        }
    }

    pub fn schedule_full_conversation_index_backfill(&self) {
        if let Some(index) = &self.conversation_index {
            index.schedule_full_backfill();
        }
    }

    pub async fn thread_snapshot(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;
        let overlay_messages = active_run_snapshot_messages(&thread_data);
        let bounded_limit = limit.max(1);

        let overlay_tail = if overlay_messages.len() > bounded_limit {
            overlay_messages[overlay_messages.len() - bounded_limit..].to_vec()
        } else {
            overlay_messages
        };

        let committed_limit = bounded_limit.saturating_sub(overlay_tail.len());
        let (committed_messages, total_committed_messages) = self
            .load_committed_messages(thread_id, &thread_data, committed_limit)
            .await?;

        Ok(ThreadHistorySnapshot {
            thread_id: thread_id.to_owned(),
            thread_data,
            committed_messages,
            overlay_messages: overlay_tail,
            total_committed_messages,
        })
    }

    pub async fn find_latest_for_run(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        if trimmed_run_id.is_empty() {
            return Ok(Vec::new());
        }
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;

        if self.transcript_store.exists(thread_id).await {
            return self
                .transcript_store
                .find_latest_for_run(thread_id, trimmed_run_id)
                .await;
        }

        if matches!(self.backend, ThreadHistoryBackend::TranscriptV1) {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        let messages = inline_messages(&thread_data);
        let mut matches = Vec::new();
        let mut collecting = false;
        for message in messages.iter().rev() {
            if extract_run_id(message).as_deref() == Some(trimmed_run_id) {
                collecting = true;
                matches.push(message.clone());
            } else if collecting {
                break;
            }
        }
        matches.reverse();
        Ok(matches)
    }

    pub async fn latest_message_text(
        &self,
        thread_id: &str,
    ) -> Result<Option<String>, ThreadHistoryError> {
        let snapshot = self.thread_snapshot(thread_id, 1).await?;
        let combined = snapshot.combined_messages();
        Ok(combined
            .last()
            .and_then(message_text)
            .map(|value| value.to_owned()))
    }

    pub async fn latest_message_text_for_role(
        &self,
        thread_id: &str,
        role: &str,
    ) -> Result<Option<String>, ThreadHistoryError> {
        let trimmed_role = role.trim();
        if trimmed_role.is_empty() {
            return Ok(None);
        }
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;

        for message in active_run_snapshot_messages(&thread_data).iter().rev() {
            if message_role(message) == Some(trimmed_role) {
                if let Some(text) = message_text(message)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Ok(Some(text.to_owned()));
                }
            }
        }

        if self.transcript_store.exists(thread_id).await {
            return self
                .transcript_store
                .find_latest_text_for_role(thread_id, trimmed_role)
                .await;
        }

        let inline = inline_messages(&thread_data);
        let inline_total = history_message_count(&thread_data).max(inline.len());
        if matches!(self.backend, ThreadHistoryBackend::TranscriptV1) && inline_total > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        for message in inline.iter().rev() {
            if message_role(message) == Some(trimmed_role) {
                if let Some(text) = message_text(message)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Ok(Some(text.to_owned()));
                }
            }
        }

        Ok(None)
    }

    pub async fn delete_thread_history(&self, thread_id: &str) -> Result<(), ThreadHistoryError> {
        self.transcript_store.delete(thread_id).await
    }

    async fn load_committed_messages(
        &self,
        thread_id: &str,
        thread_data: &Value,
        limit: usize,
    ) -> Result<(Vec<Value>, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let inline = inline_messages(thread_data);
        let inline_total = history_message_count(thread_data).max(inline.len());
        if matches!(self.backend, ThreadHistoryBackend::TranscriptV1)
            && !has_transcript
            && inline_total > 0
        {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        if has_transcript {
            let total = self.transcript_store.message_count(thread_id).await?;
            if limit == 0 {
                return Ok((Vec::new(), total));
            }
            return Ok((self.transcript_store.tail(thread_id, limit).await?, total));
        }

        let messages = inline;
        let total = inline_total;
        if limit == 0 {
            return Ok((Vec::new(), total));
        }
        let start = messages.len().saturating_sub(limit);
        Ok((messages[start..].to_vec(), total))
    }
}

pub fn inline_messages(thread_data: &Value) -> Vec<Value> {
    thread_data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub fn history_message_count(thread_data: &Value) -> usize {
    thread_data
        .get("history")
        .and_then(|value| value.get("message_count"))
        .and_then(Value::as_u64)
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        .or_else(|| {
            thread_data
                .get("message_count")
                .and_then(Value::as_u64)
                .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        })
        .unwrap_or_else(|| inline_messages(thread_data).len())
}

pub fn active_run_snapshot_messages(thread_data: &Value) -> Vec<Value> {
    thread_data
        .get("history")
        .and_then(|value| value.get("active_run_snapshot"))
        .and_then(|value| value.get("messages"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub fn active_run_snapshot_run_id(thread_data: &Value) -> Option<String> {
    thread_data
        .get("history")
        .and_then(|value| value.get("active_run_snapshot"))
        .and_then(|value| value.get("run_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn message_text(message: &Value) -> Option<&str> {
    message
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| message.get("content").and_then(Value::as_str))
}

fn message_role(message: &Value) -> Option<&str> {
    message.get("role").and_then(Value::as_str)
}

pub fn extract_run_id(message: &Value) -> Option<String> {
    let object = message.as_object()?;
    for key in ["bridge_run_id", "run_id", "client_run_id"] {
        if let Some(value) = object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_owned());
        }
        if let Some(value) = object
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_owned());
        }
    }
    None
}

fn trim_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn message_timestamp(message: &Value) -> Option<String> {
    message
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests;
