# iOS Fluid P0-A A4c Acceptance Record

Date: 2026-07-19

This record covers the v41 A4c content-route, presentation, and accessibility
slice. The UIKit occurrence container now owns every root content page. Native
navigation embedded inside modal forms, text selection, gateway setup, avatar
generation, and tool trace remains intentionally local to those presentations.

## Environment

- Xcode 26.6 (17F113), iOS Simulator SDK 26.5
- Apple Swift 6.3.3
- iPhone 17 Pro simulator, iOS 26.5

## Content routes and intent admission

- Home remains the thread list root. Conversation, every panel, every settings
  tab, and workspace, bot, and automation-thread drilldowns render directly
  from a typed `GaryxRouteEntry` occurrence.
- Settings details install their overview predecessor in one atomic route
  plan. Relative drilldowns append to the occurrence that launched them, so a
  pop returns to that page; absolute links replace the whole chain and their
  first pop reaches home.
- `GaryxMobileRoutePlan` is the Core-owned information-architecture table for
  all settings tabs, all panels, and all typed detail links. The app resolver
  supplies read-only payloads but cannot partially mutate the path.
- Every link begins a Core navigation preparation, resolves without route
  writes, and completes through one of the six typed outcomes. Dependency and
  scope epochs are revalidated at the actual admission boundary. A gesture,
  modal, newer coalesced link, or authentication boundary therefore cannot
  expose an intermediate panel or stale relative predecessor.
- Content activation is delayed until the terminal occurrence is committed and
  visible. Existing conversation occurrence semantics remain intact, including
  panel-to-conversation-to-panel and repeated conversation occurrences.

## Presentation leases and operation context

- Every route-host system presentation uses the shared lease adapters:
  sheets, full-screen covers, adaptive popovers and confirmation dialogs,
  alerts, file importers, photo pickers, camera, preview, and share hosts.
  Acquisition occurs synchronously before the framework observes a true
  presentation binding.
- Presented content receives its parent token. Parent dismissal releases the
  complete descendant tree exactly once; a released session can then acquire a
  fresh token, and released audit records are reclaimed before the next
  presentation.
- Programmatic and interactive dismissal converge on the same Core release
  event. Result-bearing file, photo, and camera leases join result disposition
  with dismissal in either order. Cancellation and failed presentation produce
  an explicit no-result terminal.
- Ordinary navigation waits behind the nonempty token tree without changing
  the path. The final release admits the complete route plan and performs one
  whole-chain hard snap.
- Composer presentation acceptance freezes a Core operation capability, its
  gateway request token, and its gateway client. Staging and upload therefore
  settle against the originating payload entry and originating gateway even if
  another route or gateway becomes active before the picker returns.
- The A4c swap-planner fixture creates a retryable owner through the production
  SQLite/staging coordinator, then hard-snaps and pops every non-conversation
  route chain before planning the replacement from that same durable snapshot.
  The final transaction proves the staged file, quota, manifest, feedback, and
  operation ownership move atomically to one successor.

## Accessibility acceptance

- Both transition hosts are removed from the accessibility tree while staged;
  the destination also has hit testing disabled. Only the canonical active host
  is interactive after terminal cleanup.
- A committed-visible terminal posts one screen-change notification with the
  committed hosting view after its wrapper is visible, enabled, and exposed to
  accessibility. Inactive completion defers the same one-shot notification;
  cancelled and superseded terminals emit none.
- Composer focus uses the shared Core gate and requires durable input readiness,
  visible live ownership, and an active scene. The accessibility escape action
  and its dismissal closure use the same canonical-top, active-lifecycle, and
  modal-barrier gate.
- The instrumented matrix covers VoiceOver, Switch Control, Full Keyboard
  Access, and Reduce Motion with Cross-Fade, including staged-host state,
  visual policy, terminal timing, argument identity, and exactly-once count.

## Same-slice removal audit

The A4c section 8a audit returns no matches across source, tests, and docs for
the former root content path owner, its three path synchronization surfaces,
the legacy leading-edge enum and values, the panel-private return history and
dispatch functions, or sidebar disappearance route writes. Remaining native
navigation stacks are the explicitly preserved modal, form, setup, text
selection, and tool-trace owners and none has a root content path binding.

## Validation

The acceptance gate runs:

```sh
cd mobile/garyx-mobile
xcodegen generate
swift test
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' \
  -only-testing:GaryxMobileTests \
  CODE_SIGNING_ALLOWED=NO
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobileFluidRoutes \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' \
  -only-testing:GaryxMobileUITests/FluidRouteStackInteractionTests \
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

- `xcodegen generate` completed successfully and the generated project includes
  the new lease host and production intent-integration suite.
- SwiftPM completed 1,383 tests with zero failures in 220.351 seconds,
  including all 17 real-process durability suites.
- The app-hosted unit and integration target completed 130 tests with zero
  failures in 6.131 seconds. The focused canonical-route gate completed all 13
  cases, including the production SQLite swap-planner fixture.
- The complete fluid-route UI scheme completed 42 tests with zero failures in
  621.799 seconds. This covers every route family, gesture regrab, modal lease
  barrier, accessibility matrix, and the explicit performance probe.
- Both generic simulator Debug and Release builds completed successfully with
  code signing disabled.

The route performance scheme remains the A4a non-regression gate for mounted
host count, transition body recomputation, frame progression, LRU bounds, and
steady-state churn. Its XCTest CPU-time and instruction-count metrics, explicit
frame probe, and all structural counters remained within their acceptance
bounds.
