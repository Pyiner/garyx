use super::*;

pub fn history_message_count(thread_data: &Value) -> usize {
    thread_data
        .get("history")
        .and_then(|value| value.get("message_count"))
        .and_then(Value::as_u64)
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        .or_else(|| {
            thread_data
                .get("message_count")
                .and_then(Value::as_u64)
                .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        })
        .unwrap_or(0)
}

pub fn count_user_query_messages(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|message| is_user_query_message(message))
        .count()
}

pub fn is_user_query_message(message: &Value) -> bool {
    message_role(message) == Some("user")
        && message
            .get("internal_kind")
            .and_then(Value::as_str)
            .map(str::trim)
            != Some("loop_continuation")
}

pub fn message_text(message: &Value) -> Option<&str> {
    message
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| message.get("content").and_then(Value::as_str))
}

pub(super) fn message_role(message: &Value) -> Option<&str> {
    message.get("role").and_then(Value::as_str)
}

pub fn extract_run_id(message: &Value) -> Option<String> {
    let object = message.as_object()?;
    for key in ["bridge_run_id", "run_id", "client_run_id"] {
        if let Some(value) = object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_owned());
        }
        if let Some(value) = object
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_owned());
        }
    }
    None
}

pub(super) fn trim_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn message_timestamp(message: &Value) -> Option<String> {
    message
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn record_from_message(thread_id: &str, seq: u64, message: &Value) -> ThreadTranscriptRecord {
    ThreadTranscriptRecord {
        seq,
        thread_id: thread_id.to_owned(),
        run_id: extract_run_id(message),
        timestamp: message_timestamp(message).unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        message: message.clone(),
    }
}

fn record_from_message_replacing(
    thread_id: &str,
    seq: u64,
    message: &Value,
    existing: &ThreadTranscriptRecord,
) -> ThreadTranscriptRecord {
    let mut record = record_from_message(thread_id, seq, message);
    if message_timestamp(message).is_none() {
        record.timestamp = existing.timestamp.clone();
    }
    record
}

pub(super) fn record_from_draft(
    thread_id: &str,
    run_id: Option<&str>,
    seq: u64,
    draft: &RunTranscriptRecordDraft,
) -> ThreadTranscriptRecord {
    ThreadTranscriptRecord {
        seq,
        thread_id: thread_id.to_owned(),
        run_id: trim_non_empty(run_id),
        timestamp: draft
            .timestamp
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        message: draft.message.clone(),
    }
}

pub(super) fn record_from_draft_replacing(
    thread_id: &str,
    run_id: Option<&str>,
    seq: u64,
    draft: &RunTranscriptRecordDraft,
    existing: &ThreadTranscriptRecord,
) -> ThreadTranscriptRecord {
    let mut record = record_from_draft(thread_id, run_id, seq, draft);
    if draft.timestamp.is_none() {
        record.timestamp = existing.timestamp.clone();
    }
    record
}

pub(super) fn reconcile_rewrite_records(
    thread_id: &str,
    existing: &[ThreadTranscriptRecord],
    messages: &[Value],
) -> Vec<ThreadTranscriptRecord> {
    if existing.is_empty() {
        return messages
            .iter()
            .enumerate()
            .map(|(offset, message)| record_from_message(thread_id, offset as u64 + 1, message))
            .collect();
    }

    let existing_identity: Vec<Value> = existing
        .iter()
        .map(|record| message_identity(&record.message))
        .collect();
    let authoritative_identity: Vec<Value> = messages.iter().map(message_identity).collect();

    if existing_identity == authoritative_identity {
        let mut rebuilt = existing.to_vec();
        let mut changed_same_seqs = Vec::new();
        for (offset, message) in messages.iter().enumerate() {
            let current = &existing[offset];
            let replacement =
                record_from_message_replacing(thread_id, current.seq, message, current);
            if replacement.timestamp != current.timestamp || replacement.message != current.message
            {
                changed_same_seqs.push(replacement.seq);
                rebuilt[offset] = replacement;
            }
        }
        append_thread_rewrite_marker_if_needed(
            &mut rebuilt,
            &changed_same_seqs,
            thread_id,
            messages.len(),
            existing.len(),
            "same_seq_overwrite",
        );
        return rebuilt;
    }

    if authoritative_identity.len() > existing_identity.len()
        && authoritative_identity[..existing_identity.len()] == existing_identity[..]
    {
        let mut rebuilt = existing.to_vec();
        let next_seq = rebuilt.last().map(|record| record.seq + 1).unwrap_or(1);
        for (seq, message) in (next_seq..).zip(messages[existing.len()..].iter()) {
            rebuilt.push(record_from_message(thread_id, seq, message));
        }
        return rebuilt;
    }

    if authoritative_identity.len() <= existing_identity.len()
        && existing_identity[..authoritative_identity.len()] == authoritative_identity[..]
        && existing[authoritative_identity.len()..]
            .iter()
            .all(|record| is_range_rewrite_control(&record.message))
    {
        let mut rebuilt = existing.to_vec();
        let mut changed_same_seqs = Vec::new();
        for (offset, message) in messages.iter().enumerate() {
            let current = &existing[offset];
            let replacement =
                record_from_message_replacing(thread_id, current.seq, message, current);
            if replacement.timestamp != current.timestamp || replacement.message != current.message
            {
                changed_same_seqs.push(replacement.seq);
                rebuilt[offset] = replacement;
            }
        }
        append_thread_rewrite_marker_if_needed(
            &mut rebuilt,
            &changed_same_seqs,
            thread_id,
            messages.len(),
            existing.len(),
            "same_seq_overwrite",
        );
        return rebuilt;
    }

    if messages.len() >= existing.len() {
        let mut rebuilt = Vec::with_capacity(messages.len() + 1);
        let mut changed_same_seqs = Vec::new();
        let mut next_seq = existing.first().map(|record| record.seq).unwrap_or(1);
        for (offset, message) in messages.iter().enumerate() {
            let seq = existing
                .get(offset)
                .map(|record| record.seq)
                .unwrap_or(next_seq);
            let replacement = if let Some(current) = existing.get(offset) {
                record_from_message_replacing(thread_id, seq, message, current)
            } else {
                record_from_message(thread_id, seq, message)
            };
            if let Some(current) = existing.get(offset)
                && (replacement.timestamp != current.timestamp
                    || replacement.message != current.message)
            {
                changed_same_seqs.push(replacement.seq);
            }
            rebuilt.push(replacement);
            next_seq = seq + 1;
        }
        append_thread_rewrite_marker_if_needed(
            &mut rebuilt,
            &changed_same_seqs,
            thread_id,
            messages.len(),
            existing.len(),
            "same_seq_overwrite",
        );
        return rebuilt;
    }

    let mut rebuilt = Vec::with_capacity(existing.len() + 1);
    let mut changed_same_seqs = Vec::new();
    let first_rewritten_seq = existing
        .get(messages.len())
        .map(|record| record.seq)
        .unwrap_or_else(|| existing.first().map(|record| record.seq).unwrap_or(1));
    let last_rewritten_seq = existing
        .last()
        .map(|record| record.seq)
        .unwrap_or(first_rewritten_seq);
    let rewrite_at = chrono::Utc::now().to_rfc3339();
    for (offset, current) in existing.iter().enumerate() {
        if let Some(message) = messages.get(offset) {
            let replacement =
                record_from_message_replacing(thread_id, current.seq, message, current);
            if replacement.timestamp != current.timestamp || replacement.message != current.message
            {
                changed_same_seqs.push(replacement.seq);
            }
            rebuilt.push(replacement);
        } else {
            let rewrite = build_range_rewrite_record(
                thread_id,
                None,
                current.seq,
                first_rewritten_seq,
                last_rewritten_seq,
                messages.len(),
                existing.len(),
                true,
                "rewrite_from_messages_shrink",
                &rewrite_at,
            );
            if rewrite.timestamp != current.timestamp || rewrite.message != current.message {
                changed_same_seqs.push(rewrite.seq);
            }
            rebuilt.push(rewrite);
        }
    }

    let first_rewritten_seq = changed_same_seqs
        .iter()
        .copied()
        .min()
        .unwrap_or(first_rewritten_seq);
    let last_rewritten_seq = changed_same_seqs
        .iter()
        .copied()
        .max()
        .unwrap_or(last_rewritten_seq);
    let marker = build_range_rewrite_record(
        thread_id,
        None,
        rebuilt.last().map(|record| record.seq + 1).unwrap_or(1),
        first_rewritten_seq,
        last_rewritten_seq,
        messages.len(),
        existing.len(),
        false,
        "rewrite_from_messages_shrink",
        &rewrite_at,
    );
    rebuilt.push(marker);
    rebuilt
}

fn append_thread_rewrite_marker_if_needed(
    records: &mut Vec<ThreadTranscriptRecord>,
    changed_same_seqs: &[u64],
    thread_id: &str,
    authoritative_len: usize,
    existing_len: usize,
    reason: &str,
) {
    let (Some(start_seq), Some(end_seq)) = (
        changed_same_seqs.iter().copied().min(),
        changed_same_seqs.iter().copied().max(),
    ) else {
        return;
    };
    let mut ignored_changed = Vec::new();
    append_range_rewrite_marker(
        records,
        &mut ignored_changed,
        thread_id,
        None,
        start_seq,
        end_seq,
        authoritative_len,
        existing_len,
        reason,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_range_rewrite_record(
    thread_id: &str,
    run_id: Option<&str>,
    seq: u64,
    start_seq: u64,
    end_seq: u64,
    authoritative_len: usize,
    existing_len: usize,
    tombstone: bool,
    reason: &str,
    at: &str,
) -> ThreadTranscriptRecord {
    let mut message = serde_json::json!({
        "role": "system",
        "kind": "control",
        "internal": true,
        "internal_kind": "control",
        "control": {
            "kind": "range_rewrite",
            "thread_id": thread_id,
            "start_seq": start_seq,
            "end_seq": end_seq,
            "tombstone": tombstone,
            "record_seq": seq,
            "authoritative_record_count": authoritative_len,
            "existing_record_count": existing_len,
            "reason": reason,
            "at": at,
        }
    });
    if let Some(run_id) = run_id
        && let Some(control) = message.get_mut("control").and_then(Value::as_object_mut)
    {
        control.insert("run_id".to_owned(), Value::String(run_id.to_owned()));
    }
    ThreadTranscriptRecord {
        seq,
        thread_id: thread_id.to_owned(),
        run_id: trim_non_empty(run_id),
        timestamp: at.to_owned(),
        message,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_range_rewrite_marker(
    records: &mut Vec<ThreadTranscriptRecord>,
    changed: &mut Vec<ThreadTranscriptRecord>,
    thread_id: &str,
    run_id: Option<&str>,
    start_seq: u64,
    end_seq: u64,
    authoritative_len: usize,
    existing_len: usize,
    reason: &str,
) {
    let at = chrono::Utc::now().to_rfc3339();
    let marker = build_range_rewrite_record(
        thread_id,
        run_id,
        records.last().map(|record| record.seq + 1).unwrap_or(1),
        start_seq,
        end_seq,
        authoritative_len,
        existing_len,
        false,
        reason,
        &at,
    );
    changed.push(marker.clone());
    records.push(marker);
}

pub(super) fn is_range_rewrite_control(value: &Value) -> bool {
    value
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
        == Some("range_rewrite")
}

pub(super) fn is_control_record_message(value: &Value) -> bool {
    value
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
        .is_some()
}

/// A message's logical identity for run-tail reconciliation: everything except
/// the cosmetic fields that legitimately differ between the streamed copy and
/// the terminal rebuild of the same message. The SDK session id is bound mid-run
/// (so the first-flush user row has `None` while the terminal rebuild has `Some`)
/// and timestamps can be backfilled, so both are stripped. Role, content, and
/// tool fields — the things that actually distinguish one message from another —
/// are preserved, so a genuine content change still reads as a divergence.
pub(super) fn message_identity(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("timestamp");
        object.remove("sdk_session_id");
        if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
            metadata.remove("sdk_session_id");
        }
        if let Some(control) = object.get_mut("control").and_then(Value::as_object_mut) {
            control.remove("at");
            control.remove("duration_ms");
            control.remove("error");
            control.remove("status");
        }
    }
    value
}
