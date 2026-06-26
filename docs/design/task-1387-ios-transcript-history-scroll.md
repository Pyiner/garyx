# TASK-1387 iOS Transcript History Scroll Stability

## Scope

This is an iOS-only fix for chat transcript history expansion in
`mobile/garyx-mobile`. It does not change desktop behavior, the gateway event
stream contract, or server-side render-state derivation.

The invariant stays unchanged:

- SSE `events` remain gapless by `after_seq`.
- `render_state.rows` may be narrowed by a render floor.
- `render_state.based_on_seq` remains the committed tail.
- iOS dumb-renders server `render_state.rows`; it does not regroup turns, pair
  tools, place final answers, or derive tail thinking.

## Reproduction

The red SwiftPM tests added for this task cover the current unstable paths:

- `GaryxTranscriptSyncPlannerTests.testCapturedOneTurnInitialWindowRequiresLargerMobileDefault`
  decodes a scrubbed structural capture of the live local
  `thread_render_frame` shape. With `initial_user_turns=1`, the first frames had
  one visible `user_turn` row and `window.has_more_above=true`, which means the
  top boundary is immediately materialized.
- `GaryxConversationScrollStateTests.testCapturedSmallWindowDoesNotAutoPrefetchAfterLightScroll`
  shows that a barely scrollable one-turn window can pass the current automatic
  prefetch gate after a small user scroll.
- `GaryxConversationScrollStateTests.testTopRowAppearanceStillHonorsDistanceFromLoadedHistoryStart`
  shows that top-row `onAppear` currently bypasses the distance gate with
  `ignoreDistance=true`.
- `GaryxHistoryPaginationPlannerTests` intentionally fails to compile because
  stable history pagination is still app-local. The missing Core seam is the bug
  boundary for `hasMoreHistoryBefore` flicker and cache-hit idempotence.
- `GaryxConversationScrollStateTests.testRenderRowPrependPreservesScrollWhenMessagesDidNotChange`
  drives two server render snapshots through `GaryxMobileRenderStateMapper`.
  The transcript cache rows are unchanged, but the render row ids prepend when
  the floor drops, proving `onChange(messages)` alone cannot observe the
  visible insertion.
- Existing window/prefetch tests have been updated to the same intended
  contract, so the suite no longer has contradictory old expectations for
  `initial_user_turns=1` or `ignoreDistance=true`.

No raw local thread id, message text, personal path, token, or user data is
committed. The capture in tests keeps only public-safe frame structure.

## Design

### 1. Make the cold window large enough

Change `GaryxThreadWindowPlanner.initialUserTurns` from `1` to `3`.

Reasoning:

- The existing HTTP open path already uses `threadHistoryUserQueryLimit = 3`.
- The current one-turn SSE cold window is the only intentionally tiny window.
- This is the narrowest change because `initial_user_turns` is client-declared
  in `GaryxThreadWindowPlanner`; the gateway already accepts the parameter.
- Floor/gapless semantics are unchanged. The client asks for a larger initial
  row window, then reconnects with the server-provided floor exactly as before.
- The existing
  `GaryxTranscriptSyncPlannerTests.testThreadWindowPlannerColdReconnectScrollUpSequence`
  assertion changes from `1` to `3`; gateway tests that explicitly pass
  `initial_user_turns=1` as a fixture are not changed.

### 2. Stop treating top-row appearance as permission to fetch

Keep top-row `onAppear` as a hint, but route it through the same scroll-distance
policy as metrics changes. Remove the `ignoreDistance` parameter from automatic
prefetch entirely:

- `GaryxLoadEarlierHistoryButton.onAppear`
- `GaryxMobileTurnRowsView.onNearHistoryBoundary`

Tighten `GaryxConversationScrollState.shouldPrefetchOlderHistory` so automatic
prefetch has this exact signature and requires every gate below:

```swift
shouldPrefetchOlderHistory(
    hasMoreHistoryBefore: Bool,
    isLoadingOlderHistory: Bool,
    hasPendingPrefetch: Bool
) -> Bool
```

- `hasMoreHistoryBefore`
- not already loading
- no pending prefetch
- a real user move toward older history
- measured scrollable content larger than a tiny cold window: compute
  `contentHeight = contentBottomOffset - contentTopOffset` and require
  `contentHeight - viewportHeight >= max(640, viewportHeight)`
- proximity to the loaded history start
  (`contentTopOffset >= -max(640, viewportHeight * 1.5)`)

The button tap remains the explicit manual path and can call
`loadOlderSelectedThreadHistory()` directly.

Expected policy examples with an 800pt viewport:

- `contentTopOffset=-80`, `contentBottomOffset=980` is rejected even though it
  is near the loaded start, because overflow is only 260pt.
- `contentTopOffset=-2000`, `contentBottomOffset=2600` is rejected because it is
  not near the loaded start.
- `contentTopOffset=-400`, `contentBottomOffset=2600` is accepted after a real
  user move, because overflow is 2200pt and the loaded start is within the
  1200pt prefetch band.

This intentionally updates the old
`testHistoryPrefetchRequiresMovementAndProximity` case that asserted
`top=-2000` could still prefetch through `ignoreDistance=true`. That assertion
encoded the deleted bypass behavior and must become false under the new
contract. The positive automatic prefetch example is now the `-400/2600/800`
case above.

### 3. Move history pagination truth into Core

Add a Core planner:

- `GaryxHistoryPaginationState`
- `GaryxHistoryPaginationPage`
- `GaryxHistoryPaginationPlanner`

It has two inputs:

- HTTP/cache pages, which are authoritative for the cached older boundary.
- render-window snapshots, which can seed or lower the render floor but must not
  clear a cached older boundary just because one transient frame says
  `has_more_above=false`.

Rules:

- A render window with `has_more_above=true` and `floor_seq > 1` yields
  `hasMoreBefore=true`, `nextBeforeIndex=floor_seq - 1`.
- A render window with `has_more_above=false` clears pagination only when the
  cached page state is present and also says there is no older boundary. Missing
  cache truth preserves the current boundary instead of treating "unknown" as
  "no older page".
- The clear path is tested: when both render window and cached state say no
  older boundary, `hasMoreBefore=false` and `nextBeforeIndex=nil`, so the
  "Load earlier" affordance disappears at the true top.
- A cached older page with the same `nextBeforeIndex` is idempotent, so
  automatic triggers do not repeatedly request the same page after a cache hit
  or duplicate server response.

Wire this planner into:

- `applyRenderWindowPagination`
- `updateSelectedThreadHistoryPagination`

The planner owns only `hasMoreBefore` and `nextBeforeIndex`. It does not own
`selectedThreadRenderFloorByThread`, `render_floor` requests, event cursors, or
`based_on_seq`; those remain in the existing stream/history code paths.

### 4. Preserve scroll for render-row prepends, not only message prepends

Older-page flow currently prepends bodies, lowers `render_floor`, then restarts
SSE. The visible row insertion may happen on the later render snapshot, while
`onChange(of: model.messages)` already fired. Add a view-level row-id change
hook backed by a Core method:

- track `selectedThreadTurnRows().map(\.id)`
- call
  `GaryxConversationScrollState.renderRowsChanged(previousIds:currentIds:threadUnchanged:hasTailContent:)`
- the Core method uses the existing prepend detector and feeds
  `contentChanged(isHistoryPrepend: true)` when prior rows moved down

This does not recompute grouping. It only compares server row ids already
produced by `GaryxMobileRenderStateMapper`.

## Files

- `Sources/GaryxMobileCore/GaryxTranscriptSyncPlanner.swift`
- `Sources/GaryxMobileCore/GaryxConversationScrollPolicy.swift`
- `App/GaryxMobile/GaryxMobileConversationViews.swift`
- `App/GaryxMobile/GaryxMobileModel+ThreadStream.swift`
- `App/GaryxMobile/GaryxMobileModel+Threads.swift`
- focused tests under `Tests/GaryxMobileCoreTests`

No new app-target source file is planned. If implementation needs one, run
`xcodegen generate` and commit the project change.

## Validation

Required before code review:

- focused red tests turn green
- `swift test --package-path mobile/garyx-mobile`
- `xcodebuild -project mobile/garyx-mobile/GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build CODE_SIGNING_ALLOWED=NO`

The code-review task must reproduce before/after behavior and confirm:

- no automatic load on cold open or light scroll
- stable `hasMoreHistoryBefore`
- idempotent cache/page handling
- prepend keeps reading position stable
- floor/gapless contract remains intact
