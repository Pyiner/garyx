# TASK-1037 — iOS home-list scroll jank: diagnosis + architecture design

Status: **diagnosis + design only.** No product code changed. This is one of two
independent diagnostic passes; conclusions here are reached independently and are
meant to be compared/merged before any implementation.

Scope delivered: a no-UI quantification harness (`GaryxMobileCore` SwiftPM
tests), a root-cause ranking backed by numbers, and a clean optimization design.

---

## 1. TL;DR

The home list (`GaryxHomeThreadListView`) janks intermittently because a
**single O(n) section derivation (`homeThreadSections`) is recomputed on the main
thread on every model invalidation**, and the home view observes a **monolithic
`@EnvironmentObject` model with ~80 `@Published` fields**, so a huge surface of
unrelated state changes — and a high-frequency one during active runs — force
that recompute while the user is scrolling.

Ranked primary→secondary, with measured numbers (Apple Swift 6.3.2, arm64 macOS,
release-equivalent `swift test`; numbers are a **headless lower bound** — they
measure the data build only, not the SwiftUI body/ForEach diff that also runs on
device):

| Rank | Root cause | Quantified evidence |
|---|---|---|
| **P0-A** | `homeThreadSections` is an O(n), un-memoized recompute on every body invalidation, living in the App target (untestable today). | **0.54 ms / recompute @ 50 threads**; 0.28 / 0.54 / **1.95 ms** at 25 / 50 / 200 threads (≈ linear). Cache hit is **~12× cheaper** (0.045 ms). |
| **P0-B** | Active-run churn: each streaming run-state delta rebuilds the whole `threads` array *and* re-publishes `runStateByThread`, and run state is baked into the section row (`isRunning`). | **601 section recomputes in 60 s** with **one** running thread (vs **1** with the proposed design). |
| **P1-C** | Monolithic model: home view observes all ~80 `@Published`; unrelated publishes recompute the home sections. | **6 unrelated publishes/min → 6 recomputes** (vs 0 proposed). |
| **P1-D** | Catalog state published without an Equatable diff (`applyAgentTargets`). | **20 identical agent re-applies → 20 wasted `objectWillChange`** (vs 0 for the gated `threads` path). |
| **P2-E** | Per-running-row 30 fps `TimelineView` badges (independent clocks); model-level 1.5 s poll does not pause during scroll. | Qualitative; localized compositing cost ∝ running rows; 1.5 s loop runs an unconditional O(n) widget-snapshot pass. |

**Refuted from the recon:** P0-1's "1.5 s reconcile re-publishes `threads` even
when identical." The `threads` assignment **is** gated
(`refreshThreads` `GaryxMobileModel+Threads.swift:215`,
`if threads != mergedThreads`), and the test confirms **40 identical reconciles →
0 `objectWillChange`**. The 1.5 s loop is therefore *not* the idle-state jank
driver via `threads`; the real driver is active-run churn (P0-B) amplified by the
un-memoized O(n) derivation (P0-A).

---

## 2. How to run the quantification

```bash
cd mobile/garyx-mobile
swift test --filter GaryxHome            # the 12 diagnostic tests
swift test                               # full suite (393 tests, all green)
```

Headline lines are printed with the `[TASK-1037]` prefix. Files:

- `Tests/GaryxMobileCoreTests/GaryxHomeSectionsReferenceSupport.swift` — a
  faithful 1:1 **port** of `homeThreadSections` / `homeThreadRow` /
  `homeThreadIdentity` onto Core types, plus synthetic fixtures, the proposed
  Equatable keys, a memoized cache, and a Combine publish probe. The port lives
  only in the test target (no product change) and **is itself the proof that the
  derivation can be sunk into `GaryxMobileCore` as a pure function**.
- `GaryxHomeThreadSectionsPerformanceTests.swift` — derivation cost + O(n) scaling.
- `GaryxHomeThreadSectionsCachingTests.swift` — purity, Equatable-gate cache,
  run-state-decoupling, cache-hit vs recompute.
- `GaryxHomePublishStormTests.swift` — `objectWillChange` accounting (gated vs
  ungated; idle vs active-run recompute counts).

Representative output:

```
homeThreadSections derivation @ 50 threads / 80 agents / 25 teams / 25 automations: 0.54 ms / recompute
derivation scaling — 25 threads: 0.28 ms | 50: 0.54 ms | 200: 1.95 ms
cache-HIT (Equatable gate): 0.045 ms vs full recompute: 0.54 ms  (speedup ~12x)
agents (ungated, applyAgentTargets): 20 identical re-applies -> 20 wasted objectWillChange
threads (gated, refreshThreads:215): 40 identical reconciles -> 0 objectWillChange (gate holds)
ACTIVE-RUN storm (300 events / 60s, 1 running thread): CURRENT section recomputes = 601 | PROPOSED = 1
IDLE minute (6 unrelated publishes): CURRENT section recomputes = 6 | PROPOSED = 0
run-state churn (300 deltas): naive-content key busts 300x | proposed identity key busts 0x
```

---

## 3. Mechanism

SwiftUI re-evaluates a view's `body` when an observed `ObservableObject` fires
`objectWillChange`. `GaryxMobileModel` is a single `ObservableObject` whose
synthesized `objectWillChange` fires on **any** of its ~80 `@Published` `willSet`s
(`GaryxMobileModel.swift:69-196`). `GaryxHomeThreadListView` holds it as
`@EnvironmentObject` (`GaryxMobileSidebarViews.swift:122`), so it is subscribed to
the entire model.

Inside the body, `sidebarThreadSections` reads `let sections =
model.homeThreadSections` (`GaryxMobileSidebarViews.swift:172`). That computed
property (`:324-377`) is **O(n)**: it builds `threadsById` / `teamsById` /
`agentsById` dictionaries, the automation-thread set, and then for every pinned +
recent thread constructs a `GaryxHomeThreadRow` — resolving identity
(`homeThreadIdentity`), building a `GaryxSidebarThreadRowPresentation` (subtitle
join + last-message-preview collapse), an avatar struct, and a relative
timestamp. There is **no Equatable input gate and no memoization**, so it runs in
full on every body invalidation.

Scrolling alone does **not** invalidate the body — but any `objectWillChange`
landing *during* a scroll forces a full O(n) rebuild that competes with the
scroll's own per-frame row materialization on the main thread. The symptom is
intermittent because it tracks background activity, chiefly **active runs**.

`LazyVStack` (`:145`) correctly virtualizes the row *views*, but the row *data*
(all N rows incl. off-screen, with avatars/identities/presentation) is built
eagerly every recompute — so virtualization is defeated at the data layer.

---

## 4. Root causes (independent verification)

### P0-A — Un-memoized O(n) section derivation (the amplifier)
- **Where:** `GaryxMobileSidebarViews.swift:324-377` (`homeThreadSections`),
  `:379-410` (`homeThreadRow`), `:412-469` (`homeThreadIdentity`).
- **Why it hurts:** every invalidation pays the full O(n). It turns *any* publish
  into main-thread work; combined with P0-B it turns into sustained work during
  scroll.
- **Number:** 0.54 ms @ 50 threads, **1.95 ms @ 200 threads**, ~linear; cache hit
  0.045 ms (**~12×**). The 0.54 ms is a *data-build floor* — on device the
  SwiftUI `ForEach` re-diff and row-body re-evaluation add to it, and rows are not
  `Equatable` so SwiftUI cannot cheaply skip unchanged ones.
- **Recon verdict:** P0-2 **confirmed**, and confirmed un-testable today (App
  target) — the port proves it sinks cleanly to Core.

### P0-B — Active-run churn rebuilds `threads` and bakes run state into rows
- **Where:** `applyTranscriptRunState` (`GaryxMobileModel+Threads.swift:1183-1189`,
  gated by `previous == state`) → on a genuine delta sets
  `runStateByThread[id]` (publish #1) and calls `applyThreadRunStateSummary`
  (`:1257-1263`) which does `threads = threads.map { … }` — a full **ungated
  rebuild** of the array (publish #2). Run state is then baked into the section
  row via `GaryxSidebarThreadRowPresentation.isRunning`
  (`GaryxMobilePresentationModels.swift:54`, consumed at
  `GaryxMobileSidebarViews.swift:1541`).
- **Why it hurts:** a single streaming thread emits many deltas/sec; each is
  **2 publishes**, each recomputing the O(n) sections. The running badge cannot
  flip without rebuilding the whole section model.
- **Number:** **601 section recomputes / 60 s** for one running thread at ~5
  events/s; the run-state-decoupled design yields **1**. The naive-content key
  busts **300/300** times; the proposed identity key **0**.
- **Recon verdict:** **stronger** than the recon's framing — this, not the 1.5 s
  idle loop, is the primary intermittent driver.

### P1-C — Monolithic model over-invalidates the home view
- **Where:** ~80 `@Published` on one object (`GaryxMobileModel.swift:69-196`);
  home view observes all of it (`:122`). Unrelated subsystems publish on their
  own cadence (bot status `botStatusesById`, workspace git
  `workspaceGitStatuses`, transcript `renderSnapshotsByThread`, `runTracker`,
  catalog refreshes, etc.).
- **Number:** 6 unrelated publishes/min → 6 home-section recomputes (proposed: 0).
- **Recon verdict:** new/under-weighted in the recon; it is the structural reason
  P0-A is so exposed.

### P1-D — Catalog publishes without Equatable dedup
- **Where:** `applyAgentTargets` (`GaryxMobileModel+StateSync.swift:13-20`):
  `agents = nextAgents` / `teams = nextTeams`, no diff. Fires on scene-active and
  stale-while-refresh catalog refreshes (`GaryxMobileModel+Gateway.swift:472,
  549, 581-615`).
- **Number:** 20 identical re-applies → **20 wasted `objectWillChange`**; the
  gated `threads` path is **0/40**.
- **Recon verdict:** P0-3 **confirmed** (it's a real wasted publish), but it is
  lower-frequency than P0-B and is *not* on the 1.5 s loop.

### P2-E — Animation timers + non-scroll-aware model polling
- **Where:** `GaryxAvatarTypingBadge` / `GaryxSidebarRunningIndicator` each own a
  `TimelineView(.animation(minimumInterval: 1/30))`
  (`GaryxMobileSidebarViews.swift:1611-1639, 1641-1679`) — one independent 30 fps
  clock per running row. The model-level 1.5 s reconcile
  (`GaryxMobileModel+Threads.swift:555-597`) runs unconditionally in the
  foreground; it does **not** consult `isThreadListInteracting` (that signal
  exists only as home-view `@State`, `:125, :159-161`, and never reaches the
  model). Each cycle also runs an unconditional O(n)
  `persistRecentThreadsWidgetSnapshot()` (`:218, :271-297`).
- **Recon verdict:** P1 (badge) **confirmed** but localized (TimelineView redraws
  only its own subtree; it does **not** recompute sections). The 1.5 s loop's
  residual cost is the unconditional widget snapshot + history hydration, not a
  `threads` republish.

### Explicitly checked and **not** primary
- **Virtualization:** `LazyVStack` present and used correctly (rows emitted
  directly, not wrapped — `:167-220`). View-level virtualization works; the
  problem is eager *data* construction (folded into P0-A).
- **Avatars:** `GaryxAgentAvatarView` decodes data-URLs through
  `GaryxDataURLImageCache` (`GaryxMobileAgentPickerComponents.swift:152-154`);
  remote avatars use `AsyncImage`. Both cached — not a hot path.
- **Timestamp formatting:** already optimized (shared `ISO8601DateFormatter` +
  bounded `NSCache`, `GaryxMobileDesignSystem.swift:428-459`). Not a hot path.
  Note: it reads `Date()`, so the relative string is wall-clock dependent — a
  caching design must treat the timestamp as a render-time concern (see D1).

---

## 5. Architecture design (clean, not patches)

Direction follows `docs/agents/mobile-ui.md`: pure derivation/presentation/
business rules belong in `GaryxMobileCore` with SwiftPM tests; the App target
does SwiftUI composition + side-effect orchestration; lists dumb-render
pre-computed immutable sections with stable `Identifiable`/`Equatable` rows; and
low-frequency catalog data uses stale-while-refresh caching.

### D1 — Sink section derivation into Core, behind an Equatable input gate + memoization
- **Change:** move the `homeThreadSections` logic into `GaryxMobileCore` as a pure
  `HomeThreadSectionsBuilder.build(_:) -> HomeThreadSections` over an
  `Equatable` `Inputs` struct (exactly the port in
  `GaryxHomeSectionsReferenceSupport.swift`). Wrap it in a small memo
  (`HomeThreadSectionsCache`) keyed by an **Equatable identity key**; recompute
  only when the key changes. The App target keeps one cache instance and calls
  it from the view.
- **Timestamps:** exclude relative time from the key; render `updatedAt` →
  display string at the row using a relative formatter (or a coarse 60 s ticker),
  so wall-clock drift never busts the cache.
- **Impact surface:** new Core file + tests; `GaryxMobileSidebarViews.swift`
  swaps the computed property for a cached call; the private row/section types
  (`GaryxHomeThreadRow`, `GaryxHomeThreadSections`) move to Core.
- **Tradeoff:** holds one cached section array (small). Equatable-key compare is
  O(n) shallow — already ~12× cheaper than rebuild; a monotonic revision counter
  (D3) can make the gate O(1) later.

### D2 — Decouple per-thread run state from section identity
- **Change:** remove `isRunning` from the baked section row; the section identity
  key excludes `runState`/`activeRunId` (proven by
  `testRunStateChurnBustsNaiveKeyButNotProposedIdentityKey`). The running badge
  is driven by a **row-scoped** observation of `runStateByThread[id]?.busy`
  (e.g. a tiny per-row `EquatableView`/child view-model reading only that slice),
  so a run-state delta updates **one** row, not the section model. Stop rebuilding
  the entire `threads` array for a single thread's run-state change
  (`applyThreadRunStateSummary` should update one element / a dedicated slice).
- **Impact surface:** `applyThreadRunStateSummary` (`+Threads.swift:1257-1263`),
  the row view (`GaryxSidebarThreadRowView`), and the Inputs/key definition.
- **Tradeoff:** the row reads run state from a second source; needs care so the
  badge still appears on first render. This is the single biggest win for the
  active-run symptom (601 → 1).

### D3 — Equatable-dedup state publishes
- **Change:** gate catalog assignments like the `threads` path already does:
  `if agents != next { agents = next }` in `applyAgentTargets`; same for `teams`
  and other high-fanout collections. Optionally bump a single
  `homeRevision: Int` only on changes that affect the home list, and key D1's
  cache on it for an O(1) gate.
- **Impact surface:** `+StateSync.swift:13-20` (+ a few sibling assignments).
- **Tradeoff:** an extra equality check per refresh (cheap vs a wasted full
  recompute). Confirmed: gated path is 0 wasted publishes vs 20.

### D4 — Throttle + scroll-pause model-level polling
- **Change:** plumb the existing `isThreadListInteracting` signal from the view to
  the model and have `startBackgroundCommittedRunReconcileLoop` skip / defer work
  while interacting (the 10 s home loop already does this — the 1.5 s model loop
  should too). Gate `persistRecentThreadsWidgetSnapshot()` on actual change.
- **Impact surface:** `+Threads.swift:555-597, 218`; a new interaction flag on the
  model set from `garyxHomeThreadListScrollInteraction`.
- **Tradeoff:** run-state freshness lags slightly during active scrolling
  (acceptable; resumes on scroll end).

### D5 — Equatable rows + `EquatableView` so SwiftUI skips unchanged rows
- **Change:** make the row model `Equatable` (the port already is) and render via
  `.equatable()` / `EquatableView`, so even when a new section array is produced,
  SwiftUI re-renders only rows whose value changed.
- **Impact surface:** row type + `ForEach` body in `GaryxMobileSidebarViews.swift`.
- **Tradeoff:** per-row `==` on diff (cheap) in exchange for skipping unchanged
  row bodies.

### D6 — Animation cost (lower priority)
- **Change:** drive running badges from one shared clock (a single
  `TimelineView` providing an environment phase, or pause animation while
  `isThreadListInteracting`). Reduces N independent 30 fps timers to one.
- **Impact surface:** `GaryxAvatarTypingBadge` / `GaryxSidebarRunningIndicator`.
- **Tradeoff:** minor; do after P0/P1.

### D7 — (Optional, larger) scope the home view's observation
- **Change:** stop observing the whole god object from the home view — expose a
  dedicated `HomeListViewModel`/`@Observable` slice (threads/pins/recents/run
  state) so unrelated `@Published` (bot status, workspace git, transcript
  snapshots) no longer invalidate the home body at all. D1+D2+D5 already
  neutralize the *cost* of over-invalidation; D7 removes the invalidation itself.
- **Tradeoff:** broader refactor; sequence it after the cache lands.

**Recommended sequence:** D1 + D2 first (kills P0-A and P0-B: 601→1, and makes
every remaining invalidation ~12× cheaper), then D3 + D5 (P1-D, cheap row diff),
then D4 (P2 scroll-pause), then D6/D7 as polish/structural follow-up.

---

## 6. Impact-surface summary

| Design | Primary files | Risk |
|---|---|---|
| D1 cache + sink to Core | new Core file + tests; `GaryxMobileSidebarViews.swift:172,324-410` | Low–med |
| D2 run-state decouple | `+Threads.swift:1257-1263`; `GaryxMobileSidebarViews.swift:1515-1546`; key def | Med (correctness of badge) |
| D3 Equatable dedup | `+StateSync.swift:13-20` | Low |
| D4 scroll-pause poll | `+Threads.swift:555-597,218`; view→model flag | Low |
| D5 Equatable rows | `GaryxMobileSidebarViews.swift:178-212,265-272` | Low |
| D6 badge clock | `GaryxMobileSidebarViews.swift:1611-1679` | Low |
| D7 scoped observation | home view + new view-model | Higher |

---

## 7. Test strategy (current vs target)

The harness already encodes the acceptance assertions:

- **Cost ceiling:** `homeThreadSections`-equivalent build stays measured and
  bounded; cache hit ≪ recompute (asserted `<`, measured ~12×).
- **No wasted recompute:** identical inputs → cache `computeCount == 1` over N
  invalidations (target), vs N full recomputes (current).
- **Run-state churn:** identity key busts `== 0` over 300 deltas (target) vs 300
  (current); active-run storm `computeCount == 1` vs 601.
- **No wasted publish:** gated assignment → `objectWillChange == 0` on identical;
  ungated → N (regression guard for D3).

When implementing, the production code should move the port into Core and these
tests should bind to the **real** `HomeThreadSectionsBuilder` (delete the port,
keep the assertions). That is the headless, no-UI regression net for the fix.

---

## 8. Independence notes / open questions

- Numbers are a **headless main-thread floor**. They prove the derivation is
  O(n), recurs at event frequency, and is mostly avoidable. The on-device frame
  cost (SwiftUI body + `ForEach` diff + row bodies) is larger and should be
  confirmed with an Instruments Time Profiler / Hitches trace while scrolling a
  list with an active run — but that is not required to justify the design.
- The biggest leverage is **P0-B (run-state decoupling)**; if the other pass
  ranks the 1.5 s idle loop higher, reconcile against the gated-`threads`
  evidence (40 reconciles → 0 publishes) before acting on it.
- D7 (scoping observation) is the cleanest *structural* fix but the largest; D1+D2
  deliver most of the win at much lower risk and should land first.
