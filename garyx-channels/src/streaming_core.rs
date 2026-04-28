//! Shared streaming text helpers for channel adapters.
use garyx_models::provider::StreamBoundaryKind;

/// Merge incoming stream text with existing accumulated text.
///
/// This function supports both delta-style streams and snapshot-style streams:
/// - Snapshot: incoming starts with existing => replace with incoming
/// - Duplicate tail: existing already ends with incoming => keep existing
/// - Delta: append incoming directly
///
/// No extra newline is ever injected.
pub fn merge_stream_text(existing: &str, incoming: &str) -> String {
    if incoming.is_empty() {
        return existing.to_owned();
    }
    if existing.is_empty() {
        return incoming.to_owned();
    }
    if incoming.starts_with(existing) {
        return incoming.to_owned();
    }
    if existing.ends_with(incoming) {
        return existing.to_owned();
    }
    format!("{existing}{incoming}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryTextEffect {
    Cleared,
    AssistantSeparatorAppended,
    Noop,
}

/// Apply the normalized text-side semantics for stream boundaries.
///
/// This is the channel-invariant contract:
/// - `UserAck`: clear buffered text (never treat it as assistant output)
/// - `AssistantSegment`: add an inline separator when there is buffered text
pub fn apply_stream_boundary_text(
    text: &mut String,
    kind: StreamBoundaryKind,
) -> BoundaryTextEffect {
    match kind {
        StreamBoundaryKind::UserAck => {
            text.clear();
            BoundaryTextEffect::Cleared
        }
        StreamBoundaryKind::AssistantSegment => {
            if text.trim().is_empty() || text.ends_with("\n\n") {
                BoundaryTextEffect::Noop
            } else if text.ends_with('\n') {
                text.push('\n');
                BoundaryTextEffect::AssistantSeparatorAppended
            } else {
                text.push_str("\n\n");
                BoundaryTextEffect::AssistantSeparatorAppended
            }
        }
    }
}

#[cfg(test)]
mod tests;
