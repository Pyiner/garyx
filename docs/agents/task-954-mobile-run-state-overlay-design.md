# TASK-954 Mobile Thread List Run State Overlay Design

## Goal

Restore the mobile thread-list typing badge for threads that are running
according to the gateway recent-thread projection, even when the mobile app has
not opened or subscribed to that thread's committed transcript stream.

## Root Cause

The gateway recent-thread projection can return `run_state="running"` and an
active run id for background threads. The sidebar row presentation correctly
treats `runState == "running"` as running.

`GaryxMobileModel.summaryWithCommittedRunState` incorrectly treats a missing
local committed transcript run state as evidence that the thread is idle. When
`runStateByThread[thread.id]` is absent, it clears `activeRunId` and rewrites a
gateway `running` state to either `idle` or `completed`. Threads that have not
been opened in the current mobile session usually have no local committed run
state, so their list-row running state is erased before presentation.

## Data Contract

- The gateway recent-thread summary is the authoritative fallback for list
  rows.
- A local committed transcript run state is a more precise overlay only when it
  exists for that thread.
- Missing local committed state means "no local overlay", not "not running".
- Conversation tail-thinking and tool activity continue to come from
  `render_state.tailActivity`; this change does not touch transcript rendering.

## Implementation Plan

1. Add a pure Core helper that resolves a thread summary run state from:
   - API summary `runState` / `recentRunId`;
   - optional local `GaryxTranscriptRunState`.
2. When the local committed state is absent, return the API run state unchanged.
   Do not treat absence as idle, and do not make active-run metadata part of the
   new Core contract.
3. When the local committed state is present:
   - `busy == true` resolves to `running`;
   - a non-empty terminal status resolves to that terminal status;
   - otherwise resolve to `completed` when a recent run id exists, or `idle`
     when no recent run id exists.
4. Call the Core helper from `summaryWithCommittedRunState` / `summary(applying:)`
   so list refreshes, pagination, workspace/bot thread refreshes, selected
   thread summaries, and background reconcile candidate selection share the same
   rule.
5. Keep active-run metadata handling outside the helper: the absent-committed
   branch preserves the gateway summary's top-level `activeRunId`, while both
   summary wrappers keep the existing nested `threadRuntime.activeRun = nil`
   normalization.

## Test-First Plan

1. Add the Core resolver with the current app rule, then add a SwiftPM Core
   test for `apiRunState="running"` with no committed state. It should fail by
   returning `idle` or `completed`, reproducing the bug in pure code.
2. Fix the helper and wire the app code to it; the red test turns green.
3. Add reverse coverage for committed terminal states to preserve selected-thread
   convergence after a run ends.
4. Add non-running API coverage for `completed`, `idle`, and `nil`, proving the
   fallback does not invent `running`.

## Validation

- `cd mobile/garyx-mobile && swift test`
- `cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build`
- Simulator/manual verification with a running background thread: the sidebar
  row avatar shows `GaryxAvatarTypingBadge`.

## Risk

Risk is low because the change is limited to the summary run-state overlay used
by thread-list projections. The intended behavior changes only for threads with
no local committed transcript state: they now preserve the gateway projection.
Threads with local committed state keep the existing more precise convergence
behavior.
