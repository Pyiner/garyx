use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use garyx_models::thread_logs::{
    ThreadLogChunk, ThreadLogEvent, ThreadLogSink, is_canonical_thread_id,
};
use serde_json::Value;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const TRIM_TARGET_BYTES: usize = 1536 * 1024;
const MAX_FIELD_LEN: usize = 400;

#[derive(Default)]
pub struct ThreadFileLogger {
    root_dir: PathBuf,
    file_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl ThreadFileLogger {
    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
            file_locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn encode_thread_id(thread_id: &str) -> String {
        let mut out = String::with_capacity(thread_id.len() * 2);
        for byte in thread_id.as_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    pub fn thread_log_path(&self, thread_id: &str) -> PathBuf {
        self.root_dir
            .join(format!("{}.log", Self::encode_thread_id(thread_id.trim())))
    }

    async fn file_lock(&self, thread_id: &str) -> Arc<Mutex<()>> {
        let mut guard = self.file_locks.lock().await;
        guard
            .entry(thread_id.trim().to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn ensure_root_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root_dir)
            .await
            .map_err(|error| format!("failed to create thread log dir: {error}"))
    }

    fn render_event(event: ThreadLogEvent) -> String {
        let timestamp = event.timestamp.unwrap_or_else(|| Utc::now().to_rfc3339());
        let mut line = format!(
            "{} {} [{}]",
            sanitize_line(&timestamp),
            event.level.as_str(),
            sanitize_line(&event.stage)
        );
        if let Some(run_id) = event
            .run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            line.push_str(" run=");
            line.push_str(&sanitize_line(run_id));
        }
        line.push(' ');
        line.push_str(&sanitize_line(&event.message));

        let mut fields: Vec<_> = event
            .fields
            .into_iter()
            .filter_map(|(key, value)| {
                let sanitized_key = key.trim().to_owned();
                if sanitized_key.is_empty() || is_sensitive_key(&sanitized_key) {
                    return None;
                }
                let summarized = summarize_value(&sanitized_key, &value);
                if summarized.is_empty() {
                    return None;
                }
                Some((sanitized_key, summarized))
            })
            .collect();
        fields.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, value) in fields {
            line.push(' ');
            line.push_str(&sanitize_line(&key));
            line.push('=');
            line.push_str(&value);
        }

        line.push('\n');
        line
    }

    async fn compact_if_needed(&self, path: &Path) -> Result<(), String> {
        let metadata = match fs::metadata(path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!("failed to stat thread log file: {error}"));
            }
        };
        if metadata.len() <= MAX_LOG_BYTES {
            return Ok(());
        }

        let text = fs::read_to_string(path)
            .await
            .map_err(|error| format!("failed to read thread log for compaction: {error}"))?;
        let compacted = compact_text(&text);
        fs::write(path, compacted)
            .await
            .map_err(|error| format!("failed to compact thread log file: {error}"))
    }
}

#[async_trait]
impl ThreadLogSink for ThreadFileLogger {
    async fn record_event(&self, event: ThreadLogEvent) {
        let thread_id = event.thread_id.trim();
        if thread_id.is_empty() || !is_canonical_thread_id(thread_id) {
            return;
        }

        let file_lock = self.file_lock(thread_id).await;
        let _guard = file_lock.lock().await;

        if self.ensure_root_dir().await.is_err() {
            return;
        }

        let path = self.thread_log_path(thread_id);
        let rendered = Self::render_event(event);
        let open = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await;
        let Ok(mut file) = open else {
            return;
        };
        if file.write_all(rendered.as_bytes()).await.is_err() {
            return;
        }
        let _ = file.flush().await;
        let _ = self.compact_if_needed(&path).await;
    }

    async fn read_chunk(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
    ) -> Result<ThreadLogChunk, String> {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() || !is_canonical_thread_id(thread_id) {
            return Err("thread logs require a canonical thread id".to_owned());
        }

        let file_lock = self.file_lock(thread_id).await;
        let _guard = file_lock.lock().await;
        let path = self.thread_log_path(thread_id);
        let path_text = path.display().to_string();

        let metadata = match fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ThreadLogChunk {
                    thread_id: thread_id.to_owned(),
                    path: path_text,
                    text: String::new(),
                    cursor: 0,
                    reset: cursor.is_none() || cursor.unwrap_or(0) > 0,
                });
            }
            Err(error) => return Err(format!("failed to stat thread log file: {error}")),
        };

        let file_len = metadata.len();
        if cursor.is_none() {
            let text = fs::read_to_string(&path)
                .await
                .map_err(|error| format!("failed to read thread log file: {error}"))?;
            return Ok(ThreadLogChunk {
                thread_id: thread_id.to_owned(),
                path: path_text,
                text,
                cursor: file_len,
                reset: true,
            });
        }

        let cursor = cursor.unwrap_or(0);
        if cursor > file_len {
            let text = fs::read_to_string(&path)
                .await
                .map_err(|error| format!("failed to read thread log file: {error}"))?;
            return Ok(ThreadLogChunk {
                thread_id: thread_id.to_owned(),
                path: path_text,
                text,
                cursor: file_len,
                reset: true,
            });
        }

        if cursor == file_len {
            return Ok(ThreadLogChunk {
                thread_id: thread_id.to_owned(),
                path: path_text,
                text: String::new(),
                cursor,
                reset: false,
            });
        }

        let mut file = File::open(&path)
            .await
            .map_err(|error| format!("failed to open thread log file: {error}"))?;
        file.seek(std::io::SeekFrom::Start(cursor))
            .await
            .map_err(|error| format!("failed to seek thread log file: {error}"))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .await
            .map_err(|error| format!("failed to read thread log delta: {error}"))?;

        Ok(ThreadLogChunk {
            thread_id: thread_id.to_owned(),
            path: path_text,
            text: String::from_utf8_lossy(&buf).to_string(),
            cursor: file_len,
            reset: false,
        })
    }

    async fn delete_thread(&self, thread_id: &str) -> Result<(), String> {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() || !is_canonical_thread_id(thread_id) {
            return Ok(());
        }

        let file_lock = self.file_lock(thread_id).await;
        let _guard = file_lock.lock().await;
        let path = self.thread_log_path(thread_id);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("failed to delete thread log file: {error}")),
        }
    }
}

fn compact_text(text: &str) -> String {
    if text.len() <= TRIM_TARGET_BYTES {
        return text.to_owned();
    }

    let mut start = text.len().saturating_sub(TRIM_TARGET_BYTES);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    let candidate = &text[start.min(text.len())..];
    if let Some(offset) = candidate.find('\n') {
        candidate[offset + 1..].to_owned()
    } else {
        candidate.to_owned()
    }
}

fn sanitize_line(value: &str) -> String {
    value
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("api_key")
        || key.contains("desktop_claude_env")
}

fn should_skip_value(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("image")
        || key.contains("base64")
        || key == "data"
        || key == "images"
        || key == "content"
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = serde_json::Map::new();
            for (key, value) in map {
                if is_sensitive_key(key) || should_skip_value(key) {
                    continue;
                }
                redacted.insert(key.clone(), redact_value(value));
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.iter().take(8).map(redact_value).collect()),
        Value::String(text) => Value::String(sanitize_line(text)),
        other => other.clone(),
    }
}

fn summarize_value(key: &str, value: &Value) -> String {
    if should_skip_value(key) {
        return String::new();
    }
    let redacted = redact_value(value);
    let rendered = match redacted {
        Value::Null => String::new(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => sanitize_line(&text),
        other => sanitize_line(&serde_json::to_string(&other).unwrap_or_default()),
    };

    if rendered.len() <= MAX_FIELD_LEN {
        rendered
    } else {
        let mut clipped = rendered
            .chars()
            .take(MAX_FIELD_LEN.saturating_sub(1))
            .collect::<String>();
        clipped.push('…');
        clipped
    }
}

pub fn default_thread_log_dir() -> PathBuf {
    crate::dashboard::default_log_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("threads")
}

#[cfg(test)]
mod tests;
