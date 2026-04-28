use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CANONICAL_THREAD_PREFIX: &str = "thread::";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThreadLogLevel {
    Info,
    Warn,
    Error,
}

impl ThreadLogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLogEvent {
    pub thread_id: String,
    pub level: ThreadLogLevel,
    pub stage: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

impl ThreadLogEvent {
    pub fn new(
        thread_id: impl Into<String>,
        level: ThreadLogLevel,
        stage: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            level,
            stage: stage.into(),
            message: message.into(),
            run_id: None,
            fields: HashMap::new(),
            timestamp: None,
        }
    }

    pub fn info(
        thread_id: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(thread_id, ThreadLogLevel::Info, stage, message)
    }

    pub fn warn(
        thread_id: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(thread_id, ThreadLogLevel::Warn, stage, message)
    }

    pub fn error(
        thread_id: impl Into<String>,
        stage: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(thread_id, ThreadLogLevel::Error, stage, message)
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.timestamp = Some(timestamp.into());
        self
    }

    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLogChunk {
    pub thread_id: String,
    pub path: String,
    pub text: String,
    pub cursor: u64,
    pub reset: bool,
}

pub fn is_canonical_thread_id(value: &str) -> bool {
    value.trim().starts_with(CANONICAL_THREAD_PREFIX)
}

pub fn resolve_thread_log_thread_id(
    thread_id: &str,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    for key in ["thread_id", "base_thread_id"] {
        if let Some(candidate) = metadata
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| is_canonical_thread_id(value))
        {
            return Some(candidate.to_owned());
        }
    }

    let trimmed = thread_id.trim();
    if is_canonical_thread_id(trimmed) {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

#[async_trait]
pub trait ThreadLogSink: Send + Sync {
    async fn record_event(&self, event: ThreadLogEvent);
    async fn read_chunk(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
    ) -> Result<ThreadLogChunk, String>;
    async fn delete_thread(&self, thread_id: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct NoopThreadLogSink;

#[async_trait]
impl ThreadLogSink for NoopThreadLogSink {
    async fn record_event(&self, _event: ThreadLogEvent) {}

    async fn read_chunk(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
    ) -> Result<ThreadLogChunk, String> {
        Ok(ThreadLogChunk {
            thread_id: thread_id.trim().to_owned(),
            path: String::new(),
            text: String::new(),
            cursor: cursor.unwrap_or(0),
            reset: cursor.is_none(),
        })
    }

    async fn delete_thread(&self, _thread_id: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
