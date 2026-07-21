# iOS Presentation Lease Owner Loss: Terminal Settlement Design

Status: approved for implementation (design review is not part of the flow).
Repro evidence: #TASK-2565, commit `e26a712fe` on branch `garyx/863cc384`.

## Problem

The presentation lease system assumes SwiftUI always delivers a terminal
callback (`onDismiss`, binding write-back, or dismissal completion) for every
presentation it tears down. Measured behavior on iOS 26.5 disproves that
assumption: when the presenting row of a `garyxFullScreenCover(item:)` is
removed from a `LazyVStack` while presented, the system removes the cover UI
(UIKit ends with no presented view controller; both the row and the presented
content receive `onDisappear`), but SwiftUI never writes the item binding back
to `nil` and never calls `onDismiss`. The row's `@StateObject` session storage
survives with no lifecycle callback left to drive it.

The lease therefore stays `.presented` forever, `GaryxPresentationLeaseTree
.hasBarrier` stays `true`, every navigation preparation queues behind
`waitForPresentationBarrier`, and both edge-gesture recognizers stay disabled:
a global navigation freeze that only an app kill clears.

Root cause is in the lease abstraction, not in any one call site: it treats
"terminal callbacks always arrive" as an axiom, so a lease's liveness is
anchored to nothing real.

## Design

### 1. Core: owner loss is a first-class terminal event

`GaryxPresentationLeaseTree` (GaryxMobileCore/GaryxPresentationTransaction
.swift) gains an explicit terminal transition, e.g.
`ownerPresentationEnded(_ token:)`:

- Semantics: the presentation's host is gone and no further callbacks can
  arrive for this lease.
- Behavior: equivalent to dismissing + dismissal completed in one transition;
  a result-bearing lease settles its result as `explicitNoResult`.
- The record carries an auditable terminal cause (normal dismissal vs owner
  loss vs presentation failure) so diagnostics can distinguish recovery from
  ordinary flow. Keep the state machine explicit, pure, and SwiftPM-tested.

### 2. Session layer: platform truth is the liveness oracle

All garyx presentation modifiers share one implementation point for presented
content (`GaryxPresentationLeaseModifierSupport.presented(_:)`). Extend it:

- Presented content gets a paired `.onDisappear` →
  `session.presentedContentDisappeared()`.
- That schedules a deferred settlement check after main-actor yields (the
  same deferral idiom as `scheduleNoResultIfPending`; never wall-clock
  timers). The check settles the lease through the new owner-loss transition
  only when all three hold:
  1. the lease record still exists and is not released;
  2. no real terminal callback won the race in the meantime;
  3. UIKit ground truth confirms the presentation no longer exists — the
     route container's presented-controller chain does not contain this
     presentation. The DEBUG-only test probe added by the repro commit is
     promoted into a narrow, reviewed query surface on
     `GaryxRouteStackContainer` for this purpose.
- Normal dismissal is unaffected: `onDismiss` / binding write-back arrive
  first on the main actor, release the lease, and the deferred check no-ops.
  Nested presentations are protected by condition 3: while an ancestor cover
  is still genuinely presented, ground truth reports it and the check no-ops.
- Settlement runs through the existing coordinator methods so barrier
  synchronization and `presentationBarrierDidChange` admission wakeups fire
  exactly like every other terminal path. No read-route or admission-side
  repair: admission keeps waiting on the barrier; the barrier now provably
  falls.

### 3. Explicitly not in scope

- No timer-based cleanup (repo contract: lifecycle + ground truth, not
  wall-clock).
- No admission/drain bypass of the barrier; recovery is a lease terminal
  transition, nothing else.
- No repo-wide migration of row-local presenters to stable anchors. Data-row
  presenters violating the existing anchor-stability rule are adjacent
  existing debt: record them in `docs/design/ios-presentation-anchor-debt.md`
  with call sites, file as separate work. This fix makes the lease abstraction
  immune to owner loss for every presenter kind, which is the actual defect.

## Verification

- Invert the repro suites from commit `e26a712fe` to assert healthy
  semantics over the same path (remove presenter row while presented):
  - Core: an owner-loss transition releases the record, clears the barrier,
    and previously queued preparations become admissible.
  - Real-component (iOS 26.5): after row removal, the lease releases within
    bounded main-actor turns, `hasPresentationBarrier` falls, a queued
    navigation preparation is admitted, `open()` pushes, and both edge
    recognizers re-enable. Fix-absent baseline is the original repro output.
- Regression: normal present → dismiss keeps identical record history
  (owner-loss branch must not fire); nested presentation on top of a cover
  does not settle the ancestor lease.
- Existing lease/navigation suites stay green.

## Impact

- Core lease state machine: new terminal transition + terminal-cause audit
  field (additive; existing transitions unchanged).
- `GaryxPresentationLeaseViews.swift`: session gains the deferred owner-loss
  settlement; all modifiers inherit it through `presented(_:)`.
- `GaryxRouteStackContainer`: narrow presented-controller liveness query.
- User-visible: scenarios that previously froze the app permanently now
  recover navigation within a frame or two after the system tears the
  presentation down.
