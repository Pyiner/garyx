use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, params};
use serde::Serialize;

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
}

fn initialize_connection(conn: &Connection) -> GaryxDbResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS thread_pins (
            thread_id TEXT PRIMARY KEY,
            pinned_at TEXT NOT NULL
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
}
