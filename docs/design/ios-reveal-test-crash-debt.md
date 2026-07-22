# Debt: reveal-interaction tests crash on main (pre-existing)

Registered 2026-07-22 while validating the workspace-mode fix
(`bbebf89ef`); independent of that change and excluded from its review
scope per the scope-discipline rule.

## Symptom

Four tests in `GaryxMobileTests/GaryxProductionRouteIntentIntegrationTests`
crash deterministically on an untouched `origin/main` checkout
(`ba473f11c`, iPhone 17 Pro simulator, iOS 26):

- `testSceneInterruptionTerminatesEveryGlobalRevealInteraction`
- `testPresentationBarrierAcquisitionTerminatesEveryGlobalRevealInteraction`
- `testRouteAndGatewayInvalidationCannotRetainRevealOwnership`
- `testSceneInterruptionStressLeavesBothLongLivedStoresIdle`

Crash point (identical for all four):

```
GaryxHorizontalRevealInteraction.swift:192:
Fatal error: host-bound reveal gesture is missing its occurrence identity
```

Verified twice on 2026-07-22: the same 13-pass/4-crash set reproduces on
the main checkout and on a feature worktree, so this is not caused by any
in-flight change.

## Repro

```
cd mobile/garyx-mobile
xcodebuild test -project GaryxMobile.xcodeproj -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,id=<iPhone 17 Pro>' \
  -only-testing:GaryxMobileTests/GaryxProductionRouteIntentIntegrationTests
```

Deterministic, not flaky.

## Next step (separate project)

Triage whether this is a test-environment change (simulator/iOS 26
behavior shift around scene interruption) or a real product defect in the
reveal gesture's occurrence-identity wiring. Follow the bug workflow:
reproduce → root-cause → fix; do not fold it into unrelated changes.
