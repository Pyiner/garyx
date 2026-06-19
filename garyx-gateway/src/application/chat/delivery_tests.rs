use garyx_channels::StreamingDispatchTarget;
use serde_json::json;

use super::buffer::BoundThreadDeliveryBuffer;
use super::images::extract_markdown_image_refs;
use super::plan::{bound_thread_delivery_targets, targets_except_streaming_target};

#[test]
fn bound_delivery_targets_skip_internal_api_bindings() {
    let thread = json!({
        "channel_bindings": [
            {
                "channel": "api",
                "account_id": "main",
                "binding_key": "loop",
                "chat_id": "loop",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "loop"
            },
            {
                "channel": "telegram",
                "account_id": "codex_bot",
                "binding_key": "chat-1",
                "chat_id": "chat-1",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "chat-1"
            }
        ]
    });

    let targets = bound_thread_delivery_targets(&thread);

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].channel, "telegram");
}

#[test]
fn bound_delivery_targets_exclude_only_direct_streaming_target() {
    let thread = json!({
        "channel_bindings": [
            {
                "channel": "telegram",
                "account_id": "bot1",
                "binding_key": "chat-a",
                "chat_id": "chat-a",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "chat-a"
            },
            {
                "channel": "telegram",
                "account_id": "bot2",
                "binding_key": "chat-b",
                "chat_id": "chat-b",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "chat-b"
            }
        ]
    });
    let targets = bound_thread_delivery_targets(&thread);
    let streaming_target = StreamingDispatchTarget {
        target_thread_id: "thread::target".to_owned(),
        channel: "telegram".to_owned(),
        account_id: "bot1".to_owned(),
        chat_id: "chat-a".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "chat-a".to_owned(),
        thread_id: None,
    };

    let filtered = targets_except_streaming_target(&targets, &streaming_target);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].account_id, "bot2");
    assert_eq!(filtered[0].chat_id, "chat-b");
}

#[test]
fn extracts_only_existing_local_markdown_images_with_supported_extensions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let png = temp.path().join("shot.png");
    let webp = temp.path().join("preview.webp");
    let pdf = temp.path().join("brief.pdf");
    std::fs::write(&png, b"png").expect("png");
    std::fs::write(&webp, b"webp").expect("webp");
    std::fs::write(&pdf, b"pdf").expect("pdf");

    let text = format!(
        "Inline stays markdown ![shot]({}) and ![same]({}) and \
         ![doc]({}) and ![remote](https://example.com/a.png) and ![webp](<{}>).",
        png.display(),
        png.display(),
        pdf.display(),
        webp.display(),
    );

    let refs = extract_markdown_image_refs(&text);

    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].path, png);
    assert_eq!(refs[0].alt.as_deref(), Some("shot"));
    assert_eq!(refs[1].path, webp);
    assert_eq!(refs[1].alt.as_deref(), Some("webp"));
}

#[test]
fn skips_missing_relative_and_non_image_markdown_targets() {
    let temp = tempfile::tempdir().expect("temp dir");
    let txt = temp.path().join("notes.txt");
    let bmp = temp.path().join("legacy.bmp");
    std::fs::write(&txt, b"text").expect("text");
    std::fs::write(&bmp, b"bmp").expect("bmp");
    let missing = temp.path().join("missing.jpg");
    let text = format!(
        "![relative](relative.png) ![txt]({}) ![bmp]({}) ![missing]({})",
        txt.display(),
        bmp.display(),
        missing.display(),
    );

    assert!(extract_markdown_image_refs(&text).is_empty());
}

#[test]
fn streaming_image_scan_collects_without_text_delivery_pending() {
    let buffer = BoundThreadDeliveryBuffer::default();

    buffer.push_image_scan_delta("![shot](/tmp/shot.png)", "test");

    assert!(buffer.take_pending_text("test").is_none());
    assert_eq!(
        buffer.take_image_scan_text("test").as_deref(),
        Some("![shot](/tmp/shot.png)")
    );
}

#[test]
fn streaming_image_scan_preserves_assistant_segment_boundary() {
    let buffer = BoundThreadDeliveryBuffer::default();

    buffer.push_image_scan_delta("first", "test");
    buffer.push_image_scan_separator("test");
    buffer.push_image_scan_delta("second", "test");

    assert_eq!(
        buffer.take_image_scan_text("test").as_deref(),
        Some("first\n\nsecond")
    );
}
