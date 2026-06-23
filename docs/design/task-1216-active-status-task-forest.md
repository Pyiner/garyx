# Task 1216 Active-Status Task Forest

## Goal

Keep the pinned-conversation task forest as the operation-room view, but only
render task nodes whose status is `in_progress` or `in_review`. Historical
`done` tasks and not-yet-started `todo` tasks stay out of the default forest.

`scope=all` remains the diagnostic escape hatch for the complete task map.

## Filtering Contract

The default forest endpoint still starts from pinned conversation roots and
uses the existing recursive task subtree walk. The status filter is applied
after the recursive walk has reached the subtree, not before it. This preserves
active descendants below inactive intermediate tasks.

Thread root nodes are not task status filtered. If a pinned conversation has a
task subtree but no retained active task, the response may contain only the
thread root for that pinned conversation.

## Reparenting Contract

Filtering inactive task nodes can remove an intermediate parent. Every retained
task is reparented as follows:

1. Follow the original parent chain using `parent_task_number`, with legacy
   `source_task_id` and `source_task_thread_id` as fallbacks.
2. The first ancestor in the same pinned subtree whose status is
   `in_progress` or `in_review` becomes the retained task's parent.
3. If no retained ancestor exists before the pinned conversation root, the task
   attaches directly to that thread root.
4. The returned `parent_node_id`, `parent_thread_id`, and `parent_task_number`
   describe this rendered parent, not the filtered-out original parent.

The resulting graph stays connected from each pinned conversation root to the
active task nodes while skipping inactive history.
