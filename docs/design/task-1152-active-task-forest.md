# Task 1152 Active Task Forest

> **Partially superseded (2026-07, #TASK-2099).** The
> `projectionCurrent`/warm-cache freshness concept described below no
> longer exists: task rows derive in the same transaction as every
> record write (#TASK-1864), so the projection is structurally current
> and there is no staleness flag or rebuild path.

## Goal

Change the Mac task forest from a complete task-history map into an active
operation room. The default forest should show only tasks that currently have
an active run plus the parent chain needed to understand where those active
tasks sit in the task tree.

This fixes the current failure mode where `/api/tasks/forest?include_done=true`
returns hundreds of rows and the desktop tries to lay out and render historical
tasks that are irrelevant to the current operation room. The layout recursion's
per-child visited-set copy is still removed as a defensive performance fix for
`scope=all` and malformed parent graphs.

## Active Chain Definition

A task is an active seed according to this priority:

- `recent_threads.run_state` is `running`, `streaming`, or `pending`, compared
  case-insensitively after trimming: active.
- `recent_threads.run_state` is `idle`, `completed`, `failed`, `error`, or
  `aborted`: inactive, even if `active_run_id` is stale and non-empty.
- `recent_threads.run_state` is null, empty, or unknown: fall back to whether
  `recent_threads.active_run_id` is present and non-empty.

This matches the existing forest client helper precedence and avoids repairing
stale `active_run_id` values in the read route.

The active forest is:

1. every active seed task;
2. every ancestor of each seed that is still present in the same filtered base
   set, following `task_projection.parent_task_number` or the legacy
   `source_task_id = '#TASK-' || parent.number` relationship until a root, a
   filter boundary, or a cycle is reached;
3. de-duplicated by task number with the existing latest-row policy.

The active forest does not include passive descendants of an active task unless
they are themselves active seeds or ancestors of another active seed. This keeps
the view focused on current work and avoids reintroducing large idle fan-outs.

Done and idle ancestors are included when needed to preserve the chain toward
root, as long as they match the same server-side filters as the seed. If no
active seed exists, the endpoint returns an empty task list and the desktop
shows a localized empty state with the English source key `No active tasks right
now.`. The product meaning is `当前没有活跃任务`.

## Gateway Contract

Extend the forest endpoint, not board/list task APIs:

```text
GET /api/tasks/forest?scope=active
GET /api/tasks/forest?scope=all
```

`scope=active` is the new desktop default. `scope=all` preserves the existing
complete forest for diagnostics and future manual exploration. If `scope` is
omitted, the gateway keeps `all` for wire compatibility, while the Mac app
explicitly requests `active`.

Response shape stays unchanged:

```ts
type TaskForestResponse = {
  tasks: TaskForestNode[];
  total: number;
  projectionCurrent: boolean;
};
```

For `scope=active`, `total` is the number of nodes returned in the active
forest, not the count of all historical tasks.

### Query Plan

Keep `TaskListFilter` and board/list filtering untouched. Add a forest-specific
scope enum around `GaryxDbService::list_task_forest`.

For active scope:

- build the same deduped base rows as the existing full forest query;
- determine active seeds from joined `recent_threads`;
- use a recursive CTE to walk each seed to its ancestors by task number and
  legacy `source_task_id` fallback within the filtered/deduped base rows;
- carry a path of task numbers to stop cycles;
- return only task numbers reached by the recursive CTE;
- use the existing row mapper so parent, run-state, and principal fields remain
  single-sourced.

Do not reuse `task_ancestor_summaries` directly for this endpoint because that
helper recurses over raw `task_projection`. The active forest must recurse over
the same filtered base rows used for seeds, so `source_bot_id`, status, and any
future forest filters apply to ancestors as well. If a parent is outside the
filtered base, the child becomes a root in that filtered forest view.

This keeps the filtering in SQLite where the projection truth already lives,
prevents cross-filter ancestor leakage, and prevents the renderer from receiving
all 706 rows in the default path.

## Desktop Contract

Add `scope?: 'active' | 'all'` to `ListTaskForestInput`. `gary-client.ts` maps it
to `scope`.

`TaskForestConsole` requests:

```ts
listTaskForest({
  includeDone: true,
  sourceBot,
  scope: "active",
})
```

The existing status chips remain a visual dimming affordance over the active
chain; they do not cause the server to remove ancestors. The bot filter still
applies server-side to seeds and ancestors, matching current Tasks behavior, and
chains may truncate at bot filter boundaries.

The empty state changes from `No tasks yet.` to
`No active tasks right now.` when the active scope returns no rows, using the
existing `t()` mechanism and English source text used by the Mac app.

Keep all forest active-run helpers aligned with the backend seed definition:
`runState` in `running`/`streaming`/`pending` is active; known idle or terminal
states are inactive even with a stale `activeRunId`; only null, empty, or
unknown run states use `activeRunId` as a fallback. This keeps node pulses,
active edges, minimap dots, sorting, and server scope semantics aligned.

## Entry Point

Make `TasksPanel` default to the `forest` view. This keeps the feature under
the existing Tasks information architecture while making the operation room one
click away from the sidebar. Board and List remain available in the segmented
control for historical task management.

## Layout Fix

Replace the current recursive collection pattern:

```ts
collect(child, new Set(seen))
```

with a global visiting set plus memoized per-node results:

- `visiting` catches cycles on the active recursion stack;
- `memo` stores each task number's subtree size and descendant status counts;
- each directed edge is evaluated once for the purpose of the subtree summary;
- cycles contribute zero additional descendants and do not block layout.

Placement should also avoid recursively placing nodes already on the current
path. This prevents a bad projection edge from hanging the UI even when the API
is switched back to `scope=all`.

Add focused renderer unit tests:

- a large deep chain completes quickly and lays out each task number once;
- a cycle does not recurse forever;
- hidden descendant counts stay correct at the depth cap.

The implementation target is linear in node plus edge count for the forest page.

## Rendering Defense

Keep the current viewport culling path:

- compute `worldViewport` from camera and stage size;
- call `visibleTaskForestNodeNumbers` with overscan above
  `CULLING_NODE_THRESHOLD`;
- render only visible nodes and visible-adjacent edges.

The active-scope default should make culling rarely necessary, but it remains
the safety net for `scope=all` and unusually large active chains.

## Validation Plan

Focused checks:

- `cargo test -p garyx-gateway --all-targets list_task_forest`
- `cd desktop/garyx-desktop && node --experimental-strip-types --test src/renderer/src/app-shell/components/task-forest-layout.test.mjs`
- `cd desktop/garyx-desktop && npm run build:ui`
- `cd desktop/garyx-desktop && npm run test:unit`

Runtime checks:

- with the real 706-task data, call
  `window.garyxDesktop.listTaskForest({ includeDone: true, scope: "active" })`
  over CDP and confirm it returns only active seeds plus ancestors;
- open the dev app on CDP port `39222`, switch to Tasks, confirm the forest is
  the default view and renders nodes without the loading hang;
- capture a screenshot showing the active-chain forest or the
  `No active tasks right now.` state;
- click a visible node and confirm the embedded thread panel opens the correct
  thread.

Packaging after code review:

- merge latest `main` into the worktree;
- run the desktop build checks again;
- run `npm run dist:dir` in `desktop/garyx-desktop`;
- replace the installed app through the repo's normal packaged-app flow before
  final handoff.
