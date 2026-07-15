# Pinned Thread Drag Reorder (Mobile-First)

Status: v2 draft for design review (revised after review round 1, #TASK-2287)
Scope: gateway + iOS (phase 1), Mac desktop (phase 2)

Revision notes: v2 addresses review findings F1-F7 — response-generation
guarding + single-flight desired-order writes (F1), collection revision CAS on
reorder (F2), drag-lifecycle requirements and gesture arbitration as an
architecture gate (F3), durable reorder outbox (F4), corrected STRICT
schema/migration plan and idempotent re-pin (F5), task-forest ordering
consumers (F6), expanded race/test coverage (F7).

## 1. Problem And Goals

Users cannot reorder pinned threads. Pin order is server-decided
(`pinned_at DESC`), so the only way to move a thread up is unpin + repin.

Goals, in priority order:

1. Drag-to-reorder inside the pinned area of the iOS home list (mobile-first).
2. The interaction must feel native and smooth: system reorder animation,
   haptics, no dropped frames.
3. **Zero flicker from server round-trips.** The local order the user just
   created is authoritative on screen. Background refresh loops (10s/15s),
   pin-write responses, reorder-write responses, and *late/stale responses*
   must never visibly reshuffle the pinned section. "Backend not settled yet"
   is never a visible state — including across app restarts.
4. Same capability on Mac desktop afterwards, reusing the same wire contract.

Non-goals:

- Reordering the Recent section (stays server recency order).
- Cross-section drag (dragging a pinned row into Recent is not an unpin
  gesture; it clamps back into the pinned segment).
- Legacy-gateway compatibility shims (repo policy: desktop/gateway ship
  together; a reorder call failing against an old gateway follows the normal
  outbox retry path in §6).

## 2. Current State (verified)

Gateway:

- Pins live in a dedicated direct-write table `thread_pins
  (thread_id TEXT PRIMARY KEY, pinned_at TEXT NOT NULL) STRICT`
  (`garyx-gateway/src/garyx_db/mod.rs:2005`). Not a projection of
  `thread_records`; not a `recent_threads` column. Archive deletes the pin in
  the same transaction (`archive_thread_record`).
- `list_pinned_threads` orders by `pinned_at DESC, thread_id ASC`
  (`mod.rs:463`). There is no user-defined order concept.
- `pin_thread` upserts and refreshes `pinned_at` on re-pin (`mod.rs:481`).
- **task_forest reads `thread_pins` directly** (not via
  `list_pinned_threads`): root rank and skipped-pin order are built from
  `pinned_at` in the forest CTE
  (`garyx-gateway/src/garyx_db/task_forest.rs:592`). Any ordering change must
  cover these read sites too.
- HTTP: `GET /api/thread-pins`, `PUT /api/thread-pins/{key}` (pin),
  `DELETE /api/thread-pins/{key}` (unpin) — handlers in
  `garyx-gateway/src/routes.rs:1660-1747`, routes in `route_graph.rs:62-85`.
  All write responses return the full pins page
  `{ thread_ids: [...], pins: [{thread_id, pinned_at}] }`
  (`thread_pins_payload`, `routes.rs:1449`).
- Schema handling convention: structural `ensure` in
  `initialize_connection` (`mod.rs:2259`) + versioned one-shot startup
  migrations with a durable marker (`mod.rs:901`).
- No SSE/push carries pins or recent-threads; clients poll.

iOS:

- Home list is a native SwiftUI `List` with one flat `ForEach(items)` — the
  pinned header, pinned rows, spacer, recent header, recent rows are flat
  items with one stable identity space (`thread:<id>`), so pin/unpin animates
  as a *move*, not delete+insert
  (`GaryxMobileSidebarViews.swift:192-283`,
  `GaryxHomeThreadListLayout.primaryItems`,
  `GaryxHomeThreadListPresentation.swift:312,338-355`).
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
  (`GaryxMobileModel+ThreadList.swift:20-243`). Requests race freely today:
  there is no response-generation tracking, so a stale GET can land after a
  newer write's ack.
- Thread rows carry a custom 0.36s long-press action menu
  (`GaryxMessageActionMenu.swift:308,329`, mounted at
  `GaryxMobileSidebarViews.swift:408`) that will compete with a long-press
  drag lift.
- No drag/onMove/EditMode code exists anywhere in the list today.
- `pinnedThreadIds` is in-memory only (`GaryxMobileModel.swift:218`) and is
  cleared on gateway reset (`GaryxMobileModel+Gateway.swift:110`).
- Generation-guarded merge precedent already in the codebase:
  `GaryxCapsuleFavorites` (generation capture + stale-response
  membership-only merge, `GaryxCapsuleFavorites.swift:83,175`).
- Publication path: `HomeProjectionActor`/`HomeProjectionReducer` →
  `GaryxHomeThreadListStore.apply(actorSnapshot:)` → transition-state overlay
  in `presentationSnapshot`.

Desktop:

- `PinnedThreadsSidebar.tsx` renders `pinnedThreadRows` in
  `desktopState.pinnedThreadIds` order; the main-process store fills it from
  `GET /api/thread-pins` and overwrites it on every state refresh
  (`main/garyx-client/threads.ts:1111-1156`, `main/store.ts:769`,
  `mergeRemoteDesktopState` at `store.ts:752` overwriting `pinnedThreadIds`
  at `store.ts:850`). No client-side re-sort.
- `@dnd-kit/{core,sortable,modifiers,utilities}` already installed; the
  ComposerQueue (`ComposerQueue.tsx`) is a working vertical-sortable
  precedent.

## 3. Design Overview

One new ordering column + a revision-guarded collection-level reorder endpoint
on the gateway; on the client a **desired-order reducer** with
generation-guarded response merging, single-flight writes, and a durable
outbox, so the user's order is authoritative on screen from the moment of the
drop until the server provably matches it — across races, failures, and
restarts. The gesture uses native `List` reorder so lift/gap/settle animations
are the system's.

Order semantics: **the pinned order is a user-managed list.** `pinned_at`
stays as "when it was first pinned" metadata; ordering is carried exclusively
by a new `sort_order`, everywhere pins are ordered (API and task_forest).

## 4. Gateway Changes

### 4.1 Schema and migration

- Structural ensure (in `initialize_connection`, per repo convention):
  `ALTER TABLE thread_pins ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0`
  — valid on a STRICT table; the column is database-level NOT NULL from the
  start, so no NULL-ordering footgun exists (`ORDER BY sort_order ASC` would
  otherwise sort NULLs first).
- Versioned one-shot startup migration with a durable marker
  (`thread_pin_sort_order_v1`, alongside the existing versioned cutovers):
  atomically backfill `sort_order = 0..n-1` following the current display
  order (`pinned_at DESC, thread_id ASC`), so the visible order does not
  change on upgrade. Marker recorded in the same transaction; second boot
  does not re-run; zero-row DB still records the marker.
- Collection revision: a `pins_revision` integer (single-row meta storage
  next to the existing migration-marker storage), bumped inside **every**
  transaction that mutates `thread_pins` (pin, unpin, reorder, archive-side
  deletes). Fresh DB starts at 0.

### 4.2 Read/write paths (`garyx_db/mod.rs`, `task_forest.rs`)

- `list_pinned_threads`: `ORDER BY sort_order ASC, pinned_at DESC,
  thread_id ASC` (trailing keys are tie-breakers for the `DEFAULT 0` edge
  only; steady state is unique).
- `pin_thread`: new pin gets
  `sort_order = COALESCE(MIN(sort_order), 0) - 1` (head), computed and
  upserted in one transaction. **Re-pin is idempotent**: `ON CONFLICT DO
  NOTHING` — it preserves both the existing `sort_order` and the existing
  `pinned_at` (behavior change from today's upsert, which refreshes
  `pinned_at`; "first pinned at" is the meaningful semantic once ordering no
  longer depends on it).
- `unpin_thread`: unchanged (row delete). Gaps in `sort_order` are fine.
- `task_forest.rs`: both direct `thread_pins` read sites (root rank and
  skipped-pin order, `task_forest.rs:592`) switch from `pinned_at` to the
  canonical `sort_order` ordering; fixtures and ordering tests updated.
- New `reorder_thread_pins(ordered_ids: &[String], expected_revision: i64)`,
  single transaction:
  - Reads current `pins_revision`; on mismatch, returns a conflict result
    carrying the current ordered pins + revision (no mutation).
  - On match: ids in the request that exist in `thread_pins` get
    `sort_order = 0,1,2,...` in request sequence; existing pins *not*
    mentioned are renumbered after the mentioned ones, preserving their
    current relative order; request ids that are not pinned are ignored.
    **This function never changes membership.** Bumps `pins_revision`.
  - Returns the full ordered pins list + new revision.

### 4.3 HTTP API

- `GET /api/thread-pins` and both existing write responses additionally carry
  `revision` and per-pin `sort_order` (additive fields; existing clients only
  read `thread_ids`).
- New `PUT /api/thread-pins` (collection PUT, registered next to the existing
  pin routes in `route_graph.rs`), body
  `{"thread_ids": ["...", ...], "expected_revision": N}`.
  - Success: 200 with the standard pins page + `revision`.
  - Revision mismatch: **409** with the current pins page + `revision`, so
    the client can merge and resend — mirrors the repo's strict conditional
    update pattern (custom-agents `expected_updated_at`).
  - Validation (400): missing/absent `expected_revision`; `thread_ids` not a
    non-empty array of non-empty strings; duplicate ids in the array.
    Unknown/unpinned ids are *not* an error (tolerated per §4.2, which is what
    makes the call race-tolerant against concurrent unpin).
- Concurrency model: SQLite serialized writes give per-transaction atomicity;
  the CAS revision gives cross-client intent ordering — a reorder based on a
  stale view cannot silently override a concurrent pin/unpin/reorder; it 409s
  and the client resends a merged order (closed loop, see §5.2 R3/R4).

### 4.4 Gateway tests

Final validation: `cargo test -p garyx-gateway --all-targets`.

- Migration: fresh DB (column present, revision 0, marker recorded);
  legacy rows backfilled to previous visible order; zero-row DB records
  marker; second boot no re-run; failed migration transaction leaves marker
  unrecorded (re-runs cleanly).
- Pin: first pin on empty table gets a valid head `sort_order`
  (COALESCE path); pin inserts at head of existing; **re-pin keeps both
  `sort_order` and `pinned_at`**; unpin leaves order of the rest; every
  mutation bumps `revision`.
- Reorder: full permutation; subset (unmentioned keep relative order, after
  mentioned); unknown ids ignored; duplicate ids 400; empty body 400; missing
  `expected_revision` 400; stale revision 409 returns current page;
  membership never changes; response order equals subsequent `GET`.
- Route/body-level tests for `PUT /api/thread-pins` (not just the db layer).
- task_forest: root rank and skipped order follow `sort_order`; fixtures
  updated.
- Archive still removes the pin row and bumps revision.

## 5. iOS Changes (Phase 1 core)

### 5.1 Gesture: native `List` reorder, gated by an architecture spike

The move mechanics use `.onMove` on the existing flat `ForEach(items)`:

- `.moveDisabled(...)` on every non-pinned-thread item (headers, spacer,
  recent rows, placeholders) so only pinned rows lift.
- The `onMove` handler receives flat-array indices; translate to
  pinned-section-relative indices and **clamp the destination into the pinned
  segment**. The handler emits a pure Core command (`movePinned(from:to:)`),
  never mutates view state directly.
- Haptics: system drag lift provides feedback; add `.sensoryFeedback` on a
  completed reorder drop.

**Known gap (review F3): `onMove` alone has no drag lifecycle.** Its public
API is only `(IndexSet, destination)` — no began/cancelled/ended callbacks —
so it cannot by itself drive the R1 freeze window, and a cancelled drag (lift
then release in place) produces no callback at all. Additionally, thread rows
already own a custom 0.36s long-press menu that competes with the system
reorder lift.

Therefore the first commit of the batch is an **architecture-gate spike**
that must demonstrate, on the minimum deployment target (iOS 17) and the
current OS, all of:

1. drag *began*, *drop*, and *cancel* each reliably drive freeze/unfreeze;
2. long-press arbitration between the existing row action menu and the
   reorder lift is deterministic (acceptable outcomes: system arbitration
   proves clean, or pinned rows move the action menu to a non-conflicting
   affordance — swipe/ellipsis — with the product owner informed);
3. a poll/ack snapshot injected mid-lift moves no rows;
4. out-of-segment destinations clamp with sane live gap/settle behavior;
5. the existing pin/unpin single-identity *move* animation is not regressed.

Candidate mechanisms, in order of preference:

- **(a) plain `List` + `onMove` + a scoped UIKit adapter for lifecycle**: a
  lightweight introspection hook on the backing `UICollectionView` observes
  its drag-interaction state (`hasActiveDrag` / drag delegate callbacks) to
  drive freeze/unfreeze, covering began/cancel/end. Keeps the fully native
  animation.
- **(b) transient reorder mode with explicit drag handles**: long-press on a
  pinned row (or a "Reorder" action) enters a short-lived reorder state whose
  entry/exit *is* the freeze lifecycle (`editMode` binding); handles remove
  the menu-arbitration problem entirely. Slightly heavier UI, fully
  deterministic lifecycle.

Hand-rolled offset-animation gesture reordering is rejected. Splitting the
pinned rows into their own `ForEach` is a last resort only, because it
threatens the single identity space that makes pin/unpin animate as a move
(`GaryxHomeThreadListPresentation.swift:312`); if ever attempted it must
re-prove point 5 above. The state-machine wiring (§5.2) does not start until
the spike commit demonstrates points 1-5.

### 5.2 Local order authority (Core state machine, the anti-flicker core)

New pure state in `GaryxMobileCore` (a value-type `GaryxPinnedOrderState`
owned by `GaryxHomeThreadListStore`, cooperating with the existing
`GaryxHomeThreadTransitionState`; pure, SwiftPM-testable). It follows the
generation-guard pattern already proven in `GaryxCapsuleFavorites`
(`GaryxCapsuleFavorites.swift:83,175`).

Core concepts:

- **`desiredOrder`** — the reduced, always-current user-intended pinned order.
  Every drop folds into it. There is no queue of individual move intents;
  later drops supersede earlier ones by construction (review F1's
  out-of-order-PUT hazard is removed structurally).
- **`mutationGeneration`** — monotonically increasing; bumped by every local
  mutation that affects pinned membership or order (reorder drop, optimistic
  pin, unpin).
- **Response capture** — every pins-touching request (pins GET, pin/unpin
  write, reorder PUT) captures `mutationGeneration` at send time. A response
  whose captured generation is behind the current generation, or that arrives
  while `desiredOrder` is unsettled, may only **merge membership** (new ids
  enter at the head, keeping their server-relative order among themselves;
  unpinned ids drop out; survivors keep local order). It never contributes
  order. This closes F1's "stale GET lands after ack" reversion: the stale
  GET was sent at an older generation, so it is membership-only forever.
- **Single-flight reorder writes** — at most one `PUT /api/thread-pins` in
  flight per client. It always sends the *current* `desiredOrder` and the
  latest known `revision`. When it completes and `desiredOrder` has moved on,
  the next PUT fires with the newer value.

Rules:

- **R1 — freeze during drag.** While a drag session is active (lifecycle from
  §5.1), the store buffers (latest-wins) incoming snapshots instead of
  publishing; the buffered snapshot is applied after drop/cancel with the
  order overlay on top. Rows never shift under the user's finger. A cancelled
  drag unfreezes without any order change.
- **R2 — commit on drop.** The drop folds into `desiredOrder`, bumps
  `mutationGeneration`, calls `recentThreadFeeds.noteLocalMutation()`,
  persists the outbox (R5), and (if none in flight) fires the single-flight
  PUT. The only visible motion is the system drop-settle.
- **R3 — local order wins while unsettled.** While `desiredOrder` is
  unsettled (in-flight or un-acked), every incoming pins page merges
  membership-only per the response-capture rule. If a membership merge
  changes the pinned set (e.g. another device pinned D → D enters local head),
  `desiredOrder` is re-reduced to include it — so the next PUT carries the
  *full merged* order. Server order is not adopted.
- **R4 — settle without motion, revision-aware.** On PUT success: if the ack
  matches the current `desiredOrder` and no newer local mutation exists, the
  intent settles — the presented order is already identical, so settling is
  invisible; the acked `revision` becomes the CAS token. On **409**: merge
  membership from the returned page into `desiredOrder` (per R3), adopt the
  returned `revision`, and immediately resend — a silent, closed CAS loop
  (this is F2's fix: a concurrent pin on another device produces one 409 →
  merged full-order resend → convergence, with zero visible motion locally).
  Only when `desiredOrder` is settled and a response's captured generation is
  current does a server pins page order apply directly (cold start with empty
  outbox, other-device changes at rest).
- **R5 — failure never loses the order (durable outbox).** A failed reorder
  write never snaps the UI back. The unsettled `desiredOrder` (+ gateway
  identity) is persisted as a **gateway-scoped durable outbox** entry when
  committed (R2), replayed after transient failures on subsequent poll ticks
  with backoff, and — because it is durable — resumed across app restarts and
  gateway reconnects: on cold start with a non-empty outbox, the fetched page
  merges membership under the outbox order and idempotent retry continues
  until acked or superseded by a newer local drop. No retry cap and no
  user-facing error path is needed: retries are idempotent CAS PUTs, a newer
  drop supersedes, and cross-device conflicts stay last-writer-wins. The
  outbox is cleared on settle and on gateway switch (an outbox is only ever
  valid against the gateway it was created for).

`applyPinnedThreadIds` and `commitRefreshedRecentThreadsPage`
(`GaryxMobileModel+ThreadList.swift`) route through this state instead of
overwriting `pinnedThreadIds` unconditionally; the pin/unpin response
write-back (`GaryxMobileModel+ThreadPersistence.swift:127`) captures
generation like any other response.

### 5.3 Files and tests

- Core: `GaryxPinnedOrderState` + move/clamp computation next to
  `GaryxHomeThreadListPresentation.swift`; outbox persistence behind a small
  protocol seam (UserDefaults-backed in the app target). New files need
  `xcodegen generate` with the regenerated `pbxproj` committed.
- SwiftPM tests (`Tests/GaryxMobileCoreTests/`), no-UI first per repo rule:
  - move/clamp: flat-index → pinned-relative translation, edge clamps,
    single-item and top/bottom moves, drag cancel (no-op unfreeze);
  - R1: snapshots buffered during drag, latest-wins, replayed after
    drop/cancel;
  - R2/R4: intent lifecycle — commit, ack settle asserts **zero row-order
    delta**; 409 → membership merge → resend → settle;
  - F1 regressions: stale GET (older captured generation) landing *after*
    ack is membership-only; two successive drops produce one superseding
    desired order (no out-of-order final state);
  - R3: poll page during in-flight reorder merges membership but preserves
    local order; concurrent other-device pin add → head insert → next PUT
    carries merged full order → next GET shows no jump;
  - reorder × optimistic pin/unpin interleavings, both success and failure
    legs of the pin transition;
  - R5: durable outbox — restart recovery replays and settles; gateway
    switch clears; supersede-by-newer-drop;
  - regression: existing pin/unpin transition tests stay green.
- App-target no-UI integration tests at the existing URLProtocol seam
  (`Tests/GaryxMobileTests/GaryxHomeThreadListRefreshCommitTests.swift:639`):
  real network wiring for reorder PUT single-flight, 409 resend, and
  stale-GET-after-ack — run via `xcodebuild test` (not compile-only).
- Manual simulator pass only for gesture *feel* (lift, gap, settle, haptic),
  not as the acceptance for ordering logic.

## 6. Desktop Changes (Phase 2)

- `PinnedThreadsSidebar` becomes a `DndContext` + `SortableContext`
  (`verticalListSortingStrategy`, `restrictToVerticalAxis`) following the
  ComposerQueue precedent; row lift/drop uses dnd-kit's transform animations.
- On drop: optimistic reorder of `desktopState.pinnedThreadIds` in the
  renderer, then preload → main `setThreadPinOrder(threadIds)` → gateway
  `PUT /api/thread-pins` (same CAS body).
- The main-process store gets the same desired-order reducer +
  generation guard + single-flight + outbox (persisted in the main-process
  store's existing persistence), applied exactly where the poll overwrite
  happens today: `mergeRemoteDesktopState` (`store.ts:752`) must merge
  membership-only into `pinnedThreadIds` (`store.ts:850`) while an intent is
  unsettled.
- Contract additions in `shared/contracts` + unit tests via
  `npm run test:unit` (not bare `node --test`, per repo test-runner rule),
  covering the main-store guard at the real overwrite site.

## 7. Failure / Race Matrix (summary)

| Scenario | Behavior |
| --- | --- |
| Poll reply lands mid-drag | Buffered; applied after drop/cancel under overlay (R1) |
| Drag cancelled (lift, no move) | Unfreeze, no order change, no PUT (R1) |
| Poll reply while reorder unsettled | Membership merged, local order kept (R3) |
| Stale GET arrives *after* ack | Captured generation is old → membership-only, no reversion (F1) |
| Two quick drops | Desired-order reducer + single-flight: second supersedes; last PUT carries final order (F1) |
| Reorder ack | Invisible settle; zero row-order delta (R4) |
| Concurrent pin on another device | 409 → merge (new id at head) → resend full order → converge, no visible jump (F2/R4) |
| Concurrent unpin on another device | Unknown id ignored server-side; id drops out locally on membership merge |
| Two devices reorder | CAS: stale writer 409s and resends; last writer wins deterministically |
| Reorder HTTP failure / unreachable gateway | Local order kept; durable outbox retries with backoff (R5) |
| App restart with unsettled reorder | Outbox restored; local order overlays fetch; retry continues (R5/F4) |
| Gateway switch | Outbox cleared; server order adopted fresh |
| Drop outside pinned segment | Clamped to segment edge; never unpins |

## 8. Delivery Batches

1. **B1 gateway**: schema ensure + versioned backfill migration + revision +
   `reorder_thread_pins` + `PUT /api/thread-pins` + task_forest ordering +
   tests (`--all-targets`).
2. **B2 iOS**: architecture-gate spike commit (gesture lifecycle proof,
   §5.1 points 1-5) → Core state machine + outbox + tests → wiring +
   haptics → `xcodebuild test`. Ships with B1 in the same merge train
   (mobile-first deliverable).
3. **B3 desktop**: dnd-kit reorder + main-store guard + contract + tests.
   Separate follow-up task after B1/B2 land.

## 9. Decisions Taken (review these explicitly)

1. Order carried by a dedicated `sort_order` on the direct-write `thread_pins`
   table — *not* a `thread_records` body field or `recent_threads` projection
   column, because pins are deliberately outside the projection contract.
   All ordering consumers (API *and* task_forest) switch to it.
2. Reorder endpoint is a **revision-CAS** collection PUT that never mutates
   membership; unknown ids ignored (unpin race tolerance); duplicates/blank
   ids/missing revision are 400; stale revision is 409 + current page
   (concurrent-pin race closed by merge-and-resend).
3. Reorder failure does **not** roll back the UI, and the unsettled order is
   a **durable, gateway-scoped outbox** with unbounded idempotent retry —
   surviving restarts — rather than a session-only ×3 retry. Rationale: the
   user's explicit spatial arrangement snapping back (including after a
   relaunch) reads as flicker/data loss; convergence is guaranteed by CAS +
   supersession, so no user-facing failure surface is required.
4. Re-pin becomes idempotent (`pinned_at` and `sort_order` both preserved) —
   deliberate behavior change; `pinned_at` now means "first pinned at".
5. Recent-section ordering untouched; no cross-section drag semantics.
6. Native `List`/`onMove` over custom gestures, gated by a mandatory
   architecture spike that must prove drag began/cancel/end lifecycle, menu
   arbitration, and mid-drag injection stability before any wiring starts;
   explicit drag-handle reorder mode is the sanctioned fallback.
