use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Mutex as StdMutex, RwLock as StdRwLock};

use chrono::{DateTime, Utc};
use garyx_models::config::ConversationIndexConfig;
use garyx_models::provider::ProviderMessage;
use reqwest::Client;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tracing::warn;

use crate::{
    ThreadStore, ThreadTranscriptRecord, ThreadTranscriptStore, is_thread_key, message_text,
    workspace_dir_from_value,
};

const CHUNK_MESSAGE_LIMIT: usize = 6;
const CHUNK_MESSAGE_OVERLAP: usize = 2;
const CHUNK_MAX_CHARS: usize = 1_200;
const VECTOR_WEIGHT: f32 = 0.85;
const KEYWORD_WEIGHT: f32 = 0.15;
const MIN_SEARCH_SCORE: f32 = 0.18;
const MIN_VECTOR_ONLY_SCORE: f32 = 0.30;
const MAX_EMBED_BATCH_SIZE: usize = 64;

#[derive(Debug, Clone)]
pub struct ConversationIndexSearchRequest {
    pub query: String,
    pub thread_id: Option<String>,
    pub workspace_dir: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: usize,
}

#[derive(Debug, Clone)]
pub struct ConversationIndexSearchHit {
    pub score: f32,
    pub vector_score: f32,
    pub keyword_score: f32,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub transcript_file: Option<String>,
    pub start_timestamp: Option<DateTime<Utc>>,
    pub end_timestamp: Option<DateTime<Utc>>,
    pub message_count: usize,
    pub snippet: String,
    pub start_seq: u64,
    pub end_seq: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ConversationIndexResult {
    pub threads_considered: usize,
    pub indexed_threads: usize,
    pub candidate_chunks: usize,
    pub results: Vec<ConversationIndexSearchHit>,
}

#[derive(Debug, Clone)]
struct VisibleMessage {
    seq: u64,
    timestamp: Option<DateTime<Utc>>,
    role: String,
    text: String,
}

#[derive(Debug, Clone)]
struct IndexedChunk {
    thread_id: String,
    workspace_dir: Option<String>,
    transcript_file: Option<String>,
    start_seq: u64,
    end_seq: u64,
    start_timestamp: Option<DateTime<Utc>>,
    end_timestamp: Option<DateTime<Utc>>,
    message_count: usize,
    snippet: String,
    search_text: String,
    embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
struct StoredChunk {
    thread_id: String,
    workspace_dir: Option<String>,
    transcript_file: Option<String>,
    start_seq: u64,
    end_seq: u64,
    start_timestamp: Option<DateTime<Utc>>,
    end_timestamp: Option<DateTime<Utc>>,
    message_count: usize,
    snippet: String,
    search_text: String,
    embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
struct SearchCandidate {
    chunk: StoredChunk,
    score: f32,
    vector_score: f32,
    keyword_score: f32,
}

#[derive(Debug, Default)]
struct EnqueueState {
    pending: HashSet<String>,
    rerun: HashSet<String>,
}

#[derive(Debug, Clone)]
struct QueryModel {
    normalized_query: String,
    lexical_terms: Vec<String>,
    embedding: Vec<f32>,
}

#[derive(Debug)]
struct StoredThreadState {
    last_seq: u64,
    transcript_messages: usize,
    workspace_dir: Option<String>,
    transcript_file: Option<String>,
}

pub struct ConversationIndexManager {
    thread_store: Arc<dyn ThreadStore>,
    transcript_store: Arc<ThreadTranscriptStore>,
    db_path: PathBuf,
    client: Client,
    config: StdRwLock<ConversationIndexConfig>,
    enqueue_state: StdMutex<EnqueueState>,
    backfill_started: AtomicBool,
    job_tx: mpsc::UnboundedSender<String>,
}

impl ConversationIndexManager {
    pub async fn new(
        thread_store: Arc<dyn ThreadStore>,
        transcript_store: Arc<ThreadTranscriptStore>,
        db_path: PathBuf,
        config: ConversationIndexConfig,
    ) -> Result<Arc<Self>, String> {
        initialize_schema(&db_path).await?;
        let (job_tx, job_rx) = mpsc::unbounded_channel();
        let manager = Arc::new(Self {
            thread_store,
            transcript_store,
            db_path,
            client: Client::builder()
                .build()
                .map_err(|error| format!("failed to build conversation index client: {error}"))?,
            config: StdRwLock::new(config),
            enqueue_state: StdMutex::new(EnqueueState::default()),
            backfill_started: AtomicBool::new(false),
            job_tx,
        });
        tokio::spawn(manager.clone().worker_loop(job_rx));
        Ok(manager)
    }

    pub fn update_config(self: &Arc<Self>, config: ConversationIndexConfig) {
        let was_enabled = self.is_enabled();
        *write_lock(&self.config) = config;
        if !was_enabled && self.is_enabled() {
            self.schedule_full_backfill();
        }
    }

    pub fn config_snapshot(&self) -> ConversationIndexConfig {
        read_lock(&self.config).clone()
    }

    pub fn is_enabled(&self) -> bool {
        let config = self.config_snapshot();
        config.enabled && resolve_openai_api_key(&config.api_key).is_some()
    }

    pub fn enqueue_thread(self: &Arc<Self>, thread_id: &str) {
        let trimmed = thread_id.trim();
        if trimmed.is_empty() || !self.is_enabled() {
            return;
        }

        let should_send = {
            let mut guard = mutex_lock(&self.enqueue_state);
            if guard.pending.contains(trimmed) {
                guard.rerun.insert(trimmed.to_owned());
                false
            } else {
                guard.pending.insert(trimmed.to_owned());
                true
            }
        };

        if should_send {
            let _ = self.job_tx.send(trimmed.to_owned());
        }
    }

    pub fn schedule_full_backfill(self: &Arc<Self>) {
        if !self.is_enabled() {
            return;
        }
        if self.backfill_started.swap(true, AtomicOrdering::SeqCst) {
            return;
        }

        let manager = self.clone();
        tokio::spawn(async move {
            for key in manager.thread_store.list_keys(None).await {
                if is_thread_key(&key) {
                    manager.enqueue_thread(&key);
                }
            }
            manager
                .backfill_started
                .store(false, AtomicOrdering::SeqCst);
        });
    }

    pub async fn search(
        &self,
        request: ConversationIndexSearchRequest,
    ) -> Result<Option<ConversationIndexResult>, String> {
        if !self.is_enabled() {
            return Ok(None);
        }

        let query = request.query.trim();
        if query.is_empty() {
            return Err("missing required parameter: query".to_owned());
        }

        let loaded = load_chunks(
            &self.db_path,
            request.thread_id.as_deref(),
            request.workspace_dir.as_deref(),
            request.from.as_ref(),
            request.to.as_ref(),
        )
        .await?;
        if loaded.candidate_chunks == 0 {
            return Ok(Some(ConversationIndexResult {
                threads_considered: loaded.threads_considered,
                indexed_threads: loaded.indexed_threads,
                candidate_chunks: 0,
                results: Vec::new(),
            }));
        }

        let query_embedding = self.embed_inputs(&[query.to_owned()]).await?;
        let query_model = QueryModel {
            normalized_query: sanitize_visible_text(query).to_ascii_lowercase(),
            lexical_terms: lexical_terms(query),
            embedding: query_embedding.into_iter().next().unwrap_or_default(),
        };

        let mut matches = loaded
            .chunks
            .into_iter()
            .filter_map(|chunk| score_candidate(chunk, &query_model))
            .collect::<Vec<_>>();
        matches.sort_by(compare_candidates);

        let mut selected = Vec::new();
        for candidate in matches {
            if selected
                .iter()
                .any(|existing: &ConversationIndexSearchHit| hits_overlap(existing, &candidate))
            {
                continue;
            }
            selected.push(ConversationIndexSearchHit {
                score: candidate.score,
                vector_score: candidate.vector_score,
                keyword_score: candidate.keyword_score,
                thread_id: candidate.chunk.thread_id,
                workspace_dir: candidate.chunk.workspace_dir,
                transcript_file: candidate.chunk.transcript_file,
                start_timestamp: candidate.chunk.start_timestamp,
                end_timestamp: candidate.chunk.end_timestamp,
                message_count: candidate.chunk.message_count,
                snippet: candidate.chunk.snippet,
                start_seq: candidate.chunk.start_seq,
                end_seq: candidate.chunk.end_seq,
            });
            if selected.len() >= request.limit.max(1) {
                break;
            }
        }

        Ok(Some(ConversationIndexResult {
            threads_considered: loaded.threads_considered,
            indexed_threads: loaded.indexed_threads,
            candidate_chunks: loaded.candidate_chunks,
            results: selected,
        }))
    }

    async fn worker_loop(self: Arc<Self>, mut job_rx: mpsc::UnboundedReceiver<String>) {
        while let Some(thread_id) = job_rx.recv().await {
            if let Err(error) = self.reindex_thread(&thread_id).await {
                warn!(thread_id = %thread_id, error = %error, "conversation index update failed");
            }
            self.finish_job(&thread_id);
        }
    }

    fn finish_job(self: &Arc<Self>, thread_id: &str) {
        let should_rerun = {
            let mut guard = mutex_lock(&self.enqueue_state);
            guard.pending.remove(thread_id);
            guard.rerun.remove(thread_id)
        };
        if should_rerun {
            self.enqueue_thread(thread_id);
        }
    }

    async fn reindex_thread(&self, thread_id: &str) -> Result<(), String> {
        let config = self.config_snapshot();
        if !config.enabled {
            return Ok(());
        }
        resolve_openai_api_key(&config.api_key).ok_or_else(|| {
            "conversation index enabled but OpenAI API key is missing: set gateway.conversation_index.api_key in garyx.json or OPENAI_API_KEY".to_owned()
        })?;

        let Some(thread_data) = self.thread_store.get(thread_id).await else {
            delete_thread_chunks(&self.db_path, thread_id).await?;
            return Ok(());
        };

        let workspace_dir = workspace_dir_from_value(&thread_data);
        let transcript_file = self
            .transcript_store
            .transcript_path(thread_id)
            .map(|path| path.display().to_string());

        if !self.transcript_store.exists(thread_id).await {
            delete_thread_chunks(&self.db_path, thread_id).await?;
            return Ok(());
        }

        let records = self
            .transcript_store
            .records(thread_id)
            .await
            .map_err(|error| format!("failed to load transcript for {thread_id}: {error}"))?;
        let transcript_messages = records.len();
        let last_seq = records.last().map(|record| record.seq).unwrap_or(0);

        if let Some(state) = load_thread_state(&self.db_path, thread_id).await? {
            if state.last_seq == last_seq
                && state.transcript_messages == transcript_messages
                && state.workspace_dir == workspace_dir
                && state.transcript_file == transcript_file
            {
                return Ok(());
            }
        }

        let visible_messages = build_visible_messages(&records);
        let mut chunks = build_chunks(
            thread_id,
            workspace_dir.as_deref(),
            transcript_file.as_deref(),
            &visible_messages,
        );

        if !chunks.is_empty() {
            let embeddings = self
                .embed_inputs(
                    &chunks
                        .iter()
                        .map(|chunk| chunk.search_text.clone())
                        .collect::<Vec<_>>(),
                )
                .await?;
            for (chunk, embedding) in chunks.iter_mut().zip(embeddings) {
                chunk.embedding = embedding;
            }
        }

        replace_thread_chunks(
            &self.db_path,
            thread_id,
            workspace_dir.as_deref(),
            transcript_file.as_deref(),
            last_seq,
            transcript_messages,
            &config.model,
            &chunks,
        )
        .await
    }

    async fn embed_inputs(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let config = self.config_snapshot();
        let api_key = resolve_openai_api_key(&config.api_key).ok_or_else(|| {
            "conversation index enabled but OpenAI API key is missing: set gateway.conversation_index.api_key in garyx.json or OPENAI_API_KEY".to_owned()
        })?;
        let url = format!("{}/embeddings", config.base_url.trim_end_matches('/'));

        let mut embeddings = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(MAX_EMBED_BATCH_SIZE) {
            let response = self
                .client
                .post(&url)
                .bearer_auth(&api_key)
                .json(&json!({
                    "model": config.model,
                    "input": batch,
                    "encoding_format": "float",
                }))
                .send()
                .await
                .map_err(|error| format!("conversation index embedding request failed: {error}"))?;
            let status = response.status();
            let body: Value = response.json().await.map_err(|error| {
                format!("conversation index embedding response parse failed: {error}")
            })?;
            if !status.is_success() {
                return Err(format!(
                    "conversation index embedding request failed with status {status}: {}",
                    body
                ));
            }

            let batch_embeddings = body
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| "conversation index embedding response missing `data`".to_owned())?
                .iter()
                .map(parse_embedding_row)
                .collect::<Result<Vec<_>, _>>()?;
            embeddings.extend(batch_embeddings);
        }

        Ok(embeddings)
    }
}

#[derive(Debug)]
struct LoadedChunks {
    threads_considered: usize,
    indexed_threads: usize,
    candidate_chunks: usize,
    chunks: Vec<StoredChunk>,
}

async fn initialize_schema(db_path: &Path) -> Result<(), String> {
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create conversation index dir: {error}"))?;
    }
    let path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = Connection::open(path)
            .map_err(|error| format!("failed to open conversation index db: {error}"))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS thread_state (
                thread_id TEXT PRIMARY KEY,
                workspace_dir TEXT,
                transcript_file TEXT,
                last_seq INTEGER NOT NULL,
                transcript_messages INTEGER NOT NULL,
                indexed_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chunks (
                chunk_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                workspace_dir TEXT,
                transcript_file TEXT,
                start_seq INTEGER NOT NULL,
                end_seq INTEGER NOT NULL,
                start_timestamp TEXT,
                end_timestamp TEXT,
                message_count INTEGER NOT NULL,
                snippet TEXT NOT NULL,
                search_text TEXT NOT NULL,
                embedding_json TEXT NOT NULL,
                embedding_model TEXT NOT NULL,
                indexed_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_chunks_thread_time
                ON chunks(thread_id, workspace_dir, start_timestamp, end_timestamp);
            CREATE INDEX IF NOT EXISTS idx_chunks_workspace
                ON chunks(workspace_dir);
            ",
        )
        .map_err(|error| format!("failed to initialize conversation index schema: {error}"))?;
        Ok(())
    })
    .await
    .map_err(|error| format!("conversation index init task failed: {error}"))?
}

async fn load_thread_state(
    db_path: &Path,
    thread_id: &str,
) -> Result<Option<StoredThreadState>, String> {
    let path = db_path.to_path_buf();
    let thread_id = thread_id.to_owned();
    tokio::task::spawn_blocking(move || -> Result<Option<StoredThreadState>, String> {
        let conn = Connection::open(path)
            .map_err(|error| format!("failed to open conversation index db: {error}"))?;
        conn.query_row(
            "
            SELECT last_seq, transcript_messages, workspace_dir, transcript_file
            FROM thread_state
            WHERE thread_id = ?1
            ",
            params![thread_id],
            |row| {
                Ok(StoredThreadState {
                    last_seq: row.get::<_, u64>(0)?,
                    transcript_messages: row.get::<_, usize>(1)?,
                    workspace_dir: row.get::<_, Option<String>>(2)?,
                    transcript_file: row.get::<_, Option<String>>(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("failed to load conversation index state: {error}"))
    })
    .await
    .map_err(|error| format!("conversation index state task failed: {error}"))?
}

async fn replace_thread_chunks(
    db_path: &Path,
    thread_id: &str,
    workspace_dir: Option<&str>,
    transcript_file: Option<&str>,
    last_seq: u64,
    transcript_messages: usize,
    embedding_model: &str,
    chunks: &[IndexedChunk],
) -> Result<(), String> {
    let path = db_path.to_path_buf();
    let thread_id = thread_id.to_owned();
    let workspace_dir = workspace_dir.map(ToOwned::to_owned);
    let transcript_file = transcript_file.map(ToOwned::to_owned);
    let embedding_model = embedding_model.to_owned();
    let chunks = chunks.to_vec();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = Connection::open(path)
            .map_err(|error| format!("failed to open conversation index db: {error}"))?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("failed to start conversation index transaction: {error}"))?;
        tx.execute(
            "DELETE FROM chunks WHERE thread_id = ?1",
            params![thread_id.clone()],
        )
        .map_err(|error| format!("failed to clear previous conversation index chunks: {error}"))?;

        let indexed_at = Utc::now().to_rfc3339();
        tx.execute(
            "
            INSERT INTO thread_state (
                thread_id,
                workspace_dir,
                transcript_file,
                last_seq,
                transcript_messages,
                indexed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(thread_id) DO UPDATE SET
                workspace_dir = excluded.workspace_dir,
                transcript_file = excluded.transcript_file,
                last_seq = excluded.last_seq,
                transcript_messages = excluded.transcript_messages,
                indexed_at = excluded.indexed_at
            ",
            params![
                thread_id.clone(),
                workspace_dir.clone(),
                transcript_file.clone(),
                last_seq,
                transcript_messages,
                indexed_at.clone(),
            ],
        )
        .map_err(|error| format!("failed to upsert conversation index state: {error}"))?;

        for chunk in &chunks {
            let chunk_id = format!("{}:{}-{}", chunk.thread_id, chunk.start_seq, chunk.end_seq);
            let embedding_json = serde_json::to_string(&chunk.embedding).map_err(|error| {
                format!("failed to serialize conversation index embedding: {error}")
            })?;
            tx.execute(
                "
                INSERT INTO chunks (
                    chunk_id,
                    thread_id,
                    workspace_dir,
                    transcript_file,
                    start_seq,
                    end_seq,
                    start_timestamp,
                    end_timestamp,
                    message_count,
                    snippet,
                    search_text,
                    embedding_json,
                    embedding_model,
                    indexed_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ",
                params![
                    chunk_id,
                    chunk.thread_id.clone(),
                    chunk.workspace_dir.clone(),
                    chunk.transcript_file.clone(),
                    chunk.start_seq,
                    chunk.end_seq,
                    chunk.start_timestamp.as_ref().map(DateTime::to_rfc3339),
                    chunk.end_timestamp.as_ref().map(DateTime::to_rfc3339),
                    chunk.message_count,
                    chunk.snippet.clone(),
                    chunk.search_text.clone(),
                    embedding_json,
                    embedding_model.clone(),
                    indexed_at.clone(),
                ],
            )
            .map_err(|error| format!("failed to insert conversation index chunk: {error}"))?;
        }

        tx.commit()
            .map_err(|error| format!("failed to commit conversation index transaction: {error}"))?;
        Ok(())
    })
    .await
    .map_err(|error| format!("conversation index write task failed: {error}"))?
}

async fn delete_thread_chunks(db_path: &Path, thread_id: &str) -> Result<(), String> {
    let path = db_path.to_path_buf();
    let thread_id = thread_id.to_owned();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = Connection::open(path)
            .map_err(|error| format!("failed to open conversation index db: {error}"))?;
        let tx = conn.transaction().map_err(|error| {
            format!("failed to start conversation index delete transaction: {error}")
        })?;
        tx.execute(
            "DELETE FROM chunks WHERE thread_id = ?1",
            params![thread_id.clone()],
        )
        .map_err(|error| format!("failed to delete conversation index chunks: {error}"))?;
        tx.execute(
            "DELETE FROM thread_state WHERE thread_id = ?1",
            params![thread_id],
        )
        .map_err(|error| format!("failed to delete conversation index state: {error}"))?;
        tx.commit()
            .map_err(|error| format!("failed to commit conversation index delete: {error}"))?;
        Ok(())
    })
    .await
    .map_err(|error| format!("conversation index delete task failed: {error}"))?
}

async fn load_chunks(
    db_path: &Path,
    thread_id: Option<&str>,
    workspace_dir: Option<&str>,
    from: Option<&DateTime<Utc>>,
    to: Option<&DateTime<Utc>>,
) -> Result<LoadedChunks, String> {
    let path = db_path.to_path_buf();
    let thread_id = thread_id.map(ToOwned::to_owned);
    let workspace_dir = workspace_dir.map(ToOwned::to_owned);
    let from = from.map(DateTime::to_rfc3339);
    let to = to.map(DateTime::to_rfc3339);
    tokio::task::spawn_blocking(move || -> Result<LoadedChunks, String> {
        let conn = Connection::open(path)
            .map_err(|error| format!("failed to open conversation index db: {error}"))?;

        let threads_considered = conn
            .query_row(
                "
                SELECT COUNT(*)
                FROM thread_state
                WHERE (?1 IS NULL OR thread_id = ?1)
                  AND (?2 IS NULL OR workspace_dir = ?2)
                ",
                params![thread_id.clone(), workspace_dir.clone()],
                |row| row.get::<_, usize>(0),
            )
            .map_err(|error| format!("failed to count indexed conversation threads: {error}"))?;

        let mut statement = conn
            .prepare(
                "
                SELECT
                    thread_id,
                    workspace_dir,
                    transcript_file,
                    start_seq,
                    end_seq,
                    start_timestamp,
                    end_timestamp,
                    message_count,
                    snippet,
                    search_text,
                    embedding_json
                FROM chunks
                WHERE (?1 IS NULL OR thread_id = ?1)
                  AND (?2 IS NULL OR workspace_dir = ?2)
                  AND (?3 IS NULL OR (end_timestamp IS NOT NULL AND end_timestamp >= ?3))
                  AND (?4 IS NULL OR (start_timestamp IS NOT NULL AND start_timestamp <= ?4))
                ",
            )
            .map_err(|error| format!("failed to prepare conversation index query: {error}"))?;

        let rows = statement
            .query_map(params![thread_id, workspace_dir, from, to], |row| {
                let embedding_json: String = row.get(10)?;
                let embedding: Vec<f32> = serde_json::from_str(&embedding_json).unwrap_or_default();
                Ok(StoredChunk {
                    thread_id: row.get(0)?,
                    workspace_dir: row.get(1)?,
                    transcript_file: row.get(2)?,
                    start_seq: row.get(3)?,
                    end_seq: row.get(4)?,
                    start_timestamp: row
                        .get::<_, Option<String>>(5)?
                        .as_deref()
                        .and_then(parse_timestamp),
                    end_timestamp: row
                        .get::<_, Option<String>>(6)?
                        .as_deref()
                        .and_then(parse_timestamp),
                    message_count: row.get(7)?,
                    snippet: row.get(8)?,
                    search_text: row.get(9)?,
                    embedding,
                })
            })
            .map_err(|error| format!("failed to load conversation index chunks: {error}"))?;

        let mut indexed_threads = HashSet::new();
        let mut chunks = Vec::new();
        for row in rows {
            let chunk =
                row.map_err(|error| format!("failed to read conversation index row: {error}"))?;
            indexed_threads.insert(chunk.thread_id.clone());
            chunks.push(chunk);
        }

        Ok(LoadedChunks {
            threads_considered,
            indexed_threads: indexed_threads.len(),
            candidate_chunks: chunks.len(),
            chunks,
        })
    })
    .await
    .map_err(|error| format!("conversation index read task failed: {error}"))?
}

fn build_visible_messages(records: &[ThreadTranscriptRecord]) -> Vec<VisibleMessage> {
    records
        .iter()
        .filter_map(|record| {
            let object = record.message.as_object()?;
            let role = object
                .get("role")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_ascii_lowercase();
            if !matches!(role.as_str(), "user" | "assistant") {
                return None;
            }
            if is_tool_related_message(&role, object) {
                return None;
            }
            let text = extract_visible_text(&record.message)?;
            Some(VisibleMessage {
                seq: record.seq,
                timestamp: parse_timestamp(&record.timestamp),
                role,
                text,
            })
        })
        .collect()
}

fn build_chunks(
    thread_id: &str,
    workspace_dir: Option<&str>,
    transcript_file: Option<&str>,
    messages: &[VisibleMessage],
) -> Vec<IndexedChunk> {
    let mut by_thread = BTreeMap::<String, Vec<VisibleMessage>>::new();
    by_thread
        .entry(thread_id.to_owned())
        .or_default()
        .extend(messages.iter().cloned());

    let mut chunks = Vec::new();
    for thread_messages in by_thread.values_mut() {
        thread_messages.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.seq.cmp(&right.seq))
        });

        let mut start = 0usize;
        while start < thread_messages.len() {
            let mut end = (start + CHUNK_MESSAGE_LIMIT).min(thread_messages.len());
            while end > start + 1
                && chunk_char_count(&thread_messages[start..end]) > CHUNK_MAX_CHARS
            {
                end -= 1;
            }

            let slice = &thread_messages[start..end];
            if let Some(chunk) = build_chunk(thread_id, workspace_dir, transcript_file, slice) {
                chunks.push(chunk);
            }
            if end >= thread_messages.len() {
                break;
            }
            let next_start = end.saturating_sub(CHUNK_MESSAGE_OVERLAP);
            start = if next_start <= start {
                start + 1
            } else {
                next_start
            };
        }
    }
    chunks
}

fn build_chunk(
    thread_id: &str,
    workspace_dir: Option<&str>,
    transcript_file: Option<&str>,
    messages: &[VisibleMessage],
) -> Option<IndexedChunk> {
    let first = messages.first()?;
    let last = messages.last()?;
    let snippet = messages
        .iter()
        .map(|message| format!("{}: {}", message.role, sanitize_visible_text(&message.text)))
        .collect::<Vec<_>>()
        .join("\n");
    let search_text = snippet.clone();

    Some(IndexedChunk {
        thread_id: thread_id.to_owned(),
        workspace_dir: workspace_dir.map(ToOwned::to_owned),
        transcript_file: transcript_file.map(ToOwned::to_owned),
        start_seq: first.seq,
        end_seq: last.seq,
        start_timestamp: first.timestamp,
        end_timestamp: last.timestamp,
        message_count: messages.len(),
        snippet,
        search_text,
        embedding: Vec::new(),
    })
}

fn chunk_char_count(messages: &[VisibleMessage]) -> usize {
    messages
        .iter()
        .map(|message| sanitize_visible_text(&message.text).len() + message.role.len() + 3)
        .sum()
}

fn score_candidate(chunk: StoredChunk, query: &QueryModel) -> Option<SearchCandidate> {
    let vector_score = cosine_similarity(&query.embedding, &chunk.embedding).max(0.0);
    let keyword_score = keyword_score(query, &chunk.search_text);
    if keyword_score <= 0.0 && vector_score < MIN_VECTOR_ONLY_SCORE {
        return None;
    }
    let score = (VECTOR_WEIGHT * vector_score) + (KEYWORD_WEIGHT * keyword_score);
    if score < MIN_SEARCH_SCORE {
        return None;
    }

    Some(SearchCandidate {
        chunk,
        score,
        vector_score,
        keyword_score,
    })
}

fn compare_candidates(left: &SearchCandidate, right: &SearchCandidate) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| right.chunk.end_timestamp.cmp(&left.chunk.end_timestamp))
        .then_with(|| left.chunk.thread_id.cmp(&right.chunk.thread_id))
        .then_with(|| left.chunk.start_seq.cmp(&right.chunk.start_seq))
}

fn hits_overlap(left: &ConversationIndexSearchHit, right: &SearchCandidate) -> bool {
    left.thread_id == right.chunk.thread_id
        && left.start_seq <= right.chunk.end_seq
        && right.chunk.start_seq <= left.end_seq
}

fn keyword_score(query: &QueryModel, text: &str) -> f32 {
    let lowered = text.to_ascii_lowercase();
    if !query.normalized_query.is_empty() && lowered.contains(&query.normalized_query) {
        return 1.0;
    }
    if query.lexical_terms.is_empty() {
        return 0.0;
    }

    let candidate_terms = lexical_terms(text).into_iter().collect::<HashSet<_>>();
    if candidate_terms.is_empty() {
        return 0.0;
    }

    let matched = query
        .lexical_terms
        .iter()
        .filter(|term| candidate_terms.contains(term.as_str()))
        .count();
    matched as f32 / query.lexical_terms.len() as f32
}

fn lexical_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() || character == '_' {
            current.push(character);
        } else if !current.is_empty() {
            terms.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    terms
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn parse_embedding_row(value: &Value) -> Result<Vec<f32>, String> {
    let embedding = value
        .get("embedding")
        .and_then(Value::as_array)
        .ok_or_else(|| "conversation index embedding row missing `embedding`".to_owned())?
        .iter()
        .map(|item| {
            item.as_f64().map(|value| value as f32).ok_or_else(|| {
                "conversation index embedding row contains non-float value".to_owned()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(normalize_embedding(embedding))
}

fn normalize_embedding(mut embedding: Vec<f32>) -> Vec<f32> {
    let magnitude = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if magnitude <= f32::EPSILON {
        return embedding;
    }
    embedding.iter_mut().for_each(|value| *value /= magnitude);
    embedding
}

fn resolve_openai_api_key(configured_api_key: &str) -> Option<String> {
    if !configured_api_key.trim().is_empty() {
        return Some(configured_api_key.trim().to_owned());
    }
    std::env::var("OPENAI_API_KEY")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn extract_visible_text(message: &Value) -> Option<String> {
    if let Some(provider_message) = ProviderMessage::from_value(message) {
        if let Some(text) = provider_message
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_owned());
        }
        let extracted = extract_text_from_content(&provider_message.content);
        if !extracted.is_empty() {
            return Some(extracted);
        }
    }

    if let Some(text) = message_text(message)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(text.to_owned());
    }

    let extracted = extract_text_from_content(message.get("content").unwrap_or(&Value::Null));
    if extracted.is_empty() {
        None
    } else {
        Some(extracted)
    }
}

fn extract_text_from_content(content: &Value) -> String {
    let mut parts = Vec::new();
    collect_content_text(content, &mut parts, 0);
    parts.join("\n")
}

fn collect_content_text(content: &Value, parts: &mut Vec<String>, depth: usize) {
    if depth > 32 {
        return;
    }
    match content {
        Value::String(text) => push_text_part(parts, text),
        Value::Array(items) => {
            for item in items {
                collect_content_text(item, parts, depth + 1);
            }
        }
        Value::Object(map) => collect_object_text(map, parts, depth + 1),
        _ => {}
    }
}

fn collect_object_text(map: &Map<String, Value>, parts: &mut Vec<String>, depth: usize) {
    if let Some(text) = map.get("text").and_then(Value::as_str) {
        push_text_part(parts, text);
    }
    if let Some(content) = map.get("content") {
        collect_content_text(content, parts, depth + 1);
    }
    if let Some(parts_value) = map.get("parts") {
        collect_content_text(parts_value, parts, depth + 1);
    }
    if let Some(items_value) = map.get("items") {
        collect_content_text(items_value, parts, depth + 1);
    }
}

fn push_text_part(parts: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_owned());
    }
}

fn sanitize_visible_text(value: &str) -> String {
    value
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_tool_related_message(role: &str, message: &Map<String, Value>) -> bool {
    if matches!(role, "tool" | "tool_use" | "tool_result") {
        return true;
    }
    if message
        .get("tool_use_result")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    if message
        .get("tool_name")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }

    contains_tool_hint(message.get("content"))
        || contains_tool_hint(message.get("metadata"))
        || contains_tool_hint(message.get("input"))
        || contains_tool_hint(message.get("result"))
}

fn contains_tool_hint(value: Option<&Value>) -> bool {
    fn inner(value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }

        match value {
            Value::String(text) => {
                let lower = text.to_ascii_lowercase();
                lower.contains("tool_use")
                    || lower.contains("tool_result")
                    || lower.contains("tool_call")
                    || lower.contains("mcp__")
            }
            Value::Array(items) => items.iter().any(|item| inner(item, depth + 1)),
            Value::Object(map) => map.iter().any(|(key, item)| {
                let lower = key.to_ascii_lowercase();
                lower == "tool_use_id"
                    || lower == "tool_call_id"
                    || lower == "tool_calls"
                    || lower.contains("mcp__")
                    || lower.contains("tool_")
                    || inner(item, depth + 1)
            }),
            _ => false,
        }
    }

    value.is_some_and(|value| inner(value, 0))
}

fn read_lock<T>(lock: &StdRwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_lock<T>(lock: &StdRwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn mutex_lock<T>(lock: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests;
