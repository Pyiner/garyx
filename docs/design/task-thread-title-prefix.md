# Task Thread Title Prefix Design

## Goal

New task backing threads should store a task-prefixed thread title in the
canonical thread record:

```text
#TASK-N Original task title
```

The task model remains unchanged: `ThreadTask.title` stays the plain task
title, and only the backing thread record's `label` is prefixed. Existing task
threads are not migrated.

## Current Code Paths

- Task creation and promotion live in `garyx-router/src/tasks.rs`.
  `TaskService::create_task` creates a thread with `create_thread_record`,
  derives the task title, allocates the task number in `build_task`, stores the
  task overlay with `set_task_on_record`, and writes the full thread record via
  `ThreadStore::set`.
- `TaskService::promote_task` loads an existing thread record, derives a task
  title from the request or existing thread content, allocates a task number,
  stores the task overlay, and writes the same thread record back.
- `TaskService::set_title` currently mutates only `ThreadTask.title` through
  `mutate_task`.
- The canonical thread title field is `label`.
  `garyx-router/src/threads.rs::label_from_value` reads `label`, falling back to
  `display_name` and `subject`.
- The desktop/mobile recent-thread list follows the stored label indirectly:
  `garyx-gateway/src/recent_thread_projection.rs` projects thread-store writes
  into `recent_threads.title`.
- Manual thread title updates go through
  `garyx-router/src/threads.rs::update_thread_record`, which writes `label`,
  sets `thread_title_source = "explicit"`, clears `provider_thread_title`, and
  persists through `ThreadStore::set`.

## Auto-Management Marker

Use the existing top-level `thread_title_source` field as the management marker:

- `thread_title_source = "task"` means the backing thread title is owned by the
  task service.
- `thread_title_source = "explicit"` means a user manually renamed the thread,
  so later task title changes must not overwrite the thread title.
- Missing or other values are treated as unmanaged for task title updates. This
  keeps old task threads untouched.

No new field is needed. This preserves JSON compatibility: existing task fields
do not change, and existing consumers that already tolerate
`thread_title_source` continue to do so.

The task service should also verify the current label still matches the
previous generated title before auto-updating on `set-title`. That protects
against nonstandard write paths that changed `label` without marking the source
as `explicit`.

Provider-generated titles and prompt-derived fallback titles already avoid
non-empty human labels. With `thread_title_source = "task"` and a non-empty
`#TASK-N ...` label:

- `garyx-bridge/src/multi_provider/run_management.rs::should_apply_provider_thread_title`
  will not replace the task label, because it only force-replaces
  `thread_title_source = "garyx_prompt"` or empty/default/API-placeholder
  labels.
- `garyx-gateway/src/application/chat/prepare.rs::should_autoname_thread` will
  not replace the task label, because the label is neither empty,
  `"Fresh Thread"`, nor the API route placeholder.

## Router Changes

Add helpers in `garyx-router/src/tasks.rs`:

- `task_thread_title(task) -> String`: returns
  `format!("{} {}", canonical_task_id(task), task.title)`.
- `set_task_thread_title(record, task)`: writes `label`, sets
  `thread_title_source = "task"`, and removes `provider_thread_title`.
- `is_task_thread_title_managed(record, task)`: returns true only when
  `thread_title_source == "task"` and the current `label` equals the generated
  title for the current task.

Creation:

1. Keep the existing `create_thread_record` flow.
2. After `build_task` allocates the task number and before the final
   `ThreadStore::set`, call `set_task_thread_title(&mut record, &task)`.
3. Then call `set_task_on_record`. The persisted thread label becomes
   `#TASK-N <task.title>`, while `task.title` remains plain.

Promotion:

1. Derive the plain task title from the request or existing thread record. If
   the request omits a title and the existing thread has
   `thread_title_source = "explicit"`, use the existing visible label before
   falling back to message-derived `derive_title`.
2. After `build_task`, call `set_task_thread_title(&mut record, &task)` before
   the final store write.
3. This intentionally prefixes the promoted thread even if the previous thread
   label was human-written, because promotion creates the task-managed backing
   thread.

Task title update:

1. Replace the current `set_title` implementation with an explicit
   load-lock-mutate-store flow so unrelated task mutators keep their existing
   `mutate_task` behavior.
2. Before changing `task.title`, calculate whether the current thread title is
   task-managed with `is_task_thread_title_managed(&record, &task)`.
3. Update `ThreadTask.title` and push the existing `TitleChanged` event.
4. If the thread title was managed, call `set_task_thread_title` with the
   updated task before `set_task_on_record`.
5. If the thread title was not managed, only persist the task overlay.

Manual rename handling:

- Leave `update_thread_record` as the manual thread-title boundary. It already
  marks the source as `explicit`.
- Because `set-title` only rewrites when `thread_title_source == "task"`, any
  later `garyx task set-title` after a manual thread rename updates only
  `task.title`.

## Gateway And Projection Effects

The router writes the full thread record through the same `ThreadStore::set`
path already used by tasks. In the gateway, that store is wrapped by
`RecentThreadProjectingStore`, so the prefixed `label` is projected to
`recent_threads.title` at write time. No desktop or mobile display-layer prefix
logic is needed.

`create_task` will still write a transient un-prefixed label through
`create_thread_record` before writing the final prefixed label after the task
number is allocated. This can project twice in the gateway, but the final
steady-state record is correct before the task API returns.

Locking stays the same: `create_task` creates a new thread and does not need a
task-thread lock; `promote_task` already locks the target thread; `set_title`
will keep the same resolved-thread lock currently provided by `mutate_task`.

The task route responses remain compatible:

- `task.title` is still the plain title.
- `GET /api/tasks/:id` returns the thread record with a prefixed `label`.
- Task list summaries still report the plain task title, because they are based
  on `ThreadTask.title`.

## Test Strategy

Router unit tests in `garyx-router/src/tasks.rs`:

- `create_task` stores a prefixed thread `label`, sets
  `thread_title_source = "task"`, and leaves `ThreadTask.title` plain.
- `promote_task` prefixes the existing thread label after promotion.
- `set_title` updates a managed backing thread label from
  `#TASK-N old` to `#TASK-N new`.
- A manual `update_thread_record` rename changes `thread_title_source` to
  `explicit`; a later task `set_title` updates only `ThreadTask.title`.
- A seeded old task record without `thread_title_source = "task"` does not get a
  backing thread title rewrite on `set_title`.

Gateway integration tests in `garyx-gateway/src/routes/tests.rs`:

- `POST /api/tasks` persists the prefixed backing thread label while the
  response task title stays plain.
- `PATCH /api/tasks/:id/title` updates the prefixed backing thread label and the
  recent-thread projection, asserted through the recent-threads route or
  `recent_threads.title`.
- `PATCH /api/threads/:id` manual rename followed by task title update leaves
  the manual thread label unchanged.

Additional regression tests:

- `garyx-bridge/src/multi_provider/run_management/tests.rs` should assert
  provider title persistence does not replace a task-managed
  `#TASK-N ...` label.
- `garyx-gateway/src/application/chat/prepare/tests.rs` should assert prompt
  auto-naming does not replace a task-managed `#TASK-N ...` label.

Validation:

```bash
cargo build
cargo test -p garyx -p garyx-router -p garyx-gateway
cargo test -p garyx-bridge
```
