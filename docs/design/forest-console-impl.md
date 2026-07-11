# Task Forest Console Implementation

> **Partially superseded (2026-07, #TASK-2099).** The
> `projectionCurrent`/warm-cache freshness concept described below no
> longer exists: task rows derive in the same transaction as every
> record write (#TASK-1864), so the projection is structurally current
> and there is no staleness flag or rebuild path.

## Goal

Land the finalized task-forest HTML demo as a real Garyx Mac app feature:
an interactive right-growing task forest, task status and run-state overview,
selection-driven thread details, continuation composer, and root/child task
creation backed by the real gateway task/thread APIs.

The HTML demo is the visual and interaction spec. The only intentional design
change is widening the right-side thread detail panel from the demo's narrow
reading width to a comfortable desktop width.

## Product Surface

Add a `Forest` task view mode inside the existing Tasks page, next to `Board`
and `List`. Keeping it under `Tasks` preserves existing navigation, task
filtering context, and user expectations while avoiding a new top-level
desktop concept.

The forest mode owns:

- A translucent top war-room bar with status chips, breadcrumb, zoom readout,
  refresh, fit controls, and New root task.
- A right-growing multi-root tree canvas with dot-grid tabletop background,
  deterministic tidy layout, orthogonal rounded edges, node cards, status
  filtering, selection, keyboard navigation, minimap, command palette, and
  reduced-motion fallbacks.
- A widened thread detail panel, target width `560px` and responsive range
  `clamp(520px, 34vw, 620px)`, that reuses the real `ThreadPage` transcript,
  tool rows, queue, attachments, and composer.
- A forest-owned panel header above `ThreadPage` with task id, title, status
  selector, assignee/runtime avatar, and quick child-task action. The selector
  calls the existing `updateTaskStatus` endpoint and treats the gateway as the
  final allowed-transition authority.
- Root and child task creation using the same task create dialog fields as the
  current Tasks page. Child creation passes `source.task_id` and
  `source.task_thread_id` so `task_projection` records parentage.

The existing board/list modes stay unchanged.

## Data Contract

### Gateway HTTP

Add a read-only endpoint:

```text
GET /api/tasks/forest?include_done=true&source_bot_id=...
```

Response shape:

```ts
type TaskForestNode = TaskSummary & {
  parentTaskNumber: number | null;
  parentThreadId: string | null;
  activeRunId: string | null;
  runState: string;
  lastActiveAt: string | null;
};

type TaskForestResponse = {
  tasks: TaskForestNode[];
  total: number;
  projectionCurrent: boolean;
};
```

Why a forest endpoint instead of deriving from `listTasks`:

- `TaskSummary` intentionally has no parent fields.
- `task_projection` already stores `source_task_thread_id`,
  `source_task_id`, and `parent_task_number`; exposing that data once keeps
  ancestry server-derived and testable.
- `recent_threads` already stores `run_state` and `active_run_id`; joining it
  in the same projection avoids local client guesses about running state.

Implementation details:

- Add a `TaskForestRow`/`TaskForestNode` projection method on
  `GaryxDbService`, using `task_projection` left-joined to `recent_threads`.
- Deduplicate by task number with the same `ROW_NUMBER() OVER (PARTITION BY
  number ORDER BY updated_at DESC, thread_id ASC)` policy as
  `list_task_summaries`.
- Support the same filter subset needed by the Tasks page: `include_done`,
  `source_bot_id`, and optional `status`.
- If `task_projection` is not current, call the existing
  `backfill_task_projection_if_incomplete` path before reading. The returned
  `projectionCurrent` flag makes stale-data risk visible to UI tests.
- Keep this read-only; creation/status mutation continue through existing
  task routes.

Runtime dependency: a gateway binary containing this endpoint must be built,
installed, and the managed gateway restarted before the installed app can use
the new API.

### Desktop IPC

Extend the existing task bridge:

- `shared/contracts.ts`
  - `DesktopTaskForestNode`
  - `DesktopTaskForestPage`
  - `ListTaskForestInput`
  - `garyxDesktop.listTaskForest(input)`
  - Add optional `source` to `CreateTaskInput` so the UI can create child
    tasks without a one-off IPC method.
- `main/gary-client.ts`
  - map snake/camel forest fields from `/api/tasks/forest`
  - include `source` in `POST /api/tasks` when present
- `main/index.ts` and `preload/index.ts`
  - add `garyx:list-task-forest`

Existing APIs reused:

- `listTasks`, `updateTaskStatus`, `stopTask`, `deleteTask`, `assignTask`
  remain board/list-owned and can be reused by forest actions where needed.
- `createTask` remains the mutation path for root and child tasks.
- Thread open, transcript history, per-thread `thread_render_frame`, and
  message sending continue through `AppShell` and `ThreadPage`.

Forest run-state freshness:

- `TaskForestConsole` silently refreshes `/api/tasks/forest` on a short
  interval while mounted, with an immediate refresh after task creation/status
  mutation and when the window regains focus.
- The selected thread still uses the normal per-thread live stream through
  `ThreadPage`; the interval is only for whole-forest overview state such as
  `run_state`, `active_run_id`, and status counts.
- Polling must be cancellation-safe and avoid overlapping requests.

## React Structure

Add:

```text
desktop/garyx-desktop/src/renderer/src/app-shell/components/TaskForestConsole.tsx
desktop/garyx-desktop/src/renderer/src/app-shell/components/task-forest-layout.ts
desktop/garyx-desktop/src/renderer/src/app-shell/components/task-forest-layout.test.mjs
```

`TaskForestConsole` props:

```ts
type TaskForestConsoleProps = {
  agents: DesktopCustomAgent[];
  botGroups: DesktopBotConsoleSummary[];
  selectedThreadId: string | null;
  selectedThreadPanel: ReactNode;
  workspaces: DesktopWorkspace[];
  workspaceMutation: string | null;
  onOpenThreadInPanel: (threadId: string) => Promise<boolean> | boolean;
  onToast: (message: string, tone?: ToastTone) => void;
};
```

The component owns forest-only state: camera transform, selected/cursor node,
status filter, command palette, minimap viewport, and new-task modal state. It
does not own transcript/composer state. Explicit subtree collapse remains a
follow-up; the v1 tree uses the three-level cap plus descendant rollups.

### AppShell integration

`openExistingThread` currently switches `contentView` to `thread`, which is
correct for normal navigation but wrong for an embedded forest detail panel.
Add a narrow helper:

```ts
async function selectExistingThread(threadId, entrySource): Promise<boolean>
```

It performs the openability check and sets `selectedThreadId` without changing
`contentView`. `openExistingThread` delegates to it after setting
`contentView("thread")`.

Extract the existing large `ThreadPage` call site into a local render helper:

```ts
function renderThreadPage(options?: {
  surfaceVariant?: "default" | "side-chat";
  threadLayoutClassName?: string;
  threadLayoutStyle?: CSSProperties;
})
```

The default thread route calls this helper with no options. Forest passes
`surfaceVariant="side-chat"` and wraps it in the widened detail panel. This
keeps transcript rows, tool grouping, composer, queued prompts, attachments,
runtime settings, and task content exactly single-sourced.

This helper aligns with the already existing side-chat surface and refs rather
than creating a second transcript/composer variant.

### TasksPanel integration

Change `TaskViewMode` from `'board' | 'list'` to
`'forest' | 'board' | 'list'`. The existing `TasksPanel` keeps the current
task loading path for board/list, and renders `TaskForestConsole` only when
`viewMode === "forest"`.

`TaskForestConsole` loads its own forest page because it needs parent and
run-state fields absent from `listTasks`. It still receives the same
`agents`, `botGroups`, `workspaces`, `workspaceMutation`, and creation helpers
as the existing task form.

## Layout Algorithm

Use a small deterministic React implementation rather than adding
`@xyflow/react`:

- The demo uses regular HTML cards plus SVG edges, not graph-editor semantics.
- Existing dependency set has no graph layout library; a bespoke tidy layout
  for a max three-level, right-growing forest is smaller than adding a graph
  editor dependency.
- The layout can be a pure function with headless tests.

Inputs:

```ts
type LayoutTask = {
  number: number;
  parentTaskNumber: number | null;
  updatedAt: string;
  status: DesktopTaskStatus;
};
```

Rules:

- Build parent/children maps from `parentTaskNumber`, falling back to
  `source.taskId` parsing only for old rows if the server field is missing.
- Roots are tasks without a resolvable parent in the current page.
- Sort roots by tree weight, active/running state, then updated time and task
  number. Sort siblings by status priority and updated time.
- Place columns by depth with `nodeWidth=264`, `colGap=92`,
  `rowGap=28`, `rootGap=44`.
- Limit visual expansion to three levels by default; deeper descendants are
  summarized in the node rollup as descendant status totals and remain
  discoverable through selection and command search.
- If filtering by bot/source omits a parent, the child becomes a root in the
  filtered view. This is expected filtered-view behavior, not a data repair.
- Return `{nodes, edges, bbox}`. Edges use orthogonal rounded paths with bus
  merge points matching the demo.

Performance:

- Layout runs in `useMemo` over the forest page.
- Render only nodes intersecting the viewport plus overscan when task count is
  large; otherwise render all nodes for crisp hover and keyboard behavior.
- At zoom below `0.58`, switch cards to birdseye LOD and hide secondary text
  while keeping stable layout geometry so panning/zooming does not relayout the
  forest.
- Minimap draws status dots and the viewport rectangle from layout positions,
  not DOM measurement.

## Visual Mapping

Use existing tokens from `styles.css`:

- Canvas: `--color-token-bg-tertiary`
- Cards/panels: `--color-token-bg-primary`
- Secondary panels: `--color-token-bg-secondary`
- Text: `--color-token-text-primary`, `--color-token-text-secondary`,
  `--color-token-description-foreground`
- Borders/rows: `--color-token-border`, `--color-token-border-light`,
  `--color-token-row-hover`, `--color-token-row-selected`
- Semantics: success green, warning orange, error red, review violet

Status colors:

- `todo`: gray
- `in_progress`: orange
- `in_review`: violet
- `done`: green
- failed runtime: red
- active run: green pulse ring and restrained green wash

Identity:

- Reuse the deterministic avatar tone algorithm from `channel-logo.tsx` for
  fallback colors.
- Prefer existing custom-agent metadata where available; otherwise display the
  principal/agent id.

Motion:

- Normal mode keeps the demo's featherweight transitions, edge flow dots, pulse
  rings, and smooth camera fit.
- Under `prefers-reduced-motion: reduce`, remove edge flow, pulse animation,
  FLIP transitions, and smooth camera easing.

## Interactions

Canvas:

- Drag background or hold Space to pan.
- Ctrl-wheel and trackpad pinch zoom to cursor.
- Keyboard zoom controls and fit controls.
- Soft camera clamping keeps the forest discoverable without hard stops.

Selection:

- Click node selects it and calls `onOpenThreadInPanel(threadId)`.
- Arrow keys move sibling/parent/child cursor. Enter opens selected cursor.
- Breadcrumb shows the selected ancestry path.

Filtering:

- Status chips filter the canvas and minimap.
- Filtering dims non-matching ancestors when needed to preserve context rather
  than making the tree appear disconnected.

Command palette:

- `Cmd+K` opens task search and quick actions: jump to task, fit forest, fit
  selected root, create root task.

Task actions:

- Root task creation opens the forest-scoped compact task form with no `source`.
- Child task creation opens the same compact form with the selected task as
  source:

```ts
source: {
  threadId: parent.threadId,
  taskId: parent.taskId,
  taskThreadId: parent.threadId,
  channel/accountId/botId from parent.source when available
}
```

## Validation

Backend:

- Add `garyx_db` tests for `list_task_forest` parent/run-state fields.
- Add a gateway handler test for `/api/tasks/forest` that seeds current
  projection rows and verifies parent/run-state fields in the response.

Desktop:

- Add mapper tests in `gary-client.test.mjs` for forest payload mapping and
  child-task `source` serialization.
- Add pure layout tests for multi-root, parent/child, three-level cap, and edge
  coordinates.
- Run `npm run build:ui` in `desktop/garyx-desktop`.

End-to-end:

- Run the desktop app against a gateway binary containing the new endpoint.
- Verify forest overview renders real tasks, selecting a node opens the real
  thread detail panel, the composer can continue the selected thread, and root
  and child creation refresh the forest.
- Capture screenshots for the overview and the widened thread panel.

## Risks

- Gateway restart dependency: local UI work can build before the installed
  gateway exposes `/api/tasks/forest`; full installed-app validation requires
  building/installing the gateway and restarting it.
- `ThreadPage` prop volume is high. Extracting a local render helper avoids
  cloning props into a separate component and keeps the refactor mechanical.
- Large forests can become noisy. The initial implementation uses LOD and
  viewport culling but does not add server paging inside a visible subtree.
- Child task source fidelity depends on parent task source fields. The server
  parent fields remain authoritative after creation via `task_projection`.
