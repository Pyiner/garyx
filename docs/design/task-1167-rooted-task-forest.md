# Task 1167 Pinned-Thread Task Forest

## Goal

Change the Mac task forest into a pinned-thread operation room. The forest root
set is the user's current pinned threads. Pinning a thread adds it to the
operation room; unpinning it removes that root. For each pinned thread that has
a task projection row, the gateway returns that task plus all recursively
derived child tasks. Unrelated tasks are not returned.

Keep the existing canvas, tidy-tree layout, cycle defense, node cards, node
selection, thread detail panel, panning, zooming, minimap, and visual styling.
This change only replaces the task set selection contract and wires the forest
to the existing pin/unpin product model.

## Product Contract

The forest no longer has a global, active-chain, or manually searched root.

- Root set: `thread_pins` in display order. The existing pin/unpin actions are
  the only v1 root management UI.
- Pinned task thread: render that task as a root and recursively render its
  descendants.
- Pinned non-task chat thread: skip it in the forest and show a small
  explanatory count in the empty/summary state when relevant.
- Empty state: when no pinned threads exist, show `Pin threads to add them to
  the operation room.`
- No task roots: when pinned threads exist but none has a task projection row,
  show `Pinned threads with tasks will appear here.`
- Pin changes: after a thread is pinned or unpinned, desktop state already
  updates `pinnedThreadIds`; the mounted forest observes that state and reloads.
- Search selector and node drill-down are not part of the v1 root contract.
  The command palette may continue to search visible nodes, but it must not own
  root selection.

Skipping non-task pinned chats keeps the visual language honest: task nodes are
task cards with status, assignee/runtime, parent edge, and task id semantics. A
synthetic chat-only root would require a second node model and extra thread-only
edge rules, which would rebuild the forest instead of reusing the existing task
projection. Users can still open pinned chat threads from the existing pinned
thread sidebar.

## Gateway Contract

Extend the existing read endpoint behavior while keeping the existing task node
shape and adding root metadata:

```text
GET /api/tasks/forest
GET /api/tasks/forest?scope=all
```

Response shape remains:

```ts
type TaskForestResponse = {
  tasks: TaskForestNode[];
  total: number;
  projectionCurrent: boolean;
  rootThreadIds: string[];
  skippedPinnedThreadIds: string[];
};
```

Rules:

- Default forest (`scope` omitted): read `thread_pins` from the gateway DB,
  treat those thread ids as the root set, and return the union of each root
  task's recursive subtree.
- Diagnostic forest (`scope=all`): keep the existing full forest behavior for
  diagnostics only.
- Remove `scope=active` from the Mac app and do not keep active-chain selection
  as a product path.
- `TaskForestScope::default()` becomes pinned. Existing desktop is the only
  product caller of `/api/tasks/forest`; `scope=all` remains available for
  explicit diagnostics.
- Run the existing task projection backfill check before reading, exactly like
  the current endpoint.
- `rootThreadIds` contains pinned threads that had a task projection row and
  seeded a subtree. `skippedPinnedThreadIds` contains pinned threads with no
  task projection row in raw, unfiltered projection data. A pinned task thread
  filtered out by `source_bot_id` or status is not mislabeled as a non-task
  chat.

### Query Plan

Use `task_projection` as the only task ancestry source.

For the pinned default:

- load pinned threads from `thread_pins`, preserving the existing
  `pinned_at DESC, thread_id ASC` order;
- build a filtered raw task set using the existing `TaskListFilter` so
  `source_bot_id`, `status`, and `include_done` remain server-side filters;
- seed the recursive CTE from every pinned thread that exists in that filtered
  raw task set;
- guarantee explicit pinned seed rows win over the number dedup policy, so a
  pinned task thread does not disappear if another projection row has the same
  task number and a newer timestamp;
- recurse parent to child by `source_task_thread_id`,
  `parent_task_number`, or legacy `source_task_id = '#TASK-' || parent.number`;
- carry a thread-id path and bounded depth to prevent cycles;
- join `thread_meta` and `recent_threads` for the same parent/run-state fields
  already returned by `TaskForestNode`;
- de-duplicate overlapping subtrees by thread id, preserving first reached root
  order;
- return rows ordered by pinned-root order, tree depth, sibling task number, and
  thread id so each pinned subtree is stable.

If a parent is filtered out by `source_bot_id` or status, descendants below it
are also outside the returned forest. The desktop default uses `includeDone:
true` and no status filter; status chips remain local dimming only.

## Desktop Contract

Extend shared desktop forest page data:

```ts
type DesktopTaskForestScope = "all";

type DesktopTaskForestPage = {
  tasks: DesktopTaskForestNode[];
  total: number;
  projectionCurrent: boolean;
  rootThreadIds: string[];
  skippedPinnedThreadIds: string[];
};
```

`gary-client.ts` maps the additive root metadata from snake/camel case.

`TaskForestConsole` changes:

- receives `pinnedThreadIds` from `TasksPanel`;
- requests `listTaskForest({ includeDone: true, sourceBot })` for the pinned
  default and does not send `scope: "active"`;
- refreshes whenever `pinnedThreadIds` changes, after task creation/status
  mutation, on window focus, and on the existing short interval;
- forces a second reload after pin/unpin IPC returns the authoritative desktop
  state, so an optimistic local `pinnedThreadIds` update cannot leave the forest
  showing a stale server-side `thread_pins` snapshot until the interval fires;
- if `pinnedThreadIds.length === 0`, shows
  `Pin threads to add them to the operation room.`;
- if pinned threads exist but `page.tasks.length === 0`, shows
  `Pinned threads with tasks will appear here.`;
- breadcrumb fallback reads `Pinned roots` when projection is current;
- command palette keeps visible-node actions only; remove root selector/search
  affordances from the v1 plan;
- root task creation still works, but it does not automatically add the new
  task to the operation room unless the user pins its thread through the
  existing pin action.

Existing pin/unpin behavior in `AppShell`, `PinnedThreadsSidebar`, desktop
store sync, and IPC remains the source of truth for root membership. The forest
should not introduce a separate root store.

## Validation Plan

Focused tests:

- gateway DB test: pinned forest returns the union of pinned task subtrees and
  excludes unrelated tasks;
- gateway DB test: pinned non-task threads are reported in
  `skippedPinnedThreadIds` and do not create synthetic nodes;
- gateway DB test: explicit pinned seed rows survive task-number dedup;
- route test: `/api/tasks/forest` with pins returns only pinned subtrees plus
  root metadata;
- desktop client test: `listTaskForest` no longer sends `scope=active` and maps
  root metadata;
- renderer or component-level test where practical: pinned id changes trigger a
  reload and empty-state copy is rooted in pins.

Build and runtime checks:

- `cargo test -p garyx-gateway --all-targets list_task_forest`
- `cargo test -p garyx-gateway`
- `cd desktop/garyx-desktop && npm run build:ui`
- `cd desktop/garyx-desktop && npm run test:unit`
- Use CDP on port `39222` against the desktop app:
  - with no pins, confirm the pinned-empty state;
  - pin a task thread and confirm its subtree appears;
  - pin a second task thread and confirm a second root/subtree appears;
  - unpin one root and confirm that subtree disappears;
  - pin a non-task chat thread and confirm it is skipped without breaking the
    task forest.

Packaging after code review:

- merge the completed worktree into `main`;
- run `bash scripts/install-local-cli.sh`;
- run `npm run dist:dir` in `desktop/garyx-desktop` and replace the installed
  app bundle through the existing packaged-app flow;
- do not restart the managed gateway from this task.
