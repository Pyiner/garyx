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
- The conversation live store is an immutable route value rather than a
  retained SwiftUI object. Draft promotion therefore rebuilds destination
  reads against the promoted thread immediately, while view-local scrolling
  state remains attached to the same route occurrence.
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

Panel, settings-detail, and workspace/bot/automation drilldown entry points
now also write typed destinations into the canonical stack before entering
that compatibility host. Re-projecting the complete canonical path preserves
the exact drilldown that opened a conversation. Workspace files remain the
file-browser panel, and settings details carry a real settings-overview
predecessor so the header and leading-edge gesture perform the same pop. A
direct settings-detail entry appends the overview and detail as one canonical
batch, so no transient intermediate route is mounted or published.

The production edge recognizer resolves ownership from the touched view's
actual view-controller ancestry. A presented sheet or native Form therefore
keeps its own leading-edge controls, while route-owned conversation content
still receives interactive pop. Full-width Form menu rows use the native cell
surface, including the visual leading inset, so their label and trailing
control have one coherent hit target.

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
- A rebuilt UIKit adapter resumes at the reducer-provided next input sequence;
  a draft-to-thread alias rebind keeps the same session/epoch and cannot reset
  deduplication. Non-live hosts read their own scope/key projection instead of
  the active top payload.
- Producer drain and reservation terminal are joined in the A3 finalization
  state machine. The finalization lease survives focus resignation and route
  release, and all six cancellation reasons deterministically terminate the
  producer side.
- Ordered persistence is serialized by an explicit transaction gate. A late
  completion may advance the durable revision but cannot project an older text
  value over a newer UIKit sequence.
- A committed presentation terminal supplies the settle-terminal or
  superseded cancellation event, bounding a producer whose UIKit callback
  never arrives. Transient close persistence failures retry autonomously with
  bounded backoff while the old host remains pinned and read-only.

## Concrete payload behavior

- The coordinator opens the A4d-1 SQLite store and protected staging directory,
  with per-scope and per-key entries, quota reservation, durable staged-file
  ownership, and scope/entry/lifecycle-token request admission.
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
- Launch recovery runs before the first activation. Core convergence settles
  staged durability work, then one transaction acknowledges process-dead
  input sessions, releases their promotion aliases, and removes materialized
  close rows without touching payload text.

## Adversarial R1 correction closure

The first cross-model review found eight merge blockers that the original
green matrix did not exercise. This revision closes each with a production
seam regression:

- draft promotion replaces the route-indexed conversation store, so the sent
  turn and streamed response render without leaving and re-entering;
- promotion rebinds the existing input session to the thread key, preserves
  the UIKit host, and a later pop can perform a virtual ordered close if the
  adapter is temporarily absent;
- every route-composer delivery durably records transport-attempted immediately
  before the gateway call, then records acknowledgement and releases the Entry
  reference on the accepted response. Failed/unknown responses become
  ambiguous evidence; 65 consecutive acknowledged sends prove quota release;
- settings, workspace files, drawer drilldowns, automation thread rows, and
  mobile route links all reach typed canonical destinations, and projection of
  the whole stack preserves their back target;
- both presentation-terminal cancellation reasons have real app call sites;
- send sealing re-reads the reducer after asynchronous preparation and uses
  that exact latest text/sequence, preserving edits admitted during the await;
  and
- close persistence retries itself after transient failure rather than waiting
  for an unrelated scene, producer, or route event.

Promotion requested during an active pop is queued by occurrence. Cancellation
applies it to the still-mounted occurrence after settle; a committed pop does
not reinsert the removed route. Reopening an existing conversation deliberately
creates a fresh occurrence, including A-to-A, as required by design section
1.1; it is not a deduplication defect.

The remaining cross-slice items are explicitly registered with their design
owners: scope-revoke settlement of every delivery/evidence phase and its quota
release belong to A4d-2; picker-to-staging Entry identity freezing belongs to
A4c presentation-to-operation bridging; row-level/incremental SQLite writes
for the current full-snapshot backend are an A4d-2 performance hardening item.
A4b keeps those writes serialized and off MainActor, and its existing frame-gap
and rapid-input non-regression gates remain mandatory. The dormant chat case
inside the panel compatibility host now renders no fallback conversation.

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

The clean SwiftPM run passed 1,387 of 1,387 tests with zero failures. Its Core
coverage includes the full input-product reducer, both producer/reservation
terminal orders, exact close idempotence, six cancellation reasons, host
activation phases, payload lifecycle, A-to-panel-to-A and A-to-A occurrence
identity, attachment A-to-B-to-A restoration, gateway partition restoration,
send locking, promotion, concrete durability validation, and the persisted
replacement-swap fixture.

The app-hosted run passed 114 of 114 tests with zero failures. Its real runtime
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
- draft promotion followed by an adapter-absent pop, adapter rebuild sequence
  continuation, autonomous finalization retry, and process-death alias release;
- latest-text sealing across a suspended preparation await, 65 acknowledged
  sends in one scope, and a real model/URLSession gateway request proving the
  attempted-before-request and accepted-response acknowledgement phases; and
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

An additional whole-scheme stress audit exercised all 42 UI tests. Its first
run passed 41 and produced one isolated Home-list frame-gap sample above the
existing threshold; the exact performance test immediately passed alone with
an 80.74 ms maximum interval against the unchanged 90 ms gate. A second run
passed 40 and missed two XCUI gesture injections that had passed in the first
run; those exact production-regrab and Home-filter cases then passed together,
2 of 2, without a code, timeout, retry, or threshold change. Thus every UI test
has a green current-code result, while the canonical A4b route run remains the
single-command 13-of-13 gate. The non-deterministic whole-scheme simulator
orchestration is recorded here rather than represented as a branch regression
or a 42-of-42 invocation.

Debug and Release generic iOS Simulator builds both passed. The emitted
warnings are pre-existing app-source iOS 26 deprecations or unrelated async
cleanup diagnostics; this revision introduced no new warning site.

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
