# TASK-2630: iOS Existing-Thread Loading Jitter Reproduction

Status: reproduced; root cause located; intentionally not fixed.

## Reported boundary

The report is specifically:

- open an existing thread with historical messages;
- let the initial message-region loading state finish;
- make no server-side data change;
- the visible transcript still jumps or wobbles.

The reporting thread did not identify the affected conversation. The
reproduction therefore uses the repository's already-sanitized real capture
`mobile/garyx-mobile/Tests/GaryxMobileCoreTests/Fixtures/task-2610-markdown-table-frame.json`.
It contains canonical transcript seq `103...116` and the corresponding idle
server snapshot:

- `based_on_seq = 116`;
- `window.floor_seq = 103`;
- `tailActivity = none`;
- one server-owned turn row with historical user and assistant bodies.

This is not a fabricated row model. The test decodes the captured
`thread_render_frame`, runs it through `GatewayStreamFrameProcessor`, and feeds
its committed bodies and `render_state` to
`GaryxMobileRenderStateMapper`.

## Deterministic headless reproduction

Test:

`GaryxExistingThreadLoadingJitterReproTests.testUnchangedCapturedThreadStopsTailWritesAfterLoadingEnds`

The test drives the production pure-state sequence:

1. Advance `GaryxConversationRoutePresentationState` through the default two
   opening frames and twelve materialization frames until the transcript is
   `.live`. This is permitted while history I/O is still outstanding.
2. Begin with zero local rows, which selects the shared message-region
   `.loading` presentation.
3. Apply the captured historical frame once. The empty-to-populated
   `messagesChanged` transition emits an `.openingThread` tail request.
4. Feed the exact same snapshot and bodies again. The mapper returns equal
   rows and equal row IDs, message geometry is equal, and both message and
   render-row reducers return no new request.
5. Evaluate the still-current initial-load retry token against the production
   retry clock and authorization policy.

Focused command:

```bash
cd mobile/garyx-mobile
swift test --filter GaryxExistingThreadLoadingJitterReproTests
```

Observed deterministic failure:

```text
XCTAssertEqual failed:
("[16, 40, 140, 320, 650, 1000]") is not equal to ("[]")
EXPECTED FAILURE (#TASK-2630): equal captured rows still authorize
delayed scrollTo(bottom) writes after the loading row disappears

Executed 1 test, with 1 failure (0 unexpected)
```

The six integers are milliseconds after the immediate initial placement.
They are six additional position writes after loading has already changed to
stable historical rows.

## Root cause

The instability is not a `render_state` identity or window change. It is an
orphaned programmatic scroll retry chain:

1. Cold thread selection deliberately exposes zero rows while disk restore and
   bounded history refresh race
   (`GaryxMobileModel+ThreadLifecycle.swift`, `applySelectedThreadRouteProjection`,
   lines 225-237). The live graph and refresh are independent
   (`conversationRouteContentPreparationBegan`, lines 246-299).
2. When historical messages replace the loader,
   `GaryxMobileConversationViews.swift:434-448` forwards the empty-to-populated
   geometry transition to `messagesChanged`.
3. `GaryxConversationScrollPolicy.swift:328-340` classifies that transition as
   initial load and emits `.openingThread`.
4. `.openingThread` owns the long production retry clock
   `0, 16, 40, 140, 320, 650, 1000 ms`
   (`GaryxConversationScrollPolicy.swift:207-238`).
5. An equal subsequent snapshot correctly emits no content or row change, but
   it also has no cancellation path for the prior token.
   `GaryxConversationTailScrollScheduler` only invalidates a token when another
   request is scheduled (`GaryxConversationScrollPolicy.swift:645-693`).
6. Every delayed `.openingThread` attempt remains unconditionally authorized
   when the reader is not touching the scroll view
   (`GaryxConversationScrollPolicy.swift:504-528`).
7. The SwiftUI adapter executes each authorized attempt as
   `proxy.scrollTo(conversationBottomAnchorId, anchor: .bottom)`
   (`GaryxMobileConversationViews.swift:990-1027`).

Thus a no-op server frame does exactly the right thing for rows but cannot
settle the already-enqueued scroll mutations. Any late Markdown/image/tool-row
layout convergence changes where those repeated bottom-anchor writes land,
which surfaces as the reported post-loading row/offset wobble.

The testability-only refactor moves the existing retry millisecond array from
the private SwiftUI function to `TailScrollReason.retryDelayMilliseconds` in
Core. The App consumes that same array; timings and authorization behavior are
unchanged.

## Validation

- The focused reproduction was run twice. Both runs executed one test and
  failed on the identical `[16, 40, 140, 320, 650, 1000]` output.
- `GaryxConversationScrollStateTests`: 46 tests, 0 failures.
- `GaryxConversationRoutePresentationTests`: 15 tests, 0 failures.
- `GaryxMarkdownTableTranscriptReproTests`: 2 tests, 0 failures.
- The `GaryxMobile` App target compiled successfully for the repository's
  reference iPhone 17 Pro Max simulator on iOS 26.5.

## Hypotheses checked

| Candidate | Result |
| --- | --- |
| Equal `render_state` creates different outer row IDs | Rejected: both mapped row arrays and ID arrays are equal. |
| `based_on_seq` / `render_floor` shrinks then expands the row window | Rejected in this sequence: both passes use seq `116`, floor `103`, and the same row set. |
| Cached bodies and server refs replace the whole turn | Rejected: the second pass uses the same bodies and refs and is fully `Equatable`. |
| Loading-to-loaded loses position without a data change | Confirmed at the scroll-effect boundary: six delayed writes survive after both content reducers report a no-op. |

## Scope

No production behavior is fixed in this task. The failing assertion remains
marked `EXPECTED FAILURE (#TASK-2630)` for the follow-up implementation to turn
green with the same capture and sequence.
