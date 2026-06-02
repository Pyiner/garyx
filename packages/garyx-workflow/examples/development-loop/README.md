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

## Dry Run

Dry run exercises the file-backed package, Task shell, SDK startup/finish, and
observability without launching child agents:

```sh
garyx task create \
  --title "Dry run development workflow" \
  --workflow development-loop \
  --workspace-dir /Users/test/project \
  --input-json '{"goal":"verify the workflow package shape","mode":"dry_run","targetSurface":"mac_app"}' \
  --notify none
```

## Real Run

Text input is the default path for simple CLI use. In product UI, the Task body
is used as the workflow text input:

```sh
garyx task create \
  --title "Implement Mac workflow UI" \
  --workflow development-loop \
  --workspace-dir /Users/test/project \
  --input "Implement the Mac app workflow management surface" \
  --notify none
```

Use JSON only for advanced knobs such as target paths, validation hints, or
role-specific child agents:

```sh
garyx task create \
  --title "Implement Mac workflow UI" \
  --workflow development-loop \
  --workspace-dir /Users/test/project \
  --input-json '{
    "goal": "Implement the Mac app workflow management surface",
    "targetSurface": "mac_app",
    "targetPaths": ["desktop/garyx-desktop", "garyx-gateway/src/workflows.rs"],
    "validationCommands": ["cd desktop/garyx-desktop && npm run build:ui"],
    "childAgentId": "claude"
  }'
```

`childAgentId` is used as the default for planner, implementer, and reviewer.
Override `plannerAgentId`, `implementerAgentId`, or `reviewerAgentId` when you
want distinct profiles.
