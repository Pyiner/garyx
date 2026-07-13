use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use garyx_models::{
    RenderSnapshot, RenderWindow, TranscriptRenderPrefixState, TranscriptRunState,
    apply_transcript_record, apply_transcript_render_prefix_record,
    reduce_transcript_render_prefix_state, reduce_transcript_render_state,
    reduce_transcript_render_state_with_prefix_state, reduce_transcript_run_state,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::file_store::thread_storage_file_name;
use crate::store::ThreadStore;

mod model;
mod reconcile;
mod repository;
mod store;

pub use model::{
    RunTranscriptRecordDraft, ThreadHistoryError, ThreadHistorySnapshot, ThreadTranscriptRecord,
    ThreadTranscriptWindow, TranscriptAppendRecordsResult, TranscriptAppendResult,
};
pub use reconcile::{
    count_user_query_messages, extract_run_id, history_message_count, is_user_query_message,
    message_text,
};
pub use repository::ThreadHistoryRepository;
pub use store::ThreadTranscriptStore;

use reconcile::*;
#[cfg(test)]
use store::*;

pub const DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT: usize = 100;
pub const RECENT_COMMITTED_RUN_IDS_LIMIT: usize = 256;
pub const THREAD_TRANSCRIPT_REPLAY_CAP: usize = 10_000;

#[cfg(test)]
mod tests;
