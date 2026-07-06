# TASK-1715: claude result-time finalize + transcript store cache

Fixes the two latency bug groups confirmed by the #TASK-1710 audit
(`docs/agents/audit-message-pipeline-latency.md`, review #TASK-1714 PASS):

- **Knife 1 (audit fix #1)**: a claude run's final answer is committed only at
  `StreamEvent::Done`, which fires after post-result drain (2s) + CLI exit
  grace (2s, always maxed) + session title read. The answer sits invisible for
  ~4-6s after it was generated.
- **Knife 2 (audit fixes #3+#4)**: every per-thread SSE frame re-reads, re-parses
  and re-reduces the whole transcript jsonl with no cache; every append re-reads
  the whole file just to compute `next_seq`; `io_lock` is one mutex for the whole
  store, so one large-thread write blocks every other thread's writes.

Reproduced baselines (2026-07-06, local gateway 0.1.32 @ port 31337, audit §3
method):

| Metric | Baseline (this task's own runs) | Audit reference |
|---|---|---|
| claude tail gap (assistant transcript ts → SSE frame w/ committed answer) | **5.764s / 4.780s** (2 runs, `thread::1084120a…`) | 4.14-4.27s |
| snapshot-only first-frame derive, 188MB / 28058 records | **535.4 / 396.8 / 397.1 ms** | 445-534ms (197MB) |
| 38MB / 6379 records | 119.9 / 102.6 / 107.8 ms | 89-95ms (25.6MB) |
| 7.0MB / 1076 records | 54.1 / 39.8 / 45.7 ms | 13.7-14.6ms (5.6MB) |
| 80KB / 9 records | 12.0 / 1.9 / 1.9 ms | 0.6-0.7ms (20KB) |

Targets: tail gap < 0.5s (≥2 runs); 188MB per-frame derive down ≥10x
steady-state **on the paths real clients hit (iOS declared floor, desktop cold
open and caught-up reconnect)**; appends no longer read the whole file; codex
tail behavior (~0.1-0.2s) unchanged.

---

## Knife 1 — finalize the answer when `ResultMessage` arrives

### Mechanism today

`garyx-bridge/src/claude_provider.rs`:

- `process_messages_streaming` (`:1443-1613`): `Message::Result` only sets
  `result_seen = true` and captures `ProcessedResult` (`:1547-1561`). The loop
  then drains with `POST_RESULT_DRAIN_TIMEOUT_SECS = 2` (`:38`, `:1459-1476`).
- After the loop: `run.finish()` (claude-agent-sdk `client.rs:47,337-369`:
  close stdin, wait `FINISH_PROCESS_GRACE_TIMEOUT = 2`, then kill — the CLI
  takes ~5s after EOF, so the grace always maxes out), unregister, optional
  session-file title read (`:1391-1404`).
- Only then `run_streaming` emits `StreamEvent::Done` (`:1885`).

The persistence worker (`multi_provider/run_management/persistence_worker.rs`)
keeps the trailing assistant segment in-flight (`persistence.rs:799-806`
`finalized_len()`), and only `ToolUse`/`ToolResult`/`Boundary`/`Done` finalize
it. So the final answer's commit + `committed_message` emission happen at Done,
~4-6s after the text arrived.

### Change

In `process_messages_streaming`, track whether the trailing assistant segment
is still in flight, and when a **successful** `ResultMessage` arrives while it
is, emit the existing segment-boundary event before continuing the drain loop:

```rust
Ok(Message::Result(result_msg)) => {
    result_seen = true;
    if assistant_text_in_flight && !result_msg.is_error {
        assistant_text_in_flight = false;
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        });
    }
    result_data = Some(ProcessedResult { ... });
}
```

`assistant_text_in_flight` bookkeeping (all inside this one function):

- set on each non-synthetic assistant message to
  `assistant_blocks_end_with_text(&content)` (new helper next to the existing
  `assistant_blocks_*` helpers): a message whose last content block is visible
  text leaves an in-flight tail; one ending in `tool_use` does not;
- cleared by tool-result user messages (`ToolResult` events finalize the tail)
  and by the `UserAck` boundary emission;
- cleared after emitting the result-time boundary.

No other control flow changes: the drain loop, `try_close_pending_inputs`,
`finish()`, grace/kill, title read, and the final `Done` at `:1885` are
untouched. `POST_RESULT_DRAIN_TIMEOUT_SECS` and `FINISH_PROCESS_GRACE_TIMEOUT`
keep their values — process teardown still happens exactly as today, it just no
longer gates answer visibility.

### Why `Boundary { AssistantSegment }` and not a new event

- The persistence worker already implements exactly the needed semantics:
  `StreamingRunSnapshot::apply_stream_event` marks
  `start_new_assistant_segment = true` for any `Boundary` (`persistence.rs:695-699`),
  which moves the tail into `finalized_len()`; the worker then appends an
  `assistant_boundary` control record (`persistence_worker.rs:135-177`), which
  sets `dirty` and triggers the flush that commits the answer row and emits the
  seq'd `committed_message` (write-then-emit preserved).
- `Boundary` is a "persistent control stream event"
  (`persistence_worker.rs:110-112`), so the channel-facing callback is invoked
  only after the commit (`run_management.rs:731-753`) — ordering identical to
  today's boundaries.
- The event already exists in the stream vocabulary; claude emits it between
  assistant segments today (`claude_provider.rs:1534-1537`), so every channel
  already has an `AssistantSegment` handler and the extra turn-end boundary
  changes no channel's policy. Effects per channel (verified against code,
  design-review #TASK-1722 correction): the plugin dispatcher flushes buffered
  segment text (`dispatcher.rs:313`), so Telegram's throttled policy
  (`OnEveryDelta` + min interval) gains an earlier final flush point; Discord's
  buffered policy (`discord.rs:856` `buffered_until_tool_or_done`,
  `plugin_tools.rs:236` returns `Wait` for text) treats the boundary as
  separator state only (`discord.rs:1325-1336`) and still sends the final
  message at `Done` (`discord.rs:1349`) — **identical to today, no regression
  and no improvement there by design** (the buffered policy is an intentional
  contract, `docs/agents/repository-contracts.md`); weixin mirrors its existing
  segment handling. Knife 1's latency target is the committed-record consumers
  (per-thread SSE → Mac/iOS/API), not channel presentation policies.
  `run_graph`/`graph_engine` do not consume `Boundary`.
- The `assistant_boundary` control record is inert for rendering: kind
  `control` rows are not rendered, and the run-state reducer only recomputes
  the activity label (`garyx-models/src/transcript_run_state.rs:129-133`).
  `done`/`run_complete` control timing is unchanged, so run/task lifecycle,
  busy state, and reconcile semantics stay as today.

### Correctness argument (task's hard constraints)

- **Mid-stream semantics preserved**: nothing changes for text before tool
  boundaries; the only new emission happens after `ResultMessage`, i.e. after
  the turn's output is complete.
- **Late/queued messages in the drain window are not lost, duplicated or
  reordered**: all events flow through the same per-run unbounded channel into
  the single persistence worker, so ordering is total. After the result-time
  boundary, `start_new_assistant_segment == true`, so any late `Delta` opens a
  *new* segment row (`persistence.rs:757-793`) which commits at its own
  boundary/Done/final-flush; `save_streaming_partial` appends strictly by the
  `already_appended` cursor (`persistence.rs:1225-1235`), so the early commit
  cannot double-append. A queued input ack (`UserAck`) resets `result_seen`
  and the run continues exactly as today; the next turn's result emits its own
  finalize boundary.
- **Multi-turn runs**: each successful result finalizes that turn's answer;
  order per the single worker channel.
- **Error/retry paths unchanged**: `result_msg.is_error` skips the early
  finalize, so `should_retry`/`should_retry_message_with_fresh_session`/
  `resumed_run_stalled_without_response` (`claude_provider.rs:1807-1869`) see
  exactly today's stream. A retry's replacement text therefore cannot get an
  extra committed error-tail row it would then diverge from;
  `reconcile_run_tail` behavior is unchanged.
- **Providers other than claude**: untouched (codex already emits Done
  immediately after `turn/completed`).

### Expected result

Answer visible at ~(CLI result latency ≈ 0.1-0.2s) + commit/SSE ≈ 10-30ms after
the answer text reaches the bridge — well under 0.5s. `done`/`run_complete`
still arrive ~4-6s later (unchanged teardown); the busy indicator tail is the
same as today, but with the answer on screen.

---

## Knife 2 — transcript store: per-thread cache, tail cursor, per-thread lock

### Mechanism today (`garyx-router/src/thread_history/store.rs`)

- `render_snapshot_at_seq` (`:1090`) / `render_snapshot_in_window` (`:1104`):
  full `read_records_from_path` (`:1364-1407`, read whole file + serde parse
  every line) + full `reduce_transcript_run_state` prefix reduce **per frame,
  per subscriber** (`garyx-gateway/src/routes.rs:2277-2338`).
- `append_committed_messages` (`:80`) / `append_run_records` (`:218`): full
  file read just for `next_seq`, while holding the lock.
- `io_lock` (`:9,45`) is store-global: a 188MB thread's append blocks writes of
  every other thread.
- `reconcile_run_tail` / `reconcile_run_records_tail` (`:528`, `:612`) read the
  whole file **outside** the lock, then re-read inside `append_*`/
  `write_records` (read-modify-write race window; benign today only because the
  bridge serializes per-thread writes).

### Change — `File` mode only; `Memory` mode untouched

**1. Per-thread state registry replaces the global lock.**

```rust
File {
    root_dir: PathBuf,
    threads: std::sync::Mutex<HashMap<String, Arc<ThreadSlot>>>, // brief registry access
}
struct ThreadSlot {
    state: tokio::sync::Mutex<Option<ThreadCache>>, // per-thread io lock + cache
}
```

Lock order is always registry → slot, the registry mutex is never held across
`.await` or slot locking (clone the `Arc`, drop the guard), and no code path
holds two slots — no deadlock, no cross-thread write contention. All write
paths (append × 2, rewrite, `write_records`, both reconciles, delete) hold the
thread's slot lock for their **entire** read-modify-write, which also closes
the reconcile TOCTOU above. Public methods lock the slot then delegate to
internal `*_locked` helpers (reconcile's internal append/rewrite calls use the
already-held guard, no re-entrant locking).

**2. `ThreadCache` — bounded tail + run-state checkpoint + counters.**

```rust
struct ThreadCache {
    /// TranscriptRunState folded over records [min_seq ..= base_seq].
    checkpoint: TranscriptRunState,
    base_seq: u64,
    /// Parsed records (base_seq, last_seq], with per-record byte estimates.
    tail: VecDeque<CachedRecord>,      // CachedRecord = { record, bytes }
    tail_bytes: usize,
    min_seq: u64,                       // first record seq (0 when empty)
    last_seq: u64,                      // == next_seq - 1
    total_records: usize,
    file_len: u64,                      // fstat length after our last write
    last_used: Instant,                 // LRU
}
```

- Built lazily on first access (read or write) of a thread: one full
  `read_records_from_path` + fold — the same cost one of today's frames pays,
  paid once per process/eviction — then trimmed to the tail cap.
- Seqs are monotonic + gapless (already a store invariant relied on by
  `records_after_seq`'s tail scan and `save_streaming_partial`'s
  "last K records have seqs total-K+1..=total"), which is what makes
  checkpoint+tail equivalent to a full prefix fold.
- Bounds: `TRANSCRIPT_CACHE_TAIL_MAX_BYTES` (per thread, 4MiB) — overflow folds
  the oldest tail records into `checkpoint` (advancing `base_seq`) and drops
  them; `TRANSCRIPT_CACHE_TOTAL_MAX_BYTES` (global, 64MiB) — overflow evicts
  least-recently-used *other* entries entirely. Byte estimate = serialized
  line length (known at parse/append time).

**3. Cache maintenance per write path** (all under the slot lock, after the
file write succeeds; any file I/O error drops the cache entry so the next
access rebuilds from disk):

| Path | Cache action |
|---|---|
| `append_committed_messages` / `append_run_records` | `next_seq = last_seq + 1` from cache (no file read); push appended records to `tail`; bump `last_seq`/`total_records`; refresh `file_len` via one fstat |
| `write_records` (reconcile rewrites) / `rewrite_from_messages` | rebuild the entry in place from the full record set already in memory (fold prefix, keep capped tail) — no extra disk read |
| `reconcile_run_tail` / `reconcile_run_records_tail` | trailing-run-block comparison served from the cached tail **iff** the tail provably contains the whole block (a record with a different/absent run_id precedes it inside the tail); otherwise full read under the lock (today's cost). No-op / suffix-append fast paths then skip the full read entirely |
| `delete` | drop entry |

Before trusting a cache entry (reads and appends alike) the store re-checks
`fstat(path).len == file_len`; mismatch drops + rebuilds. Our own writes update
`file_len` under the lock, so steady state is always a match; this is a cheap
(~µs) guard against any out-of-band writer, and is strictly safer than today
(which has no cross-process protection at all).

**4. Reads served from cache** (fallback = today's full read; equivalence
oracle-tested):

- `render_snapshot_in_window(floor, S)` — the per-frame hot path. Cache hit
  requires `S >= base_seq + 1` (well, `S >= tail start`) and
  `floor > base_seq`; then:
  - run_state at S = clone `checkpoint` + fold tail records with `seq <= S`
    (`garyx_models::apply_transcript_record` is already public);
  - window rows = tail slice `[max(floor, tail start) ..= S]` converted with
    the same `serde_json::to_value(record)` shape as today;
  - `actual_based_on_seq = min(S, last_seq)` (gapless ⇒ equals today's
    "max prefix seq"; the hit precondition guarantees a non-empty prefix);
  - `has_more_above = floor > min_seq` (equals today's
    "any record with seq < floor" for a non-empty prefix).
  Floors older than the cached tail (long-lived connections) or `S < base_seq`
  (deep replay backlog) fall back to the full read — same output, today's
  cost, plus a tracing counter so fallback frequency is observable.
- `run_state(thread)` — checkpoint + full tail fold (= run_state at tail).
- `cold_open_user_turn_window` / `tail` / `message_count` — served from cache
  when the cached tail covers the request (enough user-turn rows / limit ≤
  tail length; `message_count` from `total_records`); otherwise full read.
- Unchanged paths (still direct file access, no cache interaction beyond the
  shared slot lock on writes): `render_snapshot_at_seq` (floor=0 clients —
  after the desktop change below this is no longer any real client's
  steady-state on large threads), `records_after_seq` (+`_page`,
  `records_for_run_after_seq` — replay already tail-scan optimized), `page_*`,
  `find_*`, `records()`, `exists`.

**5. Desktop pins its render floor across reconnects** (design-review
#TASK-1722 blocker fix — without this, the Mac hot path misses the cache).

Today only iOS declares `render_floor`
(`GaryxGatewayClient.swift:915-925`); desktop connects with
`after_seq=…&windowed_resume=1` only (`desktop/src/main/gary-client/stream.ts:344`).
So a desktop cold open on a large thread gets a server-assigned window floor
via the degraded windowed replay (`routes.rs:2127-2162`), but every caught-up
resume (reconnect after stream error/lag with `forwarder.lastSeq`,
`desktop/src/main/index.ts:365-399`) lands back on `render_floor == 0`
(`routes.rs:2330-2335`), i.e. the uncached `render_snapshot_at_seq` full
read+reduce **plus full-row serialization** per live frame — the worst path on
the 188MB thread, and inconsistent with the narrowed rows the same connection
was rendering before the reconnect.

Change (desktop main process only):

- `stream.ts`: `StreamThreadEventsOptions` gains `renderFloor?: number`
  (appended as `&render_floor=` when > 0) and `onWindowFloor?: (floorSeq:
  number) => void`, invoked when a frame's `render_state.window.floor_seq` is
  a positive number (the wire shape is verbatim serde snake_case,
  `parseRenderState` at `stream.ts:254-266`).
- `index.ts`: `ThreadStreamForwarder` tracks `lastFloor` next to `lastSeq`;
  reconnects and `restartThreadEventForwarders` pass it back as `renderFloor`.

Why this is safe and contract-compliant: `render_state.rows` narrowed by a
client-declared `render_floor` is the documented contract (CLAUDE.md /
repository-contracts); the mirror already dumb-renders narrowed rows on every
frame of a degraded-open connection today, and pinning merely preserves the
narrowing that same connection was already rendering. A later degraded replay
that assigns a new window floor updates the pin (`onWindowFloor` fires on its
frames). Small threads never receive a window → `lastFloor` stays 0 → requests
are byte-identical to today. iOS is untouched (already pins its own floor).

### Contract compliance / non-regression argument

- **write-then-derive**: unchanged — commits still hit the jsonl first; the
  cache is updated under the same lock immediately after the successful write,
  before any event is emitted by callers; frames still derive from committed
  records only.
- **`based_on_seq` semantics**: cache-hit output is defined to equal the
  full-read output (min(S, last_seq) / patched-0 behavior identical);
  `routes.rs:2312` (`based_on_seq != seq` → reconnect) keeps functioning; the
  live event for seq S derives only after S's append returned, and the append
  updated the cache under the same slot lock, so a hit can never be stale.
- **Same-seq overwrite events**: produced only by reconcile rewrite paths,
  which rebuild the cache in place before the events are emitted — the re-derive
  at the same seq sees the rewritten content.
- **windowed_resume / render_floor / replay**: `records_after_seq*` and the
  replay builder are untouched; `cold_open_user_turn_window` keeps its exact
  output (oracle-tested) so degraded-resume floors are identical.
- **Cold start**: no eager work; first touch rebuilds from the file. A fresh
  store instance on the same directory returns identical results (tested).
- **Concurrency**: writes to different threads no longer serialize (this is
  the "one big append blocks everyone" fix); per-thread readers/writers
  serialize on the slot lock (bounded: cache-hit ops are µs-ms; the worst hold
  is a cold rebuild, which today's *every* frame pays without any lock at all).
  `std::sync::Mutex` registry guard never crosses `.await`.
- **Memory**: ≤ 64MiB global + one transient full parse during
  rebuild (same transient today's reads allocate on every frame).

### Expected result

- Steady-state frame derive on the 188MB thread: fold Δ + window slice +
  window reduce ≈ single-digit ms (≥10x below the 397-535ms baseline; first
  frame after restart still pays one rebuild).
- Both real desktop paths ride the cached window path: cold open via the
  degraded windowed floor (as today) and caught-up resumes via the pinned
  floor (new). iOS already declares its floor on every connect.
- Appends: O(new records) + one fstat, no full-file read (provable via a
  test-only full-read counter and by timing appends on a large fixture).
- Cross-thread write blocking eliminated.

### Explicitly out of scope

- Changing any channel presentation policy (Discord stays
  buffered-until-tool-or-done per contract; its final message still arrives at
  Done exactly as today).
- Per-subscriber duplicate derivation (audit §2.3 routes note) — cache makes
  each derivation cheap; deduping frames across subscribers is not needed for
  the target.
- iOS 3s batching (audit fix #2), delta typewriter streaming (audit #5), bus
  capacity (#6).

---

## Test & verification plan

Knife 1 (garyx-bridge):
- Unit: `process_messages_streaming` via the existing `MessageSource` test
  impls — a successful Result after trailing text emits exactly one
  `Boundary{AssistantSegment}` (before any Done); `is_error` result emits none;
  a result after a tool-result tail emits none; late post-result deltas still
  stream.
- Worker guard test: feed Delta("pong") → result-boundary → assert the flush
  committed the assistant row + `assistant_boundary` control and emitted
  seq'd `committed_message`s before Done; then Delta("late") → Done → assert
  "late" lands as a separate row, exactly once, order preserved.
- `cargo test -p garyx-bridge` (+ `-p garyx-models`) green.

Knife 2 (garyx-router):
- Equivalence oracle: randomized deterministic op sequences (appends of both
  kinds, reconciles hitting no-op/suffix/same-seq-overwrite/shrink, rewrites,
  deletes) on a File store; after every op compare `render_snapshot_in_window`
  (several floor/S combos incl. floor>tail and fallback cases), `run_state`,
  `cold_open_user_turn_window`, `tail`, `message_count`, `records` against a
  fresh uncached store instance reading the same directory. A test-only
  full-read counter asserts the hot path actually hit the cache (no
  silent-fallback fake pass).
- Tail-cap roll + LRU eviction correctness (tiny caps via test override).
- Cold restart parity; concurrent multi-thread append/read smoke (timeout
  guarded); seq continuity under concurrency.
- Desktop floor pinning unit tests in `src/main/gary-client.test.mjs` (already
  in the `test:unit` list): stream URL carries `render_floor` only when
  pinned; `onWindowFloor` fires from a frame with `render_state.window`;
  forwarder reconnect/restart carries the pinned floor.
- `cargo test -p garyx-router` (+ gateway stream tests `cargo test -p
  garyx-gateway`) green; `npm run test:unit` in `desktop/garyx-desktop` green.

End-to-end (after each knife, local gateway rebuilt via
`scripts/build-local-cli.sh`, single restart, then measure):
- Knife 1: tail-gap recorder ×2 on the claude test thread — target < 0.5s;
  codex thread ×1 — tail unchanged (~0.1-0.2s); verify done/run_complete still
  arrive and the thread returns to idle.
- Knife 2: same 4-tier snapshot benchmark — 188MB warm frames ≥10x faster;
  append timing/log evidence of no full read; desktop/iOS open + live stream
  sanity on a big thread; verify a desktop caught-up reconnect on the big
  thread now requests `render_floor > 0` and its live frames ride the cached
  window path (packaged app + gateway-side observation).

Rollout: two independent commits (knife 1 then knife 2), each with its tests;
codex design review before implementation; codex code review after; merge to
main after PASS.
