# Home Projection Cutover Design

## Goal

Cut the iOS home thread list live rendering source from the legacy
`GaryxHomeThreadListStore.apply(_:)` path to `HomeProjectionActor` snapshots.
The main actor may still collect value snapshots from already-published model
state, but it must not synchronously derive home list sections, scan running
rows, or publish the live home list from property observers.

This is the cutover step from `home-list-rebuild-v4.md` currently labeled M4
there and M5 in the task brief.

The cutover must be guarded by a runtime rollback flag. Turning that flag off
must restore the old live rendering path rather than disabling home updates.

## Current State

`GaryxMobileModel.refreshHomeThreadListSnapshot()` still does three things on
the main actor:

1. Builds `homeThreadListInput`, including `homeThreadRunningThreadIds`.
2. Calls `homeThreadListStore.apply(input)`, which can rebuild sections and
   publish the list synchronously.
3. Captures the same input for `HomeProjectionGateway` shadow parity.

The current call graph is 13 call sites plus the function definition:

- `GaryxMobileModel.swift`: `threads`, `selectedThread`, `isLoadingThreads`,
  `runTracker`, `runStateByThread`, `navigationState`, `pinnedThreadIds`,
  `recentThreadIds`, `agents`, `automations`, and init bootstrap.
- `GaryxMobileModel+Presentation.swift`: debug home-scroll fixture bootstrap.
- `GaryxMobileModel+Presentation.swift`: the function definition.

`HomeProjectionActor` already owns the reducer and emits `HomeSnapshot` plus
`CollectionDifference<String>`. `HomeProjectionGateway` already provides
transaction coalescing, latest-boundary draining, and parity counters.

## Proposed Shape

### 1. Rename the main-actor entry point

Replace the live meaning of `refreshHomeThreadListSnapshot()` with a new
`emitHomeProjectionSnapshot()` style helper. Existing property observers should
call the emitter, not a function whose name implies main-thread rendering.

The emitter builds a `HomeProjectionCapture` directly from model fields:

- `threads`
- `recentThreadIds`
- `agents`
- `automations`
- `pinnedThreadIds`
- `selectedThread?.id`
- `isLoadingThreads`
- `isHomeVisible`
- `runTracker.busyThreadIds`
- `runStateByThread.mapValues(\.busy)`

It must not build `GaryxHomeThreadListInput`, call
`homeThreadRunningThreadIds`, or call `homeThreadListStore.apply(_:)`.

The helper should keep calling
`syncBackgroundCommittedRunReconcileLoopForHomeVisibility()` so visibility
side effects stay coupled to the same state changes as before.

### 2. Make actor result application the only live store publisher

Extend `HomeProjectionGateway` with an optional main-actor result callback.
`GaryxMobileModel` registers a callback that applies each
`HomeProjectionBoundaryResult.snapshot` to `homeThreadListStore`.

Add a live apply API to `GaryxHomeThreadListStore`, for example:

```swift
func apply(actorSnapshot: HomeSnapshot, difference: CollectionDifference<String>?) -> Bool
```

The store keeps `latestActorAppliedSeq`, initially `0`, and only accepts actor
snapshots whose `appliedSeq` is greater than the last accepted sequence. Stale
actor completions from async hops are dropped before publishing.

The store must also keep the legacy content de-dupe:

```swift
guard snapshot != next else { return false }
```

`HomeProjectionReducer` increments `appliedSeq` for every accepted event and can
also bump the sequence for rejected/no-op run-state deltas while leaving the
visible snapshot unchanged. An `appliedSeq` guard alone would publish unchanged
content and fail the home-scroll probe gate. The live published value remains
`GaryxHomeThreadListSnapshot` so existing SwiftUI views stay dumb and unchanged.

The `difference` is accepted by the API now even if the current SwiftUI list does
not consume it yet; this keeps the live boundary aligned with the actor contract
and leaves M5/native-List work able to consume it without another cutover.

### 3. Keep parity live but out of the render path

`HomeProjectionActor.finishBoundary` should continue comparing actor output
against its private `checkpointStore` built from `state.legacyCheckpointInput()`.
That preserves `parityMismatchCount == 0` without needing the live store to run
legacy derivation.

Remove or stop passing the `liveLegacySnapshot` diagnostic from the live path,
because after cutover the live store is actor-backed by design. Existing tests
that validate the diagnostic can keep using the actor API directly.

### 4. Add a rollback flag distinct from shadow enablement

Add a cutover/live-source flag separate from
`HomeProjectionShadowConfiguration.isEnabled`, for example:

```swift
enum HomeProjectionLiveSourceConfiguration {
    static var usesActorSnapshots: Bool {
        // env GARYX_MOBILE_HOME_PROJECTION_CUTOVER, default true
    }
}
```

Default is actor-backed live rendering. When the flag is off, the model's home
emitter must restore the old live rendering path:

1. Build `homeThreadListInput`.
2. Call `homeThreadListStore.apply(input)`.
3. Capture shadow parity when `HomeProjectionShadowConfiguration.isEnabled`.
4. Keep the visibility reconcile side effect.

This gives the migration the required single-flag rollback. The shadow flag must
not become the rollback flag as long as its disabled behavior is gateway no-op;
otherwise disabling it after cutover leaves the home store permanently empty.

### 5. Preserve transaction boundaries and run-state sources

Keep existing `beginTransaction` / `endTransaction` boundaries around
`refreshThreads`, local archive remove, and archive rollback.

`runTracker` changes should emit a full capture through the same helper, because
`runTracker.busyThreadIds` is already part of the capture and has highest source
precedence in the reducer.

Committed transcript run-state changes should keep sending
`captureCommittedRunStateDelta(threadId:isRunning:)` for immediate actor updates.
Do not also emit a full capture from the `runStateByThread` property observer
for every per-thread dictionary subscript write. The explicit committed delta is
the immediate source for single-thread committed changes.

For bulk replacement/reset, use an explicit helper rather than trying to infer
intent inside `didSet`. For example, remove the `runStateByThread` didSet home
emission and introduce:

```swift
func replaceRunStateByThread(_ next: [String: GaryxTranscriptRunState]) {
    runStateByThread = next
    emitHomeProjectionSnapshot()
}
```

Use this helper for full resets such as gateway switch/debug snapshot reset.
Single-thread paths should keep assigning `runStateByThread[threadId] = state`
and then call `emitCommittedRunStateProjectionDelta(threadId:state:)`.

The existing `refreshHomeThreadsAfterLocalRunStateChange()` remote refresh is
kept as reconciliation, not as the immediate visual source.

### 6. Keep home visibility side effects

The old refresh helper also called
`syncBackgroundCommittedRunReconcileLoopForHomeVisibility()`. Move that call to
the emitter so navigation-driven visibility changes still start and stop the
background committed-run reconcile loop.

### 7. Initial and debug fixture bootstrap

Initialization and the debug home-scroll fixture should emit to the actor after
their synthetic fields are populated. The first live home store publish should
therefore also come from actor result application when the cutover flag is on,
or from the legacy path when the rollback flag is off.

## Validation Plan

- Add Core tests for actor snapshot application:
  - newer `appliedSeq` publishes;
  - equal/lower `appliedSeq` is dropped;
  - newer `appliedSeq` with content equal to the current snapshot does not
    publish;
  - output snapshot maps sections/loading/visibility exactly.
- Add App-target tests around the model bridge where practical:
  - a `runTracker` mutation updates `homeThreadListStore` only after
    `homeProjectionGateway.waitForIdleForTesting()`;
  - `homeThreadListStore.sectionDerivationCount` stays zero for live cutover
    events, proving the legacy store did not derive sections on main.
  - disabling the cutover flag falls back to legacy `apply(_:)` and does not
    leave the home list empty.
  - per-thread committed run-state writes emit one direct actor delta without a
    duplicate full capture.
- Run:
  - `swift test` in `mobile/garyx-mobile`
  - app-target `xcodebuild` Debug build for `GaryxMobile`
  - debug scroll probe where available. If no physical device is attached,
    keep the probe path compiled and report the missing-device limitation
    explicitly.

## Review Questions

- Are all old `refreshHomeThreadListSnapshot()` call sites converted to actor
  event emission?
- Is the actor result apply guarded by strictly increasing `appliedSeq`?
- Does actor result apply also de-dupe unchanged snapshot content?
- Does the rollback flag restore the legacy live path instead of disabling
  updates?
- Does parity remain actor-vs-legacy-checkpoint with mismatch count still
  available?
- Do all three run-state sources still reach the reducer with the existing
  source precedence?
