# Pinned Thread Drag Reorder (Mobile-First)

Status: draft for design review
Scope: gateway + iOS (phase 1), Mac desktop (phase 2)

## 1. Problem And Goals

Users cannot reorder pinned threads. Pin order is server-decided
(`pinned_at DESC`), so the only way to move a thread up is unpin + repin.

Goals, in priority order:

1. Drag-to-reorder inside the pinned area of the iOS home list (mobile-first).
2. The interaction must feel native and smooth: system reorder animation,
   haptics, no dropped frames.
3. **Zero flicker from server round-trips.** The local order the user just
   created is authoritative on screen. Background refresh loops (10s/15s),
   pin-write responses, and reorder-write responses must never visibly
   reshuffle the pinned section back and forth. "Backend not settled yet" is
   never a visible state.
4. Same capability on Mac desktop afterwards, reusing the same wire contract.

Non-goals:

- Reordering the Recent section (stays server recency order).
- Cross-section drag (dragging a pinned row into Recent is not an unpin
  gesture; it clamps back into the pinned segment).
- Legacy-gateway compatibility shims (repo policy: desktop/gateway ship
  together; a reorder call failing against an old gateway just follows the
  normal failure path in §6).

## 2. Current State (verified)

Gateway:

- Pins live in a dedicated direct-write table `thread_pins
  (thread_id TEXT PRIMARY KEY, pinned_at TEXT NOT NULL) STRICT`
  (`garyx-gateway/src/garyx_db/mod.rs:2005`). Not a projection of
  `thread_records`; not a `recent_threads` column. Archive deletes the pin in
  the same transaction (`archive_thread_record`).
- `list_pinned_threads` orders by `pinned_at DESC, thread_id ASC`
  (`mod.rs:463`). There is no user-defined order concept.
- HTTP: `GET /api/thread-pins`, `PUT /api/thread-pins/{key}` (pin),
  `DELETE /api/thread-pins/{key}` (unpin) — handlers in
  `garyx-gateway/src/routes.rs:1660-1747`, routes in `route_graph.rs:62-85`.
  All write responses return the full pins page
  `{ thread_ids: [...], pins: [{thread_id, pinned_at}] }`
  (`thread_pins_payload`, `routes.rs:1449`).
- No SSE/push carries pins or recent-threads; clients poll.

iOS:

- Home list is a native SwiftUI `List` with one flat `ForEach(items)` — the
  pinned header, pinned rows, spacer, recent header, recent rows are flat
  items with one stable identity space (`thread:<id>`), so pin/unpin animates
  as a *move*, not delete+insert
  (`GaryxMobileSidebarViews.swift:192-283`,
  `GaryxHomeThreadListLayout.primaryItems`,
  `GaryxHomeThreadListPresentation.swift:338-355`).
- Pinned order = `pinnedThreadIds` array order = server pins-page order.
  Sections built by `GaryxHomeThreadSectionsBuilder.build`
  (`GaryxHomeThreadListPresentation.swift:670-715`).
- Optimistic pin/unpin already exists: `GaryxHomeThreadTransitionState`
  overlays a sequence-stamped pin transition until the remote write resolves,
  with rollback restoration (`GaryxHomeThreadListPresentation.swift:559-607`,
  `GaryxMobileModel+ThreadPersistence.swift:63-159`).
- Refresh loops (`runSilentSidebarRefreshLoop` ~10s, reconcile ~15s, plus
  every user action) re-fetch `listRecentThreads` + `listThreadPins`
  concurrently and re-apply `applyPinnedThreadIds(server ids)`
  (`GaryxMobileModel+ThreadList.swift:20-243`). This is the flicker source to
  defend against: without protection, an optimistic reorder would be
  overwritten by the next poll reply.
- No drag/onMove/EditMode code exists anywhere in the list today.
- Publication path: `HomeProjectionActor`/`HomeProjectionReducer` →
  `GaryxHomeThreadListStore.apply(actorSnapshot:)` → transition-state overlay
  in `presentationSnapshot`.

Desktop:

- `PinnedThreadsSidebar.tsx` renders `pinnedThreadRows` in
  `desktopState.pinnedThreadIds` order; main-process store fills it from
  `GET /api/thread-pins` (`main/garyx-client/threads.ts:1111-1156`,
  `main/store.ts:769`). No client-side re-sort.
- `@dnd-kit/{core,sortable,modifiers,utilities}` already installed; the
  ComposerQueue (`ComposerQueue.tsx`) is a working vertical-sortable
  precedent.

## 3. Design Overview

One new ordering column + one new collection-level reorder endpoint on the
gateway; a local-order-authority state machine in `GaryxMobileCore` that makes
the client the order oracle whenever the user has an unsettled reorder; native
`List` reorder for the gesture so the animation is the system one.

Order semantics: **the pinned order is a user-managed list.** `pinned_at`
stays as "when it was pinned" metadata; ordering is carried exclusively by a
new `sort_order`. New pins insert at the head (matches today's optimistic
head-insert on both clients).

## 4. Gateway Changes

### 4.1 Schema

- `thread_pins` gains `sort_order INTEGER` (nullable at the column level to
  satisfy `ALTER TABLE ... ADD COLUMN` on a STRICT table; treated as NOT NULL
  by the write paths).
- One-shot, versioned migration in the existing migration flow: backfill
  `sort_order = 0..n-1` following the current display order
  (`pinned_at DESC, thread_id ASC`), so the visible order does not change on
  upgrade.

### 4.2 Read/write paths (`garyx_db/mod.rs`)

- `list_pinned_threads`: `ORDER BY sort_order ASC, pinned_at DESC,
  thread_id ASC` (the trailing keys only break ties if a NULL/duplicate ever
  slips in; steady state is unique 0..n-1).
- `pin_thread`: new pin gets `sort_order = MIN(sort_order) - 1` (head), in the
  same upsert transaction. Re-pinning an already-pinned thread keeps its
  current `sort_order` (idempotent, no jump).
- `unpin_thread`: unchanged (row delete). No renumbering needed; gaps are
  fine.
- New `reorder_thread_pins(ordered_ids: &[String])`, single transaction:
  - For ids in the request that exist in `thread_pins`: assign
    `sort_order = 0,1,2,...` following request sequence.
  - Existing pins *not* mentioned in the request: placed after the mentioned
    ones, preserving their current relative order (renumbered to follow).
  - Ids in the request that are not pinned: ignored. **This endpoint never
    changes membership** — it cannot pin or unpin.
  - Returns the full ordered pins list.

### 4.3 HTTP API

- `PUT /api/thread-pins` (collection PUT, next to the existing routes in
  `route_graph.rs`), body `{"thread_ids": ["...", ...]}`.
- Response: the standard pins page (`thread_pins_payload`), now the
  authoritative post-reorder order. `pins` entries additionally expose
  `sort_order` (additive field; existing clients only read `thread_ids`).
- Validation: body must be a non-empty array of strings; malformed → 400.
  Unknown/unpinned ids are ignored per §4.2 (not an error — this makes the
  call race-tolerant against concurrent unpin from another device).
- Concurrency: SQLite serialized writes make interleaved pin/unpin/reorder
  transactions atomic; last writer wins on order, membership always reflects
  the pin/unpin calls.

### 4.4 Gateway tests (`cargo test -p garyx-gateway --lib`)

- Migration backfills existing rows to the previous visible order.
- Pin inserts at head; re-pin keeps position; unpin leaves order of the rest.
- Reorder: full permutation; subset (unmentioned keep relative order, after
  mentioned); unknown ids ignored; empty body rejected; response order equals
  subsequent `GET`.
- Archive still removes the pin row.

## 5. iOS Changes (Phase 1 core)

### 5.1 Gesture: native `List` reorder

Primary approach: attach `.onMove` to the existing flat `ForEach(items)` and
gate which rows participate:

- `.moveDisabled(...)` on every non-pinned-thread item (headers, spacer,
  recent rows, placeholders) so only pinned rows lift.
- The `onMove` handler receives indices in the flat items array; translate to
  pinned-section-relative indices and **clamp the destination into the pinned
  segment** (drop past the segment edge lands at the edge). The handler emits
  a pure Core command (`movePinned(from:to:)`), never mutates view state
  directly.
- No `EditMode` UI: the target interaction is long-press-lift then drag, with
  the system reorder animation (List is UICollectionView-backed, so lift,
  gap-opening, and drop-settle animations come from UIKit for free — this is
  the "silky" path; hand-rolled gesture reordering is explicitly rejected).
- Haptics: system drag lift provides feedback; add `.sensoryFeedback` (or
  `UIImpactFeedbackGenerator` fallback) on a completed reorder drop.

**Spike gate (first commit of the batch):** verify on simulator that plain
(non-edit-mode) `List` + `onMove` + `moveDisabled` yields long-press drag
reorder with correct clamping on the current deployment target, and that the
existing pin/unpin *move* animation (one identity space) is not regressed. If
plain-List long-press reorder does not engage without EditMode, fallback in
order of preference: (a) keep one `List`, wrap only the pinned rows in their
own `ForEach` carrying `onMove` (must re-verify pin/unpin still animates as a
move across the two `ForEach`es); (b) a drag-handle affordance revealed by
long-press. Do not proceed to the state-machine wiring until the spike commit
demonstrates the gesture.

### 5.2 Local order authority (Core state machine, the anti-flicker core)

New pure state in `GaryxMobileCore` (either extending
`GaryxHomeThreadTransitionState` or a sibling `GaryxPinnedOrderState` owned by
`GaryxHomeThreadListStore` — implementer's choice, but it must be a pure,
SwiftPM-testable value type). Rules:

- **R1 — freeze during drag.** While a drag session is active, the store
  buffers (latest-wins) incoming snapshots instead of publishing them; the
  buffered snapshot is applied after the drop, with the reorder overlay
  applied on top. Rows must never shift under the user's finger.
- **R2 — commit on drop.** The drop immediately commits the new local pinned
  order (sequence-stamped intent), calls
  `recentThreadFeeds.noteLocalMutation()`, and fires the async
  `PUT /api/thread-pins`. The only visible motion is the system drop-settle.
- **R3 — local order wins while unsettled.** While any reorder intent is
  in flight or unacknowledged, every incoming pins page (poll loops, pin/unpin
  write responses) is merged as: membership from server (new pins enter at
  head; unpinned ids drop out), **order of surviving ids stays local**. The
  server's order is ignored.
- **R4 — settle without motion.** When the reorder response for the latest
  intent arrives, resolve the intent. Since the server applied our order, the
  presented order does not change — settling is invisible. Only when there is
  no pending/unacked intent does a server pins page order apply directly
  (cold start, other-device changes).
- **R5 — failure keeps the local order.** A failed reorder write never snaps
  the UI back. Keep the local order, retry silently (retry on the next poll
  tick, capped ×3 per intent; a newer intent supersedes retries of an older
  one). After retries are exhausted, keep the local order for the session and
  log; the next successful reorder or a cold reload converges. Cross-device
  conflicts are last-writer-wins by design.

`applyPinnedThreadIds` and `commitRefreshedRecentThreadsPage`
(`GaryxMobileModel+ThreadList.swift`) route through this state instead of
overwriting `pinnedThreadIds` unconditionally.

### 5.3 Files and tests

- Core: state machine + move/clamp computation next to
  `GaryxHomeThreadListPresentation.swift`; new files need `xcodegen generate`
  with the regenerated `pbxproj` committed, and an `xcodebuild` compile check
  (SwiftPM green alone is not proof the app target sees the files).
- SwiftPM tests (`Tests/GaryxMobileCoreTests/`), no-UI first per repo rule:
  - move/clamp: flat-index → pinned-relative translation, edge clamps,
    single-item and top/bottom moves;
  - R1: snapshots buffered during drag, latest-wins, replayed after drop;
  - R2/R4: intent lifecycle — commit, ack settle is order-identical
    (assert no row-order delta on ack);
  - R3: poll page during in-flight reorder merges membership but preserves
    local order (including simultaneous other-device pin add + local
    reorder);
  - R5: failure retention, capped retry, supersede-by-newer-intent;
  - regression: pin/unpin optimistic transition still behaves (existing
    tests stay green).
- Manual simulator pass only for gesture *feel* (lift, gap, settle, haptic),
  not as the acceptance for ordering logic.

## 6. Desktop Changes (Phase 2)

- `PinnedThreadsSidebar` becomes a `DndContext` + `SortableContext`
  (`verticalListSortingStrategy`, `restrictToVerticalAxis`) following the
  ComposerQueue precedent; row lift/drop uses dnd-kit's transform animations.
- On drop: optimistic reorder of `desktopState.pinnedThreadIds` in the
  renderer, then preload → main `setThreadPinOrder(threadIds)` → gateway
  `PUT /api/thread-pins`.
- The main-process store gets the same unsettled-intent guard: while a reorder
  is in flight/unacked, `fetchThreadPins` refresh results merge membership
  only and keep the local order (mirror of R3/R4/R5, in the store where the
  poll overwrite happens today).
- Contract additions in `shared/contracts` + unit tests via
  `npm run test:unit` (not bare `node --test`, per repo test-runner rule).

## 7. Failure / Race Matrix (summary)

| Scenario | Behavior |
| --- | --- |
| Poll reply lands mid-drag | Buffered; applied after drop under overlay (R1) |
| Poll reply lands while reorder in flight | Membership merged, local order kept (R3) |
| Reorder ack | Invisible settle; no reshuffle (R4) |
| Reorder HTTP failure / old gateway | Local order kept; capped silent retry (R5) |
| Concurrent unpin on another device | Id drops out of local order on next merge |
| Concurrent pin on another device | Id enters at head; local relative order kept |
| Two devices reorder | Last writer wins at the gateway |
| Drop outside pinned segment | Clamped to segment edge; never unpins |

## 8. Delivery Batches

1. **B1 gateway**: schema + migration + `reorder_thread_pins` + `PUT
   /api/thread-pins` + tests.
2. **B2 iOS**: spike commit (gesture proof) → Core state machine + tests →
   wiring + haptics. Ships with B1 in the same merge train (mobile-first
   deliverable).
3. **B3 desktop**: dnd-kit reorder + store guard + tests. Separate follow-up
   task after B1/B2 land.

## 9. Decisions Taken (review these explicitly)

1. Order carried by a dedicated `sort_order` on the direct-write `thread_pins`
   table — *not* a `thread_records` body field or `recent_threads` projection
   column, because pins are deliberately outside the projection contract.
2. Reorder endpoint never mutates membership; unknown ids ignored (race
   tolerance) rather than rejected.
3. Reorder failure does **not** roll back the UI (unlike pin/unpin). Rationale:
   the user's explicit spatial arrangement snapping back reads as flicker/data
   loss; order divergence is low-stakes and converges on the next write.
4. Recent-section ordering untouched; no cross-section drag semantics.
5. Native `List`/`onMove` over custom gestures, gated by a mandatory spike.
