# Task Tree: Source-Thread Root, Done Retention, and iOS Half-Screen Sidebar

Design for TASK-1566. One feature plus two tree-logic corrections, designed
together because they share one data contract:

1. iOS: a right-edge swipe-in, roughly half-screen task-tree sidebar on the
   conversation page, matching Garyx glass chrome, with node taps switching
   threads.
2. Correction A: the tree is rooted at the **source conversation thread** that
   spawned the tasks (itself not a task), clickable to navigate back. Mac and
   iOS behave identically.
3. Correction B: **done tasks stay in the tree** (visually de-emphasized, never
   pruned away).

No implementation code in this document; it is the blueprint for a follow-up
implementation task.

## 1. Verified Current State

All findings verified in this worktree (main at `9a8e2385`).

### 1.1 TASK-1269 "stable task tree" is implemented and merged

The design doc is `docs/design/task-1269-stable-task-tree.md` (the task context
said 1267; the landed doc/number is 1269). Implementation landed on main as
`1a86634e` ("Stabilize anchored task tree popover") and was extended by
`7a39a10f` ("Show source conversation task subtree", 2026-06-25).

Current server shape:

- `garyx-gateway/src/task_tree.rs` — pure function
  `prune_anchored_task_tree(raw, anchor_thread_id)`. Retention rule is 1269's:
  `Retained = Ancestors(A) ∪ Path(anchor)` where
  `A = {status ∈ {in_progress, in_review}}`; **empty when A is empty**.
- `garyx-gateway/src/garyx_db/task_forest.rs` —
  `list_task_forest_anchored(anchor_thread_id, filter)` with two branches:
  - **Task anchor** (`task_forest.rs:600`): recursive `up` CTE climbs from the
    anchor to the topmost *task* (the climb stops when `parent_edges` points at
    a thread not present in `task_projection`), then a `down` CTE expands all
    descendants. Rows go through `prune_anchored_task_tree`. Output is
    task-only; the root task gets `ResolvedParent::None`, so **the wire drops
    every reference to the source conversation** (`task_tree.rs:148-153`).
    `root_thread_ids = [root task thread]`.
  - **Conversation anchor** (`task_forest.rs:749`): `seeds` selects root tasks
    with `source_thread_id = anchor AND parent_task_number IS NULL AND
    source_task_id IS NULL`, expands descendants, prunes, and — only when the
    pruned set is non-empty — **inserts one `kind:"thread"` node at index 0**
    hydrated by `task_forest_thread_root_from_conn` (`task_forest.rs:1449`,
    title from `recent_threads`/`thread_meta` with thread-id fallback). Orphan
    task parents are rewritten to `thread-root:<anchor>`.
- The SQL loads **all statuses** (`include_done: true` at
  `task_forest.rs:594-597`). Done filtering happens **only in the Rust pure
  function**, in three ways: done leaves are dropped, dead done branches are
  dropped, and an all-done tree returns empty (popover hidden). This is the
  exact layer Correction B changes.
- Wire node enum `TaskForestNode` (`task_forest.rs:48`) already has both
  variants: `Thread { node_id, thread_id, title, thread_type, provider_type,
  agent_id, message_count, last_message_preview, active_run_id, run_state,
  updated_at, last_active_at }` and `Task { node_id, parent_node_id,
  <flattened TaskSummary>, parent_task_number, parent_thread_id, active_run_id,
  run_state, last_active_at }`, tagged `kind`, snake_case.
- Route: `GET /api/tasks/forest` (`tasks.rs:556`). `anchor_thread_id` present →
  anchored mode; otherwise `scope=pinned|all` console modes. Response:
  `{ tasks, total, projection_current, root_thread_ids,
  skipped_pinned_thread_ids }`.
- Node identity: `task:<thread_id>` / `thread-root:<thread_id>`
  (`task_tree.rs:156-162`). Node key is the thread id, per the projection
  contract.
- Anchored responses are ordered by the SQL (`ORDER BY down.depth ASC, number
  ASC`) — i.e. **BFS order**; clients rebuild the DFS tree themselves.

### 1.2 Desktop popover

`desktop/garyx-desktop/src/renderer/src/app-shell/components/ThreadTaskTreePopover.tsx`
polls `listTaskForest({ anchorThreadId })` every 5s, renders
`buildTaskRows(nodes)` from
`thread-task-tree-popover-model.ts`, hides itself when `rows.length === 0`,
badge = active count, highlight = `task.threadId === currentThreadId`.

Key defect for Correction A: `buildTaskRows` **skips `kind === "thread"` nodes**
(`thread-task-tree-popover-model.ts:117-118` walks their children at the same
depth without emitting a row). So even in the conversation-anchor case where
the gateway already sends a Thread root, the desktop renders task rows only.
Combined with the task-anchor branch never emitting the Thread node, there is
no way back to the source conversation from inside a task thread — the
reported bug.

Desktop contracts already model both node kinds
(`src/shared/contracts.ts:239-268`, `DesktopTaskForestThreadNode`), and the
main-process mapper `mapTaskForestNode` (`src/main/gary-client.ts:2546`)
already maps them. Only additive fields need mapping changes.

### 1.3 iOS

- Zero task-forest UI: nothing calls `/api/tasks/forest`; `GaryxTaskSummary`
  (`Sources/GaryxMobileCore/GaryxGatewayTaskModels.swift:32`) has no parent
  fields; there is no forest model in Core.
- The conversation page's existing tasks entry is a flat, source-thread
  filtered list (`GaryxMobileTasksPanelState.swift`, "View Tasks (n)" in the
  header ellipsis menu). It is a list, not a tree; it stays as-is.
- Navigation drawer precedent: `GaryxShellView` in
  `App/GaryxMobile/GaryxMobileViews.swift:476-810` implements a hand-rolled
  left-edge drawer whose gesture architecture we mirror on the right edge:
  24pt edge zone, 18pt minimum drag, axis decision at 14pt with a 1.5
  horizontal-dominance ratio, `@GestureState` liveness for cancel cleanup,
  `simultaneousGesture` + child-control disabling during drags, open threshold
  22%/35% (predicted) of panel width, close threshold 12%/28%, pre-baked
  gradient edge strip instead of `.shadow`, safe-area-outset clip shape,
  `GaryxMobileMotion.sidebar` spring.
- Design system helpers exist: `garyxAdaptiveGlass(_:in:)`,
  `GaryxAdaptiveGlassContainer`, `garyxPageBackground`,
  `garyxFloatingBottomChrome`, `garyxAdaptiveTopBar`
  (`GaryxMobileDesignSystem.swift`), status pill `GaryxStatusPill` with
  `GaryxTaskStatus.label/.tone` (`GaryxMobileStatusComponents.swift:33`,
  `GaryxMobileTasksViews.swift:527`), identity via
  `GaryxMobileIdentityPresentation` + avatar cache.
- Thread opening goes through the shared `GaryxMobileModel.openThread`
  (`GaryxMobileModel+AgentsWorkspaces.swift:91-127`).
- `task_projection` columns confirmed (`garyx_db/mod.rs:3039`,
  `task_forest.rs:1263`): `source_thread_id` = thread the task was created
  from (`TaskSource.thread_id`; for a conversation-spawned root task this is
  the conversation), `source_task_thread_id` = spawning *task* thread,
  `parent_task_number` = explicit parent. Root tasks created with no thread
  context (console) have `source_thread_id = NULL`.

## 2. Target Semantics (server-owned)

One definition shared by every surface. For an anchor thread `t`:

1. **Origin resolution.**
   - If `t` is not a task: `origin = t`.
   - If `t` is a task: climb parent edges (existing priority:
     `parent_task_number`, else `source_task_id` number, else
     `source_task_thread_id`) to the topmost task `r`;
     `origin = r.source_thread_id` (may be NULL).
2. **Node set (Correction B).** If `origin` exists: every task whose root's
   `source_thread_id = origin` (the seeds CTE) plus all their descendants —
   **all statuses, no active-set gate, no done pruning**. If `origin` is NULL:
   the single tree containing `t` (climb + expand), all statuses.
3. **Root node (Correction A).** If `origin` exists, emit exactly one
   `kind:"thread"` node first (hydrated title/avatar/run state), and every
   root task's `parent_node_id = thread-root:<origin>`. If `origin` is NULL,
   emit no synthetic root; root tasks keep `parent_node_id = None`.
4. **Empty rule.** The response is empty only when the node set contains no
   tasks (bare conversation). All-done trees are now visible.
5. **Order + depth.** Nodes are emitted in **DFS pre-order** with an additive
   `depth` field (thread root 0; root tasks 1 when a thread root exists,
   otherwise 0). Siblings sort by task number ascending — stable across status
   transitions, matching the existing desktop sort.
6. **Badge.** Active count (`in_progress | in_review`) is unchanged in meaning
   and is now also returned as page-level `active_count` so clients don't
   recount.
7. **Highlight.** Client-side only: `node.thread_id == current thread id`,
   applying to the thread root row too.

Because retention no longer depends on the anchor, the tree is *identical*
from the source conversation and from any task inside it; only the highlight
moves. This supersedes 1269's `Ancestors(A) ∪ Path(anchor)` (that rule existed
to keep the view stable while hiding dead branches; with done retention the
whole tree is retained, which is strictly more stable). The two 1269 decisions
explicitly called out in the task — "no thread root on the wire" and "hide
when no active tasks" — are deliberately reversed.

Out of scope: the pinned/`scope=all` console forest keeps its current
active-gated contract (`task_forest.rs:886` path); it is an "active work"
dashboard, not the per-thread tree.

## 3. Data Contract Changes

### 3.1 Wire (additive only)

`GET /api/tasks/forest?anchor_thread_id=<thread>`:

- `tasks`: now may contain one `kind:"thread"` node for **task anchors** too
  (today only conversation anchors get one). Node fields unchanged.
- Every node gains optional `depth: number` (present in anchored mode; omitted
  in console modes).
- Page gains `active_count: number` (anchored mode; computed in Rust).
- `root_thread_ids`: `[origin]` when an origin exists (both anchor kinds),
  else `[root task thread]` — aligning the two branches.
- Ordering becomes DFS pre-order. No client depends on BFS order (desktop
  rebuilds; iOS is new).

### 3.2 Compatibility matrix

| Gateway | Client | Result |
| --- | --- | --- |
| new | old desktop | `DesktopTaskForestThreadNode` already decodes; old `buildTaskRows` skips thread rows and renders tasks as today; done rows appear (old model never filtered status client-side — acceptable, it is the new intended content). Unknown `depth`/`active_count` are ignored by the payload mapper. |
| old | new desktop | No thread node / no `depth` for task anchors → model falls back to local tree building (existing `buildTaskRows` path); no back-navigation row, i.e. today's behavior. |
| old | new iOS | Same fallback: local DFS layout from `parent_node_id`, no thread root row. Sidebar still works. |
| new | new | Full behavior. |

No versioned endpoint, no breaking field changes.

### 3.3 Gateway implementation plan

`garyx-gateway/src/garyx_db/task_forest.rs`:

- Task-anchor branch: keep the `up` CTE only to resolve the root task; select
  `deduped.source_thread_id` as well (today the task-anchor SELECT omits it).
  If the root's `source_thread_id` is non-NULL, run the existing
  conversation-rooted seeds query with `origin` in place of the anchor (reuse
  the second branch's SQL — the branches converge); else keep the current
  single-tree `down` expansion.
- Emit the thread root via the existing `task_forest_thread_root_from_conn`
  for both anchor kinds when `origin` exists (hydration fallback when
  `recent_threads`/`thread_meta` rows are missing already exists).
- Known edge: a root task whose `source_thread_id` happens to be a *task*
  thread without task-linkage fields (legacy data) hydrates as a plain thread
  root; navigation still works. Not worth special-casing.

`garyx-gateway/src/task_tree.rs`:

- Replace `prune_anchored_task_tree` with a layout function (working name
  `layout_anchored_task_tree(raw, anchor_thread_id, origin_thread_id)`)
  returning DFS-ordered nodes with depths and rewritten parent ids
  (thread-root parenting for root tasks when `origin` exists). Retention: all
  nodes. Keep the cycle guard (`retain_path`'s seen-set equivalent) and the
  parent-priority resolution exactly as today. Compute `active_count`.
- `TaskForestNode` gains `depth: Option<u32>` on both variants
  (`skip_serializing_if = "Option::is_none"`); `TaskForestPage` gains
  `active_count: Option<usize>`.

Scale note: no node cap is added (recursion already capped at depth 64; a
long-lived conversation with hundreds of done tasks renders in a scrollable
container). If this becomes a problem, a server-side cap + `truncated` flag is
the follow-up, logged as a known limit.

## 4. Mac App Changes

All inside the existing popover surface (`ThreadTaskTreePopover.tsx` +
`thread-task-tree-popover-model.ts` + `gary-client.ts` mapping):

1. `mapTaskForestNode` / payload types: pass through `depth` and page
   `active_count`.
2. Model: `buildTaskRows` stops skipping thread nodes — the thread root
   becomes a real row at depth 0 (`kind: "thread"`). Prefer server
   `depth`/order when present; keep the local tree-build as the skew fallback.
   Add `isDeemphasized` (status `done`) to row output. Badge:
   `page.activeCount ?? local count`.
3. View: render the thread row (conversation glyph + `AgentOptionAvatar` from
   the node's `agent_id`/`provider_type`, title, subdued "Conversation"
   caption), clickable via the existing `onOpenThread(threadId)` — this is the
   back-to-source navigation. Task rows: add `done` row class →
   de-emphasized (reduced opacity on title/avatar, gray pill; still
   clickable). Hide the count chip when `activeCount === 0` (all-done trees
   are now visible). Empty-tree popover stays hidden (`rows.length === 0`).
4. i18n: one new label ("Conversation" caption); keep `Task tree` header.

CSS stays within `thread-subtask-*` classes per 1269's naming decision.

## 5. iOS Sidebar Design

### 5.1 Surface and entry points

A conversation-scoped trailing overlay panel (not a shell-level drawer): the
task tree belongs to the open thread, so it mounts in the conversation feature
layer, in a new feature file. Two entry points driving one state:

- **Right-edge swipe** from the trailing 24pt of the conversation page.
- **Header button** in `GaryxConversationHeader`'s trailing cluster (before
  the ellipsis menu): `ListTree`-equivalent SF Symbol
  (`list.bullet.indent`), with a small active-count badge when
  `active_count ≥ 1`. The button is shown only when the current thread's tree
  is non-empty — matching the Mac popover's hidden-when-empty rule and making
  the invisible gesture discoverable.

The existing "View Tasks (n)" flat panel in the ellipsis menu is unchanged
(different job: source-filtered task management list). Possible future
convergence is noted, not designed here.

### 5.2 Gesture (mirrors the left drawer, right edge)

Parameters copied from `GaryxShellView`'s proven drawer so the two edges feel
symmetric:

- Opening: `DragGesture(minimumDistance: 18, coordinateSpace: .global)` as a
  `simultaneousGesture` on the conversation content; qualifies only when the
  axis decision (≥14pt dominant travel, horizontal ≥ 1.5 × vertical) picks
  horizontal, translation is leftward, and `startLocation.x ≥ width − 24`.
  Progress drives the panel interactively. Open when drag > 22% of panel
  width or predicted-end > 35%; else snap back. Opening dismisses the
  keyboard (`hideKeyboard()`), like the drawer.
- Closing: mirrored gesture on the panel + scrim (rightward, thresholds
  12%/28%), plus scrim tap, plus the header button toggling off.
- Cancel safety: `@GestureState` liveness resets the axis state when the
  system cancels the gesture (drawer's documented fix, reused).
- While dragging or open: conversation controls are hit-test blocked by the
  scrim; panel controls disabled during an in-flight drag (drawer pattern).

Conflict handling:

- **System back / left drawer:** the shell's leading-edge gesture requires
  `startLocation.x ≤ 24` (left edge) — mutually exclusive with our right-edge
  start zone. While the task sidebar is open, a new
  `GaryxMobileModel.isTaskTreeSidebarOpen` published flag guards the shell's
  `openingSidebarGesture` `onEnded` action (one-line guard) so a leading-edge
  swipe cannot trigger back-navigation underneath the open panel; the swipe
  instead just closes the sidebar via the scrim's closing gesture.
- **Drawer already dragging:** the existing `garyxSidebarDragActive`
  environment value gates our gesture to a no-op while a drawer drag is live,
  and vice-versa the scrim blocks drawer opening once we're open.
- **Horizontally scrollable transcript content** (code blocks): the 24pt edge
  zone plus the 1.5 axis-dominance ratio resolves this the same way the left
  drawer coexists with horizontal list content; a code block flush against the
  right edge loses its outer 24pt as a scroll start zone — accepted tradeoff,
  identical to the drawer's.
- **Empty tree:** gesture no-ops when the tree is known-empty; while the first
  fetch is in flight it opens onto a loading state.
- **Keyboard-dismiss drag** (`GaryxMobileConversationViews.swift:455`) is
  vertical-only and gated by composer focus; the axis decision keeps them
  disjoint.

### 5.3 Layout and material

- Width: `min(max(containerWidth * 0.55, 300), 420)` — "about half screen" on
  iPhone (300pt on a 393pt device leaves the conversation peeking), capped on
  iPad. 300pt is the floor for readable rows at max indent (see 5.4 metrics).
- Full height, overlaying from the trailing edge; leading corners rounded
  28pt via a mirrored `GaryxDrawerPanelClipShape`-style clip with safe-area
  outsets so the glass reaches the physical top/bottom edges; a mirrored
  40pt pre-baked gradient strip on the panel's leading edge instead of
  `.shadow` (drawer's frame-rate lesson).
- Material: panel background `garyxAdaptiveGlass` (navigation-grade glass —
  layered material, system tint, fine top highlight), consistent with drawer
  and header chrome; the conversation behind gets a dim scrim
  (`Color.black.opacity(0.25 × progress)`). Content rows stay readable
  near-white per the mobile UI rule (glass for chrome, not per-row).
- Panel content: compact header (title "Task tree", active-count chip when
  > 0, VoiceOver-visible Close button) + scrollable tree + bottom safe-area
  padding. Motion: `GaryxMobileMotion.sidebar` spring; Reduce Motion →
  crossfade + scrim only.
- Visual details are finalized at implementation time with the
  `garyx-product-ui` skill; this section fixes structure, metrics, and
  materials.

### 5.4 Tree rendering

Row anatomy (compact, two lines, mirroring the Mac popover's information
order):

```
[indent][avatar 24pt] #TASK-1562  Provider P4 expanded editing   (Current)
                      agent-label                         [status pill]
```

- Indent: `12pt × min(depth, 4)` (Mac clamps at 4 too); depth from the wire.
- Thread root row (depth 0): conversation glyph or agent avatar from the
  node's `agent_id`/`provider_type` via the shared identity presentation
  helpers (no local switch tables), thread title, subdued "Conversation"
  caption — visibly a different species from task rows, and the tap target
  for "back to source".
- Task rows: avatar via shared identity helpers + avatar cache, mono task id,
  single-line title, `GaryxStatusPill(text: status.label, tone: status.tone)`.
- Current thread: row background tint (accent at low opacity, rounded 10pt)
  plus a "Current" tag — same rule for thread and task rows.
- Done: row content at 0.55 opacity, pill in neutral tone; fully tappable
  (Correction B's "de-emphasized, never hidden").
- Running: reuse the existing run-state accent (small pulsing dot) driven by
  `run_state`/`active_run_id` where the home list already has the pattern.
- Accessibility: rows are Buttons with labels "TASK-1562, Provider P4 …, in
  review, current"; Dynamic Type grows rows (indent constant); panel is a
  modal accessibility container with escape.

### 5.5 Navigation

Row tap → if `threadId == current`, just close; else close the panel and call
the shared `GaryxMobileModel.openThread(id:)` path (per the mobile entry-point
rule; keeps transcript loading, cold-start retry, and run-state behaviors).
`source: .replace` keeps the conversation layer at the top of the home stack —
back still pops to whatever opened the conversation, preserving the IA rule
that drilldowns return to their opener. The thread-root row is the same code
path (it is just a thread id) — this is Correction A's "click back to source"
on iOS.

### 5.6 Data flow

- Fetch: `GET /api/tasks/forest?anchor_thread_id=<current>` on conversation
  open (also powers the header badge), on sidebar open, and every 5s while
  the sidebar is open (desktop `REFRESH_MS` parity). Poll cancels on close /
  thread change / background.
- Cache: per-thread last snapshot kept in the model so reopening renders
  instantly and refreshes in place; transient errors keep the previous
  snapshot and retry silently (desktop parity). Loading spinner only on
  first-ever load for a thread.
- Why poll, not SSE: the per-thread SSE stream carries the *open thread's*
  render frames; task-tree state spans other threads. The desktop popover
  already established 5s polling for this surface. A push channel (e.g.
  recent-threads SSE fan-in) is a future optimization, out of scope.

### 5.7 GaryxMobileCore types and app wiring

Core (`mobile/garyx-mobile/Sources/GaryxMobileCore/`), all SwiftPM-testable:

- `GaryxGatewayTaskForestModels.swift`:
  `GaryxTaskForestNode` (enum, decoded by `kind`),
  `GaryxTaskForestThreadNode`, `GaryxTaskForestTaskNode` (embeds the existing
  `GaryxTaskSummary` decode for the flattened fields + `nodeId`,
  `parentNodeId`, `parentThreadId`, `runState`, `activeRunId`, `depth?`),
  `GaryxTaskForestPage` (`tasks`, `total`, `activeCount?`, `rootThreadIds`).
- `GaryxTaskTreeSidebarPresentation.swift`: pure mapping
  `rows(page:currentThreadId:) -> [GaryxTaskTreeRow]` (trusts wire order +
  `depth`; falls back to a local DFS layout from `parentNodeId` when `depth`
  is absent — old-gateway skew), `GaryxTaskTreeRow` (id, kind, threadId,
  clamped indent level, title, taskDisplayId?, status?, identity fields,
  `isCurrent`, `isDeemphasized`, run accent), `activeBadgeCount(page:)`,
  `isSidebarAvailable(page:)`, tap policy
  `shouldNavigate(currentThreadId:target:)`.
- `GaryxGatewayClient.swift`: `listTaskForest(anchorThreadId:) async throws ->
  GaryxTaskForestPage`.

App target:

- `GaryxMobileTaskTreeSidebarViews.swift` (new feature file): panel, rows,
  scrim, gestures, header button view.
- `GaryxMobileModel+TaskTree.swift`: fetch/poll/cache state,
  `isTaskTreeSidebarOpen`, open/close/toggle, row-tap handler calling
  `openThread`.
- `GaryxConversationHeader` (existing file): insert the header button.
- `GaryxShellView`: the one-line leading-edge guard on
  `isTaskTreeSidebarOpen`.

New Swift files require `xcodegen generate` and committing the regenerated
`project.pbxproj` (repo rule; `swift test` alone would be a false green).

### 5.8 SwiftPM test plan (headless-first)

Fixture-driven, no UI:

- Decoding: a captured anchored-forest JSON fixture (thread + tasks + done +
  depth, snake_case) decodes; unknown fields ignored; `kind` dispatch;
  missing `depth`/`active_count` tolerated (old gateway).
- Rows: wire order preserved; indent clamp at 4; thread row first at level 0;
  fallback layout (no `depth`) produces the same rows as server depth for the
  same fixture (cross-check test).
- Highlight: current matching for thread root and for a task; no current when
  the anchor was evicted.
- De-emphasis: exactly `done` rows flagged; badge equals wire `active_count`
  and equals the local recount fallback.
- Tap policy: current → no navigation, close-only; other → navigate.
- Availability: empty page → sidebar unavailable (gesture no-op, button
  hidden).

## 6. Impact, Tradeoffs, Rejected Alternatives

- **Unified origin-rooted forest for task anchors** (chosen) vs. "only the
  anchor's own tree": with done retention the anchor's tree alone would still
  differ from the conversation-anchored view (sibling root tasks missing),
  making the tree mutate as you navigate — rejected for instability.
- **Server DFS order + depth** (chosen) vs. both clients rebuilding trees:
  keeps node-set/root/retention *and layout* server-side per the layering
  constraint; clients keep a thin skew fallback only. Rejected pure
  client-side building because it duplicates ordering rules in two dialects
  forever.
- **Done rows inline, de-emphasized** (chosen) vs. collapsed "done" groups:
  the requirement is "never disappears"; a collapse control hides by default
  and adds state. If very large histories hurt, collapsing is a compatible
  follow-up.
- **Badge stays active-count**: done retention changes visibility, not the
  "how many things are running" semantic; also keeps old/new client badges
  consistent.
- **No synthetic root when origin is unknown** (console-created root tasks):
  fabricating a fake root would invent a concept the Mac IA doesn't have.
- **Console pinned/all forest untouched**: different product surface with an
  intentional active-work gate.
- Old desktops will start showing done rows and (already) render whatever
  statuses arrive — acceptable convergence to the new semantic without a
  client update; the thread root stays invisible on old desktops (they skip
  it) rather than breaking.
- Large trees: unbounded node count is a known limit (scrollable UI, depth-64
  SQL cap); server cap + `truncated` flag is the designated follow-up if it
  bites.
- Thread-root title hydration falls back to the raw thread id when both
  `recent_threads` and `thread_meta` rows are gone — existing behavior,
  acceptable.

## 7. Validation Plan

Layered, headless-first:

1. **Gateway pure functions** (`cargo test -p garyx-gateway --all-targets
   task_tree`): rewrite the 10 scenario tests for the new semantics (full
   retention incl. done leaf/dead branch/all-done; DFS order and depth
   values; thread-root parenting with origin; no-origin degenerate; parent
   priority unchanged; cycle guard; active_count).
2. **Gateway DB + route tests** (`cargo test -p garyx-gateway --all-targets
   list_task_forest`): task anchor emits hydrated thread root (title from
   recent_threads); conversation anchor unchanged shape plus done retention;
   deep child resolves the same origin-rooted forest as the conversation
   anchor (tree-identity test); route serializes `depth`/`active_count`;
   `root_thread_ids = [origin]`.
3. **Desktop unit** (`cd desktop/garyx-desktop && npm run build:ui && npm run
   test:unit`): popover model renders thread row, prefers server depth,
   falls back without it, flags done rows, badge from `active_count`,
   `onOpenThread` receives the thread root's id.
4. **iOS SwiftPM** (`swift test` under `mobile/garyx-mobile`; beware the
   `| tail` exit-code trap — run without piping): section 5.8 suite.
5. **End-to-end, Mac**: local gateway rebuilt (`scripts/build-local-cli.sh`,
   restart managed gateway), packaged app (`npm run dist:dir`, relaunch, CDP
   attach): from a source conversation spawn tasks → open a child task → tree
   shows conversation root + all siblings incl. done (de-emphasized) → click
   root returns to the conversation; all-done tree still visible; bare
   conversation shows no popover.
6. **End-to-end, iOS simulator** (`xcodebuild` build + idb): right-edge swipe
   opens the panel over the conversation with glass chrome; header button
   parity; tap child task switches threads via openThread (transcript loads);
   tap thread root returns to the source conversation; left-edge back swipe
   still pops when the panel is closed and only closes the panel when open;
   keyboard dismisses on open; done rows visible and dimmed; Reduce Motion
   crossfade.
7. **Device sanity** for gesture feel (edge-zone width vs. screen curvature).

## 8. Delivery Slicing (for the implementation task)

1. Gateway: layout function + origin resolution + wire additions, with tests
   (shippable alone; both clients degrade gracefully).
2. Desktop: mapper + model + popover rows (thread root, done dimming),
   with unit tests.
3. iOS Core: models + presentation + client, SwiftPM tests (no app wiring).
4. iOS app: sidebar views + model extension + header button + shell guard +
   xcodegen/pbxproj.

Each slice is independently reviewable; 1 → {2, 3} → 4 is the only ordering
constraint.

## Synthesis addenda (Gary 裁决)

This document (design A, from `78ecc9cb`) is the implementation blueprint.
Design B (`git show edd6f944:docs/design/task-1567-ios-task-tree-sidebar.md`)
was reviewed side by side; the following rulings resolve every divergence and
fold in B's engineering details. Where the two designs disagree, this section
wins.

1. **Task-anchor semantics** follow A's origin-rooted forest: climb to the
   topmost task, take its `source_thread_id` as the origin, and when an origin
   exists run the same seeds query as the conversation anchor — the tree is
   identical from the source conversation and from any task at any depth, and
   only the client-side highlight moves. Without an origin, degrade to the
   current single-tree task-only forest; never synthesize a fake root.
2. **Layout is server-owned** per A: DFS pre-order, per-node `depth`, and
   page-level `active_count` (all additive, snake_case); both clients render
   the wire order dumbly. Clients keep a thin fallback that rebuilds the tree
   locally from `parent_node_id` when an old gateway omits `depth`; B's
   orphan-parent tolerance (orphans become roots) is folded into that fallback.
3. **Done retention**: the active-set gate is removed entirely. Done rows are
   visually de-emphasized without strikethrough (B detail) and stay tappable;
   the current-thread highlight overrides the de-emphasis. Badge semantics
   stay "active count" (`in_progress` + `in_review`); clients prefer the
   page-level `active_count`. Empty (zero-task) trees still hide the entry
   points.
4. **Desktop** renders the `kind:"thread"` row (depth 0, conversation glyph +
   title + "Conversation" caption, click routes through `onOpenThread`), done
   rows drop opacity, the count chip hides at `activeCount == 0`, and the
   popover stays hidden for empty trees.
5. **iOS** executes §5 of this document in full. Folded in from B: a
   generation token keyed by gateway + anchor discards stale forest responses;
   a known-empty tree stops the 5s poll until the thread changes or a local
   task mutation occurs; a panel opened before data arrives shows a
   loading/skeleton state.
6. **Core placement** per §5.7, and any new Swift file must be added via
   `xcodegen generate` with the regenerated `project.pbxproj` committed.
7. The pinned/`scope=all` console forest contract is untouched, and no
   versioned endpoint is added.
8. **Depth = visual indent level (Gary's follow-up ruling, overrides §2.5's
   depth values).** The thread root row and top-level root tasks sit at the
   same flush indent — both `depth 0` — so adding the root row does not shift
   the whole tree one level right. Each nesting level below a task adds 1
   (with an origin, a task node's `depth` is its tree depth minus 1; the
   origin-less task-only tree already starts at 0, so both cases agree). The
   logical structure is unchanged: the thread root is still the first row,
   still the wire parent of root tasks (`parent_node_id =
   thread-root:<origin>`), and still navigates back to the source. The root
   row is distinguished by styling (conversation glyph + title + Conversation
   caption), not indentation, and the client thin fallback computes the same
   flush indent when rebuilding locally.
