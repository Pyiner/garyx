# TASK-1802 — iOS home thread list: pull-to-refresh + load-more overhaul

Boss feedback: the Recent list's pull-to-refresh and scroll-to-bottom
load-more feel "janky" overall. This is an experience overhaul, not a spot
fix: this doc lists every concrete root cause with code evidence, then the
single design that removes them as a group.

Base: `origin/main` @ `3399f6ef`. All evidence re-verified against this tree.

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
to agents, teams, skills, capsules, dreams, gateway settings, automations,
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

```swift
public struct GaryxHomeThreadListPager: Equatable, Sendable {
    public enum LoadPhase: Equatable, Sendable {
        case idle
        case refreshing          // any head refresh in flight
        case loadingMore
    }
    public enum LoadMoreGate: Equatable, Sendable {
        case ready
        case exhausted                       // server said has_more == false
        case failed(attempts: Int)           // last load-more failed
    }

    public private(set) var phase: LoadPhase
    public private(set) var gate: LoadMoreGate
    public private(set) var nextOffset: Int
    public private(set) var pageLimit: Int
    public private(set) var overlap: Int     // see §4

    // Decisions
    public mutating func requestRefresh() -> Bool            // false → coalesce into in-flight
    public mutating func requestLoadMore(trigger: LoadMoreTrigger) -> LoadMoreRequest?
    // Results
    public mutating func completeRefresh(pageOffset: Int, pageCount: Int, hasMore: Bool, resetsCursor: Bool)
    public mutating func failRefresh()
    public mutating func completeLoadMore(pageOffset: Int, pageCount: Int, hasMore: Bool)
    public mutating func failLoadMore()
    public mutating func retryLoadMore() -> LoadMoreRequest?  // user tap on failed footer
    public mutating func reset()                              // gateway switch
}

public enum LoadMoreTrigger: Equatable, Sendable { case nearTail, footer }
public struct LoadMoreRequest: Equatable, Sendable {
    public var offset: Int   // max(0, nextOffset - overlap)
    public var limit: Int
}
```

Gate rules (all unit-tested):

- `requestLoadMore` returns `nil` while `phase != .idle`, while
  `gate == .exhausted`, and while `gate == .failed` (automatic triggers
  are rejected after a failure; only `retryLoadMore()` — the explicit
  footer tap — or a completed refresh re-arms it). This kills the R5
  retry storm by construction and makes R4's "rejected once = lost
  forever" impossible: triggers are cheap and re-evaluated from state, so
  a rejection is not a consumed opportunity.
- `requestRefresh` returns `false` while `phase == .refreshing`
  (concurrent entry points coalesce — R9); a refresh during
  `.loadingMore` is allowed (both write disjoint regions: head-merge vs
  append; MainActor serializes state writes).
- `completeRefresh(hasMore:)` only applies `hasMore`/cursor when
  `resetsCursor` (initial load, i.e. nothing loaded beyond head) —
  preserving today's `hasLoadedBeyondHead` semantics — and always clears
  `.failed` back to `.ready` (a successful head refresh is evidence the
  gateway is reachable again).
- `failLoadMore` → `.failed(attempts+1)`; the pager never re-arms itself
  without new evidence (retry tap or successful refresh).

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
path is the only path). The `silent` parameter's remaining meaning —
"set `isLoadingThreads`" — is renamed to what it actually is:
`isInitialThreadListLoad`-style skeleton control, derived inside
`refreshThreads` (`threads`/`recentThreadIds` empty → show skeleton) and
no longer a caller decision. All parameterless call sites (R2 list) keep
calling `refreshThreads()` and get the non-destructive behavior
automatically. Full reset stays where it already lives:
`resetThreadListPagination()` + `recentThreadIds = []` on gateway switch
(`GaryxMobileModel+Messages.swift:123-127`,
`GaryxMobileModel+Gateway.swift:109`) — now also calling `pager.reset()`.

`isLoadingThreads` consequently stops blocking load-more (the pager's
`phase` is the only mutex), removing R4's guard-drop.

Memory note: with truncation gone, `threads` grows monotonically within a
gateway session (summaries are small; hundreds of rows ≈ a few hundred
KB). The existing LRU/reset points (gateway switch, app relaunch) bound
it; no extra cap is added now.

### 3. Pull-to-refresh awaits only the list

```swift
onRefreshAll: {
    async let catalogs: Void = model.refreshRemoteState()   // fire, don't gate
    await model.refreshThreads()
    _ = await catalogs  // structured concurrency: keep, but see below
}
```

Actually gating on `catalogs` would keep the spinner down (same R1), so
the design is: `refreshable` awaits `refreshThreads()` only;
`refreshRemoteState()` is kicked as an unstructured `Task` (it has its
own `requestId` supersede logic and is safe to overlap). The spinner ends
when the list is fresh. Catalog data (avatars, agent names) lands moments
later via its own published properties — same as every other surface that
refreshes catalogs independently.

### 4. Drift-absorbing overlap window (R7)

`LoadMoreRequest.offset = max(0, nextOffset - overlap)` with
`overlap = 5`. Removal-drift up to 5 rows between pages is absorbed: the
re-fetched seen rows dedup away (existing `appendPage` behavior), and a
row that slid left into the window is no longer skipped. `overlap` rides
on the existing dedup, needs no gateway change, and costs 5 rows per
page. Cursor (`before`-style) pagination on the gateway is the complete
fix but is out of scope here; noted as follow-up.

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

`GaryxHomeThreadListSnapshot` gains `prefetchTriggerRowId` (derived in the
projection from `recentIds` via `prefetchTriggerRowId(recentIds:
prefetchDistance: 6)`). The recent-row `ForEach` adds:

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

### 7. Offline silent refresh stops toasting (R10)

`refreshThreads` failure handling adopts the hydrate path's pattern: if
`Self.isTransientGatewayErrorMessage(message)` and the refresh was a
background one (10s loop / reconcile), set
`gatewaySettingsStatus = "Waiting to sync with gateway"` instead of
`lastError`. User-initiated pull-to-refresh failures still toast — the
user asked and deserves the answer.

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

1. Initial refresh: cursor `0→30`, `hasMore` mirrors payload; skeleton
   input state (empty list) vs non-empty.
2. Load-more happy path: request at `nextOffset - overlap`, floor at 0;
   `completeLoadMore` advances cursor from server-returned
   `offset+count` (not the overlapped request), `hasMore` applied.
3. Re-entrancy: `requestLoadMore` during `.loadingMore` → nil (no double
   fire); during `.refreshing` → nil; after completion → granted (no
   lost trigger).
4. Refresh coalescing: second `requestRefresh` while `.refreshing` →
   false; completion returns pager to `.idle`.
5. Failure ladder: `failLoadMore` → `.failed(1)`; automatic triggers
   (`.nearTail`, `.footer`) rejected; `retryLoadMore` granted →
   `.failed(2)` on second failure; successful head refresh re-arms
   `.ready` and preserves cursor.
6. Exhaustion: `hasMore == false` → `.exhausted`; triggers rejected;
   footer state `.hidden`; preserved across head refreshes
   (`resetsCursor == false` path does not resurrect `hasMore`).
7. Reset: gateway switch → offsets/gate/phase back to initial.
8. `mergeHead`: new head wins, tail order preserved, ids promoted into
   head not duplicated in tail (pin today's `applyRecentThreadsPage`
   merge behavior byte-for-byte on a captured id-list fixture).
9. `appendPage`: dedup absorbs overlap and head-insert duplicates;
   removal-drift scenario — seen `[t1..t60]`, server deletes `t10`,
   next page fetched with overlap 5 → the row that slid into the raw
   window is appended, nothing skipped.
10. `prefetchTriggerRowId`: K-from-end id; `nil` under K rows; updates as
    pages append.
11. Footer state derivation: pager state × hasMore → footer enum,
    exhaustive.

App-target binding stays thin enough to eyeball in review (model calls
pager decisions, footer renders the enum); `xcodebuild` simulator build
verifies it compiles and wires.

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
