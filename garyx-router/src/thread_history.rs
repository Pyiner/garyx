use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use garyx_models::{
    RenderSnapshot, RenderWindow, TranscriptRunState, reduce_transcript_render_state,
    reduce_transcript_render_state_with_run_state, reduce_transcript_run_state,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::file_store::thread_storage_file_name;
use crate::store::ThreadStore;

pub const DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT: usize = 100;
pub const RECENT_COMMITTED_RUN_IDS_LIMIT: usize = 256;
pub const THREAD_TRANSCRIPT_REPLAY_CAP: usize = 10_000;

const TAIL_SCAN_CHUNK_BYTES: u64 = 64 * 1024;

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

#[derive(Debug, Clone, PartialEq)]
pub struct RunTranscriptRecordDraft {
    pub timestamp: Option<String>,
    pub message: Value,
}

impl RunTranscriptRecordDraft {
    pub fn from_message(message: Value) -> Self {
        Self {
            timestamp: message_timestamp(&message),
            message,
        }
    }

    pub fn with_timestamp(message: Value, timestamp: impl Into<String>) -> Self {
        Self {
            timestamp: Some(timestamp.into()),
            message,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptAppendRecordsResult {
    pub total_messages: usize,
    pub last_message_at: Option<String>,
    pub transcript_file: Option<PathBuf>,
    pub appended_records: Vec<ThreadTranscriptRecord>,
}

#[derive(Debug, Clone)]
pub struct ThreadHistorySnapshot {
    pub thread_id: String,
    pub thread_data: Value,
    pub committed_messages: Vec<Value>,
    pub total_committed_messages: usize,
    pub committed_start_index: usize,
}

impl ThreadHistorySnapshot {
    pub fn combined_messages(&self) -> Vec<Value> {
        self.committed_messages.clone()
    }

    pub fn total_messages(&self) -> usize {
        self.total_committed_messages
    }

    pub fn message_index_at(&self, offset: usize) -> usize {
        self.committed_start_index + offset
    }

    pub fn first_message_index(&self) -> Option<usize> {
        if !self.committed_messages.is_empty() {
            Some(self.committed_start_index)
        } else {
            None
        }
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
                let next_seq = existing.last().map(|record| record.seq + 1).unwrap_or(1);
                let trimmed_run_id = trim_non_empty(run_id);
                let mut appended = Vec::with_capacity(messages.len());
                for (seq, message) in (next_seq..).zip(messages.iter()) {
                    let record = ThreadTranscriptRecord {
                        seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
                        timestamp: message_timestamp(message)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: message.clone(),
                    };
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
                let next_seq = entries.last().map(|record| record.seq + 1).unwrap_or(1);
                for (seq, message) in (next_seq..).zip(messages.iter()) {
                    entries.push(ThreadTranscriptRecord {
                        seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
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

    pub async fn append_run_records(
        &self,
        thread_id: &str,
        run_id: Option<&str>,
        records: &[RunTranscriptRecordDraft],
    ) -> Result<TranscriptAppendRecordsResult, ThreadHistoryError> {
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
                let next_seq = existing.last().map(|record| record.seq + 1).unwrap_or(1);
                let trimmed_run_id = trim_non_empty(run_id);
                let mut appended_records = Vec::with_capacity(records.len());
                for (seq, draft) in (next_seq..).zip(records.iter()) {
                    let record = ThreadTranscriptRecord {
                        seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
                        timestamp: draft
                            .timestamp
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: draft.message.clone(),
                    };
                    appended_records.push(record.clone());
                    existing.push(record);
                }

                if appended_records.is_empty() && path.exists() {
                    return Ok(TranscriptAppendRecordsResult {
                        total_messages: existing.len(),
                        last_message_at: existing.last().map(|record| record.timestamp.clone()),
                        transcript_file: Some(path),
                        appended_records,
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

                for record in &appended_records {
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

                Ok(TranscriptAppendRecordsResult {
                    total_messages: existing.len(),
                    last_message_at: existing.last().map(|record| record.timestamp.clone()),
                    transcript_file: Some(path),
                    appended_records,
                })
            }
            TranscriptStoreMode::Memory { records: store } => {
                let trimmed_run_id = trim_non_empty(run_id);
                let mut guard = store.lock().await;
                let entries = guard.entry(thread_id.to_owned()).or_default();
                let next_seq = entries.last().map(|record| record.seq + 1).unwrap_or(1);
                let mut appended_records = Vec::with_capacity(records.len());
                for (seq, draft) in (next_seq..).zip(records.iter()) {
                    let record = ThreadTranscriptRecord {
                        seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
                        timestamp: draft
                            .timestamp
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: draft.message.clone(),
                    };
                    entries.push(record.clone());
                    appended_records.push(record);
                }
                Ok(TranscriptAppendRecordsResult {
                    total_messages: entries.len(),
                    last_message_at: entries.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                    appended_records,
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
                let existing = self.read_records_from_path(thread_id, &path).await?;
                let records = reconcile_rewrite_records(thread_id, &existing, messages);
                if records == existing {
                    return Ok(TranscriptAppendResult {
                        total_messages: existing.len(),
                        last_message_at: existing.last().map(|record| record.timestamp.clone()),
                        transcript_file: Some(path),
                    });
                }

                let mut lines = Vec::with_capacity(records.len() + 1);
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
                for record in &records {
                    last_message_at = Some(record.timestamp.clone());
                    lines.push(
                        serde_json::to_string(&TranscriptLine::from(record.clone())).map_err(
                            |error| ThreadHistoryError::InvalidTranscript {
                                thread_id: thread_id.to_owned(),
                                message: error.to_string(),
                            },
                        )?,
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
                    total_messages: records.len(),
                    last_message_at,
                    transcript_file: Some(path),
                })
            }
            TranscriptStoreMode::Memory { records } => {
                let mut guard = records.lock().await;
                let entries = guard.entry(thread_id.to_owned()).or_default();
                *entries = reconcile_rewrite_records(thread_id, entries, messages);
                Ok(TranscriptAppendResult {
                    total_messages: entries.len(),
                    last_message_at: entries.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                })
            }
        }
    }

    /// Overwrite the whole transcript with `records`, preserving each record's
    /// `seq`/`run_id`/`timestamp`. Internal helper for tail reconciliation.
    async fn write_records(
        &self,
        thread_id: &str,
        records: &[ThreadTranscriptRecord],
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
                let mut lines = Vec::with_capacity(records.len() + 1);
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
                for record in records {
                    lines.push(
                        serde_json::to_string(&TranscriptLine::from(record.clone())).map_err(
                            |error| ThreadHistoryError::InvalidTranscript {
                                thread_id: thread_id.to_owned(),
                                message: error.to_string(),
                            },
                        )?,
                    );
                }
                let payload = format!("{}\n", lines.join("\n"));
                // Atomic replace: write to a temp file then rename, so a crash
                // mid-write can never truncate the committed transcript.
                let tmp = path.with_extension("jsonl.tmp");
                tokio::fs::write(&tmp, payload).await.map_err(|error| {
                    ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    }
                })?;
                tokio::fs::rename(&tmp, &path).await.map_err(|error| {
                    ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    }
                })?;
                Ok(TranscriptAppendResult {
                    total_messages: records.len(),
                    last_message_at: records.last().map(|record| record.timestamp.clone()),
                    transcript_file: Some(path),
                })
            }
            TranscriptStoreMode::Memory { records: store } => {
                let mut guard = store.lock().await;
                guard.insert(thread_id.to_owned(), records.to_vec());
                Ok(TranscriptAppendResult {
                    total_messages: records.len(),
                    last_message_at: records.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                })
            }
        }
    }

    /// Ensure the trailing block of records tagged with `run_id` exactly equals
    /// `authoritative`. No-op when they already match (the common path once a run
    /// has been appended incrementally); otherwise rewrites that run's tail.
    /// Because a `run_id` is reused across retries/resumes, this defines a run's
    /// transcript contribution as its final authoritative message set without
    /// duplicating earlier attempts.
    pub async fn reconcile_run_tail(
        &self,
        thread_id: &str,
        run_id: &str,
        authoritative: &[Value],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        let records = self.read_records(thread_id).await?;
        // Without a run_id we cannot identify which trailing records belong to
        // this run, so we cannot tell the worker's already-appended rows apart
        // from earlier runs. Reconciling blindly would re-append the whole run
        // (the empty tail is a prefix of everything). Make it a no-op and trust
        // the worker's incremental appends; the bridge always supplies a run_id.
        if trimmed_run_id.is_empty() {
            if !authoritative.is_empty() {
                tracing::warn!(
                    thread_id = %thread_id,
                    "reconcile_run_tail called without a run_id; skipping tail reconcile"
                );
            }
            return Ok(TranscriptAppendResult {
                total_messages: records.len(),
                last_message_at: records.last().map(|record| record.timestamp.clone()),
                transcript_file: self.transcript_path(thread_id),
            });
        }
        let mut split = records.len();
        while split > 0
            && records[split - 1].run_id.as_deref().map(str::trim) == Some(trimmed_run_id)
        {
            split -= 1;
        }
        let existing_tail: Vec<Value> = records[split..]
            .iter()
            .map(|record| record.message.clone())
            .collect();
        // Compare on a normalized identity that ignores cosmetic, flush-time
        // varying fields (the SDK session id is bound mid-run, so the user row
        // committed by the first flush carries `None` while the terminal rebuild
        // carries `Some`; timestamps can be backfilled). Without this the user
        // row always "diverges" and every terminal commit falls to the O(file)
        // rewrite below instead of the cheap no-op / suffix-append.
        let existing_identity: Vec<Value> = existing_tail.iter().map(message_identity).collect();
        let authoritative_identity: Vec<Value> =
            authoritative.iter().map(message_identity).collect();
        if existing_identity == authoritative_identity {
            return Ok(TranscriptAppendResult {
                total_messages: records.len(),
                last_message_at: records.last().map(|record| record.timestamp.clone()),
                transcript_file: self.transcript_path(thread_id),
            });
        }
        // Fast path: the run only grew (the already-committed tail is a prefix of
        // `authoritative`). This is the steady-state terminal case — the worker
        // streamed every finalized row during the run, and the terminal just needs
        // to append the trailing segment that was still in flight at the last
        // flush. Append the suffix instead of rewriting the whole file, which on a
        // large transcript would be an O(file) rewrite to add one message.
        if authoritative_identity.len() > existing_identity.len()
            && authoritative_identity[..existing_identity.len()] == existing_identity[..]
        {
            let suffix = &authoritative[existing_tail.len()..];
            return self
                .append_committed_messages(thread_id, Some(trimmed_run_id), suffix)
                .await;
        }
        // Divergent (a retry re-streamed different content, or the run shrank):
        // rewrite this run's tail so we keep only the final authoritative set.
        let mut rebuilt: Vec<ThreadTranscriptRecord> = records[..split].to_vec();
        let next_seq = rebuilt.last().map(|record| record.seq + 1).unwrap_or(1);
        let run_id_value = (!trimmed_run_id.is_empty()).then(|| trimmed_run_id.to_owned());
        for (seq, message) in (next_seq..).zip(authoritative.iter()) {
            rebuilt.push(ThreadTranscriptRecord {
                seq,
                thread_id: thread_id.to_owned(),
                run_id: run_id_value.clone(),
                timestamp: message_timestamp(message)
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                message: message.clone(),
            });
        }
        self.write_records(thread_id, &rebuilt).await
    }

    pub async fn reconcile_run_records_tail(
        &self,
        thread_id: &str,
        run_id: &str,
        authoritative: &[RunTranscriptRecordDraft],
    ) -> Result<TranscriptAppendRecordsResult, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        let records = self.read_records(thread_id).await?;
        if trimmed_run_id.is_empty() {
            if !authoritative.is_empty() {
                tracing::warn!(
                    thread_id = %thread_id,
                    "reconcile_run_records_tail called without a run_id; skipping tail reconcile"
                );
            }
            return Ok(TranscriptAppendRecordsResult {
                total_messages: records.len(),
                last_message_at: records.last().map(|record| record.timestamp.clone()),
                transcript_file: self.transcript_path(thread_id),
                appended_records: Vec::new(),
            });
        }

        let mut split = records.len();
        while split > 0
            && records[split - 1].run_id.as_deref().map(str::trim) == Some(trimmed_run_id)
        {
            split -= 1;
        }
        let existing_tail = &records[split..];
        if existing_tail.is_empty() {
            return self
                .append_run_records(thread_id, Some(trimmed_run_id), authoritative)
                .await;
        }

        let existing_identity: Vec<Value> = existing_tail
            .iter()
            .map(|record| message_identity(&record.message))
            .collect();
        let authoritative_identity: Vec<Value> = authoritative
            .iter()
            .map(|draft| message_identity(&draft.message))
            .collect();

        if existing_identity == authoritative_identity {
            let mut changed = Vec::new();
            let mut changed_same_seqs = Vec::new();
            let mut rebuilt = records.clone();
            for (offset, draft) in authoritative.iter().enumerate() {
                let existing = &existing_tail[offset];
                let replacement = record_from_draft_replacing(
                    thread_id,
                    Some(trimmed_run_id),
                    existing.seq,
                    draft,
                    existing,
                );
                if replacement.timestamp != existing.timestamp
                    || replacement.message != existing.message
                {
                    rebuilt[split + offset] = replacement.clone();
                    changed_same_seqs.push(replacement.seq);
                    changed.push(replacement);
                }
            }
            if changed.is_empty() {
                return Ok(TranscriptAppendRecordsResult {
                    total_messages: records.len(),
                    last_message_at: records.last().map(|record| record.timestamp.clone()),
                    transcript_file: self.transcript_path(thread_id),
                    appended_records: Vec::new(),
                });
            }
            append_range_rewrite_marker(
                &mut rebuilt,
                &mut changed,
                thread_id,
                Some(trimmed_run_id),
                changed_same_seqs.iter().copied().min().unwrap_or(1),
                changed_same_seqs.iter().copied().max().unwrap_or(1),
                authoritative.len(),
                existing_tail.len(),
                "same_seq_overwrite",
            );
            let summary = self.write_records(thread_id, &rebuilt).await?;
            return Ok(TranscriptAppendRecordsResult {
                total_messages: summary.total_messages,
                last_message_at: summary.last_message_at,
                transcript_file: summary.transcript_file,
                appended_records: changed,
            });
        }

        if authoritative_identity.len() > existing_identity.len()
            && authoritative_identity[..existing_identity.len()] == existing_identity[..]
        {
            let prefix_changed =
                authoritative
                    .iter()
                    .zip(existing_tail.iter())
                    .any(|(draft, existing)| {
                        let replacement = record_from_draft_replacing(
                            thread_id,
                            Some(trimmed_run_id),
                            existing.seq,
                            draft,
                            existing,
                        );
                        replacement.timestamp != existing.timestamp
                            || replacement.message != existing.message
                    });
            if !prefix_changed {
                return self
                    .append_run_records(
                        thread_id,
                        Some(trimmed_run_id),
                        &authoritative[existing_tail.len()..],
                    )
                    .await;
            }
        }

        if authoritative.len() >= existing_tail.len() {
            let mut changed = Vec::new();
            let mut changed_same_seqs = Vec::new();
            let mut rebuilt = records[..split].to_vec();
            let mut next_seq = existing_tail
                .first()
                .map(|record| record.seq)
                .unwrap_or_else(|| rebuilt.last().map(|record| record.seq + 1).unwrap_or(1));
            for (offset, draft) in authoritative.iter().enumerate() {
                let seq = if offset < existing_tail.len() {
                    existing_tail[offset].seq
                } else {
                    next_seq
                };
                let replacement = record_from_draft(thread_id, Some(trimmed_run_id), seq, draft);
                let replacement = if let Some(existing) = existing_tail.get(offset) {
                    record_from_draft_replacing(
                        thread_id,
                        Some(trimmed_run_id),
                        seq,
                        draft,
                        existing,
                    )
                } else {
                    replacement
                };
                let is_changed = existing_tail
                    .get(offset)
                    .map(|existing| {
                        existing.timestamp != replacement.timestamp
                            || existing.message != replacement.message
                    })
                    .unwrap_or(true);
                if is_changed {
                    if offset < existing_tail.len() {
                        changed_same_seqs.push(replacement.seq);
                    }
                    changed.push(replacement.clone());
                }
                rebuilt.push(replacement);
                next_seq = seq + 1;
            }
            if let (Some(start_seq), Some(end_seq)) = (
                changed_same_seqs.iter().copied().min(),
                changed_same_seqs.iter().copied().max(),
            ) {
                append_range_rewrite_marker(
                    &mut rebuilt,
                    &mut changed,
                    thread_id,
                    Some(trimmed_run_id),
                    start_seq,
                    end_seq,
                    authoritative.len(),
                    existing_tail.len(),
                    "same_seq_overwrite",
                );
            }
            let summary = self.write_records(thread_id, &rebuilt).await?;
            return Ok(TranscriptAppendRecordsResult {
                total_messages: summary.total_messages,
                last_message_at: summary.last_message_at,
                transcript_file: summary.transcript_file,
                appended_records: changed,
            });
        }

        if authoritative_identity.len() <= existing_identity.len()
            && existing_identity[..authoritative_identity.len()] == authoritative_identity[..]
            && existing_tail[authoritative.len()..]
                .iter()
                .all(|record| is_range_rewrite_control(&record.message))
        {
            let mut changed = Vec::new();
            let mut changed_same_seqs = Vec::new();
            let mut rebuilt = records.clone();
            for (offset, draft) in authoritative.iter().enumerate() {
                let existing = &existing_tail[offset];
                let replacement = record_from_draft_replacing(
                    thread_id,
                    Some(trimmed_run_id),
                    existing.seq,
                    draft,
                    existing,
                );
                if replacement.timestamp != existing.timestamp
                    || replacement.message != existing.message
                {
                    rebuilt[split + offset] = replacement.clone();
                    changed_same_seqs.push(replacement.seq);
                    changed.push(replacement.clone());
                }
            }
            if changed.is_empty() {
                return Ok(TranscriptAppendRecordsResult {
                    total_messages: records.len(),
                    last_message_at: records.last().map(|record| record.timestamp.clone()),
                    transcript_file: self.transcript_path(thread_id),
                    appended_records: Vec::new(),
                });
            }
            if let (Some(start_seq), Some(end_seq)) = (
                changed_same_seqs.iter().copied().min(),
                changed_same_seqs.iter().copied().max(),
            ) {
                append_range_rewrite_marker(
                    &mut rebuilt,
                    &mut changed,
                    thread_id,
                    Some(trimmed_run_id),
                    start_seq,
                    end_seq,
                    authoritative.len(),
                    existing_tail.len(),
                    "same_seq_overwrite",
                );
            }
            let summary = self.write_records(thread_id, &rebuilt).await?;
            return Ok(TranscriptAppendRecordsResult {
                total_messages: summary.total_messages,
                last_message_at: summary.last_message_at,
                transcript_file: summary.transcript_file,
                appended_records: changed,
            });
        }

        let mut changed = Vec::new();
        let mut changed_same_seqs = Vec::new();
        let mut rebuilt = records[..split].to_vec();
        let first_rewritten_seq = existing_tail
            .get(authoritative.len())
            .map(|record| record.seq)
            .unwrap_or_else(|| existing_tail.first().map(|record| record.seq).unwrap_or(1));
        let last_rewritten_seq = existing_tail
            .last()
            .map(|record| record.seq)
            .unwrap_or(first_rewritten_seq);
        let rewrite_at = chrono::Utc::now().to_rfc3339();
        for (offset, existing) in existing_tail.iter().enumerate() {
            if let Some(draft) = authoritative.get(offset) {
                let replacement = record_from_draft_replacing(
                    thread_id,
                    Some(trimmed_run_id),
                    existing.seq,
                    draft,
                    existing,
                );
                if replacement.timestamp != existing.timestamp
                    || replacement.message != existing.message
                {
                    changed_same_seqs.push(replacement.seq);
                    changed.push(replacement.clone());
                }
                rebuilt.push(replacement);
            } else {
                let rewrite = build_range_rewrite_record(
                    thread_id,
                    Some(trimmed_run_id),
                    existing.seq,
                    first_rewritten_seq,
                    last_rewritten_seq,
                    authoritative.len(),
                    existing_tail.len(),
                    true,
                    "run_tail_shrink",
                    &rewrite_at,
                );
                if rewrite.timestamp != existing.timestamp || rewrite.message != existing.message {
                    changed_same_seqs.push(rewrite.seq);
                    changed.push(rewrite.clone());
                }
                rebuilt.push(rewrite);
            }
        }

        let first_rewritten_seq = changed_same_seqs
            .iter()
            .copied()
            .min()
            .unwrap_or(first_rewritten_seq);
        let last_rewritten_seq = changed_same_seqs
            .iter()
            .copied()
            .max()
            .unwrap_or(last_rewritten_seq);
        let rewrite = build_range_rewrite_record(
            thread_id,
            Some(trimmed_run_id),
            rebuilt.last().map(|record| record.seq + 1).unwrap_or(1),
            first_rewritten_seq,
            last_rewritten_seq,
            authoritative.len(),
            existing_tail.len(),
            false,
            "run_tail_shrink",
            &rewrite_at,
        );
        changed.push(rewrite.clone());
        rebuilt.push(rewrite);
        let summary = self.write_records(thread_id, &rebuilt).await?;
        Ok(TranscriptAppendRecordsResult {
            total_messages: summary.total_messages,
            last_message_at: summary.last_message_at,
            transcript_file: summary.transcript_file,
            appended_records: changed,
        })
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

    pub async fn page_before_index(
        &self,
        thread_id: &str,
        before_index: Option<usize>,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let total = records.len();
        let end = before_index.unwrap_or(total).min(total);
        let start = end.saturating_sub(limit);
        let messages = records[start..end]
            .iter()
            .map(|record| record.message.clone())
            .collect();
        Ok((messages, total, start))
    }

    /// Forward page: committed records with position strictly greater than
    /// `after_index`, up to `limit`. Mirror of `page_before_index` for cursor
    /// (delta) sync — "give me what's new since index N".
    pub async fn page_after_index(
        &self,
        thread_id: &str,
        after_index: usize,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let total = records.len();
        let start = after_index.saturating_add(1).min(total);
        let end = start.saturating_add(limit).min(total);
        let messages = records[start..end]
            .iter()
            .map(|record| record.message.clone())
            .collect();
        Ok((messages, total, start))
    }

    pub async fn page_before_user_queries(
        &self,
        thread_id: &str,
        before_index: Option<usize>,
        user_query_limit: usize,
        fallback_message_limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let total = records.len();
        let end = before_index.unwrap_or(total).min(total);
        let target_user_queries = user_query_limit.max(1);
        let mut start = end;
        let mut user_queries = 0usize;

        while start > 0 && user_queries < target_user_queries {
            start -= 1;
            if is_user_query_message(&records[start].message) {
                user_queries += 1;
            }
        }

        if user_queries == 0 {
            start = end.saturating_sub(fallback_message_limit.max(1));
        }

        let messages = records[start..end]
            .iter()
            .map(|record| record.message.clone())
            .collect();
        Ok((messages, total, start))
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

    pub async fn run_state(
        &self,
        thread_id: &str,
    ) -> Result<TranscriptRunState, ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let values = records
            .iter()
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        Ok(reduce_transcript_run_state(&values))
    }

    pub async fn render_snapshot_at_seq(
        &self,
        thread_id: &str,
        based_on_seq: u64,
    ) -> Result<RenderSnapshot, ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let values = records
            .iter()
            .filter(|record| record.seq <= based_on_seq)
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        Ok(reduce_transcript_render_state(&values))
    }

    pub async fn render_snapshot_in_window(
        &self,
        thread_id: &str,
        floor_seq: u64,
        based_on_seq: u64,
    ) -> Result<RenderSnapshot, ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let prefix = records
            .iter()
            .filter(|record| record.seq <= based_on_seq)
            .collect::<Vec<_>>();
        let actual_based_on_seq = prefix.iter().map(|record| record.seq).max().unwrap_or(0);
        let full_values = prefix
            .iter()
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        let run_state = reduce_transcript_run_state(&full_values);
        let window_values = prefix
            .iter()
            .filter(|record| record.seq >= floor_seq)
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        let mut snapshot =
            reduce_transcript_render_state_with_run_state(&window_values, &run_state);
        if snapshot.based_on_seq == 0 {
            snapshot.based_on_seq = actual_based_on_seq;
        }
        snapshot.window = Some(RenderWindow {
            floor_seq,
            has_more_above: prefix.iter().any(|record| record.seq < floor_seq),
        });
        Ok(snapshot)
    }

    /// Committed records with `seq > after_seq`, ascending, up to `limit`. Drives
    /// the resumable per-thread stream's replay (catch-up). Optimized for the
    /// caught-up case: it scans the jsonl from the TAIL backward and stops at the
    /// first `seq <= after_seq`, so a near-current cursor parses only the delta
    /// instead of the whole file (seq is monotonic + gapless, so everything before
    /// the boundary is older). A far-behind cursor whose delta exceeds `limit`
    /// yields the NEWEST `limit` (the tail), so the stream's live handoff stays
    /// gapless — the most recent rows are always delivered and the client pages
    /// older history via before_index.
    pub async fn records_after_seq(
        &self,
        thread_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                if !path.exists() {
                    return Ok(Vec::new());
                }
                read_tail_records_after_seq_from_path(thread_id, &path, after_seq, limit).await
            }
            TranscriptStoreMode::Memory { records } => {
                let guard = records.lock().await;
                let mut filtered: Vec<ThreadTranscriptRecord> = guard
                    .get(thread_id)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|record| record.seq > after_seq)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                // Newest `limit` (tail), matching the File mode so an over-limit
                // delta keeps the stream's live handoff gapless.
                if filtered.len() > limit {
                    filtered.drain(0..filtered.len() - limit);
                }
                Ok(filtered)
            }
        }
    }

    /// Oldest committed records with `seq > after_seq`, ascending, up to `limit`.
    /// This is the explicit pagination companion to `records_after_seq`: callers
    /// use the tail scan for the caught-up fast path, then fall back to this
    /// forward page only when the tail page proves the delta exceeded the replay
    /// cap.
    pub async fn records_after_seq_page(
        &self,
        thread_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                if !path.exists() {
                    return Ok(Vec::new());
                }
                read_forward_records_after_seq_from_path(thread_id, &path, after_seq, limit).await
            }
            TranscriptStoreMode::Memory { records } => {
                let guard = records.lock().await;
                Ok(guard
                    .get(thread_id)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|record| record.seq > after_seq)
                            .take(limit)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default())
            }
        }
    }

    pub async fn records_for_run_after_seq(
        &self,
        thread_id: &str,
        run_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        if trimmed_run_id.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                read_run_records_after_seq_from_path(
                    thread_id,
                    &path,
                    trimmed_run_id,
                    after_seq,
                    limit,
                )
                .await
            }
            TranscriptStoreMode::Memory { records } => {
                let guard = records.lock().await;
                Ok(guard
                    .get(thread_id)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|record| {
                                record.seq > after_seq
                                    && (record.run_id.as_deref() == Some(trimmed_run_id)
                                        || (record.run_id.is_none()
                                            && is_control_record_message(&record.message)))
                            })
                            .take(limit)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default())
            }
        }
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
            if message_role(&record.message) == Some(trimmed_role)
                && let Some(text) = message_text(&record.message)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            {
                return Ok(Some(text.to_owned()));
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

fn transcript_io_error(thread_id: &str, error: impl std::fmt::Display) -> ThreadHistoryError {
    ThreadHistoryError::TranscriptIo {
        thread_id: thread_id.to_owned(),
        message: error.to_string(),
    }
}

fn parse_transcript_record_line(
    thread_id: &str,
    line: &str,
    location: impl std::fmt::Display,
) -> Result<Option<ThreadTranscriptRecord>, ThreadHistoryError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let parsed = serde_json::from_str::<TranscriptLine>(line).map_err(|error| {
        ThreadHistoryError::InvalidTranscript {
            thread_id: thread_id.to_owned(),
            message: format!("{location}: {error}"),
        }
    })?;
    Ok(match parsed {
        TranscriptLine::Message {
            seq,
            thread_id,
            run_id,
            timestamp,
            message,
        } => Some(ThreadTranscriptRecord {
            seq,
            thread_id,
            run_id,
            timestamp,
            message,
        }),
        TranscriptLine::Session { .. } => None,
    })
}

fn parse_transcript_record_bytes(
    thread_id: &str,
    line: &[u8],
    location: impl std::fmt::Display,
) -> Result<Option<ThreadTranscriptRecord>, ThreadHistoryError> {
    if line.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(None);
    }
    let line =
        std::str::from_utf8(line).map_err(|error| ThreadHistoryError::InvalidTranscript {
            thread_id: thread_id.to_owned(),
            message: format!("{location}: {error}"),
        })?;
    parse_transcript_record_line(thread_id, line, location)
}

fn collect_tail_scan_line(
    thread_id: &str,
    line: &[u8],
    after_seq: u64,
    limit: usize,
    tail: &mut Vec<ThreadTranscriptRecord>,
) -> Result<bool, ThreadHistoryError> {
    let Some(record) = parse_transcript_record_bytes(thread_id, line, "tail scan")? else {
        return Ok(false);
    };
    if record.seq <= after_seq {
        return Ok(true);
    }
    tail.push(record);
    Ok(tail.len() >= limit)
}

async fn read_tail_records_after_seq_from_path(
    thread_id: &str,
    path: &Path,
    after_seq: u64,
    limit: usize,
) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?;
    let mut position = file
        .metadata()
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?
        .len();
    let mut carry = Vec::new();
    let mut tail = Vec::new();

    while position > 0 && tail.len() < limit {
        let read_len = position.min(TAIL_SCAN_CHUNK_BYTES) as usize;
        position -= read_len as u64;
        file.seek(SeekFrom::Start(position))
            .await
            .map_err(|error| transcript_io_error(thread_id, error))?;
        let mut chunk = vec![0; read_len];
        file.read_exact(&mut chunk)
            .await
            .map_err(|error| transcript_io_error(thread_id, error))?;
        if !carry.is_empty() {
            chunk.extend_from_slice(&carry);
        }

        let mut end = chunk.len();
        while end > 0 {
            let Some(newline_index) = chunk[..end].iter().rposition(|byte| *byte == b'\n') else {
                break;
            };
            if collect_tail_scan_line(
                thread_id,
                &chunk[newline_index + 1..end],
                after_seq,
                limit,
                &mut tail,
            )? {
                tail.reverse();
                return Ok(tail);
            }
            end = newline_index;
        }
        carry.clear();
        carry.extend_from_slice(&chunk[..end]);
    }

    if tail.len() < limit
        && !carry.is_empty()
        && collect_tail_scan_line(thread_id, &carry, after_seq, limit, &mut tail)?
    {
        tail.reverse();
        return Ok(tail);
    }

    tail.reverse();
    Ok(tail)
}

async fn read_forward_records_after_seq_from_path(
    thread_id: &str,
    path: &Path,
    after_seq: u64,
    limit: usize,
) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?;
    let mut lines = BufReader::new(file).lines();
    let mut line_no = 0_usize;
    let mut records = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?
    {
        line_no += 1;
        let Some(record) = parse_transcript_record_line(thread_id, &line, line_no)? else {
            continue;
        };
        if record.seq <= after_seq {
            continue;
        }
        records.push(record);
        if records.len() >= limit {
            break;
        }
    }
    Ok(records)
}

async fn read_run_records_after_seq_from_path(
    thread_id: &str,
    path: &Path,
    run_id: &str,
    after_seq: u64,
    limit: usize,
) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?;
    let mut lines = BufReader::new(file).lines();
    let mut records = Vec::new();
    let mut line_no = 0usize;
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?
    {
        line_no += 1;
        let Some(record) =
            parse_transcript_record_line(thread_id, &line, format!("line {line_no}"))?
        else {
            continue;
        };
        if record.seq > after_seq
            && (record.run_id.as_deref() == Some(run_id)
                || (record.run_id.is_none() && is_control_record_message(&record.message)))
        {
            records.push(record);
            if records.len() >= limit {
                break;
            }
        }
    }
    Ok(records)
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
}

impl ThreadHistoryRepository {
    pub fn new(
        thread_store: Arc<dyn ThreadStore>,
        transcript_store: Arc<ThreadTranscriptStore>,
    ) -> Self {
        Self {
            thread_store,
            transcript_store,
        }
    }

    pub fn transcript_store(&self) -> Arc<ThreadTranscriptStore> {
        self.transcript_store.clone()
    }

    pub async fn thread_snapshot(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        self.thread_snapshot_page(thread_id, limit, None).await
    }

    pub async fn thread_snapshot_page(
        &self,
        thread_id: &str,
        limit: usize,
        before_index: Option<usize>,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;
        let bounded_limit = limit.max(1);

        if let Some(before_index) = before_index {
            let (committed_messages, total_committed_messages, committed_start_index) = self
                .load_committed_messages_before_index(
                    thread_id,
                    &thread_data,
                    before_index,
                    bounded_limit,
                )
                .await?;
            return Ok(ThreadHistorySnapshot {
                thread_id: thread_id.to_owned(),
                thread_data,
                committed_messages,
                total_committed_messages,
                committed_start_index,
            });
        }

        let (committed_messages, total_committed_messages) = self
            .load_committed_messages(thread_id, &thread_data, bounded_limit)
            .await?;
        let committed_start_index =
            total_committed_messages.saturating_sub(committed_messages.len());

        Ok(ThreadHistorySnapshot {
            thread_id: thread_id.to_owned(),
            thread_data,
            committed_messages,
            total_committed_messages,
            committed_start_index,
        })
    }

    /// Forward delta snapshot: committed messages strictly after `after_index`.
    pub async fn thread_snapshot_after_index(
        &self,
        thread_id: &str,
        after_index: usize,
        limit: usize,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;
        let bounded_limit = limit.max(1);
        let (committed_messages, total_committed_messages, committed_start_index) = self
            .load_committed_messages_after_index(
                thread_id,
                &thread_data,
                after_index,
                bounded_limit,
            )
            .await?;
        Ok(ThreadHistorySnapshot {
            thread_id: thread_id.to_owned(),
            thread_data,
            committed_messages,
            total_committed_messages,
            committed_start_index,
        })
    }

    pub async fn thread_snapshot_user_query_page(
        &self,
        thread_id: &str,
        fallback_message_limit: usize,
        before_index: Option<usize>,
        user_query_limit: usize,
    ) -> Result<ThreadHistorySnapshot, ThreadHistoryError> {
        let thread_data = self
            .thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;
        let bounded_fallback_limit = fallback_message_limit.max(1);
        let bounded_user_query_limit = user_query_limit.max(1);

        if let Some(before_index) = before_index {
            let (committed_messages, total_committed_messages, committed_start_index) = self
                .load_committed_messages_before_user_queries(
                    thread_id,
                    &thread_data,
                    Some(before_index),
                    bounded_user_query_limit,
                    bounded_fallback_limit,
                )
                .await?;
            return Ok(ThreadHistorySnapshot {
                thread_id: thread_id.to_owned(),
                thread_data,
                committed_messages,
                total_committed_messages,
                committed_start_index,
            });
        }

        let (committed_messages, total_committed_messages, committed_start_index) = self
            .load_committed_messages_before_user_queries(
                thread_id,
                &thread_data,
                None,
                bounded_user_query_limit,
                bounded_fallback_limit,
            )
            .await?;

        Ok(ThreadHistorySnapshot {
            thread_id: thread_id.to_owned(),
            thread_data,
            committed_messages,
            total_committed_messages,
            committed_start_index,
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
        self.thread_store
            .get(thread_id)
            .await
            .ok_or_else(|| ThreadHistoryError::ThreadNotFound(thread_id.to_owned()))?;

        if self.transcript_store.exists(thread_id).await {
            return self
                .transcript_store
                .find_latest_for_run(thread_id, trimmed_run_id)
                .await;
        }

        Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()))
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

        if self.transcript_store.exists(thread_id).await {
            return self
                .transcript_store
                .find_latest_text_for_role(thread_id, trimmed_role)
                .await;
        }

        if history_message_count(&thread_data) > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
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
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        if has_transcript {
            let total = self.transcript_store.message_count(thread_id).await?;
            if limit == 0 {
                return Ok((Vec::new(), total));
            }
            return Ok((self.transcript_store.tail(thread_id, limit).await?, total));
        }

        Ok((Vec::new(), 0))
    }

    async fn load_committed_messages_before_index(
        &self,
        thread_id: &str,
        thread_data: &Value,
        before_index: usize,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        if has_transcript {
            if limit == 0 {
                let total = self.transcript_store.message_count(thread_id).await?;
                return Ok((Vec::new(), total, before_index.min(total)));
            }
            return self
                .transcript_store
                .page_before_index(thread_id, Some(before_index), limit)
                .await;
        }

        Ok((Vec::new(), 0, 0))
    }

    async fn load_committed_messages_after_index(
        &self,
        thread_id: &str,
        thread_data: &Value,
        after_index: usize,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }
        if has_transcript {
            let total = self.transcript_store.message_count(thread_id).await?;
            if limit == 0 {
                return Ok((Vec::new(), total, after_index.saturating_add(1).min(total)));
            }
            return self
                .transcript_store
                .page_after_index(thread_id, after_index, limit)
                .await;
        }
        Ok((Vec::new(), 0, 0))
    }

    async fn load_committed_messages_before_user_queries(
        &self,
        thread_id: &str,
        thread_data: &Value,
        before_index: Option<usize>,
        user_query_limit: usize,
        fallback_message_limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        let has_transcript = self.transcript_store.exists(thread_id).await;
        let message_count = history_message_count(thread_data);
        if !has_transcript && message_count > 0 {
            return Err(ThreadHistoryError::MissingTranscript(thread_id.to_owned()));
        }

        if has_transcript {
            return self
                .transcript_store
                .page_before_user_queries(
                    thread_id,
                    before_index,
                    user_query_limit,
                    fallback_message_limit,
                )
                .await;
        }

        Ok((Vec::new(), 0, 0))
    }
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
        .unwrap_or(0)
}

pub fn count_user_query_messages(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|message| is_user_query_message(message))
        .count()
}

pub fn is_user_query_message(message: &Value) -> bool {
    message_role(message) == Some("user")
        && message
            .get("internal_kind")
            .and_then(Value::as_str)
            .map(str::trim)
            != Some("loop_continuation")
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

fn record_from_message(thread_id: &str, seq: u64, message: &Value) -> ThreadTranscriptRecord {
    ThreadTranscriptRecord {
        seq,
        thread_id: thread_id.to_owned(),
        run_id: extract_run_id(message),
        timestamp: message_timestamp(message).unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        message: message.clone(),
    }
}

fn record_from_message_replacing(
    thread_id: &str,
    seq: u64,
    message: &Value,
    existing: &ThreadTranscriptRecord,
) -> ThreadTranscriptRecord {
    let mut record = record_from_message(thread_id, seq, message);
    if message_timestamp(message).is_none() {
        record.timestamp = existing.timestamp.clone();
    }
    record
}

fn record_from_draft(
    thread_id: &str,
    run_id: Option<&str>,
    seq: u64,
    draft: &RunTranscriptRecordDraft,
) -> ThreadTranscriptRecord {
    ThreadTranscriptRecord {
        seq,
        thread_id: thread_id.to_owned(),
        run_id: trim_non_empty(run_id),
        timestamp: draft
            .timestamp
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        message: draft.message.clone(),
    }
}

fn record_from_draft_replacing(
    thread_id: &str,
    run_id: Option<&str>,
    seq: u64,
    draft: &RunTranscriptRecordDraft,
    existing: &ThreadTranscriptRecord,
) -> ThreadTranscriptRecord {
    let mut record = record_from_draft(thread_id, run_id, seq, draft);
    if draft.timestamp.is_none() {
        record.timestamp = existing.timestamp.clone();
    }
    record
}

fn reconcile_rewrite_records(
    thread_id: &str,
    existing: &[ThreadTranscriptRecord],
    messages: &[Value],
) -> Vec<ThreadTranscriptRecord> {
    if existing.is_empty() {
        return messages
            .iter()
            .enumerate()
            .map(|(offset, message)| record_from_message(thread_id, offset as u64 + 1, message))
            .collect();
    }

    let existing_identity: Vec<Value> = existing
        .iter()
        .map(|record| message_identity(&record.message))
        .collect();
    let authoritative_identity: Vec<Value> = messages.iter().map(message_identity).collect();

    if existing_identity == authoritative_identity {
        let mut rebuilt = existing.to_vec();
        let mut changed_same_seqs = Vec::new();
        for (offset, message) in messages.iter().enumerate() {
            let current = &existing[offset];
            let replacement =
                record_from_message_replacing(thread_id, current.seq, message, current);
            if replacement.timestamp != current.timestamp || replacement.message != current.message
            {
                changed_same_seqs.push(replacement.seq);
                rebuilt[offset] = replacement;
            }
        }
        append_thread_rewrite_marker_if_needed(
            &mut rebuilt,
            &changed_same_seqs,
            thread_id,
            messages.len(),
            existing.len(),
            "same_seq_overwrite",
        );
        return rebuilt;
    }

    if authoritative_identity.len() > existing_identity.len()
        && authoritative_identity[..existing_identity.len()] == existing_identity[..]
    {
        let mut rebuilt = existing.to_vec();
        let next_seq = rebuilt.last().map(|record| record.seq + 1).unwrap_or(1);
        for (seq, message) in (next_seq..).zip(messages[existing.len()..].iter()) {
            rebuilt.push(record_from_message(thread_id, seq, message));
        }
        return rebuilt;
    }

    if authoritative_identity.len() <= existing_identity.len()
        && existing_identity[..authoritative_identity.len()] == authoritative_identity[..]
        && existing[authoritative_identity.len()..]
            .iter()
            .all(|record| is_range_rewrite_control(&record.message))
    {
        let mut rebuilt = existing.to_vec();
        let mut changed_same_seqs = Vec::new();
        for (offset, message) in messages.iter().enumerate() {
            let current = &existing[offset];
            let replacement =
                record_from_message_replacing(thread_id, current.seq, message, current);
            if replacement.timestamp != current.timestamp || replacement.message != current.message
            {
                changed_same_seqs.push(replacement.seq);
                rebuilt[offset] = replacement;
            }
        }
        append_thread_rewrite_marker_if_needed(
            &mut rebuilt,
            &changed_same_seqs,
            thread_id,
            messages.len(),
            existing.len(),
            "same_seq_overwrite",
        );
        return rebuilt;
    }

    if messages.len() >= existing.len() {
        let mut rebuilt = Vec::with_capacity(messages.len() + 1);
        let mut changed_same_seqs = Vec::new();
        let mut next_seq = existing.first().map(|record| record.seq).unwrap_or(1);
        for (offset, message) in messages.iter().enumerate() {
            let seq = existing
                .get(offset)
                .map(|record| record.seq)
                .unwrap_or(next_seq);
            let replacement = if let Some(current) = existing.get(offset) {
                record_from_message_replacing(thread_id, seq, message, current)
            } else {
                record_from_message(thread_id, seq, message)
            };
            if let Some(current) = existing.get(offset)
                && (replacement.timestamp != current.timestamp
                    || replacement.message != current.message)
            {
                changed_same_seqs.push(replacement.seq);
            }
            rebuilt.push(replacement);
            next_seq = seq + 1;
        }
        append_thread_rewrite_marker_if_needed(
            &mut rebuilt,
            &changed_same_seqs,
            thread_id,
            messages.len(),
            existing.len(),
            "same_seq_overwrite",
        );
        return rebuilt;
    }

    let mut rebuilt = Vec::with_capacity(existing.len() + 1);
    let mut changed_same_seqs = Vec::new();
    let first_rewritten_seq = existing
        .get(messages.len())
        .map(|record| record.seq)
        .unwrap_or_else(|| existing.first().map(|record| record.seq).unwrap_or(1));
    let last_rewritten_seq = existing
        .last()
        .map(|record| record.seq)
        .unwrap_or(first_rewritten_seq);
    let rewrite_at = chrono::Utc::now().to_rfc3339();
    for (offset, current) in existing.iter().enumerate() {
        if let Some(message) = messages.get(offset) {
            let replacement =
                record_from_message_replacing(thread_id, current.seq, message, current);
            if replacement.timestamp != current.timestamp || replacement.message != current.message
            {
                changed_same_seqs.push(replacement.seq);
            }
            rebuilt.push(replacement);
        } else {
            let rewrite = build_range_rewrite_record(
                thread_id,
                None,
                current.seq,
                first_rewritten_seq,
                last_rewritten_seq,
                messages.len(),
                existing.len(),
                true,
                "rewrite_from_messages_shrink",
                &rewrite_at,
            );
            if rewrite.timestamp != current.timestamp || rewrite.message != current.message {
                changed_same_seqs.push(rewrite.seq);
            }
            rebuilt.push(rewrite);
        }
    }

    let first_rewritten_seq = changed_same_seqs
        .iter()
        .copied()
        .min()
        .unwrap_or(first_rewritten_seq);
    let last_rewritten_seq = changed_same_seqs
        .iter()
        .copied()
        .max()
        .unwrap_or(last_rewritten_seq);
    let marker = build_range_rewrite_record(
        thread_id,
        None,
        rebuilt.last().map(|record| record.seq + 1).unwrap_or(1),
        first_rewritten_seq,
        last_rewritten_seq,
        messages.len(),
        existing.len(),
        false,
        "rewrite_from_messages_shrink",
        &rewrite_at,
    );
    rebuilt.push(marker);
    rebuilt
}

fn append_thread_rewrite_marker_if_needed(
    records: &mut Vec<ThreadTranscriptRecord>,
    changed_same_seqs: &[u64],
    thread_id: &str,
    authoritative_len: usize,
    existing_len: usize,
    reason: &str,
) {
    let (Some(start_seq), Some(end_seq)) = (
        changed_same_seqs.iter().copied().min(),
        changed_same_seqs.iter().copied().max(),
    ) else {
        return;
    };
    let mut ignored_changed = Vec::new();
    append_range_rewrite_marker(
        records,
        &mut ignored_changed,
        thread_id,
        None,
        start_seq,
        end_seq,
        authoritative_len,
        existing_len,
        reason,
    );
}

#[allow(clippy::too_many_arguments)]
fn build_range_rewrite_record(
    thread_id: &str,
    run_id: Option<&str>,
    seq: u64,
    start_seq: u64,
    end_seq: u64,
    authoritative_len: usize,
    existing_len: usize,
    tombstone: bool,
    reason: &str,
    at: &str,
) -> ThreadTranscriptRecord {
    let mut message = serde_json::json!({
        "role": "system",
        "kind": "control",
        "internal": true,
        "internal_kind": "control",
        "control": {
            "kind": "range_rewrite",
            "thread_id": thread_id,
            "start_seq": start_seq,
            "end_seq": end_seq,
            "tombstone": tombstone,
            "record_seq": seq,
            "authoritative_record_count": authoritative_len,
            "existing_record_count": existing_len,
            "reason": reason,
            "at": at,
        }
    });
    if let Some(run_id) = run_id
        && let Some(control) = message.get_mut("control").and_then(Value::as_object_mut)
    {
        control.insert("run_id".to_owned(), Value::String(run_id.to_owned()));
    }
    ThreadTranscriptRecord {
        seq,
        thread_id: thread_id.to_owned(),
        run_id: trim_non_empty(run_id),
        timestamp: at.to_owned(),
        message,
    }
}

#[allow(clippy::too_many_arguments)]
fn append_range_rewrite_marker(
    records: &mut Vec<ThreadTranscriptRecord>,
    changed: &mut Vec<ThreadTranscriptRecord>,
    thread_id: &str,
    run_id: Option<&str>,
    start_seq: u64,
    end_seq: u64,
    authoritative_len: usize,
    existing_len: usize,
    reason: &str,
) {
    let at = chrono::Utc::now().to_rfc3339();
    let marker = build_range_rewrite_record(
        thread_id,
        run_id,
        records.last().map(|record| record.seq + 1).unwrap_or(1),
        start_seq,
        end_seq,
        authoritative_len,
        existing_len,
        false,
        reason,
        &at,
    );
    changed.push(marker.clone());
    records.push(marker);
}

fn is_range_rewrite_control(value: &Value) -> bool {
    value
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
        == Some("range_rewrite")
}

fn is_control_record_message(value: &Value) -> bool {
    value
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
        .is_some()
}

/// A message's logical identity for run-tail reconciliation: everything except
/// the cosmetic fields that legitimately differ between the streamed copy and
/// the terminal rebuild of the same message. The SDK session id is bound mid-run
/// (so the first-flush user row has `None` while the terminal rebuild has `Some`)
/// and timestamps can be backfilled, so both are stripped. Role, content, and
/// tool fields — the things that actually distinguish one message from another —
/// are preserved, so a genuine content change still reads as a divergence.
fn message_identity(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("timestamp");
        object.remove("sdk_session_id");
        if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
            metadata.remove("sdk_session_id");
        }
        if let Some(control) = object.get_mut("control").and_then(Value::as_object_mut) {
            control.remove("at");
            control.remove("duration_ms");
            control.remove("error");
            control.remove("status");
        }
    }
    value
}

#[cfg(test)]
mod tests;
