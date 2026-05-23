use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum GaryxDbError {
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("database lock poisoned")]
    LockPoisoned,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub type GaryxDbResult<T> = Result<T, GaryxDbError>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinnedThreadRecord {
    pub thread_id: String,
    pub pinned_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DreamSpanRecord {
    pub span_id: String,
    pub dream_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub start_seq: u64,
    pub end_seq: u64,
    pub start_at: String,
    pub end_at: String,
    pub excerpt: String,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DreamTopicRecord {
    pub dream_id: String,
    pub title: String,
    pub summary: String,
    pub first_message_at: String,
    pub last_message_at: String,
    pub updated_at: String,
    pub source: String,
    pub confidence: f64,
    pub message_count: u32,
    pub span_count: u32,
    pub spans: Vec<DreamSpanRecord>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DreamScanRunRecord {
    pub run_id: String,
    pub scanned_from: String,
    pub scanned_to: String,
    pub created_at: String,
    pub source: String,
    pub status: String,
    pub topics_count: u32,
    pub spans_count: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DreamTopicDraft {
    pub dream_id: String,
    pub title: String,
    pub summary: String,
    pub first_message_at: String,
    pub last_message_at: String,
    pub source: String,
    pub confidence: f64,
    pub message_count: u32,
    pub spans: Vec<DreamSpanDraft>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DreamSpanDraft {
    pub span_id: String,
    pub thread_id: String,
    pub workspace_dir: Option<String>,
    pub start_seq: u64,
    pub end_seq: u64,
    pub start_at: String,
    pub end_at: String,
    pub excerpt: String,
    pub message_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DreamIdResolution {
    dream_id: String,
    duplicate_dream_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DreamOverlapCandidate {
    overlap_score: u64,
    overlap_count: u32,
    last_message_at: String,
    exact_span_keys: BTreeSet<(String, u64, u64)>,
    span_count: u32,
}

pub struct GaryxDbService {
    conn: Mutex<Connection>,
}

impl GaryxDbService {
    pub fn open(path: impl AsRef<Path>) -> GaryxDbResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        initialize_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn memory() -> GaryxDbResult<Self> {
        let conn = Connection::open_in_memory()?;
        initialize_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> GaryxDbResult<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| GaryxDbError::LockPoisoned)
    }

    pub fn list_pinned_threads(&self) -> GaryxDbResult<Vec<PinnedThreadRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT thread_id, pinned_at FROM thread_pins ORDER BY pinned_at DESC, thread_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PinnedThreadRecord {
                thread_id: row.get(0)?,
                pinned_at: row.get(1)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn pin_thread(&self, thread_id: &str) -> GaryxDbResult<PinnedThreadRecord> {
        let thread_id = normalize_thread_id(thread_id)?;
        let pinned_at = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO thread_pins (thread_id, pinned_at)
             VALUES (?1, ?2)
             ON CONFLICT(thread_id) DO UPDATE SET pinned_at = excluded.pinned_at",
            params![thread_id, pinned_at],
        )?;
        Ok(PinnedThreadRecord {
            thread_id,
            pinned_at,
        })
    }

    pub fn unpin_thread(&self, thread_id: &str) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.conn()?;
        let removed = conn.execute(
            "DELETE FROM thread_pins WHERE thread_id = ?1",
            params![thread_id],
        )?;
        Ok(removed > 0)
    }

    pub fn replace_dreams_in_window(
        &self,
        scanned_from: &str,
        scanned_to: &str,
        source: &str,
        topics: &[DreamTopicDraft],
        error: Option<&str>,
    ) -> GaryxDbResult<DreamScanRunRecord> {
        let scanned_from = normalize_required("scanned_from", scanned_from)?;
        let scanned_to = normalize_required("scanned_to", scanned_to)?;
        let source = normalize_required("source", source)?;
        if scanned_from > scanned_to {
            return Err(GaryxDbError::BadRequest(
                "scanned_from must not be later than scanned_to".to_owned(),
            ));
        }

        let created_at = now_string();
        let run_id = format!("dream_scan::{}", Uuid::new_v4());
        let status = if error.is_some() { "fallback" } else { "ok" }.to_owned();
        let topics_count = topics.len() as u32;
        let spans_count = topics
            .iter()
            .map(|topic| topic.spans.len() as u32)
            .sum::<u32>();

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM dream_topics
             WHERE first_message_at >= ?1 AND last_message_at <= ?2",
            params![scanned_from, scanned_to],
        )?;
        tx.execute(
            "INSERT INTO dream_scan_runs (
                run_id, scanned_from, scanned_to, created_at, source, status,
                topics_count, spans_count, error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                run_id,
                scanned_from,
                scanned_to,
                created_at,
                source,
                status,
                topics_count,
                spans_count,
                error.map(str::trim).filter(|value| !value.is_empty()),
            ],
        )?;

        for topic in topics {
            let dream_id = normalize_required("dream_id", &topic.dream_id)?;
            let title = normalize_required("title", &topic.title)?;
            let summary = topic.summary.trim().to_owned();
            let first_message_at = normalize_required("first_message_at", &topic.first_message_at)?;
            let last_message_at = normalize_required("last_message_at", &topic.last_message_at)?;
            if first_message_at > last_message_at {
                return Err(GaryxDbError::BadRequest(format!(
                    "dream topic {dream_id} has first_message_at later than last_message_at"
                )));
            }
            let topic_source = normalize_required("source", &topic.source)?;
            let confidence = topic.confidence.clamp(0.0, 1.0);
            let span_count = topic.spans.len() as u32;
            tx.execute(
                "INSERT INTO dream_topics (
                    dream_id, title, summary, first_message_at, last_message_at,
                    updated_at, source, confidence, message_count, span_count
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(dream_id) DO UPDATE SET
                    title = excluded.title,
                    summary = excluded.summary,
                    first_message_at = excluded.first_message_at,
                    last_message_at = excluded.last_message_at,
                    updated_at = excluded.updated_at,
                    source = excluded.source,
                    confidence = excluded.confidence,
                    message_count = excluded.message_count,
                    span_count = excluded.span_count",
                params![
                    dream_id,
                    title,
                    summary,
                    first_message_at,
                    last_message_at,
                    created_at,
                    topic_source,
                    confidence,
                    topic.message_count,
                    span_count,
                ],
            )?;
            for span in &topic.spans {
                let span_id = normalize_required("span_id", &span.span_id)?;
                let thread_id = normalize_thread_id(&span.thread_id)?;
                let start_at = normalize_required("start_at", &span.start_at)?;
                let end_at = normalize_required("end_at", &span.end_at)?;
                if span.start_seq == 0 || span.end_seq == 0 || span.start_seq > span.end_seq {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has an invalid sequence range"
                    )));
                }
                if start_at > end_at {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has start_at later than end_at"
                    )));
                }
                tx.execute(
                    "INSERT INTO dream_spans (
                        span_id, dream_id, thread_id, workspace_dir, start_seq, end_seq,
                        start_at, end_at, excerpt, message_count, created_at
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        span_id,
                        dream_id,
                        thread_id,
                        span.workspace_dir
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                        span.start_seq,
                        span.end_seq,
                        start_at,
                        end_at,
                        span.excerpt.trim(),
                        span.message_count,
                        created_at,
                    ],
                )?;
            }
        }
        tx.commit()?;

        Ok(DreamScanRunRecord {
            run_id,
            scanned_from,
            scanned_to,
            created_at,
            source,
            status,
            topics_count,
            spans_count,
            error: error
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        })
    }

    pub fn upsert_dreams_incremental(
        &self,
        scanned_from: &str,
        scanned_to: &str,
        source: &str,
        topics: &[DreamTopicDraft],
        error: Option<&str>,
    ) -> GaryxDbResult<DreamScanRunRecord> {
        let scanned_from = normalize_required("scanned_from", scanned_from)?;
        let scanned_to = normalize_required("scanned_to", scanned_to)?;
        let source = normalize_required("source", source)?;
        if scanned_from > scanned_to {
            return Err(GaryxDbError::BadRequest(
                "scanned_from must not be later than scanned_to".to_owned(),
            ));
        }

        let created_at = now_string();
        let run_id = format!("dream_scan::{}", Uuid::new_v4());
        let status = if error.is_some() { "fallback" } else { "ok" }.to_owned();
        let topics_count = topics.len() as u32;
        let spans_count = topics
            .iter()
            .map(|topic| topic.spans.len() as u32)
            .sum::<u32>();

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO dream_scan_runs (
                run_id, scanned_from, scanned_to, created_at, source, status,
                topics_count, spans_count, error
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                run_id,
                scanned_from,
                scanned_to,
                created_at,
                source,
                status,
                topics_count,
                spans_count,
                error.map(str::trim).filter(|value| !value.is_empty()),
            ],
        )?;

        for topic in topics {
            let resolution = resolve_incremental_dream_id(&tx, &topic.dream_id, &topic.spans)?;
            let dream_id = resolution.dream_id;
            let title = normalize_required("title", &topic.title)?;
            let summary = topic.summary.trim().to_owned();
            let topic_source = normalize_required("source", &topic.source)?;
            let confidence = topic.confidence.clamp(0.0, 1.0);
            let mut span_map = BTreeMap::<(String, u64, u64), DreamSpanDraft>::new();
            let mut existing_span_keys = BTreeSet::<(String, u64, u64)>::new();

            for (index, span_dream_id) in std::iter::once(&dream_id)
                .chain(resolution.duplicate_dream_ids.iter())
                .enumerate()
            {
                let mut stmt = tx.prepare(
                    "SELECT span_id, thread_id, workspace_dir, start_seq, end_seq,
                            start_at, end_at, excerpt, message_count
                     FROM dream_spans
                     WHERE dream_id = ?1",
                )?;
                let rows = stmt.query_map(params![span_dream_id.as_str()], |row| {
                    Ok(DreamSpanDraft {
                        span_id: row.get(0)?,
                        thread_id: row.get(1)?,
                        workspace_dir: row.get(2)?,
                        start_seq: row.get(3)?,
                        end_seq: row.get(4)?,
                        start_at: row.get(5)?,
                        end_at: row.get(6)?,
                        excerpt: row.get(7)?,
                        message_count: row.get(8)?,
                    })
                })?;
                for row in rows {
                    let span = row?;
                    if index == 0 {
                        existing_span_keys.insert((
                            span.thread_id.clone(),
                            span.start_seq,
                            span.end_seq,
                        ));
                    }
                    span_map
                        .entry((span.thread_id.clone(), span.start_seq, span.end_seq))
                        .or_insert(span);
                }
            }

            for span in &topic.spans {
                let thread_id = normalize_thread_id(&span.thread_id)?;
                let start_at = normalize_required("start_at", &span.start_at)?;
                let end_at = normalize_required("end_at", &span.end_at)?;
                let key = (thread_id.clone(), span.start_seq, span.end_seq);
                let span_id = span_map
                    .get(&key)
                    .or_else(|| {
                        span_map.values().find(|existing| {
                            dream_span_ranges_overlap(
                                &thread_id,
                                span.start_seq,
                                span.end_seq,
                                existing,
                            )
                        })
                    })
                    .map(|existing| existing.span_id.clone())
                    .unwrap_or_else(|| span.span_id.trim().to_owned());
                let span_id = normalize_required("span_id", &span_id)?;
                if span.start_seq == 0 || span.end_seq == 0 || span.start_seq > span.end_seq {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has an invalid sequence range"
                    )));
                }
                if start_at > end_at {
                    return Err(GaryxDbError::BadRequest(format!(
                        "dream span {span_id} has start_at later than end_at"
                    )));
                }
                span_map.insert(
                    key,
                    DreamSpanDraft {
                        span_id,
                        thread_id,
                        workspace_dir: span.workspace_dir.clone(),
                        start_seq: span.start_seq,
                        end_seq: span.end_seq,
                        start_at,
                        end_at,
                        excerpt: span.excerpt.trim().to_owned(),
                        message_count: span.message_count,
                    },
                );
            }

            let spans = merge_overlapping_dream_spans(span_map.into_values());
            let retained_span_keys = spans
                .iter()
                .map(|span| (span.thread_id.clone(), span.start_seq, span.end_seq))
                .collect::<BTreeSet<_>>();
            let first_message_at = spans
                .iter()
                .map(|span| span.start_at.as_str())
                .min()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| topic.first_message_at.trim().to_owned());
            let last_message_at = spans
                .iter()
                .map(|span| span.end_at.as_str())
                .max()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| topic.last_message_at.trim().to_owned());
            let first_message_at = normalize_required("first_message_at", &first_message_at)?;
            let last_message_at = normalize_required("last_message_at", &last_message_at)?;
            if first_message_at > last_message_at {
                return Err(GaryxDbError::BadRequest(format!(
                    "dream topic {dream_id} has first_message_at later than last_message_at"
                )));
            }
            let message_count = spans.iter().map(|span| span.message_count).sum::<u32>();
            let span_count = spans.len() as u32;

            for duplicate_dream_id in &resolution.duplicate_dream_ids {
                tx.execute(
                    "DELETE FROM dream_topics WHERE dream_id = ?1",
                    params![duplicate_dream_id],
                )?;
            }

            tx.execute(
                "INSERT INTO dream_topics (
                    dream_id, title, summary, first_message_at, last_message_at,
                    updated_at, source, confidence, message_count, span_count
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(dream_id) DO UPDATE SET
                    title = excluded.title,
                    summary = excluded.summary,
                    first_message_at = excluded.first_message_at,
                    last_message_at = excluded.last_message_at,
                    updated_at = excluded.updated_at,
                    source = excluded.source,
                    confidence = excluded.confidence,
                    message_count = excluded.message_count,
                    span_count = excluded.span_count",
                params![
                    dream_id,
                    title,
                    summary,
                    first_message_at,
                    last_message_at,
                    created_at,
                    topic_source,
                    confidence,
                    message_count,
                    span_count,
                ],
            )?;
            for (thread_id, start_seq, end_seq) in
                existing_span_keys.difference(&retained_span_keys)
            {
                tx.execute(
                    "DELETE FROM dream_spans
                     WHERE dream_id = ?1
                       AND thread_id = ?2
                       AND start_seq = ?3
                       AND end_seq = ?4",
                    params![dream_id, thread_id, start_seq, end_seq],
                )?;
            }
            for span in spans {
                let updated = tx.execute(
                    "UPDATE dream_spans
                     SET workspace_dir = ?1,
                         start_at = ?2,
                         end_at = ?3,
                         excerpt = ?4,
                         message_count = ?5
                     WHERE dream_id = ?6
                       AND thread_id = ?7
                       AND start_seq = ?8
                       AND end_seq = ?9",
                    params![
                        span.workspace_dir
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                        span.start_at,
                        span.end_at,
                        span.excerpt.trim(),
                        span.message_count,
                        dream_id,
                        span.thread_id,
                        span.start_seq,
                        span.end_seq,
                    ],
                )?;
                if updated == 0 {
                    tx.execute(
                        "INSERT INTO dream_spans (
                            span_id, dream_id, thread_id, workspace_dir, start_seq, end_seq,
                            start_at, end_at, excerpt, message_count, created_at
                         )
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            span.span_id,
                            dream_id,
                            span.thread_id,
                            span.workspace_dir
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty()),
                            span.start_seq,
                            span.end_seq,
                            span.start_at,
                            span.end_at,
                            span.excerpt.trim(),
                            span.message_count,
                            created_at,
                        ],
                    )?;
                }
            }
        }
        tx.commit()?;

        Ok(DreamScanRunRecord {
            run_id,
            scanned_from,
            scanned_to,
            created_at,
            source,
            status,
            topics_count,
            spans_count,
            error: error
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        })
    }

    pub fn list_dream_topics(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        limit: usize,
    ) -> GaryxDbResult<Vec<DreamTopicRecord>> {
        let limit = limit.clamp(1, 500) as i64;
        let from = normalize_optional(from);
        let to = normalize_optional(to);
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT dream_id, title, summary, first_message_at, last_message_at,
                    updated_at, source, confidence, message_count, span_count
             FROM dream_topics
             WHERE (?1 IS NULL OR last_message_at >= ?1)
               AND (?2 IS NULL OR first_message_at <= ?2)
             ORDER BY last_message_at DESC, dream_id ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![from.as_deref(), to.as_deref(), limit], |row| {
            Ok(DreamTopicRecord {
                dream_id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                first_message_at: row.get(3)?,
                last_message_at: row.get(4)?,
                updated_at: row.get(5)?,
                source: row.get(6)?,
                confidence: row.get(7)?,
                message_count: row.get(8)?,
                span_count: row.get(9)?,
                spans: Vec::new(),
            })
        })?;
        let mut topics = Vec::new();
        for row in rows {
            topics.push(row?);
        }
        attach_dream_spans(&conn, &mut topics)?;
        Ok(topics)
    }

    pub fn get_dream_topic(&self, dream_id: &str) -> GaryxDbResult<Option<DreamTopicRecord>> {
        let dream_id = normalize_required("dream_id", dream_id)?;
        let conn = self.conn()?;
        let mut topic = conn
            .query_row(
                "SELECT dream_id, title, summary, first_message_at, last_message_at,
                        updated_at, source, confidence, message_count, span_count
                 FROM dream_topics
                 WHERE dream_id = ?1",
                params![dream_id],
                |row| {
                    Ok(DreamTopicRecord {
                        dream_id: row.get(0)?,
                        title: row.get(1)?,
                        summary: row.get(2)?,
                        first_message_at: row.get(3)?,
                        last_message_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        source: row.get(6)?,
                        confidence: row.get(7)?,
                        message_count: row.get(8)?,
                        span_count: row.get(9)?,
                        spans: Vec::new(),
                    })
                },
            )
            .optional()?;
        if let Some(topic) = topic.as_mut() {
            attach_dream_spans(&conn, std::slice::from_mut(topic))?;
        }
        Ok(topic)
    }

    pub fn list_dream_topics_for_threads(
        &self,
        thread_ids: &[String],
        from: Option<&str>,
        limit: usize,
    ) -> GaryxDbResult<Vec<DreamTopicRecord>> {
        let mut normalized_thread_ids = thread_ids
            .iter()
            .map(|thread_id| normalize_thread_id(thread_id))
            .collect::<GaryxDbResult<Vec<_>>>()?;
        normalized_thread_ids.sort();
        normalized_thread_ids.dedup();
        if normalized_thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let from = normalize_optional(from);

        let placeholders = std::iter::repeat_n("?", normalized_thread_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let limit = limit.clamp(1, 500).to_string();
        let time_filter = if from.is_some() {
            " AND t.last_message_at >= ?"
        } else {
            ""
        };
        let sql = format!(
            "SELECT DISTINCT t.dream_id, t.title, t.summary, t.first_message_at,
                    t.last_message_at, t.updated_at, t.source, t.confidence,
                    t.message_count, t.span_count
             FROM dream_topics t
             JOIN dream_spans s ON s.dream_id = t.dream_id
             WHERE s.thread_id IN ({placeholders}){time_filter}
             ORDER BY t.last_message_at DESC, t.dream_id ASC
             LIMIT {limit}"
        );
        let mut bind_values = normalized_thread_ids;
        if let Some(from) = from {
            bind_values.push(from);
        }
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(bind_values.iter()), |row| {
            Ok(DreamTopicRecord {
                dream_id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                first_message_at: row.get(3)?,
                last_message_at: row.get(4)?,
                updated_at: row.get(5)?,
                source: row.get(6)?,
                confidence: row.get(7)?,
                message_count: row.get(8)?,
                span_count: row.get(9)?,
                spans: Vec::new(),
            })
        })?;
        let mut topics = Vec::new();
        for row in rows {
            topics.push(row?);
        }
        attach_dream_spans(&conn, &mut topics)?;
        Ok(topics)
    }

    pub fn latest_dream_scan(&self) -> GaryxDbResult<Option<DreamScanRunRecord>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT run_id, scanned_from, scanned_to, created_at, source, status,
                    topics_count, spans_count, error
             FROM dream_scan_runs
             ORDER BY created_at DESC, rowid DESC
             LIMIT 1",
            [],
            |row| {
                Ok(DreamScanRunRecord {
                    run_id: row.get(0)?,
                    scanned_from: row.get(1)?,
                    scanned_to: row.get(2)?,
                    created_at: row.get(3)?,
                    source: row.get(4)?,
                    status: row.get(5)?,
                    topics_count: row.get(6)?,
                    spans_count: row.get(7)?,
                    error: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }
}

fn merge_overlapping_dream_spans(
    spans: impl IntoIterator<Item = DreamSpanDraft>,
) -> Vec<DreamSpanDraft> {
    let mut by_thread = BTreeMap::<String, Vec<DreamSpanDraft>>::new();
    for span in spans {
        by_thread
            .entry(span.thread_id.clone())
            .or_default()
            .push(span);
    }

    let mut merged = Vec::new();
    for (_thread_id, mut spans) in by_thread {
        spans.sort_by(|left, right| {
            left.start_seq
                .cmp(&right.start_seq)
                .then_with(|| left.end_seq.cmp(&right.end_seq))
        });
        let mut current: Option<DreamSpanDraft> = None;
        for span in spans {
            match current.as_mut() {
                Some(active) if span.start_seq <= active.end_seq => {
                    active.end_seq = active.end_seq.max(span.end_seq);
                    if span.end_at > active.end_at {
                        active.end_at = span.end_at.clone();
                    }
                    if span.start_at < active.start_at {
                        active.start_at = span.start_at.clone();
                    }
                    if span.workspace_dir.is_some() {
                        active.workspace_dir = span.workspace_dir.clone();
                    }
                    if !span.excerpt.trim().is_empty() {
                        active.excerpt = span.excerpt.clone();
                    }
                    active.message_count = active.message_count.max(span.message_count);
                }
                _ => {
                    if let Some(previous) = current.replace(span) {
                        merged.push(previous);
                    }
                }
            }
        }
        if let Some(last) = current {
            merged.push(last);
        }
    }
    merged
}

fn resolve_incremental_dream_id(
    tx: &Transaction<'_>,
    requested_dream_id: &str,
    spans: &[DreamSpanDraft],
) -> GaryxDbResult<DreamIdResolution> {
    let requested_dream_id = normalize_required("dream_id", requested_dream_id)?;
    let requested_exists = tx
        .query_row(
            "SELECT 1 FROM dream_topics WHERE dream_id = ?1",
            params![requested_dream_id.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    let mut overlap_scores = BTreeMap::<String, DreamOverlapCandidate>::new();
    let mut draft_span_keys = BTreeSet::<(String, u64, u64)>::new();
    for span in spans {
        let thread_id = normalize_thread_id(&span.thread_id)?;
        if span.start_seq == 0 || span.end_seq == 0 || span.start_seq > span.end_seq {
            return Err(GaryxDbError::BadRequest(format!(
                "dream span {} has an invalid sequence range",
                span.span_id.trim()
            )));
        }
        draft_span_keys.insert((thread_id.clone(), span.start_seq, span.end_seq));

        let mut stmt = tx.prepare(
            "SELECT s.dream_id, s.start_seq, s.end_seq, t.last_message_at, t.span_count
             FROM dream_spans s
             JOIN dream_topics t ON t.dream_id = s.dream_id
             WHERE s.thread_id = ?1
               AND s.start_seq <= ?2
               AND s.end_seq >= ?3",
        )?;
        let rows = stmt.query_map(params![thread_id, span.end_seq, span.start_seq], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
            ))
        })?;
        for row in rows {
            let (dream_id, existing_start_seq, existing_end_seq, last_message_at, span_count) =
                row?;
            let overlap_start = span.start_seq.max(existing_start_seq);
            let overlap_end = span.end_seq.min(existing_end_seq);
            let overlap_width = overlap_end.saturating_sub(overlap_start) + 1;
            let entry = overlap_scores
                .entry(dream_id)
                .or_insert_with(|| DreamOverlapCandidate {
                    overlap_score: 0,
                    overlap_count: 0,
                    last_message_at: last_message_at.clone(),
                    exact_span_keys: BTreeSet::new(),
                    span_count,
                });
            entry.overlap_score = entry.overlap_score.saturating_add(overlap_width);
            entry.overlap_count += 1;
            if last_message_at > entry.last_message_at {
                entry.last_message_at = last_message_at;
            }
            entry.span_count = entry.span_count.max(span_count);
            if existing_start_seq == span.start_seq && existing_end_seq == span.end_seq {
                entry
                    .exact_span_keys
                    .insert((thread_id.clone(), span.start_seq, span.end_seq));
            }
        }
    }

    let dream_id = if requested_exists {
        requested_dream_id
    } else {
        overlap_scores
            .iter()
            .max_by(|left, right| {
                left.1
                    .overlap_score
                    .cmp(&right.1.overlap_score)
                    .then_with(|| left.1.overlap_count.cmp(&right.1.overlap_count))
                    .then_with(|| left.1.last_message_at.cmp(&right.1.last_message_at))
                    .then_with(|| right.0.cmp(left.0))
            })
            .map(|(dream_id, _)| dream_id.clone())
            .unwrap_or(requested_dream_id)
    };
    let duplicate_dream_ids = overlap_scores
        .into_iter()
        .filter_map(|(overlapping_dream_id, candidate)| {
            (overlapping_dream_id != dream_id
                && candidate.span_count as usize == draft_span_keys.len()
                && candidate.exact_span_keys == draft_span_keys)
                .then_some(overlapping_dream_id)
        })
        .collect();

    Ok(DreamIdResolution {
        dream_id,
        duplicate_dream_ids,
    })
}

fn dream_span_ranges_overlap(
    thread_id: &str,
    start_seq: u64,
    end_seq: u64,
    existing: &DreamSpanDraft,
) -> bool {
    existing.thread_id == thread_id
        && start_seq <= existing.end_seq
        && existing.start_seq <= end_seq
}

fn initialize_connection(conn: &Connection) -> GaryxDbResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS thread_pins (
            thread_id TEXT PRIMARY KEY,
            pinned_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS dream_topics (
            dream_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT '',
            first_message_at TEXT NOT NULL,
            last_message_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            source TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0,
            message_count INTEGER NOT NULL DEFAULT 0,
            span_count INTEGER NOT NULL DEFAULT 0
        ) STRICT;

        CREATE TABLE IF NOT EXISTS dream_spans (
            span_id TEXT PRIMARY KEY,
            dream_id TEXT NOT NULL REFERENCES dream_topics(dream_id) ON DELETE CASCADE,
            thread_id TEXT NOT NULL,
            workspace_dir TEXT,
            start_seq INTEGER NOT NULL,
            end_seq INTEGER NOT NULL,
            start_at TEXT NOT NULL,
            end_at TEXT NOT NULL,
            excerpt TEXT NOT NULL DEFAULT '',
            message_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            UNIQUE(dream_id, thread_id, start_seq, end_seq)
        ) STRICT;

        CREATE INDEX IF NOT EXISTS idx_dream_topics_last_message_at
            ON dream_topics(last_message_at DESC);
        CREATE INDEX IF NOT EXISTS idx_dream_spans_thread
            ON dream_spans(thread_id, start_seq, end_seq);

        CREATE TABLE IF NOT EXISTS dream_scan_runs (
            run_id TEXT PRIMARY KEY,
            scanned_from TEXT NOT NULL,
            scanned_to TEXT NOT NULL,
            created_at TEXT NOT NULL,
            source TEXT NOT NULL,
            status TEXT NOT NULL,
            topics_count INTEGER NOT NULL DEFAULT 0,
            spans_count INTEGER NOT NULL DEFAULT 0,
            error TEXT
        ) STRICT;
        "#,
    )?;
    Ok(())
}

fn normalize_thread_id(thread_id: &str) -> GaryxDbResult<String> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err(GaryxDbError::BadRequest(
            "thread_id must not be empty".to_owned(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn normalize_required(field: &str, value: &str) -> GaryxDbResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(GaryxDbError::BadRequest(format!(
            "{field} must not be empty"
        )));
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
}

fn attach_dream_spans(conn: &Connection, topics: &mut [DreamTopicRecord]) -> GaryxDbResult<()> {
    let mut stmt = conn.prepare(
        "SELECT span_id, dream_id, thread_id, workspace_dir, start_seq, end_seq,
                start_at, end_at, excerpt, message_count
         FROM dream_spans
         WHERE dream_id = ?1
         ORDER BY start_at ASC, thread_id ASC, start_seq ASC",
    )?;
    for topic in topics {
        let rows = stmt.query_map(params![topic.dream_id], |row| {
            Ok(DreamSpanRecord {
                span_id: row.get(0)?,
                dream_id: row.get(1)?,
                thread_id: row.get(2)?,
                workspace_dir: row.get(3)?,
                start_seq: row.get(4)?,
                end_seq: row.get(5)?,
                start_at: row.get(6)?,
                end_at: row.get(7)?,
                excerpt: row.get(8)?,
                message_count: row.get(9)?,
            })
        })?;
        let mut spans = Vec::new();
        for row in rows {
            spans.push(row?);
        }
        topic.span_count = spans.len() as u32;
        topic.spans = spans;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_pins_round_trip_in_recency_order() {
        use std::time::Duration;

        let db = GaryxDbService::memory().expect("db opens");
        db.pin_thread("thread::older").expect("pin older");
        std::thread::sleep(Duration::from_millis(2));
        db.pin_thread("thread::newer").expect("pin newer");
        std::thread::sleep(Duration::from_millis(2));
        db.pin_thread("thread::older").expect("repin older");

        let records = db.list_pinned_threads().expect("list pins");
        assert_eq!(
            records
                .iter()
                .map(|record| record.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thread::older", "thread::newer"],
        );

        assert!(db.unpin_thread("thread::older").expect("unpin older"));
        assert!(!db.unpin_thread("thread::older").expect("unpin older again"));
        assert_eq!(
            db.list_pinned_threads()
                .expect("list remaining")
                .into_iter()
                .map(|record| record.thread_id)
                .collect::<Vec<_>>(),
            vec!["thread::newer"],
        );
    }

    #[test]
    fn empty_thread_id_is_rejected() {
        let db = GaryxDbService::memory().expect("db opens");
        assert!(matches!(
            db.pin_thread("   "),
            Err(GaryxDbError::BadRequest(_))
        ));
    }

    #[test]
    fn dreams_replace_window_lists_topics_with_spans() {
        let db = GaryxDbService::memory().expect("db opens");
        let older = DreamTopicDraft {
            dream_id: "dream::older".to_owned(),
            title: "Old Plan".to_owned(),
            summary: "Outside the scanned window.".to_owned(),
            first_message_at: "2026-05-20T08:00:00.000Z".to_owned(),
            last_message_at: "2026-05-20T08:10:00.000Z".to_owned(),
            source: "heuristic".to_owned(),
            confidence: 0.5,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::older".to_owned(),
                thread_id: "thread::older".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-20T08:00:00.000Z".to_owned(),
                end_at: "2026-05-20T08:10:00.000Z".to_owned(),
                excerpt: "old".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-20T00:00:00.000Z",
            "2026-05-20T23:59:59.999Z",
            "heuristic",
            &[older],
            None,
        )
        .expect("insert older dream");

        let topic = DreamTopicDraft {
            dream_id: "dream::today".to_owned(),
            title: "Gateway Pin Polish".to_owned(),
            summary: "Review pinned-thread routing and mobile state.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:20:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.92,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::today".to_owned(),
                thread_id: "thread::today".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 3,
                end_seq: 5,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:20:00.000Z".to_owned(),
                excerpt: "pin routing".to_owned(),
                message_count: 2,
            }],
        };
        let scan = db
            .replace_dreams_in_window(
                "2026-05-21T00:00:00.000Z",
                "2026-05-21T23:59:59.999Z",
                "claude",
                std::slice::from_ref(&topic),
                None,
            )
            .expect("insert today's dreams");

        assert_eq!(scan.topics_count, 1);
        assert_eq!(scan.spans_count, 1);

        let records = db
            .list_dream_topics(Some("2026-05-21T00:00:00.000Z"), None, 20)
            .expect("list dreams");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "Gateway Pin Polish");
        assert_eq!(records[0].spans[0].thread_id, "thread::today");
        assert_eq!(
            records[0].spans[0].workspace_dir.as_deref(),
            Some("/workspace/test")
        );

        let detail = db
            .get_dream_topic("dream::today")
            .expect("get dream")
            .expect("dream exists");
        assert_eq!(
            detail.summary,
            "Review pinned-thread routing and mobile state."
        );
        assert_eq!(detail.spans.len(), 1);

        let latest_scan = db
            .latest_dream_scan()
            .expect("latest scan")
            .expect("scan exists");
        assert_eq!(latest_scan.run_id, scan.run_id);
    }

    #[test]
    fn dreams_replace_window_removes_previous_overlapping_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::original".to_owned(),
            title: "Original".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "heuristic".to_owned(),
            confidence: 0.5,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::original".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "heuristic",
            &[original],
            None,
        )
        .expect("insert original");

        db.replace_dreams_in_window(
            "2026-05-21T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "heuristic",
            &[],
            Some("no user messages"),
        )
        .expect("replace with empty scan");

        assert!(
            db.list_dream_topics(Some("2026-05-21T00:00:00.000Z"), None, 20)
                .expect("list dreams")
                .is_empty()
        );
        assert!(matches!(
            db.latest_dream_scan().expect("scan exists"),
            Some(DreamScanRunRecord { status, .. }) if status == "fallback"
        ));
    }

    #[test]
    fn dreams_replace_window_keeps_partially_overlapping_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let spanning = DreamTopicDraft {
            dream_id: "dream::spanning".to_owned(),
            title: "Spanning".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-20T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::spanning".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 2,
                start_at: "2026-05-20T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 2,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-20T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "claude",
            &[spanning],
            None,
        )
        .expect("insert spanning topic");

        db.replace_dreams_in_window(
            "2026-05-21T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "claude",
            &[],
            None,
        )
        .expect("replace narrow window");

        assert!(
            db.get_dream_topic("dream::spanning")
                .expect("get spanning topic")
                .is_some()
        );
    }

    #[test]
    fn dreams_incremental_upsert_extends_existing_topics_without_deleting_old_spans() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::incremental".to_owned(),
            title: "Dreams".to_owned(),
            summary: "Initial dream topic.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::first".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "initial".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::incremental".to_owned(),
            title: "Dreams Auto Scan".to_owned(),
            summary: "Initial topic plus automatic scan work.".to_owned(),
            first_message_at: "2026-05-21T10:30:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:35:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::second".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 2,
                end_seq: 2,
                start_at: "2026-05-21T10:30:00.000Z".to_owned(),
                end_at: "2026-05-21T10:35:00.000Z".to_owned(),
                excerpt: "automatic scan".to_owned(),
                message_count: 1,
            }],
        };
        let scan = db
            .upsert_dreams_incremental(
                "2026-05-21T10:00:00.000Z",
                "2026-05-21T11:00:00.000Z",
                "claude_incremental",
                &[update],
                None,
            )
            .expect("incremental upsert succeeds");

        assert_eq!(scan.topics_count, 1);
        assert_eq!(scan.spans_count, 1);

        let topic = db
            .get_dream_topic("dream::incremental")
            .expect("get topic")
            .expect("topic exists");
        assert_eq!(topic.title, "Dreams Auto Scan");
        assert_eq!(topic.message_count, 2);
        assert_eq!(topic.span_count, 2);
        assert_eq!(topic.first_message_at, "2026-05-21T10:00:00.000Z");
        assert_eq!(topic.last_message_at, "2026-05-21T10:35:00.000Z");
        assert_eq!(
            topic
                .spans
                .iter()
                .map(|span| span.span_id.as_str())
                .collect::<Vec<_>>(),
            vec!["span::first", "span::second"]
        );
    }

    #[test]
    fn dreams_incremental_upsert_preserves_existing_span_identity() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::stable-span".to_owned(),
            title: "Stable Span".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::stable".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::stable-span".to_owned(),
            title: "Stable Span Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "updated excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        let topic = db
            .get_dream_topic("dream::stable-span")
            .expect("get topic")
            .expect("topic exists");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::stable");
        assert_eq!(topic.spans[0].excerpt, "updated excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_reuses_existing_topic_for_overlapping_new_id() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::existing-topic".to_owned(),
            title: "Existing Topic".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::existing-topic".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::fresh-topic".to_owned(),
            title: "Existing Topic Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh-topic".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "updated excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        assert!(
            db.get_dream_topic("dream::fresh-topic")
                .expect("get fresh topic")
                .is_none()
        );
        let topic = db
            .get_dream_topic("dream::existing-topic")
            .expect("get existing topic")
            .expect("topic exists");
        assert_eq!(topic.title, "Existing Topic Updated");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::existing-topic");
        assert_eq!(topic.spans[0].excerpt, "updated excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_merges_duplicate_existing_topics_on_overlap() {
        let db = GaryxDbService::memory().expect("db opens");
        let alpha = DreamTopicDraft {
            dream_id: "dream::alpha".to_owned(),
            title: "Alpha".to_owned(),
            summary: "Original alpha.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::alpha".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "alpha excerpt".to_owned(),
                message_count: 1,
            }],
        };
        let beta = DreamTopicDraft {
            dream_id: "dream::beta".to_owned(),
            title: "Beta".to_owned(),
            summary: "Duplicate beta.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::beta".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "beta excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[alpha, beta],
            None,
        )
        .expect("insert duplicate topics");

        let update = DreamTopicDraft {
            dream_id: "dream::fresh".to_owned(),
            title: "Merged Topic".to_owned(),
            summary: "Merged summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "merged excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        let topics = db
            .list_dream_topics(Some("2026-05-21T00:00:00.000Z"), None, 20)
            .expect("list topics");
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].dream_id, "dream::alpha");
        assert_eq!(topics[0].title, "Merged Topic");
        assert_eq!(topics[0].spans.len(), 1);
        assert_eq!(topics[0].spans[0].span_id, "span::alpha");
        assert_eq!(topics[0].spans[0].excerpt, "merged excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_merges_overlapping_spans_with_stable_identity() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::overlap-span".to_owned(),
            title: "Overlap Span".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::stable-overlap".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::overlap-span".to_owned(),
            title: "Overlap Span Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:10:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh-overlap".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 2,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:10:00.000Z".to_owned(),
                excerpt: "expanded excerpt".to_owned(),
                message_count: 2,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        let topic = db
            .get_dream_topic("dream::overlap-span")
            .expect("get topic")
            .expect("topic exists");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::stable-overlap");
        assert_eq!(topic.spans[0].start_seq, 1);
        assert_eq!(topic.spans[0].end_seq, 2);
        assert_eq!(topic.spans[0].excerpt, "expanded excerpt");
    }

    #[test]
    fn dreams_incremental_upsert_reuses_overlapping_topic_for_generated_id() {
        let db = GaryxDbService::memory().expect("db opens");
        let original = DreamTopicDraft {
            dream_id: "dream::existing".to_owned(),
            title: "Existing Topic".to_owned(),
            summary: "Original summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::existing".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "original".to_owned(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:05:00.000Z",
            "claude",
            &[original],
            None,
        )
        .expect("insert original");

        let update = DreamTopicDraft {
            dream_id: "dream::generated".to_owned(),
            title: "Existing Topic Updated".to_owned(),
            summary: "Updated summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: "updated excerpt".to_owned(),
                message_count: 1,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        assert!(
            db.get_dream_topic("dream::generated")
                .expect("get generated topic")
                .is_none()
        );
        let topic = db
            .get_dream_topic("dream::existing")
            .expect("get existing topic")
            .expect("existing topic remains");
        assert_eq!(topic.title, "Existing Topic Updated");
        assert_eq!(topic.spans.len(), 1);
        assert_eq!(topic.spans[0].span_id, "span::existing");
        assert_eq!(topic.spans[0].excerpt, "updated excerpt");

        let topics = db
            .list_dream_topics_for_threads(&["thread::one".to_owned()], None, 10)
            .expect("list thread topics");
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].dream_id, "dream::existing");
    }

    #[test]
    fn dreams_incremental_upsert_keeps_distinct_overlapping_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let broad = DreamTopicDraft {
            dream_id: "dream::broad".to_owned(),
            title: "Broad Topic".to_owned(),
            summary: "Broad summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:50:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 10,
            spans: vec![DreamSpanDraft {
                span_id: "span::broad".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 10,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:50:00.000Z".to_owned(),
                excerpt: "broad".to_owned(),
                message_count: 10,
            }],
        };
        let narrow = DreamTopicDraft {
            dream_id: "dream::narrow".to_owned(),
            title: "Narrow Topic".to_owned(),
            summary: "Narrow summary.".to_owned(),
            first_message_at: "2026-05-21T10:10:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:20:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 2,
            spans: vec![DreamSpanDraft {
                span_id: "span::narrow".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 3,
                end_seq: 4,
                start_at: "2026-05-21T10:10:00.000Z".to_owned(),
                end_at: "2026-05-21T10:20:00.000Z".to_owned(),
                excerpt: "narrow".to_owned(),
                message_count: 2,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T10:50:00.000Z",
            "claude",
            &[broad, narrow],
            None,
        )
        .expect("insert original topics");

        let update = DreamTopicDraft {
            dream_id: "dream::generated-broad".to_owned(),
            title: "Broad Topic Updated".to_owned(),
            summary: "Updated broad summary.".to_owned(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:50:00.000Z".to_owned(),
            source: "claude_incremental".to_owned(),
            confidence: 0.9,
            message_count: 10,
            spans: vec![DreamSpanDraft {
                span_id: "span::fresh-broad".to_owned(),
                thread_id: "thread::one".to_owned(),
                workspace_dir: Some("/workspace/test".to_owned()),
                start_seq: 1,
                end_seq: 10,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:50:00.000Z".to_owned(),
                excerpt: "updated broad".to_owned(),
                message_count: 10,
            }],
        };
        db.upsert_dreams_incremental(
            "2026-05-21T10:00:00.000Z",
            "2026-05-21T11:00:00.000Z",
            "claude_incremental",
            &[update],
            None,
        )
        .expect("incremental update succeeds");

        assert!(
            db.get_dream_topic("dream::generated-broad")
                .expect("get generated topic")
                .is_none()
        );
        assert_eq!(
            db.get_dream_topic("dream::broad")
                .expect("get broad topic")
                .expect("broad exists")
                .title,
            "Broad Topic Updated"
        );
        assert_eq!(
            db.get_dream_topic("dream::narrow")
                .expect("get narrow topic")
                .expect("narrow exists")
                .title,
            "Narrow Topic"
        );
        let topics = db
            .list_dream_topics_for_threads(&["thread::one".to_owned()], None, 10)
            .expect("list thread topics");
        assert_eq!(topics.len(), 2);
    }

    #[test]
    fn dreams_list_topics_for_threads_returns_only_matching_topics() {
        let db = GaryxDbService::memory().expect("db opens");
        let matching = DreamTopicDraft {
            dream_id: "dream::matching".to_owned(),
            title: "Matching".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-21T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::matching".to_owned(),
                thread_id: "thread::matching".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T10:00:00.000Z".to_owned(),
                end_at: "2026-05-21T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        let other = DreamTopicDraft {
            dream_id: "dream::other".to_owned(),
            title: "Other".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-21T11:00:00.000Z".to_owned(),
            last_message_at: "2026-05-21T11:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::other".to_owned(),
                thread_id: "thread::other".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-21T11:00:00.000Z".to_owned(),
                end_at: "2026-05-21T11:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        let old_matching = DreamTopicDraft {
            dream_id: "dream::old-matching".to_owned(),
            title: "Old Matching".to_owned(),
            summary: String::new(),
            first_message_at: "2026-05-19T10:00:00.000Z".to_owned(),
            last_message_at: "2026-05-19T10:05:00.000Z".to_owned(),
            source: "claude".to_owned(),
            confidence: 0.8,
            message_count: 1,
            spans: vec![DreamSpanDraft {
                span_id: "span::old-matching".to_owned(),
                thread_id: "thread::matching".to_owned(),
                workspace_dir: None,
                start_seq: 1,
                end_seq: 1,
                start_at: "2026-05-19T10:00:00.000Z".to_owned(),
                end_at: "2026-05-19T10:05:00.000Z".to_owned(),
                excerpt: String::new(),
                message_count: 1,
            }],
        };
        db.replace_dreams_in_window(
            "2026-05-19T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "claude",
            &[matching, other, old_matching],
            None,
        )
        .expect("insert dreams");

        let topics = db
            .list_dream_topics_for_threads(
                &["thread::matching".to_owned()],
                Some("2026-05-21T00:00:00.000Z"),
                20,
            )
            .expect("list topics by thread");

        assert_eq!(
            topics
                .iter()
                .map(|topic| topic.dream_id.as_str())
                .collect::<Vec<_>>(),
            vec!["dream::matching"]
        );
        assert_eq!(topics[0].spans[0].thread_id, "thread::matching");
    }
}
