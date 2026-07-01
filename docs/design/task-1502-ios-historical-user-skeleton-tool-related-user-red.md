# TASK-1502 iOS Historical User Skeleton Tool-Related User RED

Status: design review PASS; implementation and final SwiftPM/xcodebuild validation complete.

This document intentionally avoids production thread ids, device ids, user ids,
bot ids, local personal paths, tokens, and real message bodies. Runtime evidence
is sanitized to sequence numbers, history indexes, body lengths, roles, and
synthetic ids.

## Corrections

Two prior TASK-1502 theories are now invalid:

1. Healthy server `thread_render_frame` alone does not prove the iOS path is
   healthy. The bug report was reproduced on a latest build after reinstall.
2. `.initial&initial_user_turns=3` is not a sparse event replay. Current server
   code selects a user-turn floor but returns every record from that floor to the
   tail. Raw captured initial frames also have contiguous seq ranges.

The current RED comes from the scroll-up path, where REST history pages and a
render-floor snapshot are combined on device.

## Captured Shape

The original reported history thread was captured in two steps:

- initial selected-thread window:
  - REST messages: 519 rows, returned indexes `169..<688`, 3 user rows
  - SSE initial frame: 519 events, seq `170...688`, no seq gaps, 3 render rows
  - Core processor + mapper on the raw frame is green
- scroll-up expansion:
  - REST older page before index 169: 85 rows, returned indexes `84..<169`, 3
    user rows, starts at user index 84
  - resumed stream with `render_floor=85`: snapshot-only frame, `based_on_seq=688`,
    6 render rows

The decisive sanitized row shape is:

```text
REST committed history:
  index 84 role=user kind=user_input tool_related=true textLen=32
  index 89 role=assistant kind=assistant_reply textLen>0

render_state:
  user ref id=origin:mobile-TESTDEVICE0001 seq=85
  assistant final ref seq=90
```

The same turn's assistant final resolves, but the user row maps to the gray
history skeleton.

## RED

Test:

```text
mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxMobileLatestUserSkeletonReproTests.swift
```

Focused command:

```text
swift test --package-path mobile/garyx-mobile --filter GaryxMobileLatestUserSkeletonReproTests
```

Current RED on current HEAD:

```text
testScrollUpRenderFloorKeepsToolRelatedUserBodiesFromHistory
XCTAssertEqual failed: ("") is not equal to ("Synthetic older user prompt")
XCTAssertNotEqual failed: ("historySkeleton") is equal to ("historySkeleton")
```

The test is a sanitized version of the captured scroll-up shape:

1. decode a server `render_state` row that references user seq 85 and assistant
   final seq 90;
2. provide committed REST transcript rows for history indexes 84 and 89;
3. mark the user transcript row as `kind=user_input` and `tool_related=true`,
   matching the captured REST projection;
4. run `GaryxMobileRenderStateMapper.rows(...)` through the real mapper.

This is the correct no-UI seam: server-declared render snapshot plus committed
history bodies into the mobile Core mapper. It does not regroup turns or infer
tool/final placement locally.

## Root Cause

`GaryxMobileTranscriptMapper.mobileMessages(from:)` drops any transcript row
where `GaryxMobileTranscriptToolTraceClassifier.kind(for:)` returns non-nil.

`GaryxMobileTranscriptToolTraceClassifier.kind(for:)` currently:

- explicitly treats `.toolUse` and `.toolResult` roles as tool rows;
- then, for any `toolRelated` row, classifies by `kind`;
- `kind=user_input` contains the substring `"use"`, so the classifier returns
  `.toolUse` for a user row.

That is valid enough for tool rows, but invalid for committed user messages.
When REST history marks a user row `tool_related=true`, the mobile transcript
projection drops that body. The server render snapshot still contains the user
ref, so `MessageLookup` cannot resolve it and `userStepPlaceholder(for:)`
produces a streaming empty user message, which the view presents as
`.historySkeleton`.

The assistant rows remain normal because their committed bodies are not filtered
by this user-row path.

## Proposed Fix

Keep the dumb-render contract unchanged. `render_state` remains the only source
of visible row structure.

Change only transcript body classification:

1. A transcript row with `role == .user` must never be classified as a tool
   trace by `GaryxMobileTranscriptToolTraceClassifier`, regardless of
   `tool_related` or `kind`.
2. Keep existing classifier behavior for `.toolUse`, `.toolResult`, and
   assistant tool-trace rows. Existing tests cover assistant tool-related rows.
3. Keep `GaryxMobileRenderStateMapper` lookup order and render-row mapping
   unchanged.

This makes committed user bodies available to `MessageLookup` by history index,
so the existing server snapshot maps to text instead of a skeleton.

## Implementation Result

The implementation keeps the render-state mapper unchanged and changes only
`GaryxMobileTranscriptToolTraceClassifier.kind(for:)`: rows with `role == .user`
now return `nil` before `tool_related` kind heuristics run. Dedicated tool-use
and tool-result roles still return their tool classifications, and assistant
tool-related rows still follow the existing heuristics.

Desktop does not share this failure mode. Its render view model builds a
`MessagesBySeq` map from committed transcript rows and resolves server
`render_state` refs directly by `seq`; it does not drop user bodies through the
mobile transcript-to-message projection, and it does not synthesize a mobile
history skeleton for missing user bodies.

## Validation

RED to GREEN passed:

```text
swift test --package-path mobile/garyx-mobile --filter GaryxMobileLatestUserSkeletonReproTests
```

Focused regression passed:

```text
swift test --package-path mobile/garyx-mobile --filter 'GaryxMobile(LatestUserSkeletonReproTests|RenderStateMapperTests|ResumeCursorRenderStateReproTests)|GaryxGatewayClientTests|GaryxTranscriptSyncPlannerTests|GaryxTranscriptCacheTests|GatewayStreamActorTests'
```

Full validation passed:

```text
swift test --package-path mobile/garyx-mobile
xcodebuild -project mobile/garyx-mobile/GaryxMobile.xcodeproj -scheme GaryxMobile -configuration Debug -sdk iphonesimulator CODE_SIGNING_ALLOWED=NO build
```

No new Core source file is expected. If implementation adds one, run
`xcodegen generate` before the final build.

## Review Questions

1. Is the RED now at the right seam and based on a reachable current input shape:
   REST history user body marked `tool_related=true`, plus render-floor snapshot?
2. Is the root cause correct: user rows are being filtered by the tool-trace
   classifier before `MessageLookup` can resolve render refs?
3. Is the proposed fix narrow enough: never classify `.user` rows as tool traces,
   while preserving tool-use/tool-result and assistant tool-trace grouping?
4. Are there any desktop implications, or is this specific to the mobile Core
   transcript-to-view-model projection?
