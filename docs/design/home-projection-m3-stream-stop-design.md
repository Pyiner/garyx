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
initial cursor through an async closure and returns actions. This keeps local
selection/cursor/cache state authoritative on the main actor while moving the
byte loop and seq loop away from it.

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

The corpus must assert message ids and resume cursor values exactly.

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

`popToHome()` calls `stopSelectedThreadStream()` immediately. It still cancels
selected-thread reconcile first, then starts the home/background reconcile path
through the existing home snapshot refresh.

Stop behavior must preserve local resume state:

- Do not clear cached transcript windows.
- Do not clear render snapshots.
- Do not reset the cursor derived from `selectedThreadStreamCursor(for:)`.
- Cancel only the active network loop and the delayed stream flush task.
- If a flush task is pending, drain the current cached window before clearing the
  task so final run-state lifecycle side effects are not lost.

When the user reopens the same thread, `selectThread` still calls
`loadSelectedThreadHistory()` before the stream resumes. The stream then asks for
the current cursor and resumes from the local committed window/render snapshot,
so the reopened view cannot reuse a stale stream cursor.

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

Implementation validation:

```bash
cd mobile/garyx-mobile && xcodegen generate
cd mobile/garyx-mobile && swift test
cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build
```

Code review must independently verify the focused stream corpus, stop-on-home
coverage, cursor-resume behavior, and message-id equality.
