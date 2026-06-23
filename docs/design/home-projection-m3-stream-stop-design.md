# Home Projection M3 Stream Stop Design

## Scope

This M3 slice is based on the M2 shadow branch and implements the assigned
`GatewayStreamActor` plus `popToHome` B2 stop-stream behavior. It is a shadow
stage: the UI still renders from the legacy selected-thread and home-list paths,
and the branch is intentionally left on the integration line.

`docs/design/home-list-rebuild-v4.md` later renumbered the migration table and
described `GatewayStreamActor` as optional for the home-only performance target.
This task explicitly chooses the stream-stop slice anyway. The implementation
therefore uses only the parts of that design that still apply:

- B2: home navigation stops the selected-thread stream.
- Section 3.1: only projection input moves; transcript lifecycle side effects
  stay on the main actor.
- M2 shadow contract: `HomeProjectionActor` receives running-state inputs, but
  the UI does not cut over to its snapshot.

## Current Problem

`GaryxMobileModel+ThreadStream.swift` currently keeps three stream concerns
inside the main-actor model:

- opening `/api/threads/{id}/stream?after_seq=<cursor>`
- iterating `bytes.lines`
- maintaining per-connection committed seq state, including gap reconnect,
  stale replay handling, and control-rewrite refetch decisions

Decode and transcript-cache merge are already detached, but every SSE line still
enters the main actor before it is classified. When the user returns home, the
current stream keeps running for the selected thread because `popToHome()` only
cancels the selected-thread reconcile loop.

## Architecture

Add a Core `GatewayStreamActor` that owns one selected-thread stream connection
loop and emits typed actions back to the main model.

The actor owns:

- request/connect/retry loop
- SSE line iteration
- payload decode
- per-connection seq bookkeeping
- 404 fallback detection
- gap-reconnect resume override
- control-rewrite refetch decision

The main actor still owns:

- selected-thread identity and generation checks
- resume cursor calculation from local transcript state
- applying committed messages to the transcript cache
- applying render snapshots
- authoritative refetch after control rewrite
- fallback to history/reconcile
- every lifecycle side effect inside `applyTranscriptRunState`

The bridge shape is:

```text
GaryxMobileModel startSelectedThreadStream
  -> Task awaits GatewayStreamActor.run(...)
       -> action: applyCommittedMessages([...])
       -> action: applyRenderSnapshot(snapshot)
       -> action: updateCursor(lastSeq)
       -> action: refetchAfterControlRewrite
       -> action: fallbackAfter404
  -> GaryxMobileModel applies each action on @MainActor
```

The actor never mutates model state directly. It asks the main model for the
initial cursor through an async closure and sends actions through a strict
serial async mailbox. The stream task awaits delivery of each action before it
continues to the next frame when that action affects transcript state, cursor
state, or reconnect decisions. This keeps local selection/cursor/cache state
authoritative on the main actor while moving the byte loop and seq loop away
from it.

The blocking action contract is important for self-heal paths:

- `applyCommittedMessages` is awaited before the actor advances to later
  same-frame actions.
- `refetchAfterControlRewrite` is awaited and returns the recomputed resume
  cursor, after the main actor has cleared local transcript state, reloaded
  authoritative history, and recalculated `selectedThreadStreamCursor(for:)`.
- The actor must not request or reuse a resume cursor for the rewrite reconnect
  until that refetch action has completed.
- Non-mutating diagnostics may be fire-and-forget only if they cannot affect
  cursor, reconnect, or lifecycle behavior.

## Seq and Self-Heal Invariants

The actor must preserve the S5 behavior exactly:

- Resume cursor is still computed by `selectedThreadStreamCursor(for:)`.
- HTTP 404 still permanently falls back to history plus selected-thread
  reconcile.
- A live seq gap still reconnects from the last contiguous seq.
- Stale replay rows are skipped.
- Applied rows still get `index = seq - 1` and `id = "history:\(seq - 1)"`.
- `range_rewrite` and `transcript_reset` still clear local transcript state and
  refetch authoritative history.
- A connection with committed progress resets retry backoff.
- A sustained failure before committed progress still falls back after four
  failures.

The focused corpus test should compare the stream actor's action transcript to
the legacy planner contract for:

- ordinary replay plus live contiguous events
- stale overlap
- mid-stream gap reconnect
- HTTP 404 fallback
- control rewrite refetch

The corpus must assert message ids and resume cursor values exactly. The
control-rewrite case must also assert action ordering: if a frame contains
committed rows before `range_rewrite` or `transcript_reset`, the actor first
emits and awaits `applyCommittedMessages(precedingRows)`, then emits and awaits
`refetchAfterControlRewrite`, and only then reconnects using the cursor returned
by the refetch action.

## Projection Input Boundary

`applyTranscriptRunState(_:threadId:)` stays on the main actor. The function is
not split. The only new behavior is an immediate projection-input event after
the `runStateByThread[threadId] = state` write:

```swift
emitCommittedRunStateProjectionDelta(threadId: threadId, state: state)
```

That event updates the M2 shadow gateway with a committed-run-state delta. The
following side effects remain in place after the write and do not move into any
actor:

- provider input ack
- direct follow-up advancement
- active assistant segment completion
- title update
- terminal cleanup
- selected-thread recovery cancellation
- `runTracker` completion/interruption
- follow-up home refresh

## B2 Stop Stream

There are two stop variants. The shared existing name keeps abort semantics, and
home navigation uses a separate preserving stop.

`stopSelectedThreadStream()` remains synchronous abort/no-drain stop. It is used
by stream ownership changes such as `startSelectedThreadStream` and by
selection-clearing paths such as `openNewThreadDraft`. It cancels the active
network loop, clears stream ownership/generation/resume override, cancels any
delayed flush, and cancels any pending home-drain task. It must not run
`flushSelectedThreadStreamWindow` for a thread that is being replaced or cleared,
because that would leak selected-conversation lifecycle side effects into the
new selection/draft path.

`stopSelectedThreadStreamForHome()` is the B2 preserving stop used only by
`popToHome()`. `popToHome()` remains a synchronous SwiftUI-facing method and
calls this preserving stop immediately before it mutates navigation state to
home. The preserving stop:

- captures the current selected thread id and stream generation
- cancels the active network loop immediately, so no more SSE bytes are read
- clears stream ownership/generation and the stopped connection's one-shot
  `selectedThreadStreamResumeOverride`
- preserves cached transcript windows and render snapshots
- preserves the cursor derivable from `selectedThreadStreamCursor(for:)`
- cancels the delayed flush task and schedules a single guarded drain task that
  awaits `flushSelectedThreadStreamWindow(for: threadId)`

The guarded drain runs on the main actor and keeps the existing
`selectedThread?.id == threadId` check. In the normal B2 path, `popToHome()`
does not clear `selectedThread`, so the guard passes and the current cached
window is flushed before the flush task is forgotten, preserving terminal
run-state lifecycle side effects. If the user immediately switches to another
thread or opens a draft before the drain runs, the guard fails and no lifecycle
side effects from the old conversation are applied to the new visible
conversation. This is the caller contract that prevents the drain from being
shared with restart/new-draft paths.

Same-thread reopen also needs an explicit stream restart trigger because
`popToHome()` keeps `selectedThread` unchanged. The implementation adds an
`ensureSelectedThreadStreamForVisibleConversation()` helper that starts the
selected-thread stream when all of these are true:

- gateway settings are available and the gateway is ready
- `navigationState.presentsContent` is true
- `selectedThread?.id` is non-empty
- there is no active stream already owned by that selected thread

`openConversation()` calls this helper after it mutates navigation state to a
visible conversation. `selectThread` additionally records whether it is
reopening the already selected thread from home; for that same-thread reopen
case it calls `ensureSelectedThreadStreamForVisibleConversation()` again after
`await loadSelectedThreadHistory()`. That post-history call is the cursor-resume
proof point: the stream asks for the current cursor after the authoritative
history refresh, so it cannot resume from a stale pre-home cursor or from a
cancelled connection's transient resume override. The helper is idempotent, so
different-thread selection and non-row reopen paths keep their existing
behavior.

## Running Dot

Home-list rendering remains legacy in this shadow stage. The old visible dot is
still supplied from `recent_threads` summary rows through
`isThreadSummaryRunning(_:)`. The new actor/projection path only feeds the M2
shadow reducer and diagnostics.

Stopping the selected-thread stream on home means:

- remote or other-device running threads stay visible through `recent_threads`
  projection columns and background reconcile
- the previously selected thread may update at the reconcile cadence after the
  stream is stopped, which is an accepted B2 tradeoff
- no selected-thread SSE traffic should be necessary while the home list is the
  visible surface

## Validation

Design review must check:

- byte-loop and seq accounting are isolated from the main actor without moving
  lifecycle side effects
- B2 stop-stream preserves cursor resume and running-dot inputs
- S5 404, gap reconnect, stale overlap, and control rewrite paths are unchanged
- preserving home stop is not reused by restart or draft paths
- same-thread reopen has a visible-conversation stream restart trigger
- control rewrite applies preceding committed rows before refetch and awaits
  refetch before reconnect cursor calculation

Implementation validation:

```bash
cd mobile/garyx-mobile && xcodegen generate
cd mobile/garyx-mobile && swift test
cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build
```

Code review must independently verify the focused stream corpus, stop-on-home
coverage, cursor-resume behavior, and message-id equality.
