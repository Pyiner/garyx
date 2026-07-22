//! Per-thread SSE stream: replay, live frames, delta bases, reconnect errors.

use super::*;

#[derive(Deserialize)]
pub struct ThreadStreamParams {
    /// Resume cursor: replay committed messages with seq strictly greater than this.
    #[serde(default)]
    pub after_seq: u64,
    #[serde(default)]
    pub replay_scope: Option<ThreadStreamReplayScope>,
    #[serde(default)]
    pub initial_user_turns: Option<usize>,
    #[serde(default)]
    pub render_floor: Option<u64>,
    /// Capability negotiation (#TASK-1956 knife 1): `delta` declares that
    /// live frames may carry `render_delta` instead of a full
    /// `render_state`. Undeclared connections receive full frames
    /// indefinitely — a permanent contract surface, not a transition flag.
    #[serde(default)]
    pub render_mode: Option<ThreadStreamRenderMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStreamReplayScope {
    Resume,
    Initial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStreamRenderMode {
    Full,
    Delta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ThreadStreamReplayOptions {
    pub(super) replay_scope: ThreadStreamReplayScope,
    pub(super) initial_user_turns: Option<usize>,
    pub(super) render_floor: u64,
}

/// Serialized-replay byte budget for resume connections; over this, any
/// resume degrades to the initial window — unconditionally, for every
/// consumer (design: thread-render-frame-incremental.md knife 2).
pub(super) const THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET: usize = 1024 * 1024;

/// User-turn window served for a degraded resume — same default the
/// desktop and iOS cold-open planners use.
pub(super) const THREAD_STREAM_DEGRADED_RESUME_USER_TURNS: usize = 3;

#[cfg(test)]
impl ThreadStreamReplayOptions {
    pub(super) fn resume(render_floor: u64) -> Self {
        Self {
            replay_scope: ThreadStreamReplayScope::Resume,
            initial_user_turns: None,
            render_floor,
        }
    }
}

pub(super) fn thread_stream_replay_options(
    params: &ThreadStreamParams,
    last_event_id: Option<u64>,
    has_last_event_id: bool,
) -> (u64, ThreadStreamReplayOptions) {
    let after_seq = last_event_id.unwrap_or(params.after_seq);
    let replay_scope = if has_last_event_id {
        ThreadStreamReplayScope::Resume
    } else {
        params
            .replay_scope
            .unwrap_or(ThreadStreamReplayScope::Resume)
    };
    let initial_user_turns = match replay_scope {
        ThreadStreamReplayScope::Initial => params.initial_user_turns,
        ThreadStreamReplayScope::Resume => None,
    };
    (
        after_seq,
        ThreadStreamReplayOptions {
            replay_scope,
            initial_user_turns,
            render_floor: params.render_floor.unwrap_or(0),
        },
    )
}

/// GET /api/threads/:key/stream - resumable per-thread transcript stream (S5).
///
/// Replays committed messages with `seq > after_seq` (or the `Last-Event-ID`
/// header on reconnect), then streams that thread's live events. The bus is
/// subscribed BEFORE the replay snapshot is read so no commit is missed in the
/// gap, and exact duplicate `committed_message` payloads are deduped so the
/// resulting replay/live overlap is idempotent while same-seq overwrite events
/// still reach clients.
pub async fn thread_stream(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Query(params): Query<ThreadStreamParams>,
    headers: HeaderMap,
) -> axum::response::Response {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "thread not found"})),
            )
                .into_response();
        }
        Err(response) => return response.into_response(),
    };

    // Resume via Last-Event-ID (standard SSE) or the after_seq query param.
    let last_event_id_header = headers.get("last-event-id");
    let last_event_id = last_event_id_header
        .as_ref()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let (after_seq, replay_options) =
        thread_stream_replay_options(&params, last_event_id, last_event_id_header.is_some());

    // Subscribe BEFORE reading the replay snapshot (no gap); seq dedup below makes
    // the overlap idempotent.
    let rx = state.ops.events.subscribe();

    // Delta negotiation (#TASK-1956 knife 1): declared connections get a
    // per-connection diff base; every full frame (replay/snapshot-only
    // below, first-live/same-seq-reseed in the live loop) seeds it.
    let delta_base: Option<Arc<ThreadStreamDeltaBase>> = (params.render_mode
        == Some(ThreadStreamRenderMode::Delta))
    .then(|| Arc::new(std::sync::Mutex::new(None)));

    tracing::info!(
        thread_id = %thread_id,
        after_seq,
        render_floor = replay_options.render_floor,
        replay_scope = ?replay_options.replay_scope,
        render_delta = delta_base.is_some(),
        "per-thread stream connected"
    );

    let replay = build_thread_stream_replay(
        &state,
        &thread_id,
        after_seq,
        replay_options,
        delta_base.as_deref(),
    )
    .await;
    let render_floor_for_live = replay.render_floor;
    let replay_events = replay
        .events
        .into_iter()
        .map(|event| event.map(ThreadStreamEvent::into_sse_event));
    let mut sent_committed_payloads = replay.sent_payloads;

    let thread_for_live = thread_id.clone();
    let state_for_live = state.clone();
    let state_for_drops = state.clone();
    let mut last_sent_seq = replay.max_seq;
    let live = BroadcastStream::new(rx)
        .then(move |item| {
            let state_for_live = state_for_live.clone();
            let thread_for_live = thread_for_live.clone();
            let delta_base_for_live = delta_base.clone();
            let forwarded = match item {
                Ok(raw) => committed_thread_stream_live_payload(
                    &raw,
                    &thread_for_live,
                    &mut sent_committed_payloads,
                    &mut last_sent_seq,
                ),
                Err(_) => {
                    // Lagged: a slow consumer dropped events. Terminate this SSE
                    // response so the client reconnects from the last delivered seq and
                    // the file-backed replay fills the gap.
                    state_for_drops.ops.events.record_drop();
                    Err(thread_stream_reconnect_error("broadcast lagged"))
                }
            };
            async move {
                match forwarded {
                    Ok(Some((seq, payload))) => Some(
                        committed_thread_stream_live_event(
                            &state_for_live,
                            &thread_for_live,
                            seq,
                            payload,
                            render_floor_for_live,
                            delta_base_for_live.as_deref(),
                        )
                        .await,
                    ),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                }
            }
        })
        .filter_map(|event| async move {
            event.map(|event| event.map(ThreadStreamEvent::into_sse_event))
        });

    let combined = tokio_stream::iter(replay_events).chain(live);
    Sse::new(combined)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("ping"),
        )
        .into_response()
}

pub(super) struct ThreadStreamReplay {
    pub(super) events: Vec<Result<ThreadStreamEvent, io::Error>>,
    pub(super) max_seq: u64,
    pub(super) sent_payloads: HashMap<u64, String>,
    pub(super) render_floor: u64,
}

/// Per-connection delta base for `render_mode=delta` connections
/// (#TASK-1956 knife 1): the seq and rows digest of the last frame this
/// connection sent. `None` until the first full frame seeds it. Seeding
/// rule (one rule, everywhere): every frame that carries a full
/// `render_state` — replay, snapshot-only, or a full live frame — resets
/// this base to that frame's snapshot; the very next live frame may be a
/// delta. Wrapped in a `Mutex` only because the live stream's future
/// cannot borrow closure captures; access is strictly sequential.
pub(super) type ThreadStreamDeltaBase = std::sync::Mutex<Option<ThreadStreamRenderBase>>;

pub(super) struct ThreadStreamRenderBase {
    seq: u64,
    rows_hash: u64,
    row_hashes: HashMap<String, u64>,
}

pub(super) fn lock_thread_stream_delta_base(
    base: &ThreadStreamDeltaBase,
) -> std::sync::MutexGuard<'_, Option<ThreadStreamRenderBase>> {
    // A poisoned base only means a panic elsewhere while seeding; the
    // cached digest is still coherent (worst case the next frame goes
    // out full and reseeds), so recover instead of propagating the panic.
    base.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Seed the connection's delta base from a full snapshot and stamp the
/// snapshot with its `rows_hash` chain token. Called by every full-frame
/// constructor on delta connections.
pub(super) fn seed_thread_stream_delta_base(
    base: &ThreadStreamDeltaBase,
    render_state: &mut RenderSnapshot,
) {
    let digest = garyx_models::render_rows_digest(&render_state.rows);
    render_state.rows_hash = Some(digest.rows_hash);
    *lock_thread_stream_delta_base(base) = Some(ThreadStreamRenderBase {
        seq: render_state.based_on_seq,
        rows_hash: digest.rows_hash,
        row_hashes: digest.row_hashes,
    });
}

pub(super) struct ThreadStreamReplayBuilder {
    event_payloads: Vec<Value>,
    max_seq: u64,
    sent_payloads: HashMap<u64, String>,
    serialized_bytes: usize,
}

pub(super) struct ThreadStreamEvent {
    pub(super) id: u64,
    pub(super) payload: String,
}

impl ThreadStreamEvent {
    fn into_sse_event(self) -> Event {
        Event::default().id(self.id.to_string()).data(self.payload)
    }
}

pub(super) async fn build_thread_stream_replay(
    state: &Arc<AppState>,
    thread_id: &str,
    after_seq: u64,
    options: ThreadStreamReplayOptions,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    if matches!(options.replay_scope, ThreadStreamReplayScope::Initial) {
        if let Some(initial_user_turns) = options.initial_user_turns {
            let window = state
                .threads
                .history
                .transcript_store()
                .cold_open_user_turn_window(
                    thread_id,
                    initial_user_turns,
                    THREAD_TRANSCRIPT_REPLAY_CAP,
                )
                .await
                .unwrap_or_else(|_| garyx_router::ThreadTranscriptWindow {
                    records: Vec::new(),
                    floor_seq: 0,
                    has_more_above: false,
                });
            return thread_stream_replay_from_records(
                state,
                thread_id,
                after_seq,
                window.records,
                window.floor_seq,
                delta_base,
            )
            .await;
        }
    }

    let tail = state
        .threads
        .history
        .transcript_store()
        .records_after_seq(thread_id, after_seq, THREAD_TRANSCRIPT_REPLAY_CAP)
        .await
        .unwrap_or_default();

    let tail_has_gap = tail
        .first()
        .is_some_and(|record| record.seq > after_seq.saturating_add(1));
    if !tail_has_gap {
        let mut replay = ThreadStreamReplayBuilder {
            event_payloads: Vec::with_capacity(tail.len()),
            max_seq: after_seq,
            sent_payloads: HashMap::new(),
            serialized_bytes: 0,
        };
        append_thread_stream_replay_records(&mut replay, thread_id, tail);
        // Stale resume over the byte budget: abandon the span replay and
        // serve the initial window instead — unconditionally, for every
        // consumer (design: thread-render-frame-incremental.md knife 2).
        if replay.serialized_bytes > THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET {
            return degraded_windowed_resume_replay(state, thread_id, after_seq, delta_base).await;
        }
        return finalize_thread_stream_replay(
            state,
            thread_id,
            replay,
            options.render_floor,
            None,
            delta_base,
        )
        .await;
    }

    // Gap self-heal: page the span in forward. A sub-budget gap keeps the
    // verbatim paged replay; the moment the accumulated serialization
    // crosses the byte budget the resume degrades to the window instead of
    // paging in megabytes (design: thread-render-frame-incremental.md
    // knife 2).
    let mut cursor = after_seq;
    let mut replay = ThreadStreamReplayBuilder {
        event_payloads: Vec::new(),
        max_seq: after_seq,
        sent_payloads: HashMap::new(),
        serialized_bytes: 0,
    };
    loop {
        let page = state
            .threads
            .history
            .transcript_store()
            .records_after_seq_page(thread_id, cursor, THREAD_TRANSCRIPT_REPLAY_CAP)
            .await
            .unwrap_or_default();
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        append_thread_stream_replay_records(&mut replay, thread_id, page);
        if replay.serialized_bytes > THREAD_STREAM_RESUME_REPLAY_BYTE_BUDGET {
            return degraded_windowed_resume_replay(state, thread_id, after_seq, delta_base).await;
        }
        if replay.max_seq == cursor || page_len < THREAD_TRANSCRIPT_REPLAY_CAP {
            break;
        }
        cursor = replay.max_seq;
    }
    finalize_thread_stream_replay(
        state,
        thread_id,
        replay,
        options.render_floor,
        None,
        delta_base,
    )
    .await
}

/// Serve an over-budget stale resume as the initial window: same records a
/// `replay_scope=initial` connection would get, marked
/// `replay:"windowed"` so the client rebuilds from the window instead of
/// appending.
pub(super) async fn degraded_windowed_resume_replay(
    state: &Arc<AppState>,
    thread_id: &str,
    after_seq: u64,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    let window = state
        .threads
        .history
        .transcript_store()
        .cold_open_user_turn_window(
            thread_id,
            THREAD_STREAM_DEGRADED_RESUME_USER_TURNS,
            THREAD_TRANSCRIPT_REPLAY_CAP,
        )
        .await
        .unwrap_or_else(|_| garyx_router::ThreadTranscriptWindow {
            records: Vec::new(),
            floor_seq: 0,
            has_more_above: false,
        });
    let mut replay = ThreadStreamReplayBuilder {
        event_payloads: Vec::with_capacity(window.records.len()),
        max_seq: after_seq,
        sent_payloads: HashMap::new(),
        serialized_bytes: 0,
    };
    append_thread_stream_replay_records(&mut replay, thread_id, window.records);
    finalize_thread_stream_replay(
        state,
        thread_id,
        replay,
        window.floor_seq,
        Some("windowed"),
        delta_base,
    )
    .await
}

pub(super) async fn thread_stream_replay_from_records(
    state: &Arc<AppState>,
    thread_id: &str,
    after_seq: u64,
    records: Vec<ThreadTranscriptRecord>,
    render_floor: u64,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    let mut replay = ThreadStreamReplayBuilder {
        event_payloads: Vec::with_capacity(records.len()),
        max_seq: after_seq,
        sent_payloads: HashMap::new(),
        serialized_bytes: 0,
    };
    append_thread_stream_replay_records(&mut replay, thread_id, records);
    finalize_thread_stream_replay(state, thread_id, replay, render_floor, None, delta_base).await
}

pub(super) async fn finalize_thread_stream_replay(
    state: &Arc<AppState>,
    thread_id: &str,
    replay: ThreadStreamReplayBuilder,
    render_floor: u64,
    replay_kind: Option<&'static str>,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> ThreadStreamReplay {
    let mut events = Vec::new();
    let mut max_seq = replay.max_seq;
    if !replay.event_payloads.is_empty() {
        let event = thread_stream_frame_event(
            state,
            thread_id,
            replay.max_seq,
            replay.event_payloads,
            render_floor,
            replay_kind,
            delta_base,
        )
        .await;
        events.push(event);
    } else {
        let event = thread_stream_snapshot_only_frame_event(
            state,
            thread_id,
            replay.max_seq,
            render_floor,
            replay_kind,
            delta_base,
        )
        .await;
        if let Ok(event) = &event {
            max_seq = event.id;
        }
        events.push(event);
    }
    ThreadStreamReplay {
        events,
        max_seq,
        sent_payloads: replay.sent_payloads,
        render_floor,
    }
}

pub(super) fn append_thread_stream_replay_records(
    replay: &mut ThreadStreamReplayBuilder,
    thread_id: &str,
    records: Vec<ThreadTranscriptRecord>,
) {
    for record in records {
        replay.max_seq = replay.max_seq.max(record.seq);
        let payload = committed_thread_stream_replay_payload_value(thread_id, &record);
        let serialized = payload.to_string();
        replay.serialized_bytes += serialized.len();
        replay.sent_payloads.insert(record.seq, serialized);
        replay.event_payloads.push(payload);
    }
}

pub(super) fn committed_thread_stream_replay_payload_value(
    thread_id: &str,
    record: &ThreadTranscriptRecord,
) -> Value {
    json!({
        "type": "committed_message",
        "thread_id": thread_id,
        "run_id": record.run_id.as_deref(),
        "seq": record.seq,
        "message": &record.message,
    })
}

pub(super) fn committed_thread_stream_live_payload(
    raw: &str,
    thread_id: &str,
    sent_payloads: &mut HashMap<u64, String>,
    last_sent_seq: &mut u64,
) -> Result<Option<(u64, Value)>, io::Error> {
    let value: Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("thread_id").and_then(Value::as_str) != Some(thread_id) {
        return Ok(None);
    }
    if value.get("type").and_then(Value::as_str) != Some("committed_message") {
        return Ok(None);
    }
    let seq = value.get("seq").and_then(Value::as_u64).unwrap_or(0);
    match should_forward_committed_payload(sent_payloads, last_sent_seq, seq, raw) {
        CommittedPayloadAction::Forward => Ok(Some((seq, value))),
        CommittedPayloadAction::Skip => Ok(None),
        CommittedPayloadAction::Reconnect => Err(thread_stream_reconnect_error(
            "non-contiguous committed seq",
        )),
    }
}

pub(super) async fn committed_thread_stream_live_event(
    state: &Arc<AppState>,
    thread_id: &str,
    seq: u64,
    payload: Value,
    render_floor: u64,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> Result<ThreadStreamEvent, io::Error> {
    let Some(delta_base) = delta_base else {
        // Undeclared connection: full frames, byte-identical to the
        // pre-delta contract (no rows_hash).
        return thread_stream_frame_event(
            state,
            thread_id,
            seq,
            vec![payload],
            render_floor,
            None,
            None,
        )
        .await;
    };
    let mut render_state =
        thread_render_snapshot_at_seq(state, thread_id, seq, render_floor).await?;
    if render_state.based_on_seq != seq {
        return Err(thread_stream_reconnect_error(
            "render snapshot seq mismatch",
        ));
    }
    let digest = garyx_models::render_rows_digest(&render_state.rows);
    let delta = {
        let mut guard = lock_thread_stream_delta_base(delta_base);
        let delta = match guard.as_ref() {
            // Base strictly behind this frame: normal delta step.
            Some(base) if base.seq < seq => Some(garyx_models::derive_render_delta_from_base(
                base.seq,
                base.rows_hash,
                &base.row_hashes,
                &render_state,
                digest.rows_hash,
            )),
            // Same-seq overwrite (a changed payload re-landed on
            // `seq == last_sent_seq`; design "Same-seq overwrites") or no
            // base yet: emit a full frame instead of a delta.
            _ => None,
        };
        // Either way this frame becomes the new base (the seeding rule for
        // full frames; for delta frames the chain simply advances).
        *guard = Some(ThreadStreamRenderBase {
            seq,
            rows_hash: digest.rows_hash,
            row_hashes: digest.row_hashes,
        });
        delta
    };
    match delta {
        Some(delta) => Ok(ThreadStreamEvent {
            id: seq,
            payload: thread_stream_delta_frame_payload(thread_id, vec![payload], &delta),
        }),
        None => {
            render_state.rows_hash = Some(digest.rows_hash);
            Ok(ThreadStreamEvent {
                id: seq,
                payload: thread_stream_frame_payload(thread_id, vec![payload], &render_state, None),
            })
        }
    }
}

pub(super) async fn thread_stream_snapshot_only_frame_event(
    state: &Arc<AppState>,
    thread_id: &str,
    requested_seq: u64,
    render_floor: u64,
    replay_kind: Option<&'static str>,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> Result<ThreadStreamEvent, io::Error> {
    let mut render_state =
        thread_render_snapshot_at_seq(state, thread_id, requested_seq, render_floor).await?;
    if let Some(delta_base) = delta_base {
        seed_thread_stream_delta_base(delta_base, &mut render_state);
    }
    let id = render_state.based_on_seq;
    Ok(ThreadStreamEvent {
        id,
        payload: thread_stream_frame_payload(thread_id, Vec::new(), &render_state, replay_kind),
    })
}

pub(super) async fn thread_stream_frame_event(
    state: &Arc<AppState>,
    thread_id: &str,
    seq: u64,
    event_payloads: Vec<Value>,
    render_floor: u64,
    replay_kind: Option<&'static str>,
    delta_base: Option<&ThreadStreamDeltaBase>,
) -> Result<ThreadStreamEvent, io::Error> {
    let mut render_state =
        thread_render_snapshot_at_seq(state, thread_id, seq, render_floor).await?;
    if render_state.based_on_seq != seq {
        return Err(thread_stream_reconnect_error(
            "render snapshot seq mismatch",
        ));
    }
    if let Some(delta_base) = delta_base {
        seed_thread_stream_delta_base(delta_base, &mut render_state);
    }
    Ok(ThreadStreamEvent {
        id: seq,
        payload: thread_stream_frame_payload(thread_id, event_payloads, &render_state, replay_kind),
    })
}

pub(super) async fn thread_render_snapshot_at_seq(
    state: &Arc<AppState>,
    thread_id: &str,
    seq: u64,
    render_floor: u64,
) -> Result<RenderSnapshot, io::Error> {
    let store = state.threads.history.transcript_store();
    let result = if render_floor > 0 {
        store
            .render_snapshot_in_window(thread_id, render_floor, seq)
            .await
    } else {
        store.render_snapshot_at_seq(thread_id, seq).await
    };
    let mut snapshot = result
        .map_err(|error| io::Error::other(format!("failed to derive render snapshot: {error}")))?;
    if let Some(rate_limit) = snapshot.rate_limit.as_mut() {
        let db = state.ops.garyx_db.clone();
        let thread_id = thread_id.to_owned();
        let recovery = db
            .run_blocking(move |db| db.latest_quota_recovery_job_for_thread(&thread_id))
            .await
            .map_err(|error| {
                io::Error::other(format!("failed to read quota recovery state: {error}"))
            })?;
        apply_quota_recovery_overlay(rate_limit, recovery.as_ref());
    }
    Ok(snapshot)
}

pub(super) fn apply_quota_recovery_overlay(
    rate_limit: &mut garyx_models::transcript_render_state::RenderRateLimit,
    recovery: Option<&crate::garyx_db::QuotaRecoveryJob>,
) {
    let Some(recovery) = recovery else { return };
    if rate_limit.recovery_generation.as_deref() != Some(&recovery.blocked_run_id) {
        // The transcript may have committed a newer terminal while its async
        // SQL projection is still in flight. An older row must not mask it.
        return;
    }
    if matches!(
        recovery.state,
        crate::garyx_db::QuotaRecoveryState::Waiting | crate::garyx_db::QuotaRecoveryState::Claimed
    ) {
        let is_eligible = recovery.reset_at.is_some()
            || !matches!(
                recovery.wake_reason,
                crate::garyx_db::QuotaRecoveryWakeReason::QuotaReset
            );
        rate_limit.will_auto_resend = is_eligible;
        rate_limit.recovery_state = Some(recovery.state.as_str().to_owned());
        rate_limit.recovery_at = is_eligible.then(|| recovery.due_at.clone());
    } else {
        rate_limit.will_auto_resend = false;
        rate_limit.recovery_state = None;
        rate_limit.recovery_at = None;
    }
}

pub(super) fn thread_stream_frame_payload(
    thread_id: &str,
    event_payloads: Vec<Value>,
    render_state: &RenderSnapshot,
    replay_kind: Option<&'static str>,
) -> String {
    let mut payload = json!({
        "type": "thread_render_frame",
        "thread_id": thread_id,
        "events": event_payloads,
        "render_state": render_state,
    });
    if let (Some(kind), Some(obj)) = (replay_kind, payload.as_object_mut()) {
        obj.insert("replay".to_owned(), Value::String(kind.to_owned()));
    }
    payload.to_string()
}

/// Live frame for a `render_mode=delta` connection: `render_delta`
/// replaces `render_state`; `events` stay the cursor/body source of
/// truth, unchanged.
pub(super) fn thread_stream_delta_frame_payload(
    thread_id: &str,
    event_payloads: Vec<Value>,
    render_delta: &garyx_models::RenderDelta,
) -> String {
    json!({
        "type": "thread_render_frame",
        "thread_id": thread_id,
        "events": event_payloads,
        "render_delta": render_delta,
    })
    .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommittedPayloadAction {
    Forward,
    Skip,
    Reconnect,
}

pub(super) fn should_forward_committed_payload(
    sent_payloads: &mut HashMap<u64, String>,
    last_sent_seq: &mut u64,
    seq: u64,
    raw: &str,
) -> CommittedPayloadAction {
    if seq == 0 {
        return CommittedPayloadAction::Skip;
    }
    if sent_payloads.get(&seq).is_some_and(|sent| sent == raw) {
        return CommittedPayloadAction::Skip;
    }
    if seq > last_sent_seq.saturating_add(1) {
        return CommittedPayloadAction::Reconnect;
    }
    if seq < *last_sent_seq {
        return CommittedPayloadAction::Skip;
    }
    sent_payloads.insert(seq, raw.to_owned());
    *last_sent_seq = (*last_sent_seq).max(seq);
    CommittedPayloadAction::Forward
}

pub(super) fn thread_stream_reconnect_error(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, reason)
}
