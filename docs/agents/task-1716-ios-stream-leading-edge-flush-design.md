# TASK-1716 iOS Thread-Stream Leading-Edge Flush Design

Revision history:

- rev1: leading-edge gate driven per stream action — **rejected** by design
  review #TASK-1719 (a frame's rows would trigger a leading flush before the
  same frame's snapshot applied).
- rev2: gate driven per frame at the frame-final snapshot apply — **rejected**
  by design re-review #TASK-1721 (a no-op caught-up snapshot-only frame would
  open the window and shield the next real update for the full 3s).
- rev3 (this document): frame-settled gate with visible-change gating; a
  frame drives the gate only when it changed render inputs. Also hardens the
  in-flight flush guard against no-op cache re-applies (content equivalence
  instead of object identity), closing a lost-render hazard found while
  revising.

## Goal

Fix the iOS per-thread stream batching bug confirmed by the #TASK-1710 latency
audit (`docs/agents/audit-message-pipeline-latency.md` §2.4, recommendation #2):
the 3-second committed-row coalescing window is trailing-only, so even the
first update after a quiet period waits the full 3 seconds before it renders.

Target semantics (leading-edge throttle over *visible changes*):

- **Quiet edge:** the first frame that changes visible state flushes
  immediately — zero added latency.
- **Within the window:** subsequent changing frames are absorbed (trailing
  coalescing).
- **Window end:** if changing frames arrived during the window, flush the
  backlog once and re-arm; otherwise close (the next change is a leading edge).
- **No-op frames** (nothing visible changed — e.g. the caught-up
  snapshot-only frame a (re)connect delivers) never flush, never open the
  window, never mark backlog.
- **Sustained bursts** (catch-up replay) still collapse to one visible update
  per window. The window length (3s) does not change in this task.

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

### Frame anatomy (what the gate must model)

The stream's arrival unit is one `thread_render_frame`.
`GatewayStreamFrameProcessor.processRenderFrame`
(`Sources/GaryxMobileCore/GatewayStreamActor.swift:137-207`) splits a frame
into an ordered action list the actor delivers one at a time (:361-368):

- normal frame: `[.applyCommittedMessages?, .applyRenderSnapshot]` — the
  snapshot is **always the frame's final action** (:202-206), including
  snapshot-only caught-up frames (`events: []`, contract-guaranteed on every
  caught-up (re)connect);
- windowed reset frame: `[.resetCommittedCacheBelow, .applyCommittedMessages?,
  .applyRenderSnapshot]` — snapshot still final;
- seq-gap frame (:174-181): committed rows only, **no snapshot**, followed by
  a reconnect whose replay re-delivers a full frame;
- control-rewrite frame (:187-194): rows + `.refetchAfterControlRewrite`, no
  snapshot; the refetch path re-renders via `loadSelectedThreadHistory`
  outside this gate.

Two review counterexamples pin the drive rules:

1. **#TASK-1719 (rev1):** driving the gate from `.applyCommittedMessages`
   fires the leading flush *between* a frame's rows and its snapshot — the
   flush renders the previous `render_state` (visible rows come exclusively
   from the server snapshot) and the frame's own snapshot then waits out the
   window. ⇒ the gate must settle at the frame-final snapshot apply, with
   rows never scheduling.
2. **#TASK-1721 (rev2):** a caught-up snapshot-only frame whose snapshot
   equals the applied one (the normal first frame of every (re)connect) must
   not count as an update at all — otherwise it opens the window, its leading
   flush repaints identical content, and the *next* frame (the user-visible
   one, e.g. the reply that prompted opening the thread) gets absorbed for
   the full 3s. ⇒ the gate must only be driven by frames that *changed*
   render inputs.

### Core state machine (new, in `GaryxMobileCore`)

`GaryxStreamFlushGate` lives in
`Sources/GaryxMobileCore/GaryxTranscriptSyncPlanner.swift` (same file as
`GaryxStreamUpdateCadence`; no new source file, no pbxproj churn). Pure and
clock-free: the app target owns the timer and side effects; the gate owns
every decision.

```swift
public struct GaryxStreamFlushGate: Equatable, Sendable {
    public enum State: Equatable, Sendable {
        case idle
        case window(hasBacklog: Bool)
    }
    public enum FrameAction: Equatable, Sendable {
        case flushNowAndArmWindow   // quiet edge: render immediately, arm window timer
        case absorb                 // window open: coalesce; window end drains backlog
        case skip                   // no visible change: no flush, no window, no backlog
    }
    public enum WindowAction: Equatable, Sendable {
        case flushBacklogAndRearmWindow
        case closeWindow
    }

    public private(set) var state: State = .idle
    private var hasPendingVisibleChange = false

    /// What the app feeds the gate, not a decision: did this frame's snapshot
    /// differ from the one already applied for the thread?
    public static func snapshotChanged(
        _ incoming: GaryxRenderSnapshot,
        appliedBefore applied: GaryxRenderSnapshot?
    ) -> Bool { incoming != applied }

    /// Committed rows merged into the cache, or the cache floor was reset —
    /// body-store changes that can alter how snapshot refs resolve (e.g. a
    /// body-less placeholder upgrading once its row arrives), even when the
    /// snapshot itself is unchanged. Accumulates until the next settle.
    public mutating func recordVisibleChange()

    /// Frame finished applying (the frame-final snapshot apply). Decides the
    /// flush for everything accumulated since the last settle.
    public mutating func settleFrame(snapshotChanged: Bool) -> FrameAction

    /// Armed window timer fired.
    public mutating func windowElapsed() -> WindowAction

    /// Stream stopped / thread switched / cache refetched / home drain.
    public mutating func reset()
}
```

Transition table (`pending` = `hasPendingVisibleChange`, always cleared by
`settleFrame`; `drives` = `pending || snapshotChanged`):

| State | Event | Condition | Action | Next state |
|---|---|---|---|---|
| `idle` | `settleFrame` | `drives` | `.flushNowAndArmWindow` | `.window(false)` |
| `idle` | `settleFrame` | `!drives` | `.skip` | `idle` |
| `.window(b)` | `settleFrame` | `drives` | `.absorb` | `.window(true)` |
| `.window(b)` | `settleFrame` | `!drives` | `.skip` | `.window(b)` |
| any | `recordVisibleChange` | — | — (pending = true) | unchanged |
| `.window(true)` | `windowElapsed` | — | `.flushBacklogAndRearmWindow` | `.window(false)` |
| `.window(false)` | `windowElapsed` | — | `.closeWindow` | `idle` |
| `idle` | `windowElapsed` (defensive; cancelled-timer race) | — | `.closeWindow` | `idle` |
| any | `reset` | — | — (pending = false) | `idle` |

Notes:

- Re-arming after a backlog flush keeps a sustained burst at one flush per
  window (a close-after-trailing variant would degrade to trailing + adjacent
  leading double-flushes).
- `.skip` inside a window does **not** mark backlog: a pure no-op frame must
  not cause a window-end repaint, and a no-op heartbeat stream must not keep
  the window re-arming forever.
- `pending` survives a snapshot-less frame (seq-gap): the rows' change is
  settled by the next frame that does carry a snapshot (the post-reconnect
  replay frame), even if that snapshot happens to equal the applied one —
  this is what renders body-less placeholder upgrades from replayed rows.
- `windowElapsed` does not consume `pending` (there is nothing new to show
  without a snapshot settle); the backlog flag alone decides the window end.

### App-target wiring (`GaryxMobileModel+ThreadStream.swift`)

New stored state on the model (next to `selectedThreadStreamFlushTask` in
`GaryxMobileModel.swift`):

```swift
var selectedThreadStreamFlushGate = GaryxStreamFlushGate()
```

`selectedThreadStreamFlushTask` keeps a single meaning — "the armed window
timer" — and no longer performs the leading flush itself.

Ingest paths (the only callers, verified by rg: stream action dispatch
:186-202 is the sole entry):

- `applyStreamedCommittedMessages` (rows): merge into the cache exactly as
  today, then `gate.recordVisibleChange()`. **No scheduling.**
- `dropCommittedCacheBelow` (windowed floor reset): drop rows as today, then
  `gate.recordVisibleChange()`.
- `applyThreadRenderSnapshot` (frame-final): capture the applied snapshot
  *before* storing the new one, apply everything exactly as today, then
  settle:

```swift
private func applyThreadRenderSnapshot(_ snapshot: GaryxRenderSnapshot, threadId: String) {
    guard selectedThread?.id == threadId else { return }
    // renderSnapshot(for:) is the in-memory accessor the row mapper renders
    // from (never the disk-lazy transcriptSnapshot): an unrendered on-disk
    // window equal to the incoming frame must still count as a change.
    let appliedBefore = renderSnapshot(for: threadId)
    // ... existing body unchanged (setRenderSnapshot, pagination, cache
    //     window rebuild, conditional persist, markThreadHistoryLoaded) ...
    settleSelectedThreadStreamFrame(
        snapshotChanged: GaryxStreamFlushGate.snapshotChanged(snapshot, appliedBefore: appliedBefore),
        for: threadId
    )
}

private func settleSelectedThreadStreamFrame(snapshotChanged: Bool, for threadId: String) {
    switch selectedThreadStreamFlushGate.settleFrame(snapshotChanged: snapshotChanged) {
    case .flushNowAndArmWindow:
        armSelectedThreadStreamFlushWindow(for: threadId)
        Task { [weak self] in
            await self?.flushSelectedThreadStreamWindow(for: threadId)
        }
    case .absorb, .skip:
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

`scheduleSelectedThreadStreamFlush` (:314-326) is replaced by
`settleSelectedThreadStreamFrame` — there is deliberately no
"schedule-per-update" entry point anymore.

### In-flight flush guard: content equivalence, not object identity

`flushSelectedThreadStreamWindow` today aborts a prepared render when the
cache window object changed during the off-main prepare
(`cachedTranscriptSnapshots[threadId] == window`, :343). `GaryxCachedTranscript`
is `Equatable` **including `savedAt`**, so a no-op re-apply (a caught-up
snapshot-only frame rebuilds the window with a fresh `savedAt` and identical
content) breaks object equality.

Lost-render hazard (found in rev3, must be closed): quiet edge → changing
frame settles → leading flush starts preparing → a no-op caught-up frame
re-applies (new `savedAt`, `.skip`, no backlog) → the leading flush aborts on
the stale guard → window end closes with no backlog → **the change never
renders** until the next real frame. Today's code survives this only because
every apply re-schedules the 3s timer; rev3's `.skip` removes that accident,
so the guard must compare *render inputs*:

```swift
// GaryxMobileCore, next to GaryxCachedTranscript
/// Render-input equivalence: ignores `savedAt` (refreshed by no-op
/// re-applies) so an in-flight flush only aborts when the prepared output
/// could actually differ.
public func renderEquivalent(to other: GaryxCachedTranscript) -> Bool {
    version == other.version
        && threadId == other.threadId
        && messages == other.messages
        && renderSnapshot == other.renderSnapshot
        && hasMoreBefore == other.hasMoreBefore
        && nextBeforeIndex == other.nextBeforeIndex
}
```

The flush guard becomes
`cachedTranscriptSnapshots[threadId]?.renderEquivalent(to: window) == true`.
A *content* change during prepare still aborts — and that frame settled
`.absorb`/`pending`, so the window-end flush re-renders (unchanged self-heal,
now guaranteed by the gate). Also drop the flush body's first two lines
(:335-336, cancel/nil of the timer): timer lifecycle lives exclusively in
arm/elapse/cancel helpers — a flush must not destroy the window the leading
edge just armed.

Every existing cancel site becomes one shared helper so the gate can never
disagree with the timer (I1):

```swift
private func cancelSelectedThreadStreamFlushWindow() {
    selectedThreadStreamFlushTask?.cancel()
    selectedThreadStreamFlushTask = nil
    selectedThreadStreamFlushGate.reset()
}
```

Converted call sites (behavior per site unchanged apart from the gate reset):
`stopSelectedThreadStream` (:81-82), `stopSelectedThreadStreamForHome` (both
branches, :94-95 and :101-102; the home drain then calls
`flushSelectedThreadStreamWindow` directly as a terminal flush outside the
gate — same as today), `refetchSelectedThreadAfterTranscriptRewrite`
(:206-207; the first post-refetch frame is a leading edge again).

### Invariants

- **I1 — gate/timer coupling:** `selectedThreadStreamFlushTask != nil` iff
  `gate.state == .window`. The only transitions into `.window`
  (`flushNowAndArmWindow`, `flushBacklogAndRearmWindow`) arm the timer at the
  same call site; the only ways out (`closeWindow`, `reset`) nil/cancel it at
  the same call site. `.skip` and `.absorb` touch neither.
- **I2 — flush reads latest state:** a flush carries no payload; it renders
  `cachedTranscriptSnapshots[threadId]` at flush time, so a late flush cannot
  regress the UI.
- **I3 — no lost updates:** every render-input change is either settled
  (`flushNow`/`absorb`→backlog) or pending (`recordVisibleChange` awaiting
  its frame's snapshot). An in-flight flush aborts only on a *content*
  change (renderEquivalent guard), and that content change itself settled
  `.absorb` or is pending — the window-end flush covers it.
- **I4 — no-op frames are inert:** a frame with an unchanged snapshot and no
  row/floor changes produces `.skip`: no flush, no window state change, no
  backlog, and (via renderEquivalent) no in-flight flush abort.

### Race walkthrough (all gate/timer transitions on `@MainActor`)

1. **Timer fires vs. concurrent stop/re-arm:** `arm…` always cancels the
   previous timer first; `…WindowDidElapse` re-checks `Task.isCancelled` on
   the MainActor before touching the handle or the gate, so a stale cancelled
   timer that already passed its post-sleep check cannot nil a newer handle
   or double-drive the gate.
2. **Thread switch mid-window:** `stopSelectedThreadStream` cancels + resets;
   the next thread's first changing frame is a leading edge. An in-flight
   flush for the old thread is defused by the existing
   `selectedThread?.id == threadId` guard.
3. **Rows and snapshot of one frame:** the actor delivers a frame's actions
   in order; rows only mark `pending`, and the frame-final snapshot apply is
   the only settle point. A leading flush therefore always renders the
   complete frame — a mid-frame flush over a half-applied frame is
   structurally impossible. (#TASK-1719 closed.)
4. **(Re)connect caught-up no-op frame:** snapshot equals applied, no rows ⇒
   `.skip`, gate stays `idle`, no window opens. The next real frame flushes
   immediately. (#TASK-1721 closed.)
5. **No-op frame racing an in-flight leading flush:** the no-op re-apply
   refreshes `savedAt` only; the renderEquivalent guard lets the in-flight
   flush complete instead of aborting into a backlog-less window end. (rev3
   lost-render hazard closed.)
6. **Interleaving with the next frame:** the leading flush is an unstructured
   `Task` and may interleave with the next frame's apply — harmless in every
   order (I2 + I3): it renders newer-or-equal state; if prepare aborted, the
   aborting change's own settle drives the window-end flush.
7. **Home drain:** `stopSelectedThreadStreamForHome` cancels + resets, then
   drains once immediately (terminal flush, `respectingTaskCancellation:
   true`) — unchanged semantics, clean gate for the next open.

### What does not change

- Window length (3s), `GaryxStreamUpdateCadence.committedMessageBatchWindowNanos`.
- Data-layer merging and storage: rows still merge per-row into the cache
  (cursor stays current); snapshots still store immediately; pagination,
  `markThreadHistoryLoaded`, persist-when-idle all keep their positions.
  Only *render cadence* changes.
- Seq/gap handling (`GaryxStreamSeqPlanner`), reconnect/fallback logic,
  desktop, gateway.
- No render_state recomputation on the client (repository contract).

## Failure-mode analysis

| Risk | Mitigation |
|---|---|
| Gate says window open but timer dead → changes absorbed forever | I1: single helper owns cancel+reset; arm sites pair action+timer; transition guard tests |
| First change after `reset()` not leading | `reset()` → `idle` + pending cleared, unit-tested |
| Burst degrades to per-frame flush | no transition flushes on `settleFrame` while `.window`; sustained-burst test asserts 1 leading + 1 per elapsed window |
| Stale flush overwrites newer state | I2 + renderEquivalent abort guard |
| Trailing flush lost on prepare-abort | I3: the aborting content change settled `.absorb` or is pending |
| Leading flush fires between a frame's rows and snapshot, rendering old `render_state` (#TASK-1719) | rows never settle; the frame-final snapshot apply is the only settle point; frame ordering pinned by processor tests |
| No-op caught-up frame opens the window and shields the next real update (#TASK-1721) | visible-change gating: `.skip` neither flushes nor opens/dirties the window |
| No-op re-apply during prepare aborts the leading flush → change never renders (rev3 hazard) | renderEquivalent (ignores `savedAt`) replaces object-identity guard |
| Replayed rows with an unchanged tail snapshot never render (placeholder upgrade) | rows mark `pending`; the next settle drives even with `snapshotChanged == false` |

## Test Plan (SwiftPM, `GaryxMobileCoreTests`)

Reproduce-first: the gate lands with a **status-quo faithful** `settleFrame`
(any settle → open/keep window, `.absorb`, never immediate). The new tests
below then fail **red** against it, reproducing "first frame waits the full
window" and "no-op frames arm the window" at the Core layer; flipping to the
rev3 table turns them green. Both `swift test` outputs are recorded in the
task summary.

Gate tests (pure, clock-free — `windowElapsed()` *is* the timer edge):

1. `testQuietFirstChangingFrameFlushesImmediately` — red→green core of the fix.
2. `testChangingFramesWithinWindowAreAbsorbed`.
3. `testWindowEndFlushesBacklogAndRearms`; `testWindowEndWithoutBacklogCloses`.
4. `testQuietAfterCloseFlushesImmediatelyAgain`.
5. `testSustainedBurstFlushesOncePerWindow` — K windows of continuous
   changing frames ⇒ exactly 1 leading + K trailing flushes, never two flush
   actions without an intervening `windowElapsed`.
6. `testNoOpCaughtUpFrameDoesNotOpenWindow` (**#TASK-1721 regression**) —
   idle + `settleFrame(snapshotChanged: false)` ⇒ `.skip`, still idle; the
   next changing frame ⇒ `.flushNowAndArmWindow`.
7. `testNoOpFrameInsideWindowLeavesBacklogAlone` — leading, then no-op settle
   ⇒ `.skip`; `windowElapsed` ⇒ `.closeWindow` (no repaint, no re-arm loop).
8. `testPendingRowsDriveSettleWithUnchangedSnapshot` — `recordVisibleChange()`
   then `settleFrame(snapshotChanged: false)` ⇒ drives (leading when idle,
   `.absorb`+backlog in window); covers replayed-row placeholder upgrades and
   the seq-gap frame whose settle arrives with the post-reconnect frame.
9. `testResetClearsPendingAndWindow` — mid-window + pending, `reset()` ⇒
   idle; `settleFrame(false)` ⇒ `.skip`; changing frame ⇒ leading.
10. `testWindowElapsedWhenIdleIsBenign` — defensive `.closeWindow`, idle.
11. `testSnapshotChangedHelper` — equal snapshot ⇒ false; differing ⇒ true;
    nil applied ⇒ true.
12. Existing `testCommittedStreamBatchWindowIsThreeSeconds` stays.

Frame/wiring tests:

13. `testFrameActionsAlwaysEndWithRenderSnapshot`
    (`GatewayStreamFrameProcessor`, already landed green) — rows+snapshot,
    snapshot-only, windowed (`reset` first, snapshot last), seq-gap (rows,
    no snapshot, reconnect), control-rewrite (rows + refetch, no snapshot).
14. `testFrameLevelGateDriveRendersWholeFrameOnQuietEdge` — mirror the wiring
    rule over real processor output: rows → `recordVisibleChange`, snapshot
    apply → `settleFrame(snapshotChanged: snapshot != applied)` tracking an
    `applied` variable exactly like the app. Asserts: a no-op caught-up frame
    ⇒ `.skip` and no window; a changing frame ⇒ one leading flush after the
    whole frame applied (rows never drive mid-frame); a snapshot-less gap
    frame ⇒ no settle, pending carries to the replay frame's settle.
15. `testRenderEquivalentIgnoresSavedAtOnly` — same content + different
    `savedAt` ⇒ equivalent; differing messages / renderSnapshot / pagination
    fields ⇒ not equivalent.

Path-parity note: rows cannot outrun or lag the snapshot cadence because rows
never settle — the frame-final snapshot apply is the single settle point
(tests 13/14 pin the ordering and the drive rule).

## Validation

- `swift test` in `mobile/garyx-mobile` (red run, then green run; full suite).
- `xcodebuild` of the iOS app target (guards against SwiftPM-only false
  green; no new files, so no `xcodegen generate` needed — verified by the
  build).
- Design + code review by codex (separate tasks, `--notify current-thread`),
  merge to main only after PASS.
