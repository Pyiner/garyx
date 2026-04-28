//! Process-unique stream identifiers + tombstone registry.
//!
//! Per §6.2 of the protocol doc, a `stream_id` is unique for the
//! **host process lifetime**. That rules out reuse across account,
//! across `deliver_inbound`, across plugin respawn, and across host
//! restart-within-process. The ID is a process-startup nonce plus a
//! monotonic counter.
//!
//! Tombstones are set by the plugin-side SDK at `abandon_inbound` send
//! time (NOT at ACK time) so that any in-flight bytes the host has
//! already written to the pipe are discarded at codec-read *and*
//! queue-execution time. We publish the registry type here so the
//! same shape is used by both the host-side enforcement (§6.2
//! host-emission rule — host checks its own registry before writing)
//! and the plugin SDK's discard path.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Server-assigned stream identifier.
///
/// Format is `str_<hex_nonce>_<counter>`, e.g.
/// `str_a1b2c3d4e5f60000_42`. The prefix makes it distinguishable
/// from other `*_id` fields in logs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(String);

impl StreamId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<&str> for StreamId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for StreamId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Allocates fresh, process-unique stream IDs.
///
/// Cheap to clone (the nonce is shared). Thread-safe.
#[derive(Clone)]
pub struct StreamIdGenerator {
    nonce_hex: String,
    counter: std::sync::Arc<AtomicU64>,
}

impl StreamIdGenerator {
    pub fn new() -> Self {
        let nonce = gen_nonce();
        Self {
            nonce_hex: nonce,
            counter: std::sync::Arc::new(AtomicU64::new(0)),
        }
    }

    /// Construct a generator with a specific nonce. Useful for tests
    /// that want determinism, never for production.
    pub fn with_nonce(nonce_hex: impl Into<String>) -> Self {
        Self {
            nonce_hex: nonce_hex.into(),
            counter: std::sync::Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn next(&self) -> StreamId {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        StreamId(format!("str_{}_{n}", self.nonce_hex))
    }

    pub fn nonce(&self) -> &str {
        &self.nonce_hex
    }
}

impl Default for StreamIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Random-ish 64-bit nonce rendered as 16 hex digits. We don't need
/// cryptographic randomness here — just enough to make the
/// `{nonce, counter}` pair unique across host restarts on the same
/// machine, so historical logs stay unambiguous.
fn gen_nonce() -> String {
    let pid = std::process::id() as u64;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // XOR mix so we don't expose either value verbatim in logs.
    let mixed = pid
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(now.rotate_left(17));
    let n = mixed ^ now;
    format!("{n:016x}")
}

/// Reason a stream was tombstoned. Kept explicit so `garyx doctor`
/// can report which path fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TombstoneReason {
    /// Plugin called `abandon_inbound` (stop grace expired, plugin
    /// decided to cancel). Host-side emissions after the ACK are a
    /// protocol violation.
    Abandoned,
    /// Host is cancelling on its own shutdown path. The terminal
    /// `inbound/end` carries `{ "error": "host_shutting_down" }` and
    /// no further frames are expected.
    HostShutdown,
    /// The stream idled past the per-frame idle timeout (§11.1). Host
    /// emitted the terminal `inbound/end` and will never emit for it
    /// again.
    IdleTimeout,
}

/// Tracks the set of `stream_id`s that are terminal-for-no-more-events.
///
/// Used by both sides:
/// - **Plugin SDK:** tombstones a stream id at `abandon_inbound` send
///   time, then discards any subsequent `stream_frame` / `inbound/end`
///   at the codec layer *and* at queue-execution time.
/// - **Host transport:** tombstones a stream id before writing the
///   `abandon_inbound` response, then refuses to emit any further
///   `stream_frame` / `inbound/end` for that id (protocol violation
///   otherwise).
///
/// Since IDs are unique per host process lifetime, tombstones never
/// need eviction. Memory grows O(streams) over the process lifetime;
/// realistically tens to hundreds of thousands, fine for a `HashSet`.
#[derive(Default)]
pub struct StreamRegistry {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tombstoned: HashSet<String>,
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `id` tombstoned. Returns `true` if this was the first
    /// tombstone for the id, `false` if it was already tombstoned.
    pub fn tombstone(&self, id: &StreamId, _reason: TombstoneReason) -> bool {
        let mut inner = self.inner.lock().expect("StreamRegistry poisoned");
        inner.tombstoned.insert(id.as_str().to_owned())
    }

    pub fn is_tombstoned(&self, id: &StreamId) -> bool {
        let inner = self.inner.lock().expect("StreamRegistry poisoned");
        inner.tombstoned.contains(id.as_str())
    }

    /// For diagnostics / `garyx doctor`. Not performance-sensitive.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("StreamRegistry poisoned")
            .tombstoned
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests;
