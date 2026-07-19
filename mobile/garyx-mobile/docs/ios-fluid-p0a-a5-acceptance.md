# iOS Fluid P0-A A5 Acceptance Record

Date: 2026-07-19

This record covers the v41 A5 slice: the home drawer, conversation task tree,
and row actions now use the shared horizontal reveal physics; the complete
gesture-arbitration table is hosted by the route container; and the A5 legacy
implementations have been removed. The deployment target remains iOS 26.

## Environment

- Xcode 26.6 (17F113), Apple Swift 6.3.3
- iPhone 17 Pro simulator, iOS 26.5
- LTR and forced RTL production-host fixtures
- A0 reference traces from `/tmp/GesturePhysicsSpike`, when present

## Shared reveal physics

`GaryxHorizontalRevealState` is the pure Core owner for all three surfaces.
Its finite states are `idle`, `dragging`, and `settling(target)`. Positive
values always mean logically more open; the app adapter is the only layer that
maps logical leading/trailing motion to physical LTR or RTL coordinates.

- Release first projects the landing with the canonical `ProjectionPolicy`,
  applies the halfway decision, and passes the actual release velocity into
  `GaryxGestureSettleDriver`. Drawer and task tree use the long-travel policy;
  row actions use the 0.20-second short-travel policy.
- Both endpoints use the signed Core rubber-band curve instead of hard clamps.
- A new touch during settle interrupts the analytic trajectory, adopts the
  exact last-drawn reveal and instantaneous velocity, and re-enters dragging.
  UIKit cancellation is an explicit reducer event that resumes the endpoint
  which owned the surface when the interrupted touch began.
- Both projection variants are sampled against the analytic spring at every
  120 Hz frame through terminal. Exact mid-settle takeover and target reversal
  are separately asserted with an injected clock and frame source.

The drawer owns the home node's physical leading edge. The task tree owns the
conversation node's physical trailing edge and remains the only task surface;
while it is open, route pop is ineligible and a leading-edge close cannot pop
the conversation. Both publish a single model-owned reveal value rather than
maintaining view-local drag gates or duplicate state machines.

`GaryxSwipeActionRow` now uses a UIKit pan stream so cancellation is explicit,
vertical intent can fail before row ownership, and the exact velocity reaches
the shared settle driver. A non-edge row touch belongs to the row; a physical
edge touch waits for the public navigation recognizer.

## Arbitration matrix acceptance

The route container installs public leading and trailing pan recognizers on the
same `UIWindow` as production descendants. Its delegate captures a physical
touch-down snapshot before recognition and uses that frozen snapshot for zone
ownership. Moving from 5 pt to 25 pt cannot lose navigation ownership, and
starting at 25 pt then moving into the edge cannot acquire it.

| Competition | Accepted owner and evidence |
| --- | --- |
| Push-page logical leading edge | Route pop through the public leading recognizer |
| Home logical leading edge | Drawer through that same recognizer, dispatched by node |
| Conversation logical trailing edge | Task tree through the public trailing recognizer |
| Horizontal scroll in content | Descendant scroll; both edge recognizers fail outside their frozen zone |
| Horizontal scroll beginning in an edge zone | Navigation; `shouldBeRequiredToFailBy` makes descendant pans wait |
| Focused-composer keyboard drag | Keyboard drag outside an edge; edge recognizer at an edge |
| Row-local swipe | Row outside an edge; navigation at an edge |
| Presented modal or active presentation lease | Both public edge recognizers disabled; modal subtree owns its touches |
| Open task tree versus route pop | Task-tree close; pop remains ineligible |
| Vertical intent at any start zone | Descendant vertical scroll through the Core axis lock |

App-host tests exercise the real window failure graph, real descendant
`UIScrollView` pans, modal ownership, lease enable/disable, node-specific
owners, settle interruption, physical LTR and RTL coordinates, both frozen-zone
cases, and vertical axis rejection. Production XCUITests repeat physical
leading/trailing gestures in LTR and RTL and cover drawer, task-tree, route,
horizontal-scroll, keyboard, and row surfaces.

## Slow-motion and A0 comparison

The retained A0 system traces record:

| A0 case | Release | Completion | Observed settle | Outcome |
| --- | ---: | ---: | ---: | --- |
| Visible-edge cancel | 4,591 ms | 4,961 ms | about 370 ms | cancel at 13.02% |
| Visible-edge finish | 4,205 ms | 4,584 ms | about 379 ms | commit at 83.91% |

Both system traces report a 0.35-second transition duration. The shared app
curve is `response=0.22`, `dampingRatio=0.88`, with an analytic settling
duration of 404.69 ms. It stays inside the frozen 300-440 ms acceptance band
and preserves release velocity instead of beginning a fixed zero-velocity
spring.

The production slow-motion fixture advances spatial geometry from 0 through
18/40 in 50 ms steps. Every step asserts the outgoing wrapper, 30% incoming
parallax, scrim, shadow, and physical direction against the frozen geometry.
The run attached the inspected middle frame as
`/tmp/task-2446-a5-slow-motion.png`: the destination remains visible along the
leading side, the source is translated one-to-one, and no gap or tear is
visible. The corresponding settle attachment is
`/tmp/task-2446-a5-performance.txt`.

The A5 slow-motion walkthrough used this checklist:

- Drawer: physical leading edge mirrors in RTL; drawer and content consume the
  same reveal; fast short flick opens; slow middle release can cancel; endpoint
  overshoot is damped; a cancel settle can be touched again and reversed.
- Task tree: physical trailing edge mirrors in RTL; content and task panel
  share one reveal; fast/slow decisions, endpoint damping, and settle takeover
  match the drawer; open state keeps pop ineligible.
- Row actions: logical trailing direction mirrors in RTL; short-travel
  projection receives measured velocity; endpoint damping and the action track
  stay continuous; a second touch can take over the settling row; vertical and
  edge-origin touches never flash row actions.
- Shared frame path: long- and short-travel tracks match every analytic 120 Hz
  sample; route settle reports 39 frames, zero backwards frames, zero route
  subtree body recomputations, and a maximum 18.16 ms frame gap.

## Frozen performance gates

| Gate | A5 result | Frozen limit | Result |
| --- | ---: | ---: | --- |
| Mounted hosts, 20-layer stack | peak 2 | at most 4 | pass |
| 20-layer RSS delta, worst of three cold pairs | 3,632 KB (3.55 MB) | at most 100 MB | pass |
| Actual settle route-subtree body recomputations | 0 over 39 frames | 0/frame | pass |
| Actual settle maximum frame gap | 18.16 ms | at most 25 ms fixture gate | pass |
| Actual settle backwards frames | 0 | 0 | pass |
| Settle calibration | 404.69 ms | 300-440 ms | pass |
| Weak host/root deallocation | all released | all released | pass |
| 500-iteration churn | bounded, zero terminal residue | steady state | pass |
| Evictable route state after churn | at most 32 entries / 2 MB | 32 entries / 2 MB | pass |

The three cold RSS pairs, sampled two seconds after launch, were:

| Sample | Depth 0 | Depth 20 | Delta |
| --- | ---: | ---: | ---: |
| 1 | 254,496 KB | 258,128 KB | 3,632 KB |
| 2 | 255,264 KB | 258,064 KB | 2,800 KB |
| 3 | 255,392 KB | 258,160 KB | 2,768 KB |

In the same-process 1,000-iteration run, RSS was 255,296 KB before churn,
311,680 KB after the first 500 iterations, and 311,664 KB after the second 500.
The second batch changed by -16 KB. Eight steady-state samples stayed inside
an 80 KB band (311,648-311,728 KB). The A4a maximum frame gap was 18.28 ms;
the A5 recheck is 18.16 ms, so the hitch gate did not regress.

The retained performance attachment reports:

```text
performance=pass;settleFrames=39;maxGapMs=18.16;backwards=0;bodyDelta=0;peakMounted=2
```

## A5 deletion audit

The old drawer opening/closing gestures and return branches, app-layer sidebar
axis helper, task-tree local drag machine, row fixed-spring settle path, and
cancel-liveness workaround are absent. The audit deliberately reconstructs
historical identifiers in the shell so the acceptance document does not
reintroduce them:

```sh
cd mobile/garyx-mobile
legacy_patterns=(
  'openingSidebar''Gesture'
  'closingSidebar''Gesture'
  'decideSidebar''Axis'
  'canStartOpening''Drag'
)
for pattern in "${legacy_patterns[@]}"; do
  ! rg -n "$pattern" .
done

! rg -n \
  'GaryxMobileMotion\.row''Swipe|predictedEnd''Translation|withAnimation\([^)]*row''Swipe|interactive''Spring\(' \
  App/GaryxMobile/GaryxMobileListComponents.swift
! rg -n \
  'sidebarDragOffset|sidebarDragAxis|sidebarDragLive|taskTreeDragOffset|taskTreeDragAxis|taskTreeDragLive|resetSidebarDrag|resetDrag\(\)' \
  App/GaryxMobile/GaryxMobileViews.swift \
  App/GaryxMobile/GaryxMobileTaskTreeSidebarViews.swift
```

The complete command exits zero with no matches.

## Four-layer regression

The generated Xcode project has no post-generation drift. The complete A5 gate
is:

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
  CODE_SIGNING_ALLOWED=NO
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile -configuration Debug \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile -configuration Release \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO
```

Final results:

- Core and persistence: 1,406 of 1,406 SwiftPM tests passed with zero
  failures, including the arbitration decision tables and all retained real
  kill/relaunch durability suites.
- Instrumented app host: 142 of 142 tests passed with zero failures, including
  the real-window failure graph, exact settle interruption, every-120-Hz
  long/short trajectory comparison, lifecycle, memory, and accessibility
  policy coverage.
- Production/fake-host UI: 53 of 53 XCUITests passed with zero failures. This
  is the entire UI target, not an A5-only selection, and includes drawer,
  task-tree, row, route, keyboard, horizontal-scroll, LTR/RTL, frozen-zone,
  slow-motion, churn, durability, image, reorder, and workspace coverage.
- Generic simulator Debug and Release builds both completed successfully with
  code signing disabled.
