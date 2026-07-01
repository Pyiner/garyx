# TASK-1502 iOS Historical User Skeleton Diagnosis

Status: design review PASS; final SwiftPM and xcodebuild validation complete.

This document intentionally avoids production thread ids, user ids, bot ids,
local personal paths, tokens, and real message bodies. Runtime observations below
are sanitized to sequence numbers, roles, body lengths, and render-window shape.

## Problem

The reported iOS symptom is that older historical user bubbles in an already
opened thread keep rendering as the gray loading skeleton, while assistant
content in the same turns renders normally. The risky area is the interaction
between:

- server-owned `thread_render_frame { events, render_state }`
- windowed `render_state.rows` under `render_floor`
- the mobile committed-body cache / visible message projection
- `GaryxMobileRenderStateMapper` resolving render refs into view models

The red line remains unchanged: iOS must not regroup user turns, regroup tools,
derive tail thinking, place final answers, or otherwise recompute transcript
structure. It may only map server `render_state` refs to committed bodies already
held by the client, or show an unresolved-body placeholder.

## Current-Code Evidence

I checked the current branch (`main`) against the suspected files and the prior
designs:

- `docs/design/thread-open-replay-trim-design.md`
- `docs/design/task-1387-ios-transcript-history-scroll.md`
- `docs/design/task-1462-ios-stream-cursor-frontier.md`
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxMobileRenderState.swift`
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GatewayStreamActor.swift`
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxTranscriptSyncPlanner.swift`
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+ThreadStream.swift`
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+Threads.swift`

The current code already contains the two fixes that match the strongest prior
root-cause theory:

1. The cold selected-thread stream requests `initial_user_turns=3`, matching the
   HTTP history window and avoiding the old one-turn opening window.
2. `GatewayStreamFrameProcessor` advances the committed body frontier only from
   accepted `committed_message` events. It does not treat
   `render_state.based_on_seq` from a render-only frame as proof that bodies were
   received.
3. `MessageLookup.mobileMessage(for:)` resolves in this order:
   visible mobile projection by history index / id, then committed transcript
   cache by history index / id. This covers the normal throttle state where
   `render_state` references a body that has reached the transcript cache but the
   visible `messages` array has not flushed yet.
4. The scroll-up history path closes the arbitrary-floor concern in current UI:
   `loadOlderSelectedThreadHistory()` persists the fetched older committed
   messages into the transcript cache, prepends their real bodies into the
   visible projection, computes `render_floor` from that older page's first
   history index through `GaryxThreadWindowPlanner.floorSeqForOlderPage`, then
   restarts the selected-thread stream so the expanded server snapshot is
   requested after the client has the corresponding bodies.

Focused existing SwiftPM coverage is green on current HEAD:

```text
swift test --package-path mobile/garyx-mobile --filter 'GaryxMobile(LatestUserSkeletonReproTests|RenderStateMapperTests)|GaryxTranscriptSyncPlannerTests|GaryxTranscriptCacheTests'
```

Result: 50 selected tests passed. This includes the previous latest-user skeleton
repro and the stream-cursor/frontier coverage.

## Server Oracle

I captured the live per-thread SSE for the reported thread without storing raw
content in the repo. The current server response for the mobile cold-open shape:

```text
GET /api/threads/{thread}/stream?after_seq=0&replay_scope=initial&initial_user_turns=3
```

Sanitized shape:

```text
events: 105 committed messages
event seq range: 2...106
user events:
  seq 2  index 1   textLen 216
  seq 32 index 31  textLen 2342
  seq 85 index 84  textLen 32
render_state:
  based_on_seq: 106
  window.floor_seq: 2
  window.has_more_above: true
  rows: 3 user_turn rows
rows:
  row 1 user seq 2  with assistant/tool/final refs through seq 27
  row 2 user seq 32 with assistant/tool/final refs through seq 80
  row 3 user seq 85 with assistant/tool/final refs through seq 103
```

Important conclusion: for current server + current mobile request parameters,
the target thread's old user bodies are present in the replay events and the
server render state includes user refs for the historical turns. That points away
from a current server-side missing-body or bodyless-row bug for this target.

I also checked the old one-turn request shape:

```text
GET /api/threads/{thread}/stream?after_seq=0&replay_scope=initial&initial_user_turns=1
```

It returns only the latest user turn (`floor_seq=85`, one render row). That is the
pre-TASK-1387 behavior and matches how a stale iOS build could make the user hit
the top boundary immediately and exercise scroll-up expansion.

## Remaining Repro Status

I do not yet have an honest RED against current HEAD for the exact reported
state. The strongest current observation is:

- The live oracle for the reported thread is healthy under current mobile
  request parameters.
- The known REDs for the prior failure family are already fixed and green on
  `main`.
- The installed iOS app that produced the screenshot may be older than current
  `main`, or the remaining bug may require a state not covered by the live
  thread capture.

Given the user's reproduce-first requirement, this is not enough to proceed to a
product-code change. A design review must decide whether this task should:

1. close as already fixed on current `main` pending app rebuild/release proof, or
2. require a new current-code RED from a different captured state before
   implementation starts.

## Plausible Current Failure Modes To Challenge

These are the only remaining seams I found worth adversarial review:

1. **Stale installed app:** an iOS build before the TASK-1387 / TASK-1462 fixes
   can still request a one-turn initial window or skip bodies after a render-only
   frame. This fits the symptom and is not a current source bug.
2. **Stale local transcript cache:** if a device has a persisted render snapshot
   or message cache created by old code, it may briefly render stale placeholders
   until the current stream/history path replaces the cache. I have not proven a
   permanent stuck state in current code.
3. **Floor inside a turn:** manually requesting `render_floor` one seq after a
   user row yields an orphan server row with assistant/tool activity but no user
   ref. This is expected for arbitrary floors. Current mobile does not appear to
   produce that input from scroll-up history: it first merges the older page's
   committed bodies, lowers the floor from the page boundary, and restarts the
   stream. This makes the manual mid-turn floor a server-contract edge case, not
   a reachable current iOS path found in this task.
4. **Projection precedence:** if the visible `messages` array somehow contained a
   stale bodyless user placeholder at the same history index while
   `transcriptMessages` already had the final body, the current lookup would
   choose the visible projection first. I did not find a current path that stores
   mapper-created user placeholders into `messages`; they are view-model output,
   not cache state.

## Proposed Decision

Do not change product code until design review resolves the missing current-code
RED.

Design review agreed this is already fixed on `main`. The implementation phase
should therefore be limited to validation evidence:

- run full `swift test --package-path mobile/garyx-mobile`
- run real `xcodebuild` for the iOS target
- optionally add a non-RED regression test only if review asks for extra
  historical-turn coverage, with a clear note that it is a guardrail, not the
  original RED
- no `xcodegen generate` unless new source files are added

If a future report finds a reachable current bug, implementation must start with
that specific failing SwiftPM test before any fix.

## Design Review Result

Review task `#TASK-1505` returned PASS. The independent review verified the
current-source trigger for `.historySkeleton`, the committed-frontier fix in
`GatewayStreamFrameProcessor`, lookup precedence in
`GaryxMobileRenderStateMapper`, and every current writer of the visible and
cached transcript bodies. It also confirmed that mapper-created unresolved-body
placeholders are pure view-model output from `rows()` and are not written back
into `messages` or the transcript cache.

The review strengthened the scroll-up conclusion: current
`loadOlderSelectedThreadHistory()` prepends real older bodies before restarting
the stream with the page-boundary floor, so the arbitrary mid-turn floor could
not be constructed through the current iOS UI path reviewed here.

## Review Questions

1. Is the current-source diagnosis sound: the reported thread's live
   `thread_render_frame` includes historical user refs and bodies under current
   mobile parameters?
2. Is there a reachable current iOS path where `transcriptMessages` lacks the
   older user bodies after scroll-up even though assistant rows render?
3. Is the arbitrary `render_floor` orphan-row behavior a server contract bug, or
   only invalid input unless mobile can produce that floor?
4. Should TASK-1502 proceed as "already fixed on main; validate/rebuild" or do we
   need a new current-code RED before any implementation work?
