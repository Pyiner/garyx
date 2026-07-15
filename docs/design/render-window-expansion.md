# Render Window Expansion: History Below the Windowed-Resume Floor

Status: v4 (revised after design review #TASK-2299 v3 FAIL; re-review pending)
Related: `thread-render-frame-incremental.md` (knife 2: over-budget degrade),
`thread-open-replay-trim-design.md` (floor-windowed rows),
`perf-thread-stream-replay-degrade.md`.

v4 changes after v3 review: stale-request frames are no longer dropped whole
— `streamRequestId` gates only connection-scoped semantics (render state,
windowed marker, errors, pending settle) while committed events are
ledger-scoped and always applied idempotently, closing the deterministic
event-loss trace (hub cursor advances at `sendEvent`, so a dropped event is
never replayed); the start gate defines the normal-plan fallback when
reconcile returns null and pins joins as new logical requests; a pending
rebind is counted as the new epoch's first consumed attempt
(`retryCount = 1`); cache-restored snapshots seed `effectiveFloor` (but never
settle a pending); physical generations are per-SSE-attempt (the hub retry
loop shares one controller across attempts, so controller identity alone
cannot distinguish them); the perf contract gets concrete gating criteria
(rows/bytes gating, duration informational).

v3 changes: logical `streamRequestId` across the IPC boundary; formal
anti-loop state (`demandEpoch`, `retryCount`) with demand-convergent
semantics (no timers); capped success-gated prepay replacing unbounded
exponential widening; single start plan; pending transitions enumerated;
`rows_hash` excluded from value comparison.

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
   to hang on, so they never render.
5. Even if a wider snapshot arrived at the same `based_on_seq`, the frontier
   returns `changed: false` for an equal cursor and `mirror.applyFrame` gates
   on `accepted && changed`, so it would be silently dropped.

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

iOS does own that obligation (see Prior Art); desktop never adopted it. This
design closes the gap with the same model, hardened where the desktop's
multi-consumer streams and main/renderer process split need more than iOS
does.

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
  not just the cursor.
- Every full `render_state` frame reseeds downstream state unconditionally
  (`GatewayStreamActor`).

Desktop aligns with this model. Where this design goes beyond what iOS
currently has (request correlation, retry budget, bounded widening), a
follow-up iOS audit is listed at the end — gated on a reproduced iOS failure,
per review policy.

## Design Invariant (explicit, demand-convergent)

> **Demand-convergence property**: every qualifying external demand trigger
> (§5) either restores `effective floor_seq <= min(seq of loaded committed
> bodies)` (or reaches the full window, `floor_seq = 0`) within one expansion
> round trip, or — when >1 MiB gaps race in repeatedly — leaves the system
> held with an intact retry budget for the *next* demand trigger.

This is deliberately **weaker than unconditional quiescent convergence**:
with no timers in the design, a thread that degrades twice in a row and then
goes quiet stays at the degraded window until the user next interacts with it
(scrolls, reopens, pages). That is an explicit product trade-off: the stuck
state is only reachable through repeated >1 MiB bursts *during* an expansion,
and the very next user interaction repairs it. The inequality is also
transiently violated between an older-page apply and the expansion frame's
arrival; that window is one round trip.

**"Loaded committed bodies" is defined against the store the UI actually
renders from**: the transcript cache's `uiMessages` — the same store
`ThreadPage`'s `messagesBySeq` is built from — restricted to entries carrying
a finite positive record `seq` (remote-final committed bodies; local
optimistic rows without a seq are excluded). It is explicitly NOT
`recordsBySeq`: pagination writes `uiMessages` only, and the windowed-degrade
`dropCommittedBelow` clears `recordsBySeq` only, so the two stores
legitimately diverge. The accessor (`earliestLoadedCommittedBodySeq()`) lives
on `ThreadTranscriptCache` next to `getHistoryPagination()`.

## Protocol Design

### 1. Snapshot acceptance: ordering gate + value change detection

Two orthogonal concerns, currently conflated in the frontier, get separated:

- **Ordering (frontier)**: a full `render_state` frame is *stale* iff
  `based_on_seq < current`. Everything at `based_on_seq >= current` is
  accepted — including same-seq frames. This is required independently of
  expansion: the server legitimately emits **same-seq overwrite frames** (the
  stream doc comment: "same-seq overwrite events still reach clients", with
  existing gateway tests), so "equal cursor means equal snapshot" was never a
  valid protocol assumption. A tuple identity `(based_on_seq, floor_seq)`
  would repeat the mistake one dimension later; rejected.
- **Change detection (mirror)**: whether an accepted snapshot *replaces* the
  held one is decided by structural equality on the full `render_state` —
  rows, scalars, window — **ignoring `rows_hash`**, which is a delta-chain
  transport token, not render content (it is absent on plain connections and
  must not make otherwise-identical snapshots compare unequal). Unchanged
  snapshots skip `setRenderState`, preserving downstream memo/reference
  stability exactly as today's `changed: false` path does.

`ThreadFrontier.acceptRender(basedOnSeq)` keeps only the ordering rule.
`ThreadFrontier.setRenderFloor` has no production callers today (dead code)
and is removed; floor state is tracked where it is owned (§4).

### 2. Request correlation across the IPC boundary: `streamRequestId`

Two distinct identities, one per layer, correlated by a token that rides the
**local** event envelope (the gateway wire is untouched):

- **Physical `connectionGeneration` (hub-internal)**: each SSE attempt inside
  the hub's retry loop gets its own generation number. The retry loop shares
  one `AbortController` across successive attempts, so controller identity
  alone cannot distinguish attempt N from N+1; callbacks (`sendEvent`
  forwarding, `onCommittedSeq`, `onWindowFloor`) capture their attempt's
  generation and are guarded by "this generation is still the forwarder's
  current one AND this controller is still the thread's current forwarder".
  This kills both post-abort callbacks and cross-attempt stragglers.
  `restartAll` and hub-internal reconnects create new physical attempts under
  the *same* logical request (below).
- **Logical `streamRequestId` (renderer-assigned)**: every lifecycle-issued
  start carries a fresh opaque request id in `StartThreadStreamInput`; the
  hub stores it on the forwarder and stamps it on every
  `DesktopChatStreamEvent` it forwards for that thread. Hub-internal physical
  reconnects and `restartAll` preserve the forwarder's current
  `streamRequestId` — the renderer's logical request survives transport
  churn.

Renderer-side rules built on the token — **the request id gates
connection-scoped semantics only; it never filters ledger-scoped data**:

- **Committed events are ledger-scoped and always applied**, whatever request
  id their frame carries. They are seq-keyed and idempotent in the cache, and
  same-seq rewrite/reset controls must keep triggering the authoritative
  refetch path. Dropping them is not an option: the hub's committed cursor
  advances at `sendEvent` time (before the renderer consumes the frame), a
  superseding start resumes from `max(input.afterSeq, lastSeq)`, and the
  server only replays `seq > after_seq` — so an event dropped from a stale
  queued frame is deterministically never redelivered (and a same-seq
  overwrite equal to `after_seq` would not be replayed even by a
  cursor-rewind scheme). A stale frame therefore applies as an
  **event-only mirror commit**.
- **Connection-scoped parts of a stale-request frame are dropped**: its
  `render_state` (a same-`based_on_seq` different-value snapshot from the old
  narrow window would pass the §1 value gate and re-narrow the window), its
  `replay: "windowed"` marker (a stale marker must not fire
  `dropCommittedBelow`), its error signals, and any pending-settle effect.
  Structure for the thread comes from the current request's snapshot.
- `pendingExpansion` (§4) settles **only** on an authoritative stream frame
  that carries the pending request's id, holds a full `render_state`, and
  passes the ordering gate. Cache-restored or locally synthesized snapshots
  never settle a pending (they may seed `effectiveFloor`, §4).

IPC surface changes: `StartThreadStreamInput.requestId` (opaque string) and a
`requestId` field on the locally-forwarded stream event envelope. `start`
IPC stays `Promise<void>`; no attempt information needs to flow back because
the correlation is carried by the events themselves.

### 3. Single start gate: reconcile returns a plan

All stream starts for a thread — selected consumer, side-chat consumer,
post-refetch restart — go through **one lifecycle gate** per thread, which
executes exactly one `start()`:

```
gate(thread, consumerStart):
  seedEffectiveFloorIfCold(thread)                    # §4
  expansionPlan = reconcile(thread)                   # may be null
  if expansionPlan == null and not consumerStart.mustEstablishStream:
    return                                            # nothing to do
  plan = expansionPlan
       ?? normalPlan(afterSeq: committedCursor,
                     renderFloor: pendingExpansion?.targetFloor
                                  ?? effectiveFloor,
                     requestId: fresh())
  execute start(plan)                                 # exactly one
```

`reconcile` returning null (full window, invariant satisfied, or budget held)
therefore still yields a **normal plan** for any consumer start that must
establish or join the stream — a new thread, a full-window thread, or a plain
reopen starts its stream exactly as today, at the current effective floor.
Consumers never compute their own floor and never race reconcile with their
own start.

**Joins are new logical requests.** A second owner joining a thread's stream
goes through the gate and gets a fresh `requestId`; the hub keeps today's
semantics (abort + restart with merged owners) — there is no silent
no-reconnect join, so no dangling generation. If a `pendingExpansion` is in
flight, the gate **rebinds** it to the new request id atomically and the plan
adopts `pendingExpansion.targetFloor` as its floor, so a joined start can
never raise an unmet request.

**Rebind retry accounting**: a consumer start is a demand trigger (§5) — it
bumps `demandEpoch` and resets `retryCount` — and when it rebinds an in-flight
pending, that rebound start is **counted as the new epoch's first consumed
attempt (`retryCount = 1`)**. The per-epoch invariant "attempts ≤ 1 +
RETRY_BUDGET" holds unconditionally: a rebound attempt that settles in a
degrade has exactly one retry left, not a fresh double budget.

`pendingExpansion` lifecycle transitions (exhaustive):

| event | pending transition |
|---|---|
| authoritative full frame, matching request id, passes ordering | settle (§5) |
| lifecycle-issued start via the gate (consumer join/restart, refetch restart) | rebind to the new request id; plan floor = pending target |
| hub-internal reconnect / `restartAll` | untouched (same logical request id) |
| last owner stops the thread stream | cancel pending, reset `retryCount` |
| stream terminal error surfaced to renderer (gap error → authoritative refetch) | cancel pending; the refetch path re-enters the gate, which reconciles fresh |
| `clearThreadTranscript` / mirror LRU eviction of the thread entry | cancel pending, drop all per-thread window state |

### 4. Floor state machine (lifecycle-owned, per thread, formal)

```
effectiveFloor : number        # window.floor_seq ?? 0 of the latest accepted
                               # full frame of the current logical request
                               # (0 = full window; normalized on EVERY full
                               # frame, clearing included — today's
                               # "only fires when positive" onWindowFloor is
                               # replaced by this).
                               # COLD SEED: when no live-stream frame has
                               # been accepted yet this session (no active
                               # request, no pending), a cache-restored
                               # snapshot seeds effectiveFloor from its
                               # window.floor_seq ?? 0 — the gate runs this
                               # seed before its first reconcile. Without it
                               # a cold restore at floor F would read as
                               # "full window" and the first consumer start
                               # would request floor 0: exactly the
                               # full-thread snapshot this design avoids.
                               # Seeding never settles a pending.
neededFloor    : derived       # min(earliestLoadedCommittedBodySeq ?? +inf,
                               #     effectiveFloor)
pendingExpansion : { requestId, targetFloor } | null   # at most one
demandEpoch    : counter       # bumped by external demand triggers only
retryCount     : number        # per-epoch expansion attempts after a failed
                               # settle; reset on demandEpoch bump
prepayMargin   : number        # speculative widening span in records; grows
                               # only on successful settles; capped (§6)
```

`desiredFloor` — the floor any start plan carries — is now a *defined*
projection of this state, not a free-floating term:

```
desiredFloor = pendingExpansion ? pendingExpansion.targetFloor
             : expansionWarranted() ? expansionTarget()      # §6
             : effectiveFloor
```

### 5. Reconcile, demand triggers, and the retry budget

**Demand triggers** (bump `demandEpoch`, reset `retryCount`, then run
reconcile): older-history page apply; cache-restored transcript ingest;
authoritative-refetch transcript apply; a consumer start entering the gate
(thread open/reopen, side-chat join — with the §3 rebind rule: when the start
rebinds an in-flight pending, the rebound attempt immediately consumes the
new epoch's first slot, `retryCount = 1`).

**Settle triggers** (run reconcile *without* bumping the epoch): every
accepted full `render_state` frame. A frame updates `effectiveFloor`, settles
a matching pending, but never refills the retry budget — the v2 flaw where
"degrade frame → reconcile → re-issue" looped forever is structurally
impossible because re-issuing consumes `retryCount`, which only a demand
trigger resets.

```
reconcile(thread) -> plan | null:
  if pendingExpansion != null: return null          # settle first
  if effectiveFloor == 0:      return null          # full window
  if neededFloor >= effectiveFloor: return null     # invariant holds
  if retryCount >= 1 + RETRY_BUDGET: return null    # held until next epoch
  retryCount += 1
  target = expansionTarget(thread)                  # §6
  pendingExpansion = { requestId: fresh(), targetFloor: target }
  return { afterSeq: committedCursor, renderFloor: target,
           requestId: pendingExpansion.requestId }
```

with `RETRY_BUDGET = 1`: the first attempt per epoch plus one retry. On
settle, if `effectiveFloor <= targetFloor` the expansion succeeded
(`prepayMargin` may grow, §6); if the resume re-degraded
(`effectiveFloor > targetFloor`), reconcile runs again as a settle trigger
and either spends the retry (which is caught-up by construction — the degrade
just replayed the window to the tail — so it fails only if *another* >1 MiB
burst lands mid-flight) or holds. Held threads repair on the next demand
trigger; see the invariant section for why this weakened semantics is the
accepted trade-off. No timers exist anywhere in the design.

### 6. Expansion target: demand plus capped, success-gated prepay

Expansion = a reconnect with a lower `render_floor`. `render_floor` is a
connection parameter; changing the window means a new connection, reusing the
existing machinery (hub abort + `afterSeq = max(input, lastSeq)` continuity,
snapshot-only caught-up frames, `render_snapshot_in_window` derivation, delta
base reseeding on every full frame). Sub-budget and caught-up expansion
frames carry no `replay: "windowed"` marker, so `dropCommittedBelow` does not
fire; a resume that hits a fresh over-budget gap degrades again with the
marker — correct server authority, handled by §5.

**Target formula** — demand first, speculation strictly bounded:

```
expansionTarget(thread):
  return max(0, neededFloor - prepayMargin)
```

- `prepayMargin` starts at one history-page span (in records), **doubles only
  after a pending settles successfully at its requested floor**, and is
  **capped at MAX_PREPAY_RECORDS** (implementation constant on the order of
  half the store's tail-cache record budget, e.g. 2048). Failed settles never
  grow it — the v2 formula grew the span *before* the request, so consecutive
  re-degrades inflated the next window as a side effect of failure; that is
  reversed.
- Consequences: the target reaches 0 only when the user's loaded bodies are
  within `MAX_PREPAY_RECORDS` of the ledger head — never as an unbounded
  speculative jump (the v2 counterexample `neededFloor=4999, span=8192 →
  target=0` is impossible). Each expansion's window growth is bounded by
  (new demand + capped prepay), so snapshot payload, row count, and
  derivation input grow by a bounded increment per step.
- Cost model, stated honestly: the router store keeps only a parsed tail
  (8 MiB / 4096 records; 64 MiB store-wide LRU); a floor below the cached
  tail is a full-prefix read+reduce. With capped prepay, a deliberate deep
  scroll costs O(depth / prepayCap) full-prefix derivations — linear with a
  large divisor, each step user-paid and interaction-spread, not the v2
  O(log) claim (which was only achievable with the unbounded jump). The
  **snapshot-only caught-up path bypasses the 1 MiB replay budget entirely**
  (the budget guards replay events, not snapshot derivation), which is
  exactly why the client-side cap is load-bearing and why the perf contract
  below gates the design.
- **Perf contract (gating)**: per-expansion assertions on snapshot serialized
  bytes and row count (bounded by demand + prepay cap) and derivation
  duration; `full_file_reads` (a router-store test counter) as an auxiliary
  bound on read amplification. If measurement shows the bounded-target cost
  is still unacceptable on huge rolled-file threads, a server-side
  backward-derivation reuse (cache or floor-clamping budget on the snapshot
  path) **enters scope as a contingency** — a production Rust change is not
  assumed but is explicitly not ruled out.

### 7. UI layer: zero changes

`buildThreadViewRows` / `buildThreadViewRowsWithLocalUsers` and the scroll /
prepend-anchor machinery stay exactly as they are. Restoring the
structure-⊇-bodies invariant upstream makes paged bodies light up through the
existing "skip rows with missing bodies" behavior. Over-widened windows
(prepay) render exactly like today's normal full-structure threads with
unloaded bodies. The litmus test of the design: the display contract never
changes, only the coverage guarantee feeding it.

## Contract Documentation

`docs/agents/repository-contracts.md` (Transcript Rendering) gains: the
server may emit same-seq overwrite frames, so clients gate render-state
acceptance on cursor ordering and detect change by full-snapshot value
(excluding `rows_hash`), never by cursor equality alone; clients that narrow
structure via `render_floor` own the demand-convergent invariant that the
window covers their loaded committed bodies, and widen it by resuming with a
lower `render_floor`. `AGENTS.md`/`CLAUDE.md` mirror-sync applies.

## Alternatives Rejected

- **History pages return row fragments; client stitches structure.** Client
  re-derives transcript structure; violates the server-render-state contract.
- **In-connection floor control (HTTP side channel + connection id).** A new
  stateful channel for a low-frequency operation; reconnect already exists.
- **Stop pinning the floor (resume full every reconnect).** Resurrects the
  full-transcript stall windowed resume exists to prevent.
- **Client-synthesized rows for pre-floor bodies.** Reimplements user-turn
  grouping locally; forbidden by contract.
- **Tuple identity `(based_on_seq, floor_seq)` (v1).** Refuted by same-seq
  overwrite frames.
- **`rows_hash` as snapshot identity.** Transport integrity token only; also
  excluded from value comparison (§1).
- **Unbounded exponential widening (v2).** Could speculatively request
  floor 0 and regenerate a full-thread snapshot through the budget-exempt
  snapshot-only path; replaced by capped, success-gated prepay.
- **Timer-based retry/backoff for held threads.** Would buy unconditional
  quiescent convergence at the cost of background reconnect churn on threads
  the user is not looking at; the demand-convergent semantics keeps repair
  aligned with actual user attention.

## Test Plan

- **Gateway (`garyx-gateway` `routes/tests.rs`)**:
  - caught-up resume with a lower `render_floor` → snapshot-only frame, same
    `based_on_seq`, `window.floor_seq == requested`, wider rows, no `replay`
    marker;
  - within-budget gap resume with a lower requested floor → verbatim events
    plus snapshot at the requested floor;
  - over-budget gap resume with a lower requested floor → still degrades,
    `replay: "windowed"` present;
  - delta-mode: expansion full frame reseeds the base; next live delta chains
    from the wide snapshot (true downward-expansion chain test).
- **Router store (`garyx-router` store tests — `full_file_reads` is a
  store-internal test counter, so these live here, not in gateway tests)**,
  with a **deterministic fixture and concrete gating criteria**: a rolled-file
  thread of R records (R > tail-cache record budget, e.g. R = 12_000) with
  fixed-size bodies, driven through K successive floor lowerings of one
  prepay-cap span each. Gating assertions (deterministic, CI-safe):
  - `full_file_reads` increases by exactly 1 per below-tail derivation
    (≤ K total);
  - each derived window's row count ≤ (tail − target) and grows by at most
    (demand span + MAX_PREPAY_RECORDS) per step;
  - each snapshot's serialized bytes ≤ ceil(row count × per-row byte bound
    measured from the fixture's own first window) × slack factor 2 — a
    self-calibrated bound, not a machine-dependent constant.
  Derivation wall-time is recorded and reported as an **informational**
  diagnostic only (timing assertions are not CI-gating; the byte/row/read
  bounds are the contract).
- **Frontier unit tests**: ordering matrix — stale rejected; same-seq
  accepted; forward cursor accepted.
- **Mirror change-detection tests** (value gate lives here, not in the
  frontier): same-seq overwrite with different rows → applied; same-seq
  scalar-only change → applied; identical snapshot re-delivery → not applied,
  reference stability preserved; identical snapshot differing only in
  `rows_hash` presence → not applied; same-seq wider window → applied.
- **Hub unit tests**: physical-generation guard (post-abort callbacks
  discarded); `restartAll` and hub-internal reconnects preserve the logical
  `streamRequestId`; events stamped with the forwarder's request id.
- **Lifecycle + mirror integration (headless, no UI)** — anchor regression
  plus the state machine:
  1. `uiMessages` holds bodies seq 1–4; windowed state at floor 3; initial
     rows contain only seq ≥ 3. Older-page apply → gate issues one expansion
     start requesting floor ≤ 1 → full wide frame (same `based_on_seq`, no
     marker, matching request id) applies → `buildThreadViewRows` yields the
     turns owning seq 1–2.
  2. **Stale queued frame (split semantics)**: a floor-3 frame with the
     *old* request id, carrying a unique committed event (a seq the hub
     cursor has passed but the renderer never applied) plus a
     `replay: "windowed"` marker, arrives after the new request's floor-1
     frame. Assert: the event's body is applied exactly once (present, not
     duplicated); the frontier committed cursor advances correctly; the
     stale `render_state` is NOT applied (window stays wide); the stale
     windowed marker does NOT fire `dropCommittedBelow`; a same-seq
     rewrite/reset control carried by a stale frame still triggers the
     authoritative refetch.
  3. **Retry budget**: expansion answered by a degrade → exactly one retry;
     a second degrade → held (no further starts on settle triggers); next
     demand trigger (page apply) issues a fresh attempt. Quiescence during
     hold produces no starts (weakened-invariant semantics pinned by test).
  4. **Consumer join during pending**: side-chat start rebinds pending to the
     new request id; plan floor = pending target; exactly one physical start;
     the joined start cannot raise the floor; `retryCount` is set to 1 —
     "pending join → degrade → one retry → second degrade → held" shows
     total attempts within the per-epoch budget, never a fresh double
     budget.
  4b. **Normal-plan fallback**: consumer start on a new thread / full-window
     thread / invariant-satisfied thread (reconcile null) still issues
     exactly one start at the current effective floor with a fresh request
     id.
  4c. **Cold seed**: cache-restored snapshot at floor 3 with body seq 1
     present → gate seeds `effectiveFloor = 3` before first reconcile; the
     first issued plan is an expansion target ≤ 1 — never floor 0 from a
     "no state" read; the restored snapshot settles no pending.
  5. **Last-owner stop before first frame** → pending cancelled; thread
     reopen reconciles fresh (no permanently-stuck `pending != null`).
  6. **Gap error during pending** → pending cancelled; authoritative refetch
     re-enters the gate and reconciles.
  7. Cache-restore and authoritative-refetch ingests with pre-floor bodies
     trigger reconcile (demand epoch bumps); cache-synthesized snapshots
     never settle a pending.
  8. `uiMessages` vs `recordsBySeq` divergence drives `neededFloor`;
     successful expansion does not drop cached records; a genuine re-degrade
     with marker does.
  9. **Prepay cap**: `neededFloor` just below a huge `effectiveFloor` with a
     large grown `prepayMargin` → target never overshoots
     `neededFloor - MAX_PREPAY_RECORDS`; near the ledger head, target reaches
     0 only when loaded bodies are within the cap; `prepayMargin` unchanged
     after failed settles.
- **iOS**: no changes in this task. Follow-up audit (separate task, gated on
  a reproduced failure): stale-callback guarding around the stop/start pair,
  retry bounds, and expansion-cost behavior of the iOS floor planner.

## Open Questions

None blocking. Deliberate scope cuts: window re-shrinking under memory
pressure (mirror LRU already bounds retained state); iOS hardening (prior art
works today; audit gated on a repro); server-side backward-derivation reuse
(contingency, entered only if the §6 perf contract fails on measurement);
timer-based convergence (rejected above as a product trade-off, revisitable
if the demand-convergent semantics proves insufficient in practice).
