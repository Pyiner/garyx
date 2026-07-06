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

Race guard (pure logic, Core:`GaryxColdOpenRestorePolicy`): apply restored
messages only when (a) the restored thread is still `selectedThread`, and
(b) nothing newer arrived while decoding — `cachedMessages(for:)` is still
empty. The network fetch (`loadSelectedThreadHistory`, kicked off by
`selectThread` immediately after) wins any race: if it lands first, restore
output is discarded; if restore lands first, the fetch's
`setMessages`/`applyThreadTranscriptToCache` overwrite it as today. The
restore task also seeds the in-memory mirror (`cachedTranscriptSnapshots`),
so the incremental fetch's `transcriptAfterCursorAsync` sees the cursor
exactly as before (both paths already serialize through
`GaryxTranscriptCachePersistenceQueue`; a duplicate disk read in the race
window is possible and harmless — page-cache-hot).

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
- New: `GaryxColdOpenRestorePolicyTests` — apply/discard matrix (still
  selected × messages still empty × restore empty/non-empty).

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

### P3 — bounded render window over turn rows (keep the eager VStack)

**Not** LazyVStack. The eager VStack is load-bearing: the in-code comment
records that LazyVStack's estimated row heights put the synthetic bottom
anchor below the real content end, breaking scroll-to-tail (phantom space).
Re-litigating that trade-off is out of scope and high-risk; instead we bound
*how many rows exist*.

Design: render only the **tail window** of prepared turn rows.

- New Core planner `GaryxTurnRowsWindowPlanner`:
  - `initialLimit = 60` turn rows (a turn row is a whole user-turn: user
    bubble + activity), `expandStep = 60`.
  - `windowedRows(rows, limit)` → suffix slice (tail-inclusive).
  - `expandedLimit(current:total:)` → `min(current + expandStep, total)`.
  - `isWindowExhausted(limit:total:)` → whether the window already shows all
    in-memory rows (gates network paging and the Load-earlier button).
- Model: `@Published private(set) var selectedTurnRowsWindowLimit`; reset to
  `initialLimit` on thread switch (`showSelectedThread`,
  `openNewThreadDraft`); `expandTurnRowsWindow()` bumps it.
- View wiring: `prefetchOlderHistoryIfNeeded()` (geometry-gated by the
  existing scroll-state machine: `isNearLoadedHistoryStart` +
  `isLargeEnoughForAutomaticHistoryPrefetch` + `hasMovedTowardOlderHistory`)
  becomes a two-stage boundary action — if the window hides in-memory rows,
  expand the window (pure state change, no network); only when the window is
  exhausted fall through to `loadOlderSelectedThreadHistory()` as today. The
  "Load earlier" button shows when the window hides in-memory rows OR
  `selectedThreadHasMoreHistoryBefore`; its tap expands the window locally
  first (instant, no spinner) and only performs the network page once the
  window is exhausted — strictly faster than today's always-network button.

**Anchoring compatibility argument** (the sensitive part):

1. *Streaming follow-tail*: the window is a tail-inclusive suffix. New rows
   append inside the window; `contentChanged`/`renderRowsChanged` see the
   same id-suffix relationships as today. Follow-tail and the bottom-anchor
   metrics (`GaryxConversationBottomOffsetKey` on the real content end)
   are untouched — the anchor stays glued to the actual rendered tail.
2. *History prepend position-keeping*: expanding the window prepends rows at
   the top of the ForEach — **exactly** the shape of today's
   `loadOlderSelectedThreadHistory` prepend (100 committed rows at once).
   That path is already handled: `preservesScrollForPrependedHistory`
   detects "previous first id moved down", `renderRowsChanged` →
   `contentChanged(isHistoryPrepend: true)` returns nil (no programmatic
   scroll), and the view preserves reading position the same way it does for
   network pages today. Window expansion introduces **no new scroll event
   category** — it reuses the identical, shipped prepend path with identical
   guards. Risk is bounded to "same as pressing Load earlier today".
3. *Initial open*: `windowLimit=60` covers the newest window
   (`threadHistoryUserQueryLimit = 3` user turns fetched on open, far fewer
   than 60 rows), so short/normal threads render identically; `threadOpened`
   still jumps to tail.
4. *Trimming*: the window only ever grows within a thread session and resets
   on thread switch. No mid-scroll shrink ⇒ no mid-scroll jump, and rows
   above the viewport are never removed while the user is reading them.
5. *Scroll-to-bottom / composer focus / keyboard*: operate on the bottom
   anchor id, which is inside the window by construction.

Failure mode honestly stated: a reader who pages very deep in one session
still accumulates rows (window grows monotonically). That bounds the *entry*
cost (open + streaming on the newest window) without capping a deliberate
deep browse; deep-browse trimming would require mid-scroll row removal —
exactly the jump risk this task forbids. The audit's pain point (long
threads with much prepended history are slow *by default*) is fixed because
the default window is 60 rows regardless of how much history previous
sessions had cached (today: every cached committed row is instantiated on
open).

Tests: `GaryxTurnRowsWindowPlannerTests` — suffix windowing, expansion
monotonic + clamped, exhaustion gating, default covers initial fetch,
window(60) of 3,000 rows leaves 60 (the after-metric vs 3,000 before), and
an id-shape test asserting expansion produces exactly the
`preservesScrollForPrependedHistory == true` prepend shape (locks the
anchoring argument into a test).

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

Contract note: save remains fire-and-forget for callers (UI never blocks on
cache persistence; the durable source of truth is the gateway). Observability
+ cleanup is the fix; retry policy is out of scope.

Tests: the RED repro `testP5_saveReplaceFailureCleansUpTemporaryFile` turns
GREEN; new `testP5_saveFailureEmitsDiagnostics` (replace-failure reason
surfaces), `testP5_saveEncodeFailure…` if constructible, plus a
read-only-directory write-failure case (`chmod 0500`) asserting the event
fires and no tmp remains.

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
