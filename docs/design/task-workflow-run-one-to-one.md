# Task Workflow Run 1:1 Convergence

## Confirmed Contract

Task-backed workflow execution is strictly one workflow run per task. The run id
is the task thread id:

- `WorkflowRuntime::start_sdk` uses `taskThreadId` as the workflow thread/run id
  for task-backed starts.
- A task-backed start that supplies a different `workflowRunId` is rejected.
- The workflow entrypoint exports `GARYX_WORKFLOW_RUN_ID` as the task thread id.

There is no retry/rerun path that appends additional workflow runs to an existing
task. A rerun must be represented by creating a new task.

## Delete

Delete only the obsolete "list all workflow runs for a task" path:

- Gateway route `GET /api/tasks/{task_id}/workflow-runs` and
  `list_task_workflow_runs`.
- `WorkflowStore::list_runs_for_task`.
- `GaryxDbService::list_workflow_runs_for_task`.
- The `workflow_runs(task_id, created_at DESC)` index used only by that query.
- Desktop IPC/client/contracts for `listTaskWorkflowRuns`.
- Mac task workflow display logic that loads a list by task id.
- CLI `garyx task get` fetch of `/api/tasks/{id}/workflow-runs`.
- Architecture docs that describe the removed by-task list API.

## Keep

Keep the workflow run model and single-run access paths:

- `workflow_runs.task_id` and `workflow_runs.task_thread_id`, so a run still
  records the owning task.
- `create_workflow_run`.
- `GET /api/workflows/{runId}` and `garyx workflow get`.
- `GET /api/workflows` and `GET /api/threads/{thread_id}/workflows`, because
  those are general workflow and parent-thread queries rather than task-run
  multiplicity.
- Workflow child, event, lifecycle, cancellation, and interrupted-task queries.
- The workflow entrypoint completion check, but change it from "any run for this
  task" to "the task thread id resolves to the expected workflow run".

## Mac UI

The Mac Tasks surface remains the source of truth:

- A workflow task card opens the workflow detail with `task.threadId` as
  `workflowRunId`.
- `WorkflowRunsPanel` becomes a single-run detail surface. It calls
  `getWorkflowRun({ workflowRunId })` and renders that one drilldown.
- The existing route shape `#/workflow/{taskId}` can remain as a user-facing
  task route. When opened from a deep link, the app first resolves the task
  through desktop `getTask`/gateway `GET /api/tasks/{taskId}`, then uses the
  returned `threadId` as the `workflowRunId`.
- The embedded thread workflow detail already passes a `workflowRunId`; it keeps
  using the same single-run component.

## Validation

Focused validation should cover:

- Gateway tests for workflow start/get, entrypoint failure handling, and removal
  of the by-task route test.
- CLI tests for `garyx task get` rendering a single workflow drilldown from
  `GET /api/workflows/{threadId}`.
- Desktop unit/build tests proving task workflow navigation uses task thread id
  and no `listTaskWorkflowRuns` contract remains.
- Manual Mac check: open a workflow task from Tasks and confirm it lands directly
  on the single workflow run detail.
