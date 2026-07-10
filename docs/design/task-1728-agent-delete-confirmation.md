# Task 1728: Mac Agent Delete Confirmation And Catalog Refresh

## Evidence

- Gateway/API path is not the reproduced failure. A synthetic custom agent
  `task-1728-delete-repro` was created through the real gateway, appeared in
  `GET /api/custom-agents`, and was present in the persisted
  `custom-agents.json` state. `garyx agent delete task-1728-delete-repro
  --json` returned `{}`; immediate and delayed `GET /api/custom-agents` checks
  returned zero matching rows, and the persisted state no longer contained the
  synthetic id.
- CDP against the running packaged Mac app reproduced the frontend stale-state
  symptom. Before deletion, the new-thread Agent submenu contained
  `Task 1728 Delete Repro`. Deleting from the Agents page removed the row from
  the Agents page and `window.garyxDesktop.listCustomAgents()` returned zero
  matching rows, but the new-thread Agent submenu still contained
  `Task 1728 Delete Repro`.
- The same CDP run confirmed there is no confirmation dialog today:
  selecting the destructive `Delete` menu item immediately deleted the agent,
  and `[role="alertdialog"]` was absent.

## Root Cause

`AgentsHubPanel` owns a local `agents` list and refreshes it with `loadData()`
after create/update/delete. `AppShell` owns the shared `desktopAgents` catalog
used by the new-thread composer, task forms, capsule/task identity helpers, and
other root-level consumers. The Agents page delete path does not notify
`AppShell` to refresh `desktopAgents`, so the page-local row disappears while
root-level consumers can keep offering the deleted agent until another catalog
refresh happens.

The gateway delete endpoint does remove the custom agent from memory, persisted
state, and bridge profiles. No gateway resurrection path was reproduced.

## Fix

- Add a narrow `onRefreshAgentTargets?: () => Promise<void>` callback to
  `AgentsHubPanel`.
- Pass `refreshAgentTargets` from `AppShell` when rendering the Agents hub.
- After a successful custom-agent delete, close any agent dialog, refresh the
  hub-local data, then invoke `onRefreshAgentTargets` so `desktopAgents` and
  the agent catalog is reloaded from the gateway.
- Keep errors scoped: if the API delete fails, do not mutate either local or
  parent catalog. If the parent refresh fails, keep the delete success visible
  and surface the existing toast/error behavior only if the callback throws.

## Confirmation Dialog

- Replace the direct destructive dropdown action with opening a small
  Dialog/AlertDialog-style confirmation state owned by `AgentsHubPanel`.
- The dialog copy names the agent and states that deletion is permanent and
  cannot be undone.
- Cancel closes the dialog and must not call `deleteCustomAgent`.
- Confirm runs the existing delete flow while disabling the confirm button
  during `saving`.
- Use existing desktop dialog primitives and button styles; do not introduce a
  new UI abstraction.

## Validation Plan

- Add a focused renderer unit test around the delete confirmation/controller
  behavior: cancel does not call delete, confirm calls delete and invokes the
  parent refresh callback after success.
- Run `cd desktop/garyx-desktop && npm run test:unit`.
- Because renderer behavior changes, run `cd desktop/garyx-desktop && npm run
  dist:dir`, relaunch the installed app, attach via CDP, and prove:
  cancel leaves the synthetic agent in API/list state, confirm deletes it,
  the Agents page row disappears, the gateway API no longer lists it, and the
  new-thread Agent submenu no longer contains it.
