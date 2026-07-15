# Render Window Expansion: History Below the Windowed-Resume Floor

Status: v2 (revised after design review #TASK-2299 FAIL; re-review pending)
Related: `thread-render-frame-incremental.md` (knife 2: over-budget degrade),
`thread-open-replay-trim-design.md` (floor-windowed rows),
`perf-thread-stream-replay-degrade.md`.

v2 changes after review: identity model rebuilt around legitimate same-seq
overwrites (ordering gate + full-snapshot change detection, matching the iOS
precedent); connection generations in the hub; explicit
desired/effective/pending floor state; trigger centralized into one reconcile
with full coverage; loaded-body store disambiguated; bounded anti-loop with a
convergence argument; performance section rewritten around the store's tail
cache (no more "zero Rust change" or "cost ∝ loaded range" claims); iOS
section inverted from "phase 2 fix" to "prior art to align with".

## Problem

After a stale resume degrades to a windowed replay (>1 MiB gap;
`resume_over_budget_degrades_to_window_by_default`), older history becomes
permanently invisible on desktop for the rest of the app session:

1. The windowed frame carries `render_floor > 0`; `render_state.rows` is
   derived by `render_snapshot_in_window(floor, tail)` — rows below the floor
   do not exist on any subsequent frame of that connection.
2. The Electron hub pins the floor upward-only
   (`lastFloor = max(input, existing)`, `thread-stream-hub.ts`), and the
   lifecycle re-pins it from the current `renderState.window.floor_seq` on
   every stream start (`transcript-lifecycle.ts`). No desktop code path ever
   requests a lower floor.
3. Scroll pagination (`fetchOlderThreadHistoryPage`) is HTTP-only: it prepends
   older message **bodies** into the cache and never touches the stream or the
   floor.
4. The UI renders strictly from `render_state.rows`
   (`buildThreadViewRowsWithLocalUsers`). Bodies below the floor have no row
   to hang on, so they never render. The pagination spinner works, the fetch
   succeeds, nothing appears.
5. Even if a wider snapshot arrived at the same `based_on_seq`, the frontier
   returns `changed: false` for an equal cursor (`frontier.ts`) and
   `mirror.applyFrame` gates on `accepted && changed`, so it would be silently
   dropped.

The only escape today is an app relaunch or mirror LRU eviction.

## Root Cause, Stated Architecturally

The transcript pipeline loads two independent resources:

- **Structure**: `render_state.rows`, server-derived, delivered by the stream.
- **Bodies**: committed message bodies, loaded by HTTP history pagination.

Pagination has always relied on an *implicit* contract: **structure is total,
bodies gate visibility**. Windowed resume made structure *partial*, but no
desktop component owns the obligation that structure must cover whatever
bodies are loaded, and the desktop's snapshot-change gate (`based_on_seq`
comparison alone) cannot even accept a structure-coverage change.

iOS does own that obligation (see Prior Art below); desktop never adopted it.
This design closes the gap with the same model, hardened where the desktop's
multi-consumer streams and reconnect machinery need more than iOS does.

## Prior Art: iOS Already Does This

The iOS app already implements the core loop
(`GaryxMobileModel+ThreadHistory.swift`, `GaryxThreadWindowPlanner` in
`GaryxTranscriptSyncPlanner.swift`):

- An older-history page extends the cached committed window backward, computes
  `floorSeqForOlderPage(firstIndex:)`, lowers the thread's stored render
  floor, and does `stopSelectedThreadStream(); startSelectedThreadStream()` —
  a reconnect with the lower floor.
- Snapshot acceptance compares the **full snapshot**
  (`setRenderSnapshot: guard renderSnapshotsByThread[threadId] != snapshot`),
  not just the cursor, so same-`based_on_seq` overwrites and wider windows
  both apply naturally.
- Every full `render_state` frame reseeds downstream state unconditionally
  (`GatewayStreamActor`).

Desktop aligns with this model. Where this design goes beyond what iOS
currently has (connection generations, desired/effective/pending floor state,
bounded expansion widening), a follow-up audit of iOS against those aspects is
listed at the end — gated on a reproduced iOS failure, per review policy.

## Design Invariant (explicit, convergent)

> **Convergence property**: once body loading and the stream quiesce, the
> render window covers every loaded committed body:
> `effective floor_seq <= min(seq of loaded committed bodies)` (or the window
> is full, `floor_seq = 0`).

The inequality is *transiently* violated between an older-page apply and the
arrival of the expansion frame — the design makes that window short and
guarantees convergence (see Anti-Loop). Full-structure threads
(`floor_seq = 0`) are the trivial case. The display layer keeps its existing
contract untouched: dumb-render `render_state.rows`, bodies gate visibility.

**"Loaded committed bodies" is defined against the store the UI actually
renders from**: the transcript cache's `uiMessages` — the same store
`ThreadPage`'s `messagesBySeq` is built from — restricted to entries carrying
a finite positive record `seq` (i.e. remote-final committed bodies; local
optimistic rows without a seq are excluded). It is explicitly NOT
`recordsBySeq`: pagination writes `uiMessages` only, and the windowed-degrade
`dropCommittedBelow` clears `recordsBySeq` only, so the two stores legitimately
diverge. The accessor (`earliestLoadedCommittedBodySeq()`) lives on
`ThreadTranscriptCache` next to `getHistoryPagination()`.

## Protocol Design

### 1. Snapshot acceptance: ordering gate + value change detection

Two orthogonal concerns, currently conflated in the frontier, get separated:

- **Ordering (frontier)**: a full `render_state` frame is *stale* iff
  `based_on_seq < current`. Stale frames are rejected. Everything at
  `based_on_seq >= current` is accepted — including same-seq frames. This is
  required independently of expansion: the server legitimately emits
  **same-seq overwrite frames** (the stream doc comment: "same-seq overwrite
  events still reach clients", with existing gateway tests), so "equal cursor
  means equal snapshot" was never a valid protocol assumption. A tuple
  identity `(based_on_seq, floor_seq)` would repeat the same mistake one
  dimension later; rejected.
- **Change detection (mirror)**: whether an accepted snapshot *replaces* the
  held one is decided by structural equality on the **full** `render_state`
  (rows, scalars, window) — the iOS `setRenderSnapshot` precedent. Unchanged
  snapshots skip `setRenderState`, preserving downstream memo/reference
  stability exactly as the current `changed: false` path does. Equality is a
  plain deep compare of the decoded snapshot; `rows_hash` stays what it is
  today — the delta-chain integrity token — and is not promoted to identity
  (it covers rows only and is not guaranteed on non-delta full frames).

`ThreadFrontier.acceptRender(basedOnSeq)` keeps only the ordering rule.
`ThreadFrontier.setRenderFloor` has **no production callers today** (dead
code) and is removed; the effective floor is tracked where it is owned (§3).

### 2. Floor ownership: renderer requests, server decides, hub carries

- **Renderer (transcript lifecycle)** is the floor *requester*: every stream
  start for a thread — selected consumer, side-chat consumer, refetch restart
  — passes the thread's single `desiredFloor` (§3). Consumers never compute
  their own floor, so a later consumer start cannot raise an unmet request.
- **Server** is the floor *decider*: a within-budget resume honors the
  requested floor (`finalize_thread_stream_replay` derives the snapshot at
  `options.render_floor` on both the caught-up and sub-budget paths); an
  over-budget gap still degrades to the cold-open floor regardless of the
  request (server authority, unchanged).
- **Hub** is a dumb carrier with **connection generations**: each `start()`
  attempt gets a generation; `sendEvent` forwarding, `onCommittedSeq`, and
  `onWindowFloor` are all guarded by "this controller is still the thread's
  current forwarder". Today an aborted attempt's in-flight callbacks only
  check sink liveness, so an old floor-3 frame can land *after* the new
  floor-1 connection's frame and corrupt cursor/floor state; `restartAll` has
  the same window. The upward-only `Math.max` pin is removed; the hub's
  `lastFloor` is simply "what the current generation last requested /
  was answered with", used for its own reconnect loop.

### 3. Floor state machine (lifecycle-owned, per thread)

Three explicit values replace the single overloaded `lastFloor` meaning:

- `effectiveFloor`: `window.floor_seq ?? 0` from the latest full
  `render_state` frame of the **current generation** (0 = full window; a
  full-window announcement clears it — the "only fires when positive"
  behavior of today's `onWindowFloor` is normalized to fire on every full
  frame with the 0/absent case included).
- `neededFloor`: `min(earliestLoadedCommittedBodySeq, effectiveFloor)` —
  what the invariant requires right now.
- `pendingExpansion { generation, targetFloor } | null`: at most one in
  flight. It ends at the **first authoritative full `render_state` frame of
  that generation** (whatever floor it carries) — not at IPC return, since
  the SSE attempt is long-lived.

One reconcile function owns the loop:

```
reconcile(thread):
  if pendingExpansion != null: return            # settle first
  if effectiveFloor == 0: return                 # full window, nothing to do
  if neededFloor >= effectiveFloor: return       # invariant holds
  target = expansionTarget(thread)               # §4
  pendingExpansion = { generation: nextGen, targetFloor: target }
  restart stream with renderFloor = target       # afterSeq continuity via hub
```

`reconcile` is invoked from **every path that can move either side of the
inequality** — not just the two points v1 named:

- older-history page apply (`applyOlderPage`);
- every full `render_state` frame apply (covers degrades raising the floor,
  and settles `pendingExpansion`);
- authoritative-refetch transcript apply;
- cache-restored transcript ingest (restored snapshots can carry pre-floor
  bodies and a windowed floor together);
- immediately before any consumer stream start (selected, side-chat,
  post-refetch restart), which also guarantees those starts read the
  reconciled `desiredFloor`.

### 4. Expansion = a reconnect with a lower `render_floor`, exponentially widened

`render_floor` is a connection parameter; changing the window means a new
connection. This reuses the existing machinery — no new channel, no new
server-side state:

- The hub's `start()` aborts the existing connection and resumes with
  `afterSeq = max(input.afterSeq, existing.lastSeq)`, so committed-event
  continuity is preserved across the swap; generation guards (§2) make the
  swap race-free.
- A caught-up expansion resume costs one snapshot-only frame (`events: []`)
  whose rows are derived by the existing `render_snapshot_in_window`.
- Sub-budget and caught-up expansion frames carry no `replay: "windowed"`
  marker, so `dropCommittedBelow` does not fire. **If a new over-budget gap
  has accumulated by the time the expansion connects, the resume degrades
  again** — marker present, drop fires, floor rises. That is correct server
  authority, and the anti-loop below handles it.
- Delta-mode connections reseed their delta base from every full
  `render_state` frame (existing gateway rule), so the rows-hash chain stays
  honest across expansion.

**Expansion target.** Naively expanding to exactly the loaded-body range
would issue one expensive derivation per scrolled page (§6). Instead the
target widens exponentially:

```
expansionTarget(thread):
  span = max(effectiveFloor - neededFloor, 2 × lastWidenSpan)
  lastWidenSpan = span
  return max(0, effectiveFloor - span)
```

The first expansion jumps at least to the global minimum loaded body seq
(which may already be several pages below the floor — cache-restored bodies,
earlier pagination); each subsequent expansion at least doubles the widened
span. Over-widening is harmless by construction: rows whose bodies are not
loaded are skipped by the renderer — that is the normal full-structure state
— and it prepays future pages, bounding the number of server derivations per
thread session to O(log(scrolled depth)) instead of O(pages).

### 5. Anti-loop: settle, one bounded retry, convergence

- At most one `pendingExpansion` per thread; it settles at the first
  authoritative full frame of its generation.
- If it settles with `effectiveFloor > targetFloor` (the resume re-degraded
  because a fresh over-budget gap raced in), reconcile may issue **one**
  immediate follow-up expansion — which is caught-up by construction (the
  degrade just replayed the window to the tail) and therefore succeeds unless
  yet another >1 MiB burst landed mid-flight. After that one retry, the
  lifecycle holds until the next external trigger (another page apply, frame
  apply, or consumer start). No timers anywhere.
- **Convergence argument**: on a quiescent ledger, an expansion resume is
  caught-up, cannot degrade (degrade requires an over-budget *gap*), and
  honors the requested floor — so the invariant is restored in exactly one
  round trip. Sustained non-convergence requires an unbounded stream of
  >1 MiB gaps between consecutive attempts, in which case holding at the
  degraded window until the next user-driven trigger is the correct behavior
  anyway.

### 6. Performance: the store's tail cache bounds what expansion may assume

The router transcript store keeps only a parsed **tail** per thread
(8 MiB / 4096 records; 64 MiB store-wide LRU). A floor below the cached tail
falls back to reading and reducing the full prefix from disk
(`store.rs`) — so, contrary to v1, expansion cost is **not** proportional to
the loaded range: each below-tail derivation is a full-prefix pass, and the
v1 per-page trigger would have been O(pages × ledger) in the worst case.

Mitigations in this design:

- **Exponential widening (§4)** bounds derivations per thread session to
  O(log(scrolled depth)); a deliberate scroll through hundreds of pages costs
  ~10 full-prefix derivations, not hundreds.
- **Contract tests with instrumentation**: the store's existing read counters
  (`full_file_reads`-style stats) gate the bound — a rolled-file thread
  driven through N pagination steps must show O(log N) full-file reads, and a
  single expansion must show at most one.
- No production Rust change is *assumed*; the gateway already honors
  requested floors on within-budget resumes. If the contract tests expose a
  gap (e.g. derivation counters missing for assertions, or a pathological
  re-read), the fix lands server-side behind the same tests — never as a
  client workaround. A reusable backward-derivation cache is explicitly out
  of scope unless the measured bound fails.

### 7. UI layer: zero changes

`buildThreadViewRows` / `buildThreadViewRowsWithLocalUsers` and the scroll /
prepend-anchor machinery stay exactly as they are. Restoring the
structure-⊇-bodies invariant upstream makes paged bodies light up through the
existing "skip rows with missing bodies" behavior. The litmus test of the
design: the display contract never changes, only the coverage guarantee
feeding it.

## Contract Documentation

`docs/agents/repository-contracts.md` (Transcript Rendering) gains: the
server may emit same-seq overwrite frames, so clients gate render-state
acceptance on cursor ordering and detect change by full-snapshot value, never
by cursor equality alone; clients that narrow structure via `render_floor`
own the convergent invariant that the window covers their loaded committed
bodies, and widen it by resuming with a lower `render_floor`.
`AGENTS.md`/`CLAUDE.md` mirror-sync applies.

## Alternatives Rejected

- **History pages return row fragments; client stitches structure.** Client
  would re-derive transcript structure (turn grouping across page
  boundaries), violating the server-render-state contract.
- **In-connection floor control (HTTP side channel + connection id).** A new
  stateful channel for a low-frequency, scroll-driven operation; reconnect
  already exists, is cheap when caught up, and keeps floor a plain connection
  parameter.
- **Stop pinning the floor (resume full every reconnect).** Resurrects the
  full-transcript stall on huge threads that windowed resume exists to
  prevent.
- **Client-synthesized rows for pre-floor bodies.** Reimplements user-turn
  grouping locally; forbidden by the transcript-rendering contract.
- **Tuple identity `(based_on_seq, floor_seq)` (v1).** Refuted by same-seq
  overwrite frames: equal tuple does not imply equal snapshot. Replaced by
  ordering gate + full-value change detection.
- **`rows_hash` as snapshot identity.** Covers rows only, not guaranteed on
  non-delta full frames; remains the delta-chain integrity token.

## Test Plan

- **Gateway (`routes/tests.rs`)**:
  - caught-up resume with a lower `render_floor` than the previous
    connection → snapshot-only frame, same `based_on_seq`,
    `window.floor_seq == requested`, rows cover the wider window, no
    `replay` marker;
  - within-budget gap resume with a lower requested floor → verbatim events
    plus snapshot at the requested floor;
  - over-budget gap resume with a lower requested floor → still degrades
    (request does not override the budget), `replay: "windowed"` present;
  - delta-mode connection: expansion full frame reseeds the base; the next
    live delta chains from the wide snapshot (a true downward-expansion
    delta test, beyond the existing floor-advance coverage);
  - store instrumentation: rolled-file thread, repeated floor lowering →
    `full_file_reads` grows O(log steps), one per expansion at most.
- **Frontier unit tests**: ordering matrix — stale rejected; same-seq
  accepted; same-seq same-floor different-rows (overwrite) accepted; scalar-
  only change accepted; forward cursor accepted.
- **Mirror change-detection tests**: unchanged full snapshot → no
  `setRenderState`, reference stability preserved; same-seq overwrite with
  different rows → applied; same-seq wider window → applied.
- **Hub unit tests**: generation guard — events/cursor/floor callbacks from
  an aborted attempt are discarded after a newer `start()`; `restartAll`
  generation safety; floor follows the renderer's declaration downward;
  reconnect carries the current generation's floor.
- **Lifecycle + mirror integration (headless, no UI)** — the reviewer's
  counterexample as the anchor regression:
  1. `uiMessages` holds bodies seq 1–4; windowed state at floor 3; initial
     rows contain only seq ≥ 3.
  2. Older-page apply (or cache-restore ingest) triggers reconcile; an
     expansion start is issued requesting floor ≤ 1.
  3. A full wide frame at the same `based_on_seq`, no `replay` marker,
     applies; `buildThreadViewRows` now yields the turns owning seq 1–2.
  4. Loop-free: a degrade response to the expansion (floor rises) triggers
     at most one follow-up; a second degrade holds until a new trigger.
  5. Multi-consumer: selected + side-chat on the same thread — the second
     consumer's start does not raise the unmet floor; expansion serves both.
  6. Cache-restore and authoritative-refetch paths trigger reconcile (bodies
     below floor present at ingest time).
  7. `uiMessages` vs `recordsBySeq` divergence: bodies only in `uiMessages`
     still drive `neededFloor`; successful expansion does not drop cached
     records; a genuine re-degrade with marker does.
- **iOS**: no changes in this task. Follow-up audit (separate task, gated on
  a reproduced failure): generation-style stale-callback guarding around the
  stop/start pair, anti-loop bounds, and expansion-cost behavior of the iOS
  floor planner.

## Open Questions

None blocking. Deliberate scope cuts: window *re-shrinking* under memory
pressure (mirror LRU already bounds retained state; shrinking would need its
own identity story and has no driving defect); iOS hardening (prior art works
today; audit gated on a repro); backward-derivation server cache (only if the
measured O(log) bound fails).
