# iOS Composer: First-Frame Interactivity + Keyboard Dismissal

Status: approved design, ready for implementation.
Scope: iOS app only (`mobile/garyx-mobile`). Desktop already has the correct
behavior and is the reference.

## Problem

1. Entering an existing thread, the composer is non-interactive for a
   noticeable window. Investigation confirmed transcript/history loading is
   **not** wired into any composer disable path. The real causes are two
   entry-time mechanisms:
   - **Route opening cover**: `GaryxConversationRouteView.swift` flips
     `allowsHitTesting` on the whole live conversation
     (`presentationDriver.allowsLiveConversationInteraction`, driven by
     `GaryxConversationRoutePresentation.renderPhase == .live`, which waits
     for 2 opening frames + 12 stable materialization frames). The cover's
     composer is `GaryxConversationOpeningComposerChrome` — a static,
     non-interactive look-alike. When frames are unstable (exactly when
     history is loading), this window stretches and reads as "loading locks
     the input".
   - **Payload activation fallback**: until the durable composer store
     finishes `activate()` for the thread's composer key,
     `inputConfiguration()` is nil and the input mounts
     `GaryxComposerInputFallback` (`Color.clear`, `allowsHitTesting(false)`).
2. With the keyboard up, tapping outside the input (transcript area) does not
   dismiss it, and scrolling does not dismiss it interactively.

The send pipeline itself has **no transcript dependency**: thread_id, composer
key, send barrier, and outbox are all established at open time
(`applySelectedThreadRouteProjection` → `activateComposerPayload`), fully
independent of `loadSelectedThreadHistory`.

## Behavior contract

- **C1** — Entering an existing thread, the composer is tappable, focusable,
  editable, and sendable from the first visible frame, fully independent of
  transcript history loading state (parity with the Mac app, the IA source of
  truth).
- **C2** — The complete set of composer interaction locks is exactly:
  1. not the canonical-top live route occurrence (locks input + send);
  2. durable payload not yet activated / read-only barrier (locks input +
     send; this window is a fast local store operation, no visual loading
     state);
  3. send commit in flight (locks send only);
  4. attachments still uploading (locks send only);
  5. run busy without queueing allowance (locks send only; existing
     stop-button semantics unchanged).
  No other input lock may exist. Transcript loading must never appear in any
  composer lock derivation.
- **C3** — With the keyboard presented: scrolling the transcript dismisses it
  interactively (native iOS `.interactively` behavior); tapping non-input
  blank space in the message area dismisses it without stealing in-row
  interactions (links, long-press menus, tool-row expansion).
- **C4** — Existing new-draft-without-enabled-agent disabled semantics are
  unchanged.

## Design

### A. Single composer instance, live from the first frame

- The opening cover stops owning the composer region. The cover keeps its
  pixel + interaction-shield role for the **transcript region only**; the
  composer is the one live `GaryxComposer` instance from the first destination
  frame. No static look-alike twin, no fake-to-real swap seam — single owner.
- Delete `GaryxConversationOpeningComposerChrome` (or whatever remains of the
  opening composer path) once the live instance owns the region from frame
  one. Do not keep it as a fallback branch.
- Narrow `allowsLiveConversationInteraction` semantics to the transcript
  region's interaction gate and rename it accordingly (e.g.
  `allowsTranscriptInteraction`) so the name matches the narrowed meaning.
- P0-A gesture-physics spec
  (`mobile/garyx-mobile/docs/ios-fluid-p0a-gesture-physics-design.md`) remains
  authoritative: the live composer joins the prewarm set; the
  moving-transition and post-reveal hitch gates must keep passing. If the
  first-frame live mount trips a hitch gate, fix it with prewarming — do not
  regress to a look-alike cover.

### B. Activation window

- Locks (1) and (2) in C2 are correctness locks and stay. Verify activation
  starts at the earliest synchronous point of the open path (it already runs
  in `applySelectedThreadRouteProjection`); do not serialize it behind
  anything unrelated. The fallback stays visually identical to the live field
  (geometry-stable), with no loading affordance.

### C. Keyboard dismissal

- Transcript scroll container: `.scrollDismissesKeyboard(.interactively)`.
- Background tap in the message region dismisses the keyboard. Attach at the
  transcript background layer so child gestures (links, long-press context
  menus, tool-row taps) win untouched — never a full-screen gesture overlay
  and never `simultaneousGesture` on rows.

## Impact surface

- `App/GaryxMobile/GaryxConversationRouteView.swift` — hit-testing split
  between transcript cover and composer region.
- `Sources/GaryxMobileCore/GaryxConversationRoutePresentation.swift` —
  interaction-gate naming/semantics (state machine itself unchanged).
- `App/GaryxMobile/GaryxMobileComposerViews.swift` — remove the opening
  composer look-alike.
- Prewarm list + transition hitch gates.
- Conversation transcript container — keyboard dismissal.

## Validation

- `GaryxMobileCore` SwiftPM tests: presentation-state tests updated so that
  for an existing thread every `renderPhase` yields composer
  interactivity = true while the transcript gate keeps its current phase
  behavior. Headless-first: drive the state machine directly.
- Existing moving-transition + post-reveal hitch gates all pass.
- Simulator (iPhone 17 Pro Max, iOS 26.5, light mode): enter a thread and
  immediately tap the input → keyboard appears while history still loads;
  tap blank transcript space → keyboard dismisses; scroll → interactive
  dismissal; long-press menus and links unaffected; new-draft-no-agent state
  still disabled.

## Scope boundary

Only the surfaces listed above. Adjacent pre-existing issues discovered along
the way go to `docs/design/ios-composer-unlock-review-debt.md` with
provenance, filed separately — never folded into this task. Review checks the
touched paths against this document (intent + no regressions), not a full
audit of neighboring code.
