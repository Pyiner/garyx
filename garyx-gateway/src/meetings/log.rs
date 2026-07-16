use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::MeetingError;

pub(crate) const MAX_SEGMENT_LINE_BYTES: usize = 32 * 1024;
pub(crate) const MAX_PAGE_ITEMS: usize = 100;
pub(crate) const INDEX_STRIDE: i64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Transcript,
    Chat,
    ShareStart,
    ShareEnd,
    Join,
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentDraft {
    pub kind: SegmentKind,
    pub speaker: String,
    pub start: String,
    pub end: String,
    pub text: String,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingSegment {
    pub seq: i64,
    pub kind: SegmentKind,
    pub speaker: String,
    pub start: String,
    pub end: String,
    pub text: String,
    pub sources: Vec<String>,
    pub cont: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SegmentLine {
    t: String,
    #[serde(flatten)]
    segment: MeetingSegment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CheckpointLine {
    t: String,
    epoch: i64,
    cursor_out: String,
    at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SparseOffset {
    pub seq: i64,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogScan {
    pub epoch: i64,
    pub generation: i64,
    pub cursor: String,
    pub latest_seq: i64,
    pub byte_len: u64,
    pub offsets: Vec<SparseOffset>,
    pub truncated_bytes: u64,
    pub had_invalid_tail: bool,
}

impl LogScan {
    pub(crate) fn empty(epoch: i64) -> Self {
        Self {
            epoch,
            generation: 0,
            cursor: String::new(),
            latest_seq: 0,
            byte_len: 0,
            offsets: Vec::new(),
            truncated_bytes: 0,
            had_invalid_tail: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    kind: SegmentKind,
    speaker: String,
    start: String,
    end: String,
    text: String,
    sources: Vec<String>,
}

pub(crate) fn normalize_page(
    drafts: Vec<SegmentDraft>,
    first_seq: i64,
) -> Result<Vec<MeetingSegment>, MeetingError> {
    if drafts.len() > MAX_PAGE_ITEMS {
        return Err(MeetingError::bad_request("events page exceeds 100 items"));
    }
    if first_seq <= 0 {
        return Err(MeetingError::storage(
            "next meeting segment sequence must be positive",
        ));
    }

    let mut candidates = Vec::<Candidate>::with_capacity(drafts.len());
    for draft in drafts {
        if draft.source_id.is_empty() {
            return Err(MeetingError::bad_request(
                "meeting event source id must not be empty",
            ));
        }
        let start = normalize_timestamp("start", &draft.start)?;
        let end = normalize_timestamp("end", &draft.end)?;
        if parse_timestamp(&end)? < parse_timestamp(&start)? {
            return Err(MeetingError::bad_request(
                "segment end must not precede start",
            ));
        }
        let candidate = Candidate {
            kind: draft.kind,
            speaker: truncate_utf8(draft.speaker.trim(), 256),
            start,
            end,
            text: draft.text,
            sources: vec![normalize_source_id(&draft.source_id)],
        };

        let prospective_seq = first_seq
            .checked_add(
                i64::try_from(candidates.len()).map_err(|_| {
                    MeetingError::storage("meeting page item count exceeds i64 range")
                })?,
            )
            .and_then(|seq| seq.checked_sub(1))
            .ok_or_else(|| MeetingError::storage("meeting segment sequence exhausted i64 range"))?;
        if let Some(previous) = candidates.last_mut()
            && can_coalesce(previous, &candidate, prospective_seq)?
        {
            previous.text.push('\n');
            previous.text.push_str(&candidate.text);
            previous.end = candidate.end;
            previous.sources.extend(candidate.sources);
            continue;
        }
        candidates.push(candidate);
    }

    let mut segments = Vec::new();
    let mut seq = first_seq;
    for candidate in candidates {
        let unsplit = candidate_as_segment(&candidate, seq, false, candidate.text.clone());
        if encoded_segment_line_len(&unsplit)? <= MAX_SEGMENT_LINE_BYTES {
            segments.push(unsplit);
            seq = seq.checked_add(1).ok_or_else(|| {
                MeetingError::storage("meeting segment sequence exhausted i64 range")
            })?;
            continue;
        }

        let mut remaining = candidate.text.as_str();
        while !remaining.is_empty() {
            let take = largest_fitting_prefix(&candidate, seq, remaining)?;
            if take == 0 {
                return Err(MeetingError::bad_request(
                    "segment metadata leaves no room for text within the 32 KiB line bound",
                ));
            }
            let chunk = remaining[..take].to_owned();
            segments.push(candidate_as_segment(&candidate, seq, true, chunk));
            remaining = &remaining[take..];
            seq = seq.checked_add(1).ok_or_else(|| {
                MeetingError::storage("meeting segment sequence exhausted i64 range")
            })?;
        }
    }
    Ok(segments)
}

fn can_coalesce(
    previous: &Candidate,
    next: &Candidate,
    prospective_seq: i64,
) -> Result<bool, MeetingError> {
    if previous.kind != SegmentKind::Transcript
        || next.kind != SegmentKind::Transcript
        || previous.speaker != next.speaker
    {
        return Ok(false);
    }
    let gap = parse_timestamp(&next.start)?
        .signed_duration_since(parse_timestamp(&previous.end)?)
        .num_seconds();
    if !(0..=60).contains(&gap) {
        return Ok(false);
    }
    let mut merged = previous.clone();
    merged.text.push('\n');
    merged.text.push_str(&next.text);
    merged.end.clone_from(&next.end);
    merged.sources.extend(next.sources.iter().cloned());
    let segment = candidate_as_segment(&merged, prospective_seq, false, merged.text.clone());
    Ok(encoded_segment_line_len(&segment)? <= MAX_SEGMENT_LINE_BYTES)
}

fn candidate_as_segment(
    candidate: &Candidate,
    seq: i64,
    cont: bool,
    text: String,
) -> MeetingSegment {
    MeetingSegment {
        seq,
        kind: candidate.kind,
        speaker: candidate.speaker.clone(),
        start: candidate.start.clone(),
        end: candidate.end.clone(),
        text,
        sources: candidate.sources.clone(),
        cont,
    }
}

fn largest_fitting_prefix(
    candidate: &Candidate,
    seq: i64,
    text: &str,
) -> Result<usize, MeetingError> {
    let mut boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    boundaries.push(text.len());
    if boundaries.first() == Some(&0) {
        boundaries.remove(0);
    }
    let mut low = 0usize;
    let mut high = boundaries.len();
    while low < high {
        let mid = (low + high) / 2;
        let end = boundaries[mid];
        let segment = candidate_as_segment(candidate, seq, true, text[..end].to_owned());
        if encoded_segment_line_len(&segment)? <= MAX_SEGMENT_LINE_BYTES {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    if low == 0 {
        Ok(0)
    } else {
        Ok(boundaries[low - 1])
    }
}

pub(crate) fn encoded_segment_line(segment: &MeetingSegment) -> Result<Vec<u8>, MeetingError> {
    let line = SegmentLine {
        t: "seg".to_owned(),
        segment: segment.clone(),
    };
    serde_json::to_vec(&line).map_err(MeetingError::from)
}

fn encoded_segment_line_len(segment: &MeetingSegment) -> Result<usize, MeetingError> {
    Ok(encoded_segment_line(segment)?.len())
}

pub(crate) fn checkpoint_line(
    epoch: i64,
    cursor_out: &str,
    at: &str,
) -> Result<Vec<u8>, MeetingError> {
    if epoch < 0 {
        return Err(MeetingError::storage(
            "checkpoint epoch must be non-negative",
        ));
    }
    if cursor_out.len() > 1_024 {
        return Err(MeetingError::bad_request("cursor_out exceeds 1024 bytes"));
    }
    let at = normalize_timestamp("at", at)?;
    serde_json::to_vec(&CheckpointLine {
        t: "ckpt".to_owned(),
        epoch,
        cursor_out: cursor_out.to_owned(),
        at,
    })
    .map_err(MeetingError::from)
}

pub(crate) fn append_lines_and_sync(
    path: &Path,
    segments: &[MeetingSegment],
    checkpoint: &[u8],
    stop_after_segments: Option<usize>,
    skip_sync: bool,
) -> Result<u64, MeetingError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .map_err(|error| MeetingError::io("open meeting log for append", error))?;
    for (index, segment) in segments.iter().enumerate() {
        let line = encoded_segment_line(segment)?;
        debug_assert!(line.len() <= MAX_SEGMENT_LINE_BYTES);
        file.write_all(&line)
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| MeetingError::io("append meeting segment", error))?;
        if stop_after_segments == Some(index + 1) {
            file.flush()
                .map_err(|error| MeetingError::io("flush uncommitted meeting page", error))?;
            return Err(MeetingError::injected(
                "crash after segment before checkpoint",
            ));
        }
    }
    file.write_all(checkpoint)
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|error| MeetingError::io("append meeting checkpoint", error))?;
    if skip_sync {
        file.flush()
            .map_err(|error| MeetingError::io("flush meeting checkpoint", error))?;
        return Err(MeetingError::injected(
            "crash after checkpoint before fdatasync",
        ));
    }
    file.sync_data()
        .map_err(|error| MeetingError::io("fdatasync meeting page", error))?;
    file.seek(SeekFrom::End(0))
        .map_err(|error| MeetingError::io("measure meeting log", error))
}

pub(crate) fn scan_log(
    path: &Path,
    epoch: i64,
    truncate_invalid_tail: bool,
) -> Result<LogScan, MeetingError> {
    if epoch < 0 {
        return Err(MeetingError::storage("meeting log epoch is negative"));
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogScan::empty(epoch));
        }
        Err(error) => return Err(MeetingError::io("open meeting log", error)),
    };
    let original_len = file
        .metadata()
        .map_err(|error| MeetingError::io("inspect meeting log", error))?
        .len();
    let mut reader = BufReader::new(file);
    let mut position = 0u64;
    let mut valid_end = 0u64;
    let mut next_seq = 1i64;
    let mut committed_latest = 0i64;
    let mut generation = 0i64;
    let mut cursor = String::new();
    let mut committed_offsets = Vec::new();
    let mut pending_offsets = Vec::new();
    let mut pending_segments = false;
    let mut invalid = false;

    loop {
        let line_start = position;
        let mut raw = Vec::new();
        let read = reader
            .read_until(b'\n', &mut raw)
            .map_err(|error| MeetingError::io("read meeting log", error))?;
        if read == 0 {
            break;
        }
        position = position.saturating_add(read as u64);
        if raw.last() != Some(&b'\n') {
            invalid = true;
            break;
        }
        raw.pop();
        if raw.last() == Some(&b'\r') || raw.is_empty() {
            invalid = true;
            break;
        }
        let value: serde_json::Value = match serde_json::from_slice(&raw) {
            Ok(value) => value,
            Err(_) => {
                invalid = true;
                break;
            }
        };
        match value.get("t").and_then(serde_json::Value::as_str) {
            Some("seg") => {
                if !has_exact_keys(
                    &value,
                    &[
                        "t", "seq", "kind", "speaker", "start", "end", "text", "sources", "cont",
                    ],
                ) {
                    invalid = true;
                    break;
                }
                let line: SegmentLine = match serde_json::from_value::<SegmentLine>(value) {
                    Ok(line) if line.t == "seg" => line,
                    _ => {
                        invalid = true;
                        break;
                    }
                };
                if validate_segment(&line.segment, raw.len(), next_seq).is_err() {
                    invalid = true;
                    break;
                }
                if line.segment.seq % INDEX_STRIDE == 0 {
                    pending_offsets.push(SparseOffset {
                        seq: line.segment.seq,
                        offset: line_start,
                    });
                }
                next_seq = match next_seq.checked_add(1) {
                    Some(next) => next,
                    None => {
                        invalid = true;
                        break;
                    }
                };
                pending_segments = true;
            }
            Some("ckpt") => {
                if !has_exact_keys(&value, &["t", "epoch", "cursor_out", "at"]) {
                    invalid = true;
                    break;
                }
                let line: CheckpointLine = match serde_json::from_value::<CheckpointLine>(value) {
                    Ok(line) if line.t == "ckpt" => line,
                    _ => {
                        invalid = true;
                        break;
                    }
                };
                if line.epoch != epoch
                    || line.cursor_out.len() > 1_024
                    || !normalize_timestamp("at", &line.at)
                        .is_ok_and(|canonical| canonical == line.at)
                {
                    invalid = true;
                    break;
                }
                generation = match generation.checked_add(1) {
                    Some(next) => next,
                    None => {
                        invalid = true;
                        break;
                    }
                };
                committed_latest = next_seq - 1;
                cursor = line.cursor_out;
                committed_offsets.append(&mut pending_offsets);
                pending_segments = false;
                valid_end = position;
            }
            _ => {
                invalid = true;
                break;
            }
        }
    }

    let had_invalid_tail = invalid || pending_segments || position > valid_end;
    let truncated_bytes = original_len.saturating_sub(valid_end);
    if truncate_invalid_tail && had_invalid_tail && original_len != valid_end {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| MeetingError::io("open meeting log for repair", error))?;
        file.set_len(valid_end)
            .map_err(|error| MeetingError::io("truncate meeting log tail", error))?;
        file.sync_data()
            .map_err(|error| MeetingError::io("fdatasync repaired meeting log", error))?;
    }

    Ok(LogScan {
        epoch,
        generation,
        cursor,
        latest_seq: committed_latest,
        byte_len: valid_end,
        offsets: committed_offsets,
        truncated_bytes,
        had_invalid_tail,
    })
}

fn has_exact_keys(value: &serde_json::Value, expected: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

pub(crate) fn read_segments(
    path: &Path,
    offsets: &[SparseOffset],
    start: i64,
    end: i64,
    snapshot_byte_len: u64,
    epoch: i64,
) -> Result<Vec<MeetingSegment>, MeetingError> {
    if start <= 0 || end < start {
        return Ok(Vec::new());
    }
    let mut file = File::open(path).map_err(|error| MeetingError::io("open meeting log", error))?;
    let seek_entry = offsets
        .iter()
        .rev()
        .find(|entry| entry.seq <= start)
        .map(|entry| (entry.seq, entry.offset));
    let (mut expected_seq, seek) = seek_entry.unwrap_or((1, 0));
    file.seek(SeekFrom::Start(seek))
        .map_err(|error| MeetingError::io("seek meeting log", error))?;
    let mut reader = BufReader::new(file);
    let mut position = seek;
    let mut segments = Vec::new();
    while position < snapshot_byte_len {
        let mut raw = Vec::new();
        let read = reader
            .read_until(b'\n', &mut raw)
            .map_err(|error| MeetingError::io("read meeting log span", error))?;
        if read == 0 {
            break;
        }
        position = position.saturating_add(read as u64);
        if position > snapshot_byte_len || raw.last() != Some(&b'\n') {
            break;
        }
        raw.pop();
        let value: serde_json::Value = serde_json::from_slice(&raw)?;
        match value.get("t").and_then(serde_json::Value::as_str) {
            Some("seg")
                if has_exact_keys(
                    &value,
                    &[
                        "t", "seq", "kind", "speaker", "start", "end", "text", "sources", "cont",
                    ],
                ) =>
            {
                let line: SegmentLine = serde_json::from_value(value)?;
                validate_segment(&line.segment, raw.len(), expected_seq)?;
                expected_seq = expected_seq.checked_add(1).ok_or_else(|| {
                    MeetingError::storage("meeting segment sequence exhausted i64 range")
                })?;
                if line.segment.seq < start {
                    continue;
                }
                if line.segment.seq > end {
                    break;
                }
                segments.push(line.segment);
            }
            Some("ckpt") if has_exact_keys(&value, &["t", "epoch", "cursor_out", "at"]) => {
                let line: CheckpointLine = serde_json::from_value(value)?;
                if line.epoch != epoch
                    || line.cursor_out.len() > 1_024
                    || !normalize_timestamp("at", &line.at)
                        .is_ok_and(|canonical| canonical == line.at)
                {
                    return Err(MeetingError::storage(
                        "meeting checkpoint bounds are invalid during read",
                    ));
                }
            }
            _ => {
                return Err(MeetingError::storage(
                    "meeting log contains a non-canonical record during read",
                ));
            }
        }
    }
    Ok(segments)
}

fn validate_segment(
    segment: &MeetingSegment,
    encoded_len: usize,
    expected_seq: i64,
) -> Result<(), MeetingError> {
    if segment.seq != expected_seq || segment.seq <= 0 {
        return Err(MeetingError::storage(
            "meeting log has non-dense segment sequence",
        ));
    }
    if encoded_len > MAX_SEGMENT_LINE_BYTES {
        return Err(MeetingError::storage("meeting segment line exceeds 32 KiB"));
    }
    if segment.speaker.len() > 256 {
        return Err(MeetingError::storage(
            "meeting segment speaker exceeds 256 bytes",
        ));
    }
    let start = normalize_timestamp("start", &segment.start)?;
    let end = normalize_timestamp("end", &segment.end)?;
    if start != segment.start || end != segment.end {
        return Err(MeetingError::storage(
            "meeting segment timestamps are not canonical",
        ));
    }
    if parse_timestamp(&end)? < parse_timestamp(&start)? {
        return Err(MeetingError::storage("meeting segment end precedes start"));
    }
    if segment.sources.len() > MAX_PAGE_ITEMS
        || segment
            .sources
            .iter()
            .any(|source| !is_normalized_source_id(source))
    {
        return Err(MeetingError::storage(
            "meeting segment source bounds are invalid",
        ));
    }
    Ok(())
}

fn is_normalized_source_id(source: &str) -> bool {
    if source.is_empty() {
        return false;
    }
    if source.len() <= 64 {
        return true;
    }
    source.len() == 71
        && source.strip_prefix("sha256:").is_some_and(|digest| {
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

pub(crate) fn normalize_timestamp(field: &str, value: &str) -> Result<String, MeetingError> {
    let trimmed = value.trim();
    if trimmed.len() != 24 || !trimmed.ends_with('Z') {
        return Err(MeetingError::bad_request(format!(
            "{field} must be fixed-width RFC3339 milliseconds UTC"
        )));
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed).map_err(|_| {
        MeetingError::bad_request(format!(
            "{field} must be fixed-width RFC3339 milliseconds UTC"
        ))
    })?;
    Ok(parsed
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, MeetingError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| MeetingError::bad_request("invalid segment timestamp"))
}

pub(crate) fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn normalize_source_id(value: &str) -> String {
    if value.len() <= 64 {
        return value.to_owned();
    }
    let mut hasher = Sha256::new();
    hasher.update(b"garyx-meeting-source-id-v1\0");
    hasher.update(value.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

pub fn share_text(title: &str, url: &str) -> String {
    format!(
        "{}\n{}",
        truncate_utf8(title, 256),
        truncate_utf8(url, 1_024)
    )
}

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

pub(crate) fn ensure_parent(path: &Path) -> Result<(), MeetingError> {
    let Some(parent) = path.parent() else {
        return Err(MeetingError::storage("meeting log path has no parent"));
    };
    fs::create_dir_all(parent)
        .map_err(|error| MeetingError::io("create meeting content directory", error))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn timestamp(second: u8) -> String {
        format!("2026-07-16T02:35:{second:02}.123Z")
    }

    fn transcript(text: String, source_id: String) -> SegmentDraft {
        SegmentDraft {
            kind: SegmentKind::Transcript,
            speaker: "Test Speaker".to_owned(),
            start: timestamp(0),
            end: timestamp(1),
            text,
            source_id,
        }
    }

    #[test]
    fn normalization_enforces_line_and_source_bounds_with_escape_inflation() {
        let segments = normalize_page(
            vec![transcript("\"\\\n".repeat(30_000), "source".repeat(40))],
            1,
        )
        .expect("normalize");
        assert!(segments.len() > 1);
        assert!(segments.iter().all(|segment| segment.cont));
        assert!(segments.iter().all(|segment| {
            encoded_segment_line(segment).expect("encode").len() <= MAX_SEGMENT_LINE_BYTES
        }));
        assert!(segments.iter().all(|segment| {
            segment.sources.len() == 1
                && segment.sources[0].starts_with("sha256:")
                && segment.sources[0].len() == 71
        }));
        assert_eq!(
            normalize_source_id(&"source".repeat(40)),
            normalize_source_id(&"source".repeat(40))
        );
    }

    #[test]
    fn source_id_boundary_is_exact_and_canonical() {
        let raw = "r".repeat(64);
        assert_eq!(normalize_source_id(&raw), raw);
        let hashed = normalize_source_id(&"h".repeat(65));
        assert_eq!(hashed.len(), 71);
        assert!(hashed.starts_with("sha256:"));
        assert!(is_normalized_source_id(&hashed));
        assert!(!is_normalized_source_id(&"x".repeat(65)));
        assert!(!is_normalized_source_id(&format!(
            "sha256:{}",
            "A".repeat(64)
        )));
    }

    #[test]
    fn coalescing_only_happens_when_the_merged_line_fits() {
        let first = transcript("a".repeat(20_000), "first".to_owned());
        let mut second = transcript("b".repeat(20_000), "second".to_owned());
        second.start = timestamp(2);
        second.end = timestamp(3);
        let segments = normalize_page(vec![first, second], 1).expect("normalize");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].sources, vec!["first"]);
        assert_eq!(segments[1].sources, vec!["second"]);
        assert!(!segments[0].cont);
        assert!(!segments[1].cont);

        let small = normalize_page(
            vec![transcript("one".to_owned(), "one".to_owned()), {
                let mut draft = transcript("two".to_owned(), "two".to_owned());
                draft.start = timestamp(2);
                draft.end = timestamp(3);
                draft
            }],
            1,
        )
        .expect("small merge");
        assert_eq!(small.len(), 1);
        assert_eq!(small[0].text, "one\ntwo");
        assert_eq!(small[0].sources, vec!["one", "two"]);
    }

    #[test]
    fn every_truncated_field_stops_on_a_utf8_boundary() {
        let title = "😀".repeat(100);
        let url = "界".repeat(500);
        let rendered = share_text(&title, &url);
        let (title, url) = rendered.split_once('\n').expect("share pair");
        assert!(title.len() <= 256);
        assert!(url.len() <= 1_024);
        assert!(std::str::from_utf8(title.as_bytes()).is_ok());
        assert!(std::str::from_utf8(url.as_bytes()).is_ok());

        let mut draft = transcript("body".to_owned(), "source".to_owned());
        draft.speaker = "😀".repeat(100);
        let segment = normalize_page(vec![draft], 1).expect("normalize")[0].clone();
        assert!(segment.speaker.len() <= 256);
        assert!(std::str::from_utf8(segment.speaker.as_bytes()).is_ok());
    }

    #[test]
    fn maximum_page_member_count_stays_bounded_in_one_coalesced_record() {
        let drafts = (0..MAX_PAGE_ITEMS)
            .map(|index| {
                let mut draft = transcript("x".to_owned(), format!("source-{index}"));
                draft.start = timestamp(1);
                draft.end = timestamp(1);
                draft
            })
            .collect();
        let segments = normalize_page(drafts, 1).expect("normalize");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].sources.len(), MAX_PAGE_ITEMS);
        assert!(encoded_segment_line(&segments[0]).expect("line").len() <= MAX_SEGMENT_LINE_BYTES);
    }

    prop_compose! {
        fn bounded_unicode(max_chars: usize)
            (chars in prop::collection::vec(any::<char>(), 0..=max_chars)) -> String {
            chars.into_iter().collect()
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn normative_record_bounds_hold_for_generated_unicode_and_escape_patterns(
            speaker in bounded_unicode(300),
            text in bounded_unicode(2_000),
            source in bounded_unicode(150).prop_filter("source id is non-empty", |value| !value.is_empty()),
            first_seq in 1i64..1_000_000i64,
        ) {
            let segments = normalize_page(
                vec![SegmentDraft {
                    kind: SegmentKind::Transcript,
                    speaker,
                    start: timestamp(0),
                    end: timestamp(1),
                    text,
                    source_id: source,
                }],
                first_seq,
            )
            .expect("generated record normalizes");
            prop_assert!(!segments.is_empty());
            for segment in segments {
                prop_assert!(segment.speaker.len() <= 256);
                prop_assert!(std::str::from_utf8(segment.speaker.as_bytes()).is_ok());
                prop_assert!(segment.sources.iter().all(|source| !source.is_empty() && source.len() <= 71));
                prop_assert!(encoded_segment_line(&segment).expect("line").len() <= MAX_SEGMENT_LINE_BYTES);
            }
        }

        #[test]
        fn generated_share_fields_truncate_on_utf8_boundaries(
            title in bounded_unicode(400),
            url in bounded_unicode(1_400),
        ) {
            let rendered = share_text(&title, &url);
            let normalized_title = truncate_utf8(&title, 256);
            let normalized_url = truncate_utf8(&url, 1_024);
            prop_assert_eq!(
                rendered,
                format!("{normalized_title}\n{normalized_url}")
            );
            prop_assert!(normalized_title.len() <= 256);
            prop_assert!(normalized_url.len() <= 1_024);
            prop_assert!(std::str::from_utf8(normalized_title.as_bytes()).is_ok());
            prop_assert!(std::str::from_utf8(normalized_url.as_bytes()).is_ok());
        }
    }
}
