# Stable Task Tree Popover

## Problem

The Mac chat page task popover used the currently open thread as a subtree root.
When the user clicked a child task, the popover reloaded with that child as the
root, so the parent and sibling context disappeared. The desired behavior is a
stable view of the current task's whole task tree: moving between retained nodes
changes only the current highlight.

The popover is task-only. Bare conversation threads do not produce a visible
tree.

## Comprehensive Decisions

1. Empty state is hidden. When the anchored tree retains no nodes, the popover
   returns `null` instead of showing an empty placeholder.
2. API shape stays additive and small. `listTaskForest` gets one new
   `anchorThreadId` parameter. The response shape is unchanged.
3. Parent resolution follows the existing backend helper
   `task_forest_immediate_parent_index` to avoid changing historical semantics:
   `parent_task_number`, then the number parsed from `source_task_id`, then
   `source_task_thread_id`. This differs from one draft design that listed the
   thread reference before the legacy task id; current code is the authority.
4. `rootThreadId` and `list_task_forest_rooted` are removed from the product API.
   Repository grep showed the only product caller was
   `ThreadTaskTreePopover`; after migrating that caller to `anchorThreadId`, the
   old rooted mode had no remaining product use. The pinned and `scope=all`
   console paths still use `list_task_forest` and are not changed.

## Display Rule

For an anchor task node `a`, load the full task tree `T` by climbing from `a` to
the topmost parent task and then expanding all descendants from that root.

Let:

- `A = { n in T | n.status in {in_progress, in_review} }`
- `Path(n)` be `n` plus all parents up to the root
- `Ancestors(S) = union(Path(n) for n in S)`

The retained set is:

```text
Retained = empty                         if a is missing
Retained = empty                         if A is empty
Retained = Ancestors(A) union Path(a)    otherwise
```

`Path(a)` is the approved Plan B: if the current task is on a dead branch, keep
the current node and its path to root so the highlight still has a visible
target. When the current node is active or an ancestor of an active node, this
adds nothing, so the tree remains stable and only the highlight moves.

Because every retained node's original parent is also retained, the output keeps
original parent edges. There is no reparenting in the anchored tree.

The badge is the active count, not the visible node count:

```text
count(status in {in_progress, in_review})
```

## Layering

SQLite loads the authoritative raw tree. Rust owns the product pruning as a pure
function. The desktop renderer only reads the returned nodes, computes the local
current highlight from `node.threadId === currentThreadId`, and renders.

This split keeps projection and dedupe rules server-side while making the
retention rule directly testable without SQLite, HTTP, Electron, or React.

## Backend Query

Anchored mode uses one recursive query. The first recursion climbs from the
anchor to the root task. The second expands all descendants from that root.
Both operate over the same deduped projection set and all statuses.

The parent edge is resolved once in `parent_edges`, using the existing priority:
`parent_task_number`, else `source_task_id` number, else `source_task_thread_id`.

```sql
WITH RECURSIVE
  filtered AS (
    SELECT task.thread_id, task.number, task.status, task.title,
           task.creator_json, task.assignee_json, task.source_json,
           task.executor_json, task.updated_at, task.updated_by_json,
           COALESCE(meta.agent_id, '') AS runtime_agent_id,
           COALESCE(meta.message_count, 0) AS reply_count,
           task.parent_task_number,
           task.source_task_thread_id,
           task.source_task_id,
           recent.active_run_id,
           recent.run_state,
           recent.last_active_at,
           ROW_NUMBER() OVER (
             PARTITION BY task.number
             ORDER BY task.updated_at DESC, task.thread_id ASC
           ) AS rn
    FROM task_projection task
    LEFT JOIN thread_meta meta ON meta.thread_id = task.thread_id
    LEFT JOIN recent_threads recent ON recent.thread_id = task.thread_id
    WHERE task.projection_version = ?
  ),
  deduped AS (
    SELECT * FROM filtered WHERE rn = 1
  ),
  parent_edges AS (
    SELECT child.thread_id,
           COALESCE(
             (
               SELECT parent.thread_id
               FROM deduped parent
               WHERE (
                   child.parent_task_number IS NOT NULL
                   AND parent.number = child.parent_task_number
                 )
                 OR (
                   child.parent_task_number IS NULL
                   AND child.source_task_id = ('#TASK-' || parent.number) COLLATE NOCASE
                 )
               ORDER BY parent.updated_at DESC, parent.thread_id ASC
               LIMIT 1
             ),
             child.source_task_thread_id
           ) AS parent_thread_id
    FROM deduped child
  ),
  up(thread_id, number, depth, path) AS (
    SELECT d.thread_id, d.number, 0, ',' || d.thread_id || ','
    FROM deduped d
    WHERE d.thread_id = ?
    UNION ALL
    SELECT parent.thread_id, parent.number, up.depth + 1,
           up.path || parent.thread_id || ','
    FROM up
    JOIN parent_edges edge ON edge.thread_id = up.thread_id
    JOIN deduped parent ON parent.thread_id = edge.parent_thread_id
    WHERE up.depth < 64
      AND instr(up.path, ',' || parent.thread_id || ',') = 0
  ),
  root AS (
    SELECT thread_id
    FROM up
    ORDER BY depth DESC, thread_id ASC
    LIMIT 1
  ),
  down(thread_id, number, depth, path) AS (
    SELECT root.thread_id, d.number, 0, ',' || root.thread_id || ','
    FROM root
    JOIN deduped d ON d.thread_id = root.thread_id
    UNION ALL
    SELECT child.thread_id, child.number, down.depth + 1,
           down.path || child.thread_id || ','
    FROM down
    JOIN parent_edges edge ON edge.parent_thread_id = down.thread_id
    JOIN deduped child ON child.thread_id = edge.thread_id
    WHERE down.depth < 64
      AND instr(down.path, ',' || child.thread_id || ',') = 0
  )
SELECT deduped.thread_id, deduped.number, deduped.status, deduped.title,
       deduped.creator_json, deduped.assignee_json,
       deduped.source_json, deduped.executor_json,
       deduped.updated_at, deduped.updated_by_json,
       deduped.runtime_agent_id, deduped.reply_count,
       deduped.parent_task_number,
       edge.parent_thread_id,
       deduped.active_run_id,
       COALESCE(deduped.run_state, 'idle') AS run_state,
       deduped.last_active_at,
       deduped.source_task_id,
       deduped.source_task_thread_id,
       (SELECT thread_id FROM root) AS root_thread_id
FROM down
JOIN deduped ON deduped.thread_id = down.thread_id
LEFT JOIN parent_edges edge ON edge.thread_id = deduped.thread_id
ORDER BY down.depth ASC, deduped.number ASC, deduped.thread_id ASC;
```

The Rust pure function receives the raw rows and `anchorThreadId`, computes
`Retained`, and emits task nodes with original retained parent edges:

```rust
pub fn prune_anchored_task_tree(
    raw: Vec<RawTaskNode>,
    anchor_thread_id: &str,
) -> Vec<TaskForestNode>
```

If the anchor is not in the raw tree or if there are no active tasks, it returns
an empty vector.

## API Contract

`GET /api/tasks/forest?anchor_thread_id=<thread>` returns the anchored tree.

Dispatch order:

```text
anchor_thread_id present -> list_task_forest_anchored
otherwise                -> list_task_forest(filter, scope)
```

`scope=pinned` and `scope=all` keep their existing behavior. The rooted
descendant-only mode is removed from the desktop contract and route query.

Desktop `ListTaskForestInput` gets:

```ts
anchorThreadId?: string | null;
```

## Desktop Behavior

`ThreadTaskTreePopover`:

- calls `listTaskForest({ anchorThreadId: threadId })`
- filters only non-task root nodes, not task statuses
- returns `null` when no task nodes are returned
- highlights `task.threadId === threadId`
- shows badge count for active statuses only
- renders `todo`, `in_progress`, `in_review`, and `done` status pills
- renames the visible label from `Subtask tree` to `Task tree`

Component and CSS names can stay `ThreadTaskTreePopover` and `thread-subtask-*`
to keep the diff scoped.

## Cross-Platform Impact

iOS has no task-forest popover and does not call `/api/tasks/forest`. This task
is desktop plus gateway only. There is no router write-path change; the gateway
only reads `task_projection`.

## Test Plan

Automated:

- pure `prune_anchored_task_tree` tests for all 10 accepted scenarios
- pure test for parent priority matching `task_forest_immediate_parent_index`
- DB tests proving deep anchors resolve the true root, inactive ancestors are
  kept, inactive leaves are pruned, Plan B keeps the current dead branch path,
  all-done trees are hidden, and bare threads return no nodes
- route test for `anchor_thread_id`
- desktop client test for `anchorThreadId` serialization
- renderer model test proving no status filtering, active-count badge, and
  local highlight logic

Focused commands:

```bash
cargo test -p garyx-gateway --all-targets task_tree list_task_forest
cd desktop/garyx-desktop && npm run build:ui && npm run test:unit
```

Runtime:

- open a deep child thread and confirm the whole retained tree is visible
- click a parent or sibling and confirm the node set is stable while highlight
  moves
- open a done dead-branch task and confirm Plan B adds the current path
- open a bare thread or all-done tree and confirm the popover is hidden
