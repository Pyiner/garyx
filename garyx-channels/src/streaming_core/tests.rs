use garyx_models::provider::StreamBoundaryKind;

use super::{BoundaryTextEffect, apply_stream_boundary_text, merge_stream_text};

#[test]
fn test_merge_stream_text_snapshot() {
    assert_eq!(merge_stream_text("hello", "hello world"), "hello world");
}

#[test]
fn test_merge_stream_text_duplicate_tail() {
    assert_eq!(merge_stream_text("hello world", "world"), "hello world");
}

#[test]
fn test_merge_stream_text_delta() {
    assert_eq!(merge_stream_text("hello", " world"), "hello world");
}

#[test]
fn test_merge_stream_text_no_auto_newline() {
    assert_eq!(merge_stream_text("first", "second"), "firstsecond");
}

#[test]
fn test_apply_stream_boundary_text_user_ack_clears_buffer() {
    let mut text = "prefilled".to_owned();
    let effect = apply_stream_boundary_text(&mut text, StreamBoundaryKind::UserAck);
    assert_eq!(effect, BoundaryTextEffect::Cleared);
    assert!(text.is_empty());
}

#[test]
fn test_apply_stream_boundary_text_assistant_segment_appends_separator() {
    let mut text = "segment-1".to_owned();
    let effect = apply_stream_boundary_text(&mut text, StreamBoundaryKind::AssistantSegment);
    assert_eq!(effect, BoundaryTextEffect::AssistantSeparatorAppended);
    assert_eq!(text, "segment-1\n\n");
}

#[test]
fn test_apply_stream_boundary_text_assistant_segment_noop_when_empty() {
    let mut text = "   ".to_owned();
    let effect = apply_stream_boundary_text(&mut text, StreamBoundaryKind::AssistantSegment);
    assert_eq!(effect, BoundaryTextEffect::Noop);
    assert_eq!(text, "   ");
}

#[test]
fn test_apply_stream_boundary_text_assistant_segment_noop_when_already_separated() {
    let mut text = "segment-1\n\n".to_owned();
    let effect = apply_stream_boundary_text(&mut text, StreamBoundaryKind::AssistantSegment);
    assert_eq!(effect, BoundaryTextEffect::Noop);
    assert_eq!(text, "segment-1\n\n");
}
