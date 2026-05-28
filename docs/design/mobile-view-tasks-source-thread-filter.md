# Mobile View Tasks Source-Thread Filter Design

## Goal

Remove the rejected mobile "Promote to Task" flow and repurpose the thread
menu task action as "View Tasks". The new action shows tasks that were spawned
from the currently selected thread, using that thread id as
`source_thread_id`.

The semantic change is:

- Old: turn this thread into a task.
- New: show child tasks whose `source.thread_id` is this thread.

When a selected thread has no known child tasks, the menu remains visible as
`View Tasks (0)`. This keeps the action discoverable and still lets the user
open the filtered panel, where a fresh gateway query may find tasks missing from
the local cache.

## Code Map

- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxGatewayTaskModels.swift`
  - Add `GaryxTaskListFilter.sourceThreadId`.
  - Remove `GaryxTaskPromoteRequest`.
  - Add a small pure helper for source-thread filtering and the `View Tasks`
    menu title so SwiftPM tests can cover the behavior without UI inspection.
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxMobileTasksPanelState.swift`
  - Add `GaryxMobileTasksPanelState`, a pure value-type state machine owned by
    `GaryxMobileModel`.
  - State:
    - `sourceThreadFilterId: String?`
    - `sourceThreadFilteredTasks: [GaryxTaskSummary]`
    - `sourceThreadFilterLoadPhase: GaryxMobileLoadPhase`
  - Transitions:
    - `setSourceFilter(threadId:)`
    - `beginSourceFilterRefresh(threadId:)`
    - `applySourceFilterResult(threadId:tasks:)`
    - `applySourceFilterFailure(threadId:message:)`
    - `clearSourceFilter()`
    - `applyDeletion(taskId:)`
  - `applySourceFilterResult` and `applySourceFilterFailure` only mutate state
    when the response thread id still matches the active filter. Stale responses
    from a previous selected thread are dropped.
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxGatewayClient.swift`
  - Encode `source_thread_id` in `listTasks(filter:)`.
  - Remove `promoteTask(_:)`.
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel.swift`
  - Add `@Published var tasksPanelState = GaryxMobileTasksPanelState()`.
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+Presentation.swift`
  - Replace `selectedThreadTask` with `selectedThreadTasks`.
  - Add presentation helpers for visible task list, filtered subtitle/counts,
    and the menu title.
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+TasksDreamsAutomations.swift`
  - Remove `promoteSelectedThreadToTask()`.
  - Remove promote-only helpers:
    - `taskSummary(forThreadId:)`
    - `reconcileTaskForThread(_:)`
    - `isAlreadyTaskError(_:)`
  - Add `openSelectedThreadTasks()`, `refreshVisibleTasks()`,
    `refreshAllTasks()`, `refreshTasksForSourceThread(_:)`, and
    `clearTaskSourceThreadFilter()`.
  - Keep `tasks` as the all-tasks cache. Filtered panel results live in
    `tasksPanelState.sourceThreadFilteredTasks` so the global all-tasks counts
    and cache are not replaced by a filtered subset.
  - `refreshVisibleTasks()` is the single refresh entry point for tasks view
    and task mutation handlers:
    - Without a source-thread filter, call `refreshAllTasks()` to fetch only
      `GET /api/tasks` and update `tasks`.
    - With a source-thread filter, call `refreshAllTasks()` and then
      `refreshTasksForSourceThread(_:)` so both the all-tasks cache and the
      authoritative filtered list are current.
    - This avoids using broad `refreshRemoteState()` for task-only mutations.
  - Filtered fetches keep the existing `gatewayRuntimeGeneration` guard and
    also pass the requested source thread id through `GaryxMobileTasksPanelState`
    so stale responses are ignored when the user quickly switches threads.
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileConversationViews.swift`
  - Delete the "Promote to Task" branch.
  - Always show `View Tasks (N)` for selected threads.
  - Tap action calls `openSelectedThreadTasks()`.
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileTasksViews.swift`
  - Render `model.visibleTasks` instead of raw `model.tasks`.
  - For source-thread mode, show a compact source filter row with a clear
    action. Clearing returns the panel to the normal all-tasks list.
  - Refresh button calls `refreshVisibleTasks()` so filtered mode refetches
    `GET /api/tasks?source_thread_id=...`.
  - Filtered render states:
    - `loading`: show `GaryxLoadingPanelView(title: "Loading source tasks...")`
      below the source filter row.
    - `loaded` with rows: show the filtered list.
    - `loaded` with zero rows: show `GaryxEmptyPanelView` with title
      `No tasks dispatched from this thread.`
    - `failed`: show an empty/error panel with the load failure text and keep
      the refresh button available.
- `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileModel+Navigation.swift`
  - Opening the general tasks panel clears the source-thread filter.
  - Task deep links open the unfiltered panel before showing details.
- `mobile/garyx-mobile/README.md`
  - Remove the stale claim that mobile supports the promote helper.

No Rust backend files are in scope.

## UI State Machine

- `tasksPanelState.sourceThreadFilterId == nil`
  - Tasks panel shows the normal all-tasks cache in `tasks`.
  - Pull-to-refresh/top refresh calls `refreshVisibleTasks()`, which delegates
    to task-only `refreshAllTasks()`.
- `tasksPanelState.sourceThreadFilterId != nil`
  - Tasks panel shows `tasksPanelState.sourceThreadFilteredTasks`.
  - Entered only through `View Tasks (N)` on a thread menu.
  - The first transition opens the tasks panel, then sets the filter, then
    fetches `listTasks(filter: .init(sourceThreadId: threadId,
    includeDone: true, limit: 200))`. Opening first matters because generic
    `openPanel(.tasks)` clears source-thread filters.
  - The filter row's clear action resets the filter state and shows all tasks.
  - Mutating a task from the filtered panel keeps local lists coherent:
    deletes call `tasksPanelState.applyDeletion(taskId:)` and remove from
    `tasks`; status, title, assignment, and stop operations call
    `refreshVisibleTasks()`, which refreshes all tasks and then refetches the
    active source-thread filter.

The menu count comes from the all-tasks cache filtered by
`task.source.threadId`. It is intentionally a best-effort local badge; the
filtered panel fetch is authoritative.

When `refreshRemoteState()` completes while a source-thread filter is active,
the filtered list is left as-is rather than derived from the all-tasks cache.
The full refresh uses `limit: 120`, while the source-thread fetch uses
`limit: 200`; deriving from the smaller full list could under-count threads that
fan out heavily. The next task-panel refresh or task mutation refetches the
source-thread list authoritatively.

The initial implementation uses `limit: 200` for source-thread child tasks. That
matches the current non-paginated mobile task surface and assumes a single
thread dispatches fewer than 200 child tasks. Supporting more than 200 children
would require a follow-up load-more design for the tasks panel.

If the user opens `View Tasks` on a thread whose local badge says `(0)`, then a
filtered fetch returns rows, the panel shows those rows but the menu badge still
comes from the all-tasks cache. The filtered results are merged into the global
task cache model-side immediately after an accepted source-thread fetch result.
The merge is by `task.id`; fetched server rows replace existing rows with the
same id, newly discovered rows are inserted without purging unrelated global
rows, and deletions still rely on delete responses or the next full task refresh.

## Testing Strategy

SwiftPM tests in `mobile/garyx-mobile/Tests/GaryxMobileCoreTests`:

- `GaryxTaskListFilter` encodes `source_thread_id` into gateway query items.
- The pure source-thread helper returns only tasks whose `source.threadId`
  matches the requested thread id, and returns the full list without a filter.
- The menu title helper returns `View Tasks (N)`.
- `GaryxMobileTasksPanelState` covers:
  - setting and clearing the source-thread filter;
  - fetch success populating rows and setting phase to `loaded`;
  - fetch failure setting phase to `failed` without accepting stale rows;
  - stale fetch responses being dropped after switching filters;
  - deletion removing a task from the filtered list;
  - task merges replacing same-id rows, preserving unrelated rows, and adding
    newly discovered source-thread rows;
  - visible task selection returning all tasks without a filter and filtered
    rows with a filter.
- A small navigation-state helper covers the intended ordering: generic tasks
  navigation clears the filter, while the `View Tasks` flow opens the panel and
  then applies the source filter.

Source/build validation:

- `rg` checks confirm `Promote to Task`, `promoteSelectedThreadToTask`,
  `GaryxTaskPromoteRequest`, `promoteTask`, `reconcileTaskForThread`, and
  `isAlreadyTaskError` are gone from mobile app/core code.
- `cd mobile/garyx-mobile && swift test`
- `cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build`
