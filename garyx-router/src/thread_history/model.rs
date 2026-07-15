use super::*;

#[derive(Debug, thiserror::Error)]
pub enum ThreadHistoryError {
    #[error("thread not found: {0}")]
    ThreadNotFound(String),
    #[error("thread store failed: {0}")]
    Storage(String),
    #[error("missing transcript for thread: {0}")]
    MissingTranscript(String),
    #[error("transcript io error for thread {thread_id}: {message}")]
    TranscriptIo { thread_id: String, message: String },
    #[error("invalid transcript for thread {thread_id}: {message}")]
    InvalidTranscript { thread_id: String, message: String },
    #[error("atomic transcript replace failed for thread {thread_id} at {stage}: {message}")]
    AtomicReplace {
        thread_id: String,
        stage: TranscriptReplaceStage,
        message: String,
    },
}

/// Durable stages of a whole-transcript replacement. Failures before the
/// parent-directory fsync leave the old target untouched; a parent-fsync
/// failure is reported after the complete new target has been renamed into
/// place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptReplaceStage {
    TempWrite,
    FileFsync,
    Rename,
    ParentFsync,
}

impl std::fmt::Display for TranscriptReplaceStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TempWrite => "temp_write",
            Self::FileFsync => "file_fsync",
            Self::Rename => "rename",
            Self::ParentFsync => "parent_fsync",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillOutcome {
    /// The target was absent, empty, a strict identity prefix, or had a
    /// recoverable torn tail and was replaced atomically.
    Backfilled,
    /// The existing logical message sequence already matched the archive.
    AlreadyComplete,
    /// The structurally valid transcript has evolved beyond the archive and
    /// therefore remains authoritative.
    PreservedDiverged,
}

impl BackfillOutcome {
    pub fn wrote_transcript(self) -> bool {
        matches!(self, Self::Backfilled)
    }
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

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadTranscriptWindow {
    pub records: Vec<ThreadTranscriptRecord>,
    pub floor_seq: u64,
    pub has_more_above: bool,
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
