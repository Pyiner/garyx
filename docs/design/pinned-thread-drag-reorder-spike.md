# Pinned-thread reorder iOS architecture gate

Date: 2026-07-15

Design contract: `docs/design/pinned-thread-drag-reorder.md` v8, §5.1

Result: **PASS — native List + observation-only adapter**

## Fixed runtime and availability

- Xcode 26.6 (17F113), iPhone 17 Pro simulator, iOS 26.5.
- Deployment target remains iOS 17.0.
- Reorder availability is a runtime check for iOS 26.5. iOS 17–25 keep the
  existing non-reorder Home list; there is no EditMode compatibility path.
- The pressure fixture is synthetic: 50 threads, six pinned, one visible
  running row.

## Gate evidence

1. `UICollectionView.hasActiveDrag` starts the freeze. Root collection-view
   drag recognizers distinguish a normally ended in-bounds drop from an
   outside/cancelled end, while `onMove` supplies preview-only order. The UI
   suite proves one accepted-drop commit and a cancelled drag with baseline
   restore and zero commit.
2. The adapter captures the List collection view's `delegate`,
   `dragDelegate`, and `dropDelegate` identities and asserts they remain
   unchanged. It adds observation targets only to the collection view's own
   recognizers; it never substitutes a delegate. The verified recognizers
   include `_UIDragLiftGestureRecognizer`.
3. Menu arbitration lands on the owner's preferred tier. A scoped
   `UIGestureRecognizerRepresentable` arms the stationary row-menu long press
   simultaneously with native reorder. A stationary 0.55-second hold presents
   the existing menu; detected movement dismisses that floating panel and the
   same touch stream completes native reorder. The movement-suppresses-menu
   fallback and menu relocation were not needed.
4. The debug harness injects a reversed canonical snapshot as soon as lift
   begins. The rendered pinned order remains equal to the pre-lift order until
   session completion (`midlift_frozen=1`).
5. The pure flat-to-pinned index translator clamps above and below the pinned
   segment. Core tests cover both extremes; the runtime UI test drags through
   the lower boundary and observes a native settle at the pinned tail.
6. The existing pin transition test observes exactly one accessibility
   identity for the thread while its row moves by more than 40 points between
   Pinned and Recent.
7. The completed-drop path advances a `.sensoryFeedback(.selection)` trigger.

## Quantified hitch gate

The baseline was captured before the lifecycle adapter was wired, using the
fixed configuration above:

| Metric | Baseline | Absolute ceiling | Relative ceiling |
| --- | ---: | ---: | ---: |
| Hitch time ratio | 0.041255 | 0.075 | `baseline × 1.5 + 0.005` = 0.066883 |
| Max frame interval | 0.065399 s | 0.090 s | `baseline × 1.25 + 0.008` = 0.089749 s |
| Worst frame delta | 0.048732 s | 0.075 s | `baseline × 1.35 + 0.008` = 0.073788 s |

The wired adapter run produced:

| Metric | Result |
| --- | ---: |
| Hitch time ratio | 0.049184 |
| Max frame interval | 0.074745 s |
| Worst frame delta | 0.058078 s |

All six absolute/relative assertions passed. The same test also ran four
measured scroll round trips with `XCTHitchMetric` on iOS 26.5; together with
the explicit four-swipe probe this keeps the simulator test below the UI-test
runner's one-minute event-loop instability while retaining a mechanical
pass/fail gate.

## Verification commands

```sh
swift test --filter PinnedListReorderTests
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobileScrollPerformance \
  -destination 'platform=iOS Simulator,id=<iOS-26.5-device>' \
  -only-testing:GaryxMobileUITests/PinnedThreadReorderArchitectureTests
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobileScrollPerformance \
  -destination 'platform=iOS Simulator,id=<iOS-26.5-device>' \
  -only-testing:GaryxMobileUITests/HomeListScrollPerformanceTests/testHomeListScrollPerformanceWithVisibleRunningRows
```
