//! LSP-style `Content-Length:` framing for JSON-RPC messages.
//!
//! Wire format per frame:
//!
//! ```text
//! Content-Length: <n>\r\n
//! \r\n
//! <n bytes of UTF-8 JSON>
//! ```
//!
//! Only `Content-Length` is parsed. Any other header lines before the
//! blank line are ignored (so a future addition like `Content-Type:
//! application/vscode-jsonrpc; charset=utf-8` stays forward-compatible
//! without a protocol bump).
//!
//! Framing errors are fatal for the subprocess protocol — the
//! `SubprocessPlugin` supervisor treats any [`CodecError`] on the
//! plugin's stdout as grounds to kill + restart the child, so we do not
//! try to recover mid-stream.

use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Host-side default for the largest frame we will allocate to decode.
///
/// The hard host cap is `MAX_FRAME_BYTES_HARD_CAP`; a manifest may shrink
/// the effective limit per plugin but never raise it above the hard
/// cap.
pub const MAX_FRAME_BYTES_DEFAULT: usize = 8 * 1024 * 1024;

/// Absolute ceiling. A manifest that asks for more is clamped to this.
pub const MAX_FRAME_BYTES_HARD_CAP: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("peer closed the pipe before a complete frame arrived")]
    UnexpectedEof,
    #[error("malformed header line: {0:?}")]
    BadHeader(String),
    #[error("missing Content-Length header")]
    MissingContentLength,
    #[error("Content-Length {advertised} exceeds configured cap {cap}")]
    FrameTooLarge { advertised: usize, cap: usize },
    #[error("body was not valid UTF-8")]
    InvalidUtf8,
    #[error("body was not valid JSON: {0}")]
    InvalidJson(serde_json::Error),
    #[error("frame violates JSON-RPC 2.0 envelope: {0}")]
    InvalidEnvelope(String),
}

/// Streaming frame codec.
///
/// Holds a small read buffer so we can assemble frames that span
/// multiple `read()` syscalls. The codec is *not* `Clone` — one codec
/// per pipe end.
pub struct FrameCodec {
    read_buf: Vec<u8>,
    /// Largest single frame we'll decode. Clamped to
    /// [`MAX_FRAME_BYTES_HARD_CAP`] at construction.
    max_frame_bytes: usize,
}

impl FrameCodec {
    pub fn new() -> Self {
        Self::with_cap(MAX_FRAME_BYTES_DEFAULT)
    }

    pub fn with_cap(max_frame_bytes: usize) -> Self {
        Self {
            read_buf: Vec::with_capacity(16 * 1024),
            max_frame_bytes: max_frame_bytes.min(MAX_FRAME_BYTES_HARD_CAP),
        }
    }

    pub fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    /// Encode a single JSON-RPC value as a framed write on `writer`.
    ///
    /// Does not flush. Callers batch their own writes and call
    /// `writer.flush()` explicitly.
    pub async fn write_frame<W>(&self, writer: &mut W, body: &Value) -> Result<(), CodecError>
    where
        W: AsyncWrite + Unpin,
    {
        let bytes = serde_json::to_vec(body).map_err(CodecError::InvalidJson)?;
        if bytes.len() > self.max_frame_bytes {
            return Err(CodecError::FrameTooLarge {
                advertised: bytes.len(),
                cap: self.max_frame_bytes,
            });
        }
        let header = format!("Content-Length: {}\r\n\r\n", bytes.len());
        writer.write_all(header.as_bytes()).await?;
        writer.write_all(&bytes).await?;
        Ok(())
    }

    /// Read one frame from `reader`. Blocks until a complete frame
    /// (header + body) has been assembled, or returns a definite error.
    ///
    /// Assembly is resilient to arbitrary read chunking: the codec
    /// retains any bytes past the end of the current frame for the
    /// next call.
    pub async fn read_frame<R>(&mut self, reader: &mut R) -> Result<Value, CodecError>
    where
        R: AsyncRead + Unpin,
    {
        // 1. Consume header block. Header lines are separated by CRLF,
        //    block is terminated by an empty line (CRLFCRLF).
        let header_end = self.fill_until_double_crlf(reader).await?;
        let parsed = {
            let header_block = &self.read_buf[..header_end];
            let mut content_length: Option<usize> = None;
            let mut oversized: Option<usize> = None;
            for line in split_header_lines(header_block) {
                if line.is_empty() {
                    continue;
                }
                let (name, value) = parse_header_line(line)?;
                if name.eq_ignore_ascii_case("content-length") {
                    let advertised: usize = value.trim().parse().map_err(|_| {
                        CodecError::BadHeader(String::from_utf8_lossy(line).into_owned())
                    })?;
                    if advertised > self.max_frame_bytes {
                        oversized = Some(advertised);
                        break;
                    }
                    content_length = Some(advertised);
                }
                // Unknown headers are ignored for forward compatibility.
            }
            if let Some(advertised) = oversized {
                Err(advertised)
            } else {
                Ok(content_length)
            }
        };

        // 2. Drop the header block (including the terminator) from
        //    the buffer. We do this BEFORE surfacing oversized or
        //    missing-header errors so the buffer does not start the
        //    next read with a stale header.
        self.read_buf.drain(..header_end);

        let content_length = match parsed {
            Err(advertised) => {
                return Err(CodecError::FrameTooLarge {
                    advertised,
                    cap: self.max_frame_bytes,
                });
            }
            Ok(Some(n)) => n,
            Ok(None) => return Err(CodecError::MissingContentLength),
        };

        // 3. Read enough bytes to cover the body.
        while self.read_buf.len() < content_length {
            let mut chunk = [0u8; 8 * 1024];
            let want = (content_length - self.read_buf.len()).min(chunk.len());
            let n = reader.read(&mut chunk[..want]).await?;
            if n == 0 {
                return Err(CodecError::UnexpectedEof);
            }
            self.read_buf.extend_from_slice(&chunk[..n]);
        }

        // 4. Consume the body bytes.
        let body_bytes: Vec<u8> = self.read_buf.drain(..content_length).collect();
        let body = std::str::from_utf8(&body_bytes).map_err(|_| CodecError::InvalidUtf8)?;
        serde_json::from_str(body).map_err(CodecError::InvalidJson)
    }

    async fn fill_until_double_crlf<R>(&mut self, reader: &mut R) -> Result<usize, CodecError>
    where
        R: AsyncRead + Unpin,
    {
        // Search the existing buffer first; the remaining-bytes path
        // from the previous frame may already contain a full header.
        let mut search_from = 0usize;
        loop {
            if let Some(pos) = find_double_crlf(&self.read_buf[search_from..]) {
                return Ok(search_from + pos + 4);
            }
            // Protect against pathological header floods.
            if self.read_buf.len() > 64 * 1024 {
                return Err(CodecError::BadHeader(
                    "header block exceeded 64 KiB without terminator".to_owned(),
                ));
            }
            search_from = self.read_buf.len().saturating_sub(3);
            let mut chunk = [0u8; 8 * 1024];
            let n = reader.read(&mut chunk).await?;
            if n == 0 {
                if self.read_buf.is_empty() {
                    return Err(CodecError::UnexpectedEof);
                }
                return Err(CodecError::UnexpectedEof);
            }
            self.read_buf.extend_from_slice(&chunk[..n]);
        }
    }
}

impl Default for FrameCodec {
    fn default() -> Self {
        Self::new()
    }
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn split_header_lines(block: &[u8]) -> impl Iterator<Item = &[u8]> {
    // Header terminator is also CRLFCRLF and is included in `block`; we
    // split on CRLF and skip the trailing empty entry that arises from
    // the terminator.
    block.split(|b| *b == b'\n').map(|line| {
        let end = line.last().map(|c| *c == b'\r').unwrap_or(false);
        if end { &line[..line.len() - 1] } else { line }
    })
}

fn parse_header_line(line: &[u8]) -> Result<(&str, &str), CodecError> {
    let raw = std::str::from_utf8(line)
        .map_err(|_| CodecError::BadHeader(String::from_utf8_lossy(line).into_owned()))?;
    match raw.find(':') {
        Some(idx) => Ok((&raw[..idx], &raw[idx + 1..])),
        None => Err(CodecError::BadHeader(raw.to_owned())),
    }
}

#[cfg(test)]
mod tests;
