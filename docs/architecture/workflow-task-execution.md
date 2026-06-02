# Workflow Task Execution Model

This document defines the next Garyx workflow architecture step. It supersedes
earlier designs that treated workflow runs as standalone records or tried to
execute Claude Code workflow scripts directly.

## Core Decision

Workflow has two product meanings:

- `WorkflowDefinition`: a reusable workflow capability, similar to an Agent or
  Team.
- `Task`: one user-visible execution of that workflow.

The workflow's internal work is not made of Tasks. Internal `agent()` calls run
as hidden child threads attached to the workflow run.

In product terms:

```text
Task
  executor = Agent | Team | WorkflowDefinition
```

For a Workflow-backed Task:

```text
Task
  WorkflowRun
    WorkflowNode -> hidden child thread
    WorkflowNode -> hidden child thread
    ...
```

## Non-Goals

- Do not add a Garyx workflow-script interpreter.
- Do not execute Claude Code's generated workflow files as Garyx's primary
  workflow model.
- Do not represent every workflow child call as a Task.
- Do not expose workflow child threads in Recent or default thread lists.

Garyx may match Claude Code workflow process and effect, but the execution
contract is SDK-first: user TypeScript owns control flow while Garyx provides
observable execution primitives.

## Objects

### WorkflowDefinition

`WorkflowDefinition` is the reusable template or capability.

Workflow definitions are global Garyx objects, like Agent and Team
definitions. They are not scoped to a workspace. A definition may carry a
default workspace in its defaults, and a Task execution may pass a workspace as
input or runtime context, but workspace is execution context rather than
definition identity.

Definitions are file-backed workflow packages, not database rows. The package
root contains a manifest named `garyx.workflow.json` plus the entrypoint code
and any supporting files. Listing and fetching workflow definitions read the
configured workflow package directory; installing or updating a definition
copies a package into that directory.

The manifest should contain:

- stable id
- name, description, icon
- input text metadata
- entrypoint
- defaults such as agent id, workspace, concurrency, budget, and timeout
- version
- created/updated metadata

The entrypoint is package-relative and should start simple:

```json
{
  "type": "local_command",
  "command": "node",
  "args": ["workflow.mjs"]
}
```

The command runs ordinary user code. That code imports `@garyx/workflow` and
connects back to the gateway through explicit options or `GARYX_*` environment
variables.

Garyx is a local, single-user deployment. WorkflowDefinition execution does not
need a multi-tenant permission model or RBAC layer. A local-command workflow is
trusted user code running with the user's local privileges, the same as running
the script from the terminal.

Definitions must be versioned. A Task execution must preserve the definition id
and the definition version or snapshot used for that execution, so later edits
to the reusable workflow do not rewrite history.

### Task

Task remains the user-visible unit of work.

Task should gain an executor shape:

```json
{
  "type": "workflow",
  "workflowId": "example",
  "workflowVersion": 1
}
```

Existing Agent/Team execution should fit the same concept:

```json
{ "type": "agent", "agentId": "codex" }
{ "type": "team", "teamId": "team::research" }
```

Task owns product lifecycle:

- title/body/input
- assignee/executor
- notification target
- review state
Task does not own the workflow's phase graph or child-thread result details.

For now, Workflow-backed Tasks use the same review lifecycle as Agent-backed
Tasks: when the WorkflowRun reaches a terminal status (`succeeded`, `failed`,
or `cancelled`), Garyx moves the outer Task to `in_review` if it is still
`in_progress`. This keeps review and notification behavior consistent while the
Workflow UI matures.

### WorkflowRun

`WorkflowRun` is one technical execution of a workflow definition, attached to a
Task.

It should contain:

- workflow run id
- task id
- workflow definition id
- workflow definition version or snapshot
- status: `queued`, `running`, `succeeded`, `failed`, `cancelled`
- input
- result
- error
- started/finished timestamps
- aggregate cost, token, and timing metrics

A Task can have multiple workflow runs over time when the user retries. The
latest run is the active execution, but old runs remain audit history.

### WorkflowNode

`WorkflowNode` is one internal execution node inside a workflow run. The common
node is an SDK `agent()` call.

It should contain:

- node id / child run id
- workflow run id
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
- be openable from Task/WorkflowRun detail
- retain provider transcript, tool calls, and structured result evidence
- carry top-level classification fields so shallow metadata writes cannot
  re-expose them

## Status Mapping

Task status is the product/review state. WorkflowRun status is execution state.
They should not be collapsed into one enum.

Recommended mapping:

```text
Task in_progress
  WorkflowRun running

WorkflowRun succeeded
  -> Task in_review

WorkflowRun failed
  -> Task in_review with execution error details

WorkflowRun cancelled
  -> Task in_review
```

Failure should still return the Task for human review. Re-running a workflow is
a future product action; it must not mutate prior run history.

## Execution Flow

1. User creates a Task and chooses a WorkflowDefinition as the executor.
2. Gateway reads the file-backed workflow package and snapshots the selected
   manifest/version onto the Task execution context.
3. Gateway starts the definition entrypoint as a process from the package
   directory with environment:

```text
GARYX_TASK_ID
GARYX_TASK_THREAD_ID
GARYX_PARENT_THREAD_ID
GARYX_WORKFLOW_DEFINITION_ID
GARYX_WORKFLOW_DEFINITION_VERSION
GARYX_WORKFLOW_DEFINITION_SNAPSHOT
GARYX_WORKFLOW_DIR
GARYX_WORKFLOW_INPUT_JSON
GARYX_GATEWAY_URL
GARYX_GATEWAY_TOKEN
GARYX_WORKSPACE_DIR
GARYX_WORKFLOW_SDK_IMPORT
```

4. User code imports `@garyx/workflow`.
5. SDK creates the WorkflowRun for the Task-launched entrypoint and attaches
   `task_id`, `task_thread_id`, definition id, definition version, and the
   manifest snapshot.
6. SDK calls such as `phase()`, `agent()`, `parallel()`, and `pipeline()` write
   observable events and create hidden child threads through gateway APIs.
7. Final SDK return value becomes the WorkflowRun result.
8. Gateway maps the terminal WorkflowRun status back onto the Task review state.

The gateway does not interpret the user's workflow source code. It launches the
entrypoint and records what the SDK reports.

## UI Model

Task detail is the primary user surface.

For a Workflow-backed Task, the detail should show:

```text
Task #123 - Run workflow

Status: In Review
Executor: Workflow / Example v1
Body/Input: ...

Workflow Run
  Plan         succeeded   1 child
  Implement    succeeded   1 child
  Review       succeeded   1 child

Result
  status
  output
```

Each phase can expand into node rows:

```text
Search
  search: adoption trends        succeeded   open thread
  search: REST critique          succeeded   open thread
  search: GraphQL cost           succeeded   open thread
```

The user sees the Task first, the WorkflowRun second, and child threads only as
drilldown evidence.

## API And CLI Shape

WorkflowDefinition management should be separate from WorkflowRun execution.

Suggested API groups:

- `GET /api/workflow-definitions`
- `GET /api/workflow-definitions/{id}`
- `POST /api/tasks` with `executor.type = "workflow"`
- `GET /api/tasks/{id}/workflow-runs`
- `GET /api/workflows/{runId}`
- `GET /api/workflows/{runId}/events`

Suggested CLI:

```bash
garyx workflow definition list
garyx workflow definition upsert --file ./my-workflow
garyx task create --workflow example --input-json '{"goal":"run"}'
garyx task get '#TASK-123'
garyx workflow get run-abc
```

The existing SDK run APIs can remain as the low-level run plumbing, but the
product entrypoint should be Task creation with a Workflow executor.

## Persistence Notes

Do not store reusable WorkflowDefinition rows in the runtime database. The
definition source of truth is the workflow package on disk.

The database stores execution history:

Task records should store the selected executor and workflow definition
snapshot. WorkflowRun rows should point to the Task and to the definition
snapshot/version.

Existing workflow run/event/child tables can continue to store execution
details, but they should gain a first-class `task_id` relationship.

## Open Decisions

- Whether workflow package install/update should remain CLI-only or gain a Mac
  management UI.
Not open: WorkflowDefinition is global, and Garyx does not add
workflow-specific permissions or multi-user authorization checks for this local
single-user model.

## Current Implementation Status

Implemented:

1. File-backed workflow definition packages with `garyx.workflow.json`.
2. CLI install/list/get for definitions through the gateway.
3. Task executor metadata for Agent, Team, and Workflow.
4. Workflow-backed Task launch through the package entrypoint.
5. SDK-created WorkflowRuns linked to the outer Task.
6. Hidden child threads for internal `agent()` nodes.
7. Task drilldown over WorkflowRuns, children, and events.
8. Mac task creation surface with Agent / Agent Team / Workflow executor tabs.

Next:

1. Add a Mac workflow package management surface if CLI-only management becomes
   too narrow.
2. Continue promoting small real workflows as file-backed workflow packages
   without moving their business policy into the gateway or SDK.
