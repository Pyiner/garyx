# iOS Representable Lifecycle Publish Debt

Status: debt register discovered while implementing `#TASK-2587`. The attach
item is resolved by the converged `#TASK-2586` implementation; the geometry
item remains independent debt.

## #TASK-2586 / #TASK-2587 convergence

Both tasks began from the same build-158 reproduction before `#TASK-2587`
landed on `main` as `3924fbb16` / merge `20142dd29`. The combined implementation
keeps the test and fixture improvements from `#TASK-2587`, but replaces its
per-detach generation counters and `Task { @MainActor }` point fixes with the
single `GaryxObservableStateSettler` design from `#TASK-2586`.

The unified settler separates synchronous ownership and semantic settlement
from observable projection. A deferred flush always reads the latest semantic
value, so detach followed by rebind is generation-guard equivalent without a
second deferral mechanism. Production representable make, update, and
dismantle paths all select graph-safe projection timing, including the attach
counterexample below. See
[`ios-teardown-publish-review-debt.md`](ios-teardown-publish-review-debt.md) for
the collision record and the independently tracked fixture findings.

## Presentation barrier attach after an immediate remount

Resolution: closed by the converged `#TASK-2586` implementation and its real
hosting-controller lifecycle replacement oracle. The text below preserves the
original finding that required the follow-up.

`GaryxProductionRouteStack.makeUIViewController` calls
`GaryxProductionRouteStore.attach`, which reaches
`GaryxPresentationLeaseCoordinator.attach` and its synchronous
`synchronizeBarrier()` call at
`mobile/garyx-mobile/App/GaryxMobile/GaryxPresentationLeaseViews.swift:19`.
The ordinary initial mount is a no-op because both the store and fresh
container have no active barrier.

There is a narrower remount window after `#TASK-2587`: if SwiftUI dismantles an
old container with an active barrier and mounts a replacement container for the
same model before the deferred detach settlement runs, the store still
observably reports `hasPresentationBarrier == true` while the fresh container
reports `false`. The attach-side `synchronizeBarrier()` can therefore call
`GaryxProductionRouteStore.presentationBarrierStateChanged(false)` and publish
from `makeUIViewController`, another SwiftUI graph-update context.

The original `#TASK-2587` slice did not change attach behavior because its
approved design scoped barrier deferral to
`GaryxPresentationLeaseCoordinator.detach`. The converged implementation now
propagates graph-safe settlement through production attach/make as well.

### Follow-up acceptance criteria

- Make attach/remount barrier reconciliation publish-free throughout every
  representable make/update callback, without a caller-supplied context flag or
  wall-clock delay.
- Preserve immediate imperative coordinator/container ownership and ensure a
  stale detach settlement cannot clear a replacement container's active
  barrier.
- Add an XCTest that dismantles an active-barrier container and mounts a
  replacement for the same store in one SwiftUI update, proving zero
  synchronous publication and eventual correct barrier state.
- Keep normal lease acquisition, dismissal, and deferred owner-loss settlement
  synchronous in their existing legal callback contexts.

## Reveal geometry reconciliation from SwiftUI lifecycle callbacks

The `garyx-review` adversarial pass for `#TASK-2587` (`#TASK-2593`) identified
another adjacent path for separate evaluation. An extent change in
`GaryxHorizontalRevealInteractionStore.configure` synchronously calls
`forceTerminal` and `publish` at
`mobile/garyx-mobile/App/GaryxMobile/GaryxHorizontalRevealInteraction.swift:123`.
Production callers include the drawer width callback at
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileViews.swift:588`, the task-tree
panel width callback at
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileTaskTreeSidebarViews.swift:99`,
and the row-local reveal width callback at
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileListComponents.swift:306`.

These are SwiftUI `onChange` lifecycle callbacks rather than the representable
make/update/dismantle callbacks covered by the approved `#TASK-2587` design.
The review therefore did not treat this path as a blocker or expand the fix to
cover it. Its publication safety still needs a deterministic real-component
test instead of an assumption based on callback naming.

### Follow-up acceptance criteria

- Drive a real drawer, task-tree, or row reveal through an extent change in an
  XCTest and prove whether publication occurs inside an active SwiftUI graph
  update window.
- If the path is unsafe, separate immediate geometry/driver invalidation from
  observable settlement without caller flags or wall-clock timers.
- Preserve synchronous gesture and display-link publication and the existing
  geometry-change canonical endpoint semantics.
