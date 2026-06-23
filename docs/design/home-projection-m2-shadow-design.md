# Home Projection M2 Shadow Design

## Scope

M2 keeps the existing `GaryxHomeThreadListStore` as the only UI render source.
The new pipeline computes the same home snapshot in shadow mode and asserts
parity at explicit transaction boundaries. It must be removable by a single
feature flag without changing legacy behavior.

This builds on the M1 reducer in
`mobile/garyx-mobile/Sources/GaryxMobileCore/HomeProjectionReducer.swift`.
M2 does not rewrite the reducer or move transcript lifecycle side effects.

## Components

`HomeProjectionActor` lives in `GaryxMobileCore` so its mailbox, transaction
coalescing, parity bookkeeping, and `snapshotEmitCount` are covered by
SwiftPM tests. It owns a `HomeProjectionState`, applies
`HomeProjectionReducer.reduce`, and returns:

- `HomeSnapshot`
- `appliedSeq`
- latest row `CollectionDifference<String>?`
- actor counters, including `snapshotEmitCount`

The actor is the only writer of `HomeProjectionState`. It has no UI references,
no `@MainActor` isolation, and no side effects.

`HomeProjectionGateway` is a thin `@MainActor` app-target shim. It is the only
bridge from legacy `@Published` didSet sites to the Core actor. It:

- captures legacy home inputs after the old store applies them
- diffs captures into `HomeProjectionEvent` values
- batches events inside explicit transactions
- sends exactly one actor boundary apply per completed transaction
- passes the live legacy snapshot to Core as diagnostics only
- increments `parityMismatchCount` on mismatch

The gateway is controlled by a single development shadow flag:
`GARYX_MOBILE_HOME_PROJECTION_SHADOW=0` disables all dual-emits. When disabled,
the old store path still runs exactly as it does today. This is a development
and test gate, not a TestFlight runtime flag.

## Capture Points

The capture point is centralized in `refreshHomeThreadListSnapshot()` after
`homeThreadListStore.apply(homeThreadListInput)`. This keeps all existing
didSet sites covered without duplicating event construction logic at every
property:

- `threads`
- `selectedThread`
- `isLoadingThreads`
- `runTracker`
- `runStateByThread`
- `navigationState`
- `pinnedThreadIds`
- `recentThreadIds`
- `agents`
- `teams`
- `automations`

The gateway stores the last captured legacy state and emits only deltas:

- display/source changes -> `recentThreadsIngested`
- pin changes -> `pinsChanged`
- selection changes -> `selectedThreadChanged`
- loading changes -> `loadingChanged`
- home visibility changes -> `homeVisibilityChanged`
- `runTracker.busyThreadIds` changes -> `runStateDelta(.runTracker, ...)`
- `runStateByThread[thread].busy` changes ->
  `runStateDelta(.committedRunState, ...)`

`recentThreadsIngested` continues to feed the reducer's
`.recentThreadSummary` source from `thread.runState`.

The app shim reads `runTracker.busyThreadIds` and `runStateByThread` directly
from `GaryxMobileModel` when shadow mode is enabled. Those sources are not
present in `homeThreadListInput`, which intentionally remains the legacy store
input.

## Transactions

Every capture belongs to a transaction.

For ordinary single didSet refreshes, the gateway creates an implicit
transaction, sends its events to the actor, and checks parity for that capture.

For known multi-write sequences, the model opens an explicit transaction at the
function or operation boundary before the first write and ends it after the
final legacy write:

- `refreshThreads(silent:)`: begins at function entry and ends in a function
  `defer`, independent of `silent`. This covers the silent path where
  `isLoadingThreads` never changes but pins/recent/threads/selectedThread may
  still write across awaits.
- `removeArchivedThreadLocally(_:)`: covers pinned/recent/thread removals.
- archive failure rollback: covers restoring pinned/recent/thread state.

The transaction id is monotonic and only used by M2 shadow diagnostics. It is
not a data model id and is not persisted.

## Parity Boundary

Parity never runs per intermediate write in an explicit transaction.

At transaction end Core compares the actor snapshot to a checkpoint legacy store
fed with `HomeProjectionState.legacyCheckpointInput()`. This matches the M1
parity contract: it verifies that the actor's final reducer state renders the
same sections as the existing store when both are given the same resolved
running input.

The checkpoint parity compares:

- actor rendered `HomeSnapshot.sections`
- actor `isLoadingThreads`
- actor `isHomeVisible`
- checkpoint legacy store sections/loading/visibility
- rendered semantic counters derived from the checkpoint snapshot:
  pinned row count, recent row count, total row count, selected row count,
  running row count, and archiveable row count

`appliedSeq` is monotonic actor metadata and is not compared to the legacy
store. Checkpoint store internal counters are recorded for diagnostics, but not
equality-checked, because M2 intentionally lets the live old store publish
intermediate transaction states while the actor emits once at the boundary.

On mismatch, `parityMismatchCount` increments and the latest mismatch record
keeps the transaction id, actor checkpoint, legacy checkpoint, and actor
`appliedSeq`. The UI still renders from the legacy store regardless of parity
status.

The live `homeThreadListStore.snapshot` is not the parity oracle for running
rows in M2. Today's live store derives running rows from `thread.runState`
summary only, while the actor intentionally folds three sources:
`runTracker`, committed transcript run state, and recent thread summary. Those
three-source running semantics are compute-only in M2 and become a live behavior
change only at the later cutover stage. M2 records the live legacy snapshot and
live-vs-actor differences for diagnostics, but `parityMismatchCount == 0`
means checkpoint parity, not live summary-only running parity.

## Run-State Side-Effect Boundary

`applyTranscriptRunState(_:threadId:)` keeps its existing lifecycle side
effects in place:

- provider input ack
- pending direct follow-up advancement
- active assistant segment completion
- title update
- terminal cleanup
- selected thread recovery cancellation
- `runTracker` completion/interruption
- follow-up home refresh

M2 emits only the projection input (`busy` as a committed run-state delta, plus
runTracker busy/idle deltas). No ack/title/terminal/runTracker lifecycle side
effect moves into the actor.

## Snapshot Emission Gate

The Core actor increments `snapshotEmitCount` only when it returns a transaction
boundary snapshot to the gateway. A burst such as running thread open,
`popToHome`, idle pause, and drain inside one transaction must leave
`snapshotEmitCount == 1`.

This counter proves actor coalescing without changing the old store's current
render behavior.

## Validation

Focused validation:

- SwiftPM tests for reducer parity plus Core actor transaction/parity/coalescing
  support
- a replay corpus test that drives the running-thread open -> popToHome ->
  pause -> drain sequence and asserts `snapshotEmitCount == 1`
- an app-target build to prove app wiring and project membership

Final commands:

```bash
cd mobile/garyx-mobile && xcodegen generate
cd mobile/garyx-mobile && swift test
cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build
cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Release build CODE_SIGNING_ALLOWED=NO
```

Release uses `CODE_SIGNING_ALLOWED=NO` only to avoid local provisioning
requirements while still compiling the app target.
