# Development Review Loop

Observable Garyx workflow for coding work:

1. plan the change,
2. implement the plan,
3. review the diff.

The workflow is SDK-first. The control flow is normal TypeScript in
`workflow.ts`; Garyx provides the Task shell, child-agent execution, structured
results, and event log. Garyx-managed runs execute `workflow.ts` with the
built-in Workflow runtime, so the file can import `@garyx/workflow` directly.

## Install

```sh
garyx workflow definition upsert --file packages/garyx-workflow/examples/development-loop
garyx workflow definition get development-loop
```

## Run

`--input` is a single plain-text string, which the workflow uses as its goal.
In product UI, the Task body is used as that text input:

```sh
garyx task create \
  --title "Implement Mac workflow UI" \
  --workflow development-loop \
  --workspace-dir /Users/test/project \
  --input "Implement the Mac app workflow management surface" \
  --notify none
```

Advanced options (a dry-run mode, target paths, validation commands, or
role-specific child agents) are only available to programmatic SDK callers that
invoke the workflow with an object input; they are not a CLI input format.
