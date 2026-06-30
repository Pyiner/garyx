# TASK-1462 iOS Stream Cursor Frontier Fix

Status: design pending review.

All examples use synthetic ids and text. No production thread ids, user ids,
paths, run ids, or message bodies are used here.

## Problem

iOS treats two different sequence concepts as one cursor:

- `render_state.based_on_seq`: the server render snapshot boundary. It says the
  snapshot was reduced from committed ledger records with `seq <= based_on_seq`.
  A snapshot-only frame can carry this value even when the frame has no
  committed message bodies in `events`.
- `connectionLastSeq` / reconnect `after_seq`: the client body frontier. It
  must mean "the highest contiguous committed body the client already holds"
  for this stream request.

`GatewayStreamFrameProcessor.processRenderFrame` currently applies committed
events and then unconditionally runs:

```swift
connectionLastSeq = max(connectionLastSeq, frame.renderState.basedOnSeq)
```

That is the bug. A render-only frame can make the client reconnect with
`after_seq` equal to a body it has not received. The gateway replay contract is
strictly `seq > after_seq`, so the missing body is then skipped permanently and
the server `render_state` ref maps to a `.historySkeleton` forever.

## Decision

Use direction B as the structural fix:

1. `render_state.based_on_seq` remains only the render snapshot freshness
   boundary. It drives render snapshot application, pagination window metadata,
   and presentation state.
2. `connectionLastSeq` remains only the committed body frontier. It advances
   from committed message events that the processor actually accepts for apply
   or control-rewrite refetch.
3. Reconnect `after_seq` is derived from the committed body frontier only. It
   must never consume `render_state.based_on_seq`.
4. Do not add a server compatibility mode, legacy cursor stripping, or a
   based-on-seq fallback branch. The client must stop lying about the bodies it
   holds.

## Processor Design

Change `GatewayStreamFrameProcessor` so it is explicitly initialized for each
stream request:

```swift
processor.resetConnection(
    afterSeq: streamRequest.afterSeq,
    replayScope: streamRequest.replayScope ?? .resume
)
```

The processor keeps two pieces of state:

- `connectionLastSeq`: committed body frontier for this request.
- `appliedCommittedOnConnection`: whether this connection actually accepted at
  least one committed body event.

For resume requests, initialize `connectionLastSeq` to the request `afterSeq`.
That cursor comes from committed transcript cache state (`afterCursor` or
cached message history index), so it is already a body frontier held by the
client. For initial window requests, keep the existing window-reset behavior:
`connectionLastSeq = 0` and allow the first committed event to start above 1,
because an intentional cold window may begin at a high seq.

When processing a render frame:

1. Iterate `frame.events` in order.
2. For each `committed_message` with a body, run the sequence planner against
   the current committed frontier.
3. On `.apply`, rewrite the body id/index, append it to the committed batch,
   set `connectionLastSeq = seq`, and set
   `appliedCommittedOnConnection = true`.
4. On control rewrite, advance the committed frontier to that control seq only
   because the control body was actually received, mark committed progress,
   flush preceding bodies, and request authoritative refetch as today.
5. On `.gapReconnect`, flush preceding accepted bodies and reconnect from the
   current committed frontier.
6. Append `.applyRenderSnapshot(frame.renderState)` without changing
   `connectionLastSeq`.

The sequence planner needs to preserve two cases that look similar but are not:

- Initial-window replay may start high. That first high seq is allowed only when
  `replay_scope=initial`.
- Resume replay may not start high unless it is exactly contiguous with
  `after_seq`. If a resume request has no accepted body yet and the first
  committed event is greater than `connectionLastSeq + 1`, it is a gap and must
  reconnect from `connectionLastSeq`.

This makes the upper repro deterministic: after a render-only frame for seq 95,
`connectionLastSeq` stays below 95. If the next committed event is seq 113 on a
resume stream, the processor reconnects from the held body frontier, so server
replay includes seq 95 because replay is `seq > after_seq`.

The actor's persistent-failure counter must not use `connectionLastSeq > 0` as
"progress" after this change, because a resume connection can start with
`afterSeq > 0`. A valid render frame is enough to prove the stream connected and
the server can answer, even if it is caught up and carries no new bodies. Use an
explicit per-connection progress signal that becomes true when either a render
frame is accepted or a committed body/control body is accepted. Persistent
failure fallback remains for connections that cannot deliver any valid frame.
Initialize/reset this progress state only after the request state has been built
for the current connection, so a request-construction failure cannot inherit the
previous connection's progress bit.

## Mapper Decision

Question: after B, is the state "mobile `messages` lacks a committed body but
the authoritative transcript cache has that body" still legal?

Answer: yes, but only as a bounded client-side projection lag, not as the
permanent skipped-body state caused by the bad cursor.

Current iOS stream ingestion has two layers:

- `applyStreamedCommittedMessages` merges committed bodies into
  `cachedTranscriptSnapshots[threadId].messages` immediately.
- visible `messages` are rebuilt by the throttled stream flush window
  (`GaryxStreamUpdateCadence.committedMessageBatchWindowNanos`).

During that throttle window, `GaryxMobileRenderStateMapper.rows` can see a
server render ref, a committed body in `transcriptMessages`, and no corresponding
`GaryxMobileMessage` yet. That is a normal timing state after B. It is not
legal for the body to be permanently absent after reconnect; B fixes that.

Therefore the lower repro should remain a valid mapper robustness test, with a
precise semantic: the mapper may resolve a render ref from the committed
transcript cache when the visible mobile-message projection has not flushed yet.
This is not a transcript-structure fallback. The row, turn boundary, tool
grouping, final-message placement, and tail activity still come exclusively
from server `render_state`; the mapper only converts an already-committed body
from the same transcript cache into the same `GaryxMobileMessage` shape the
flush would later produce.

Implementation consequence:

- Add a body lookup fallback in `MessageLookup.mobileMessage(for:)`:
  `mobileByHistoryIndex/id` first, then `transcriptByHistoryIndex/id` converted
  through `GaryxMobileTranscriptMapper.mobileMessages(from:)` semantics for the
  single message.
- Use it consistently for user rows, assistant flat rows, assistant step
  final blocks, and assistant step items. Tool trace rows already use
  `transcriptMessage(for:)` because they render tool payloads from committed
  transcript records.
- Keep unresolved refs as placeholders. A placeholder still means the body is
  genuinely not held by either `messages` or `transcriptMessages` yet.

This is not direction A as a patch over the cursor bug. It does not allow
`based_on_seq` to skip replay, and it does not make the mapper invent body
content. It only removes an avoidable UI projection lag once the committed body
is already present on the client.

## Impact Review

Current `basedOnSeq` uses are safe except the stream cursor mutation:

- `GaryxRenderSnapshot.basedOnSeq` codable model: data contract only.
- `GatewayStreamFrameProcessor` render snapshot action: keep as presentation
  snapshot application.
- `GaryxThreadWindowPlanner.floorSeq` / render-floor requests: windowing only.
- `HomeProjectionReducer` / `HomeProjectionActor`: independent home projection
  epochs and run-state source ordering, not selected-thread stream replay
  cursors.
- Tests that assert decoded `basedOnSeq`: contract coverage only.

Current cursor/frontier uses after this change:

- `GaryxStreamSeqPlanner.decide(...)`: still detects stale, same-seq
  replacement, contiguous apply, and gaps, but needs an explicit
  initial-window allowance instead of treating every first high seq as safe.
- `GatewayStreamReconnect.gap(resumeAfterSeq:)`: must carry the committed body
  frontier, not the render snapshot boundary.
- `selectedThreadStreamRequestForActor`: already computes `afterSeq` from
  committed transcript cache or cached message indexes, not from
  `renderSnapshot.basedOnSeq`. Keep that behavior.
- Gateway `/api/threads/{id}/stream`: no server change planned. It already
  replays `seq > after_seq`, forward-pages over replay-cap gaps, and derives
  `render_state` separately.

Expected behavior by case:

- Normal contiguous committed body stream: `afterSeq=N`, incoming `N+1`,
  `N+2`, ... all apply and advance the body frontier.
- Same-seq authoritative replacement: incoming `N` with
  `connectionLastSeq == N` still applies.
- Stale overlap: incoming `< connectionLastSeq` still skips.
- Resume gap: incoming `> connectionLastSeq + 1` reconnects from
  `connectionLastSeq`.
- Initial window: first high committed seq still applies when request scope is
  initial.
- Render-only lead: snapshot applies, transient placeholders are allowed, but
  the body frontier does not move. Any later reconnect can still replay the
  missing body.

## Tests And Validation Plan

First add the two existing reproductions to this branch and prove they are red
before product changes:

- `GaryxMobileResumeCursorRenderStateReproTests`: must fail before the cursor
  fix and pass after B. This is the structural proof that render-only
  `based_on_seq` is not a committed resume cursor and replay includes the
  missing body.
- `GaryxMobileLatestUserSkeletonReproTests`: keep the semantic that committed
  transcript cache bodies are a valid mapper fallback during mobile-message
  flush lag. It should fail before the mapper lookup refinement and pass after.

Focused updates:

- Add or update `GatewayStreamActorTests` for render-only frames, resume first
  high seq, initial-window first high seq, failure-progress accounting, stale
  overlap, gap reconnect, and control rewrite.
- Add or update `GaryxTranscriptSyncPlannerTests` if the planner signature gains
  an explicit initial-window allowance.
- Keep the lower mapper repro synthetic and public-safe.

Final gates:

```text
cd mobile/garyx-mobile && swift test
xcodebuild -project GaryxMobile.xcodeproj -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,name=<available simulator>' \
  CODE_SIGNING_ALLOWED=NO build
```

No new Core source file is planned. If implementation adds or moves a Core file,
run `xcodegen generate` and commit the `.pbxproj` update so the app target
compiles the same code that SwiftPM tests compile.

## Non-Goals

- No server replay compatibility mode.
- No client fallback that treats `basedOnSeq` as a body cursor.
- No local regrouping, final-answer placement, or tail-thinking derivation in
  Swift.
- No TestFlight or release packaging.
