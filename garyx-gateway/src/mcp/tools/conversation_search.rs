use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::json;

use super::super::*;
use super::history::{
    HistoryEntry, TimeBound, collect_history_entries, normalize_thread_id_filter, parse_time_bound,
    sanitize_transcript_text,
};

const DEFAULT_SEARCH_LIMIT: usize = 5;
const MAX_SEARCH_LIMIT: usize = 20;
const CHUNK_MESSAGE_LIMIT: usize = 6;
const CHUNK_MESSAGE_OVERLAP: usize = 2;
const CHUNK_MAX_CHARS: usize = 1_200;
const VECTOR_DIMS: usize = 256;
const VECTOR_WEIGHT: f32 = 0.75;
const KEYWORD_WEIGHT: f32 = 0.25;
const MIN_SEARCH_SCORE: f32 = 0.1;
const MIN_VECTOR_ONLY_SCORE: f32 = 0.35;

#[derive(Debug)]
struct SearchChunk {
    thread_id: String,
    workspace_dir: Option<String>,
    start_timestamp: Option<DateTime<Utc>>,
    end_timestamp: Option<DateTime<Utc>>,
    start_sequence: u64,
    end_sequence: u64,
    message_count: usize,
    transcript: String,
    search_text: String,
}

#[derive(Debug)]
struct SearchMatch {
    chunk: SearchChunk,
    score: f32,
    vector_score: f32,
    keyword_score: f32,
}

#[derive(Debug)]
struct QueryModel {
    normalized_query: String,
    lexical_terms: Vec<String>,
    vector: Vec<f32>,
}

pub(crate) async fn run(
    server: &GaryMcpServer,
    params: ConversationSearchParams,
) -> Result<String, String> {
    let started = Instant::now();
    let result = async {
        let query = params.query.trim();
        if query.is_empty() {
            return Err("missing required parameter: query".to_owned());
        }

        let thread_filter = normalize_thread_id_filter(params.thread_id.as_deref());
        let workspace_filter =
            garyx_router::normalize_workspace_dir(params.workspace_dir.as_deref());
        let from = parse_time_bound(params.from.as_deref(), TimeBound::Start)?;
        let to = parse_time_bound(params.to.as_deref(), TimeBound::End)?;
        if let (Some(from), Some(to)) = (from.as_ref(), to.as_ref())
            && from > to
        {
            return Err(
                "invalid time range: `from` must be earlier than or equal to `to`".to_owned(),
            );
        }

        let limit = params
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);

        if let Some(index) = server.app_state.threads.history.conversation_index()
            && index.is_enabled()
        {
            let indexed = index
                .search(garyx_router::ConversationIndexSearchRequest {
                    query: query.to_owned(),
                    thread_id: thread_filter.clone(),
                    workspace_dir: workspace_filter.clone(),
                    from,
                    to,
                    limit,
                })
                .await?;
            if let Some(indexed) = indexed
                && (indexed.indexed_threads > 0 || indexed.candidate_chunks > 0)
            {
                return Ok(serde_json::to_string(&json!({
                    "tool": "conversation_search",
                    "status": "ok",
                    "backend": "vector_index",
                    "query": query,
                    "thread_id": thread_filter,
                    "workspace_dir": workspace_filter,
                    "from": from.map(|value| value.to_rfc3339()),
                    "to": to.map(|value| value.to_rfc3339()),
                    "limit": limit,
                    "threads_scanned": indexed.threads_considered,
                    "matched_threads": indexed.indexed_threads,
                    "candidate_chunks": indexed.candidate_chunks,
                    "results": indexed
                        .results
                        .iter()
                        .enumerate()
                        .map(|(idx, entry)| json!({
                            "rank": idx + 1,
                            "score": round_score(entry.score),
                            "vector_score": round_score(entry.vector_score),
                            "keyword_score": round_score(entry.keyword_score),
                            "thread_id": entry.thread_id.clone(),
                            "workspace_dir": entry.workspace_dir.clone(),
                            "transcript_file": entry.transcript_file.clone(),
                            "start_timestamp": entry
                                .start_timestamp
                                .as_ref()
                                .map(DateTime::to_rfc3339),
                            "end_timestamp": entry
                                .end_timestamp
                                .as_ref()
                                .map(DateTime::to_rfc3339),
                            "message_count": entry.message_count,
                            "snippet": entry.snippet.clone(),
                        }))
                        .collect::<Vec<_>>(),
                }))
                .unwrap_or_default());
            }
        }

        let collected = collect_history_entries(
            server,
            thread_filter.as_deref(),
            workspace_filter.as_deref(),
            from,
            to,
        )
        .await?;

        let chunks = build_chunks(&collected.entries);
        let query_model = QueryModel::new(query);
        let mut matches = chunks
            .into_iter()
            .filter_map(|chunk| score_chunk(chunk, &query_model))
            .collect::<Vec<_>>();

        matches.sort_by(compare_search_matches);

        let mut selected = Vec::new();
        for candidate in matches {
            if selected
                .iter()
                .any(|existing: &SearchMatch| chunks_overlap(&existing.chunk, &candidate.chunk))
            {
                continue;
            }
            selected.push(candidate);
            if selected.len() >= limit {
                break;
            }
        }

        Ok(serde_json::to_string(&json!({
            "tool": "conversation_search",
            "status": "ok",
            "backend": "ephemeral_fallback",
            "query": query,
            "thread_id": thread_filter,
            "workspace_dir": workspace_filter,
            "from": from.map(|value| value.to_rfc3339()),
            "to": to.map(|value| value.to_rfc3339()),
            "limit": limit,
            "threads_scanned": collected.threads_scanned,
            "matched_threads": collected.matched_threads,
            "candidate_chunks": selected.len(),
            "results": selected
                .iter()
                .enumerate()
                .map(|(idx, entry)| json!({
                    "rank": idx + 1,
                    "score": round_score(entry.score),
                    "vector_score": round_score(entry.vector_score),
                    "keyword_score": round_score(entry.keyword_score),
                    "thread_id": entry.chunk.thread_id.clone(),
                    "workspace_dir": entry.chunk.workspace_dir.clone(),
                    "transcript_file": serde_json::Value::Null,
                    "start_timestamp": entry
                        .chunk
                        .start_timestamp
                        .as_ref()
                        .map(DateTime::to_rfc3339),
                    "end_timestamp": entry
                        .chunk
                        .end_timestamp
                        .as_ref()
                        .map(DateTime::to_rfc3339),
                    "message_count": entry.chunk.message_count,
                    "snippet": entry.chunk.transcript.clone(),
                }))
                .collect::<Vec<_>>(),
        }))
        .unwrap_or_default())
    }
    .await;

    server.record_tool_metric(
        "conversation_search",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result
}

impl QueryModel {
    fn new(query: &str) -> Self {
        let normalized_query = sanitize_transcript_text(query).to_ascii_lowercase();
        let lexical_terms = lexical_terms(query);
        let vector = hashed_feature_vector(query);
        Self {
            normalized_query,
            lexical_terms,
            vector,
        }
    }
}

fn build_chunks(entries: &[HistoryEntry]) -> Vec<SearchChunk> {
    let mut by_thread = BTreeMap::<String, Vec<HistoryEntry>>::new();
    for entry in entries {
        by_thread
            .entry(entry.thread_id.clone())
            .or_default()
            .push(entry.clone());
    }

    let mut chunks = Vec::new();
    for thread_entries in by_thread.values_mut() {
        thread_entries.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.sequence.cmp(&right.sequence))
        });

        let mut start = 0usize;
        while start < thread_entries.len() {
            let mut end = (start + CHUNK_MESSAGE_LIMIT).min(thread_entries.len());
            while end > start + 1 && chunk_char_count(&thread_entries[start..end]) > CHUNK_MAX_CHARS
            {
                end -= 1;
            }

            let slice = &thread_entries[start..end];
            if let Some(chunk) = build_chunk(slice) {
                chunks.push(chunk);
            }
            if end >= thread_entries.len() {
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

fn build_chunk(entries: &[HistoryEntry]) -> Option<SearchChunk> {
    let first = entries.first()?;
    let last = entries.last()?;
    let transcript = entries
        .iter()
        .map(|entry| format!("{}: {}", entry.role, sanitize_transcript_text(&entry.text)))
        .collect::<Vec<_>>()
        .join("\n");
    let search_text = entries
        .iter()
        .map(|entry| format!("{} {}", entry.role, sanitize_transcript_text(&entry.text)))
        .collect::<Vec<_>>()
        .join("\n");

    Some(SearchChunk {
        thread_id: first.thread_id.clone(),
        workspace_dir: first.workspace_dir.clone(),
        start_timestamp: first.timestamp,
        end_timestamp: last.timestamp,
        start_sequence: first.sequence,
        end_sequence: last.sequence,
        message_count: entries.len(),
        transcript,
        search_text,
    })
}

fn chunk_char_count(entries: &[HistoryEntry]) -> usize {
    entries
        .iter()
        .map(|entry| sanitize_transcript_text(&entry.text).len() + entry.role.len() + 3)
        .sum()
}

fn score_chunk(chunk: SearchChunk, query: &QueryModel) -> Option<SearchMatch> {
    let vector = hashed_feature_vector(&chunk.search_text);
    let vector_score = cosine_similarity(&query.vector, &vector).max(0.0);
    let keyword_score = keyword_score(query, &chunk.search_text);
    if keyword_score <= 0.0 && vector_score < MIN_VECTOR_ONLY_SCORE {
        return None;
    }
    let score = (VECTOR_WEIGHT * vector_score) + (KEYWORD_WEIGHT * keyword_score);
    if score < MIN_SEARCH_SCORE {
        return None;
    }
    Some(SearchMatch {
        chunk,
        score,
        vector_score,
        keyword_score,
    })
}

fn compare_search_matches(left: &SearchMatch, right: &SearchMatch) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            right
                .chunk
                .end_timestamp
                .as_ref()
                .cmp(&left.chunk.end_timestamp.as_ref())
        })
        .then_with(|| left.chunk.thread_id.cmp(&right.chunk.thread_id))
        .then_with(|| left.chunk.start_sequence.cmp(&right.chunk.start_sequence))
}

fn chunks_overlap(left: &SearchChunk, right: &SearchChunk) -> bool {
    left.thread_id == right.thread_id
        && left.start_sequence <= right.end_sequence
        && right.start_sequence <= left.end_sequence
}

fn round_score(value: f32) -> f64 {
    ((value as f64) * 10_000.0).round() / 10_000.0
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
    token_regex()
        .find_iter(&text.to_ascii_lowercase())
        .filter_map(|capture| {
            let term = capture.as_str().trim();
            if term.is_empty() {
                None
            } else {
                Some(term.to_owned())
            }
        })
        .collect()
}

fn hashed_feature_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; VECTOR_DIMS];
    for (feature, weight) in hashed_features(text) {
        let hash = stable_hash64(feature.as_bytes());
        let idx = (hash as usize) % VECTOR_DIMS;
        let sign = if (hash & 1) == 0 { 1.0 } else { -1.0 };
        vector[idx] += sign * weight;
    }

    let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude <= f32::EPSILON {
        return vector;
    }
    vector.iter_mut().for_each(|value| *value /= magnitude);
    vector
}

fn hashed_features(text: &str) -> Vec<(String, f32)> {
    let mut features = Vec::new();
    for term in lexical_terms(text) {
        features.push((format!("tok:{term}"), 1.0));
    }

    let compact_chars = text
        .to_ascii_lowercase()
        .chars()
        .filter(|character| is_feature_char(*character))
        .collect::<Vec<_>>();
    for width in [2usize, 3usize] {
        for window in compact_chars.windows(width) {
            let gram = window.iter().collect::<String>();
            if !gram.is_empty() {
                features.push((format!("gram:{gram}"), 0.5));
            }
        }
    }

    features
}

fn is_feature_char(character: char) -> bool {
    !character.is_whitespace() && !character.is_ascii_punctuation()
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn token_regex() -> &'static Regex {
    static TOKEN_REGEX: OnceLock<Regex> = OnceLock::new();
    TOKEN_REGEX.get_or_init(|| Regex::new(r"[\p{L}\p{N}_]+").expect("valid token regex"))
}
