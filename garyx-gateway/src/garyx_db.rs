use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
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
             WHERE last_message_at >= ?1 AND first_message_at <= ?2",
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
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
}
