# TASK-1716 iOS Thread-Stream Leading-Edge Flush Design

## Goal

Fix the iOS per-thread stream batching bug confirmed by the #TASK-1710 latency
audit (`docs/agents/audit-message-pipeline-latency.md` §2.4, recommendation #2):
the 3-second committed-row coalescing window is trailing-only, so even the
first update after a quiet period waits the full 3 seconds before it renders.

Target semantics (leading-edge throttle):

- **Quiet edge (no window open):** the first update flushes to the UI
  immediately — zero added latency.
- **Within the window:** subsequent updates are absorbed (trailing
  coalescing), exactly as today.
- **Window end:** if updates arrived during the window, flush the accumulated
  backlog once and re-arm the window; if nothing arrived, close the window so
  the next update is again a leading edge.
- **Sustained bursts** (catch-up replay) still collapse to one visible
  update per window — the SwiftUI list-rebuild protection that motivated the
  3s window is preserved. The window length itself (3s) does not change in
  this task.

## Current Behavior (the bug)

`GaryxMobileModel+ThreadStream.swift:314-326` — `scheduleSelectedThreadStreamFlush`:

```swift
guard selectedThreadStreamFlushTask == nil else { return }
selectedThreadStreamFlushTask = Task { [weak self] in
    try? await Task.sleep(nanoseconds: Self.streamedCommittedFlushDelayNanos)  // 3s
    guard !Task.isCancelled else { return }
    await self?.flushSelectedThreadStreamWindow(for: threadId)
}
```

The first row after a quiet period only *schedules* a flush; the render
happens after the full `Task.sleep`. The doc comment claims
"Leading-throttle" but the implementation is a trailing debounce-window: the
first visible update is always delayed by up to 3s. Both ingest paths feed
this scheduler:

- committed rows: `applyStreamedCommittedMessages` (:245-262)
- render snapshots: `applyThreadRenderSnapshot` (:264-292)

Combined with the claude run-tail delay (audit #1), the final answer takes
~7s to appear on iOS.

Constants: `GaryxStreamUpdateCadence.committedMessageBatchWindowNanos = 3s`
(`Sources/GaryxMobileCore/GaryxTranscriptSyncPlanner.swift:41-46`), aliased as
`GaryxMobileModel.streamedCommittedFlushDelayNanos`
(`App/GaryxMobile/GaryxMobileModel.swift:58`).

## Design

### Core state machine (new, in `GaryxMobileCore`)

Add `GaryxStreamFlushGate` to
`Sources/GaryxMobileCore/GaryxTranscriptSyncPlanner.swift` (same file as
`GaryxStreamUpdateCadence`; no new source file, so no pbxproj churn). It is a
pure, clock-free value type: the app target owns the timer; the gate owns
every decision.

```swift
/// Leading-edge throttle for selected-thread stream flushes.
public struct GaryxStreamFlushGate: Equatable, Sendable {
    public enum State: Equatable, Sendable {
        case idle
        case window(hasBacklog: Bool)
    }
    public enum UpdateAction: Equatable, Sendable {
        case flushNowAndArmWindow   // quiet edge: render immediately, start window timer
        case absorb                 // window open: coalesce; window end handles the backlog
    }
    public enum WindowAction: Equatable, Sendable {
        case flushBacklogAndRearmWindow  // updates arrived during the window
        case closeWindow                 // quiet window: next update is a leading edge
    }

    public private(set) var state: State = .idle
    public init() {}

    public mutating func recordUpdate() -> UpdateAction
    public mutating func windowElapsed() -> WindowAction
    public mutating func reset()
}
```

Transition table:

| State | Event | Action | Next state |
|---|---|---|---|
| `idle` | `recordUpdate` | `.flushNowAndArmWindow` | `.window(hasBacklog: false)` |
| `.window(_)` | `recordUpdate` | `.absorb` | `.window(hasBacklog: true)` |
| `.window(hasBacklog: true)` | `windowElapsed` | `.flushBacklogAndRearmWindow` | `.window(hasBacklog: false)` |
| `.window(hasBacklog: false)` | `windowElapsed` | `.closeWindow` | `.idle` |
| `idle` | `windowElapsed` (defensive; cancelled-timer race) | `.closeWindow` | `.idle` |
| any | `reset` | — | `.idle` |

Re-arming after a backlog flush (instead of closing) is what keeps a
sustained burst at exactly one flush per window; a close-after-trailing
variant would degrade to two adjacent flushes (trailing + next leading) per
window under continuous traffic.

### App-target wiring (`GaryxMobileModel+ThreadStream.swift`)

New stored state on the model (next to `selectedThreadStreamFlushTask` in
`GaryxMobileModel.swift`):

```swift
var selectedThreadStreamFlushGate = GaryxStreamFlushGate()
```

`selectedThreadStreamFlushTask` keeps its single meaning "the armed window
timer", but the timer no longer performs the leading flush itself.

```swift
private func scheduleSelectedThreadStreamFlush(for threadId: String) {
    switch selectedThreadStreamFlushGate.recordUpdate() {
    case .flushNowAndArmWindow:
        armSelectedThreadStreamFlushWindow(for: threadId)
        Task { [weak self] in
            await self?.flushSelectedThreadStreamWindow(for: threadId)
        }
    case .absorb:
        break
    }
}

private func armSelectedThreadStreamFlushWindow(for threadId: String) {
    selectedThreadStreamFlushTask?.cancel()
    selectedThreadStreamFlushTask = Task { [weak self] in
        try? await Task.sleep(nanoseconds: Self.streamedCommittedFlushDelayNanos)
        guard !Task.isCancelled else { return }
        await self?.selectedThreadStreamFlushWindowDidElapse(for: threadId)
    }
}

private func selectedThreadStreamFlushWindowDidElapse(for threadId: String) async {
    guard !Task.isCancelled else { return }
    selectedThreadStreamFlushTask = nil
    switch selectedThreadStreamFlushGate.windowElapsed() {
    case .flushBacklogAndRearmWindow:
        armSelectedThreadStreamFlushWindow(for: threadId)
        await flushSelectedThreadStreamWindow(for: threadId)
    case .closeWindow:
        break
    }
}
```

`flushSelectedThreadStreamWindow` drops its first two lines
(`selectedThreadStreamFlushTask?.cancel(); … = nil` at :335-336): cancelling
the window timer from inside the flush would destroy the window that the
leading edge just armed. Timer lifecycle now lives exclusively in
arm/elapse/cancel helpers. The rest of the flush body (thread guard, stale
`window == cachedTranscriptSnapshots[threadId]` guard, prepare off-main,
run-state apply, conditional persist) is unchanged.

Every existing cancel site becomes one shared helper so the gate can never
disagree with the timer (invariant I1 below):

```swift
private func cancelSelectedThreadStreamFlushWindow() {
    selectedThreadStreamFlushTask?.cancel()
    selectedThreadStreamFlushTask = nil
    selectedThreadStreamFlushGate.reset()
}
```

Call sites converted to the helper (behavior per site unchanged apart from
the added gate reset):

- `stopSelectedThreadStream` (:81-82)
- `stopSelectedThreadStreamForHome` (both branches, :94-95 and :101-102);
  the home drain then calls `flushSelectedThreadStreamWindow` directly as a
  terminal flush, outside the gate — same as today.
- `refetchSelectedThreadAfterTranscriptRewrite` (:206-207) — the
  control-rewrite path clears the cache and refetches history; the gate
  resets so the first post-refetch stream row is a leading edge again.

### Invariants

- **I1 — gate/timer coupling:** `selectedThreadStreamFlushTask != nil` iff
  `gate.state == .window`. Maintained by construction: the only transitions
  into `.window` (`flushNowAndArmWindow`, `flushBacklogAndRearmWindow`)
  arm the timer at the same call site; the only ways out (`closeWindow`,
  `reset`) nil/cancel it at the same call site.
- **I2 — flush reads latest state:** a flush never carries payload; it
  renders `cachedTranscriptSnapshots[threadId]` at flush time. A late flush
  therefore cannot regress the UI to older data.
- **I3 — no lost updates:** any update that arrives while a flush is
  preparing (off-main `prepare`) has already gone through `recordUpdate()`,
  so the gate holds `hasBacklog = true` and the window-end flush re-renders.
  The existing stale-window guard (`cachedTranscriptSnapshots[threadId] ==
  window`, :343) makes the superseded flush abort instead of rendering old
  data; the trailing flush covers it. This is exactly today's
  self-heal, now guaranteed by the gate rather than by the next 3s timer.

### Race walkthrough (all on `@MainActor`, so transitions are serialized)

1. **Timer fires vs. concurrent stop/re-arm:** `arm…` always cancels the
   previous timer before installing a new one, and
   `…WindowDidElapse` re-checks `Task.isCancelled` on the MainActor before
   touching the task handle or the gate. A cancelled stale timer that already
   passed its post-sleep check therefore returns without nil-ing a newer
   timer handle or double-driving the gate.
2. **Thread switch mid-window:** `stopSelectedThreadStream` (called by
   `startSelectedThreadStream` and by stream-policy stop) cancels the timer
   and resets the gate; the first update of the next thread is a leading
   edge. A leading-flush `Task` still in flight for the old thread is
   defused by the existing `selectedThread?.id == threadId` guard.
3. **Snapshot and rows interleaving:** both paths call the same
   `scheduleSelectedThreadStreamFlush`, hence share one gate — identical
   cadence by construction, no rows/run-state skew between the two paths
   (they were already coupled through the same flush; this stays).
4. **Home drain:** `stopSelectedThreadStreamForHome` cancels + resets, then
   drains once immediately (terminal flush, `respectingTaskCancellation:
   true`) — unchanged semantics, now with a clean gate for the next open.

### What does not change

- Window length (3s), `GaryxStreamUpdateCadence.committedMessageBatchWindowNanos`.
- Data-layer merging: `applyStreamedCommittedMessages` still merges each row
  into the cache immediately (cursor stays per-row current);
  `applyThreadRenderSnapshot` still stores the snapshot immediately. Only
  *render cadence* changes.
- Persistence policy (persist on flush only when the run is idle), seq/gap
  handling (`GaryxStreamSeqPlanner`), reconnect/fallback logic, desktop.
- No render_state recomputation on the client (repository contract).

## Failure-mode analysis

| Risk | Mitigation |
|---|---|
| Gate says window open but timer dead → updates absorbed forever | I1: single helper owns cancel+reset; arm sites pair action+timer; guard test for every transition |
| First event after `reset()` not leading | `reset()` → `.idle` covered by unit test |
| Burst degrades to per-event flush | transition table has no path that flushes on `recordUpdate` while `.window`; sustained-burst test asserts flush count == 1 leading + 1 per elapsed window |
| Stale flush overwrites newer state | I2 + existing stale-window abort guard (unchanged) |
| Trailing flush lost when prepare-abort happens | I3: the aborting update itself set `hasBacklog` |

## Test Plan (SwiftPM, `GaryxMobileCoreTests`)

Reproduce-first: the gate lands first with a **status-quo faithful**
`recordUpdate` (`idle → .absorb`, i.e. first update only opens the window —
exactly what the app does today). The new test
`testQuietFirstUpdateFlushesImmediately` then **fails (red)** against that
implementation, reproducing "first row waits the full window" at the Core
layer. Flipping `recordUpdate` to the leading-edge table turns it green. Both
runs' actual `swift test` output are recorded in the task summary.

Guard tests (all pure, clock-free — `windowElapsed()` *is* the timer edge):

1. `testQuietFirstUpdateFlushesImmediately` — red→green core of the fix.
2. `testUpdatesWithinWindowAreAbsorbed` — N follow-up updates → all `.absorb`.
3. `testWindowEndFlushesBacklogAndRearms` — backlog → `.flushBacklogAndRearmWindow`, state back to `hasBacklog: false`.
4. `testWindowEndWithoutBacklogCloses` — quiet window → `.closeWindow` → `.idle`.
5. `testQuietAfterCloseFlushesImmediatelyAgain` — idle again → leading edge again.
6. `testSustainedBurstFlushesOncePerWindow` — drive K windows of continuous updates; assert exactly 1 leading + K trailing flushes and never two flush actions without an intervening `windowElapsed`.
7. `testResetReturnsToIdle` — mid-window reset → next update is leading (covers stop/refetch/home-drain wiring semantics).
8. `testWindowElapsedWhenIdleIsBenign` — defensive transition is a no-op `.closeWindow`.
9. Existing `testCommittedStreamBatchWindowIsThreeSeconds` stays (window length unchanged).

Path-parity note: rows and snapshots cannot diverge because both call sites
feed one gate — asserted structurally by the wiring diff (one schedule
function), exercised by test 6's interleaving being source-agnostic.

## Validation

- `swift test` in `mobile/garyx-mobile` (red run, then green run; full suite).
- `xcodebuild` of the iOS app target (guards against SwiftPM-only false
  green; no new files, so no `xcodegen generate` needed — verified by the
  build).
- Design + code review by codex (separate tasks, `--notify current-thread`),
  merge to main only after PASS.
