# iOS Fluid P0-A A4b Acceptance Record

Date: 2026-07-19

This record covers the v41 A4b conversation/composer slice. Production home
and conversation navigation now use the A4a route container, the composer is
owned by the canonical route occurrence, and the A4d-1 concrete durability
domain is connected to the user-facing input and attachment path.

## Environment

- Xcode 26.6 (17F113), iOS Simulator SDK 26.5
- Apple Swift 6.3.3
- iPhone 17 Pro simulator, iOS 26.5

## Production route wiring

- `GaryxProductionRouteStore` is the sole owner of the production
  home/conversation route path and preserves occurrence identity separately
  from the conversation or draft key. Model selection is a projection of the
  canonical stack top; it no longer drives a second conversation path.
- `GaryxProductionRouteStack` renders home and conversation values through
  `GaryxRouteStackContainer`. Every hosted route receives a route-local
  `GaryxRouteContext`, `GaryxConversationLiveStore`, focus coordinator, and
  composer occurrence.
- A committed pop releases the source composer, resigns focus, updates the
  canonical path, and transfers activation only after input finalization is
  durable. A cancelled pop restores the same route occurrence, adapter, live
  store, focus, and payload without a second path write.
- The container keeps its source view enabled so UIKit first-responder
  ownership survives cancel. A transparent sibling interaction shield freezes
  source controls during the transition without revoking the keyboard; the
  leading-edge recognizer remains the gesture owner.
- The settle renderer clamps the analytic spring to endpoint-bounded monotonic
  progress. Finish, cancel, and cancel-settle regrab retain the frozen A4a
  spatial geometry and system timing oracle with zero backwards frames.

## Panel transition strategy

Home and conversation are fully route-container owned in this slice. Panels
remain reachable through one contained compatibility `NavigationStack` below
their route destination until A4c migrates the remaining routes. There is no
second root conversation writer, no dual conversation renderer, and no
feature loss at this merge point. A4c owns deletion of that one panel
compatibility host and its old path store, as required by the design split.

## Composer input and activation

- `GaryxComposerOrderedTextView` is the UIKit source of truth for focus and
  ordered input. Every accepted edit carries the route key, session, epoch,
  generation, reservation, and strictly increasing input sequence.
- Closing runs the required MainActor critical section: revoke live input,
  call `unmarkText()` first, snapshot the resulting text and exact final
  sequence, then register any still-pending asynchronous producer.
- The dictation registry admits a producer before recognition and emits one
  terminal event for either result or failure. Late callbacks after terminal
  are rejected.
- `GaryxComposerPayloadCoordinator` enforces one live adapter. Host activation
  advances through live, finalizing input, and closing; the next occurrence
  receives only a read-only snapshot until the durable close boundary has
  completed.
- Producer drain and reservation terminal are joined in the A3 finalization
  state machine. The finalization lease survives focus resignation and route
  release, and all six cancellation reasons deterministically terminate the
  producer side.
- Ordered persistence is serialized by an explicit transaction gate. A late
  completion may advance the durable revision but cannot project an older text
  value over a newer UIKit sequence.

## Concrete payload behavior

- The coordinator opens the A4d-1 SQLite store and protected staging directory,
  with per-scope and per-key entries, quota reservation, durable staged-file
  ownership, and activation-bound request tokens.
- Text, attachment metadata, upload operation state, and staged files are
  stored under the stable payload entry. Switching A to B and back restores
  A's text and attachment; empty text does not erase an attachment-bearing
  entry.
- Gateway G1 and G2 are independent scope partitions. Switching away suspends
  G1's payload, and switching back restores it without accepting a stale G1
  completion into G2.
- A pending or retryable upload returns `.payloadPreparing` without advancing
  text or generation. Promotion preserves the payload entry while changing
  the route key, including concurrent ordered input.

## A4d-1 reviewer follow-up

- Generic durability validation now rejects a durable-committed send barrier
  unless the same transaction state contains its matching reservation
  delivery. The fail-closed fixture proves the prior snapshot remains
  unchanged.
- `GaryxReplacementFeedbackSwapPlanner` plans from an authoritative persisted
  retryable operation, manifest, entry membership, staged-file owner, and
  quota reservation. Its concrete conversation-side fixture injects a failure
  at every planned mutation, then proves the successful transaction leaves O2
  as the sole manifest/file/quota owner while O1 remains lineage-only audit
  state.

## Same-slice deletion audit

All A4b legacy identifier families and write points listed in design section
8a are absent from `mobile/garyx-mobile`, including tests, comments, and design
records. The old text-only store source and its tests are deleted. The retained
empty-text regression proves a key with an attachment remains addressable.

The following case-insensitive assertion reconstructs the legacy identifiers
at shell evaluation time so the acceptance record itself does not reintroduce
them:

```sh
cd mobile/garyx-mobile
a4b_legacy_patterns=(
  'composer''Attachments'
  'clearAllComposer''Drafts'
  'gatewayRuntime''Generation'
  'GaryxComposerDraft''Store'
  'setComposer''Draft'
)
for a4b_pattern in "${a4b_legacy_patterns[@]}"; do
  ! rg -ni "$a4b_pattern" .
done
! rg -n '\.on(Change|Disappear).*composer|\.onChange\(of: draftText\)' \
  App Sources Tests UITests docs
```

Both assertions completed with zero matches.

## Functional acceptance

The clean SwiftPM run passed 1,386 of 1,386 tests with zero failures. Its Core
coverage includes the full input-product reducer, both producer/reservation
terminal orders, exact close idempotence, six cancellation reasons, host
activation phases, payload lifecycle, A-to-panel-to-A and A-to-A occurrence
identity, attachment A-to-B-to-A restoration, gateway partition restoration,
send locking, promotion, concrete durability validation, and the persisted
replacement-swap fixture.

The app-hosted run passed 102 of 102 tests with zero failures. Its real runtime
fixtures cover:

- a real `UITextView` CJK marked range, `unmarkText()` commit, and exact final
  sequence;
- pending dictation result and failure branches, each with exactly one terminal
  event;
- text plus staged attachment restoration across keys and gateway scopes;
- `.payloadPreparing` without text advancement;
- unique live adapter handoff, promotion concurrent with ordered typing, and a
  rapid-input durability completion that cannot regress visible text;
- durable close before destination activation for all six cancellation events;
  and
- production route occurrence behavior for A-to-panel-to-A and repeated A.

The focused `GaryxMobileFluidRoutes` run passed 13 of 13 XCUITests with no
skips or expected failures. Ten A4a fake-host tests re-exercised the frozen
physics, ownership, 20-layer, RTL, 500-churn, body-recomputation, mounted-host,
and frame-gap performance gates. Three new production tests exercised the real
conversation and UIKit composer:

- slow cancel: source keyboard remains first responder, exactly one live and
  focused adapter remains, terminal is cancelled-visible, system-frame oracle
  passes, and backwards frames remain zero;
- finish: focus is present at release, route depth reaches zero, both live and
  focused adapter counts reach zero, terminal is committed-visible, and the
  same frame oracle passes; and
- cancel-settle regrab: the production container records one regrab, commits
  to home, drains adapter/focus ownership, passes the system-frame oracle, and
  records zero backwards frames.

Debug and Release generic iOS Simulator builds both passed. The build warnings
were pre-existing app-source iOS 26 deprecations; the A4b sources emitted no
compiler warnings.

## Reproduction

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
