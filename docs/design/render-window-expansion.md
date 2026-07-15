# Render Window Expansion: History Below the Windowed-Resume Floor

Status: draft (design review pending)
Related: `thread-render-frame-incremental.md` (knife 2: over-budget degrade),
`thread-open-replay-trim-design.md` (floor-windowed rows),
`perf-thread-stream-replay-degrade.md`.

## Problem

After a stale resume degrades to a windowed replay (>1 MiB gap;
`resume_over_budget_degrades_to_window_by_default`), older history becomes
permanently invisible for the rest of the app session:

1. The windowed frame carries `render_floor > 0`; `render_state.rows` is
   derived by `render_snapshot_in_window(floor, tail)` — rows below the floor
   do not exist on any subsequent frame of that connection.
2. The Electron hub pins the floor upward-only
   (`lastFloor = max(input, existing)`, `thread-stream-hub.ts`), and the
   lifecycle re-pins it from the current `renderState.window.floor_seq` on
   every stream start (`transcript-lifecycle.ts`). No code path ever requests
   a lower floor.
3. Scroll pagination (`fetchOlderThreadHistoryPage`) is HTTP-only: it prepends
   older message **bodies** into the cache and never touches the stream or the
   floor.
4. The UI renders strictly from `render_state.rows`
   (`buildThreadViewRowsWithLocalUsers`). Bodies below the floor have no row
   to hang on, so they never render. The pagination spinner works, the fetch
   succeeds, nothing appears.
5. Even if a wider snapshot arrived, the frontier treats an equal
   `based_on_seq` as "equal snapshot" (`frontier.ts`), so it would be silently
   dropped by the `accepted && changed` gate in `mirror.applyFrame`.

The only escape today is an app relaunch or mirror LRU eviction. iOS consumes
the same contract (`GaryxTranscriptSyncPlanner.renderFloor`,
`resetCommittedCacheBelow`) and shares the defect class.

## Root Cause, Stated Architecturally

The transcript pipeline loads two independent resources:

- **Structure**: `render_state.rows`, server-derived, delivered by the stream.
- **Bodies**: committed message bodies, loaded by HTTP history pagination.

Pagination has always relied on an *implicit* contract: **structure is total,
bodies gate visibility** — a row whose body is not loaded is skipped, and
lights up when the body pages in. Windowed resume made structure *partial*,
but nothing owns the invariant that structure must cover whatever bodies are
loaded, and the snapshot identity (`based_on_seq` alone) cannot even express a
change in structure coverage.

This is not a bug in any one component; it is a missing invariant plus an
identity that lost a dimension. The fix makes both explicit.

## Design Invariant (new, explicit)

> **The render window must cover the loaded committed-body range**:
> `render_state.window.floor_seq <= min(seq of loaded committed bodies)`,
> for every rendered thread.

Full-structure threads (`floor_seq = 0`) are the trivial case. Pagination
extends bodies downward; the window must follow. The display layer keeps its
existing contract untouched: dumb-render `render_state.rows`, bodies gate
visibility.

## Protocol Design

### 1. Snapshot identity is `(based_on_seq, floor_seq)`

`based_on_seq` alone stopped being a complete identity the moment windows
existed: the same committed tail with a different floor is a *different*
snapshot. The frontier accept rule becomes:

| condition | verdict |
|---|---|
| `based_on_seq < current` | reject (stale) |
| `based_on_seq > current` | accept, changed (server authoritative, any floor) |
| `based_on_seq == current`, `floor_seq != current floor` | accept, changed (expansion, or server re-degrade) |
| `based_on_seq == current`, `floor_seq == current floor` | accept, unchanged (idempotent redelivery) |

The frontier comment "an equal cursor means an equal snapshot" is replaced by
"an equal `(cursor, floor)` means an equal snapshot" — the honesty of the
dedup gate is restored, not weakened.

`ThreadFrontier` therefore tracks the floor of the *accepted snapshot* (today
`renderFloor` on the frontier is set externally; it becomes part of
`acceptRender(basedOnSeq, floorSeq)`).

### 2. Floor ownership: renderer requests, server decides, hub carries

- **Renderer (transcript lifecycle)** is the floor *requester*: it declares
  the floor it needs, derived from what it renders (see §4).
- **Server** is the floor *decider*: a within-budget resume honors the
  requested floor (this already works — `finalize_thread_stream_replay`
  derives the snapshot at `options.render_floor`, and
  `render_snapshot_in_window` / `render_snapshot_at_seq` already implement
  both shapes); an over-budget gap still degrades to the cold-open floor
  regardless of the request (server authority, unchanged).
- **Hub** is a dumb carrier. The upward-only pin
  (`Math.max(input.renderFloor, existing.lastFloor)`) is removed: `lastFloor`
  is simply the last renderer-declared value, overwritten by the
  server-announced floor (`onWindowFloor`) so mid-session reconnects resume
  the *server-decided* window. No monotonicity policy lives in the hub.

### 3. Expansion = a reconnect with a lower `render_floor`

`render_floor` is a connection parameter; changing the window means a new
connection. This reuses the entire existing machinery — no new channel, no new
server-side state, no in-connection control messages:

- The hub's `start()` already aborts the existing connection and resumes with
  `afterSeq = max(input.afterSeq, existing.lastSeq)`, so committed-event
  continuity is preserved across the swap.
- A caught-up resume with a lower floor costs exactly one snapshot-only frame
  (`events: []`) whose rows are derived by the existing
  `render_snapshot_in_window(newFloor, tail)`. Expansion frames are *not*
  `replay: "windowed"` degrades, so `dropCommittedBelow` does not fire.
- Delta-mode connections reseed their delta base from the full snapshot frame
  (existing rule: every full `render_state` frame reseeds the base), so the
  rows-hash chain stays honest across expansion.

### 4. Expansion trigger (desktop)

The transcript lifecycle already owns both ends of the loop: it runs
`loadOlderThreadHistoryPage` and it starts streams with a pinned floor. After
an older page applies (and after any frame that raises the floor, i.e. a
degrade), it evaluates:

```
targetFloor = min(earliest loaded committed-body seq, current floor_seq)
if targetFloor < current floor_seq: restart stream with renderFloor = targetFloor
```

Properties:

- **Bounded**: the target is derived from *loaded bodies*, so each expansion
  step is page-sized. The window never speculatively jumps to 0; when
  pagination is exhausted the earliest body seq is the ledger head and the
  window converges to (effectively) full. Server-side derivation cost per
  expansion is proportional to the loaded range, matching the pagination
  budget the user already paid.
- **Loop-free**: at most one in-flight expansion per thread; re-evaluation
  happens only on trigger events (page apply / frame apply), never on timers.
  If the server answers with a higher floor than requested (an over-budget
  gap degraded the resume again), the client accepts the verdict; the very
  next expansion attempt is caught-up by construction and succeeds. Guard:
  don't re-issue while a request for the same-or-lower floor is in flight.
- **Invariant-restoring**: the trigger is precisely the invariant from the
  Design Invariant section, evaluated at the two points where either side of
  the inequality can change.

"Earliest loaded committed-body seq" means the minimum `seq` across the
thread's loaded committed bodies (paged history entries carry their record
seq, stamped at the wire boundary). The exact accessor lives on the transcript
cache next to `getHistoryPagination()`.

### 5. Server

Expected to need **no production Rust change** — within-budget resumes already
honor the requested floor, snapshot-only caught-up frames already exist, and
the degrade path already asserts server authority. The implementation must
*verify* this with contract tests rather than assume it:

- Caught-up resume with `render_floor` lower than the previous connection's
  floor → snapshot-only frame, same `based_on_seq`, `window.floor_seq` equals
  the requested floor, rows cover the wider window.
- Within-budget gap resume with a lower requested floor → verbatim event
  replay plus a snapshot at the requested floor.
- Over-budget gap resume with a lower requested floor → still degrades to the
  cold-open window (request does not override the budget).

If any of these fail, the fix belongs in `build_thread_stream_replay` /
`finalize_thread_stream_replay` — not in a client workaround.

### 6. UI layer: zero changes

`buildThreadViewRows` / `buildThreadViewRowsWithLocalUsers` and the scroll /
prepend-anchor machinery stay exactly as they are. Restoring the
structure-⊇-bodies invariant upstream makes paged bodies light up through the
existing "skip rows with missing bodies" behavior. This is the litmus test of
the design: the display contract never needed to change, only the coverage
guarantee feeding it.

## iOS (phase 2, same contract)

iOS shares the architecture and the defect class:
`GaryxTranscriptSyncPlanner` pins `renderFloor` on resume, and its snapshot
acceptance keys on `based_on_seq`. Adoption mirrors the desktop changes inside
`GaryxMobileCore` with SwiftPM tests:

- Snapshot identity `(based_on_seq, floor_seq)` in the render-state acceptance
  path.
- Sync planner: floor follows the renderer's declaration downward; server
  announcement overrides.
- Expansion trigger wired to iOS history paging (same invariant, same
  trigger points). The view-layer `GaryxTurnRowsWindowPlanner` (row-count
  window over prepared rows) is orthogonal and untouched.

Phasing: desktop ships first (the reported P1 surface); iOS follows as its own
task against this design. The wire contract is identical, so no gateway
changes sit between the phases.

## Contract Documentation

`docs/agents/repository-contracts.md` (Transcript Rendering) gains two
sentences: snapshot identity is `(based_on_seq, floor_seq)`; clients that
narrow structure via `render_floor` own the invariant that the window covers
their loaded committed bodies, and widen it by resuming with a lower
`render_floor`. `AGENTS.md`/`CLAUDE.md` mirror-sync applies.

## Alternatives Rejected

- **History pages return row fragments; client stitches structure.** The
  client would re-derive/merge transcript structure (turn grouping across
  page boundaries), violating the server-render-state contract. Rejected.
- **In-connection floor control (HTTP side channel + connection id).** A new
  stateful channel for a low-frequency, scroll-driven operation; reconnect
  already exists, is cheap when caught up, and keeps floor a plain connection
  parameter. Rejected.
- **Stop pinning the floor (resume full every reconnect).** Resurrects the
  full-transcript stall on huge threads that windowed resume exists to
  prevent (the hub comment documents this exact regression). Rejected.
- **Client-synthesized rows for pre-floor bodies.** Reimplements user-turn
  grouping locally; forbidden by the transcript-rendering contract. Rejected.
- **Snapshot identity via `rows_hash` instead of `floor_seq`.** Hash-as-identity
  would also catch expansion, but it turns an explainable ordering rule into
  an opaque equality check and cannot distinguish "wider" from "different".
  `floor_seq` is the dimension that actually changed; use it. (`rows_hash`
  remains the delta-chain integrity token, unchanged.)

## Test Plan

- **Gateway (`routes/tests.rs`)**: the three contract tests from §5.
- **Frontier unit tests**: the full identity matrix from §1, including
  re-degrade (floor rises at same tail — accepted, changed) and idempotent
  redelivery (unchanged).
- **Hub unit tests**: floor follows renderer declaration downward; server
  announcement overrides; reconnect carries the last floor; `restartAll`
  preserves it.
- **Mirror + lifecycle integration (headless, no UI)**: the reporter's
  counterexample as a regression test — windowed degrade at floor F with
  bodies seq < F already cached, page older history, assert an expansion
  restart is issued with the lower floor, apply the wider snapshot-only frame
  (same `based_on_seq`), assert `buildThreadViewRows` now yields the pre-floor
  turns. Plus the loop-free guard: degrade response to an expansion request
  does not re-trigger without a new page/frame event.
- **iOS phase**: SwiftPM tests mirroring the frontier matrix and the planner
  trigger; no app-target UI tests required (Core-first rule).

## Open Questions

None blocking. One deliberate scope cut: automatic *re-shrinking* of the
window (memory pressure on very long sessions) is out of scope — the existing
mirror LRU already bounds per-thread retained state, and shrinking would
reintroduce the identity problem in the other direction without a driving
defect.
