# TASK-2644: iOS Existing-Thread Loading Flash Reproduction

Status: reproduced; root cause located; intentionally not fixed.

## Reported boundary

The affected sequence is:

1. an existing thread already has locally persisted historical messages;
2. opening the thread first shows the message-region loading skeleton;
3. while loading finishes, gray skeleton bars and real message text are
   visible together;
4. the skeleton disappears and the historical messages visibly flash or
   rebuild.

The required invariant is binary: a frame with zero local renderable rows may
show only the skeleton; a frame with local renderable rows must show only real
rows. Skeleton and real rows must never share one visible transcript viewport.

## Real captured input

The reproduction reuses the repository's sanitized real stream capture:

`mobile/garyx-mobile/Tests/GaryxMobileCoreTests/Fixtures/task-2610-markdown-table-frame.json`

It contains committed transcript seq `103...116` and the matching idle
server-owned `render_state`:

- `based_on_seq = 116`;
- `window.floor_seq = 103`;
- one historical `user_turn` row;
- resolved user, assistant-step, and assistant-final message bodies;
- `tailActivity = none`.

The test decodes the `thread_render_frame`, processes it through
`GatewayStreamFrameProcessor`, and round-trips the resulting committed bodies
and render snapshot through `GaryxCachedTranscript`. It then maps those inputs
with `GaryxMobileRenderStateMapper`.

This establishes a locally persisted existing thread with a real renderable
row. Mapping the same inputs twice returns equal rows and equal row IDs, and
none of the mapped message blocks has the row-level `.historySkeleton`
presentation.

## Deterministic state sequence

Test:

`GaryxExistingThreadLoadingFlashReproTests.testPersistedRowsMixWithLoadingSkeletonBeforeLiveReveal`

The test drives the production Core gates and observes the composition used by
the SwiftUI route:

| Route phase | Opening cover | Live transcript | Visible content kinds |
| --- | --- | --- | --- |
| `openingPage` | frozen `.loading` skeleton | not mounted | `[skeleton]` |
| `materializingConversation` | still visible | restored captured row mounted | `[real(row-id), skeleton]` |
| `live` | opacity becomes `0` | still mounted | `[real(row-id)]` |

The middle frame is the reported illegal mixed state. The transition from that
frame to `live` removes the skeleton compositor layer while leaving the real
row behind it, which is the loading-completion flash/reveal.

The focused test fails with both symptoms:

```text
XCTAssertEqual failed: ("loading") is not equal to ("localMessages")
EXPECTED FAILURE (#TASK-2644): a locally persisted existing thread is
frozen as loading because opening metadata only sees the empty in-memory
row store

XCTAssertEqual failed:
("[materializingConversation:[real(user_turn:origin:mobile-00000000-0000-0000-0000-000000000003),skeleton]]")
is not equal to ("[]")
EXPECTED FAILURE (#TASK-2644): loading must be all-skeleton only when
there are zero local rows, or all-real when local rows exist;
materialization currently composites both kinds in one viewport

Executed 1 test, with 2 failures (0 unexpected)
```

## Root cause and behavior chain

There are two coupled ownership gaps.

### 1. Persisted rows are absent from the frozen opening decision

`GaryxMobileModel+ThreadLifecycle.swift` calls
`cacheConversationOpeningMetadata` before opening the route. That method reads
only the current in-memory `GaryxConversationLiveStore` rows. On a cold open,
`applySelectedThreadRouteProjection` deliberately leaves the in-memory message
store empty and starts disk restoration asynchronously.

`GaryxConversationRouteMetadataCache.store` then freezes
`transcriptPresentation` from that zero in-memory row count. A valid persisted
`GaryxCachedTranscript` is not an input to the opening policy, so the route
keeps `.loading` metadata even after the persisted row has restored.

Relevant locations:

- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+ThreadLifecycle.swift`
  (`showSelectedThread`, `cacheConversationOpeningMetadata`,
  `applySelectedThreadRouteProjection`);
- `mobile/garyx-mobile/App/GaryxMobile/GaryxConversationRouteView.swift`
  (`GaryxConversationRouteMetadataCache.store`);
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxConversationRoutePresentation.swift`
  (`GaryxConversationOpeningTranscriptPolicy`).

### 2. Materialization intentionally shows both transparent layers

Core reports both of these as true during
`.materializingConversation`:

- `mountsLiveTranscript` because the phase is no longer `.openingPage`;
- `showsOpeningTranscriptCover` because the phase is not yet `.live`.

`GaryxMobileConversationViews.swift` consumes those gates in one `ZStack`:
the live transcript is the lower child, and
`GaryxConversationOpeningTranscriptView` is the upper child at opacity
`0.999`. The loading cover has no opaque backing; its scroll content starts
from `Color.clear`. Real transcript pixels therefore remain visible between
the skeleton bars during the entire materialization proof.

When the phase changes to `.live`, the same cover's opacity switches directly
to `0`. That handoff removes the overlaid skeleton and exposes only the
already-mounted real rows.

Relevant locations:

- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxConversationRoutePresentation.swift`
  (`mountsLiveTranscript`, `showsOpeningTranscriptCover`, `presentedFrame`);
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileConversationViews.swift`
  (`GaryxConversationView.body`, staged transcript ZStack and opacity);
- `mobile/garyx-mobile/App/GaryxMobile/GaryxConversationRouteView.swift`
  (`GaryxConversationOpeningTranscriptView.openingTranscript`);
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileConversationStatusViews.swift`
  (`GaryxThreadHistoryLoadingView`, the gray skeleton bars).

## Hypotheses checked

| Candidate | Result |
| --- | --- |
| Starting history refresh clears existing in-memory messages | Rejected for this path: `loadSelectedThreadHistory` sets the loading flag but does not clear a non-empty cache. |
| The same captured `render_state` remaps to new outer row IDs | Rejected: repeated mapping returns equal rows and equal IDs. |
| Unresolved render refs create row-level placeholders | Rejected for this capture: every mapped block is resolved and none is `.historySkeleton`. |
| Loading and real content coexist at the presentation boundary | Confirmed: materialization simultaneously mounts the live row and retains the transparent skeleton cover. |
| The previous delayed tail-scroll jitter is still present | Rejected: the existing TASK-2630 reproduction now passes on this baseline. |

## Testability boundary

The route phase, opening policy, captured rows, and row presentations are
Core-owned and tested directly. The last decision that combines those facts
into simultaneous visual layers still lives in SwiftUI
(`GaryxConversationView.body`).

The reproduction therefore includes a test-only `visibleFrame` adapter that
mirrors the two production gates above. A future implementation task should
make that final composition a pure Core value (for example, exactly one of
`loadingSkeleton` or `realRows`) and have SwiftUI render it. This task makes no
such product refactor because its scope is reproduction only.

## How to run

```bash
cd mobile/garyx-mobile
swift test --filter GaryxExistingThreadLoadingFlashReproTests
```

Expected result on the reproduced baseline: one test fails with the two
assertions shown above.

The prior, now-fixed scroll-jitter reproduction remains green:

```bash
swift test --filter GaryxExistingThreadLoadingJitterReproTests
```

## Scope

Only the failing SwiftPM reproduction and this evidence report are added.
Production logic is unchanged.
