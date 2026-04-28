//! Session key generation and parsing utilities.
//!
//! Based on the original session-key-utils.ts.
//!
//! Session Key Formats:
//! - Simple: `{agentId}::{sessionType}::{peerId}`
//! - Hierarchical: `agent:{agentId}:{channel}:{surface}:{id}`
//! - Thread: `agent:{agentId}:{channel}:{surface}:{id}:thread:{threadId}`
//! - Subagent: `agent:{agentId}:subagent:{subagentName}`

use std::fmt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const AGENT_PREFIX: &str = "agent:";
pub const SUBAGENT_SEGMENT: &str = ":subagent:";
pub const THREAD_SEGMENT: &str = ":thread:";
pub const GLOBAL_KEY: &str = "global";
pub const UNKNOWN_KEY: &str = "unknown";
pub const DEFAULT_AGENT_ID: &str = "main";

pub const MAX_KEY_LENGTH: usize = 1000;
pub const MAX_COMPONENT_LENGTH: usize = 256;
pub const MAX_SESSION_KEY_LENGTH: usize = 1024;

/// Valid session types.
pub const VALID_SESSION_TYPES: &[&str] = &["main", "group", "channel", "dm", "thread", "subagent"];

mod parsing;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error type for session key operations.
#[derive(Debug, Clone, thiserror::Error)]
pub struct SessionKeyError {
    pub message: String,
    pub key: Option<String>,
}

impl fmt::Display for SessionKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl SessionKeyError {
    pub fn new(message: impl Into<String>, key: Option<&str>) -> Self {
        Self {
            message: message.into(),
            key: key.map(String::from),
        }
    }
}

// ---------------------------------------------------------------------------
// ParsedSessionKey
// ---------------------------------------------------------------------------

/// Parsed session key components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSessionKey {
    pub agent_id: String,
    pub session_type: String,
    pub peer_id: String,
    pub channel: Option<String>,
    pub surface: Option<String>,
    pub thread_id: Option<String>,
    pub is_subagent: bool,
    pub subagent_name: Option<String>,
    pub raw_key: String,
}

// ---------------------------------------------------------------------------
// Session key classification
// ---------------------------------------------------------------------------

/// Classification of a session key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKeyClass {
    Global,
    Unknown,
    Subagent,
    Thread,
    Group,
    Direct,
}

impl fmt::Display for SessionKeyClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Global => "global",
            Self::Unknown => "unknown",
            Self::Subagent => "subagent",
            Self::Thread => "thread",
            Self::Group => "group",
            Self::Direct => "direct",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// Key building functions
// ---------------------------------------------------------------------------

/// Build a hierarchical agent session key.
///
/// Format: `agent:{agentId}:{channel}:{surface}:{peerId}[:thread:{threadId}]`
pub fn build_agent_session_key(
    agent_id: &str,
    channel: &str,
    surface: &str,
    peer_id: &str,
    thread_id: Option<&str>,
) -> String {
    let mut key = format!("agent:{agent_id}:{channel}:{surface}:{peer_id}");
    if let Some(tid) = thread_id {
        if !tid.is_empty() {
            key.push_str(":thread:");
            key.push_str(tid);
        }
    }
    key
}

/// Build a subagent session key.
///
/// Format: `agent:{agentId}:subagent:{subagentName}`
pub fn build_subagent_session_key(agent_id: &str, subagent_name: &str) -> String {
    format!("agent:{agent_id}:subagent:{subagent_name}")
}

// ---------------------------------------------------------------------------
// Key parsing functions
// ---------------------------------------------------------------------------

pub use parsing::parse_hierarchical_key;

// ---------------------------------------------------------------------------
// Predicate helpers
// ---------------------------------------------------------------------------

/// Check if key is a subagent session.
pub fn is_subagent_session_key(session_key: &str) -> bool {
    session_key.contains(SUBAGENT_SEGMENT)
}

/// Check if key is a thread session.
pub fn is_thread_session_key(session_key: &str) -> bool {
    session_key.contains(THREAD_SEGMENT)
}

/// Check if key is the global session key.
pub fn is_global_key(session_key: &str) -> bool {
    session_key == GLOBAL_KEY
}

/// Check if key is the unknown session key.
pub fn is_unknown_key(session_key: &str) -> bool {
    session_key == UNKNOWN_KEY
}

// ---------------------------------------------------------------------------
// Thread resolution
// ---------------------------------------------------------------------------

/// Get parent session key for a thread session.
///
/// Returns `None` if not a thread key.
pub fn resolve_thread_parent_session_key(session_key: &str) -> Option<String> {
    if !is_thread_session_key(session_key) {
        return None;
    }
    let idx = session_key.find(THREAD_SEGMENT)?;
    Some(session_key[..idx].to_owned())
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Normalize a session key to hierarchical format.
///
/// Converts channel-prefixed keys (e.g. `telegram:dm:user123`) to
/// `agent:main:telegram:dm:user123`. Already-hierarchical, simple, and
/// special keys are returned unchanged.
pub fn normalize_session_key(session_key: &str, default_agent_id: Option<&str>) -> String {
    let default = default_agent_id.unwrap_or(DEFAULT_AGENT_ID);

    // Already hierarchical
    if session_key.starts_with(AGENT_PREFIX) {
        return session_key.to_owned();
    }

    // Special keys
    if session_key == GLOBAL_KEY || session_key == UNKNOWN_KEY {
        return session_key.to_owned();
    }

    // Simple format - keep as is
    if session_key.contains("::") {
        return session_key.to_owned();
    }

    // Channel-prefixed key (telegram:dm:user123) - convert to hierarchical
    let parts: Vec<&str> = session_key.split(':').collect();
    if parts.len() >= 3 {
        return format!("agent:{default}:{session_key}");
    }

    session_key.to_owned()
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify a session key type.
pub fn classify_session_key(session_key: &str) -> SessionKeyClass {
    if is_global_key(session_key) {
        return SessionKeyClass::Global;
    }
    if is_unknown_key(session_key) {
        return SessionKeyClass::Unknown;
    }
    if is_subagent_session_key(session_key) {
        return SessionKeyClass::Subagent;
    }
    if is_thread_session_key(session_key) {
        return SessionKeyClass::Thread;
    }

    let key_lower = session_key.to_lowercase();
    if key_lower.contains(":group:")
        || key_lower.contains(":channel:")
        || key_lower.contains("::group::")
        || key_lower.contains("::channel::")
    {
        return SessionKeyClass::Group;
    }

    SessionKeyClass::Direct
}

// ---------------------------------------------------------------------------
// Channel extraction
// ---------------------------------------------------------------------------

/// Extract channel name from session key.
pub fn extract_channel_from_key(session_key: &str) -> Option<String> {
    if session_key.starts_with(AGENT_PREFIX) {
        if let Ok(parsed) = parse_hierarchical_key(session_key) {
            return parsed.channel;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests;
