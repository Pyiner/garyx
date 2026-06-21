# Workflow Thread Execution Model

This document defines Garyx workflow execution after the thread-first
architecture change. It supersedes earlier designs that treated workflow runs as
standalone records under Tasks.

## Core Decision

Thread is the canonical execution object.

Workflow has two product meanings:

- `WorkflowDefinition`: a reusable workflow capability, parallel to an Agent or
  Team.
- `WorkflowExecutionThread`: one execution of a workflow definition. Its thread
  id is the workflow run id.

Task is an optional management wrapper around a thread. A Task can wrap a
workflow execution thread for review state, notification target, title/body, and
task-list management. Task is not the execution identity.

In product terms:

```text
Thread
  executor = Agent | Team | WorkflowDefinition

Task
  wraps Thread
```

For a workflow execution:

```text
WorkflowExecutionThread
  WorkflowNode -> hidden child Thread
  WorkflowNode -> hidden child Thread
  ...
```

The workflow execution thread appears in thread lists. Internal child threads
remain hidden from Recent/default thread lists and are opened only as drilldown
evidence.

## Non-Goals

- Do not add a Garyx workflow-script interpreter.
- Do not execute Claude Code generated workflow files as Garyx's primary
  workflow model.
- Do not represent workflow child calls as Tasks.
- Do not expose workflow child threads in Recent or default thread lists.
- Do not require SDK users to pass thread ids, task ids, or run ids by hand.

Garyx may match Claude Code workflow process and effect, but the execution
contract is SDK-first: user TypeScript owns control flow while Garyx provides
observable execution primitives.

## Objects

### WorkflowDefinition

`WorkflowDefinition` is the reusable template or capability.

Definitions are global Garyx objects, like Agent and Team definitions. They are
not scoped to a workspace. A definition may carry default workspace metadata,
and an execution may pass a workspace as input or runtime context, but workspace
is execution context rather than definition identity.

Definitions are file-backed workflow packages, not database rows. The package
root contains `garyx.workflow.json`, a fixed `workflow.ts` entrypoint, and any
supporting files. Listing and fetching workflow definitions read the configured
workflow package directory; installing or updating a definition copies a package
into that directory.

The manifest contains:

- stable id
- name, description, icon
- input text metadata
- defaults such as agent id, workspace, concurrency, budget, and timeout
- version
- created/updated metadata

### WorkflowExecutionThread

`WorkflowExecutionThread` is a normal Garyx thread with workflow execution
classification:

```json
{
  "thread_kind": "workflow_run",
  "workflow_run_id": "thread::...",
  "workflow_definition_id": "development-loop",
  "workflow_definition_version": 1,
  "workflow_status": "running"
}
```

The workflow run id is the thread id. API payloads may continue to return
`workflowRunId` and `workflowId` aliases, but both identify the same thread.

The execution thread owns:

- execution identity
- workspace context
- workflow definition id/version/snapshot
- input
- status: `queued`, `running`, `succeeded`, `failed`, `cancelled`
- result, summary, and error
- aggregate cost, token, and timing metrics

Implementation may keep workflow event and child-run tables as durable execution
ledgers, but those rows are keyed by the workflow thread id. They are not a
separate workflow-run identity.

### Task

Task remains a user-visible management wrapper.

Task owns product lifecycle:

- title/body/input
- review status
- notification target
- task-list indexing
- optional source metadata

Task can wrap a workflow execution thread, an agent thread, or a team thread.
For a Workflow-backed Task, the Task's backing thread is the workflow execution
thread itself.

Task status is the product/review state. Workflow status is execution state.
They must not collapse into one enum.

Recommended mapping:

```text
Task in_progress
  WorkflowExecutionThread running

WorkflowExecutionThread succeeded
  -> Task in_review

WorkflowExecutionThread failed
  -> Task in_review with execution error details

WorkflowExecutionThread cancelled
  -> Task in_review
```

Failure should still return the Task for human review. A retry is a new workflow
execution thread, which may be wrapped by a new or updated product action later.
It must not rewrite the prior execution thread.

### WorkflowNode

`WorkflowNode` is one internal execution node inside a workflow thread. The
common node is an SDK `agent()` call.

It stores:

- node id / child run id
- workflow thread id
- phase title
- label
- order index
- status
- child thread id
- structured schema, if any
- structured result or error
- started/finished timestamps

Nodes are not Tasks. Nodes are workflow execution evidence.

### Child Thread

Each `agent()` node uses a child thread as the transcript carrier.

Child threads must:

- be hidden from Recent and default thread lists
- be openable from Workflow detail
- retain provider transcript, tool calls, and structured result evidence
- carry top-level classification fields so shallow metadata writes cannot
  re-expose them

## Execution Flow

### Task-launched workflow

1. User creates a Task and chooses a WorkflowDefinition as the executor.
2. Gateway creates one backing thread for that Task.
3. The backing thread is classified as `thread_kind = workflow_run`.
4. Gateway launches the workflow package's fixed `workflow.ts` with built-in Bun.
5. Gateway injects `GARYX_WORKFLOW_THREAD_ID` and `GARYX_WORKFLOW_RUN_ID` using
   the backing thread id, plus internal Task wrapper context.
6. User code imports `@garyx/workflow`.
7. SDK calls `/api/workflows/sdk`.
8. Gateway reuses the workflow thread, records execution state keyed by the
   thread id, and returns `workflowRunId == threadId`.
9. SDK calls such as `phase()`, `agent()`, `parallel()`, and `pipeline()` write
   observable events and create hidden child threads through gateway APIs.
10. Final SDK return value becomes the workflow thread result.
11. Gateway maps terminal workflow status back onto the Task review state.

### Direct SDK workflow

1. User runs ordinary TypeScript or JavaScript that imports `@garyx/workflow`.
2. User does not pass thread id, task id, or run id.
3. SDK calls `/api/workflows/sdk`.
4. Gateway creates a new `WorkflowExecutionThread`.
5. The rest of the execution is identical to a Task-launched workflow, except
   no Task wrapper is present.

## Environment

Garyx-managed workflow processes receive:

```text
GARYX_WORKFLOW_THREAD_ID
GARYX_WORKFLOW_RUN_ID
GARYX_TASK_ID
GARYX_TASK_THREAD_ID
GARYX_PARENT_THREAD_ID
GARYX_WORKFLOW_DEFINITION_ID
GARYX_WORKFLOW_DEFINITION_VERSION
GARYX_WORKFLOW_DEFINITION_SNAPSHOT
GARYX_WORKFLOW_DIR
GARYX_WORKFLOW_INPUT_JSON
GARYX_WORKFLOW_ARGS
GARYX_GATEWAY_URL
GARYX_GATEWAY_TOKEN
GARYX_WORKSPACE_DIR
```

Only the launcher sets identity variables. SDK users should not provide them.

## UI Model

The thread list shows workflow execution threads.

Task detail remains a useful management surface when the execution was launched
from Tasks. For a Workflow-backed Task, the detail should show the wrapped
workflow thread first, then phase/node drilldown:

```text
Task #123 - Run workflow

Status: In Review
Wrapped thread: thread::...
Executor: Workflow / Example v1
Body/Input: ...

Workflow
  Plan         succeeded   1 child
  Implement    succeeded   1 child
  Review       succeeded   1 child
```

Each phase can expand into node rows:

```text
Search
  search: adoption trends        succeeded   open child thread
  search: REST critique          succeeded   open child thread
  search: GraphQL cost           succeeded   open child thread
```

## API And CLI Shape

WorkflowDefinition management remains separate from workflow execution.

API groups:

- `GET /api/workflow-definitions`
- `GET /api/workflow-definitions/{id}`
- `POST /api/workflows/sdk` creates or reuses a workflow execution thread
- `POST /api/tasks` with `executor.type = "workflow"` creates a Task-wrapped
  workflow execution thread
- `GET /api/tasks/{id}` returns the task's backing `thread_id`
- `GET /api/workflows/{threadId}` returns the wrapped workflow thread
- `GET /api/workflows/{threadId}/events`

CLI:

```bash
garyx workflow definition list
garyx workflow definition upsert --file ./my-workflow
garyx task create --workflow example --input "run this"
garyx task get '#TASK-123'
garyx workflow get thread::abc
```

`workflowRunId` in CLI/API output is the workflow thread id.

## Persistence Notes

Do not store reusable WorkflowDefinition rows in the runtime database. The
definition source of truth is the workflow package on disk.

Thread records are the source of truth for execution identity and thread-list
presence. Workflow event and child-run tables store execution evidence keyed by
the workflow thread id.

Existing workflow ledger tables may keep their historical names for now, but
their `workflow_id` value is the workflow thread id. No caller should depend on
a standalone workflow-run id that differs from the thread id.
