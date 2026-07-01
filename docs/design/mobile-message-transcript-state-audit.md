# Mobile Message Transcript State Audit

Task: #TASK-1497. Status: **implemented**.

## Scope And Oracle

This audit checked the iOS Messages / chat transcript display seam against two
mobile rules:

- committed transcript structure must dumb-render server `render_state`;
- pure mobile route state, presentation mapping, formatting, and business-rule
  transformations must live in `GaryxMobileCore` with SwiftPM tests.

The oracle is source-of-truth state, not a screenshot: given a server
`thread_render_frame` / history response plus local pending-user overlays, the
visible transcript model must be derivable by pure Core helpers and directly
assertable under `Tests/GaryxMobileCoreTests`.

## Finding 1 - Selected-Thread Transcript Preparation

### Correct Semantics

Preparing selected-thread display state from a history window or stream window is
pure and deterministic. Given:

- fetched/cached transcript messages;
- current local messages for optimistic pending-user rows;
- the local run-tracker busy bit;
- the current active assistant message id;

the output is a pure value: reduced run state, thread-active flag, merged message
bodies, active-assistant marker, and message-list signature. The App target
should only gather those inputs, call Core, then apply values to `@Published`
state and persistence side effects.

### Diagnosis

The reviewed diagnosis found this selected-thread preparation still lived in
`App/GaryxMobile` and had no SwiftPM coverage. That matched the same root-cause
shape as the earlier mobile message-state parity work: non-UI message state
decisions escaped Core-only tests by living in the `@MainActor` App target.

This was not a `render_state` structural violation. Committed rows were already
mapped through `GaryxMobileRenderStateMapper`; the gap was layer ownership and
testability of the selected-thread prepared display state.

### Implementation

The pure preparation is now Core-owned in
`mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxMobileToolTraceBuilder.swift`:

- `GaryxMessageListSignature` computes the display-signature value at
  `:108-196`;
- `GaryxPreparedThreadMessages` merges remote/local messages and reconciles the
  active assistant marker at `:198-261`;
- `GaryxPreparedSelectedThreadTranscriptUpdate` reduces run state, computes
  `threadRunActive = localRunTrackerBusy || runState.busy`, maps transcript
  messages, and returns the prepared selected-thread value at `:263-329`.

The App target now stays as input collection plus application:

- `GaryxMobileModel+Messages.swift:35-63` routes ordinary message updates
  through Core signatures and routes active-assistant reconciliation through
  `GaryxPreparedThreadMessages.make(...)`;
- `GaryxMobileModel+Messages.swift:65-78` applies a prepared Core value to the
  per-thread message cache and selected-thread state;
- `GaryxMobileModel+Messages.swift:80-89`,
  `GaryxMobileModel.swift:61` / `:104-111`, and
  `GaryxMobileModel+Navigation.swift:708-710` use the Core
  `GaryxMessageListSignature` value;
- stream-window flush gathers inputs and calls the Core helper from
  `GaryxMobileModel+ThreadStream.swift:324-338`;
- history loading gathers inputs from
  `GaryxMobileModel+Threads.swift:1201-1215`, and
  `GaryxMobileModel+Threads.swift:1173-1199` applies the prepared result.

SwiftPM coverage was added in
`mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxPreparedSelectedThreadTranscriptUpdateTests.swift`:

- active assistant id clear/preserve behavior: `:5-41`;
- remote/local merge and optimistic-user preservation: `:43-71`;
- selected-thread `localRunTrackerBusy || runState.busy` behavior: `:73-122`;
- cached-window preparation and older local row preservation: `:124-146`;
- message-list signature changes for text, status, attachments, long-text
  sampling, and tool-trace status: `:148-181`.

## Checked Areas That Already Matched The Contract

### Render Rows

`selectedThreadTurnRows()` in
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+Messages.swift:20-33`
delegates row production to `GaryxMobileRenderStateMapper.rows(...)`. The mapper
in `Sources/GaryxMobileCore/GaryxMobileRenderState.swift:725-760` maps server
`render_state.rows` and appends only local optimistic user rows.

That optimistic overlay is allowed by
`docs/agents/repository-contracts.md:53-56` as pending-ack chrome, while
committed rows, tail activity, and active tool group stay server-owned.

SwiftPM coverage includes:

- server field decode and render snapshot mapping:
  `Tests/GaryxMobileCoreTests/GaryxMobileRenderStateMapperTests.swift:4-68`;
- tool groups mapped from server rows without local grouping:
  `GaryxMobileRenderStateMapperTests.swift:314-372`;
- optimistic user-row append/dedupe behavior:
  `GaryxMobileRenderStateMapperTests.swift:422-467` and `:581-625`.

### Loading / Initial History State

`isSelectedThreadAwaitingInitialHistory` in
`App/GaryxMobile/GaryxMobileModel+Presentation.swift:252-265` delegates to the
Core pure function
`GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(...)`
(`Sources/GaryxMobileCore/GaryxMobileRenderState.swift:667-723`). The settle
behavior is covered by
`Tests/GaryxMobileCoreTests/GaryxSelectedThreadHistoryPresentationTests.swift:58-87`.

### Transcript Cache And Sync Decisions

The cache file is side-effect glue. Its header states the merge/cursor logic
lives in Core (`App/GaryxMobile/GaryxMobileModel+TranscriptCache.swift:3-7`).
The side-effecting path calls Core planners:

- `GaryxTranscriptCacheLogic.merged(...)` at
  `GaryxMobileModel+TranscriptCache.swift:76-87`;
- `GaryxTranscriptFetchPlanner.pageAction(...)` at `:148-153`.

The corresponding pure logic lives in:

- `Sources/GaryxMobileCore/GaryxTranscriptCache.swift:67-138`;
- `Sources/GaryxMobileCore/GaryxTranscriptSyncPlanner.swift:21-39`.

SwiftPM coverage exists in
`Tests/GaryxMobileCoreTests/GaryxTranscriptCacheTests.swift` and
`Tests/GaryxMobileCoreTests/GaryxTranscriptSyncPlannerTests.swift`.

### Tail Thinking And Rate-Limit Presentation

`showsTailThinkingIndicator` and `selectedThreadRateLimit` in
`App/GaryxMobile/GaryxMobileModel+Presentation.swift:304-315` read the selected
thread's `render_state` directly. Rate-limit text/countdown formatting is a Core
pure model at
`Sources/GaryxMobileCore/GaryxMobileRenderState.swift:1058-1159`, with SwiftPM
tests in `Tests/GaryxMobileCoreTests/GaryxRateLimitBannerModelTests.swift`.

### Text Selection And In-Place Action Menu

`GaryxMessageTextSelectionViews.swift` is platform adapter UI around
`UITextView` (`:32-68`), not transcript state derivation. The in-place action
menu in `GaryxMessageActionMenu.swift` owns gesture/anchor/menu layout only
(`:52-159`) and does not classify transcript rows or derive server state.

## Validation

- `swift test --filter GaryxPreparedSelectedThreadTranscriptUpdateTests`:
  7 tests, 0 failures.
- `swift test`: 612 tests, 0 failures.
- `xcodebuild -project GaryxMobile.xcodeproj -scheme GaryxMobile -sdk iphonesimulator -configuration Debug build`:
  succeeded.

## Conclusion

Audit result: **implemented**. One real state-management gap was found and
fixed: selected-thread transcript preparation and message signatures now live in
`GaryxMobileCore` and are covered by SwiftPM tests. The render-state red line
was preserved: no local user-turn grouping, tool grouping, tail thinking,
active-tool, or final-answer derivation was added.
