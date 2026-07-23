# iOS Conversation Opening Transcript Unification

Status: approved design; implementation pending. Author: Gary (2026-07-23).
Reproduction and root cause: #TASK-2644
(`docs/design/task-2644-ios-existing-thread-loading-flash-reproduction.md`,
red test `GaryxExistingThreadLoadingFlashReproTests`, commit `5f4abd1e6` on the
#TASK-2644 worktree branch).

## 1. Problem

Opening an existing thread that already has locally persisted messages shows:

1. a loading skeleton that, on completion, reveals the historical messages
   with a visible flash/rebuild;
2. gray skeleton bars and real message rows visible together in one transcript
   viewport (interleaved).

Both symptoms are one state chain:
`openingPage [skeleton] → materializingConversation [real rows + skeleton
overlay at opacity 0.999] → live [real rows, overlay opacity 0]`.

This class of defect has recurred repeatedly. This design removes the class,
not the instance.

## 2. Why it recurs: three uncoordinated authorities

Archaeology (over TASK-1497 audit, TASK-1387, TASK-1502, TASK-1802,
TASK-2630/settlement, fluid P0-A) shows the rule itself was always written
down — *"a message skeleton is valid only when there is nothing local to
render"* (`GaryxConversationRoutePresentation.swift:52-59`, mobile-ui contract,
TASK-1497 audit). What drifted is ownership. Today three independent
authorities decide what the transcript region shows, and no test asserts their
composition:

| # | Authority | Layer | Nature |
| - | --- | --- | --- |
| 1 | `GaryxConversationOpeningMetadata.transcriptPresentation` — frozen once at row-tap time from the then-empty in-memory row store | App (`GaryxMobileModel+ThreadLifecycle.swift`) | stale snapshot |
| 2 | `GaryxConversationRoutePresentationState.renderPhase` frame clock (openingPage → materializing → live) | Core | performance choreography, explicitly network-blind |
| 3 | `isSelectedThreadLoadingInitialHistory` (TASK-1497 Core pure function) | Model, real-time | live truth |

Every fast-path change (first-frame chrome, composer unlock, refresh
decoupling) touched one authority, passed its isolated tests, and re-broke the
composition. The cold-open sequence "empty memory → async disk restore →
materialize" has never been driven by any test.

## 3. Design

### 3.1 Single-value treatment, derived every frame, never frozen

One Core-owned pure function is the only authority for what the transcript
region shows:

```
GaryxConversationTranscriptTreatment  =  .skeleton | .content
```

- Derived from the **same real-time inputs** as
  `GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(...)`
  (local renderable row count, history-loaded flag, rendered snapshot
  availability, transcript mirror). One input set, one function, one value.
- Evaluated on every input change. There is no cached/frozen copy: the
  `transcriptPresentation` field of the opening-metadata cache is **removed**
  (the cache keeps only chrome metadata such as title/identity). Async disk
  restore completing is an input change and re-derives the treatment
  naturally.
- SwiftUI renders the value with an exhaustive `switch`. Skeleton and real
  rows are structurally incapable of co-existing in one viewport because the
  render input is a single enum value, not two composited layers.

### 3.2 Frame clock keeps choreography, loses content semantics

`GaryxConversationRoutePresentationState` (openingPage → materializing → live)
remains as the mount/hitch choreography — when the heavy live transcript graph
is attached, and when interaction is handed to it. It no longer implies
anything about visible content:

- `showsOpeningTranscriptCover` in its current "overlay until live" meaning is
  deleted. There is no translucent layer over mounted real rows, ever. The
  `opacity(0.999 → 0)` composite is removed.
- The opening cover is re-scoped as a **transition-continuity surface** with a
  strict legality rule (3.3). While legal, it is opaque and is the only
  visible transcript layer; the phase switch to the live transcript is an
  atomic replacement (boss red line 2026-07-21: jitter is solved by
  anchoring/atomic replacement, never by layered cross-fades).

### 3.3 Cover legality rule

The cover may exist only while it can show **the same treatment the live
transcript would show**:

```
coverIsLegal = (treatment == .skeleton)            // nothing local to render
             || hasTranscriptSnapshotPixels        // cached pixels of the same content
```

- `.skeleton` cover: zero local renderable rows — identical to what live
  would show, so the later handoff is visually a no-op.
- Snapshot cover: the existing `GaryxConversationTranscriptSnapshotView`
  cached-pixels path, geometry-aligned (existing page-y contract test stays).
- The moment `treatment == .content` with no snapshot pixels available (e.g.
  disk restore lands mid-materialization), the cover has nothing truthful to
  show: the state machine **promotes to `.live` immediately** and atomically
  reveals the already-mounted real rows. Continuing the stability proof would
  only be observable as a lie (skeleton over content — today's bug), so the
  proof is short-circuited by rule, not by timing.

### 3.4 Scenario matrix (normative)

| Scenario | Treatment timeline | Skeleton ever visible? |
| --- | --- | --- |
| S1 Cold open, disk has messages, memory empty | tap: snapshot pixels if available, else `.skeleton` only until restore lands (typically < first frames); restore lands → `.content`; cover either shows snapshot or promotes live immediately | Only before any local content exists; never over real rows |
| S2 Cold open, no local content anywhere | `.skeleton` (cover and live derive the same value) → first renderable rows arrive → atomic switch to `.content` | Yes, as the only layer; switch is atomic, no mixed frame |
| S3 In-thread incremental frames (already live) | Opening system not involved; `render_state` drives row updates; treatment stays `.content` | Never |
| S4 Gateway failure + auto-retry on cold start | Same pure function: no rows → `.skeleton` persists seamlessly across cover→live handoff (identical treatment both sides); rows exist → `.content` stays visible while retry continues in background. Auto-retry / no-manual-Reload contract unchanged | Only while zero local rows |

### 3.5 Invariants (each becomes a test assertion)

- **INV-1** Visible transcript kinds per frame are exactly `[skeleton]` or
  `[content]`; a mixed frame is unrepresentable in the render input and
  asserted never to occur across the driven sequence.
- **INV-2** `localRenderableRowCount > 0 || hasRenderedSnapshot` ⟹ visible
  treatment is `.content`. (The TASK-1497 rule, now held continuously instead
  of sampled once at tap time.)
- **INV-3** Cover treatment ≡ live treatment at every frame (same inputs, same
  function); the cover never outlives its legality rule.
- **INV-4** The skeleton→content transition is a single atomic replacement:
  zero frames in which both layers are visible; no opacity intermediate.

## 4. Impact surface

- `Sources/GaryxMobileCore/GaryxConversationRoutePresentation.swift`: new
  treatment value + derivation; cover-legality rule; short-circuit promotion;
  remove content semantics from `showsOpeningTranscriptCover`.
- `App/GaryxMobile/GaryxMobileModel+ThreadLifecycle.swift`: stop freezing
  `transcriptPresentation` in `cacheConversationOpeningMetadata`.
- `App/GaryxMobile/GaryxConversationRouteView.swift`: metadata cache loses the
  presentation field; opening view renders the derived treatment.
- `App/GaryxMobile/GaryxMobileConversationViews.swift`: replace the
  ZStack + opacity composite with exhaustive single-value rendering; atomic
  cover→live replacement.
- Tests: `GaryxExistingThreadLoadingFlashReproTests` (red → green),
  new composition-sequence test (3.5), existing
  `GaryxConversationRoutePresentationTests` updated for the re-scoped state
  machine; snapshot geometry contract test retained.

## 5. Structural guard (why it will not drift again)

- The composition itself becomes a Core value: there is no second place to
  decide visibility, and the App target has no material to build a competing
  decision from (the frozen field no longer exists).
- One SwiftPM composition test drives the full cold-open sequence — empty
  memory → async restore lands → frame-clock materialization → live — using
  the real captured fixture from #TASK-2644, asserting INV-1..4 at every step.
  Any future fast-path change that reintroduces a second authority fails this
  test at the composition layer, not at a unit boundary.
- Guard is structural (data flow + pure function + composition assertion), per
  repo policy; no source scanning.

## 6. Non-goals and trade-offs

- The frame-clock hitch choreography (P0-A asset) is kept; only its content
  semantics are removed. We deliberately accept that a mid-materialization
  restore short-circuits the stability proof: correctness of what the user
  sees outranks completing a proof whose only observable effect would be
  showing a stale cover.
- `render_state` ownership, mapper dumbness, tail-scroll settlement machine,
  and the auto-retry contract are untouched.
- Adjacent pre-existing issues discovered during implementation (e.g. anything
  in the TASK-2630 settlement area) go to a debt doc and separate tasks; they
  do not expand this scope.
