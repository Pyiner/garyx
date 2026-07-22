# iOS Representable Lifecycle Publish Debt

Status: debt register discovered while implementing `#TASK-2587`; explicitly
outside the approved dismantle-settlement slice.

## Presentation barrier attach after an immediate remount

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

This task does not change attach behavior because the approved design scopes
barrier deferral to `GaryxPresentationLeaseCoordinator.detach` and forbids an
adjacent lifecycle sweep.

## Follow-up acceptance criteria

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
