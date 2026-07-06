# TASK-1751 — iOS chat pipeline: five audited fixes

Fixes five audit-confirmed problems in the mobile chat pipeline. Every problem
was first reproduced/quantified deterministically without UI in SwiftPM tests
(`Tests/GaryxMobileCoreTests/GaryxChatPipelineReproTests.swift`); the numbers
below come from those tests.

Measurement environment: `swift test` (debug) on an arm64 Mac. Debug builds
overstate absolute cost vs a release app build, but an A-series/older iPhone
CPU is far slower than an M-series Mac; the *orders of magnitude* are what
matter, and they are all ≥ one 60fps frame budget (16ms) on the main thread.

## Reproduction results (before)

| # | What was measured | Result |
|---|---|---|
| P1 | `store.load()` (Data(contentsOf:) + JSONDecoder) + `mobileMessages` map for a 2,800-row / 4.4MB cached thread — the exact synchronous work `showSelectedThread` does on the main actor | 26–35ms (first cold run 181ms); at audit scale 10,000 rows / 29.8MB: **132–136ms** |
| P2 | One `GaryxMobileRenderStateMapper.rows()` call for 700 turn rows (2,800 transcript messages) — rebuilt from scratch on *every* call | **55–137ms per call**, and the conversation body triggers ≥2 calls per SwiftUI body evaluation (body + `.onChange`). Purity verified: identical inputs ⇒ identical output (caching precondition) |
| P3 | Structural: `messageScroll` uses `ScrollView` + eager `VStack`; `GaryxMobileTurnRowsView` `ForEach`es *all* rows. Each older-history page prepends 100 committed rows; a 30-page-deep browse instantiates ~3,000 row views, every one re-laid-out on every streaming flush | quantified via row counts; no UI metric exists in SwiftPM, see design §P3 |
| P4 | Structural: `messagesByThread`, `messageSignaturesByThread`, `cachedTranscriptSnapshots`, `renderSnapshotsByThread`, `activeAssistantMessageIdsByThread` are unbounded per-thread dictionaries, only cleared in `resetForGatewaySwitch`. With P1-scale threads, ~6 visited big threads ≈ 100s of MB resident | code-fact + arithmetic |
| P5 | `GaryxTranscriptFileCacheStore.save()` failure injected at the atomic-replace step (`FileManager` subclass throwing from `replaceItem`) | **RED test failing on current code**: `testP5_saveReplaceFailureCleansUpTemporaryFile` — `_ = try?` swallows the error, the `.json.tmp` file leaks (nothing ever sweeps it), no signal is emitted, stale window remains masquerading as current |

Evidence call chains (verified in source):

- P1: `GaryxMobileModel+ThreadLifecycle.swift:80-94` (`showSelectedThread`,
  `@MainActor`) → `restoredCachedMessages` (`+TranscriptCache.swift:48`) →
  `transcriptSnapshot(for:)` (`:11`, synchronous) →
  `GaryxTranscriptFileCacheStore.load` (`GaryxTranscriptCache.swift:258`,
  `Data(contentsOf:)` + decode) plus full `mobileMessages` mapping.
- P2: `GaryxMobileConversationViews.swift:187` (`.onChange(of:
  model.selectedThreadTurnRows().map(\.id))`) and `:268` (body) →
  `selectedThreadTurnRows()` (`+Messages.swift:20`) →
  `GaryxMobileRenderStateMapper.rows` → `MessageLookup.init`
  (`GaryxMobileRenderState.swift:776`) rebuilds six dictionaries and re-maps
  every transcript message through `GaryxMobileTranscriptMapper` per call.
- P3: `GaryxMobileConversationViews.swift:249-256` (deliberate eager VStack,
  see comment), `GaryxMobileTurnViews.swift:28`,
  `+ThreadHistory.swift:232` (100-row prepends).
- P4: `GaryxMobileModel.swift:370/:371/:380/:384` and `:163`
  (`renderSnapshotsByThread` is even `@Published`); cleared only at
  `+Gateway.swift:118-127`.
- P5: `GaryxTranscriptCache.swift:279` (`try? encode` → silent return),
  `:287` (`_ = try? replaceItemAt` → swallow + tmp leak; the `catch` only
  covers `write`/`moveItem`).

## Fixes

### P1 — cold-open restore goes fully async; the main actor never touches disk

`showSelectedThread`'s synchronous section only consults in-memory state:

- `messagesByThread` hit → show it (unchanged).
- Miss → `messages = []` (the existing `isSelectedThreadLoadingInitialHistory`
  presentation shows the loading state: with no live/cached render snapshot and
  `historyLoaded == false`, `isAwaitingInitialHistory` is true) and spawn a
  **cold-open restore task**: `transcriptSnapshotAsync` (already exists; loads
  + decodes on the persistence-queue actor off the main thread) → map
  `mobileMessages` in `Task.detached` (same pattern as
  `updateTranscriptCache`) → back on the main actor, apply render snapshot +
  restored messages **only if** the guard below passes.

Race guard (pure logic, Core:`GaryxColdOpenRestorePolicy.shouldApply`).

> **Design-review v1 correction (finding 2).** `cachedMessages.isEmpty` alone
> is not a sufficient freshness marker: a history fetch can legitimately
> complete with an *empty* transcript (still calls `markThreadHistoryLoaded`),
> and a stream frame can apply a newer render snapshot before the throttled
> message flush populates `messages`. The restore must abort if **any** newer
> content path has run, not just if `messages` is non-empty.

`shouldApply` returns true only when **all** hold at apply time (checked on the
main actor, captured token compared):

1. `selectedThreadId == restoredThreadId` — still the same open thread.
2. `restoreGeneration == capturedGeneration` — no thread-switch churn since
   the task was spawned. A new monotonic `selectedThreadColdOpenGeneration`
   (UInt64) is bumped in `showSelectedThread` whenever the thread id changes;
   the restore task captures it at spawn. This catches "switched away and back
   to the same id" that a bare id compare would miss.
3. `!threadHistoryLoaded` — the network history apply
   (`markThreadHistoryLoaded`, called unconditionally by every apply path,
   including an empty transcript) has **not** run. This is the arm finding 2
   named: an empty-but-loaded transcript must suppress restore.
4. `liveRenderSnapshot == nil` — no stream/render frame
   (`renderSnapshotsByThread[threadId]`) has applied. This is the other
   finding-2 arm: a stream snapshot that landed before its message flush must
   suppress restore.
5. `cachedMessages(for:).isEmpty` — no messages present yet.

Any newer path (history apply, stream apply) sets (3) or (4) and restore
discards its output; if restore lands first, the later fetch/stream overwrites
it via the existing `setPreparedMessages`/`applyThreadRenderSnapshot` paths as
today. The restore task also seeds the in-memory mirror
(`cachedTranscriptSnapshots`) so the incremental fetch's
`transcriptAfterCursorAsync` sees the cursor exactly as before (both paths
serialize through `GaryxTranscriptCachePersistenceQueue`; a duplicate disk
read in the race window is harmless — page-cache-hot). Mirror seeding is
itself gated by (1)+(2)+(4) so a stale decode cannot clobber a fresher live
window in the mirror.

Behavior change: on a cold open of a large cached thread the user sees the
loading indicator for the decode duration (~130ms at 30MB on a Mac) instead
of a frozen main thread (which blocked *everything*, including navigation
animation). Small caches restore visually as fast as before.

`transcriptSnapshot(for:)` (sync, disk-touching) remains for the
stream-frame paths (`+ThreadStream.swift` `dropCommittedCacheBelow` /
`applyStreamedCommittedMessages` / `applyThreadRenderSnapshot`). Verified
ordering argument for why those never become the new main-thread disk read:
a stream frame can only arrive after the connection is up, and every
(re)connect's request is built by `selectedThreadStreamRequestForActor`,
which `await transcriptSnapshotAsync(...)` — that seeds the in-memory mirror
off-main before the first frame can exist, so the frame-path sync calls are
mirror hits. The only mirror-cold window is right after
`refetchAfterControlRewrite`'s `clearTranscriptCache` (where the disk file
is deleted too, so the sync load is a cheap nil stat), and P4 pins
`streamOwnedThreadId` against eviction so eviction can never chill the
stream thread's mirror.

Restore-vs-stream snapshot ordering: the restore task re-applies the
disk snapshot's `renderSnapshot` only when `renderSnapshotsByThread[threadId]`
is still nil — a live stream frame's snapshot is always newer than the disk
copy and must not be clobbered.

Tests:
- Keep `testP1_coldOpenSyncRestoreCostExceedsFrameBudget` as the quantified
  justification (documents why sync restore is banned).
- New: `GaryxColdOpenRestorePolicyTests` — the full 5-condition matrix:
  thread changed ⇒ discard; generation bumped (switch-away-and-back) ⇒
  discard; history loaded (incl. empty transcript) ⇒ discard; render snapshot
  present ⇒ discard; messages non-empty ⇒ discard; all-clear ⇒ apply.

### P2 — memoize prepared turn rows keyed by input identity

New Core type `GaryxTurnRowsCache` (a small struct):

```swift
mutating func rows(
    threadId: String?,
    snapshot: GaryxRenderSnapshot?,
    messages: [GaryxMobileMessage],
    transcriptMessages: [GaryxTranscriptMessage],
    build: () -> [GaryxMobileTurnRow]
) -> GaryxPreparedTurnRows   // { rows, ids }
```

It stores the last inputs + prepared output and returns the cached value when
all four inputs compare equal. Array `==` has an identity fast path (same
buffer ⇒ O(1)), and the model already deduplicates `renderSnapshotsByThread`
writes (`setRenderSnapshot` guards `!=`) and messages writes (signature
guards), so between flushes the comparison is pointer-cheap; on a real input
change the compare either fast-fails on count or costs one Equatable pass,
which is far below one mapper rebuild (55–137ms).

`GaryxMobileModel.selectedThreadTurnRows()` routes through a non-`@Published`
`selectedTurnRowsCache`; `selectedThreadTurnRowIds()` exposes the cached ids
for the `.onChange` observer, so a body evaluation performs **zero** mapper
work when inputs are unchanged and exactly one rebuild when they changed.

Cache correctness: the key covers *all* mapper inputs, so any change that can
alter output (placeholder resolution state included — it is a function of
`messages`/`transcriptMessages`) invalidates. The mapper itself stays a dumb,
pure mapping — no derivation logic moves into the client (repro test asserts
purity; hard constraint respected: no user-turn grouping / tool pairing /
tail-thinking derivation added client-side).

Tests: `GaryxTurnRowsCacheTests` — build-count via injected closure (same
inputs ⇒ 1 build; each changed input ⇒ rebuild; thread switch ⇒ rebuild;
ids match rows). Plus a quantified after-test: cached call at P2 scale is
~0ms vs 55–137ms rebuild.

### P3 — floor-anchored render window over turn rows (keep the eager VStack)

**Not** LazyVStack. The eager VStack is load-bearing: the in-code comment
records that LazyVStack's estimated row heights put the synthetic bottom
anchor below the real content end, breaking scroll-to-tail (phantom space).
Re-litigating that trade-off is out of scope and high-risk; instead we bound
*how many rows exist*.

> **Design-review v1 correction (finding 1).** The first draft defined the
> window as a *tail-inclusive suffix keyed by a count from the end*
> (`rows.suffix(limit)`). That is unsafe: once `total > limit`, every streamed
> tail append advances the suffix start and **removes the oldest rendered
> row** — if the reader is browsing inside the window, that removal shifts
> content under the viewport with no scroll request, a jump regression. The
> corrected design below anchors the window to an **absolute floor row
> identity** that never moves on a tail append, so streaming can only grow the
> window at the bottom, never remove from the top.

Design: render only the rows from an **anchored floor** to the tail. The floor
is the *identity* (stable row id) of the oldest visible row, not a count from
the end.

- New Core planner `GaryxTurnRowsWindowPlanner` operating on
  `[GaryxMobileTurnRow]` (already in ascending order) with state
  `GaryxTurnRowsWindowState { var floorRowId: String? }`:
  - `initialLimit = 60`, `expandStep = 60`.
  - `resolve(rows:state:) -> (visible: [GaryxMobileTurnRow], state)`:
    1. `rows` empty ⇒ visible `[]`, `floorRowId = nil`.
    2. Determine the floor index:
       - if `state.floorRowId` resolves to an index `f` in `rows` ⇒ `f`
         (the anchor is **honored** — this is what makes a tail append a
         no-op for the top of the window);
       - else (uninitialized, or the anchored row was dropped by a
         windowed-resume reset) ⇒ `max(0, rows.count - initialLimit)`
         (re-anchor to the newest `initialLimit`).
    3. visible = `rows[floorIndex...]`; new `floorRowId = rows[floorIndex].id`.
  - `expand(rows:state:) -> state`: `newFloor = max(0, currentFloorIndex -
    expandStep)`; `floorRowId = rows[newFloor].id`. The floor only ever moves
    **up** (older); it is never pushed down by resolve, so the visible set is
    monotonically non-shrinking within a thread session.
  - `isWindowExhausted(rows:state:) -> Bool`: floor index is 0 — every
    in-memory row is shown (gates network paging + the Load-earlier button).
- Model: `@Published private(set) var selectedTurnRowsWindowState`; reset to
  `.init(floorRowId: nil)` on thread switch (`showSelectedThread`,
  `openNewThreadDraft`); `expandSelectedTurnRowsWindow()` calls `expand`.
  `selectedThreadTurnRows()` returns the resolved `visible` slice and, as a
  pure by-product, writes back the resolved `floorRowId` (idempotent — a plain
  read with no floor change is a no-op, so it does not thrash `@Published`).
- View wiring: `prefetchOlderHistoryIfNeeded()` (geometry-gated by the existing
  scroll-state machine: `isNearLoadedHistoryStart` +
  `isLargeEnoughForAutomaticHistoryPrefetch` + `hasMovedTowardOlderHistory`)
  becomes a two-stage boundary action — if the window is not exhausted, expand
  it (pure state change, no network); only when exhausted fall through to
  `loadOlderSelectedThreadHistory()` as today. The "Load earlier" button shows
  when the window is not exhausted OR `selectedThreadHasMoreHistoryBefore`; its
  tap expands the window locally first (instant, no spinner) and performs the
  network page only once the window is exhausted.

**Anchoring compatibility argument** (the sensitive part):

1. *Streaming while following tail*: new rows append after the floor;
   `floorRowId` still resolves to the same row at (almost) the same index, so
   the top of the window is unchanged and the window grows only at the bottom.
   `contentChanged` sees a tail append (not a prepend, not a removal) exactly
   as today; follow-tail and the bottom-anchor metrics
   (`GaryxConversationBottomOffsetKey` on the real content end) are untouched.
2. *Streaming while browsing off-tail* (the finding-1 case): identical
   mechanics — the anchored floor does not move on a tail append, so **no row
   is removed from the top or inside the viewport**. The appended rows land
   below the viewport (off-screen, since the reader is up in history) and
   `contentChanged(isFollowingTail == false)` returns nil (no programmatic
   scroll). No jump. This is the case the suffix design broke and the anchor
   design fixes.
3. *History prepend / window expansion position-keeping*: expanding the window
   lowers the floor, prepending rows at the top of the ForEach — **exactly**
   the shape of today's `loadOlderSelectedThreadHistory` network prepend (100
   committed rows). Already handled: `preservesScrollForPrependedHistory`
   detects "previous first id moved down", `renderRowsChanged` →
   `contentChanged(isHistoryPrepend: true)` returns nil, and the view
   preserves reading position. Window expansion introduces **no new scroll
   event category** — it reuses the identical shipped prepend path with
   identical guards.
4. *Initial open*: floor is uninitialized ⇒ resolves to the newest
   `initialLimit` (60). The open fetch is `threadHistoryUserQueryLimit = 3`
   user turns — far fewer than 60 rows — so short/normal threads render every
   row identically to today; `threadOpened` still jumps to tail.
5. *Re-anchor only on reset*: the floor re-anchors to tail **only** when its
   row was dropped from `rows` (windowed-resume `dropCommittedCacheBelow`,
   which already reflows the transcript and resets scroll) — never on a plain
   append or a normal render. So the only "window shrinks" event coincides
   with an existing transcript-reset event, not with steady-state reading.
6. *Scroll-to-bottom / composer focus / keyboard*: operate on the bottom
   anchor id, always inside the window by construction (the window is
   tail-inclusive).

Invariant that kills the regression, stated precisely and tested:
**for any `rows' = rows + appended` (tail append) and an initialized
`state.floorRowId` still present in `rows'`, `resolve(rows', state).visible`
has the same prefix boundary — the same set of hidden head rows — as
`resolve(rows, state).visible`.** i.e. a tail append never changes which rows
are hidden at the top. (`GaryxTurnRowsWindowPlannerTests` asserts this
directly.)

Failure mode honestly stated: within one session the window grows
monotonically (deep browse + long live runs accumulate rows). That is
deliberate — it bounds the *entry* cost (opening a thread with thousands of
previously-cached history rows now instantiates 60, not all of them) without
ever removing a row mid-scroll, which is the only way to avoid the jump the
task forbids. P4 bounds the underlying per-thread message memory
independently. Thread switch resets the window to 60.

Tests: `GaryxTurnRowsWindowPlannerTests` —
- resolve on uninitialized state shows newest `initialLimit`;
- **tail-append invariant** (above): append rows with an initialized floor ⇒
  identical hidden-head set (the anti-regression lock);
- expand lowers the floor by `expandStep`, clamps at 0, monotonic
  (never raises the floor);
- resolve after expand keeps the lowered floor across a subsequent tail
  append;
- floor row dropped (reset) ⇒ re-anchors to newest `initialLimit`;
- exhaustion true iff floor index 0;
- expansion prepend produces `preservesScrollForPrependedHistory == true`
  (locks the anchoring argument to the shipped prepend path);
- after-metric: 3,000 rows ⇒ 60 visible on open (vs 3,000 before).

### P4 — LRU residency cap for per-thread memory

New Core type `GaryxThreadResidencyTracker`: ordered access list,
`maxResidentThreads = 6`, `touch(_:)` on every per-thread write,
`evictionCandidates(pinned:)` returns over-cap least-recently-used ids
excluding pinned.

Model wiring (`trimThreadResidency()` after the write paths:
`setMessages`/`setPreparedMessages`, `setRenderSnapshot`,
`updateTranscriptCache`-mirror writes, `showSelectedThread`):

- Pinned (never evicted): `selectedThread`, `streamOwnedThreadId`, threads
  with unsettled local rows (any message whose `localState` is non-nil and
  != `.remoteFinal` — optimistic sends/pending acks must survive; pure
  predicate in Core, tested).
- Eviction drops the five per-thread projections together:
  `messagesByThread`, `messageSignaturesByThread` (must go with messages or
  the signature-skip in `setMessages` would refuse to rebuild),
  `activeAssistantMessageIdsByThread`, `renderSnapshotsByThread`,
  `cachedTranscriptSnapshots`.
- NOT evicted: `transcriptCachePersistenceGenerations` (drives the
  monotonic write-ordering guard in `GaryxTranscriptCachePersistenceQueue`;
  resetting it would make the actor reject subsequent saves — verified
  failure mode) and `selectedThreadRenderFloorByThread` (tiny; floor is
  re-derived on open). Both are O(bytes) per thread.
- Eviction also removes the thread from `threadHistoryLoadedIds`: an evicted
  thread is presentation-wise a cold thread again, so re-opening it must take
  the `isAwaitingInitialHistory == true` (loading) path instead of flashing
  the empty-conversation view for a frame while the async restore/fetch run.
- Re-opening an evicted thread hits the disk cache through the P1 async
  restore path — data loss is impossible (evicted state is always
  re-derivable from disk + gateway).
- `deleteThread`/`resetForGatewaySwitch` also clear tracker entries.

Background-reconcile churn: the 15s committed-run reconcile writes other
threads' messages; they get touched to most-recent and older ones evict —
residency stays ≤ cap by induction; no eviction/rebuild ping-pong because
eviction only fires above cap and always removes the *least* recent.

Tests: `GaryxThreadResidencyTrackerTests` — cap enforcement, LRU order,
touch refresh, pinned exemption, unsettled-local-rows predicate, remove.
After-metric: 40 simulated visited threads ⇒ residency ≤ 6 + pinned (vs 40
before — the audit's unbounded growth).

### P5 — cache persistence failures become observable + tmp cleanup

`GaryxTranscriptFileCacheStore` gains an injectable diagnostics sink:

```swift
public enum GaryxTranscriptCacheStoreEvent: Equatable, Sendable {
    case saveEncodeFailed(threadId: String)
    case saveWriteFailed(threadId: String, reason: String)
}
public init(directory:ttl:now:fileManager:diagnostics: ((Event) -> Void)? = nil)
```

`save()` fixes: the replace step joins the real `do/catch`
(`_ = try fileManager.replaceItemAt(...)` — no more `try?`), the catch
cleans up the tmp file (fixing the leak) and reports
`saveWriteFailed(reason:)`; encode failure reports `saveEncodeFailed`. The
app wires `diagnostics` to an `os.Logger` (subsystem `com.garyx.mobile`,
category `transcript-cache`) at the `GaryxMobileModel` store construction —
Core stays logging-free, tests inject a recording closure.

**tmp sweep (design-review v1 correction, finding 3).** Cleaning tmp only in
the current-failure catch does not remove `.json.tmp` residue from *older app
versions*, from a crash after `data.write(to: tmp)` but before the replace, or
from a `.tmp` whose owning save was interrupted. Since the repro identifies
leaked tmp files as accumulating forever (nothing sweeps them), the store
sweeps orphan `.tmp` files at every existing directory-scan touchpoint:

- `init` (before `pruneExpired`, which already scans the directory on every
  launch) removes every `*.json.tmp`. A live save that races the sweep is
  safe: `data.write(..., .atomic)` writes to `<tmp>` and the write+replace is
  synchronized under the store's `NSLock`; the init sweep runs before any save
  can be issued on this instance, and cross-process cache access does not
  occur (one app process owns the caches dir). To be defensive the sweep
  ignores a `.tmp` newer than a small threshold is unnecessary — the lock and
  single-writer property already exclude an in-flight tmp at init time.
- `clearAll()` removes `*.json.tmp` alongside `*.json`.
- `remove(threadId:)` removes the thread's `<key>.json.tmp` alongside its
  `.json`.
- `pruneExpired()` (init + reusable sweep) drops orphan `*.json.tmp` in the
  same pass it prunes expired `.json`.

The tmp filename is deterministic (`<base64 key>.json.tmp`), so per-thread
cleanup targets the exact sibling; the directory sweep catches everything
else including keys no longer mapped to a live thread.

Contract note: save remains fire-and-forget for callers (UI never blocks on
cache persistence; the durable source of truth is the gateway). Observability
+ cleanup is the fix; retry policy is out of scope.

Tests: the RED repro `testP5_saveReplaceFailureCleansUpTemporaryFile` turns
GREEN; `testP5_saveFailureEmitsWriteDiagnostics` (reason surfaces),
`testP5_successfulSaveEmitsNoDiagnostics` (happy path silent), and new tmp-sweep
tests: `testP5_initSweepsOrphanTmpFiles` (a pre-seeded `.json.tmp` is gone
after constructing a store), `testP5_removeSweepsThreadTmp`,
`testP5_clearAllSweepsTmp`.

## Non-goals / explicitly out of scope

- Making the mapper itself incremental (P2 removes *redundant* rebuilds; a
  single rebuild on real input change is still O(n) — acceptable and now
  bounded by P3's window for view-layer cost).
- LazyVStack migration (see P3 argument).
- Cache write retry/backoff (P5 adds observability, not durability).
- Any change to server `render_state` contracts — the mapper stays a dumb
  ref-resolver; caching is wrapped *around* it, not inside row derivation.

## Compatibility with mobile-ui rules

- Render source stays server `render_state` (P2 cache keys on the snapshot,
  P3 windows the *prepared* rows — row structure untouched).
- No new top-level concepts; no desktop patterns ported.
- All new logic lives in `Sources/GaryxMobileCore` with SwiftPM tests; app
  files keep composition/side-effect orchestration only.
- New Core files ⇒ `xcodegen generate` + committed pbxproj + `xcodebuild`
  app-target build before handoff.

## Rollout / validation

1. Repro tests committed first (this doc cites their numbers).
2. Implementation in this worktree; per-fix unit tests as listed.
3. Full `swift test` (no pipe-tail), `xcodegen generate`, `xcodebuild build`
   for the app target.
4. Adversarial code review (codex) with before/after reproduction required.
