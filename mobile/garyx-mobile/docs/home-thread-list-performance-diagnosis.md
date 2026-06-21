# Home Thread List Performance Diagnosis

Date: 2026-06-21

Scope: iOS home root thread list, meaning pinned plus recent threads. This
diagnosis adds only headless SwiftPM tests and this design note. Product code is
unchanged.

## Headless Harness

Test file:
`Tests/GaryxMobileCoreTests/GaryxHomeThreadListPerformanceDiagnosticsTests.swift`

The harness uses synthetic public-safe data:

- 50 threads
- 8 pinned thread ids and 50 recent ids
- 72 agents
- 28 teams
- 24 automations
- 12 running rows

Because the current `homeThreadSections` implementation is a private
App-target extension in `GaryxMobileSidebarViews.swift`, the test target mirrors
the same pure derivation shape in test-only code. That proves the current seam is
not directly unit-testable and gives a Core-equivalent function that can become
the production Core presentation seam later.

Command used:

```sh
swift test --filter GaryxHomeThreadListPerformanceDiagnosticsTests
```

Result: 9 tests passed.

## Measurements

Measured in SwiftPM debug tests on local arm64 macOS. These numbers are
headless main-thread work proxies; they do not include SwiftUI diffing, layout,
image decoding, or compositor work.

| Probe | Current measurement | Target measurement / assertion |
| --- | ---: | ---: |
| Home sections derivation, 50 rows | 0.524 ms average; XCTest samples 0.488-0.527 ms | Cached hit 0.00056 ms average; XCTest samples 0.001-0.003 ms |
| Identical catalog refresh: agents + teams + automations | 3 duplicate publishes, 150 rows rederived | 0 publishes, 0 rows rederived |
| Identical thread refresh | 0 publishes | Existing guard already matches target |
| Widget projection during identical thread refresh | 0.306 ms per projection | Should be behind the same changed-input gate |
| Background reconcile cadence | 4 refreshes in 6 seconds, 40 refreshes/minute | 0 refreshes during a 6 second simulated list interaction |
| Selection moves from one row to another | 50 rows rederived, only 2 rows semantically changed | Row-level stable Equatable output should limit downstream work |
| Typing badge, 12 running rows | 360 timeline invalidations/s, 1080 `sin` calls/s, 0.156 ms pure math per simulated second | Shared/paused clock should remove per-row timelines while scrolling |

## Source Findings

Confirmed:

- `homeThreadSections` is private App-target UI code and is recomputed from
  `threads`, `pinnedThreadIds`, `recentThreadIds`, `selectedThread`, `agents`,
  `teams`, and `automations` whenever the home list body is invalidated:
  `App/GaryxMobile/GaryxMobileSidebarViews.swift:323`.
- The derivation builds dictionaries and rows synchronously, formats timestamps,
  resolves identity, and emits full pinned/recent arrays:
  `App/GaryxMobile/GaryxMobileSidebarViews.swift:324-377`.
- `applyAgentTargets` assigns equal `agents` and `teams` without an Equatable
  gate:
  `App/GaryxMobile/GaryxMobileModel+StateSync.swift:7-28`.
- `refreshRemoteState` assigns `automations` without an Equatable gate:
  `App/GaryxMobile/GaryxMobileModel+Gateway.swift:595-596`.
- The background committed-run reconcile loop sleeps 1.5 seconds and calls
  `refreshThreads(silent: true)` before checking candidate ids:
  `App/GaryxMobile/GaryxMobileModel+Threads.swift:555-597`.
- The list-local silent sidebar refresh loop already uses a 10 second interval
  and skips refresh while `isThreadListInteracting` is true:
  `App/GaryxMobile/GaryxMobileSidebarViews.swift:126`,
  `App/GaryxMobile/GaryxMobileSidebarViews.swift:227-248`.
- `LazyVStack` virtualization is intentionally preserved: section rows are
  direct children, not wrapped in a section `VStack`:
  `App/GaryxMobile/GaryxMobileSidebarViews.swift:143-170`.
- Badge animation uses one `TimelineView(.animation(minimumInterval: 1 / 30))`
  per running row:
  `App/GaryxMobile/GaryxMobileSidebarViews.swift:1611-1639`.

Refuted or downgraded:

- I did not reproduce the claim that identical `threads` refreshes republish the
  `threads` array. The current code guards `threads`, `pinnedThreadIds`,
  `recentThreadIds`, and `selectedThread` before assignment:
  `App/GaryxMobile/GaryxMobileModel+Threads.swift:215-216`,
  `App/GaryxMobile/GaryxMobileModel+Threads.swift:247-268`,
  `App/GaryxMobile/GaryxMobileModel+Threads.swift:234-236`.
- Even with that guard, `refreshThreads` still rebuilds the widget projection on
  every refresh before it can skip the disk write/reload:
  `App/GaryxMobile/GaryxMobileModel+Threads.swift:271-303`.

## Root Cause Ranking

1. Primary P0: duplicate catalog publication plus UI-only section derivation.
   Equal agents, teams, and automations can produce 3 publishes and force 150
   rows of derivation for a no-op refresh in the test harness. The pure
   derivation cost is about 0.524 ms per invalidation at 50 rows; the real UI then
   also pays SwiftUI diff/layout work. This is the strongest confirmed invalid
   refresh storm.

2. Secondary P0: the 1.5 second background reconcile loop. It currently creates
   40 `refreshThreads` calls per minute and has no scroll-interaction pause. The
   identical-thread publish part is already guarded, but the loop still performs
   network refresh, merge/equality work, and a 0.31 ms widget projection at the
   same cadence. This is periodic main-actor work that can line up with a drag or
   deceleration frame.

3. P0 amplifier: `homeThreadSections` lives in the App view layer and has no
   reusable Equatable input, output, or cache. A selection-only change rederived
   all 50 rows while only 2 row values changed. This does not create invalidation
   by itself, but it turns unrelated model publications into full list work.

4. P1: per-row typing badge timelines. The math is cheap in isolation, but 12
   running rows still mean 360 timeline invalidations per second. This is likely
   a secondary scroll smoothness tax, especially when combined with model
   publishes.

Not primary from this headless pass:

- `LazyVStack` virtualization: current row emission preserves lazy children.
- `AsyncImage` avatars: not reproduced by the synthetic no-remote-avatar harness;
  remote avatar URLs can still be a separate image-loading cost.
- `runStateByThread`: state assignment is guarded, but a real run-state change
  still maps and republishes the full `threads` array for one row. Treat it as a
  secondary invalidation source.

## Architecture Design

### 1. Move Home Presentation To Core

Add a Core presentation module, for example
`Sources/GaryxMobileCore/GaryxHomeThreadListPresentation.swift`.

Core types:

- `GaryxHomeThreadListInput: Equatable, Sendable`
- `GaryxHomeThreadSections: Equatable, Sendable`
- `GaryxHomeThreadRow: Identifiable, Equatable, Sendable`
- `GaryxHomeThreadRowAvatar: Equatable, Sendable`
- `GaryxHomeThreadSectionDeriver.derive(input:)`
- `GaryxHomeThreadSectionCache`

The input should include all real dependencies:

- ordered `threads`
- normalized pinned and recent ids
- selected thread id
- agents and teams
- automations or precomputed automation target thread ids
- busy thread ids from committed run state and local run tracker
- a coarse relative-time bucket, probably minute-level, so cached timestamps do
  not become permanently stale

The deriver should build agent/team dictionaries once per derivation and return
stable immutable rows. The App target should only assemble SwiftUI views from
the precomputed rows.

### 2. Publish A Cached Snapshot, Not A Computed View Property

Move the current private computed `model.homeThreadSections` out of the view
file. `GaryxMobileModel` should own a published snapshot:

```swift
@Published private(set) var homeThreadSections = GaryxHomeThreadSections()
```

After model state mutations, build a `GaryxHomeThreadListInput` and ask the
Core cache for sections. If the input fingerprint is unchanged, reuse the prior
sections and do not publish. If the output is unchanged, also avoid publishing.

This makes the home list a dumb renderer:

```swift
let sections = model.homeThreadSections
ForEach(sections.recent) { row in ... }
```

### 3. Add Equatable Publication Gates

Add a small assignment helper for `@Published` state that already conforms to
`Equatable`:

```swift
@discardableResult
func assignIfChanged<Value: Equatable>(_ current: inout Value, _ next: Value) -> Bool
```

Apply it first to home-list inputs:

- `agents`
- `teams`
- `automations`
- thread runtime summary mapping in `applyThreadRunStateSummary`
- thread runtime metadata mapping in `applyThreadRuntimeSummary`

`threads`, pinned ids, recent ids, and selected thread already have partial
guards; keep those and extend the same style where full-array mapping currently
publishes unconditionally.

### 4. Centralize Refresh Cadence And Scroll Pausing

The existing `isThreadListInteracting` signal is local to
`GaryxHomeThreadListView`. Promote the signal into the model or a small refresh
coordinator so every thread-list refresh source can observe it.

Policy:

- Home visible silent refresh remains low frequency, about every 10 seconds.
- Background committed-run reconcile should not call `refreshThreads` every
  1.5 seconds when there are no candidate running threads.
- During list interaction, skip or defer background reconcile refreshes.
- On transition back to idle, run one trailing refresh after a short debounce.
- Coalesce competing silent refresh requests by reason, with a minimum interval,
  so the home loop and background loop cannot stack refreshes.

Tradeoff: pausing refresh while scrolling can delay completion indicators by a
few seconds. That is preferable to dropped frames and is bounded by the trailing
refresh.

### 5. Reduce Badge Animation Cost

Replace per-row `TimelineView` clocks with one shared list-level clock or a
Core-provided phase value. Lower the cadence to 10-15 fps for the small typing
badge and pause it while `isThreadListInteracting` is true.

Tradeoff: the badge becomes less fluid while scrolling or at rest, but it
retains the semantic "running" signal and removes independent per-row animation
invalidations.

### 6. Keep Virtualization, Add Guardrails

The current `LazyVStack` structure is correct because rows are direct lazy
children. Keep the comment and add a test or review checklist item that prevents
wrapping all rows in a parent `VStack`.

If the root list grows beyond roughly 100 visible candidates, evaluate `List` or
an explicit windowed model. That is not required by this 50-row measurement.

## Test Strategy For Implementation

Convert the test-only harness into production Core tests when implementing:

- Core derivation returns the same pinned/recent rows as the current UI logic.
- Equal `GaryxHomeThreadListInput` returns the cached sections and does not bump
  derivation count.
- Changing only the selected thread changes exactly the old and new selected
  rows.
- Equal agents, teams, and automations do not publish or recompute sections.
- Background reconcile cadence is skipped while the thread list is interacting.
- Badge phase provider produces one shared phase and can be paused.

Keep these as SwiftPM tests; no UI runner is needed for the correctness and
cadence contracts. A later implementation should still be checked with an iOS
app build and, if allowed, a visual/device profiler pass for SwiftUI layout and
animation cost.
