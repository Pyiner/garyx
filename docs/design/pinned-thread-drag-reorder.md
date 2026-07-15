# Pinned Thread Drag Reorder (Mobile-First)

Status: v6 draft for design review (revised after review rounds 1-5, #TASK-2287)
Scope: gateway + iOS (phase 1), Mac desktop (phase 2)

Revision notes:

- v5 → v6 (review round 5): membership-gated reorder dispatch
  (`waitingForMembership`) so a reorder never spins against live optimistic
  pin/unpin intents; all settle comparisons use the server page's raw
  canonical order (never an intent-overlay projection); any accepted page
  whose raw order equals `desiredOrder` settles the outbox; empty
  `desiredOrder` clears the outbox and the client never sends an empty
  `thread_ids` (gateway 400 stays as defense-in-depth); bounded-write test
  obligations (PUT count bounded, no 400, no revision storm, outbox drains)
  at Core, App-URLProtocol, and desktop main-store layers.
- v4 → v5 (review round 4): V4-1 desktop stamp ownership corrected — persisted
  `DesktopState` carries server-domain fields only; `capturedEpoch` +
  `rendererSessionId` + gateway identity live in a renderer-only delivery
  envelope stamped *before* the request awaits; V4-2 round-3 F1/F4
  regressions duplicated at the App-target URLProtocol orchestration layer;
  V4-3 performance gate rewritten against the real probe (extend it to record
  worst-frame data; frozen baselines + explicit threshold assertions;
  separate iOS 17 vs adapter-OS measurement paths); V4-4 explicit response
  pipeline order (identity → completion → acceptance → publication) and the
  freeze buffer holds accepted projections, never raw pages.
- v3 → v4 (review round 3): F1 explicit page-acceptance outcomes, transport
  completion decoupled from page acceptance, immediate floor-token resend of
  an unsettled outbox, revision-bounded per-id intent lifecycle; F2 desktop
  stamped pins envelope with epoch/revision rejection at the React ingress
  (commit-time rebase, centralized setter); F3 iOS 17 on-runtime verification
  made non-waivable, version-gated `EditMode` fallback, decidable perf
  thresholds; F4 gateway-scoped pinned-order state domain (per-gateway
  revision floor, identity-tagged requests).
- v2 → v3 (review round 2): V2-1 revision-monotonic page acceptance +
  epoch advance on settle + intent-overlay membership merge; V2-2 atomic
  `ThreadPinsPage` snapshot contract + shared revision-bump tx helper over
  *all* pin delete points; V2-3 desktop reducer applied at the last hop before
  React state, not only in the main store; V2-4 classified retry policy with a
  non-blocking pending-sync state instead of unbounded silent retry; V2-5
  drag preview order, accepted-drop-vs-cancel proof obligations, quantified
  hitch regression in the spike.
- v1 → v2 (review round 1): F1 generation guard + desired-order reducer +
  single-flight writes; F2 revision CAS; F3 drag-lifecycle architecture gate;
  F4 durable outbox; F5 corrected STRICT schema/migration + idempotent
  re-pin; F6 task_forest ordering consumers; F7 expanded race/test coverage.

## 1. Problem And Goals

Users cannot reorder pinned threads. Pin order is server-decided
(`pinned_at DESC`), so the only way to move a thread up is unpin + repin.

Goals, in priority order:

1. Drag-to-reorder inside the pinned area of the iOS home list (mobile-first).
2. The interaction must feel native and smooth: system reorder animation,
   haptics, quantified no-hitch scrolling.
3. **Zero flicker from server round-trips.** The local order the user just
   created is authoritative on screen. Background refresh loops (10s/15s),
   pin-write responses, reorder-write responses, and *late/stale/out-of-order
   responses* must never visibly reshuffle the pinned section. "Backend not
   settled yet" is never a visible reshuffle — including across app restarts.
4. Same capability on Mac desktop afterwards, reusing the same wire contract.

Non-goals:

- Reordering the Recent section (stays server recency order).
- Cross-section drag (dragging a pinned row into Recent is not an unpin
  gesture; it clamps back into the pinned segment).
- Legacy-gateway compatibility shims (repo policy: desktop/gateway ship
  together). A reorder call hitting an old gateway (405) is handled by the
  permanent-error leg of the retry policy (§5.2 R5) — order kept, requests
  paused, pending-sync surfaced — not by a compat path.

## 2. Current State (verified)

Gateway:

- Pins live in a dedicated direct-write table `thread_pins
  (thread_id TEXT PRIMARY KEY, pinned_at TEXT NOT NULL) STRICT`
  (`garyx-gateway/src/garyx_db/mod.rs:2005`). Not a projection of
  `thread_records`; not a `recent_threads` column.
- `list_pinned_threads` orders by `pinned_at DESC, thread_id ASC`
  (`mod.rs:463`) and runs on the WAL read pool (`mod.rs:327-423`) —
  independent of any write transaction.
- `pin_thread` upserts and refreshes `pinned_at` on re-pin (`mod.rs:481`).
- Existing pin/unpin handlers mutate first, then *separately* list pins for
  the response (`routes.rs:1693-1696,1728-1730`) — i.e. today's response page
  is not from the mutation's transaction.
- **Pin rows are deleted from four sites**, not one: `unpin_thread`
  (`mod.rs:501`), `archive_thread_record` (`mod.rs:532`), runtime hard delete
  `delete_thread_record_with_projections` (`mod.rs:1709`), and startup
  cleanup `purge_retired_workflow_state` (`mod.rs:2368`).
- **task_forest reads `thread_pins` directly**: root rank and skipped-pin
  order come from `pinned_at` in the forest CTE (`task_forest.rs:592`).
- HTTP: `GET /api/thread-pins`, `PUT /api/thread-pins/{key}` (pin),
  `DELETE /api/thread-pins/{key}` (unpin) — handlers in
  `routes.rs:1660-1747`, routes in `route_graph.rs:62-85`. Write responses
  return the full pins page (`thread_pins_payload`, `routes.rs:1449`).
- Schema handling convention: structural `ensure` in `initialize_connection`
  (`mod.rs:2259`) + versioned one-shot startup migrations with a durable
  marker (`mod.rs:901`).
- No SSE/push carries pins or recent-threads; clients poll.

iOS:

- Home list is a native SwiftUI `List` with one flat `ForEach(items)`; one
  stable identity space (`thread:<id>`) makes pin/unpin animate as a *move*
  (`GaryxMobileSidebarViews.swift:192-283`,
  `GaryxHomeThreadListPresentation.swift:312,338-355`).
- Pinned order = `pinnedThreadIds` array order = server pins-page order
  (`GaryxHomeThreadSectionsBuilder.build`,
  `GaryxHomeThreadListPresentation.swift:670-715`).
- Optimistic pin/unpin exists: `GaryxHomeThreadTransitionState` sequence-
  stamped per-id transitions with rollback
  (`GaryxHomeThreadListPresentation.swift:377-607`,
  `GaryxMobileModel+ThreadPersistence.swift:63-168`).
- Refresh loops (~10s silent, ~15s reconcile, plus every user action)
  re-fetch pins concurrently and overwrite `pinnedThreadIds`
  (`GaryxMobileModel+ThreadList.swift:20-243`). Requests race freely; no
  response-generation tracking.
- Thread rows carry a custom 0.36s long-press action menu
  (`GaryxMessageActionMenu.swift:329-343`, mounted at
  `GaryxMobileSidebarViews.swift:408`) that competes with a long-press lift.
- `pinnedThreadIds` is in-memory (`GaryxMobileModel.swift:218`), cleared on
  gateway reset (`GaryxMobileModel+Gateway.swift:110`).
- In-repo precedents: generation-guarded merge that **advances its
  generation on success/failure settle** (`GaryxCapsuleFavorites.swift:83-175`,
  test `testRefreshSentDuringPendingCannotClobberSettledFavorite`);状态级
  retry classifier `GaryxGatewayRetryClassifier`
  (`GaryxGatewayClient.swift:205-256`); quantified list-performance harness
  with hitch probe (`HomeListScrollPerformanceTests.swift:35-59`).

Desktop:

- `PinnedThreadsSidebar.tsx` renders `desktopState.pinnedThreadIds` order;
  the main store overwrites it on every state refresh
  (`mergeRemoteDesktopState`, `main/store.ts:752,850`).
- **Stale state can also reach the renderer around the main store**: the
  gateway mirror keeps an already-computed `nextState`
  (`gateway-mirror/mirror.ts:350-355`) and
  `useGatewayConnectionController.ts:368-376` commits it via a deferred
  `startTransition(setDesktopState)`. A snapshot computed before a drop can
  therefore commit to React after the renderer's optimistic reorder.
- `@dnd-kit/{core,sortable,modifiers,utilities}` installed; ComposerQueue
  (`ComposerQueue.tsx`) is the vertical-sortable precedent.

## 3. Design Overview

One new ordering column + a revision-CAS reorder endpoint whose pages are
**atomic snapshots** (page + revision from one transaction); on the client a
**desired-order reducer** with revision-monotonic page acceptance,
epoch-guarded authority, single-flight writes, a durable outbox with a
classified retry policy, and reducer application at the *last hop* before UI
state. The gesture uses native `List` reorder so lift/gap/settle animations
are the system's, with drag ordering held in a preview state until an
accepted drop.

Order semantics: **the pinned order is a user-managed list.** `pinned_at`
stays as "when it was first pinned" metadata; ordering is carried exclusively
by a new `sort_order`, everywhere pins are ordered (API and task_forest).

## 4. Gateway Changes

### 4.1 Schema and migration

- Structural ensure (in `initialize_connection`, per repo convention):
  `ALTER TABLE thread_pins ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0`
  — valid on a STRICT table; database-level NOT NULL from the start (no
  NULL-ordering footgun).
- Versioned one-shot startup migration with a durable marker
  (`thread_pin_sort_order_v1`): atomically backfill `sort_order = 0..n-1`
  following the current display order (`pinned_at DESC, thread_id ASC`), so
  the visible order does not change on upgrade. Marker recorded in the same
  transaction; second boot does not re-run; zero-row DB still records the
  marker; a failed migration transaction leaves the marker unrecorded.
- Collection revision: a `pins_revision` integer (single-row meta storage
  next to the existing marker storage), starting at 0 on fresh DBs.

### 4.2 The atomic `ThreadPinsPage` boundary (review V2-2)

A single database-layer type is the only way pins cross the db boundary:

```
ThreadPinsPage { pins: [ { thread_id, pinned_at, sort_order } ], revision }
```

Invariants:

- **Reads**: `GET` produces `pins` and `revision` inside one WAL read
  transaction (or one equivalent SQL statement joining the meta row). A page
  can never pair a newer revision with older membership or vice versa.
- **Writes**: pin, unpin, and reorder each perform *mutation + revision bump
  + response-page read* inside the same write transaction and return the
  resulting `ThreadPinsPage`. Handlers must not re-list after the
  transaction (change from today's mutate-then-list shape at
  `routes.rs:1693-1696,1728-1730`).
- **Every mutation bumps `pins_revision`**, enforced by a shared tx helper
  used at *all four* delete/write sites: `unpin_thread` (`mod.rs:501`),
  `archive_thread_record` (`mod.rs:532`), runtime hard delete
  (`delete_thread_record_with_projections`, `mod.rs:1709`), startup cleanup
  (`purge_retired_workflow_state`, `mod.rs:2368`), plus `pin_thread` and the
  new reorder. No pin-row mutation may bypass the helper.

### 4.3 Read/write paths (`garyx_db/mod.rs`, `task_forest.rs`)

- `list_pinned_threads`: `ORDER BY sort_order ASC, pinned_at DESC,
  thread_id ASC` (trailing keys are tie-breakers for the `DEFAULT 0` edge
  only; steady state is unique).
- `pin_thread`: new pin gets `sort_order = COALESCE(MIN(sort_order), 0) - 1`
  (head), computed in the same transaction. **Re-pin is idempotent**
  (`ON CONFLICT DO NOTHING`): preserves existing `sort_order` *and*
  `pinned_at` (deliberate behavior change; `pinned_at` now means "first
  pinned at"). An idempotent re-pin that changes nothing does not bump the
  revision.
- `unpin_thread`: row delete via the shared helper. Gaps in `sort_order` are
  fine.
- `task_forest.rs`: both direct read sites (root rank, skipped-pin order at
  `task_forest.rs:592`) switch from `pinned_at` to canonical `sort_order`;
  fixtures and ordering tests updated.
- New `reorder_thread_pins(ordered_ids, expected_revision)` in one write
  transaction: on revision mismatch, return conflict + current
  `ThreadPinsPage` (no mutation). On match: mentioned-and-pinned ids get
  `sort_order = 0,1,2,...` in request sequence; unmentioned pins renumber
  after them preserving current relative order; non-pinned ids ignored.
  **Never changes membership.** Bumps revision; returns the new page.

### 4.4 HTTP API

- `GET /api/thread-pins` and both existing write responses carry `revision`
  and per-pin `sort_order` (additive; existing clients read `thread_ids`).
- New `PUT /api/thread-pins` (collection PUT), body
  `{"thread_ids": [...], "expected_revision": N}`.
  - Success: 200 with the transaction's `ThreadPinsPage`.
  - Revision mismatch: **409** with the current page + revision (client
    merges and resends — repo's strict conditional-update pattern).
  - Validation (400): missing `expected_revision`; `thread_ids` not a
    non-empty array of non-empty strings; duplicate ids. Unknown/unpinned
    ids are *not* an error (unpin race tolerance). Clients are specified to
    never send an empty `thread_ids` (an emptied `desiredOrder` clears the
    client outbox instead, §5.2); the 400 remains as defense-in-depth, not a
    path a conforming client can hit.
- Concurrency: per-transaction atomicity + CAS revision gives cross-client
  intent ordering; a stale-view reorder 409s instead of silently overriding
  concurrent pin/unpin/reorder.

### 4.5 Gateway tests

Final validation: `cargo test -p garyx-gateway --all-targets`.

- Migration: fresh DB (column, revision 0, marker); legacy backfill keeps
  visible order; zero-row marker; second boot no re-run; failed transaction
  re-runs cleanly.
- Atomic snapshot (V2-2): deterministic seam test proving a writer committing
  between "read pins" and "read revision" is impossible through the
  `ThreadPinsPage` API (single-transaction read); handler-level test that
  write responses come from the mutation transaction.
- Revision inventory (V2-2): pin, unpin, reorder, archive, runtime hard
  delete, and startup cleanup each bump revision exactly once; idempotent
  re-pin bumps nothing and preserves `pinned_at` + `sort_order`.
- Pin/order: first pin on empty table (COALESCE head); head insert; unpin
  leaves rest; reorder full permutation / subset / unknown-id / duplicate-400
  / empty-400 / missing-revision-400 / stale-revision-409-with-page;
  membership never changes; response order equals subsequent GET.
- Route/body-level tests for `PUT /api/thread-pins`.
- task_forest: root rank and skipped order follow `sort_order`; fixtures
  updated.

## 5. iOS Changes (Phase 1 core)

### 5.1 Gesture: native `List` reorder, gated by an architecture spike

Move mechanics: `.onMove` on the existing flat `ForEach(items)`;
`.moveDisabled(...)` on every non-pinned-thread item; flat indices translate
to pinned-relative indices with destination **clamped into the pinned
segment**; haptic `.sensoryFeedback` on a completed drop.

**Drag ordering is a preview, not a commit (review V2-5).** `onMove`
callbacks update only a `dragPreviewOrder` inside the drag session. They
never advance the epoch, touch the outbox, or trigger a PUT. Exactly one
commit happens, on an *accepted drop* (preview folds into `desiredOrder`,
§5.2 R2). A cancelled drag — including after multiple `onMove` callbacks —
restores from the pre-drag baseline with **zero remote mutation**.

**Known gaps (reviews F3/V2-5):** `onMove` exposes no began/cancelled/ended
lifecycle; `dragSessionDidEnd` alone cannot distinguish an accepted drop from
a terminated session; thread rows already own a 0.36s long-press menu that
competes with the lift.

The first commit of the batch is therefore an **architecture-gate spike**
that must demonstrate, on the app's **minimum deployment target (iOS 17, per
`project.yml` / `Package.swift`) and the current OS**. iOS 17 verification is
**non-waivable** for the adapter path (review round-3 F3): the OS behaviors
under test — `onMove`'s callback-only surface, drag-session end semantics,
delegate ownership on the backing collection view — are exactly the unknowns,
so API availability alone proves nothing. Obligations 1-6 must run on an
iOS 17 simulator, device, or CI runner (install the runtime via `xcodebuild
-downloadPlatform iOS` if absent). If an iOS 17 runtime genuinely cannot be
obtained anywhere, the adapter path must not ship to iOS 17 at all:
**version-gate** it — iOS 17 gets the officially documented `EditMode`-based
explicit reorder mode (fallback (b)), and the adapter path enables only on OS
versions it was actually verified on. Raising the deployment target is a
product decision for the owner, never an implementer fallback. The spike must
demonstrate:

1. drag *began*, *accepted drop*, and *cancel* are reliably distinguished and
   each drives freeze/unfreeze (accepted drop ⇒ single commit; cancel ⇒
   baseline restore, zero mutation);
2. the UIKit observation adapter does **not** replace or interfere with
   SwiftUI's own drag/drop delegates on the backing `UICollectionView`
   (observe, never substitute);
3. long-press arbitration between the row action menu and the reorder lift is
   deterministic (acceptable outcomes: system arbitration proves clean, or
   pinned rows move the action menu to a non-conflicting affordance —
   swipe/ellipsis — with the product owner informed);
4. a poll/ack snapshot injected mid-lift moves no rows;
5. out-of-segment destinations clamp with sane live gap/settle behavior;
6. the existing pin/unpin single-identity *move* animation is not regressed;
7. **quantified performance** (corrected against the real harness, round-4
   V4-3): today `GaryxHomeScrollPerformanceReport` records only
   `hitchTimeRatio` (`GaryxHomeScrollPerformanceProbe.swift:48-78,139-146`)
   and `HomeListScrollPerformanceTests.swift:35-60` only emits XCTest
   `measure` output with no baseline or assertion, and `XCTHitchMetric` is
   only available on the current OS, not iOS 17. The spike must therefore:
   (a) **extend the probe** to also record worst-frame data
   (`maxFrameInterval` / worst-frame delta); (b) **freeze, before any feature
   wiring starts**, one fixed simulator/device configuration, recorded
   baselines, and absolute/relative thresholds for `hitchTimeRatio` and the
   new worst-frame metric; (c) make the tests **assert those thresholds
   explicitly** (mechanical pass/fail, not `measure` output to eyeball);
   (d) define the two measurement paths separately — the verified-adapter OS
   path may use `XCTHitchMetric`, the iOS 17 fallback path measures via the
   probe's own frame-interval instrumentation.

Candidate mechanisms, in order of preference: (a) plain `List` + `onMove` +
a scoped, observation-only UIKit adapter for lifecycle (keeps fully native
animation); (b) transient reorder mode with explicit drag handles
(`editMode`-bound lifecycle, officially supported; removes menu arbitration
entirely). Hand-rolled offset-animation reordering is rejected; splitting
pinned rows into their own `ForEach` is a last resort that would re-open the
single-identity-space proof (point 6). State-machine wiring (§5.2) does not
start until the spike demonstrates points 1-7.

### 5.2 Local order authority (Core state machine, the anti-flicker core)

New pure value-type state in `GaryxMobileCore` (`GaryxPinnedOrderState`,
owned by `GaryxHomeThreadListStore`, cooperating with
`GaryxHomeThreadTransitionState`), following — and correcting to — the
`GaryxCapsuleFavorites` precedent, **including its settle-time generation
advance** (`GaryxCapsuleFavorites.swift:114-166`).

Core concepts:

- **`desiredOrder`** — the reduced, always-current user-intended pinned
  order. Every accepted drop folds into it; later drops supersede earlier
  ones structurally (no intent queue, no out-of-order finals).
- **`epoch`** — monotone counter advanced by (a) every local mutation
  affecting pinned membership or order (accepted drop, optimistic pin,
  unpin) **and (b) every settle, success or failure** (review V2-1: settle
  invalidates all requests issued during the unsettled window, so none of
  them can later be mistaken for authoritative).
- **`highestObservedRevision`** — monotone acceptance floor over server
  `revision` values.
- **Single-flight reorder writes, membership-gated (round-5 F1)** — at most
  one collection PUT in flight; it always carries the current `desiredOrder`
  + latest known revision. **Dispatch is gated on membership quiescence**: a
  PUT may only be issued while no per-id pin/unpin intent is live. When a PUT
  completes without settling (order moved on, 409, below-floor) *and* no
  membership intent is live, the next PUT fires immediately with current
  values; if membership intents are live, the outbox instead enters an
  explicit **`waitingForMembership`** state — flight slot freed, no resend.
  Wake triggers: the last live membership intent resolves (success *or*
  failure rollback), or any accepted page lands. On wake: re-reduce
  `desiredOrder` over the merged membership; if it is **empty**, clear the
  outbox (nothing left to order — the client never sends an empty
  `thread_ids`); if the last accepted page's **raw canonical order** already
  equals `desiredOrder`, settle without a write; otherwise dispatch with the
  floor token. This bounds writes to at most one PUT per
  membership-quiescent window: a reorder can no longer spin re-appending an
  id whose unpin is still in flight, nor 400 itself into a permanently
  paused outbox after a full unpin.

**Page acceptance vs. flight completion (review round-3 F1).** Every response
does two independent things: it always **completes its transport flight**
(frees the single-flight slot, resolves the per-id intent's transport,
feeds the retry classifier) and it **offers a pins page** which the reducer
accepts or rejects. The reducer returns an explicit outcome —
`discardedBelowFloor | merged | authoritative` — so callers can distinguish
"page unusable" from "page applied":

1. `discardedBelowFloor` — `page.revision < highestObservedRevision`: the
   page contributes nothing (not even membership) and its revision is never
   usable as a CAS token. This closes V2-1's main timeline (a poll that read
   pre-PUT state carries a lower revision than the PUT ack, whenever it
   arrives) and revision-descending pin write-backs (an old pin-B page
   arriving after the pin-C ack cannot temporarily delete C).
2. Otherwise raise `highestObservedRevision` to `page.revision`, then:
3. `merged` — if the request's captured epoch < current epoch, **or**
   `desiredOrder` is unsettled: **membership-only merge with intent
   overlay** — merged membership = page membership, minus ids with a live
   unpin intent, plus ids with a live pin intent or a retired pin intent
   whose completion revision the accepted floor has not yet reached (per-id
   transitions from `GaryxHomeThreadTransitionState`); new ids enter at the
   head keeping their server-relative order among themselves; survivors keep
   local order. Missing-from-page is never interpreted as unpin for an id
   with a live pin intent. `desiredOrder` is re-reduced over the merged
   membership, so the next PUT carries the full merged order.
4. `authoritative` — settled **and** captured epoch current **and** revision
   ≥ floor: adopt the page's order directly (cold start with empty outbox,
   other-device changes at rest).

Completion semantics on a `discardedBelowFloor` page, per request kind:

- **Reorder PUT (200 or 409) below floor** (a remote write raised the floor
  while our request was in flight): the flight ends, the intent does **not**
  settle, and — because the outbox is still unsettled with an unchanged
  `desiredOrder` — the single-flight **immediately resends** the current full
  `desiredOrder` with `expected_revision = highestObservedRevision`. No
  waiting for a further local mutation; the outbox can never hang settled-less
  with no flight in progress. Convergence is last-writer-wins by design.
- **Pin/unpin write-back below floor**: the per-id intent retires at
  completion regardless of page acceptance, recording its completion
  revision. Overlays apply only while the intent is live or until an accepted
  page with `revision ≥` that completion revision arrives — so a remote
  opposite operation at a higher revision wins (LWW) and server state is
  never permanently hidden behind a stale overlay. No automatic re-issue of
  pin/unpin.

**Response pipeline order (review round-4 V4-4).** Every response is
processed in this fixed order, on both platforms:

1. **identity check** — wrong gateway identity / renderer session ⇒ dropped;
2. **transport completion** — flight slot freed, per-id intent transport
   resolved, retry classifier fed;
3. **revision acceptance** — the reducer runs (floor update, outcome
   `discardedBelowFloor | merged | authoritative`), producing an *accepted
   projection* (or nothing);
4. **publication** — the accepted projection publishes to the UI, or, while a
   drag freeze is active, lands in the freeze buffer.

The freeze buffer therefore holds only **accepted projections** — the
highest-revision accepted state — never raw pages latest-wins by arrival
time. A delayed rev-11 page arriving after a rev-12 page during a drag is
discarded at step 3 and can never overwrite the buffered rev-12 projection,
so neither drop nor cancel replays it.

**Gateway-scoped state domain (review round-3 F4).** The entire pinned-order
state — revision floor, `desiredOrder`/baseline, drag preview and freeze
buffer, per-id intents, in-flight token, pending-sync state, and outbox — is
keyed by gateway identity. Every request is tagged with the identity it was
issued against; a response arriving under a different identity is dropped
outright (identity check precedes the revision check, so an old gateway's
floor can never suppress a new gateway's revision-0 pages). Gateway switch
atomically replaces the whole domain with a fresh one (floor 0, empty
outbox) inside the existing reset path
(`GaryxMobileModel+Gateway.swift:74-113`, alongside `resetTransitions()`),
cancelling in-flight requests. §7's "server order adopted fresh" holds by
construction.

Rules:

- **R1 — freeze during drag.** While a drag session is active (lifecycle from
  §5.1), publication is deferred: incoming responses still run the full
  pipeline (identity → completion → acceptance), and only the resulting
  **accepted projection** lands in the freeze buffer (which thus always holds
  the highest accepted revision, not the latest arrival — V4-4). The buffered
  projection is applied after drop/cancel under the order overlay. Rows never
  shift under the finger; cancel unfreezes with no order change.
- **R2 — commit on accepted drop.** The preview folds into `desiredOrder`,
  `epoch` advances, `recentThreadFeeds.noteLocalMutation()` fires, the outbox
  persists (R5), and (if none in flight) the single-flight PUT fires. The
  only visible motion is the system drop-settle.
- **R3 — local order wins while unsettled.** Every page landing while
  unsettled goes through the acceptance procedure and can at most merge
  membership (step 3). Server order is not adopted.
- **R4 — settle without motion, revision-aware, raw-order-compared.** All
  settle equality checks compare the server page's **raw canonical order**
  against `desiredOrder` — never an intent-overlay projection (round-5 F1;
  while membership intents are live the two legitimately differ, which is
  exactly why dispatch is gated until quiescence). A PUT ack settles only
  if: ack raw order == current `desiredOrder`, the request's epoch is still
  current (no local mutation since send), and `ack.revision ≥
  highestObservedRevision`. Settling adopts `ack.revision`, advances `epoch`
  (V2-1), clears the outbox, and changes nothing visually. A below-floor ack
  never settles; it ends the flight and triggers the floor-token resend —
  subject to the membership gate (round-3 F1, round-5 F1). On **409**: run
  the acceptance procedure on the returned page; if `discardedBelowFloor`,
  resend with `expected_revision = highestObservedRevision` (membership gate
  applies); if the accepted page's raw order already equals `desiredOrder`,
  **settle directly without re-PUT** (V2-4); otherwise resend with the
  accepted page's revision — a silent, closed CAS loop. Additionally, **any
  accepted page from any request** (GET, pin/unpin write-back) whose raw
  canonical order equals the current `desiredOrder` settles the outbox — a
  reorder that in fact landed server-side (or was arranged identically by
  another device) never leaves a stale outbox behind, including across
  restarts (round-5 F1).
- **R5 — failure never loses the order; retries are classified (V2-4).** A
  failed reorder write never snaps the UI back. The unsettled `desiredOrder`
  (+ gateway identity + last known revision) persists as a **gateway-scoped
  durable outbox**, restored across app restarts (cold start with a non-empty
  outbox: fetched pages merge membership under the outbox order; retry
  resumes). Retry policy via the existing status classifier
  (`GaryxGatewayRetryClassifier`, `GaryxGatewayClient.swift:205-256`):
  - 409 → CAS loop per R4 (not a failure);
  - network errors / 429 / retryable 5xx → capped backoff with jitter,
    piggybacking poll ticks;
  - **permanent errors (400/401/403/404/405 / contract violations, incl. an
    old gateway's 405)** → keep the outbox and the on-screen order, **pause
    requests**, and expose a **non-blocking pending-sync state** from the
    store (subtle indicator; no alert, no rollback). Retry resumes on
    gateway/settings/app-version change, app foreground, or an explicit
    user refresh.
  A newer drop always supersedes the outbox; settle and gateway switch clear
  it (an outbox is only valid against the gateway it was created for).

`applyPinnedThreadIds` and `commitRefreshedRecentThreadsPage`
(`GaryxMobileModel+ThreadList.swift`) route through this state; the pin/unpin
response write-back captures epoch/revision like any other response.

### 5.3 Files and tests

- Core: `GaryxPinnedOrderState` + move/clamp + acceptance procedure next to
  `GaryxHomeThreadListPresentation.swift`; outbox persistence behind a small
  protocol seam (UserDefaults-backed in the app target). New files need
  `xcodegen generate` with the regenerated `pbxproj` committed.
- SwiftPM tests (`Tests/GaryxMobileCoreTests/`), no-UI first:
  - move/clamp: flat→pinned-relative translation, edge clamps, top/bottom
    moves; preview-only `onMove` folding; cancel after multiple `onMove`s ⇒
    baseline restore, zero mutation (V2-5);
  - R1: buffering of accepted projections, replay after drop/cancel;
    **round-4 V4-4 regression** — during drag, rev-12 page arrives then a
    delayed rev-11 page ⇒ rev-11 discarded at acceptance, buffer keeps
    rev-12; both drop and cancel variants end with no visible movement and no
    later rev-12-poll re-flip;
  - **V2-1 exact regressions**: GET issued *after* drop (same epoch) reading
    the old page, arriving *after* ack ⇒ discarded by revision floor, no
    reversion; two pin write-backs arriving revision-descending ⇒ old page
    fully discarded, confirmed pin never temporarily removed; settle advances
    epoch so unsettled-window requests can't become authoritative;
  - **round-3 F1 exact regressions**: high-revision remote page raises floor,
    then our 200 ack with lower revision arrives ⇒ flight ends, no settle,
    immediate resend with `expected_revision = floor`, eventual settle (no
    permanently hanging outbox); same with a below-floor 409 (returned page
    unusable as CAS token); remote opposite pin/unpin at higher revision then
    local low-revision ack ⇒ intent retires, higher-revision state shown, no
    permanent overlay;
  - **round-3 F4 gateway-domain regressions**: old gateway floor=100 → switch
    → new gateway revision-0 page accepted (fresh domain); late response from
    the old gateway identity dropped by identity check before any revision
    logic;
  - R2/R4: ack settle asserts zero row-order delta; 409 → merge → resend →
    settle; 409-page-equals-desired → direct settle without re-PUT;
  - R3: poll during in-flight merge keeps local order; other-device pin →
    head insert → merged full-order PUT → next GET no jump;
  - reorder × optimistic pin/unpin interleavings (success and failure legs);
    **round-5 F1 exact regressions**: delayed unpin (reorder completes
    unsettled while unpin in flight ⇒ `waitingForMembership`, no resend, no
    re-append spin; unpin resolves ⇒ single PUT with reduced order ⇒ settle);
    delayed pin (unknown-id PUT never dispatched while pin intent live);
    membership-intent failure rollback wakes the gate; full unpin ⇒
    `desiredOrder` empties ⇒ outbox cleared, no `[]` PUT, no 400; restart
    with outbox while server already equals desired (or is empty) ⇒ settled
    by accepted-page-equals-desired, no write. Each asserts **bounded PUT
    count, zero 400s, no revision storm, outbox eventually empty**;
  - R5: durable outbox restart recovery; supersede-by-newer-drop; retry
    classification (429/5xx backoff vs 405 pause + pending-sync state);
    gateway-switch clear;
  - regression: existing pin/unpin transition tests stay green.
- App-target no-UI integration tests at the existing URLProtocol seam
  (`Tests/GaryxMobileTests/GaryxHomeThreadListRefreshCommitTests.swift:639`),
  run via `xcodebuild test` (not compile-only). Beyond single-flight, plain
  409 resend, stale-GET-after-ack, and permanent-error pause, the round-3
  F1/F4 behaviors are **orchestration-level** and must be proven here against
  real transport, not only in the pure reducer (round-4 V4-2):
  - high-revision GET → low-revision 200 ack ⇒ assert a second PUT is
    *actually issued immediately* and its body carries
    `expected_revision = floor`;
  - the same for a below-floor 409;
  - gateway switch ⇒ a late old-identity response triggers no retry; the new
    gateway's revision-0 page is accepted;
  - high-revision opposite pin/unpin page, then the local low-revision ack ⇒
    the real presentation immediately shows the saved high-revision baseline
    (intent retired, nothing hidden);
  - **round-5 F1 at real transport**: reorder PUT completes unsettled while a
    real unpin request is still in flight ⇒ no second PUT until the unpin
    resolves, then exactly one PUT with the reduced order; full unpin ⇒ no
    `[]` PUT ever issued; assert total PUT count bounded and no 400
    responses.
- Spike carries the quantified hitch gate (§5.1 point 7). Manual simulator
  pass only for gesture *feel*, never as ordering-logic acceptance.

## 6. Desktop Changes (Phase 2)

- `PinnedThreadsSidebar` becomes `DndContext` + `SortableContext`
  (vertical strategy, `restrictToVerticalAxis`) per the ComposerQueue
  precedent.
- On drop: optimistic reorder in the renderer, then preload → main
  `setThreadPinOrder(threadIds)` → gateway `PUT /api/thread-pins` (same CAS
  body). Main store owns the desired-order reducer, epoch/revision guards,
  single-flight, and outbox (persisted in the main store's existing
  persistence), applied where the poll overwrite happens
  (`mergeRemoteDesktopState`, `store.ts:752,850`).
- **Stamped delivery envelope with corrected ownership + commit-time
  rejection at the React ingress (reviews V2-3, round-3 F2, round-4 V4-1).**
  The main-store guard alone is insufficient: already-computed snapshots
  reach React through the gateway mirror (`mirror.ts:350-355`) and a deferred
  `startTransition(setDesktopState)`
  (`useGatewayConnectionController.ts:368-376`), and a one-time projection at
  queue time still loses the race where the overlay has *already retired*.
  Ownership boundaries (V4-1: `DesktopState` is produced and persisted by the
  main process — `main/store.ts:442-535` — which cannot know renderer
  epochs):
  - **Persisted/main-owned `DesktopState` carries server-domain fields
    only**: `{ pinnedThreadIds, pinsRevision }` (contract change in
    `shared/contracts/state.ts`; renderer counters are never persisted, so
    reload never compares incomparable epochs). If the persisted shape
    changes, legacy-state normalization is covered by tests.
  - **`capturedEpoch` lives in a renderer-only delivery envelope**
    `{ state, capturedEpoch, rendererSessionId, gatewayIdentity }`, wrapped
    around every async state request by **one unified request entry point
    that captures the stamp *before* the `await`** (a response landing late
    carries the epoch at issue time — it cannot be re-stamped with the
    current epoch at landing). Centralizing only `setDesktopState` is not
    enough; the capture point is the request issue site.
  - The renderer holds the authority counters (current epoch, revision
    floor, unsettled desired order). A drop advances the renderer epoch
    *synchronously*; **settle also advances it**, exactly as on iOS. On
    renderer startup, the floor/order initialize from the main process's
    pins/outbox snapshot **before** state ingress opens; a new
    `rendererSessionId` invalidates any envelope from a previous session.
  - All `setDesktopState` ingress funnels through one wrapper whose
    functional setter **rebases at commit time**: an envelope carrying a
    stale epoch, below-floor revision, wrong session, or wrong gateway
    identity is rejected/rebased against the current authoritative order —
    even after the overlay has retired.
- The main-store reducer carries the same membership gate
  (`waitingForMembership`), raw-order settle comparisons,
  accepted-page-equals-desired settle path, and empty-order outbox clearing
  as iOS (round-5 F1).
- Tests (`npm run test:unit`): main-store guard at the real overwrite site;
  the round-5 F1 set (delayed pin/unpin, failure rollback, full unpin,
  restart + server-already-equal/empty; bounded PUT count, no 400, no
  revision storm, outbox drains);
  the V2-3 race — refresh resolved and `nextState` captured, drop happens,
  deferred transition commits ⇒ committed state carries the local order;
  the round-3 F2 race — stale transition queued → drop → ack settle clears
  the overlay → stale transition commits ⇒ final state still the local
  order; **the round-4 V4-1 races** — request issued at epoch 7 → drop (8) →
  settle (9) → response lands last ⇒ envelope carries 7 and is rejected (not
  re-stamped at landing); renderer reload ⇒ previous-session envelope dropped
  by `rendererSessionId`; legacy persisted-state normalization if the shape
  changed; revision-floor rejection at the ingress; contract tests for the
  new preload/main API and the envelope.

## 7. Failure / Race Matrix (summary)

| Scenario | Behavior |
| --- | --- |
| Poll reply lands mid-drag | Buffered; applied after drop/cancel under overlay (R1) |
| Drag cancelled (even after several `onMove`s) | Baseline restore; zero remote mutation (V2-5) |
| Poll reply while reorder unsettled | Membership-only merge with intent overlay (R3) |
| GET sent after drop, reads pre-PUT page, arrives after ack | Revision floor discards the whole page (V2-1) |
| Pin write-backs arrive revision-descending | Older page fully discarded; confirmed pin never flickers out (V2-1) |
| Two quick drops | Reducer + single-flight: second supersedes; final PUT carries final order |
| Reorder ack | Settles only when order/epoch/revision all match; zero visual delta; epoch advances (R4) |
| Concurrent pin on another device | 409 → merge (new id at head) → resend full order → converge; if 409 page already matches, settle without re-PUT (R4) |
| Concurrent unpin on another device | Unknown id ignored server-side; drops out locally on membership merge |
| Two devices reorder | CAS: stale writer 409s and resends; deterministic last-writer-wins |
| Reorder completes unsettled while local pin/unpin still in flight | `waitingForMembership`: no resend until intents resolve; then ≤1 PUT with reduced order; no re-append spin (round-5 F1) |
| All pinned threads unpinned while reorder outbox live | `desiredOrder` empties ⇒ outbox cleared; `[]` is never sent; no 400 (round-5 F1) |
| Restart with outbox; server already equals desired (or empty) | Accepted-page-equals-desired settles without a write (round-5 F1) |
| Remote write raises floor; our 200/409 completes below floor | Flight ends, page discarded, no settle; floor-token resend once membership-quiescent (round-3 F1, round-5 F1) |
| Remote opposite pin/unpin at higher revision, local low-revision ack later | Per-id intent retires at completion; higher-revision page wins (LWW); nothing permanently hidden (round-3 F1) |
| Transient failure (network/429/5xx) | Local order kept; capped backoff with jitter (R5) |
| Permanent failure (400/401/403/404/405, old gateway) | Local order kept; requests paused; non-blocking pending-sync state; resume on env change (R5) |
| App restart with unsettled reorder | Durable outbox restored; local order overlays fetch; retry resumes (R5) |
| Gateway switch | Whole pinned-order domain swapped atomically (fresh floor 0); old-identity responses dropped; new gateway's pages adopted fresh (round-3 F4) |
| Out-of-order pages during a drag (rev12 then delayed rev11) | Acceptance runs before buffering; rev11 discarded; no re-flip on drop/cancel (round-4 V4-4) |
| Desktop: snapshot computed pre-drop, committed post-drop via startTransition | Commit-time rebase in the centralized ingress keeps local order (V2-3) |
| Desktop: stale transition commits *after* ack settle cleared the overlay | Envelope epoch/revision check rejects it at commit time (round-3 F2) |
| Desktop: request issued pre-drop, response lands after drop+settle | Stamp captured before the await (epoch 7 < current 9) ⇒ rejected (round-4 V4-1) |
| Desktop: renderer reload with a previous session's response in flight | `rendererSessionId` mismatch ⇒ dropped (round-4 V4-1) |
| Drop outside pinned segment | Clamped to segment edge; never unpins |

## 8. Delivery Batches

1. **B1 gateway**: schema ensure + versioned backfill + atomic
   `ThreadPinsPage` boundary + shared revision helper over all four delete
   sites + `reorder_thread_pins` + collection PUT + task_forest ordering +
   tests (`--all-targets`).
2. **B2 iOS**: architecture-gate spike (§5.1 points 1-7, incl. hitch gate) →
   Core state machine + outbox + tests → wiring + haptics + pending-sync
   surface → `xcodebuild test`. Ships with B1 in the same merge train.
3. **B3 desktop**: dnd-kit reorder + main-store guard + last-hop projection +
   contract + tests. Separate follow-up task after B1/B2 land.

## 9. Decisions Taken (review these explicitly)

1. Order carried by a dedicated `sort_order` on the direct-write `thread_pins`
   table (pins stay outside the projection contract). All ordering consumers
   (API *and* task_forest) switch to it.
2. Reorder endpoint is a **revision-CAS** collection PUT that never mutates
   membership; pages are **atomic snapshots** (page + revision from one
   transaction) and every pin-row mutation — including archive, runtime hard
   delete, and startup cleanup — bumps the revision through one shared
   helper.
3. Client authority = revision-monotonic page acceptance with **explicit
   outcomes** (`discardedBelowFloor | merged | authoritative`) + epoch guard
   that advances on local mutation **and on settle** + membership merges
   overlaid with per-id pin intents. **Transport completion is decoupled from
   page acceptance**: a below-floor completion frees the flight, never
   settles, and triggers an immediate floor-token resend of the unsettled
   outbox; per-id intents retire at completion with revision-bounded
   overlays (LWW). Responses run a fixed pipeline — identity → transport
   completion → revision acceptance → publication — and drag-freeze buffers
   hold accepted projections only, never raw arrivals. Reorder dispatch is
   **membership-gated** (`waitingForMembership` while per-id intents are
   live), settle comparisons use raw canonical order, any accepted
   page equal to `desiredOrder` settles the outbox, and an emptied
   `desiredOrder` clears it (no `[]` writes). This is the corrected form of
   the in-repo `GaryxCapsuleFavorites` pattern.
4. Reorder failure does **not** roll back the UI; the unsettled order is a
   durable, gateway-scoped outbox. Retries are **classified**: CAS loop for
   409, capped jittered backoff for transient errors, and a paused,
   non-blocking **pending-sync state** for permanent errors — not unbounded
   silent retry. The **entire pinned-order state is a per-gateway domain**
   (identity-tagged requests, per-gateway revision floor, atomic domain swap
   on switch).
5. Re-pin becomes idempotent (`pinned_at` and `sort_order` preserved, no
   revision bump) — deliberate behavior change; `pinned_at` means "first
   pinned at".
6. Recent-section ordering untouched; no cross-section drag semantics.
7. Native `List`/`onMove` over custom gestures, gated by a mandatory
   architecture spike proving lifecycle (began/accepted-drop/cancel),
   observation-only UIKit adaptation, menu arbitration, mid-drag injection
   stability, and a quantified hitch gate with recorded baselines; drag
   ordering is preview-only until an accepted drop; explicit drag-handle
   reorder mode is the sanctioned fallback. **iOS 17 verification is
   non-waivable**: the adapter path ships only to OS versions it was verified
   on; if no iOS 17 runtime is obtainable, iOS 17 gets the documented
   `EditMode` reorder mode via a version gate (raising the deployment target
   is an owner decision).
