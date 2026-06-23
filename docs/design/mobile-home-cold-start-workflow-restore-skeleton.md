# Mobile Home Cold-Start Restore And Skeleton Loading

## Scope

Fix two iOS home-list cold-start issues:

- Bug 2: a cold launch can restore the workflow-run conversation surface and show
  `Workflow run` instead of landing on the home list.
- Bug 1: first empty home-list load renders a single spinner row instead of a
  multi-row skeleton placeholder.

The state derivation belongs in `GaryxMobileCore`; SwiftUI should only render
the snapshot.

## Reproduction Evidence

### Bug 2 Current Sequence From Simulator Capture

Verified from current code:

- `restoreLastOpenedThreadIfNeeded()` runs after gateway connect and before
  `refreshThreads()` in `GaryxMobileModel+Gateway.swift`.
- `showSelectedThread(_:)` persists `thread.id` into the ordinary
  `lastOpenedThreadId` slot for non-excluded chat threads.
- `showWorkflowRun(...)` clears `selectedThread`, sets `draftThreadTitle` to
  `thread?.title ?? "Workflow run"`, and currently calls
  `persistLastOpenedThreadId(workflowRunId)`.
- `persistLastSessionLocation()` only writes the session-location flag when
  called from scene phase handling. It computes true only when content is
  presented, panel is `.chat`, and `selectedThread != nil`.
- `openThread(id:)` resolves `thread_type == "workflow_run"` through
  `GaryxWorkflowRunDestination` and opens the workflow-run panel.

Real capture source:

- Simulator: `Garyx Mobile UI QA`
  (`1317C3D8-106D-429F-9B65-4AD7E73E10EB`), iOS 26.2.
- App build: `XcodeBuildMCP build_run_sim`, `CODE_SIGNING_ALLOWED=NO`,
  bundle id `com.garyx.mobile`.
- Gateway: local gateway `http://127.0.0.1:31337`, app scope
  `gateway::711d009cbb773339`.
- Captured fixture:
  `mobile/garyx-mobile/Tests/Fixtures/mobile-cold-start-workflow-restore-capture.json`.
- The raw simulator plist/catalog and full gateway task source were not
  committed because they contain local/private app state. The committed fixture
  is mechanically derived from the capture and keeps only public-safe fields:
  synthetic test thread summaries, empty transcript counts, workflow event
  types/counts, and the app-written defaults state sequence.

Capture commands / flow:

1. Created a synthetic chat thread through the real gateway:
   `garyx thread create --title "Test Chat Before Workflow Restore" --workspace-dir /Users/test/project --json`.
2. Created a synthetic workflow-backed task/run through the real gateway:
   `garyx task create --title "Test Workflow Run Capture" --body "Synthetic task for mobile cold-start capture." --workflow thread-unify-smoke-20260603152639 --input "Test workflow run for mobile cold-start capture" --workspace-dir /Users/test/project --notify none --json`.
3. Clean-installed the app on the simulator, launched it, opened the chat via
   `xcrun simctl openurl <sim> garyx://mobile/thread?threadId=<chat>`.
4. Pressed the simulator Home button through XcodeBuildMCP to trigger the real
   scene-phase background persistence.
5. Opened the workflow-run thread via
   `xcrun simctl openurl <sim> garyx://mobile/thread?threadId=<workflow-run>`.
6. Cold-stopped and relaunched the app through XcodeBuildMCP and captured the
   restored UI state.

Captured trigger:

1. Open a normal chat thread. The app persists `lastOpenedThreadId =
   <captured chat thread id>`; after the real Home button background,
   `lastSessionOnThread = true`.
2. Open a workflow-backed thread/run. Current `showWorkflowRun` writes
   `lastOpenedThreadId = <captured workflow thread id>` and leaves
   `lastSessionOnThread = true`.
3. Cold launch with that mismatched persisted pair.
4. `restoreLastOpenedThreadIfNeeded()` sees `lastSessionOnThread = true` and
   `lastOpenedThreadId = <captured workflow thread id>`, calls
   `openThread(id:)`, and the workflow destination opens the workflow-run panel.
   The XcodeBuildMCP UI snapshot matched the workflow title
   `Thread Unify Smoke 20260603152639` after cold launch.

Captured control path:

- A separate clean simulator sequence opened the workflow-run thread directly,
  then pressed Home. The app wrote `lastSessionOnThread = false`. A cold launch
  from that state kept the false flag. The bug is therefore the stale flag plus
  polluted id pair, not simply "background from workflow run always restores
  workflow run."

### RED Tests Added Before Production Fix

- `swift test --package-path mobile/garyx-mobile --filter GaryxLastOpenedThreadRestorationPolicyTests`
  currently fails because `GaryxLastOpenedThreadRestorationPolicy` and
  `GaryxHomeThreadListSnapshot.recentPlaceholder` do not exist yet.
- `GaryxLastOpenedThreadRestorationPolicyTests` uses the simulator-captured
  fixture above to derive the workflow-run destination and assert the captured
  bad state sequence.
- `GaryxLastOpenedWorkflowRestoreTests` is an App-state no-UI regression test
  that opens a chat thread, persists the session flag, then opens the captured
  workflow thread and asserts the workflow run does not overwrite the persisted
  chat slot and marks launch restore non-restorable.
- Implementation must also add an App-level regression for the already-polluted
  old-default case: `lastSessionOnThread = true` and `lastOpenedThreadId =
  <captured workflow id>` must stay on home, clear or invalidate the polluted
  restore state, and avoid the transient resolving workflow panel during
  cold-launch restore.

## Bug 2 Design

Add a Core policy type, tentatively
`GaryxLastOpenedThreadRestorationPolicy`, responsible for the semantics of
"which opened destination is restorable on cold launch":

- `persistedThreadId(afterOpening:previousThreadId:)`
  - `.chat(threadId)` returns the normalized chat thread id.
  - `.workflowRun` and `.unresolved` keep the previous thread id and never write
    the workflow id into the ordinary thread slot.
- `isSessionRestorableAfterOpening(_:)`
  - true for `.chat`.
  - false for `.workflowRun` and `.unresolved`.
- `isCurrentSessionRestorable(...)`
  - takes the current live navigation facts currently used by
    `persistLastSessionLocation()` (`navigationState`, `activePanel`,
    `selectedThreadId`, and active workflow-run id).
  - true only for a selected chat conversation.
  - false for workflow-run, unresolved, home/sidebar, and other panel states.
- `restoreThreadId(...)`
  - applies the existing restore guards: no selected thread, no pending route,
    no pending thread intent, `.chat` active panel, sidebar closed, persisted
    session was on a thread, non-empty persisted id.
  - when the destination is already resolved, returns nil for `.workflowRun` and
    `.unresolved`.

Wire App code to the policy:

- `showSelectedThread(_:)` remains the only normal path that writes a chat id
  into `lastOpenedThreadId`.
- `showWorkflowRun(...)` stops writing `workflowRunId` into
  `lastOpenedThreadId` and immediately persists `lastSessionOnThread = false`.
  This handles process termination before scene-phase persistence.
- `persistLastSessionLocation()` delegates the current-route decision to the
  Core policy instead of duplicating route semantics locally. This keeps
  scene-phase persistence and explicit workflow-run opening on the same
  restorable/non-restorable contract.
- `restoreLastOpenedThreadIfNeeded()` should use a restore-specific open path
  that never opens workflow destinations:
  - if the summary is already in memory and resolves to `.workflowRun`, clear the
    polluted slot / mark session non-restorable and stay on home.
  - if it must refresh/fetch the thread first, still evaluate the resolved
    destination before opening; only `.chat` may call the existing selected
    thread path.
  - do not show the transient resolving workflow surface during cold-launch
    restore; home remains visible until a chat thread is confirmed restorable.

This is structural rather than a timing guard: workflow runs are represented as
non-restorable destinations at the route policy boundary.

## Bug 1 Design

Add a Core-derived placeholder to `GaryxHomeThreadListSnapshot`:

```swift
enum GaryxHomeRecentPlaceholder: Equatable, Sendable {
    case none
    case loadingSkeleton(rowCount: Int)
    case empty
}
```

Derivation:

- `.loadingSkeleton(rowCount: 6)` when `isLoadingThreads == true` and
  `sections.recent.isEmpty`.
- `.empty` when not loading and `sections.recent.isEmpty`.
- `.none` when recent rows exist, including cached rows during refresh.

`recentPlaceholder` should be a computed property derived from
`sections.recent.isEmpty` and `isLoadingThreads`, not stored snapshot state. The
row count lives in Core so tests can assert first empty load behavior without
rendering SwiftUI. Six rows fill the first phone viewport without looking like a
single spinner status line, while staying short enough not to imply exact result
count.

SwiftUI changes:

- Replace `GaryxSidebarLoadingRow(title:)` in the Recent section with
  `GaryxSidebarSkeletonRows(rowCount:)` driven by
  `snapshot.recentPlaceholder`.
- Keep existing real `GaryxHomeThreadButton` rows unchanged.
- Skeleton visual shape should match the existing thread row density: circular
  avatar block, title line, subtitle line, optional short timestamp line, using
  the same soft shimmer treatment already used by transcript loading skeletons.
- Accessibility should expose one grouped "Loading recent threads" element and
  hide individual decorative skeleton bars.

## Impact And Risks

- Deep links, task taps, and explicit workflow-run opens still route to workflow
  runs because the normal `openThread(id:)` path keeps allowing workflow
  destinations. Only cold-launch last-opened restore uses the stricter policy.
- Existing polluted defaults are handled by refusing workflow destinations at
  restore time and clearing the polluted state.
- Chat thread cold-start restore remains intact for summaries resolving to
  `.chat`.
- Cached recent rows suppress skeletons during refresh, so the list does not
  flash placeholders over usable cached content.

## Validation Plan

Before implementation:

- RED: `swift test --package-path mobile/garyx-mobile --filter GaryxLastOpenedThreadRestorationPolicyTests`.
- RED data source check: the failing test decodes
  `Tests/Fixtures/mobile-cold-start-workflow-restore-capture.json` and asserts
  the captured simulator sequence before it asserts the missing policy.

After implementation:

- GREEN: same SwiftPM filter.
- GREEN: full `swift test --package-path mobile/garyx-mobile`.
- GREEN: targeted App-state test through Xcode after adding the new App test to
  the test target.
- GREEN: iOS app-target `xcodebuild ... build` with simulator destination;
  `CODE_SIGNING_ALLOWED=NO` only if signing is the only blocker.
- Simulator end-to-end smoke:
  - With captured polluted defaults (`lastSessionOnThread=true`,
    `lastOpenedThreadId=<captured workflow id>`), cold launch must stay on the
    home list and must not flash the workflow resolving/panel surface.
  - Clean first load with no cached recent rows must show the multi-row skeleton
    placeholder before real rows arrive.
- Real-device end-to-end smoke on the boss's iPhone after signing/device access
  is available.

## Device Probe

Initial required probe was run before implementation:

- `xcrun devicectl list devices`:
  - The connected physical iPhone entry was present but `unavailable`.
- `xcrun xctrace list devices`:
  - The same physical iPhone was listed offline on iOS 26.5.
  - Available simulator used for capture: `Garyx Mobile UI QA Simulator (26.2)`
    `1317C3D8-106D-429F-9B65-4AD7E73E10EB`.
- Signing probe from `xcodebuild -showBuildSettings`:
  - `CODE_SIGN_STYLE = Automatic`
  - `CODE_SIGN_IDENTITY = iPhone Developer`
  - `PRODUCT_BUNDLE_IDENTIFIER = com.garyx.mobile`
  - `PROVISIONING_PROFILE_REQUIRED = YES`
  - no `DEVELOPMENT_TEAM` value was present in the printed build settings.

Current blocker for the real-device smoke: the iPhone is offline/unavailable,
and provisioning cannot be confirmed without the device online/trusted and a
usable development team/profile. The exact physical device name and UDID were
reported in the task thread but intentionally not committed to this public repo.
Do not claim real-device pass until the device is online and install succeeds.
