# TASK-1802 — iOS home thread list: pull-to-refresh + load-more overhaul

Boss feedback: the Recent list's pull-to-refresh and scroll-to-bottom
load-more feel "janky" overall. This is an experience overhaul, not a spot
fix: this doc lists every concrete root cause with code evidence, then the
single design that removes them as a group.

Base: `origin/main` @ `3399f6ef`. All evidence re-verified against this tree.

Design v2 — addresses design-review findings (#TASK-1803): F1 pager
concurrency model (dual in-flight flags + epoch tickets, ordering matrix
tested), F2 explicit `RefreshSource`/error-presentation policy as tested
API, F3 R7 downgraded to "mitigated, not fixed" with the >overlap boundary
pinned, F4 xcodegen/pbxproj acceptance gate.

Design v3 — addresses the re-review finding: gate forgiveness is
revision-scoped (`loadMoreFailureRevision` + ticket-observed revision), so
the final gate is completion-order-commutative; the ordering matrix
asserts identical (gate, cursor, hasMore) for both completion orders.

Design v4 — addresses re-review 2: `.failed(attempts:)` counter removed
(no consumer; a current-gate-derived counter breaks order commutativity).
Gate is a three-value enum; failure freshness lives solely in
`loadMoreFailureRevision`, and the matrix asserts the full
(gate, revision, cursor, hasMore) tuple.

## Current architecture (for orientation)

- `GaryxMobileModel` (MainActor) owns list state: `threads`,
  `recentThreadIds`, `pinnedThreadIds`, `isLoadingThreads`,
  `isLoadingMoreThreads`, `hasMoreThreadSummaries`, and the non-published
  cursor `nextThreadListOffset` (`GaryxMobileModel.swift:137-144,446`).
- Row derivation is already off-main: model `didSet` →
  `emitHomeProjectionSnapshot()` → `HomeProjectionGateway` →
  `HomeProjectionActor` (background) → `GaryxHomeThreadListStore.snapshot`
  → `GaryxHomeThreadListView` (native `List`).
- Pagination flags flow separately: `isLoadingMoreThreads` /
  `hasMoreThreadSummaries` → `GaryxHomeObservationStore.applyPagination`
  → the auto-load footer.
- Gateway API: `GET /api/recent-threads?limit&offset` only — offset paging
  over the SQLite `recent_threads` projection ordered by recency
  (`garyx-gateway/src/routes.rs:1677`, `GaryxGatewayClient.swift:325`).
- Refresh entry points today: pull-to-refresh (`refreshable`), a 10s
  silent loop while home is visible
  (`GaryxMobileSidebarViews.swift:192-305`), a 15s background committed-run
  reconcile loop (`GaryxMobileModel+ThreadList.swift:374-429`), and ad-hoc
  `refreshThreads()` calls after user/run actions.

## Root causes (each with evidence)

### R1 — Pull-to-refresh awaits a 14-endpoint catalog sweep, serially

`GaryxHomeThreadListView` → `.refreshable { await refreshAll() }` →
`onRefreshAll` (`GaryxMobileViews.swift:32-35`):

```swift
onRefreshAll: {
    await model.refreshThreads(silent: true)
    await model.refreshRemoteState()
}
```

`refreshRemoteState()` (`GaryxMobileModel+Gateway.swift:520-604`) fans out
to agents, teams, skills, capsules, gateway settings, automations,
slash commands, MCP servers, channel endpoints, workspaces, configured
bots, bot consoles, and channel plugins — and `refreshAll` awaits the
whole thing *after* the thread refresh completes. The refresh spinner
stays down for the full catalog sweep even though the list content the
user asked for was fresh seconds earlier. On a slow gateway this is a
multi-second hang; HIG expects the indicator to end when the refreshed
content is in.

### R2 — Non-silent `refreshThreads()` truncates loaded pages → the big list jump

`refreshThreads(silent: false)` resets pagination to page one:

- `applyRecentThreadsPage(page, preservesLoadedPages: false)` →
  `recentThreadIds = pageIds` (first 30 ids only)
  (`GaryxMobileModel+ThreadList.swift:113-135`).
- `existingThreads = silent ? ... : []` — loaded tail summaries are also
  dropped from `threads` (`:40`).

Every parameterless `refreshThreads()` call takes this truncating path,
and they are all "an action happened, re-sync the list" semantics:

- run completion: `GaryxMobileModel+ThreadRunState.swift:191,276`
- archive thread: `GaryxMobileModel+ThreadLifecycle.swift:334`
- interrupt run: `GaryxMobileModel+Composer.swift:522`
- delete bot thread: `GaryxMobileModel+Bots.swift:295`
- foreground sync: `GaryxMobileModel+Gateway.swift:303`
- open-thread fallback: `GaryxMobileModel+AgentsWorkspaces.swift:131,189`

Scenario: user loads 3 pages (90 rows), scrolls to row 70, then archives a
row / a background run finishes / the app returns to foreground → the list
snaps back to 30 rows and the scroll position is gone. This is the single
biggest "list jumps" cause.

Also, these paths set `isLoadingThreads = true`, which (a) flashes the
skeleton placeholder if the recent section is momentarily empty
(`GaryxHomeThreadListSnapshot.recentPlaceholder`) and (b) blocks load-more
via the guard in R4.

### R3 — Load-more triggers only when a 1pt sentinel is already on screen

`GaryxSidebarThreadAutoLoadFooter` (`GaryxMobileSidebarViews.swift:769-796`):

```swift
} else if homeObservationStore.hasMoreThreadSummaries {
    Color.clear.frame(height: 1)
        .onAppear { Task { await loadMoreThreads() } }
}
```

There is no prefetch distance: the fetch starts only after the user has
physically hit the bottom, so every page boundary is a visible stall.
Native lists prefetch when the user is *near* the tail (a few rows out).

### R4 — `.onAppear` is consumed once; guards turn that into missed triggers

`loadMoreThreads()` (`GaryxMobileModel+ThreadList.swift:206-215`) guards on
`!isLoadingThreads && !isLoadingMoreThreads`. When a non-silent refresh is
in flight (`isLoadingThreads == true`), the footer's `.onAppear` fires,
the call is rejected, and — because the sentinel stays on screen and
`.onAppear` does not re-fire for an unchanged identity — nothing ever
retries. The user sits at the bottom of a list that silently refuses to
grow until they scroll away and back.

### R5 — Load-more failure = infinite retry storm + toast spam

On error (`GaryxMobileModel+ThreadList.swift:233-236`):
`lastError = displayMessage(for: error)` (global toast),
`hasMoreThreadSummaries` stays `true`, and `isLoadingMoreThreads` returns
to `false` via `defer`. The footer branch flips spinner →
`Color.clear`, which is a *new* view identity, so `.onAppear` fires again
immediately → another request → another failure → repeat. With the
gateway briefly unreachable this loops at network RTT, firing a global
error toast per iteration. There is no failure state, no backoff, no
inline retry affordance.

### R6 — Footer geometry jumps between states

Same footer: spinner branch is `minHeight: 44`, sentinel branch is
`height: 1`, exhausted is 0. Every load completion shifts bottom content
by ~43pt while the user is anchored at the bottom — a per-page visible
hop.

### R7 — Offset paging drifts: deletions/archives skip rows

The cursor is a running server offset (`nextThreadListOffset =
page.offset + page.count`, `:239-252`) against a recency-ordered list.
Head *insertions* shift the window left → duplicates → already absorbed
by `seenRecentIds` dedup (`:227-230`). But *removals* inside the seen
range (archive/delete on this or another client) shift the unseen region
left → the next `[offset ..< offset+30)` window **skips** a never-seen
thread. Nothing re-surfaces it until that thread happens to update into
the head window.

### R8 — Pagination state machine lives in the app target, untested

The cursor, flags, merge rules (`applyRecentThreadsPage` head-merge,
`loadMoreThreads` dedup-append, `updateThreadListPagination`) are spread
across `GaryxMobileModel+ThreadList.swift` in the app target. `swift test`
covers only `Tests/GaryxMobileCoreTests` (`Package.swift:17-23`), and no
existing test touches thread-list pagination (grep: only observation-store
plumbing). Exactly the state/UI separation this task requires is missing.

### R9 — Concurrent refresh entry points re-enter freely

Pull-to-refresh, the 10s visible loop, the 15s reconcile loop, and action
refreshes can all run `refreshThreads` concurrently (silent refreshes set
no flag that others check; `canRefreshSidebarThreads()` only checks
`isLoadingThreads`/`isLoadingMoreThreads`). Interleaved awaits produce
redundant gateway calls and interleaved state writes. Mostly harmless
today because merges are idempotent — but it is unowned behavior, and the
new single-writer pager needs it explicit.

### R10 — Offline silent refresh toasts every 10s

`refreshThreads` failure always sets `lastError` (global toast) — including
the 10s silent loop, so a gateway blip/offline stretch fires a toast every
cycle. The hydrate path already has the right pattern: transient gateway
errors degrade to `gatewaySettingsStatus` ("Waiting to sync with gateway",
`GaryxMobileModel+ThreadList.swift:364-371`).

## Design

One state machine in Core owns paging/refresh/failure state; the model
keeps IO only; the footer becomes a fixed-height, state-rendered row; all
refreshes become non-destructive head-merges.

### 1. `GaryxHomeThreadListPager` (new, `Sources/GaryxMobileCore`)

Pure, synchronous, `Equatable` state + decision functions. No IO, no
Combine, no timers — the model asks it for decisions and reports results.

Head refresh and load-more are **two independent in-flight tracks**, not
one `phase` (design-review F1): a refresh is a background-cadence event
(10s loop) and must neither destroy a load-more in flight nor be starved
by one, and vice versa — a single phase enum cannot represent "both in
flight" without dropping one. Each track self-coalesces; they never block
each other. Stale completions (after `reset()` on gateway switch) are
rejected structurally via epoch-stamped tickets instead of caller-side
generation guards.

```swift
public struct GaryxHomeThreadListPager: Equatable, Sendable {
    public enum LoadMoreGate: Equatable, Sendable {
        case ready
        case exhausted                       // server said has_more == false
        case failed                          // last load-more failed
    }
    // No attempt counter: nothing consumes it (the footer shows a retry
    // affordance regardless of count, and there is deliberately no
    // automatic backoff — §5). Failure freshness lives in
    // loadMoreFailureRevision, which is order-commutative; a counter
    // derived from "current gate + 1" is not (re-review round 2).

    public private(set) var epoch: Int              // bumped by reset()
    public private(set) var isRefreshingHead: Bool
    public private(set) var isLoadingMore: Bool
    public private(set) var gate: LoadMoreGate
    public private(set) var loadMoreFailureRevision: Int  // bumped by every failLoadMore
    public private(set) var nextOffset: Int          // 0 = not primed (no head page yet)
    public let pageLimit: Int
    public let overlap: Int                          // see §4

    // Decisions
    public mutating func requestRefresh() -> GaryxThreadListRefreshTicket?
        // nil → refresh already in flight (concurrent entry points coalesce — R9)
    public mutating func requestLoadMore(trigger: LoadMoreTrigger) -> GaryxThreadListLoadMoreTicket?
        // nil → not primed (nextOffset == 0) / already loading more /
        //        gate exhausted / gate failed (automatic triggers only)
    public mutating func retryLoadMore() -> GaryxThreadListLoadMoreTicket?
        // explicit user tap on the failed footer; bypasses only the .failed gate
    // Results (ticket.epoch != epoch → no-op: stale completion after reset)
    public mutating func completeRefresh(
        _ ticket: GaryxThreadListRefreshTicket,
        pageOffset: Int, pageCount: Int, hasMore: Bool
    )
    public mutating func failRefresh(_ ticket: GaryxThreadListRefreshTicket)
    public mutating func completeLoadMore(
        _ ticket: GaryxThreadListLoadMoreTicket,
        pageOffset: Int, pageCount: Int, hasMore: Bool
    )
    public mutating func failLoadMore(_ ticket: GaryxThreadListLoadMoreTicket)
    public mutating func reset()                     // gateway switch: epoch += 1, all state initial
}

public enum LoadMoreTrigger: Equatable, Sendable { case nearTail, footer }
public struct GaryxThreadListRefreshTicket: Equatable, Sendable {
    public let epoch: Int
    /// loadMoreFailureRevision observed when this refresh was issued. A
    /// successful refresh may only forgive failures it already knew about
    /// (see the failure-forgiveness rule below).
    public let observedLoadMoreFailureRevision: Int
}
public struct GaryxThreadListLoadMoreTicket: Equatable, Sendable {
    public let epoch: Int
    public let offset: Int   // max(0, nextOffset - overlap)
    public let limit: Int
}
```

Gate rules (all unit-tested):

- `requestLoadMore` returns `nil` while `isLoadingMore` (self-coalesce),
  while `nextOffset == 0` (not primed — replaces today's `offset > 0`
  guard), while `gate == .exhausted`, and while `gate == .failed`
  (automatic triggers are rejected after a failure; only
  `retryLoadMore()` — the explicit footer tap — or a head refresh
  issued after the failure re-arms it). This kills the R5 retry storm by construction and
  makes R4's "rejected once = lost forever" impossible: triggers are
  cheap and re-evaluated from state, so a rejection is not a consumed
  opportunity. A refresh in flight does **not** block load-more — that
  is exactly the R4 drop scenario.
- `requestRefresh` returns `nil` while `isRefreshingHead` (R9 coalesce).
  A load-more in flight does not block refresh.
- **Concurrent write safety** (both tracks in flight): all state writes
  happen on the MainActor, so completions serialize; the two completion
  paths write through the two merge functions below — refresh completion
  head-merges ids and touches the cursor only in the reset case
  (`nextOffset <= pageOffset + pageCount`, see next bullet), load-more
  completion appends ids and always advances the cursor from the
  server-returned `offset + count`. The ordering matrix
  (refresh-start → loadMore-start → either completion order, and each
  with either/both failing) is pinned in tests — see test plan §3.
- **Ticket completion is the transaction end** (code-review
  #TASK-1804): the model calls `completeRefresh`/`failRefresh` only
  after its *last* await — the in-flight flag must span the whole
  App-layer refresh (page fetch **and** the pinned/selected summary
  backfill), not just the page request. Completing early would re-open
  the gate mid-transaction, letting a second refresh land newer state
  that the first, still-suspended refresh then overwrites with its
  older page on resume. Same rule for load-more tickets.
- **Local surgery invalidates in-flight commits on both tracks**
  (code-review #TASK-1804 rounds 2–4): the pager carries
  `localMutationSequence`, bumped via `noteLocalMutation()` whenever
  the model performs local list surgery (archive/delete local removals,
  pin edits and their rollbacks). Refresh **and load-more** tickets
  record the observed value; `completeRefresh` / `completeLoadMore`
  return `.abandonedLocalMutation` (gate/cursor/revisions untouched,
  in-flight flag released) when they differ. The model responds by
  dropping the page and following up: refresh re-runs with the same
  source; load-more re-requests the same window (the cursor never
  advanced). Why re-filtering at the commit point is not enough: an
  archive that *succeeds* resolves its tombstone before its own
  follow-up refresh — which the stale in-flight refresh coalesced
  away — so the surgery marker must outlive the tombstone. And why
  load-more cannot rely on dedup: an overlapped page fetched before a
  removal still contains the removed row, and appending it against the
  post-surgery list — which no longer has the id — would resurrect it
  as a "new" row. Both pinned by tests.
- `completeRefresh` cursor semantics replace today's two-condition
  `hasLoadedBeyondHead` (`nextThreadListOffset > returnedEnd ||
  recentThreadIds.count > pageIds.count`,
  `GaryxMobileModel+ThreadList.swift:116-117`) with the single pager-own
  condition `nextOffset > pageOffset + pageCount`:
  - beyond head (`nextOffset > returnedEnd`) → keep cursor and `hasMore`
    (head content merges, pagination untouched);
  - not beyond head → adopt `offset + count` / `hasMore` from the page.
  Deliberate micro-change: when only one page was ever loaded and a
  drifted head page arrives (31 merged ids vs 30 page ids), today's
  list-length condition keeps the dropped id; the new rule lets the
  head page own the list (the dropped row returns via load-more). Both
  are harmless; the new rule means "no load-more yet ⇒ list == server
  head page", is decidable from pager state alone, and is pinned by a
  test.
- **Failure forgiveness is revision-scoped** (re-review finding): every
  `failLoadMore` bumps `loadMoreFailureRevision`. A successful
  `completeRefresh` clears `.failed` back to `.ready` **only when**
  `ticket.observedLoadMoreFailureRevision == loadMoreFailureRevision` —
  i.e. a refresh forgives exactly the failures that existed when it was
  issued (it proved the gateway reachable *after* them), never a failure
  produced by a load-more that was still in flight while it ran. This
  makes the final gate independent of completion order — with both
  tracks in flight and the load-more failing:
  - load-more fails first (revision bumps) → refresh completes
    (observed revision is stale) → `.failed` survives;
  - refresh completes first (gate not `.failed`, nothing to forgive) →
    load-more fails → `.failed`.
  Both orders end `.failed`. Conversely, when the failure pre-dates the
  refresh (`.failed` at issue time, revisions equal), the refresh
  re-arms `.ready` in either order — the intended "gateway is back"
  recovery. Pinned by the ordering-matrix tests (test plan §3).
- `failLoadMore` → `.failed` + revision bump (idempotent in shape — no
  counter, so the write commutes with any interleaved forgiveness); the
  pager never re-arms itself without new evidence (retry tap or a head
  refresh issued after the failure).
- Any `complete*/fail*` whose `ticket.epoch != epoch` is a no-op — an
  in-flight response from before a gateway switch cannot corrupt the new
  gateway's state (today this relies on caller-side `runtimeGeneration`
  guards only).

Also moved into Core as pure functions (currently inline in the model,
untestable):

```swift
public enum GaryxThreadListPageMerge {
    /// Head refresh: new first page wins the head, previously loaded tail
    /// keeps its order, minus ids that moved into the head.
    public static func mergeHead(pageIds: [String], existingIds: [String]) -> [String]
    /// Load-more: append page ids not already present (absorbs the
    /// overlap window and head-insert drift duplicates).
    public static func appendPage(pageIds: [String], existingIds: [String]) -> [String]
    /// Prefetch sentinel: id of the row whose appearance should trigger
    /// a near-tail load (K rows from the end), nil for short lists.
    public static func prefetchTriggerRowId(recentIds: [String], prefetchDistance: Int) -> String?
}
```

### 2. Refresh never truncates; `silent` loses its destructive meaning

`refreshThreads` becomes head-merge-only (today's `silent: true` merge
path is the only path). The `silent` parameter is deleted; callers pass a
`GaryxThreadListRefreshSource` instead (§7), and the two things `silent`
used to bundle get their own owners: skeleton visibility is derived from
list emptiness (`GaryxThreadListRefreshPolicy.showsSkeleton`), failure
presentation from the source. All current call sites (R2 list) map to
`.userAction` and get the non-destructive behavior automatically. Full
reset stays where it already lives: `resetThreadListPagination()` +
`recentThreadIds = []` on gateway switch
(`GaryxMobileModel+Messages.swift:123-127`,
`GaryxMobileModel+Gateway.swift:109`) — now also calling `pager.reset()`.

`isLoadingThreads` consequently stops blocking load-more (the pager's
own in-flight flags are the only mutex), removing R4's guard-drop.

Memory note: with truncation gone, `threads` grows monotonically within a
gateway session (summaries are small; hundreds of rows ≈ a few hundred
KB). The existing LRU/reset points (gateway switch, app relaunch) bound
it; no extra cap is added now.

### 3. Pull-to-refresh awaits only the list

```swift
onRefreshAll: {
    Task { await model.refreshRemoteState() }        // kick catalogs, don't gate
    await model.refreshThreads(source: .userPullToRefresh)
}
```

`refreshable` awaits `refreshThreads()` only; `refreshRemoteState()` is
kicked as an unstructured `Task` — awaiting it in any form (serial or
`async let`) would keep the spinner down for the catalog sweep, which is
R1 itself. It has its own `requestId` supersede logic and is safe to
overlap. The spinner ends when the list is fresh. Catalog data (avatars,
agent names) lands moments later via its own published properties — same
as every other surface that refreshes catalogs independently.

### 4. Drift-absorbing overlap window (R7 — mitigated, not fixed)

`LoadMoreTicket.offset = max(0, nextOffset - overlap)` with
`overlap = 5`. Removal-drift of **up to 5 rows** between two consecutive
load-more requests is absorbed: the re-fetched seen rows dedup away
(existing `appendPage` behavior), and a row that slid left into the
window is no longer skipped. `overlap` rides on the existing dedup,
needs no gateway change, and costs 5 rows per page.

Scope statement (review F3): this **mitigates** R7, it does not remove
it. More than `overlap` removals between two pages still skip rows; the
residual is bounded, low-probability (requires 6+ archives/deletions
between two consecutive page fetches), and self-heals when a skipped
thread updates into the head window. The complete fix is recency-cursor
(`before`-style) pagination on the gateway — a cross-surface API change
explicitly out of scope for this iOS task; tracked as follow-up. The
`> overlap` boundary (6 removals ⇒ exactly one skipped row) is pinned by
a test so the residual is documented behavior, not an accident.

### 5. Footer: fixed-height, state-rendered, with an explicit failure affordance

`GaryxSidebarThreadAutoLoadFooter` renders from a single
`GaryxHomeLoadMoreFooterState` (Core enum derived from pager state, so it
is testable):

```swift
public enum GaryxHomeLoadMoreFooterState: Equatable, Sendable {
    case hidden          // exhausted (or nothing loaded yet)
    case idle            // more available; sentinel row, visually empty
    case loading         // spinner + "Loading more"
    case failed          // "Couldn't load more · Tap to retry" (Button)
}
```

- The footer row has a **constant 44pt height** for `idle`/`loading`/
  `failed` — state flips change content, never geometry (R6). `hidden`
  removes the row; that one-time collapse happens only when the last page
  arrives, animated by the List's default row animation.
- `failed` is a tappable row calling `retryLoadMore()`; load-more errors
  no longer write `lastError` (no global toast for a bottom-of-list
  concern; the footer is the feedback) — R5.
- The footer keeps an `.onAppear` trigger (`.footer`) as the fallback for
  short lists / fast flings, but it is now stateless-safe: rejection by
  the pager costs nothing and the row re-arms whenever pager state
  changes because footer content is identity-keyed by state.

### 6. Near-tail prefetch (R3)

The sentinel row id is derived from the recent section via the Core pure
function `prefetchTriggerRowId(recentIds:prefetchDistance: 6)`.
Implementation note: it is computed in the list view's body (once per
snapshot publish, O(n) over row ids) rather than added as a
`GaryxHomeThreadListSnapshot` field — same Core-tested derivation and the
same "UI only binds" split, without threading a new field through both
the actor and legacy projection paths and their parity checkpoints. The
recent-row `ForEach` adds:

```swift
.onAppear {
    if row.id == snapshot.prefetchTriggerRowId { onLoadMoreTrigger(.nearTail) }
}
```

Appearing rows already stream `.onAppear` during scroll (native List cell
lifecycle), so this costs one string compare per appearance. Triggering
through the pager gate makes duplicate fires free. Result: the next page
is usually in before the user reaches the bottom; the footer spinner only
shows on fast flings.

### 7. Explicit refresh source + error-presentation policy (R10, review F2)

Today one boolean (`silent`) conflates three orthogonal questions — does
this refresh truncate, does it drive the skeleton, does its failure
toast — and the call sites prove it: `silent: true` covers
pull-to-refresh (`GaryxMobileViews.swift:32`), the 10s loop
(`GaryxMobileSidebarViews.swift:293`), and the 15s reconcile
(`GaryxMobileModel+ThreadList.swift:409,427`), which need *different*
failure behavior. The `silent` parameter is deleted and replaced by an
explicit source, and each question gets its own owner:

```swift
public enum GaryxThreadListRefreshSource: Equatable, Sendable {
    case userPullToRefresh   // user dragged the list
    case userAction          // run completion, archive, interrupt, foreground
                             // sync, open-thread fallback, connect
    case backgroundLoop      // 10s visible loop, 15s reconcile
}

public enum GaryxThreadListRefreshFailurePresentation: Equatable, Sendable {
    case toast               // global error toast (lastError)
    case transientStatus     // gatewaySettingsStatus = "Waiting to sync with gateway"
}

public enum GaryxThreadListRefreshPolicy {
    public static func failurePresentation(
        source: GaryxThreadListRefreshSource
    ) -> GaryxThreadListRefreshFailurePresentation
    public static func showsSkeleton(listIsEmpty: Bool) -> Bool
}
```

Pinned policy table (Core-tested, exhaustive over the product):

| source | failure presentation |
|---|---|
| `userPullToRefresh` | `.toast` — the user asked and deserves the answer |
| `userAction` | `.toast` — an explicit action visibly failed |
| `backgroundLoop` | `.transientStatus` — never toast from a timer: offline ⇒ one low-key status line ("Waiting to sync with gateway", same surface the hydrate path already uses, `GaryxMobileModel+ThreadList.swift:364-371`) instead of a toast per 10s cycle. Deliberately simpler than the hydrate path's transient-only check: a timer-initiated failure never toasts regardless of error kind — a non-transient error (e.g. auth) will surface as a toast on the user's next explicit action. |

- Truncation: nobody — refresh never truncates (§2), so no source can
  reintroduce R2.
- Skeleton: `showsSkeleton = listIsEmpty` — a first-load presentation
  concern derived from list content, independent of source (a background
  refresh that races a cold start may legitimately drive the skeleton;
  a pull-to-refresh on a populated list never does).
- Load-more failures use neither: the footer is the feedback (§5).

The model's signature becomes
`refreshThreads(source: GaryxThreadListRefreshSource)`; every current
call site maps 1:1 (`silent: true` pull path → `.userPullToRefresh`,
loops → `.backgroundLoop`, parameterless action calls → `.userAction`),
so review can verify the mapping mechanically.

### 8. What does NOT change

- The projection pipeline (model → gateway → actor → store → List) —
  pagination state simply feeds it as before via
  `GaryxHomeObservationStore.applyPagination` + snapshot fields.
- Row identity/reuse, sections builder, running-state overlay, widget
  snapshot persistence.
- The Mac desktop app (Electron; shares nothing with these Swift paths).
- Gateway API (offset paging stays; cursor pagination is a noted
  follow-up).
- The 10s/15s background cadences.

## Test plan (SwiftPM, `Tests/GaryxMobileCoreTests`)

`GaryxHomeThreadListPagerTests` — every rule pinned with realistic page
shapes (30-id pages, real `GaryxRecentThreadsPage`-decoded fixtures where
shape matters):

1. Initial refresh: cursor `0→30`, `hasMore` mirrors payload; not-primed
   gate — `requestLoadMore` returns nil before the first
   `completeRefresh`.
2. Load-more happy path: ticket offset at `nextOffset - overlap`, floor
   at 0; `completeLoadMore` advances cursor from server-returned
   `offset+count` (not the overlapped request), `hasMore` applied.
3. **Concurrency ordering matrix** (review F1 + both re-reviews): with
   both tracks in flight — refresh-start → loadMore-start, then each
   completion order (`completeRefresh` first / `completeLoadMore`
   first), and each of the four success/failure combinations — assert
   both in-flight flags resolve independently and **the final
   (gate, loadMoreFailureRevision, cursor, hasMore) tuple is identical
   for both completion orders of every combination**. In particular:
   load-more fails + refresh succeeds ⇒ `.failed` in both orders
   (revision-scoped forgiveness — the refresh was issued before the
   failure, so it cannot forgive it). Separately, the re-review-2
   counterexample pinned: gate already `.failed` when a refresh and a
   retry are both issued (revisions equal at issue time) —
   retry-fails-first (revision bumps; the refresh's observed revision is
   stale, no forgiveness) and refresh-succeeds-first (re-arms `.ready`;
   the retry failure then re-enters `.failed`) both end at
   `(.failed, revision + 1)` — identical in both orders.
4. Self-coalescing: second `requestLoadMore` while `isLoadingMore` → nil
   (no double fire); second `requestRefresh` while `isRefreshingHead` →
   nil; a refresh in flight does **not** block `requestLoadMore` and a
   load-more in flight does not block `requestRefresh`; after completion
   the next request is granted (no lost trigger).
5. Failure ladder: `failLoadMore` → `.failed` + revision bump;
   automatic triggers (`.nearTail`, `.footer`) rejected; `retryLoadMore`
   granted → still `.failed` and revision bumps again on second failure;
   a head refresh issued *after* the failure re-arms `.ready` and
   preserves the cursor; a refresh issued *before* the failure does not
   (stale observed revision).
6. Exhaustion: `hasMore == false` → `.exhausted`; triggers rejected;
   footer state `.hidden`; preserved across beyond-head refreshes (the
   `nextOffset > returnedEnd` path does not resurrect `hasMore`).
7. Reset + stale tickets (review F1): `reset()` bumps `epoch` and
   returns all state to initial; a `complete*/fail*` carrying a
   pre-reset ticket is a no-op on every field.
8. Cursor semantics change pinned: single-page list + drifted head page
   (31 merged ids vs 30 page ids, `nextOffset == 30`) adopts the page
   cursor (documented micro-change from today's list-length condition);
   multi-page list (`nextOffset == 90`) keeps cursor and `hasMore`.
9. `mergeHead`: new head wins, tail order preserved, ids promoted into
   head not duplicated in tail (pin today's `applyRecentThreadsPage`
   merge behavior on a captured id-list fixture).
10. `appendPage` + drift: dedup absorbs overlap and head-insert
    duplicates; removal-drift within budget — seen `[t1..t60]`, 5
    removals, next page with overlap 5 → nothing skipped; **boundary
    beyond budget** (review F3) — 6 removals → exactly one row skipped
    (documented residual, asserted so the mitigation limit is pinned).
11. `prefetchTriggerRowId`: K-from-end id; `nil` under K rows; updates as
    pages append.
12. Footer state derivation: pager state × gate → footer enum,
    exhaustive over all reachable combinations.
13. Refresh policy (review F2): `failurePresentation(source:)` full
    table (all 3 sources); `showsSkeleton(listIsEmpty:)` both branches.

App-target binding stays thin enough to eyeball in review (model calls
pager decisions, footer renders the enum); `xcodebuild` simulator build
verifies it compiles and wires.

Acceptance gate for new files (review F4): this repo's Xcode project is
generated — new Core/test Swift files require `xcodegen generate` in
`mobile/garyx-mobile` and committing the regenerated
`GaryxMobile.xcodeproj/project.pbxproj` (project.yml includes
`Sources/GaryxMobileCore` in the app target; a green `swift test` alone
can be a false pass). Verification order: `swift test` (SwiftPM) →
`xcodegen generate` → `xcodebuild` iOS-Simulator build of the app
target.

## Rollout / risk

- Single PR; no feature flag (pure client behavior, no data migration).
- Biggest behavioral change is R2 (no more truncation). Cold start and
  gateway switch paths were audited: both run with empty
  `threads`/`recentThreadIds`, where head-merge ≡ replace, and the
  explicit reset path already exists and keeps working.
- The pager is additive state; if an unforeseen regression appears, the
  old flags (`isLoadingMoreThreads`, `hasMoreThreadSummaries`) remain as
  derived outputs feeding the same observation store, so surfaces other
  than the footer are untouched.

## Follow-ups (explicitly out of scope)

- Gateway cursor pagination (`before=<updated_at cursor>`) to fully
  eliminate offset drift (R7 residual: >5-row drift between two pages).
- Scroll-anchor compensation for head reorders during active scrolling
  (product-intended recency reorder; low observed impact once R2's
  truncation is gone).
