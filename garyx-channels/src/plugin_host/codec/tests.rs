use super::*;
use serde_json::json;
use tokio::io::{AsyncWriteExt, duplex};

#[tokio::test]
async fn write_then_read_roundtrips() {
    let (mut a, mut b) = duplex(1024);
    let codec = FrameCodec::new();
    codec
        .write_frame(&mut a, &json!({"jsonrpc": "2.0", "id": 1, "result": "ok"}))
        .await
        .unwrap();
    a.flush().await.unwrap();
    let mut reader = FrameCodec::new();
    let msg = reader.read_frame(&mut b).await.unwrap();
    assert_eq!(msg["jsonrpc"], "2.0");
    assert_eq!(msg["id"], 1);
    assert_eq!(msg["result"], "ok");
}

#[tokio::test]
async fn read_assembles_across_chunks() {
    // Write the header and body in two pieces; the reader must
    // assemble them.
    let (mut tx, mut rx) = duplex(1024);
    let body = b"{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":42}";
    let header = format!("Content-Length: {}\r\n\r\n", body.len());

    tokio::spawn(async move {
        tx.write_all(&header.as_bytes()[..10]).await.unwrap();
        tx.flush().await.unwrap();
        tokio::task::yield_now().await;
        tx.write_all(&header.as_bytes()[10..]).await.unwrap();
        tx.flush().await.unwrap();
        tokio::task::yield_now().await;
        // Split the body too.
        tx.write_all(&body[..20]).await.unwrap();
        tx.flush().await.unwrap();
        tokio::task::yield_now().await;
        tx.write_all(&body[20..]).await.unwrap();
        tx.flush().await.unwrap();
    });

    let mut codec = FrameCodec::new();
    let msg = codec.read_frame(&mut rx).await.unwrap();
    assert_eq!(msg["id"], 7);
    assert_eq!(msg["result"], 42);
}

#[tokio::test]
async fn back_to_back_frames_do_not_interleave() {
    let (mut a, mut b) = duplex(4096);
    let w = FrameCodec::new();
    w.write_frame(&mut a, &json!({"id": 1, "result": "first"}))
        .await
        .unwrap();
    w.write_frame(&mut a, &json!({"id": 2, "result": "second"}))
        .await
        .unwrap();
    w.write_frame(&mut a, &json!({"id": 3, "result": "third"}))
        .await
        .unwrap();
    a.flush().await.unwrap();
    drop(a);

    let mut r = FrameCodec::new();
    let m1 = r.read_frame(&mut b).await.unwrap();
    let m2 = r.read_frame(&mut b).await.unwrap();
    let m3 = r.read_frame(&mut b).await.unwrap();
    assert_eq!(m1["result"], "first");
    assert_eq!(m2["result"], "second");
    assert_eq!(m3["result"], "third");
}

#[tokio::test]
async fn oversized_frame_is_rejected_before_allocation() {
    let (mut tx, mut rx) = duplex(256);
    // Advertise 10 MiB on a codec capped at 1 KiB. We should fail
    // on the header without touching the body.
    tokio::spawn(async move {
        tx.write_all(b"Content-Length: 10485760\r\n\r\n")
            .await
            .unwrap();
        tx.flush().await.unwrap();
    });

    let mut codec = FrameCodec::with_cap(1024);
    let err = codec.read_frame(&mut rx).await.unwrap_err();
    matches!(err, CodecError::FrameTooLarge { .. });
}

#[tokio::test]
async fn missing_content_length_errors() {
    let (mut tx, mut rx) = duplex(256);
    tokio::spawn(async move {
        tx.write_all(b"X-Other: foo\r\n\r\n{}").await.unwrap();
        tx.flush().await.unwrap();
    });
    let mut codec = FrameCodec::new();
    let err = codec.read_frame(&mut rx).await.unwrap_err();
    assert!(matches!(err, CodecError::MissingContentLength));
}

#[tokio::test]
async fn unknown_headers_are_ignored() {
    let (mut tx, mut rx) = duplex(256);
    let body = br#"{"v":1}"#;
    let raw = format!(
        "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    tokio::spawn(async move {
        tx.write_all(raw.as_bytes()).await.unwrap();
        tx.write_all(body).await.unwrap();
        tx.flush().await.unwrap();
    });
    let mut codec = FrameCodec::new();
    let msg = codec.read_frame(&mut rx).await.unwrap();
    assert_eq!(msg["v"], 1);
}

#[tokio::test]
async fn truncated_pipe_surfaces_unexpected_eof() {
    let (mut tx, mut rx) = duplex(256);
    tokio::spawn(async move {
        tx.write_all(b"Content-Length: 20\r\n\r\n{\"id\":")
            .await
            .unwrap();
        tx.flush().await.unwrap();
    });
    let mut codec = FrameCodec::new();
    let err = codec.read_frame(&mut rx).await.unwrap_err();
    assert!(matches!(err, CodecError::UnexpectedEof));
}

#[tokio::test]
async fn header_flood_bounded() {
    // Writer emits 64KiB of header bytes without a terminator.
    let (mut tx, mut rx) = duplex(128 * 1024);
    tokio::spawn(async move {
        let junk = vec![b'x'; 65 * 1024];
        tx.write_all(&junk).await.unwrap();
        tx.flush().await.unwrap();
    });
    let mut codec = FrameCodec::new();
    let err = codec.read_frame(&mut rx).await.unwrap_err();
    assert!(matches!(err, CodecError::BadHeader(_)));
}

#[tokio::test]
async fn hard_cap_is_enforced_on_construction() {
    let codec = FrameCodec::with_cap(usize::MAX);
    assert_eq!(codec.max_frame_bytes(), MAX_FRAME_BYTES_HARD_CAP);
}

#[tokio::test]
async fn frame_exactly_at_cap_is_accepted() {
    // Pick a payload whose length lines up with a round cap. The
    // interior payload is padding bytes that still form valid JSON.
    let payload = serde_json::json!({"pad": "a".repeat(200)});
    let len = serde_json::to_vec(&payload).unwrap().len();
    let (mut a, mut b) = duplex(16 * 1024);
    let writer = FrameCodec::with_cap(len);
    writer.write_frame(&mut a, &payload).await.unwrap();
    a.flush().await.unwrap();
    drop(a);
    let mut reader = FrameCodec::with_cap(len);
    let msg = reader.read_frame(&mut b).await.unwrap();
    assert_eq!(msg, payload);
}

#[tokio::test]
async fn frame_one_byte_over_cap_is_rejected() {
    let payload = serde_json::json!({"pad": "a".repeat(200)});
    let len = serde_json::to_vec(&payload).unwrap().len();
    let (mut tx, mut rx) = duplex(16 * 1024);
    let header = format!("Content-Length: {len}\r\n\r\n");
    tokio::spawn(async move {
        tx.write_all(header.as_bytes()).await.unwrap();
        tx.write_all(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
        tx.flush().await.unwrap();
    });
    // Cap is one byte smaller than the frame.
    let mut codec = FrameCodec::with_cap(len - 1);
    let err = codec.read_frame(&mut rx).await.unwrap_err();
    assert!(matches!(err, CodecError::FrameTooLarge { .. }));
}

#[tokio::test]
async fn invalid_utf8_body_is_rejected() {
    // Hand-craft a frame whose body contains a raw 0xFF byte —
    // invalid UTF-8. Manufacture the Content-Length from the byte
    // count.
    let body: &[u8] = &[0xFFu8, b'{', b'}'];
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let (mut tx, mut rx) = duplex(256);
    tokio::spawn(async move {
        tx.write_all(header.as_bytes()).await.unwrap();
        tx.write_all(body).await.unwrap();
        tx.flush().await.unwrap();
    });
    let mut codec = FrameCodec::new();
    let err = codec.read_frame(&mut rx).await.unwrap_err();
    assert!(matches!(err, CodecError::InvalidUtf8));
}
