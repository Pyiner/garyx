# iOS Teardown Publish Review Debt

This file records adjacent pre-existing findings discovered while reviewing
`#TASK-2586`. They are intentionally excluded from the teardown-publication
fix and tracked independently.

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
