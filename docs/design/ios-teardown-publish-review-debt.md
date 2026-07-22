# iOS Teardown Publish Review Debt

This file records adjacent pre-existing findings discovered while reviewing
`#TASK-2586`. They are intentionally excluded from the teardown-publication
fix and tracked independently.

## 2026-07-22: #TASK-2586 / #TASK-2587 collision and convergence

`#TASK-2586` and `#TASK-2587` independently started from the same build-158
reproduction (`2206e5287`) before either branch reached `main`. `#TASK-2587`
landed first as `3924fbb16` / merge `20142dd29`, using generation-guarded
`Task { @MainActor }` blocks at individual reveal and presentation-barrier
detach sites. The later combined result intentionally removes those point
deferrals and makes `GaryxObservableStateSettler` the single owner of deferred
observable projection.

The settler records terminal semantic state synchronously, coalesces one
deferred projection flush, and reads the latest semantic value when that flush
executes. This preserves the generation guard's stale-detach guarantee: a
replacement attach or gesture cannot be overwritten by the older queued
detach. The same mechanism also closes the active-barrier-to-barrier-free
`makeUIViewController` counterexample that remained reachable after
`#TASK-2587`.

Historical `#TASK-2587` lifecycle findings and the remaining geometry-change
debt are recorded in
[`ios-representable-lifecycle-publish-debt.md`](ios-representable-lifecycle-publish-debt.md).
The entries below remain scoped as independent fixture debt.

## 2026-07-22: Test fixtures bypass root-occurrence ownership

Tracking: `#TASK-2589`

- Four tests in
  `GaryxProductionRouteIntentIntegrationTests.swift` configure model-owned,
  host-bound reveal stores without a root occurrence and then call the
  occurrence-free gesture API. They deterministically trap at
  `GaryxHorizontalRevealInteraction.swift:211` before exercising their stated
  assertions. The fixtures originated in commits `78e32faeed` and
  `2e1c3929e6`.
- `GaryxRouteStackContainerTests.testProductionDraftRouteNeverConnectsStagedDriverAcrossPromotion`
  mounts production task-tree content without supplying a root surface
  occurrence. It deterministically traps at
  `GaryxMobileTaskTreeSidebarViews.swift:141`. The fixture originated in
  commit `be3c520ccd`; the ownership guard originated in `08126b2dd8`.

The follow-up must repair the fixtures to establish real root-surface and host
occurrence ownership. Production ownership assertions must remain intact.
