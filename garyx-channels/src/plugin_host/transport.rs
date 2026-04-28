//! Transport: the middle layer between the codec and the
//! `SubprocessPlugin` lifecycle wrapper.
//!
//! Responsibilities:
//! - A single writer task owns the stdin pipe. Everyone else sends
//!   frames through an `mpsc::UnboundedSender<WriteItem>`. This is
//!   how we serialize writes from many tokio tasks without holding a
//!   `Mutex<ChildStdin>` across `.await` points.
//! - A single reader task owns the stdout pipe. It parses frames and
//!   dispatches them:
//!     - responses → match to pending request by id, deliver via
//!       oneshot.
//!     - host-bound requests → user-provided handler trait.
//!     - notifications → user-provided handler trait.
//! - A dropped [`PluginRpcClient`] closes the writer and causes any
//!   still-pending waits to fail with [`RpcError::Disconnected`].
//!
//! This module is transport-agnostic: [`Transport::new`] takes any
//! `AsyncRead + AsyncWrite` pair. Production uses `ChildStdin` and
//! `ChildStdout`; tests use `tokio::io::duplex`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use super::codec::{CodecError, FrameCodec};

// ---------------------------------------------------------------------------
// Public config + handles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// For logs + error messages.
    pub plugin_id: String,
    /// Upper bound on a single decoded frame.
    pub max_frame_bytes: usize,
    /// Default RPC timeout applied when a caller passes `None`.
    pub default_rpc_timeout: Duration,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            plugin_id: "<unknown>".to_owned(),
            max_frame_bytes: super::codec::MAX_FRAME_BYTES_DEFAULT,
            default_rpc_timeout: Duration::from_secs(30),
        }
    }
}

/// Anything the host-side user code can receive unsolicited from a
/// plugin. Implementations are spawned on a short-lived task per
/// arrival, so they do not need to be `!Sync` on the hot path.
#[async_trait]
pub trait InboundHandler: Send + Sync + 'static {
    /// Plugin made a request to the host. Return value becomes the
    /// JSON-RPC response. Return `Err(code, message)` to send back a
    /// JSON-RPC error.
    async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)>;

    /// Plugin sent a notification (no id, no response expected).
    async fn on_notification(&self, method: String, params: Value);
}

/// Handles returned by [`Transport::spawn`] — one per reader task,
/// one per writer task. Awaited during shutdown.
pub struct TransportHandles {
    pub reader: JoinHandle<Result<(), CodecError>>,
    pub writer: JoinHandle<()>,
}

#[derive(Debug, Error)]
pub enum RpcError {
    #[error("rpc timed out after {0:?}")]
    Timeout(Duration),
    #[error("peer disconnected before the response arrived")]
    Disconnected,
    #[error("rpc error ({code}): {message}")]
    Remote { code: i32, message: String },
    #[error("malformed response: {0}")]
    MalformedResponse(String),
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Host-driven abort. Emitted by [`PluginRpcClient::abort_pending`]
    /// to fulfil §9.4's "stragglers get `ChannelError::Connection(...)`"
    /// guarantee when the host respawns a plugin before its in-flight
    /// `dispatch_outbound` RPCs have drained. Distinct from
    /// [`RpcError::Disconnected`] so the sender-side error mapper can
    /// preserve the host-authored message verbatim.
    #[error("{0}")]
    HostAborted(String),
}

// ---------------------------------------------------------------------------
// Internal wire types (subset of §5.2)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
struct WireError {
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// A frame that has been validated against the JSON-RPC 2.0 envelope
/// rules from §5.2 of the design doc:
///
/// - `jsonrpc` is present and exactly `"2.0"`.
/// - Ids are integers (we reject strings / nulls / fractional numbers).
/// - Exactly one of request / response / notification shape.
/// - No `result` alongside `error`; no `method` on a response.
///
/// `run_reader` only dispatches on this. Any violation is fatal
/// (§11.1): the reader resolves all pending waiters with
/// `Disconnected` and returns `CodecError::InvalidEnvelope`.
enum DecodedMessage {
    Request {
        id: i64,
        method: String,
        params: Value,
    },
    Response {
        id: i64,
        outcome: Result<Value, WireError>,
    },
    Notification {
        method: String,
        params: Value,
    },
}

fn decode_envelope(frame: &Value) -> Result<DecodedMessage, CodecError> {
    let obj = frame
        .as_object()
        .ok_or_else(|| CodecError::InvalidEnvelope("frame is not a JSON object".to_owned()))?;

    match obj.get("jsonrpc").and_then(Value::as_str) {
        Some("2.0") => {}
        Some(other) => {
            return Err(CodecError::InvalidEnvelope(format!(
                "jsonrpc must be \"2.0\"; got {other:?}"
            )));
        }
        None => {
            return Err(CodecError::InvalidEnvelope(
                "missing jsonrpc field".to_owned(),
            ));
        }
    }

    let has_method = obj.contains_key("method");
    let has_id = obj.contains_key("id");
    let has_result = obj.contains_key("result");
    let has_error = obj.contains_key("error");

    if has_result && has_error {
        return Err(CodecError::InvalidEnvelope(
            "response has both result and error".to_owned(),
        ));
    }

    // Parse and validate id when present.
    let id_int = if has_id {
        let raw_id = &obj["id"];
        if raw_id.is_null() {
            return Err(CodecError::InvalidEnvelope(
                "id must be an integer; got null".to_owned(),
            ));
        }
        match raw_id.as_i64() {
            Some(n) => Some(n),
            None => {
                return Err(CodecError::InvalidEnvelope(format!(
                    "id must be an integer; got {raw_id}"
                )));
            }
        }
    } else {
        None
    };

    if has_method {
        // Request or notification.
        if has_result || has_error {
            return Err(CodecError::InvalidEnvelope(
                "message has both method and result/error".to_owned(),
            ));
        }
        let method = obj
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| CodecError::InvalidEnvelope("method must be a string".to_owned()))?
            .to_owned();
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        Ok(match id_int {
            Some(id) => DecodedMessage::Request { id, method, params },
            None => DecodedMessage::Notification { method, params },
        })
    } else {
        // Must be a response.
        let Some(id) = id_int else {
            return Err(CodecError::InvalidEnvelope(
                "response is missing id".to_owned(),
            ));
        };
        if !has_result && !has_error {
            return Err(CodecError::InvalidEnvelope(
                "response has neither result nor error".to_owned(),
            ));
        }
        let outcome = if has_error {
            let err: WireError = serde_json::from_value(obj["error"].clone())
                .map_err(|e| CodecError::InvalidEnvelope(format!("malformed error object: {e}")))?;
            Err(err)
        } else {
            Ok(obj.get("result").cloned().unwrap_or(Value::Null))
        };
        Ok(DecodedMessage::Response { id, outcome })
    }
}

enum WriteItem {
    Frame(Value),
    Shutdown,
}

// ---------------------------------------------------------------------------
// Client handle
// ---------------------------------------------------------------------------

/// Cheap-to-clone handle for issuing RPCs over a transport.
///
/// Dropping the last clone closes the writer side of the transport;
/// the reader task drains and exits.
#[derive(Clone)]
pub struct PluginRpcClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    cfg: TransportConfig,
    next_id: AtomicI64,
    writer_tx: mpsc::UnboundedSender<WriteItem>,
    /// Map of in-flight requests to their response waiters.
    ///
    /// The oneshot value carries an `RpcError` directly (not a
    /// `WireError`) so the host-driven abort path
    /// ([`PluginRpcClient::abort_pending`]) can deliver
    /// [`RpcError::HostAborted`] without having to synthesise a
    /// wire-level error-code that has no plugin on the other end.
    ///
    /// `std::sync::Mutex` (not `tokio::sync::Mutex`) because every
    /// critical section is a single `HashMap` mutation with no await
    /// inside, and because the `PendingGuard` Drop impl must be able
    /// to remove an entry from a non-async context when the caller's
    /// future is cancelled.
    pending: StdMutex<HashMap<i64, oneshot::Sender<Result<Value, RpcError>>>>,
}

/// RAII cleanup for an in-flight request's `pending` entry. See
/// `call_value_with_timeout` for the lifecycle: the guard is armed at
/// construction and disarmed by the happy/disconnect/remote paths once
/// they know the entry was already removed elsewhere. The only path
/// that keeps it armed is future cancellation (or a local timeout),
/// and on drop the guard removes the stale entry so the map can't
/// accumulate dead waiters indefinitely.
struct PendingGuard<'a> {
    inner: &'a ClientInner,
    id: i64,
    armed: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Mutex poisoning here would mean another thread panicked while
        // holding the lock. That's a bug, but from this Drop we have no
        // recourse and the process is likely shutting down — swallow.
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

impl PluginRpcClient {
    pub fn plugin_id(&self) -> &str {
        &self.inner.cfg.plugin_id
    }

    /// Fire a host → plugin **request** and await the response.
    pub async fn call<P, R>(&self, method: &str, params: &P) -> Result<R, RpcError>
    where
        P: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        self.call_with_timeout(method, params, None).await
    }

    pub async fn call_with_timeout<P, R>(
        &self,
        method: &str,
        params: &P,
        timeout: Option<Duration>,
    ) -> Result<R, RpcError>
    where
        P: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        let params = serde_json::to_value(params)?;
        let raw = self
            .call_value_with_timeout(method, params, timeout)
            .await?;
        Ok(serde_json::from_value(raw)?)
    }

    pub async fn call_value_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, RpcError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().expect("pending mutex poisoned");
            pending.insert(id, tx);
        }
        // From this point on, the entry in `pending` MUST be cleaned up on
        // every exit — including future cancellation between now and the
        // response arriving. A RAII drop-guard does this unconditionally;
        // the happy paths disarm it once they know the reader already
        // removed the entry (response delivered) or that `fail_pending`
        // cleared it (transport disconnected).
        let mut guard = PendingGuard {
            inner: &self.inner,
            id,
            armed: true,
        };

        let frame = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if self.inner.writer_tx.send(WriteItem::Frame(frame)).is_err() {
            // Writer gone; `guard` removes the pending entry on drop.
            return Err(RpcError::Disconnected);
        }

        let effective_timeout = timeout.unwrap_or(self.inner.cfg.default_rpc_timeout);
        match tokio::time::timeout(effective_timeout, rx).await {
            Ok(Ok(Ok(value))) => {
                // Reader removed and signalled `tx` before sending
                // outcome through `rx`; nothing left to clean up.
                guard.armed = false;
                Ok(value)
            }
            Ok(Ok(Err(err))) => {
                // Either a plugin-authored error (propagated as-is from
                // the reader) or a host-driven abort
                // ([`abort_pending`]). Both are sender-delivered, so
                // the pending entry is already gone — disarm.
                guard.armed = false;
                Err(err)
            }
            Ok(Err(_recv_err)) => {
                // `fail_pending` dropped every `oneshot::Sender`; the
                // entry was removed there.
                guard.armed = false;
                Err(RpcError::Disconnected)
            }
            Err(_elapsed) => {
                // Timeout: `guard` will remove the pending entry on
                // drop. The late response, if any, is dispatched via
                // the reader's "orphan response" branch.
                Err(RpcError::Timeout(effective_timeout))
            }
        }
    }

    /// Fire a host → plugin **notification**. Fire-and-forget.
    pub async fn notify<P>(&self, method: &str, params: &P) -> Result<(), RpcError>
    where
        P: Serialize,
    {
        let params = serde_json::to_value(params)?;
        let frame = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.inner
            .writer_tx
            .send(WriteItem::Frame(frame))
            .map_err(|_| RpcError::Disconnected)
    }

    /// Close the writer. Any subsequent `call` returns
    /// `RpcError::Disconnected`. The reader will return on EOF after
    /// the peer closes its end.
    pub fn close_writer(&self) {
        let _ = self.inner.writer_tx.send(WriteItem::Shutdown);
    }

    /// Resolve every in-flight waiter with
    /// [`RpcError::HostAborted`] carrying `message`. The §9.4 respawn
    /// path calls this at grace expiry so stragglers see the mandated
    /// "plugin X respawning; outbound aborted" error instead of a
    /// generic disconnect — preserving the caller's retry semantics
    /// (Connection is retryable; unknown remote codes are not).
    ///
    /// Does NOT close the writer or tear down the transport; the
    /// caller is expected to follow with
    /// [`super::subprocess::SubprocessPlugin::shutdown_gracefully`] to
    /// run §6.3 escalation. Safe to call with an empty pending map —
    /// a no-op in that case.
    pub fn abort_pending(&self, message: String) {
        let drained: Vec<_> = {
            let mut pending = self.inner.pending.lock().expect("pending mutex poisoned");
            pending.drain().collect()
        };
        for (_, tx) in drained {
            let _ = tx.send(Err(RpcError::HostAborted(message.clone())));
        }
    }

    /// Number of in-flight requests currently awaiting a response.
    ///
    /// The respawn quiesce path (§9.4 step 2) uses this to poll whether
    /// the OLD plugin's `dispatch_outbound` calls have drained before
    /// the host escalates to `shutdown` → SIGTERM. Value is an instant
    /// observation and may race with concurrent issuers or the reader
    /// task removing entries; callers MUST tolerate racy reads and
    /// bound their wait with a deadline regardless.
    pub fn pending_count(&self) -> usize {
        self.inner.pending.lock().map(|m| m.len()).unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn pending_len_for_test(&self) -> usize {
        self.pending_count()
    }
}

// ---------------------------------------------------------------------------
// Transport — reader + writer spawner
// ---------------------------------------------------------------------------

pub struct Transport;

impl Transport {
    /// Spawn the reader + writer tasks and return a client handle
    /// for issuing RPCs. The caller keeps [`TransportHandles`] so it
    /// can await task termination during graceful shutdown.
    pub fn spawn<R, W, H>(
        reader: R,
        writer: W,
        cfg: TransportConfig,
        handler: Arc<H>,
    ) -> (PluginRpcClient, TransportHandles)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
        H: InboundHandler,
    {
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<WriteItem>();
        let pending = StdMutex::new(HashMap::new());
        let inner = Arc::new(ClientInner {
            cfg: cfg.clone(),
            next_id: AtomicI64::new(1),
            writer_tx: writer_tx.clone(),
            pending,
        });

        let writer_task = tokio::spawn(run_writer(writer, writer_rx, cfg.max_frame_bytes));
        // Reader holds only Weak refs to both the client state and the
        // writer channel. This matters for clean shutdown: once the
        // last `PluginRpcClient` is dropped, the `ClientInner` Arc
        // count can reach zero, the inner-owned `writer_tx` drops, the
        // writer task exits on `rx.recv() == None`, its `shutdown`
        // closes the pipe, and the peer's reader observes EOF. If the
        // reader held strong refs, both sides' readers would keep each
        // other's writers alive — a four-way deadlock that only
        // resolves when someone force-kills a pipe.
        let reader_task = tokio::spawn(run_reader(
            reader,
            Arc::downgrade(&inner),
            writer_tx.downgrade(),
            handler,
            cfg.max_frame_bytes,
            cfg.plugin_id.clone(),
        ));
        drop(writer_tx);

        let handles = TransportHandles {
            reader: reader_task,
            writer: writer_task,
        };
        (PluginRpcClient { inner }, handles)
    }
}

async fn run_writer<W>(
    mut writer: W,
    mut rx: mpsc::UnboundedReceiver<WriteItem>,
    max_frame_bytes: usize,
) where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let codec = FrameCodec::with_cap(max_frame_bytes);
    while let Some(item) = rx.recv().await {
        match item {
            WriteItem::Frame(value) => {
                if let Err(err) = codec.write_frame(&mut writer, &value).await {
                    warn!(error = %err, "plugin transport: write_frame failed; closing writer");
                    break;
                }
                if let Err(err) = tokio::io::AsyncWriteExt::flush(&mut writer).await {
                    warn!(error = %err, "plugin transport: flush failed; closing writer");
                    break;
                }
            }
            WriteItem::Shutdown => break,
        }
    }
    // Best-effort close. Errors here are uninteresting — the peer is
    // already gone or we are gone.
    let _ = tokio::io::AsyncWriteExt::shutdown(&mut writer).await;
}

async fn run_reader<R, H>(
    mut reader: R,
    client: Weak<ClientInner>,
    writer_tx: mpsc::WeakUnboundedSender<WriteItem>,
    handler: Arc<H>,
    max_frame_bytes: usize,
    plugin_id: String,
) -> Result<(), CodecError>
where
    R: AsyncRead + Unpin + Send + 'static,
    H: InboundHandler,
{
    let mut codec = FrameCodec::with_cap(max_frame_bytes);
    loop {
        let frame = match codec.read_frame(&mut reader).await {
            Ok(v) => v,
            Err(CodecError::UnexpectedEof) => {
                debug!(plugin = %plugin_id, "plugin transport: reader hit EOF");
                if let Some(client) = client.upgrade() {
                    fail_pending(&client);
                }
                return Err(CodecError::UnexpectedEof);
            }
            Err(err) => {
                warn!(plugin = %plugin_id, error = %err, "plugin transport: codec error; failing pending RPCs");
                if let Some(client) = client.upgrade() {
                    fail_pending(&client);
                }
                return Err(err);
            }
        };

        let decoded = match decode_envelope(&frame) {
            Ok(d) => d,
            Err(err) => {
                // §11.1: a malformed JSON-RPC envelope is fatal. Fail
                // pending and bail — the supervisor's exit-watch path
                // turns this into a respawn.
                warn!(plugin = %plugin_id, error = %err, raw = %frame, "plugin transport: invalid envelope; fatal");
                if let Some(client) = client.upgrade() {
                    fail_pending(&client);
                }
                return Err(err);
            }
        };

        match decoded {
            DecodedMessage::Response { id, outcome } => {
                if let Some(client) = client.upgrade() {
                    let waiter = {
                        let mut pending = client.pending.lock().expect("pending mutex poisoned");
                        pending.remove(&id)
                    };
                    if let Some(waiter) = waiter {
                        let mapped = outcome.map_err(|w| RpcError::Remote {
                            code: w.code,
                            message: w.message,
                        });
                        let _ = waiter.send(mapped);
                    } else {
                        warn!(plugin = %plugin_id, response_id = %id, "orphan response (no pending request)");
                    }
                }
                // If `client` can't be upgraded, the last handle has
                // been dropped; no one is waiting, so the response is
                // simply discarded.
            }
            DecodedMessage::Request { id, method, params } => {
                let handler = Arc::clone(&handler);
                // Upgrade writer_tx at spawn time. If the writer is
                // already gone (client dropped) we silently skip the
                // reply — the peer is no longer listening anyway.
                let Some(writer_tx) = writer_tx.upgrade() else {
                    debug!(plugin = %plugin_id, "plugin transport: writer gone; dropping inbound request");
                    continue;
                };
                tokio::spawn(async move {
                    let reply = match handler.on_request(method, params).await {
                        Ok(value) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": value,
                        }),
                        Err((code, message)) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": WireError { code, message, data: None },
                        }),
                    };
                    let _ = writer_tx.send(WriteItem::Frame(reply));
                });
            }
            DecodedMessage::Notification { method, params } => {
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    handler.on_notification(method, params).await;
                });
            }
        }
    }
}

/// Drop every pending waiter so each caller sees its oneshot close,
/// which maps to `RpcError::Disconnected`. Deliberately avoids
/// synthesising a remote `InternalError` — pipe loss is a local
/// transport failure, not a plugin-authored RPC error (§11.1).
fn fail_pending(client: &ClientInner) {
    let mut pending = client.pending.lock().expect("pending mutex poisoned");
    pending.clear();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
