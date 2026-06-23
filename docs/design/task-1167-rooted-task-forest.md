# Task 1167 Pinned-Thread Task Forest

## Goal

Change the Mac task forest into a pinned-thread operation room. The forest root
set is the user's current pinned threads. Pinning a thread adds it to the
operation room; unpinning it removes that root. Each pinned conversation thread
is rendered as a root node using thread metadata, then the gateway attaches
tasks whose `TaskSource.thread_id` points at that conversation and recursively
attaches task children below them. Unrelated tasks are not returned.

Follow-up: Task 1216 keeps the pinned-root contract but changes the default task
node set to active statuses only. See
`docs/design/task-1216-active-status-task-forest.md` for the status filter and
reparenting rules.

Keep the existing canvas, tidy-tree layout, cycle defense, node cards, node
selection, thread detail panel, panning, zooming, minimap, and visual styling.
This change only replaces the task set selection contract and wires the forest
to the existing pin/unpin product model.

## Product Contract

The forest no longer has a global, active-chain, or manually searched root.

- Root set: `thread_pins` in display order. The existing pin/unpin actions are
  the only v1 root management UI.
- Pinned conversation thread: render the conversation as a thread root even
  when it has no task overlay. The root title/label/preview comes from
  `recent_threads` / `thread_meta`.
- Direct child tasks: render tasks whose source conversation is the pinned
  thread (`task_projection.source_thread_id = pinned.thread_id`) as first-layer
  children.
- Task descendants: recurse from those tasks by task parent identity.
- Skipped pinned thread: skip only when the pinned conversation has neither a
  task projection row nor any directly derived task in raw projection data.
- Empty state: when no pinned threads exist, show `Pin conversations to add them to
  the operation room.`
- No visible roots: when pinned threads exist but none has a visible task
  overlay or derived task under the active filters, show `Pinned conversations
  with tasks will appear here.`
- Pin changes: after a thread is pinned or unpinned, desktop state already
  updates `pinnedThreadIds`; the mounted forest observes that state and reloads.
- Search selector and node drill-down are not part of the v1 root contract.
  The command palette may continue to search visible nodes, but it must not own
  root selection.

Thread roots and task nodes are visually distinct. Thread roots use conversation
metadata and open the pinned conversation in the thread panel; task nodes keep
task card semantics, status, assignee/runtime, task id, child task creation, and
status mutation.

## Gateway Contract

Extend the existing read endpoint behavior with tagged forest nodes and root
metadata:

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

type TaskForestNode =
  | ({ kind: "thread"; nodeId: string } & ThreadRootFields)
  | ({ kind: "task"; nodeId: string; parentNodeId?: string | null } & TaskSummaryFields);
```

Rules:

- Default forest (`scope` omitted): read `thread_pins` from the gateway DB,
  treat those thread ids as root conversations, and return each emitted
  conversation root plus its visible derived task subtree.
- Diagnostic forest (`scope=all`): keep the existing full forest behavior for
  diagnostics only.
- Remove `scope=active` from the Mac app and do not keep active-chain selection
  as a product path.
- `TaskForestScope::default()` becomes pinned. Existing desktop is the only
  product caller of `/api/tasks/forest`; `scope=all` remains available for
  explicit diagnostics.
- Run the existing task projection backfill check before reading, exactly like
  the current endpoint.
- `rootThreadIds` contains pinned conversations that emitted a visible root
  node. `skippedPinnedThreadIds` contains pinned conversations that have neither
  a raw task projection row for themselves nor any raw direct derived task.
  A pinned conversation whose derived tasks are filtered out by `source_bot_id`
  or status is not mislabeled as a non-task chat.

### Query Plan

Use `task_projection` as the only task ancestry source.

For the pinned default:

- load pinned threads from `thread_pins`, preserving the existing
  `pinned_at DESC, thread_id ASC` order;
- build a filtered raw task set using the existing `TaskListFilter` so
  `source_bot_id`, `status`, and `include_done` remain server-side filters;
- seed the recursive CTE from filtered tasks that either are the pinned thread's
  own task overlay or are direct children with
  `source_thread_id = pinned.thread_id` and no task parent;
- guarantee explicit pinned/direct seed rows win over the number dedup policy,
  so the visible subtree does not disappear if another projection row has the
  same task number and a newer timestamp;
- recurse parent to child by `source_task_thread_id`,
  `parent_task_number`, or legacy `source_task_id = '#TASK-' || parent.number`;
- carry a thread-id path and bounded depth to prevent cycles;
- join `thread_meta` and `recent_threads` for thread root metadata and for the
  task parent/run-state fields already returned by task nodes;
- de-duplicate overlapping subtrees by thread id, preserving first reached root
  order;
- return rows ordered by pinned-root order, tree depth, sibling task number, and
  thread id so each pinned subtree is stable;
- insert one `kind: "thread"` root node before each root's task nodes. Direct
  task children point to that root with `parentNodeId =
  "thread-root:<thread_id>"`; task descendants point to task parents with
  `parentNodeId = "task:<parent_task_thread_id>"`.

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
It maps tagged `kind: "thread"` and `kind: "task"` forest nodes while keeping
compatibility with old task-only payloads as task nodes.

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
  `Pin conversations to add them to the operation room.`;
- if pinned threads exist but `page.tasks.length === 0`, shows
  `Pinned conversations with tasks will appear here.`;
- renders thread roots with a conversation style and task nodes with the
  existing task status style;
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
- gateway DB test: pinned chat roots render as `kind: "thread"` roots and
  `source_thread_id` direct tasks render as first-layer children;
- gateway DB test: pinned conversations with no self task and no direct derived
  task are reported in `skippedPinnedThreadIds`;
- gateway DB test: explicit pinned/direct seed rows survive task-number dedup;
- route test: `/api/tasks/forest` with pins returns only pinned subtrees plus
  root metadata;
- desktop client test: `listTaskForest` no longer sends `scope=active` and maps
  root metadata;
- renderer/component tests: thread roots lay out above direct task children,
  pinned id changes trigger a reload, and empty-state copy is rooted in
  conversations.

Build and runtime checks:

- `cargo test -p garyx-gateway --all-targets list_task_forest`
- `cargo test -p garyx-gateway`
- `cd desktop/garyx-desktop && npm run build:ui`
- `cd desktop/garyx-desktop && npm run test:unit`
- Use CDP on port `39222` against the desktop app:
  - with no pins, confirm the pinned-conversation empty state;
  - pin a task-producing conversation such as `thread::pinned-chat` and confirm the conversation root plus its
    derived task subtree appears;
  - pin a second conversation and confirm a second root/subtree appears;
  - unpin one root and confirm that subtree disappears;
  - pin a conversation with no derived tasks and confirm it is skipped without
    breaking the task forest.

Packaging after code review:

- merge the completed worktree into `main`;
- run `bash scripts/install-local-cli.sh`;
- run `npm run dist:dir` in `desktop/garyx-desktop` and replace the installed
  app bundle through the existing packaged-app flow;
- do not restart the managed gateway from this task.
