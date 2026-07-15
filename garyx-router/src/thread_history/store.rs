use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(test)]
use std::sync::atomic::AtomicBool;

use super::*;

const TAIL_SCAN_CHUNK_BYTES: u64 = 64 * 1024;

/// Per-thread budget for the parsed transcript tail kept in memory.
const TRANSCRIPT_CACHE_TAIL_MAX_BYTES: usize = 8 * 1024 * 1024;
const TRANSCRIPT_CACHE_TAIL_MAX_RECORDS: usize = 4096;
/// Store-wide cache budget; least-recently-used thread entries are dropped
/// once the sum of cached tail bytes exceeds it.
const TRANSCRIPT_CACHE_TOTAL_MAX_BYTES: usize = 64 * 1024 * 1024;

#[async_trait]
trait TranscriptAtomicFs: std::fmt::Debug + Send + Sync {
    async fn write_temp(&self, path: &Path, payload: &[u8]) -> std::io::Result<tokio::fs::File>;
    async fn fsync_file(&self, file: &tokio::fs::File) -> std::io::Result<()>;
    async fn rename(&self, source: &Path, target: &Path) -> std::io::Result<()>;
    async fn fsync_parent(&self, parent: &Path) -> std::io::Result<()>;
}

#[derive(Debug, Default)]
struct RealTranscriptAtomicFs;

#[async_trait]
impl TranscriptAtomicFs for RealTranscriptAtomicFs {
    async fn write_temp(&self, path: &Path, payload: &[u8]) -> std::io::Result<tokio::fs::File> {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .await?;
        file.write_all(payload).await?;
        file.flush().await?;
        Ok(file)
    }

    async fn fsync_file(&self, file: &tokio::fs::File) -> std::io::Result<()> {
        file.sync_all().await
    }

    async fn rename(&self, source: &Path, target: &Path) -> std::io::Result<()> {
        tokio::fs::rename(source, target).await
    }

    async fn fsync_parent(&self, parent: &Path) -> std::io::Result<()> {
        tokio::fs::File::open(parent).await?.sync_all().await
    }
}

#[cfg(test)]
#[derive(Debug)]
struct FailOnceTranscriptAtomicFs {
    inner: RealTranscriptAtomicFs,
    stage: TranscriptReplaceStage,
    failed: AtomicBool,
}

#[cfg(test)]
impl FailOnceTranscriptAtomicFs {
    fn new(stage: TranscriptReplaceStage) -> Self {
        Self {
            inner: RealTranscriptAtomicFs,
            stage,
            failed: AtomicBool::new(false),
        }
    }

    fn fail_now(&self, stage: TranscriptReplaceStage) -> std::io::Result<()> {
        if self.stage == stage && !self.failed.swap(true, Ordering::SeqCst) {
            return Err(std::io::Error::other(format!("injected {stage} failure")));
        }
        Ok(())
    }
}

#[cfg(test)]
#[async_trait]
impl TranscriptAtomicFs for FailOnceTranscriptAtomicFs {
    async fn write_temp(&self, path: &Path, payload: &[u8]) -> std::io::Result<tokio::fs::File> {
        self.fail_now(TranscriptReplaceStage::TempWrite)?;
        self.inner.write_temp(path, payload).await
    }

    async fn fsync_file(&self, file: &tokio::fs::File) -> std::io::Result<()> {
        self.fail_now(TranscriptReplaceStage::FileFsync)?;
        self.inner.fsync_file(file).await
    }

    async fn rename(&self, source: &Path, target: &Path) -> std::io::Result<()> {
        self.fail_now(TranscriptReplaceStage::Rename)?;
        self.inner.rename(source, target).await
    }

    async fn fsync_parent(&self, parent: &Path) -> std::io::Result<()> {
        self.fail_now(TranscriptReplaceStage::ParentFsync)?;
        self.inner.fsync_parent(parent).await
    }
}

#[derive(Debug, Clone, Copy)]
struct TranscriptCacheBudget {
    tail_max_bytes: usize,
    tail_max_records: usize,
    total_max_bytes: usize,
}

impl Default for TranscriptCacheBudget {
    fn default() -> Self {
        Self {
            tail_max_bytes: TRANSCRIPT_CACHE_TAIL_MAX_BYTES,
            tail_max_records: TRANSCRIPT_CACHE_TAIL_MAX_RECORDS,
            total_max_bytes: TRANSCRIPT_CACHE_TOTAL_MAX_BYTES,
        }
    }
}

/// Per-thread I/O + cache slot. `state` is the thread's write lock — every
/// file mutation for the thread runs its whole read-modify-write under it —
/// and owns the thread's cache entry. The atomics mirror the entry's size and
/// last-use time so the store-wide eviction pass can account for and pick
/// victims without locking other threads' slots. Slots are never removed from
/// the registry (the `Arc` identity IS the per-thread lock identity); eviction
/// only clears the cache content.
#[derive(Debug, Default)]
struct ThreadSlot {
    state: Mutex<Option<ThreadCache>>,
    cached_bytes: AtomicUsize,
    last_used_ms: AtomicU64,
}

#[derive(Debug)]
struct CachedTranscriptRecord {
    record: ThreadTranscriptRecord,
    /// Serialized jsonl line length — the record's memory estimate.
    bytes: usize,
}

/// A transcript record message that belongs to the provider session
/// content: run content rows (user/assistant/tool traffic), excluding run
/// control records. The legacy thread-record `messages` snapshot only ever
/// contained content rows, so this is the equivalence filter for its
/// transcript-backed replacement (#TASK-1864). Internal dispatch user
/// messages carry a top-level `internal: true` but are session content
/// (the legacy snapshot kept them) — only `kind == "control"` marks a
/// control record.
fn is_provider_session_message(message: &Value) -> bool {
    let Some(object) = message.as_object() else {
        return false;
    };
    if object.get("kind").and_then(Value::as_str) == Some("control") {
        return false;
    }
    object.get("role").and_then(Value::as_str).is_some()
}

/// Newest `limit` provider-session messages from an ordered record walk,
/// returned in transcript order.
fn provider_session_tail_from_messages<'a>(
    messages: impl DoubleEndedIterator<Item = &'a Value>,
    limit: usize,
) -> Vec<Value> {
    let mut tail: Vec<Value> = messages
        .rev()
        .filter(|message| is_provider_session_message(message))
        .take(limit)
        .cloned()
        .collect();
    tail.reverse();
    tail
}

/// Parsed tail + run-state checkpoint for one thread's transcript.
/// Invariants (maintained under the slot lock; seqs are monotonic + gapless,
/// which `records_after_seq`'s tail scan and the persistence layer's
/// "last K records are seqs total-K+1..=total" already rely on):
/// - `checkpoint` equals the run-state fold of every record with
///   `seq <= base_seq`;
/// - `tail` holds exactly the records with `base_seq < seq <= last_seq`;
/// - `min_seq`/`last_seq`/`total_records` describe the whole file
///   (`min_seq == 0` only when the transcript is empty);
/// - `file_len` is the on-disk length after our last verified read/write, so
///   any out-of-band mutation is caught by one fstat and drops the entry.
#[derive(Debug)]
struct ThreadCache {
    checkpoint: TranscriptRunState,
    render_prefix_checkpoint: TranscriptRenderPrefixState,
    base_seq: u64,
    tail: Vec<CachedTranscriptRecord>,
    tail_bytes: usize,
    min_seq: u64,
    last_seq: u64,
    total_records: usize,
    file_len: u64,
}

impl ThreadCache {
    fn from_records(records: Vec<CachedTranscriptRecord>, file_len: u64) -> Self {
        let min_seq = records.first().map(|c| c.record.seq).unwrap_or(0);
        let last_seq = records.last().map(|c| c.record.seq).unwrap_or(0);
        let total_records = records.len();
        Self {
            checkpoint: TranscriptRunState::default(),
            render_prefix_checkpoint: TranscriptRenderPrefixState::default(),
            base_seq: min_seq.saturating_sub(1),
            tail: records,
            tail_bytes: 0,
            min_seq,
            last_seq,
            total_records,
            file_len,
        }
        .with_recomputed_tail_bytes()
    }

    fn with_recomputed_tail_bytes(mut self) -> Self {
        self.tail_bytes = self.tail.iter().map(|c| c.bytes).sum();
        self
    }

    fn tail_start_seq(&self) -> u64 {
        self.base_seq + 1
    }

    fn cache_bytes(&self) -> usize {
        self.tail_bytes
            .saturating_add(self.render_prefix_checkpoint.estimated_heap_bytes())
    }

    fn covers_whole_file(&self) -> bool {
        self.total_records == self.tail.len()
    }

    fn last_message_at(&self) -> Option<String> {
        self.tail.last().map(|c| c.record.timestamp.clone())
    }

    fn fold_record(state: &mut TranscriptRunState, record: &ThreadTranscriptRecord) {
        if let Ok(value) = serde_json::to_value(record) {
            apply_transcript_record(state, &value);
        }
    }

    fn fold_checkpoint_record(
        run_state: &mut TranscriptRunState,
        render_prefix_state: &mut TranscriptRenderPrefixState,
        record: &ThreadTranscriptRecord,
    ) {
        if let Ok(value) = serde_json::to_value(record) {
            apply_transcript_record(run_state, &value);
            apply_transcript_render_prefix_record(render_prefix_state, &value);
        }
    }

    /// Fold the oldest tail records into the checkpoint until the tail fits
    /// the per-thread budget again. Always keeps at least one record so the
    /// tail stays anchored at the file's end.
    fn roll_tail(&mut self, budget: &TranscriptCacheBudget) {
        let mut drained = 0usize;
        while self.tail.len() - drained > 1
            && (self.tail.len() - drained > budget.tail_max_records
                || self.tail_bytes > budget.tail_max_bytes)
        {
            let cached = &self.tail[drained];
            Self::fold_checkpoint_record(
                &mut self.checkpoint,
                &mut self.render_prefix_checkpoint,
                &cached.record,
            );
            self.base_seq = cached.record.seq;
            self.tail_bytes -= cached.bytes;
            drained += 1;
        }
        if drained > 0 {
            self.tail.drain(..drained);
        }
    }

    /// Streaming-build variant of `push_appended`: accepts one parsed record
    /// at a time and defers `roll_tail` until the tail overshoots the budget
    /// by 2x, so the front-drain cost amortizes to O(1) per record. Fold
    /// order is unchanged, so after a final `roll_tail` the checkpoint/tail
    /// state is identical to a full read followed by one roll.
    fn push_streamed(&mut self, cached: CachedTranscriptRecord, budget: &TranscriptCacheBudget) {
        if self.min_seq == 0 {
            self.min_seq = cached.record.seq;
            self.base_seq = cached.record.seq.saturating_sub(1);
        }
        self.last_seq = cached.record.seq;
        self.total_records += 1;
        self.tail_bytes += cached.bytes;
        self.tail.push(cached);
        if self.tail.len() > budget.tail_max_records.saturating_mul(2)
            || self.tail_bytes > budget.tail_max_bytes.saturating_mul(2)
        {
            self.roll_tail(budget);
        }
    }

    fn push_appended(
        &mut self,
        appended: &[ThreadTranscriptRecord],
        line_sizes: &[usize],
        file_len: u64,
        budget: &TranscriptCacheBudget,
    ) {
        for (record, bytes) in appended.iter().zip(line_sizes.iter().copied()) {
            if self.min_seq == 0 {
                self.min_seq = record.seq;
            }
            self.last_seq = record.seq;
            self.total_records += 1;
            self.tail_bytes += bytes;
            self.tail.push(CachedTranscriptRecord {
                record: record.clone(),
                bytes,
            });
        }
        self.file_len = file_len;
        self.roll_tail(budget);
    }

    /// The trailing block of records tagged `run_id`, when the cache can
    /// PROVE it holds the whole block: either a record with a different
    /// run_id precedes it inside the tail, or the tail covers the whole file.
    fn trailing_run_block(&self, trimmed_run_id: &str) -> Option<&[CachedTranscriptRecord]> {
        let mut start = self.tail.len();
        while start > 0
            && self.tail[start - 1].record.run_id.as_deref().map(str::trim) == Some(trimmed_run_id)
        {
            start -= 1;
        }
        (start > 0 || self.covers_whole_file()).then(|| &self.tail[start..])
    }

    fn run_state_at(&self, based_on_seq: u64) -> TranscriptRunState {
        let mut state = self.checkpoint.clone();
        for cached in &self.tail {
            if cached.record.seq > based_on_seq {
                break;
            }
            Self::fold_record(&mut state, &cached.record);
        }
        state
    }

    fn render_prefix_state_before(
        &self,
        floor_seq: u64,
        based_on_seq: u64,
    ) -> TranscriptRenderPrefixState {
        let mut state = self.render_prefix_checkpoint.clone();
        let prefix_end = floor_seq.saturating_sub(1).min(based_on_seq);
        for cached in &self.tail {
            if cached.record.seq > prefix_end {
                break;
            }
            if let Ok(value) = serde_json::to_value(&cached.record) {
                apply_transcript_render_prefix_record(&mut state, &value);
            }
        }
        state
    }

    /// Serve `render_snapshot_in_window` when the cached tail covers the
    /// requested window. Returns `None` (caller falls back to the full read)
    /// when the floor or based_on bound reaches below the cached tail.
    fn render_snapshot_in_window(
        &self,
        floor_seq: u64,
        based_on_seq: u64,
    ) -> Option<RenderSnapshot> {
        if floor_seq == 0 {
            return None;
        }
        if self.total_records > 0
            && (based_on_seq < self.tail_start_seq() || floor_seq < self.tail_start_seq())
        {
            return None;
        }
        let run_state = self.run_state_at(based_on_seq);
        let render_prefix_state = self.render_prefix_state_before(floor_seq, based_on_seq);
        let window_values: Vec<Value> = self
            .tail
            .iter()
            .filter(|c| c.record.seq >= floor_seq && c.record.seq <= based_on_seq)
            .filter_map(|c| serde_json::to_value(&c.record).ok())
            .collect();
        let mut snapshot = reduce_transcript_render_state_with_prefix_state(
            window_values.iter(),
            &run_state,
            &render_prefix_state,
        );
        if snapshot.based_on_seq == 0 {
            snapshot.based_on_seq = based_on_seq.min(self.last_seq);
        }
        snapshot.window = Some(RenderWindow {
            floor_seq,
            has_more_above: self.total_records > 0 && self.min_seq < floor_seq,
        });
        Some(snapshot)
    }

    /// Serve messages with seq in `[start_seq, end_seq]` (inclusive) when the
    /// whole range provably lies inside the cached tail; `None` sends the
    /// caller to the streaming fallback.
    fn messages_in_seq_range(&self, start_seq: u64, end_seq: u64) -> Option<Vec<Value>> {
        if start_seq > end_seq {
            return Some(Vec::new());
        }
        if start_seq < self.tail_start_seq() || end_seq > self.last_seq {
            return None;
        }
        Some(
            self.tail
                .iter()
                .filter(|cached| cached.record.seq >= start_seq && cached.record.seq <= end_seq)
                .map(|cached| cached.record.message.clone())
                .collect(),
        )
    }

    fn tail_messages(&self, limit: usize) -> Option<Vec<Value>> {
        if limit >= self.total_records {
            if !self.covers_whole_file() {
                return None;
            }
            return Some(self.tail.iter().map(|c| c.record.message.clone()).collect());
        }
        if limit > self.tail.len() {
            return None;
        }
        let start = self.tail.len() - limit;
        Some(
            self.tail[start..]
                .iter()
                .map(|c| c.record.message.clone())
                .collect(),
        )
    }

    /// Newest `limit` provider-session (non-control) messages, or `None`
    /// when the cached tail cannot prove completeness: the filtered tail
    /// came up short of `limit` while older uncached records might still
    /// hold more content rows.
    fn provider_session_tail_messages(&self, limit: usize) -> Option<Vec<Value>> {
        let filtered =
            provider_session_tail_from_messages(self.tail.iter().map(|c| &c.record.message), limit);
        if filtered.len() >= limit || self.covers_whole_file() {
            return Some(filtered);
        }
        None
    }

    /// Serve `cold_open_user_turn_window` when the whole window provably lies
    /// inside the cached tail; `None` falls back to the full read.
    fn cold_open_window(&self, user_turns: usize, cap: usize) -> Option<ThreadTranscriptWindow> {
        let total = self.total_records;
        if total == 0 {
            return Some(ThreadTranscriptWindow {
                records: Vec::new(),
                floor_seq: 0,
                has_more_above: false,
            });
        }

        let tail_global_start = total - self.tail.len();
        let target_user_turns = user_turns.max(1);
        let mut start = total;
        let mut user_queries = 0usize;
        while start > tail_global_start && user_queries < target_user_turns {
            start -= 1;
            if is_user_query_message(&self.tail[start - tail_global_start].record.message) {
                user_queries += 1;
            }
        }
        if user_queries < target_user_turns && !self.covers_whole_file() {
            // The scan would have to continue below the cached tail.
            return None;
        }

        if user_queries == 0 {
            start = total.saturating_sub(cap.max(1));
        }
        if total.saturating_sub(start) > cap {
            start = total.saturating_sub(cap);
        }
        if start < tail_global_start {
            return None;
        }

        let window: Vec<ThreadTranscriptRecord> = self.tail[start - tail_global_start..]
            .iter()
            .map(|c| c.record.clone())
            .collect();
        let floor_seq = window.first().map(|record| record.seq).unwrap_or(0);
        Some(ThreadTranscriptWindow {
            records: window,
            floor_seq,
            has_more_above: start > 0,
        })
    }
}

#[derive(Debug)]
enum RecordsReconcilePlan {
    NoOp,
    /// Append `authoritative[skip..]`.
    AppendSuffix {
        skip: usize,
    },
    Rewrite {
        rebuilt: Vec<ThreadTranscriptRecord>,
        changed: Vec<ThreadTranscriptRecord>,
    },
}

fn plan_reconcile_run_records_tail(
    thread_id: &str,
    records: &[ThreadTranscriptRecord],
    trimmed_run_id: &str,
    authoritative: &[RunTranscriptRecordDraft],
) -> RecordsReconcilePlan {
    let mut split = records.len();
    while split > 0 && records[split - 1].run_id.as_deref().map(str::trim) == Some(trimmed_run_id) {
        split -= 1;
    }
    let existing_tail = &records[split..];
    if existing_tail.is_empty() {
        return RecordsReconcilePlan::AppendSuffix { skip: 0 };
    }

    let existing_identity: Vec<Value> = existing_tail
        .iter()
        .map(|record| message_identity(&record.message))
        .collect();
    let authoritative_identity: Vec<Value> = authoritative
        .iter()
        .map(|draft| message_identity(&draft.message))
        .collect();

    if existing_identity == authoritative_identity {
        let mut changed = Vec::new();
        let mut changed_same_seqs = Vec::new();
        let mut rebuilt = records.to_vec();
        for (offset, draft) in authoritative.iter().enumerate() {
            let existing = &existing_tail[offset];
            let replacement = record_from_draft_replacing(
                thread_id,
                Some(trimmed_run_id),
                existing.seq,
                draft,
                existing,
            );
            if replacement.timestamp != existing.timestamp
                || replacement.message != existing.message
            {
                rebuilt[split + offset] = replacement.clone();
                changed_same_seqs.push(replacement.seq);
                changed.push(replacement);
            }
        }
        if changed.is_empty() {
            return RecordsReconcilePlan::NoOp;
        }
        append_range_rewrite_marker(
            &mut rebuilt,
            &mut changed,
            thread_id,
            Some(trimmed_run_id),
            changed_same_seqs.iter().copied().min().unwrap_or(1),
            changed_same_seqs.iter().copied().max().unwrap_or(1),
            authoritative.len(),
            existing_tail.len(),
            "same_seq_overwrite",
        );
        return RecordsReconcilePlan::Rewrite { rebuilt, changed };
    }

    if authoritative_identity.len() > existing_identity.len()
        && authoritative_identity[..existing_identity.len()] == existing_identity[..]
    {
        let prefix_changed =
            authoritative
                .iter()
                .zip(existing_tail.iter())
                .any(|(draft, existing)| {
                    let replacement = record_from_draft_replacing(
                        thread_id,
                        Some(trimmed_run_id),
                        existing.seq,
                        draft,
                        existing,
                    );
                    replacement.timestamp != existing.timestamp
                        || replacement.message != existing.message
                });
        if !prefix_changed {
            return RecordsReconcilePlan::AppendSuffix {
                skip: existing_tail.len(),
            };
        }
    }

    if authoritative.len() >= existing_tail.len() {
        let mut changed = Vec::new();
        let mut changed_same_seqs = Vec::new();
        let mut rebuilt = records[..split].to_vec();
        let mut next_seq = existing_tail
            .first()
            .map(|record| record.seq)
            .unwrap_or_else(|| rebuilt.last().map(|record| record.seq + 1).unwrap_or(1));
        for (offset, draft) in authoritative.iter().enumerate() {
            let seq = if offset < existing_tail.len() {
                existing_tail[offset].seq
            } else {
                next_seq
            };
            let replacement = record_from_draft(thread_id, Some(trimmed_run_id), seq, draft);
            let replacement = if let Some(existing) = existing_tail.get(offset) {
                record_from_draft_replacing(thread_id, Some(trimmed_run_id), seq, draft, existing)
            } else {
                replacement
            };
            let is_changed = existing_tail
                .get(offset)
                .map(|existing| {
                    existing.timestamp != replacement.timestamp
                        || existing.message != replacement.message
                })
                .unwrap_or(true);
            if is_changed {
                if offset < existing_tail.len() {
                    changed_same_seqs.push(replacement.seq);
                }
                changed.push(replacement.clone());
            }
            rebuilt.push(replacement);
            next_seq = seq + 1;
        }
        if let (Some(start_seq), Some(end_seq)) = (
            changed_same_seqs.iter().copied().min(),
            changed_same_seqs.iter().copied().max(),
        ) {
            append_range_rewrite_marker(
                &mut rebuilt,
                &mut changed,
                thread_id,
                Some(trimmed_run_id),
                start_seq,
                end_seq,
                authoritative.len(),
                existing_tail.len(),
                "same_seq_overwrite",
            );
        }
        return RecordsReconcilePlan::Rewrite { rebuilt, changed };
    }

    if authoritative_identity.len() <= existing_identity.len()
        && existing_identity[..authoritative_identity.len()] == authoritative_identity[..]
        && existing_tail[authoritative.len()..]
            .iter()
            .all(|record| is_range_rewrite_control(&record.message))
    {
        let mut changed = Vec::new();
        let mut changed_same_seqs = Vec::new();
        let mut rebuilt = records.to_vec();
        for (offset, draft) in authoritative.iter().enumerate() {
            let existing = &existing_tail[offset];
            let replacement = record_from_draft_replacing(
                thread_id,
                Some(trimmed_run_id),
                existing.seq,
                draft,
                existing,
            );
            if replacement.timestamp != existing.timestamp
                || replacement.message != existing.message
            {
                rebuilt[split + offset] = replacement.clone();
                changed_same_seqs.push(replacement.seq);
                changed.push(replacement.clone());
            }
        }
        if changed.is_empty() {
            return RecordsReconcilePlan::NoOp;
        }
        if let (Some(start_seq), Some(end_seq)) = (
            changed_same_seqs.iter().copied().min(),
            changed_same_seqs.iter().copied().max(),
        ) {
            append_range_rewrite_marker(
                &mut rebuilt,
                &mut changed,
                thread_id,
                Some(trimmed_run_id),
                start_seq,
                end_seq,
                authoritative.len(),
                existing_tail.len(),
                "same_seq_overwrite",
            );
        }
        return RecordsReconcilePlan::Rewrite { rebuilt, changed };
    }

    let mut changed = Vec::new();
    let mut changed_same_seqs = Vec::new();
    let mut rebuilt = records[..split].to_vec();
    let first_rewritten_seq = existing_tail
        .get(authoritative.len())
        .map(|record| record.seq)
        .unwrap_or_else(|| existing_tail.first().map(|record| record.seq).unwrap_or(1));
    let last_rewritten_seq = existing_tail
        .last()
        .map(|record| record.seq)
        .unwrap_or(first_rewritten_seq);
    let rewrite_at = chrono::Utc::now().to_rfc3339();
    for (offset, existing) in existing_tail.iter().enumerate() {
        if let Some(draft) = authoritative.get(offset) {
            let replacement = record_from_draft_replacing(
                thread_id,
                Some(trimmed_run_id),
                existing.seq,
                draft,
                existing,
            );
            if replacement.timestamp != existing.timestamp
                || replacement.message != existing.message
            {
                changed_same_seqs.push(replacement.seq);
                changed.push(replacement.clone());
            }
            rebuilt.push(replacement);
        } else {
            let rewrite = build_range_rewrite_record(
                thread_id,
                Some(trimmed_run_id),
                existing.seq,
                first_rewritten_seq,
                last_rewritten_seq,
                authoritative.len(),
                existing_tail.len(),
                true,
                "run_tail_shrink",
                &rewrite_at,
            );
            if rewrite.timestamp != existing.timestamp || rewrite.message != existing.message {
                changed_same_seqs.push(rewrite.seq);
                changed.push(rewrite.clone());
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
    let rewrite = build_range_rewrite_record(
        thread_id,
        Some(trimmed_run_id),
        rebuilt.last().map(|record| record.seq + 1).unwrap_or(1),
        first_rewritten_seq,
        last_rewritten_seq,
        authoritative.len(),
        existing_tail.len(),
        false,
        "run_tail_shrink",
        &rewrite_at,
    );
    changed.push(rewrite.clone());
    rebuilt.push(rewrite);
    RecordsReconcilePlan::Rewrite { rebuilt, changed }
}

#[derive(Debug)]
enum TranscriptStoreMode {
    File {
        root_dir: PathBuf,
        slots: std::sync::Mutex<HashMap<String, Arc<ThreadSlot>>>,
        budget: TranscriptCacheBudget,
        created_at: std::time::Instant,
        atomic_fs: Arc<dyn TranscriptAtomicFs>,
    },
    Memory {
        records: Mutex<HashMap<String, Vec<ThreadTranscriptRecord>>>,
    },
}

#[derive(Debug)]
pub struct ThreadTranscriptStore {
    mode: TranscriptStoreMode,
    /// Counts whole-file transcript reads so tests can prove the hot paths
    /// actually hit the cache instead of silently falling back.
    #[cfg(test)]
    pub(super) full_file_reads: AtomicUsize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum TranscriptLine {
    Session {
        version: u32,
        thread_id: String,
        created_at: String,
    },
    Message {
        seq: u64,
        thread_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        timestamp: String,
        message: Value,
    },
}

#[derive(Debug)]
enum ValidatedImportTranscript {
    MissingOrEmpty,
    Parsed {
        records: Vec<ThreadTranscriptRecord>,
        torn_tail: bool,
        file_len: u64,
    },
}

impl ThreadTranscriptStore {
    pub async fn file(root_dir: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::file_with_budget(root_dir, TranscriptCacheBudget::default()).await
    }

    async fn file_with_budget(
        root_dir: impl AsRef<Path>,
        budget: TranscriptCacheBudget,
    ) -> std::io::Result<Self> {
        Self::file_with_budget_and_atomic_fs(root_dir, budget, Arc::new(RealTranscriptAtomicFs))
            .await
    }

    async fn file_with_budget_and_atomic_fs(
        root_dir: impl AsRef<Path>,
        budget: TranscriptCacheBudget,
        atomic_fs: Arc<dyn TranscriptAtomicFs>,
    ) -> std::io::Result<Self> {
        tokio::fs::create_dir_all(root_dir.as_ref()).await?;
        Ok(Self {
            mode: TranscriptStoreMode::File {
                root_dir: root_dir.as_ref().to_path_buf(),
                slots: std::sync::Mutex::new(HashMap::new()),
                budget,
                created_at: std::time::Instant::now(),
                atomic_fs,
            },
            #[cfg(test)]
            full_file_reads: AtomicUsize::new(0),
        })
    }

    #[cfg(test)]
    pub(super) async fn file_for_tests(
        root_dir: impl AsRef<Path>,
        tail_max_bytes: usize,
        tail_max_records: usize,
        total_max_bytes: usize,
    ) -> std::io::Result<Self> {
        Self::file_with_budget(
            root_dir,
            TranscriptCacheBudget {
                tail_max_bytes,
                tail_max_records,
                total_max_bytes,
            },
        )
        .await
    }

    #[cfg(test)]
    pub(super) async fn file_with_atomic_failure_for_tests(
        root_dir: impl AsRef<Path>,
        stage: TranscriptReplaceStage,
    ) -> std::io::Result<Self> {
        Self::file_with_budget_and_atomic_fs(
            root_dir,
            TranscriptCacheBudget::default(),
            Arc::new(FailOnceTranscriptAtomicFs::new(stage)),
        )
        .await
    }

    pub fn memory() -> Self {
        Self {
            mode: TranscriptStoreMode::Memory {
                records: Mutex::new(HashMap::new()),
            },
            #[cfg(test)]
            full_file_reads: AtomicUsize::new(0),
        }
    }

    pub fn transcript_path(&self, thread_id: &str) -> Option<PathBuf> {
        match &self.mode {
            TranscriptStoreMode::File { root_dir, .. } => {
                Some(root_dir.join(thread_storage_file_name(thread_id, "jsonl")))
            }
            TranscriptStoreMode::Memory { .. } => None,
        }
    }

    pub async fn exists(&self, thread_id: &str) -> bool {
        match &self.mode {
            TranscriptStoreMode::File { .. } => self
                .transcript_path(thread_id)
                .is_some_and(|path| path.exists()),
            TranscriptStoreMode::Memory { records } => records
                .lock()
                .await
                .get(thread_id)
                .is_some_and(|entries| !entries.is_empty()),
        }
    }

    // -----------------------------------------------------------------------
    // Per-thread slot + cache plumbing (File mode)
    // -----------------------------------------------------------------------

    fn file_slot(&self, thread_id: &str) -> Option<Arc<ThreadSlot>> {
        match &self.mode {
            TranscriptStoreMode::File { slots, .. } => {
                let mut slots = slots.lock().expect("transcript slot registry poisoned");
                Some(
                    slots
                        .entry(thread_id.to_owned())
                        .or_insert_with(|| Arc::new(ThreadSlot::default()))
                        .clone(),
                )
            }
            TranscriptStoreMode::Memory { .. } => None,
        }
    }

    fn cache_budget(&self) -> TranscriptCacheBudget {
        match &self.mode {
            TranscriptStoreMode::File { budget, .. } => *budget,
            TranscriptStoreMode::Memory { .. } => TranscriptCacheBudget::default(),
        }
    }

    #[cfg(test)]
    pub(super) async fn render_cache_checkpoint_debug(
        &self,
        thread_id: &str,
    ) -> Option<(u64, u64, usize)> {
        let slot = self.file_slot(thread_id)?;
        let cache = slot.state.lock().await;
        cache.as_ref().map(|entry| {
            (
                entry.base_seq,
                entry.tail_start_seq(),
                entry.render_prefix_checkpoint.estimated_heap_bytes(),
            )
        })
    }

    fn store_now_ms(&self) -> u64 {
        match &self.mode {
            TranscriptStoreMode::File { created_at, .. } => created_at.elapsed().as_millis() as u64,
            TranscriptStoreMode::Memory { .. } => 0,
        }
    }

    /// Mirror the slot's cache size/recency into its atomics (read by the
    /// eviction pass without taking the slot lock).
    fn sync_slot_accounting(&self, slot: &ThreadSlot, cache: &Option<ThreadCache>) {
        slot.cached_bytes.store(
            cache.as_ref().map(ThreadCache::cache_bytes).unwrap_or(0),
            Ordering::Relaxed,
        );
        slot.last_used_ms
            .store(self.store_now_ms(), Ordering::Relaxed);
    }

    /// Drop least-recently-used cache entries until the store-wide budget
    /// holds again. Never blocks: victims are `try_lock`ed and skipped when
    /// busy (busy means recently used anyway), and no other lock is held
    /// while this runs.
    fn evict_over_budget(&self, current_thread_id: &str) {
        let TranscriptStoreMode::File { slots, budget, .. } = &self.mode else {
            return;
        };
        let entries: Vec<(String, Arc<ThreadSlot>)> = {
            let slots = slots.lock().expect("transcript slot registry poisoned");
            slots
                .iter()
                .map(|(thread_id, slot)| (thread_id.clone(), slot.clone()))
                .collect()
        };
        let mut total: usize = entries
            .iter()
            .map(|(_, slot)| slot.cached_bytes.load(Ordering::Relaxed))
            .sum();
        if total <= budget.total_max_bytes {
            return;
        }
        let mut candidates: Vec<&(String, Arc<ThreadSlot>)> = entries
            .iter()
            .filter(|(thread_id, slot)| {
                thread_id != current_thread_id && slot.cached_bytes.load(Ordering::Relaxed) > 0
            })
            .collect();
        candidates.sort_by_key(|(_, slot)| slot.last_used_ms.load(Ordering::Relaxed));
        for (_, slot) in candidates {
            if total <= budget.total_max_bytes {
                break;
            }
            if let Ok(mut state) = slot.state.try_lock()
                && let Some(entry) = state.take()
            {
                total = total.saturating_sub(entry.cache_bytes());
                slot.cached_bytes.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Validate a cached entry against the file (one fstat); drop it when the
    /// on-disk length diverged (out-of-band writer, deleted file, …).
    async fn verify_cache(&self, cache: &mut Option<ThreadCache>, path: &Path) {
        let Some(entry) = cache.as_ref() else {
            return;
        };
        let disk_len = tokio::fs::metadata(path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        if disk_len != entry.file_len {
            *cache = None;
        }
    }

    /// Verify-or-build the thread's cache entry (one full read + fold when
    /// absent). `Err` means the transcript could not be read/parsed — the
    /// same error the uncached path would surface.
    async fn ensure_cache(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        path: &Path,
    ) -> Result<(), ThreadHistoryError> {
        self.verify_cache(cache, path).await;
        if cache.is_some() {
            return Ok(());
        }
        *cache = Some(self.build_cache_streaming(thread_id, path).await?);
        Ok(())
    }

    /// Stream the transcript jsonl once, invoking `visit` for every message
    /// record in file order. Bounded memory: each record is parsed and handed
    /// over without materializing the file; `visit` returns `Break` to stop
    /// the scan early.
    async fn for_each_transcript_record(
        &self,
        thread_id: &str,
        path: &Path,
        mut visit: impl FnMut(CachedTranscriptRecord) -> std::ops::ControlFlow<()>,
    ) -> Result<(), ThreadHistoryError> {
        if !path.exists() {
            return Ok(());
        }
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|error| transcript_io_error(thread_id, error))?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut line_no = 0usize;
        loop {
            line.clear();
            let read = reader
                .read_line(&mut line)
                .await
                .map_err(|error| transcript_io_error(thread_id, error))?;
            if read == 0 {
                break;
            }
            line_no += 1;
            let stripped = line.strip_suffix('\n').unwrap_or(&line);
            let stripped = stripped.strip_suffix('\r').unwrap_or(stripped);
            if stripped.trim().is_empty() {
                continue;
            }
            let parsed = serde_json::from_str::<TranscriptLine>(stripped).map_err(|error| {
                ThreadHistoryError::InvalidTranscript {
                    thread_id: thread_id.to_owned(),
                    message: format!("line {line_no}: {error}"),
                }
            })?;
            let TranscriptLine::Message {
                seq,
                thread_id,
                run_id,
                timestamp,
                message,
            } = parsed
            else {
                continue;
            };
            let cached = CachedTranscriptRecord {
                bytes: stripped.len(),
                record: ThreadTranscriptRecord {
                    seq,
                    thread_id,
                    run_id,
                    timestamp,
                    message,
                },
            };
            if visit(cached).is_break() {
                break;
            }
        }
        Ok(())
    }

    /// Build a thread's cache entry by streaming the transcript jsonl once,
    /// folding rolled-off records into the run-state checkpoint as the scan
    /// advances. Peak memory stays near the tail budget instead of
    /// materializing the whole file — large transcripts previously ballooned
    /// to gigabytes on their first post-restart touch.
    async fn build_cache_streaming(
        &self,
        thread_id: &str,
        path: &Path,
    ) -> Result<ThreadCache, ThreadHistoryError> {
        let file_len = tokio::fs::metadata(path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        let mut entry = ThreadCache::from_records(Vec::new(), file_len);
        if !path.exists() {
            return Ok(entry);
        }
        let budget = self.cache_budget();
        self.for_each_transcript_record(thread_id, path, |cached| {
            entry.push_streamed(cached, &budget);
            std::ops::ControlFlow::Continue(())
        })
        .await?;
        entry.roll_tail(&budget);
        Ok(entry)
    }

    /// Materialize only the messages whose seq falls in
    /// `[start_seq, end_seq]` (both inclusive), streaming the file once and
    /// stopping at the range's end. Memory is bounded by the page size —
    /// this is the paging fallback for ranges below the cached tail, which
    /// previously re-read whole multi-hundred-MB transcripts per page.
    async fn stream_messages_in_seq_range(
        &self,
        thread_id: &str,
        path: &Path,
        start_seq: u64,
        end_seq: u64,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        let mut messages = Vec::new();
        if start_seq > end_seq {
            return Ok(messages);
        }
        self.for_each_transcript_record(thread_id, path, |cached| {
            if cached.record.seq > end_seq {
                return std::ops::ControlFlow::Break(());
            }
            if cached.record.seq >= start_seq {
                messages.push(cached.record.message);
            }
            std::ops::ControlFlow::Continue(())
        })
        .await?;
        Ok(messages)
    }

    // -----------------------------------------------------------------------
    // Appends
    // -----------------------------------------------------------------------

    pub async fn append_committed_messages(
        &self,
        thread_id: &str,
        run_id: Option<&str>,
        messages: &[Value],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let mut cache = slot.state.lock().await;
                let result = self
                    .append_committed_messages_file(&mut cache, thread_id, run_id, messages)
                    .await;
                if result.is_err() {
                    *cache = None;
                }
                self.sync_slot_accounting(&slot, &cache);
                drop(cache);
                self.evict_over_budget(thread_id);
                result
            }
            TranscriptStoreMode::Memory { records } => {
                let trimmed_run_id = trim_non_empty(run_id);
                let mut guard = records.lock().await;
                let entries = guard.entry(thread_id.to_owned()).or_default();
                let next_seq = entries.last().map(|record| record.seq + 1).unwrap_or(1);
                for (seq, message) in (next_seq..).zip(messages.iter()) {
                    entries.push(ThreadTranscriptRecord {
                        seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
                        timestamp: message_timestamp(message)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: message.clone(),
                    });
                }
                Ok(TranscriptAppendResult {
                    total_messages: entries.len(),
                    last_message_at: entries.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                })
            }
        }
    }

    async fn append_committed_messages_file(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        run_id: Option<&str>,
        messages: &[Value],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        let path =
            self.transcript_path(thread_id)
                .ok_or_else(|| ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: "missing transcript path".to_owned(),
                })?;
        self.ensure_cache(cache, thread_id, &path).await?;
        let entry = cache.as_ref().expect("cache ensured above");
        let next_seq = entry.last_seq + 1;
        let trimmed_run_id = trim_non_empty(run_id);
        let mut appended = Vec::with_capacity(messages.len());
        for (seq, message) in (next_seq..).zip(messages.iter()) {
            appended.push(ThreadTranscriptRecord {
                seq,
                thread_id: thread_id.to_owned(),
                run_id: trimmed_run_id.clone(),
                timestamp: message_timestamp(message)
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                message: message.clone(),
            });
        }

        if appended.is_empty() && path.exists() {
            return Ok(TranscriptAppendResult {
                total_messages: entry.total_records,
                last_message_at: entry.last_message_at(),
                transcript_file: Some(path),
            });
        }

        let line_sizes = self
            .append_record_lines(thread_id, &path, &appended)
            .await?;
        let entry = cache.as_mut().expect("cache ensured above");
        let file_len = tokio::fs::metadata(&path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        entry.push_appended(&appended, &line_sizes, file_len, &self.cache_budget());
        Ok(TranscriptAppendResult {
            total_messages: entry.total_records,
            last_message_at: entry.last_message_at(),
            transcript_file: Some(path),
        })
    }

    pub async fn append_run_records(
        &self,
        thread_id: &str,
        run_id: Option<&str>,
        records: &[RunTranscriptRecordDraft],
    ) -> Result<TranscriptAppendRecordsResult, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let mut cache = slot.state.lock().await;
                let result = self
                    .append_run_records_file(&mut cache, thread_id, run_id, records)
                    .await;
                if result.is_err() {
                    *cache = None;
                }
                self.sync_slot_accounting(&slot, &cache);
                drop(cache);
                self.evict_over_budget(thread_id);
                result
            }
            TranscriptStoreMode::Memory { records: store } => {
                let trimmed_run_id = trim_non_empty(run_id);
                let mut guard = store.lock().await;
                let entries = guard.entry(thread_id.to_owned()).or_default();
                let next_seq = entries.last().map(|record| record.seq + 1).unwrap_or(1);
                let mut appended_records = Vec::with_capacity(records.len());
                for (seq, draft) in (next_seq..).zip(records.iter()) {
                    let record = ThreadTranscriptRecord {
                        seq,
                        thread_id: thread_id.to_owned(),
                        run_id: trimmed_run_id.clone(),
                        timestamp: draft
                            .timestamp
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                        message: draft.message.clone(),
                    };
                    entries.push(record.clone());
                    appended_records.push(record);
                }
                Ok(TranscriptAppendRecordsResult {
                    total_messages: entries.len(),
                    last_message_at: entries.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                    appended_records,
                })
            }
        }
    }

    async fn append_run_records_file(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        run_id: Option<&str>,
        records: &[RunTranscriptRecordDraft],
    ) -> Result<TranscriptAppendRecordsResult, ThreadHistoryError> {
        let path =
            self.transcript_path(thread_id)
                .ok_or_else(|| ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: "missing transcript path".to_owned(),
                })?;
        self.ensure_cache(cache, thread_id, &path).await?;
        let entry = cache.as_ref().expect("cache ensured above");
        let next_seq = entry.last_seq + 1;
        let trimmed_run_id = trim_non_empty(run_id);
        let mut appended_records = Vec::with_capacity(records.len());
        for (seq, draft) in (next_seq..).zip(records.iter()) {
            appended_records.push(ThreadTranscriptRecord {
                seq,
                thread_id: thread_id.to_owned(),
                run_id: trimmed_run_id.clone(),
                timestamp: draft
                    .timestamp
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                message: draft.message.clone(),
            });
        }

        if appended_records.is_empty() && path.exists() {
            return Ok(TranscriptAppendRecordsResult {
                total_messages: entry.total_records,
                last_message_at: entry.last_message_at(),
                transcript_file: Some(path),
                appended_records,
            });
        }

        let line_sizes = self
            .append_record_lines(thread_id, &path, &appended_records)
            .await?;
        let entry = cache.as_mut().expect("cache ensured above");
        let file_len = tokio::fs::metadata(&path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(0);
        entry.push_appended(
            &appended_records,
            &line_sizes,
            file_len,
            &self.cache_budget(),
        );
        Ok(TranscriptAppendRecordsResult {
            total_messages: entry.total_records,
            last_message_at: entry.last_message_at(),
            transcript_file: Some(path),
            appended_records,
        })
    }

    /// Shared file-append tail: open in append mode, write the session header
    /// when the file is new/empty, then write one jsonl line per record.
    /// Returns each written line's length (the cache's byte estimate).
    async fn append_record_lines(
        &self,
        thread_id: &str,
        path: &Path,
        appended: &[ThreadTranscriptRecord],
    ) -> Result<Vec<usize>, ThreadHistoryError> {
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|error| ThreadHistoryError::TranscriptIo {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            })?;

        if !path.exists()
            || tokio::fs::metadata(path)
                .await
                .map(|meta| meta.len())
                .unwrap_or(0)
                == 0
        {
            let header = serde_json::to_string(&TranscriptLine::Session {
                version: 1,
                thread_id: thread_id.to_owned(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
            .map_err(|error| ThreadHistoryError::InvalidTranscript {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            })?;
            file.write_all(header.as_bytes()).await.map_err(|error| {
                ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: error.to_string(),
                }
            })?;
            file.write_all(b"\n")
                .await
                .map_err(|error| ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: error.to_string(),
                })?;
        }

        let mut line_sizes = Vec::with_capacity(appended.len());
        for record in appended {
            let line =
                serde_json::to_string(&TranscriptLine::from(record.clone())).map_err(|error| {
                    ThreadHistoryError::InvalidTranscript {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    }
                })?;
            file.write_all(line.as_bytes()).await.map_err(|error| {
                ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: error.to_string(),
                }
            })?;
            file.write_all(b"\n")
                .await
                .map_err(|error| ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: error.to_string(),
                })?;
            line_sizes.push(line.len());
        }
        file.flush()
            .await
            .map_err(|error| ThreadHistoryError::TranscriptIo {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            })?;
        Ok(line_sizes)
    }

    // -----------------------------------------------------------------------
    // Rewrites
    // -----------------------------------------------------------------------

    /// Replace the complete transcript with a header plus `records` using a
    /// same-directory temp file, file fsync, atomic rename, and parent fsync.
    /// The per-thread cache is invalidated on every error, including the
    /// parent-fsync case where the complete new target is already visible.
    pub async fn replace_transcript_atomic(
        &self,
        thread_id: &str,
        records: &[ThreadTranscriptRecord],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let mut cache = slot.state.lock().await;
                let result = self
                    .replace_transcript_atomic_file(&mut cache, thread_id, records)
                    .await;
                if result.is_err() {
                    *cache = None;
                }
                self.sync_slot_accounting(&slot, &cache);
                drop(cache);
                self.evict_over_budget(thread_id);
                result
            }
            TranscriptStoreMode::Memory { records: stored } => {
                stored
                    .lock()
                    .await
                    .insert(thread_id.to_owned(), records.to_vec());
                Ok(TranscriptAppendResult {
                    total_messages: records.len(),
                    last_message_at: records.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                })
            }
        }
    }

    /// Import-only transcript gate. Structurally invalid files fail closed;
    /// valid identity prefixes are completed while diverged transcripts stay
    /// authoritative.
    pub async fn ensure_transcript_backfilled(
        &self,
        thread_id: &str,
        legacy_messages: &[Value],
    ) -> Result<BackfillOutcome, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let mut cache = slot.state.lock().await;
                let result = self
                    .ensure_transcript_backfilled_file(&mut cache, thread_id, legacy_messages)
                    .await;
                if result.is_err() {
                    *cache = None;
                }
                self.sync_slot_accounting(&slot, &cache);
                drop(cache);
                self.evict_over_budget(thread_id);
                result
            }
            TranscriptStoreMode::Memory { records } => {
                let mut guard = records.lock().await;
                let Some(existing) = guard.get_mut(thread_id) else {
                    guard.insert(
                        thread_id.to_owned(),
                        reconcile_rewrite_records(thread_id, &[], legacy_messages),
                    );
                    return Ok(BackfillOutcome::Backfilled);
                };
                if existing.is_empty() {
                    *existing = reconcile_rewrite_records(thread_id, &[], legacy_messages);
                    return Ok(BackfillOutcome::Backfilled);
                }
                let existing_identity: Vec<Value> = existing
                    .iter()
                    .map(|record| message_identity(&record.message))
                    .collect();
                let legacy_identity: Vec<Value> =
                    legacy_messages.iter().map(message_identity).collect();
                if existing_identity == legacy_identity {
                    return Ok(BackfillOutcome::AlreadyComplete);
                }
                if existing_identity.len() < legacy_identity.len()
                    && legacy_identity[..existing_identity.len()] == existing_identity
                {
                    *existing = reconcile_rewrite_records(thread_id, existing, legacy_messages);
                    return Ok(BackfillOutcome::Backfilled);
                }
                Ok(BackfillOutcome::PreservedDiverged)
            }
        }
    }

    pub async fn rewrite_from_messages(
        &self,
        thread_id: &str,
        messages: &[Value],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let mut cache = slot.state.lock().await;
                let result = self
                    .rewrite_from_messages_file(&mut cache, thread_id, messages)
                    .await;
                if result.is_err() {
                    *cache = None;
                }
                self.sync_slot_accounting(&slot, &cache);
                drop(cache);
                self.evict_over_budget(thread_id);
                result
            }
            TranscriptStoreMode::Memory { records } => {
                let mut guard = records.lock().await;
                let entries = guard.entry(thread_id.to_owned()).or_default();
                *entries = reconcile_rewrite_records(thread_id, entries, messages);
                Ok(TranscriptAppendResult {
                    total_messages: entries.len(),
                    last_message_at: entries.last().map(|record| record.timestamp.clone()),
                    transcript_file: None,
                })
            }
        }
    }

    async fn rewrite_from_messages_file(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        messages: &[Value],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        let path =
            self.transcript_path(thread_id)
                .ok_or_else(|| ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: "missing transcript path".to_owned(),
                })?;
        let existing = self.read_records_from_path(thread_id, &path).await?;
        let records = reconcile_rewrite_records(thread_id, &existing, messages);
        if records == existing {
            let file_len = tokio::fs::metadata(&path)
                .await
                .map(|meta| meta.len())
                .unwrap_or(0);
            self.rebuild_cache_from_records(cache, &existing, file_len);
            return Ok(TranscriptAppendResult {
                total_messages: existing.len(),
                last_message_at: existing.last().map(|record| record.timestamp.clone()),
                transcript_file: Some(path),
            });
        }

        self.replace_transcript_atomic_file(cache, thread_id, &records)
            .await
    }

    /// Overwrite the whole transcript with `records`, preserving each record's
    /// `seq`/`run_id`/`timestamp`. Internal helper for tail reconciliation;
    /// callers already hold the thread's slot lock.
    async fn write_records_file(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        records: &[ThreadTranscriptRecord],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        self.replace_transcript_atomic_file(cache, thread_id, records)
            .await
    }

    async fn replace_transcript_atomic_file(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        records: &[ThreadTranscriptRecord],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        let (path, atomic_fs) = match &self.mode {
            TranscriptStoreMode::File {
                root_dir,
                atomic_fs,
                ..
            } => (
                root_dir.join(thread_storage_file_name(thread_id, "jsonl")),
                Arc::clone(atomic_fs),
            ),
            TranscriptStoreMode::Memory { .. } => {
                return Err(ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: "atomic file replace requested for memory store".to_owned(),
                });
            }
        };
        let parent = path
            .parent()
            .ok_or_else(|| ThreadHistoryError::TranscriptIo {
                thread_id: thread_id.to_owned(),
                message: "missing transcript parent directory".to_owned(),
            })?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("transcript.jsonl");
        let temp = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
        let payload = serialize_transcript(thread_id, records)?;

        let temp_file = atomic_fs
            .write_temp(&temp, &payload)
            .await
            .map_err(|error| {
                atomic_replace_error(thread_id, TranscriptReplaceStage::TempWrite, error)
            })?;
        atomic_fs.fsync_file(&temp_file).await.map_err(|error| {
            atomic_replace_error(thread_id, TranscriptReplaceStage::FileFsync, error)
        })?;
        drop(temp_file);
        atomic_fs.rename(&temp, &path).await.map_err(|error| {
            atomic_replace_error(thread_id, TranscriptReplaceStage::Rename, error)
        })?;
        atomic_fs.fsync_parent(parent).await.map_err(|error| {
            atomic_replace_error(thread_id, TranscriptReplaceStage::ParentFsync, error)
        })?;

        self.rebuild_cache_from_records(cache, records, payload.len() as u64);
        Ok(TranscriptAppendResult {
            total_messages: records.len(),
            last_message_at: records.last().map(|record| record.timestamp.clone()),
            transcript_file: Some(path),
        })
    }

    async fn ensure_transcript_backfilled_file(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        legacy_messages: &[Value],
    ) -> Result<BackfillOutcome, ThreadHistoryError> {
        let path =
            self.transcript_path(thread_id)
                .ok_or_else(|| ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: "missing transcript path".to_owned(),
                })?;
        let state = self
            .read_validated_import_transcript(thread_id, &path)
            .await?;
        let ValidatedImportTranscript::Parsed {
            records: existing,
            torn_tail,
            file_len,
        } = state
        else {
            let records = reconcile_rewrite_records(thread_id, &[], legacy_messages);
            self.replace_transcript_atomic_file(cache, thread_id, &records)
                .await?;
            return Ok(BackfillOutcome::Backfilled);
        };

        let existing_identity: Vec<Value> = existing
            .iter()
            .map(|record| message_identity(&record.message))
            .collect();
        let legacy_identity: Vec<Value> = legacy_messages.iter().map(message_identity).collect();
        let existing_is_prefix = existing_identity.len() <= legacy_identity.len()
            && legacy_identity[..existing_identity.len()] == existing_identity;

        if torn_tail && !existing_is_prefix {
            return Err(invalid_transcript(
                thread_id,
                "torn tail follows records that are not an identity prefix of the legacy archive",
            ));
        }
        if !torn_tail && existing_identity == legacy_identity {
            self.rebuild_cache_from_records(cache, &existing, file_len);
            return Ok(BackfillOutcome::AlreadyComplete);
        }
        if existing_is_prefix {
            let completed = if existing_identity == legacy_identity {
                existing
            } else {
                reconcile_rewrite_records(thread_id, &existing, legacy_messages)
            };
            self.replace_transcript_atomic_file(cache, thread_id, &completed)
                .await?;
            return Ok(BackfillOutcome::Backfilled);
        }

        self.rebuild_cache_from_records(cache, &existing, file_len);
        Ok(BackfillOutcome::PreservedDiverged)
    }

    async fn read_validated_import_transcript(
        &self,
        thread_id: &str,
        path: &Path,
    ) -> Result<ValidatedImportTranscript, ThreadHistoryError> {
        let raw = match tokio::fs::read(path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ValidatedImportTranscript::MissingOrEmpty);
            }
            Err(error) => return Err(transcript_io_error(thread_id, error)),
        };
        #[cfg(test)]
        self.full_file_reads.fetch_add(1, Ordering::Relaxed);
        if raw.is_empty() {
            return Ok(ValidatedImportTranscript::MissingOrEmpty);
        }

        parse_import_transcript_bytes(thread_id, &raw)
    }

    /// Rebuild a thread's cache entry from a full record set already in
    /// memory (rewrite paths) — no extra disk read.
    fn rebuild_cache_from_records(
        &self,
        cache: &mut Option<ThreadCache>,
        records: &[ThreadTranscriptRecord],
        file_len: u64,
    ) {
        let sized: Vec<CachedTranscriptRecord> = records
            .iter()
            .map(|record| CachedTranscriptRecord {
                bytes: serde_json::to_string(&TranscriptLine::from(record.clone()))
                    .map(|line| line.len())
                    .unwrap_or(0),
                record: record.clone(),
            })
            .collect();
        let mut entry = ThreadCache::from_records(sized, file_len);
        entry.roll_tail(&self.cache_budget());
        *cache = Some(entry);
    }

    // -----------------------------------------------------------------------
    // Reconciles
    // -----------------------------------------------------------------------

    pub async fn reconcile_run_records_tail(
        &self,
        thread_id: &str,
        run_id: &str,
        authoritative: &[RunTranscriptRecordDraft],
    ) -> Result<TranscriptAppendRecordsResult, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        if trimmed_run_id.is_empty() {
            if !authoritative.is_empty() {
                tracing::warn!(
                    thread_id = %thread_id,
                    "reconcile_run_records_tail called without a run_id; skipping tail reconcile"
                );
            }
            let records = self.read_records(thread_id).await?;
            return Ok(TranscriptAppendRecordsResult {
                total_messages: records.len(),
                last_message_at: records.last().map(|record| record.timestamp.clone()),
                transcript_file: self.transcript_path(thread_id),
                appended_records: Vec::new(),
            });
        }

        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let mut cache = slot.state.lock().await;
                let result = self
                    .reconcile_run_records_tail_file(
                        &mut cache,
                        thread_id,
                        trimmed_run_id,
                        authoritative,
                    )
                    .await;
                if result.is_err() {
                    *cache = None;
                }
                self.sync_slot_accounting(&slot, &cache);
                drop(cache);
                self.evict_over_budget(thread_id);
                result
            }
            TranscriptStoreMode::Memory { .. } => {
                let records = self.read_records(thread_id).await?;
                match plan_reconcile_run_records_tail(
                    thread_id,
                    &records,
                    trimmed_run_id,
                    authoritative,
                ) {
                    RecordsReconcilePlan::NoOp => Ok(TranscriptAppendRecordsResult {
                        total_messages: records.len(),
                        last_message_at: records.last().map(|record| record.timestamp.clone()),
                        transcript_file: self.transcript_path(thread_id),
                        appended_records: Vec::new(),
                    }),
                    RecordsReconcilePlan::AppendSuffix { skip } => {
                        self.append_run_records(
                            thread_id,
                            Some(trimmed_run_id),
                            &authoritative[skip..],
                        )
                        .await
                    }
                    RecordsReconcilePlan::Rewrite { rebuilt, changed } => {
                        let summary = self.write_records_memory(thread_id, &rebuilt).await?;
                        Ok(TranscriptAppendRecordsResult {
                            total_messages: summary.total_messages,
                            last_message_at: summary.last_message_at,
                            transcript_file: summary.transcript_file,
                            appended_records: changed,
                        })
                    }
                }
            }
        }
    }

    async fn reconcile_run_records_tail_file(
        &self,
        cache: &mut Option<ThreadCache>,
        thread_id: &str,
        trimmed_run_id: &str,
        authoritative: &[RunTranscriptRecordDraft],
    ) -> Result<TranscriptAppendRecordsResult, ThreadHistoryError> {
        let path =
            self.transcript_path(thread_id)
                .ok_or_else(|| ThreadHistoryError::TranscriptIo {
                    thread_id: thread_id.to_owned(),
                    message: "missing transcript path".to_owned(),
                })?;
        self.verify_cache(cache, &path).await;

        // Fast path off the cached tail: no-op and pure-suffix-append are
        // decidable from the trailing run block; everything else needs the
        // full record set for the rewrite.
        if let Some(entry) = cache.as_ref()
            && let Some(block) = entry.trailing_run_block(trimmed_run_id)
        {
            if block.is_empty() {
                return self
                    .append_run_records_file(cache, thread_id, Some(trimmed_run_id), authoritative)
                    .await;
            }
            let existing_identity: Vec<Value> = block
                .iter()
                .map(|c| message_identity(&c.record.message))
                .collect();
            let authoritative_identity: Vec<Value> = authoritative
                .iter()
                .map(|draft| message_identity(&draft.message))
                .collect();
            if existing_identity == authoritative_identity {
                let unchanged = authoritative.iter().enumerate().all(|(offset, draft)| {
                    let existing = &block[offset].record;
                    let replacement = record_from_draft_replacing(
                        thread_id,
                        Some(trimmed_run_id),
                        existing.seq,
                        draft,
                        existing,
                    );
                    replacement.timestamp == existing.timestamp
                        && replacement.message == existing.message
                });
                if unchanged {
                    return Ok(TranscriptAppendRecordsResult {
                        total_messages: entry.total_records,
                        last_message_at: entry.last_message_at(),
                        transcript_file: Some(path),
                        appended_records: Vec::new(),
                    });
                }
            } else if authoritative_identity.len() > existing_identity.len()
                && authoritative_identity[..existing_identity.len()] == existing_identity[..]
            {
                let prefix_changed =
                    authoritative
                        .iter()
                        .zip(block.iter())
                        .any(|(draft, existing)| {
                            let replacement = record_from_draft_replacing(
                                thread_id,
                                Some(trimmed_run_id),
                                existing.record.seq,
                                draft,
                                &existing.record,
                            );
                            replacement.timestamp != existing.record.timestamp
                                || replacement.message != existing.record.message
                        });
                if !prefix_changed {
                    let skip = block.len();
                    return self
                        .append_run_records_file(
                            cache,
                            thread_id,
                            Some(trimmed_run_id),
                            &authoritative[skip..],
                        )
                        .await;
                }
            }
        }

        let records = self.read_records_from_path(thread_id, &path).await?;
        match plan_reconcile_run_records_tail(thread_id, &records, trimmed_run_id, authoritative) {
            RecordsReconcilePlan::NoOp => {
                let file_len = tokio::fs::metadata(&path)
                    .await
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                self.rebuild_cache_from_records(cache, &records, file_len);
                Ok(TranscriptAppendRecordsResult {
                    total_messages: records.len(),
                    last_message_at: records.last().map(|record| record.timestamp.clone()),
                    transcript_file: Some(path),
                    appended_records: Vec::new(),
                })
            }
            RecordsReconcilePlan::AppendSuffix { skip } => {
                let file_len = tokio::fs::metadata(&path)
                    .await
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                self.rebuild_cache_from_records(cache, &records, file_len);
                self.append_run_records_file(
                    cache,
                    thread_id,
                    Some(trimmed_run_id),
                    &authoritative[skip..],
                )
                .await
            }
            RecordsReconcilePlan::Rewrite { rebuilt, changed } => {
                let summary = self.write_records_file(cache, thread_id, &rebuilt).await?;
                Ok(TranscriptAppendRecordsResult {
                    total_messages: summary.total_messages,
                    last_message_at: summary.last_message_at,
                    transcript_file: summary.transcript_file,
                    appended_records: changed,
                })
            }
        }
    }

    async fn write_records_memory(
        &self,
        thread_id: &str,
        records: &[ThreadTranscriptRecord],
    ) -> Result<TranscriptAppendResult, ThreadHistoryError> {
        let TranscriptStoreMode::Memory { records: store } = &self.mode else {
            return Err(ThreadHistoryError::TranscriptIo {
                thread_id: thread_id.to_owned(),
                message: "memory write on file store".to_owned(),
            });
        };
        let mut guard = store.lock().await;
        guard.insert(thread_id.to_owned(), records.to_vec());
        Ok(TranscriptAppendResult {
            total_messages: records.len(),
            last_message_at: records.last().map(|record| record.timestamp.clone()),
            transcript_file: None,
        })
    }

    // -----------------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------------

    /// Serve a read from the thread's cache, streaming the cache build first
    /// when the entry is missing (the post-restart cold case). The build is a
    /// single bounded-memory pass — roll-off folds into the checkpoint as the
    /// scan advances — so it is strictly cheaper than the full-file fallback
    /// read the caller would otherwise do; entries the store-wide budget
    /// cannot keep are evicted right after serving. Returns `None` when the
    /// entry cannot cover the request or the build fails (callers fall back
    /// to explicit reads, which surface the underlying error).
    async fn with_built_cache<T>(
        &self,
        thread_id: &str,
        serve: impl FnOnce(&ThreadCache) -> Option<T>,
    ) -> Option<T> {
        let slot = self.file_slot(thread_id)?;
        let mut cache = slot.state.lock().await;
        let path = self.transcript_path(thread_id)?;
        self.verify_cache(&mut cache, &path).await;
        if cache.is_none() {
            match self.build_cache_streaming(thread_id, &path).await {
                Ok(entry) => *cache = Some(entry),
                Err(_) => {
                    // verify_cache may have just dropped a stale entry;
                    // sync the slot's accounting before bailing so the
                    // store-wide eviction budget stops counting bytes for
                    // a cache that no longer exists.
                    self.sync_slot_accounting(&slot, &cache);
                    return None;
                }
            }
        }
        let served = cache.as_ref().and_then(serve);
        self.sync_slot_accounting(&slot, &cache);
        drop(cache);
        self.evict_over_budget(thread_id);
        served
    }

    pub async fn tail(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        if let Some(messages) = self
            .with_built_cache(thread_id, |entry| entry.tail_messages(limit))
            .await
        {
            return Ok(messages);
        }
        let records = self.read_records(thread_id).await?;
        let start = records.len().saturating_sub(limit);
        Ok(records[start..]
            .iter()
            .map(|record| record.message.clone())
            .collect())
    }

    /// Tail of the thread's provider-session content: the newest `limit`
    /// content records' messages in transcript order, with run control
    /// records skipped. This is the transcript-backed replacement for the
    /// legacy thread-record `messages` snapshot, which only ever contained
    /// run content rows (#TASK-1864 batch 1).
    pub async fn provider_session_tail(
        &self,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        if let Some(messages) = self
            .with_built_cache(thread_id, |entry| {
                entry.provider_session_tail_messages(limit)
            })
            .await
        {
            return Ok(messages);
        }
        let records = self.read_records(thread_id).await?;
        Ok(provider_session_tail_from_messages(
            records.iter().map(|record| &record.message),
            limit,
        ))
    }

    pub async fn page_before_index(
        &self,
        thread_id: &str,
        before_index: Option<usize>,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        if matches!(&self.mode, TranscriptStoreMode::Memory { .. }) {
            let records = self.read_records(thread_id).await?;
            let total = records.len();
            let end = before_index.unwrap_or(total).min(total);
            let start = end.saturating_sub(limit);
            let messages = records[start..end]
                .iter()
                .map(|record| record.message.clone())
                .collect();
            return Ok((messages, total, start));
        }
        let total = self.message_count(thread_id).await?;
        let end = before_index.unwrap_or(total).min(total);
        let start = end.saturating_sub(limit);
        self.page_messages_by_index(thread_id, start, end, total)
            .await
    }

    /// Forward page: committed records with position strictly greater than
    /// `after_index`, up to `limit`. Mirror of `page_before_index` for cursor
    /// (delta) sync — "give me what's new since index N".
    pub async fn page_after_index(
        &self,
        thread_id: &str,
        after_index: usize,
        limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        if matches!(&self.mode, TranscriptStoreMode::Memory { .. }) {
            let records = self.read_records(thread_id).await?;
            let total = records.len();
            let start = after_index.saturating_add(1).min(total);
            let end = start.saturating_add(limit).min(total);
            let messages = records[start..end]
                .iter()
                .map(|record| record.message.clone())
                .collect();
            return Ok((messages, total, start));
        }
        let total = self.message_count(thread_id).await?;
        let start = after_index.saturating_add(1).min(total);
        let end = start.saturating_add(limit).min(total);
        self.page_messages_by_index(thread_id, start, end, total)
            .await
    }

    pub async fn page_before_user_queries(
        &self,
        thread_id: &str,
        before_index: Option<usize>,
        user_query_limit: usize,
        fallback_message_limit: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        if matches!(&self.mode, TranscriptStoreMode::Memory { .. }) {
            let records = self.read_records(thread_id).await?;
            let total = records.len();
            let end = before_index.unwrap_or(total).min(total);
            let start =
                user_query_window_start(end, user_query_limit, fallback_message_limit, |index| {
                    is_user_query_message(&records[index].message)
                });
            let messages = records[start..end]
                .iter()
                .map(|record| record.message.clone())
                .collect();
            return Ok((messages, total, start));
        }
        let total = self.message_count(thread_id).await?;
        let end = before_index.unwrap_or(total).min(total);
        let target_user_queries = user_query_limit.max(1);

        // Streaming pass 1: collect the indexes of the last
        // `target_user_queries` user queries below `end` (bounded memory),
        // mirroring the backward scan of the in-memory variant.
        let Some(path) = self.transcript_path(thread_id) else {
            return Ok((Vec::new(), total, 0));
        };
        let mut recent_queries: std::collections::VecDeque<usize> =
            std::collections::VecDeque::with_capacity(target_user_queries);
        let mut index = 0usize;
        self.for_each_transcript_record(thread_id, &path, |cached| {
            if index >= end {
                return std::ops::ControlFlow::Break(());
            }
            if is_user_query_message(&cached.record.message) {
                recent_queries.push_back(index);
                if recent_queries.len() > target_user_queries {
                    recent_queries.pop_front();
                }
            }
            index += 1;
            std::ops::ControlFlow::Continue(())
        })
        .await?;
        let start = if recent_queries.is_empty() {
            end.saturating_sub(fallback_message_limit.max(1))
        } else if recent_queries.len() < target_user_queries {
            // The backward scan of the old code walked to the file head when
            // it ran out of user queries before reaching the target.
            0
        } else {
            recent_queries.front().copied().unwrap_or(0)
        };
        self.page_messages_by_index(thread_id, start, end, total)
            .await
    }

    /// Page `[start, end)` by record index against a File-mode transcript.
    /// Seqs are gapless from 1 (see the `ThreadCache` invariants), so index
    /// `i` holds seq `i + 1`: serve from the cached tail when it covers the
    /// range, else stream just the range from disk.
    async fn page_messages_by_index(
        &self,
        thread_id: &str,
        start: usize,
        end: usize,
        total: usize,
    ) -> Result<(Vec<Value>, usize, usize), ThreadHistoryError> {
        if start >= end {
            return Ok((Vec::new(), total, start));
        }
        let start_seq = start as u64 + 1;
        let end_seq = end as u64;
        if let Some(messages) = self
            .with_built_cache(thread_id, |entry| {
                entry.messages_in_seq_range(start_seq, end_seq)
            })
            .await
        {
            return Ok((messages, total, start));
        }
        let Some(path) = self.transcript_path(thread_id) else {
            return Ok((Vec::new(), total, start));
        };
        let messages = self
            .stream_messages_in_seq_range(thread_id, &path, start_seq, end_seq)
            .await?;
        Ok((messages, total, start))
    }

    pub async fn cold_open_user_turn_window(
        &self,
        thread_id: &str,
        user_turns: usize,
        cap: usize,
    ) -> Result<ThreadTranscriptWindow, ThreadHistoryError> {
        if let Some(window) = self
            .with_built_cache(thread_id, |entry| entry.cold_open_window(user_turns, cap))
            .await
        {
            return Ok(window);
        }
        let records = self.read_records(thread_id).await?;
        let total = records.len();
        if total == 0 {
            return Ok(ThreadTranscriptWindow {
                records: Vec::new(),
                floor_seq: 0,
                has_more_above: false,
            });
        }

        let target_user_turns = user_turns.max(1);
        let mut start = total;
        let mut user_queries = 0usize;
        while start > 0 && user_queries < target_user_turns {
            start -= 1;
            if is_user_query_message(&records[start].message) {
                user_queries += 1;
            }
        }

        if user_queries == 0 {
            start = total.saturating_sub(cap.max(1));
        }
        if total.saturating_sub(start) > cap {
            start = total.saturating_sub(cap);
        }

        let window_records = records[start..total].to_vec();
        let floor_seq = window_records.first().map(|record| record.seq).unwrap_or(0);
        Ok(ThreadTranscriptWindow {
            records: window_records,
            floor_seq,
            has_more_above: start > 0,
        })
    }

    pub async fn message_count(&self, thread_id: &str) -> Result<usize, ThreadHistoryError> {
        if let Some(total) = self
            .with_built_cache(thread_id, |entry| Some(entry.total_records))
            .await
        {
            return Ok(total);
        }
        Ok(self.read_records(thread_id).await?.len())
    }

    pub async fn records(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        self.read_records(thread_id).await
    }

    pub async fn run_state(
        &self,
        thread_id: &str,
    ) -> Result<TranscriptRunState, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let path = self.transcript_path(thread_id).expect("file mode has path");
                let mut cache = slot.state.lock().await;
                let result = self.ensure_cache(&mut cache, thread_id, &path).await;
                if result.is_err() {
                    *cache = None;
                }
                self.sync_slot_accounting(&slot, &cache);
                result?;
                let entry = cache.as_ref().expect("cache ensured above");
                let state = entry.run_state_at(entry.last_seq);
                drop(cache);
                self.evict_over_budget(thread_id);
                Ok(state)
            }
            TranscriptStoreMode::Memory { .. } => {
                let records = self.read_records(thread_id).await?;
                let values = records
                    .iter()
                    .filter_map(|record| serde_json::to_value(record).ok())
                    .collect::<Vec<_>>();
                Ok(reduce_transcript_run_state(&values))
            }
        }
    }

    pub async fn render_snapshot_at_seq(
        &self,
        thread_id: &str,
        based_on_seq: u64,
    ) -> Result<RenderSnapshot, ThreadHistoryError> {
        let records = self.read_records(thread_id).await?;
        let values = records
            .iter()
            .filter(|record| record.seq <= based_on_seq)
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        Ok(reduce_transcript_render_state(&values))
    }

    pub async fn render_snapshot_in_window(
        &self,
        thread_id: &str,
        floor_seq: u64,
        based_on_seq: u64,
    ) -> Result<RenderSnapshot, ThreadHistoryError> {
        if let TranscriptStoreMode::File { .. } = &self.mode {
            let slot = self.file_slot(thread_id).expect("file mode has slots");
            let path = self.transcript_path(thread_id).expect("file mode has path");
            let mut cache = slot.state.lock().await;
            let ensured = self.ensure_cache(&mut cache, thread_id, &path).await;
            if ensured.is_err() {
                *cache = None;
            }
            self.sync_slot_accounting(&slot, &cache);
            ensured?;
            let served = cache
                .as_ref()
                .and_then(|entry| entry.render_snapshot_in_window(floor_seq, based_on_seq));
            drop(cache);
            self.evict_over_budget(thread_id);
            if let Some(snapshot) = served {
                return Ok(snapshot);
            }
            tracing::debug!(
                thread_id = %thread_id,
                floor_seq,
                based_on_seq,
                "render window below cached tail; deriving from full transcript read"
            );
        }
        let records = self.read_records(thread_id).await?;
        let prefix = records
            .iter()
            .filter(|record| record.seq <= based_on_seq)
            .collect::<Vec<_>>();
        let actual_based_on_seq = prefix.iter().map(|record| record.seq).max().unwrap_or(0);
        let full_values = prefix
            .iter()
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        let run_state = reduce_transcript_run_state(&full_values);
        let render_prefix_values = prefix
            .iter()
            .filter(|record| record.seq < floor_seq)
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        let render_prefix_state = reduce_transcript_render_prefix_state(&render_prefix_values);
        let window_values = prefix
            .iter()
            .filter(|record| record.seq >= floor_seq)
            .filter_map(|record| serde_json::to_value(record).ok())
            .collect::<Vec<_>>();
        let mut snapshot = reduce_transcript_render_state_with_prefix_state(
            &window_values,
            &run_state,
            &render_prefix_state,
        );
        if snapshot.based_on_seq == 0 {
            snapshot.based_on_seq = actual_based_on_seq;
        }
        snapshot.window = Some(RenderWindow {
            floor_seq,
            has_more_above: prefix.iter().any(|record| record.seq < floor_seq),
        });
        Ok(snapshot)
    }

    /// Committed records with `seq > after_seq`, ascending, up to `limit`. Drives
    /// the resumable per-thread stream's replay (catch-up). Optimized for the
    /// caught-up case: it scans the jsonl from the TAIL backward and stops at the
    /// first `seq <= after_seq`, so a near-current cursor parses only the delta
    /// instead of the whole file (seq is monotonic + gapless, so everything before
    /// the boundary is older). A far-behind cursor whose delta exceeds `limit`
    /// yields the NEWEST `limit` (the tail), so the stream's live handoff stays
    /// gapless — the most recent rows are always delivered and the client pages
    /// older history via before_index.
    pub async fn records_after_seq(
        &self,
        thread_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                if !path.exists() {
                    return Ok(Vec::new());
                }
                read_tail_records_after_seq_from_path(thread_id, &path, after_seq, limit).await
            }
            TranscriptStoreMode::Memory { records } => {
                let guard = records.lock().await;
                let mut filtered: Vec<ThreadTranscriptRecord> = guard
                    .get(thread_id)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|record| record.seq > after_seq)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                // Newest `limit` (tail), matching the File mode so an over-limit
                // delta keeps the stream's live handoff gapless.
                if filtered.len() > limit {
                    filtered.drain(0..filtered.len() - limit);
                }
                Ok(filtered)
            }
        }
    }

    /// Oldest committed records with `seq > after_seq`, ascending, up to `limit`.
    /// This is the explicit pagination companion to `records_after_seq`: callers
    /// use the tail scan for the caught-up fast path, then fall back to this
    /// forward page only when the tail page proves the delta exceeded the replay
    /// cap.
    pub async fn records_after_seq_page(
        &self,
        thread_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                if !path.exists() {
                    return Ok(Vec::new());
                }
                read_forward_records_after_seq_from_path(thread_id, &path, after_seq, limit).await
            }
            TranscriptStoreMode::Memory { records } => {
                let guard = records.lock().await;
                Ok(guard
                    .get(thread_id)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|record| record.seq > after_seq)
                            .take(limit)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default())
            }
        }
    }

    pub async fn records_for_run_after_seq(
        &self,
        thread_id: &str,
        run_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        if trimmed_run_id.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                read_run_records_after_seq_from_path(
                    thread_id,
                    &path,
                    trimmed_run_id,
                    after_seq,
                    limit,
                )
                .await
            }
            TranscriptStoreMode::Memory { records } => {
                let guard = records.lock().await;
                Ok(guard
                    .get(thread_id)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter(|record| {
                                record.seq > after_seq
                                    && (record.run_id.as_deref() == Some(trimmed_run_id)
                                        || (record.run_id.is_none()
                                            && is_control_record_message(&record.message)))
                            })
                            .take(limit)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default())
            }
        }
    }

    pub async fn find_latest_for_run(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<Value>, ThreadHistoryError> {
        let trimmed_run_id = run_id.trim();
        if trimmed_run_id.is_empty() {
            return Ok(Vec::new());
        }
        let records = self.read_records(thread_id).await?;
        let mut matches = Vec::new();
        let mut collecting = false;
        for record in records.iter().rev() {
            match record.run_id.as_deref() {
                Some(candidate) if candidate == trimmed_run_id => {
                    collecting = true;
                    matches.push(record.message.clone());
                }
                _ if collecting => break,
                _ => {}
            }
        }
        matches.reverse();
        Ok(matches)
    }

    pub async fn find_latest_text_for_role(
        &self,
        thread_id: &str,
        role: &str,
    ) -> Result<Option<String>, ThreadHistoryError> {
        let trimmed_role = role.trim();
        if trimmed_role.is_empty() {
            return Ok(None);
        }
        let records = self.read_records(thread_id).await?;
        for record in records.iter().rev() {
            if message_role(&record.message) == Some(trimmed_role)
                && let Some(text) = message_text(&record.message)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            {
                return Ok(Some(text.to_owned()));
            }
        }
        Ok(None)
    }

    pub async fn delete(&self, thread_id: &str) -> Result<(), ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let slot = self.file_slot(thread_id).expect("file mode has slots");
                let mut cache = slot.state.lock().await;
                *cache = None;
                self.sync_slot_accounting(&slot, &cache);
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(());
                };
                match tokio::fs::remove_file(&path).await {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(ThreadHistoryError::TranscriptIo {
                        thread_id: thread_id.to_owned(),
                        message: error.to_string(),
                    }),
                }
            }
            TranscriptStoreMode::Memory { records } => {
                records.lock().await.remove(thread_id);
                Ok(())
            }
        }
    }

    async fn read_records(
        &self,
        thread_id: &str,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        match &self.mode {
            TranscriptStoreMode::File { .. } => {
                let Some(path) = self.transcript_path(thread_id) else {
                    return Ok(Vec::new());
                };
                self.read_records_from_path(thread_id, &path).await
            }
            TranscriptStoreMode::Memory { records } => Ok(records
                .lock()
                .await
                .get(thread_id)
                .cloned()
                .unwrap_or_default()),
        }
    }

    async fn read_records_from_path(
        &self,
        thread_id: &str,
        path: &Path,
    ) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
        Ok(self
            .read_records_sized_from_path(thread_id, path)
            .await?
            .into_iter()
            .map(|cached| cached.record)
            .collect())
    }

    async fn read_records_sized_from_path(
        &self,
        thread_id: &str,
        path: &Path,
    ) -> Result<Vec<CachedTranscriptRecord>, ThreadHistoryError> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        #[cfg(test)]
        self.full_file_reads.fetch_add(1, Ordering::Relaxed);
        let raw = tokio::fs::read_to_string(path).await.map_err(|error| {
            ThreadHistoryError::TranscriptIo {
                thread_id: thread_id.to_owned(),
                message: error.to_string(),
            }
        })?;
        let mut records = Vec::new();
        for (line_no, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed = serde_json::from_str::<TranscriptLine>(line).map_err(|error| {
                ThreadHistoryError::InvalidTranscript {
                    thread_id: thread_id.to_owned(),
                    message: format!("line {}: {}", line_no + 1, error),
                }
            })?;
            if let TranscriptLine::Message {
                seq,
                thread_id,
                run_id,
                timestamp,
                message,
            } = parsed
            {
                records.push(CachedTranscriptRecord {
                    bytes: line.len(),
                    record: ThreadTranscriptRecord {
                        seq,
                        thread_id,
                        run_id,
                        timestamp,
                        message,
                    },
                });
            }
        }
        Ok(records)
    }
}

fn serialize_transcript(
    thread_id: &str,
    records: &[ThreadTranscriptRecord],
) -> Result<Vec<u8>, ThreadHistoryError> {
    let mut lines = Vec::with_capacity(records.len() + 1);
    lines.push(
        serde_json::to_string(&TranscriptLine::Session {
            version: 1,
            thread_id: thread_id.to_owned(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .map_err(|error| invalid_transcript(thread_id, error))?,
    );
    for record in records {
        lines.push(
            serde_json::to_string(&TranscriptLine::from(record.clone()))
                .map_err(|error| invalid_transcript(thread_id, error))?,
        );
    }
    Ok(format!("{}\n", lines.join("\n")).into_bytes())
}

fn atomic_replace_error(
    thread_id: &str,
    stage: TranscriptReplaceStage,
    error: impl std::fmt::Display,
) -> ThreadHistoryError {
    ThreadHistoryError::AtomicReplace {
        thread_id: thread_id.to_owned(),
        stage,
        message: error.to_string(),
    }
}

fn invalid_transcript(thread_id: &str, message: impl std::fmt::Display) -> ThreadHistoryError {
    ThreadHistoryError::InvalidTranscript {
        thread_id: thread_id.to_owned(),
        message: message.to_string(),
    }
}

fn parse_import_transcript_bytes(
    expected_thread_id: &str,
    raw: &[u8],
) -> Result<ValidatedImportTranscript, ThreadHistoryError> {
    let trailing_newline = raw.ends_with(b"\n");
    let mut lines: Vec<&[u8]> = raw.split(|byte| *byte == b'\n').collect();
    if trailing_newline {
        lines.pop();
    }
    let mut records = Vec::new();
    let mut saw_header = false;
    let mut previous_seq = None;
    let mut torn_tail = !trailing_newline;

    for (index, raw_line) in lines.iter().enumerate() {
        let line_no = index + 1;
        let is_unterminated_tail = !trailing_newline && index + 1 == lines.len();
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            if is_unterminated_tail && saw_header {
                break;
            }
            return Err(invalid_transcript(
                expected_thread_id,
                format!("line {line_no}: empty transcript line"),
            ));
        }

        let parsed = match serde_json::from_slice::<TranscriptLine>(line) {
            Ok(parsed) => parsed,
            Err(_) if is_unterminated_tail && saw_header => break,
            Err(error) => {
                return Err(invalid_transcript(
                    expected_thread_id,
                    format!("line {line_no}: {error}"),
                ));
            }
        };
        match parsed {
            TranscriptLine::Session {
                version, thread_id, ..
            } if line_no == 1 => {
                if version != 1 {
                    return Err(invalid_transcript(
                        expected_thread_id,
                        format!("unsupported session version {version}"),
                    ));
                }
                if thread_id != expected_thread_id {
                    return Err(invalid_transcript(
                        expected_thread_id,
                        format!(
                            "session header thread_id {thread_id:?} does not match {expected_thread_id:?}"
                        ),
                    ));
                }
                saw_header = true;
            }
            TranscriptLine::Session { .. } => {
                return Err(invalid_transcript(
                    expected_thread_id,
                    format!("line {line_no}: duplicate session header"),
                ));
            }
            TranscriptLine::Message { .. } if line_no == 1 => {
                return Err(invalid_transcript(
                    expected_thread_id,
                    "first line is not a session header",
                ));
            }
            TranscriptLine::Message {
                seq,
                thread_id,
                run_id,
                timestamp,
                message,
            } => {
                if thread_id != expected_thread_id {
                    return Err(invalid_transcript(
                        expected_thread_id,
                        format!(
                            "line {line_no}: record thread_id {thread_id:?} does not match {expected_thread_id:?}"
                        ),
                    ));
                }
                if previous_seq.is_some_and(|previous| seq <= previous) {
                    return Err(invalid_transcript(
                        expected_thread_id,
                        format!("line {line_no}: seq {seq} is not strictly increasing"),
                    ));
                }
                previous_seq = Some(seq);
                records.push(ThreadTranscriptRecord {
                    seq,
                    thread_id,
                    run_id,
                    timestamp,
                    message,
                });
            }
        }
    }

    if !saw_header {
        return Err(invalid_transcript(
            expected_thread_id,
            "missing session header",
        ));
    }
    if trailing_newline {
        torn_tail = false;
    }
    Ok(ValidatedImportTranscript::Parsed {
        records,
        torn_tail,
        file_len: raw.len() as u64,
    })
}

/// Backward window start for "the last K user queries before `end`": walk
/// from `end` toward the head counting user queries; running short of the
/// target stops at the head, zero matches falls back to a fixed message
/// window. Shared between the in-memory scan and the streaming pass so both
/// modes keep identical semantics.
fn user_query_window_start(
    end: usize,
    user_query_limit: usize,
    fallback_message_limit: usize,
    is_user_query_at: impl Fn(usize) -> bool,
) -> usize {
    let target = user_query_limit.max(1);
    let mut start = end;
    let mut user_queries = 0usize;
    while start > 0 && user_queries < target {
        start -= 1;
        if is_user_query_at(start) {
            user_queries += 1;
        }
    }
    if user_queries == 0 {
        start = end.saturating_sub(fallback_message_limit.max(1));
    }
    start
}

fn transcript_io_error(thread_id: &str, error: impl std::fmt::Display) -> ThreadHistoryError {
    ThreadHistoryError::TranscriptIo {
        thread_id: thread_id.to_owned(),
        message: error.to_string(),
    }
}

fn parse_transcript_record_line(
    thread_id: &str,
    line: &str,
    location: impl std::fmt::Display,
) -> Result<Option<ThreadTranscriptRecord>, ThreadHistoryError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let parsed = serde_json::from_str::<TranscriptLine>(line).map_err(|error| {
        ThreadHistoryError::InvalidTranscript {
            thread_id: thread_id.to_owned(),
            message: format!("{location}: {error}"),
        }
    })?;
    Ok(match parsed {
        TranscriptLine::Message {
            seq,
            thread_id,
            run_id,
            timestamp,
            message,
        } => Some(ThreadTranscriptRecord {
            seq,
            thread_id,
            run_id,
            timestamp,
            message,
        }),
        TranscriptLine::Session { .. } => None,
    })
}

fn parse_transcript_record_bytes(
    thread_id: &str,
    line: &[u8],
    location: impl std::fmt::Display,
) -> Result<Option<ThreadTranscriptRecord>, ThreadHistoryError> {
    if line.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(None);
    }
    let line =
        std::str::from_utf8(line).map_err(|error| ThreadHistoryError::InvalidTranscript {
            thread_id: thread_id.to_owned(),
            message: format!("{location}: {error}"),
        })?;
    parse_transcript_record_line(thread_id, line, location)
}

fn collect_tail_scan_line(
    thread_id: &str,
    line: &[u8],
    after_seq: u64,
    limit: usize,
    tail: &mut Vec<ThreadTranscriptRecord>,
) -> Result<bool, ThreadHistoryError> {
    let Some(record) = parse_transcript_record_bytes(thread_id, line, "tail scan")? else {
        return Ok(false);
    };
    if record.seq <= after_seq {
        return Ok(true);
    }
    tail.push(record);
    Ok(tail.len() >= limit)
}

async fn read_tail_records_after_seq_from_path(
    thread_id: &str,
    path: &Path,
    after_seq: u64,
    limit: usize,
) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?;
    let mut position = file
        .metadata()
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?
        .len();
    let mut carry = Vec::new();
    let mut tail = Vec::new();

    while position > 0 && tail.len() < limit {
        let read_len = position.min(TAIL_SCAN_CHUNK_BYTES) as usize;
        position -= read_len as u64;
        file.seek(SeekFrom::Start(position))
            .await
            .map_err(|error| transcript_io_error(thread_id, error))?;
        let mut chunk = vec![0; read_len];
        file.read_exact(&mut chunk)
            .await
            .map_err(|error| transcript_io_error(thread_id, error))?;
        if !carry.is_empty() {
            chunk.extend_from_slice(&carry);
        }

        let mut end = chunk.len();
        while end > 0 {
            let Some(newline_index) = chunk[..end].iter().rposition(|byte| *byte == b'\n') else {
                break;
            };
            if collect_tail_scan_line(
                thread_id,
                &chunk[newline_index + 1..end],
                after_seq,
                limit,
                &mut tail,
            )? {
                tail.reverse();
                return Ok(tail);
            }
            end = newline_index;
        }
        carry.clear();
        carry.extend_from_slice(&chunk[..end]);
    }

    if tail.len() < limit
        && !carry.is_empty()
        && collect_tail_scan_line(thread_id, &carry, after_seq, limit, &mut tail)?
    {
        tail.reverse();
        return Ok(tail);
    }

    tail.reverse();
    Ok(tail)
}

async fn read_forward_records_after_seq_from_path(
    thread_id: &str,
    path: &Path,
    after_seq: u64,
    limit: usize,
) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?;
    let mut lines = BufReader::new(file).lines();
    let mut line_no = 0_usize;
    let mut records = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?
    {
        line_no += 1;
        let Some(record) = parse_transcript_record_line(thread_id, &line, line_no)? else {
            continue;
        };
        if record.seq <= after_seq {
            continue;
        }
        records.push(record);
        if records.len() >= limit {
            break;
        }
    }
    Ok(records)
}

async fn read_run_records_after_seq_from_path(
    thread_id: &str,
    path: &Path,
    run_id: &str,
    after_seq: u64,
    limit: usize,
) -> Result<Vec<ThreadTranscriptRecord>, ThreadHistoryError> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?;
    let mut lines = BufReader::new(file).lines();
    let mut records = Vec::new();
    let mut line_no = 0usize;
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| transcript_io_error(thread_id, error))?
    {
        line_no += 1;
        let Some(record) =
            parse_transcript_record_line(thread_id, &line, format!("line {line_no}"))?
        else {
            continue;
        };
        if record.seq > after_seq
            && (record.run_id.as_deref() == Some(run_id)
                || (record.run_id.is_none() && is_control_record_message(&record.message)))
        {
            records.push(record);
            if records.len() >= limit {
                break;
            }
        }
    }
    Ok(records)
}

impl Default for ThreadTranscriptStore {
    fn default() -> Self {
        Self::memory()
    }
}

impl From<ThreadTranscriptRecord> for TranscriptLine {
    fn from(value: ThreadTranscriptRecord) -> Self {
        Self::Message {
            seq: value.seq,
            thread_id: value.thread_id,
            run_id: value.run_id,
            timestamp: value.timestamp,
            message: value.message,
        }
    }
}

#[cfg(test)]
mod streaming_build_tests {
    use super::*;

    fn fixture_message(index: usize) -> Value {
        match index % 4 {
            0 => serde_json::json!({"role": "user", "content": format!("user message {index}")}),
            1 => serde_json::json!({
                "role": "assistant",
                "content": format!("assistant reply {index} {}", "x".repeat(index * 17 % 300)),
            }),
            2 => serde_json::json!({
                "role": "system",
                "kind": "control",
                "internal": true,
                "control": {
                    "kind": if index % 8 == 2 { "run_start" } else { "run_complete" },
                    "run_id": format!("run-{}", index / 4),
                },
            }),
            _ => serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": format!("tool-{index}"), "name": "probe"}],
            }),
        }
    }

    fn filtered_session_oracle(all: &[Value], limit: usize) -> Vec<Value> {
        let filtered: Vec<Value> = all
            .iter()
            .filter(|message| message.get("kind").and_then(Value::as_str) != Some("control"))
            .cloned()
            .collect();
        filtered[filtered.len().saturating_sub(limit)..].to_vec()
    }

    #[tokio::test]
    async fn provider_session_tail_skips_control_records_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        let thread_id = "thread::provider-session";
        for index in 0..24usize {
            store
                .append_committed_messages(
                    thread_id,
                    Some(&format!("run-{}", index / 4)),
                    &[fixture_message(index)],
                )
                .await
                .unwrap();
        }

        let all = store.tail(thread_id, 24).await.unwrap();
        // fixture_message writes a control row at every index % 4 == 2.
        let full = store.provider_session_tail(thread_id, 100).await.unwrap();
        assert_eq!(full, filtered_session_oracle(&all, 100));
        assert_eq!(full.len(), 18, "24 records minus 6 control rows");
        assert!(
            full.iter()
                .all(|m| m.get("kind").and_then(Value::as_str) != Some("control"))
        );

        let limited = store.provider_session_tail(thread_id, 5).await.unwrap();
        assert_eq!(limited, filtered_session_oracle(&all, 5));
    }

    #[tokio::test]
    async fn provider_session_tail_falls_back_to_full_read_when_cache_is_short() {
        let dir = tempfile::tempdir().unwrap();
        // Tiny tail budget: the cache holds at most 5 records, far fewer
        // than the requested content window, forcing the full-read path.
        let store = ThreadTranscriptStore::file_for_tests(dir.path(), 2048, 5, 1 << 20)
            .await
            .unwrap();
        let thread_id = "thread::provider-session-short";
        for index in 0..40usize {
            store
                .append_committed_messages(
                    thread_id,
                    Some(&format!("run-{}", index / 4)),
                    &[fixture_message(index)],
                )
                .await
                .unwrap();
        }

        let all = store.tail(thread_id, 40).await.unwrap();
        let tail = store.provider_session_tail(thread_id, 20).await.unwrap();
        assert_eq!(tail, filtered_session_oracle(&all, 20));
        assert_eq!(tail.len(), 20);
    }

    #[tokio::test]
    async fn provider_session_tail_keeps_internal_dispatch_user_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        let thread_id = "thread::provider-session-internal";
        store
            .append_committed_messages(
                thread_id,
                Some("run-1"),
                &[
                    // Internal dispatch user rows carry a top-level
                    // `internal: true` but are provider-session content —
                    // the legacy `messages` snapshot always kept them.
                    serde_json::json!({
                        "role": "user",
                        "content": "task dispatch body",
                        "internal": true,
                        "internal_kind": "dispatch",
                    }),
                    serde_json::json!({
                        "role": "system",
                        "kind": "control",
                        "internal": true,
                        "internal_kind": "control",
                        "control": {"kind": "run_complete", "run_id": "run-1"},
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "content": "task reply",
                    }),
                ],
            )
            .await
            .unwrap();

        let tail = store.provider_session_tail(thread_id, 100).await.unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0]["content"], "task dispatch body");
        assert_eq!(tail[0]["internal"], true);
        assert_eq!(tail[1]["content"], "task reply");
    }

    async fn assert_streaming_matches_full_read(store: &ThreadTranscriptStore, thread_id: &str) {
        let path = store.transcript_path(thread_id).unwrap();
        let full = store
            .read_records_sized_from_path(thread_id, &path)
            .await
            .unwrap();
        let file_len = tokio::fs::metadata(&path).await.unwrap().len();
        let mut oracle = ThreadCache::from_records(full, file_len);
        oracle.roll_tail(&store.cache_budget());

        let streamed = store.build_cache_streaming(thread_id, &path).await.unwrap();

        assert_eq!(streamed.checkpoint, oracle.checkpoint, "checkpoint fold");
        assert_eq!(
            streamed.render_prefix_checkpoint, oracle.render_prefix_checkpoint,
            "render-prefix checkpoint fold"
        );
        assert_eq!(streamed.base_seq, oracle.base_seq, "base_seq");
        assert_eq!(streamed.min_seq, oracle.min_seq, "min_seq");
        assert_eq!(streamed.last_seq, oracle.last_seq, "last_seq");
        assert_eq!(
            streamed.total_records, oracle.total_records,
            "total_records"
        );
        assert_eq!(streamed.file_len, oracle.file_len, "file_len");
        assert_eq!(streamed.tail_bytes, oracle.tail_bytes, "tail_bytes");
        let streamed_tail: Vec<(&ThreadTranscriptRecord, usize)> = streamed
            .tail
            .iter()
            .map(|cached| (&cached.record, cached.bytes))
            .collect();
        let oracle_tail: Vec<(&ThreadTranscriptRecord, usize)> = oracle
            .tail
            .iter()
            .map(|cached| (&cached.record, cached.bytes))
            .collect();
        assert_eq!(streamed_tail, oracle_tail, "tail records");
    }

    #[tokio::test]
    async fn streaming_cache_build_matches_full_read_when_tail_rolls() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThreadTranscriptStore::file_for_tests(dir.path(), 2048, 5, 1 << 20)
            .await
            .unwrap();
        let thread_id = "thread::streaming-rolls";
        for index in 0..40usize {
            store
                .append_committed_messages(
                    thread_id,
                    Some(&format!("run-{}", index / 4)),
                    &[fixture_message(index)],
                )
                .await
                .unwrap();
        }
        assert_streaming_matches_full_read(&store, thread_id).await;
    }

    #[tokio::test]
    async fn streaming_cache_build_matches_full_read_when_budget_covers_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThreadTranscriptStore::file_for_tests(dir.path(), 1 << 20, 4096, 1 << 24)
            .await
            .unwrap();
        let thread_id = "thread::streaming-covered";
        for index in 0..12usize {
            store
                .append_committed_messages(thread_id, Some("run-1"), &[fixture_message(index)])
                .await
                .unwrap();
        }
        assert_streaming_matches_full_read(&store, thread_id).await;

        let path = store.transcript_path(thread_id).unwrap();
        let streamed = store.build_cache_streaming(thread_id, &path).await.unwrap();
        assert!(
            streamed.covers_whole_file(),
            "small file stays fully cached"
        );
    }

    async fn write_bench_fixture(dir: &Path, thread_id: &str) -> u64 {
        let store = ThreadTranscriptStore::file(dir).await.unwrap();
        let filler = "x".repeat(40 * 1024);
        for index in 0..2000usize {
            let role = if index % 5 == 0 { "user" } else { "assistant" };
            store
                .append_committed_messages(
                    thread_id,
                    Some("run-bench"),
                    &[serde_json::json!({"role": role, "content": format!("{index} {filler}")})],
                )
                .await
                .unwrap();
        }
        let path = store.transcript_path(thread_id).unwrap();
        tokio::fs::metadata(&path).await.unwrap().len()
    }

    /// Manual benchmark halves — run each in its own process and compare
    /// `maximum resident set size`:
    /// `/usr/bin/time -l cargo test -p garyx-router bench_cold_start_streaming_build -- --ignored --nocapture`
    /// `/usr/bin/time -l cargo test -p garyx-router bench_cold_start_full_read_build -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "manual memory/latency benchmark"]
    async fn bench_cold_start_streaming_build() {
        let dir = tempfile::tempdir().unwrap();
        let thread_id = "thread::bench-cold";
        let file_len = write_bench_fixture(dir.path(), thread_id).await;

        let cold = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        let path = cold.transcript_path(thread_id).unwrap();
        let started = std::time::Instant::now();
        let entry = cold.build_cache_streaming(thread_id, &path).await.unwrap();
        println!(
            "streaming cold build: {:?} for {} bytes ({} records total, {} in tail)",
            started.elapsed(),
            file_len,
            entry.total_records,
            entry.tail.len(),
        );
    }

    #[tokio::test]
    #[ignore = "manual memory/latency benchmark"]
    async fn bench_cold_start_full_read_build() {
        let dir = tempfile::tempdir().unwrap();
        let thread_id = "thread::bench-cold";
        let file_len = write_bench_fixture(dir.path(), thread_id).await;

        let cold = ThreadTranscriptStore::file(dir.path()).await.unwrap();
        let path = cold.transcript_path(thread_id).unwrap();
        let started = std::time::Instant::now();
        let records = cold
            .read_records_sized_from_path(thread_id, &path)
            .await
            .unwrap();
        let mut entry = ThreadCache::from_records(records, file_len);
        entry.roll_tail(&cold.cache_budget());
        println!(
            "full-read cold build: {:?} for {} bytes ({} records total, {} in tail)",
            started.elapsed(),
            file_len,
            entry.total_records,
            entry.tail.len(),
        );
    }

    #[tokio::test]
    async fn failed_streaming_build_resets_slot_accounting() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThreadTranscriptStore::file_for_tests(dir.path(), 1 << 20, 4096, 1 << 24)
            .await
            .unwrap();
        let thread_id = "thread::failed-build-accounting";
        store
            .append_committed_messages(thread_id, Some("run-1"), &[fixture_message(0)])
            .await
            .unwrap();
        // Warm the cache through a read.
        assert_eq!(store.message_count(thread_id).await.unwrap(), 1);
        let slot = store.file_slot(thread_id).unwrap();
        assert!(slot.cached_bytes.load(Ordering::Relaxed) > 0);

        // Out-of-band corruption drops the warm entry on the next verify and
        // makes the streaming rebuild fail.
        let path = store.transcript_path(thread_id).unwrap();
        tokio::fs::write(&path, b"not-json\n").await.unwrap();

        // The read surfaces the parse error via the fallback path...
        assert!(store.message_count(thread_id).await.is_err());
        // ...and the slot no longer claims bytes for the dropped cache.
        assert_eq!(
            slot.cached_bytes.load(Ordering::Relaxed),
            0,
            "failed rebuild must sync slot accounting for the dropped entry"
        );
    }

    #[tokio::test]
    async fn streaming_cache_build_handles_missing_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThreadTranscriptStore::file_for_tests(dir.path(), 2048, 5, 1 << 20)
            .await
            .unwrap();
        let missing = dir.path().join("missing.jsonl");
        let entry = store
            .build_cache_streaming("thread::missing", &missing)
            .await
            .unwrap();
        assert_eq!(entry.total_records, 0);
        assert_eq!(entry.file_len, 0);

        let empty = dir.path().join("empty.jsonl");
        tokio::fs::write(&empty, b"").await.unwrap();
        let entry = store
            .build_cache_streaming("thread::empty", &empty)
            .await
            .unwrap();
        assert_eq!(entry.total_records, 0);
    }
}
