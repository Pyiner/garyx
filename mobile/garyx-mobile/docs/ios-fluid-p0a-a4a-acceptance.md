# iOS Fluid P0-A A4a Acceptance Record

Date: 2026-07-18

This record covers only the A4a renderer and its instrumented fake routes. The
fixture is compiled only in `DEBUG` and requires
`GARYX_MOBILE_FLUID_FAKE_ROUTES=1`; the existing `NavigationStack` remains the
default app root and no production route is connected to the container.

## Environment

- Xcode 26.6 (17F113)
- iPhone 17 Pro simulator, iOS 26.5 (23F77)
- Debug fake hosts retain a synthetic 64 KB payload each
- Release simulator build with the fake fixture compiled out: passed

## Functional acceptance

The focused `GaryxMobileFluidRoutes` run passed 10 of 10 XCUITests with no
skips or expected failures. It covers:

- finish, slow cancel, cancel-settle regrab, and the measured 18.24% fast-flick
  commit case;
- a 20-layer synthetic stack, first-layer pop to home, and the home leading-edge
  action;
- physical LTR and RTL coordinates;
- 5 pt to 25 pt edge ownership and a 25 pt start moving back into the edge;
- frame-by-frame slow-motion comparison with the frozen spatial geometry; and
- 500 push/pop churn iterations ending at the original path with zero terminal
  residue.

The app-target container suite passed 89 of 89 tests in total. Its A4a cases
assert exactly-one `screenChanged` at committed-and-visible terminal, no
announcement after cancelled background restoration, no superseded side
effects, no staged-host lifecycle writes, wrapper-only writes for all three
visual policies, explicit z-order, geometry rederivation, lease barriers and
hard snap, and weak deallocation of every hosted root.

The A3 Core lease matrix remains green and covers nested leases, both
programmatic and interactive dismissal callbacks, result/dismissal in both
orders, explicit no-result and presentation failure, forced subtree dismissal,
same-frame intent contention, exactly-once release, and hard-snap blocking.

The adversarial-review rework added five permanent app-host regressions. Before
the fix, the selected tests reproduced an inactive immediate settle as
`(committed, visible)` with one premature announcement, a deterministic
`mounted -> inactive` lifecycle crash after deferred-terminal supersession,
replay of a superseded deferred announcement, and a retained middle host after
multi-pop. The same focused container suite passes 18 of 18 after the fix;
scene visibility now has one source of truth, a new transaction permanently
cancels prior deferred effects, host deactivation is idempotent for inactive
scene delivery, and every permanently removed identity is unmounted at
terminal.

## Frozen performance gates

| Gate | Measured result | Limit | Result |
| --- | ---: | ---: | --- |
| Mounted hosts, 20-layer stack | peak 2 | at most 4 | pass |
| 20-layer RSS delta, worst of three cold pairs | 2,896 KB (2.83 MB) | at most 100 MB | pass |
| Actual settle route-subtree body recomputations | 0 over 39 frames | 0/frame | pass |
| Synthetic 120-frame drag body recomputations | 0 | 0/frame | pass |
| Actual settle maximum frame gap | 18.28 ms | at most 25 ms fixture gate | pass |
| Actual settle backwards frames | 0 | 0 | pass |
| Settle calibration | 404.69 ms | 300-440 ms | pass |
| Weak host/root deallocation | all released | all released | pass |
| 500-iteration push/pop churn | bounded, zero residue | steady state | pass |
| Evictable route state after churn | at most 32 entries / 2 MB | 32 entries / 2 MB | pass |

The three cold RSS pairs, sampled two seconds after launch, were:

| Sample | Depth 0 | Depth 20 | Delta |
| --- | ---: | ---: | ---: |
| 1 | 252,992 KB | 255,584 KB | 2,592 KB |
| 2 | 252,768 KB | 255,664 KB | 2,896 KB |
| 3 | 252,768 KB | 255,568 KB | 2,800 KB |

An additional same-process 1,000-iteration push/pop run separates allocator
warm-up from steady-state growth. RSS was 252,688 KB before churn, 308,752 KB
after the first 500 iterations, and 308,816 KB after the second 500 iterations:
the second batch added 64 KB. Samples from seconds 3 through 10 remained within
a 160 KB band (308,752-308,912 KB).

The retained XCUITest performance attachment reports:

```text
performance=pass;settleFrames=39;maxGapMs=18.28;backwards=0;bodyDelta=0;peakMounted=2
```

## Reproduction

```sh
cd mobile/garyx-mobile
xcodegen generate
swift test
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' \
  CODE_SIGNING_ALLOWED=NO
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobileFluidRoutes \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' \
  -only-testing:GaryxMobileUITests/FluidRouteStackInteractionTests \
  CODE_SIGNING_ALLOWED=NO
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile -configuration Release \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO
```

Observed totals for this acceptance run were 1,315 SwiftPM tests, 89 app-hosted
unit tests, and 10 focused fake-route XCUITests, all passing with zero failures.
