# @garyx/workflow

TypeScript SDK for observable Garyx workflows.

Garyx-managed workflows are file packages with a fixed `workflow.ts` file.
Garyx runs that file with Bun for workflow execution threads. When a workflow is
started from Tasks, the Task wraps that same thread for review and notification
management. The SDK connects each run to Garyx for observability and agent
child-thread execution.

Code you run yourself is still just ordinary TypeScript or JavaScript. It can
import this SDK and connect to Garyx without a manifest or Garyx-managed
runtime. In that mode the gateway creates a workflow thread automatically when
the SDK starts.

```ts
import { defineWorkflow, s } from "@garyx/workflow";

const Finding = s.object<{ summary: string; risk: "low" | "high" }>({
  summary: s.string(),
  risk: s.enum(["low", "high"] as const),
});

await defineWorkflow({
  name: "Review change",
  agent: "claude",
  phases: ["Review"],
  output: (result) => `Reviewed ${result.findings.length} surfaces.`,
  async run(flow) {
    await flow.log("review.started", { owner: "Test User" });
    const review = flow.phase("Review").start();
    const findings = await review.parallel([
      () => review.agent("api", "Review the API surface", { schema: Finding }),
      () => review.agent("mobile", "Review the mobile behavior", { schema: Finding }),
    ]);
    return { workflowRunId: flow.ctx.workflowRunId, findings };
  },
});
```

Environment variables:

- `GARYX_GATEWAY_URL` defaults to `http://127.0.0.1:31337`.
- `GARYX_GATEWAY_TOKEN` is sent as a bearer token when present.
- `GARYX_WORKFLOW_THREAD_ID` / `GARYX_WORKFLOW_RUN_ID` identify the workflow
  execution thread when Garyx launches the process.
- `GARYX_TASK_ID` and `GARYX_TASK_THREAD_ID` are internal Task wrapper context
  when the workflow was started from Tasks.
- `GARYX_WORKSPACE_DIR` is used as the default workspace for child agents.
- `GARYX_WORKFLOW_RUN_ID` is set by `workflow()`. `GARYX_WORKFLOW_ID` is also
  set as a compatibility alias.

Normal Task-launched workflows should rely on the gateway-provided
environment. `gatewayUrl`, `gatewayToken`, and `workspaceDir` may be passed
explicitly for local tests; thread and Task identity are managed by the SDK and
gateway.

`defineWorkflow()` is the recommended authoring API. The older primitives
`workflow()`, `phase()`, `agent()`, `parallel()`, `pipeline()`, and `schema()`
remain exported for low-level or compatibility use.

`flow.phase(name)` returns a phase-scoped handle. Agents launched through the
handle inherit the phase, workspace, and default child agent, so workflow code
maps cleanly to the UI shape: phase -> agent -> result.

`pipeline()` starts one chain per input item. Each item flows through its stages
in order, while different items run concurrently:

```ts
const reports = await flow.phase("Research").pipeline(
  ["api", "mobile"],
  (topic) => flow.phase("Search").agent(`search:${topic}`, `Search for ${topic}`),
  (search) => flow.phase("Synthesize").agent("synthesize", `Summarize: ${JSON.stringify(search)}`),
);
```

Complex workflows are ordinary TypeScript that call these primitives. Garyx
records the workflow run id, phase events, child threads, structured child
results, and final output while your process owns the control flow.

The workflow return value is stored as machine-readable `result`. The optional
human-readable final text is stored separately as `outputText` and rendered as
Markdown in Garyx. Provide it explicitly with `output: (result, ctx) => string`,
return a string from the workflow, or return an object with `outputText`,
`output`, or `markdown`. Do not rely on business fields such as `summary` to
become the visible Workflow output.

## CLI Workflow Task Flow

Reusable workflow definitions are global file-backed packages. The gateway reads
workflow definitions from the local workflow root rather than treating the
database as the source of truth. A Garyx-managed package contains
`garyx.workflow.json`, a fixed `workflow.ts` file, and any support files it
references:

```text
~/.garyx/workflows/smoke/
  garyx.workflow.json
  workflow.ts
```

`garyx.workflow.json`:

```json
{
  "workflowId": "smoke",
  "version": 1,
  "name": "Smoke Workflow",
  "description": "Minimal package workflow used to verify CLI execution",
  "input": {
    "placeholder": "What should this workflow do?"
  },
  "defaults": {
    "workspaceDir": "/Users/test/project"
  }
}
```

```sh
garyx workflow definition upsert --file /Users/test/project/smoke-workflow
garyx workflow definition list
garyx workflow definition get smoke
```

`upsert --file` accepts either the package directory or the package manifest. It
installs the package into the configured Garyx workflow root. Garyx always runs
`workflow.ts` from that package with its built-in Bun runtime.

Run it as the executor for a Task. No Agent assignee is required. This creates a
workflow execution thread and wraps it in Task review metadata. The default CLI
input and the product UI Task body model are plain text:

```sh
garyx task create \
  --title "Run smoke workflow" \
  --workflow smoke \
  --input "smoke test" \
  --workspace-dir /Users/test/project \
  --notify none
```

`--input` is a single plain-text string (for a larger prompt, pass it inline or
via your shell, e.g. `--input "$(cat prompt.md)"`). Reusable workflow
definitions should describe user-facing input with an `input` text metadata
object, not an input contract. If a user-facing workflow needs structured data,
make the first workflow step structure the plain-text request.

`workflow.ts` receives `GARYX_WORKFLOW_THREAD_ID`, `GARYX_WORKFLOW_RUN_ID`,
`GARYX_TASK_ID`, `GARYX_TASK_THREAD_ID`, `GARYX_PARENT_THREAD_ID`,
`GARYX_WORKFLOW_DEFINITION_ID`,
`GARYX_WORKFLOW_DEFINITION_VERSION`, `GARYX_WORKFLOW_DEFINITION_SNAPSHOT`,
`GARYX_WORKFLOW_INPUT_JSON`, `GARYX_WORKFLOW_ARGS`, `GARYX_GATEWAY_URL`, and
`GARYX_GATEWAY_TOKEN` when configured. `GARYX_WORKFLOW_DIR` is the installed
package directory. `GARYX_WORKSPACE_DIR` is set from `--workspace-dir`, then
from `input.workspaceDir` / `input.workspace_dir`, then from
`defaults.workspaceDir` / `defaults.workspace_dir`. The SDK exposes the task
input as `ctx.input`, sourced from `GARYX_WORKFLOW_INPUT_JSON`.

Observe the run from the Task and workflow surfaces:

```sh
garyx task get '#TASK-123'
garyx workflow get run-abc
garyx workflow events run-abc
```

## Development Review Loop Example

`examples/development-loop/` is a reusable workflow package for coding tasks.
It orchestrates one normal engineering pass with observable child agents:

1. planner agent reads the task and returns a structured plan,
2. implementer agent edits the workspace and runs focused validation,
3. reviewer agent performs a read-only code review.

Install the workflow package:

```sh
garyx workflow definition upsert --file packages/garyx-workflow/examples/development-loop
garyx workflow definition get development-loop
```

Run the implementation/review loop with a plain-text goal:

```sh
garyx task create \
  --title "Implement Mac workflow UI" \
  --workflow development-loop \
  --workspace-dir /Users/test/project \
  --input "Implement the Mac app workflow management surface" \
  --notify none
```

`--input` is a single plain-text string, which the workflow uses as its goal.
Advanced options (a dry-run mode, target paths, validation commands, or
role-specific child agents) are only available to programmatic SDK callers that
invoke the workflow with an object input; they are not a CLI input format.

Garyx-managed workflow processes resolve `@garyx/workflow` through the gateway's
runtime package. User-run scripts may instead install this package however they
prefer.

When the SDK finishes with `succeeded`, `failed`, or `cancelled`, the outer
Task moves to `in_review`.

## Deep Research Example

`examples/deep-research/` mirrors Claude Code's deep-research workflow shape:

1. Scope the question into 3-6 complementary search angles.
2. Run capped parallel web-search agents.
3. Deduplicate URLs, fetch up to 10 sources by default, and extract falsifiable
   claims with direct quotes.
4. Verify up to 12 highest-value claims by default with 3 independent
   adversarial votes; 2 refuting votes kill a claim. Verifier fan-out is capped
   separately from the number of votes.
5. Synthesize surviving claims into a Markdown report with a conclusion,
   numbered citations, transparent refutations, caveats, open questions, and
   stats.

Install and run it:

```sh
garyx workflow definition upsert --file packages/garyx-workflow/examples/deep-research
garyx task create \
  --title "Research API architecture" \
  --workflow deep-research \
  --workspace-dir /Users/test/project \
  --input "Should a new 2026 product API default to GraphQL or REST?" \
  --notify none
```

For a cheap validation run, keep the workflow input as text and lower the knobs
with environment variables:

```sh
GARYX_DEEP_RESEARCH_MAX_FETCH=2 \
GARYX_DEEP_RESEARCH_MAX_VERIFY_CLAIMS=2 \
GARYX_DEEP_RESEARCH_VOTES_PER_CLAIM=2 \
GARYX_DEEP_RESEARCH_VOTE_CONCURRENCY=1 \
garyx task create \
  --title "Smoke deep research" \
  --workflow deep-research \
  --workspace-dir /Users/test/project \
  --input "Compare Garyx workflow observability with Claude Code deep research." \
  --notify none
```
