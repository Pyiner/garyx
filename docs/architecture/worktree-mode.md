# Worktree Mode Design Notes

This document records the confirmed product and implementation decisions for
Garyx worktree mode.

## Goal

Garyx should support an explicit worktree mode for newly created threads and
tasks. When enabled, Garyx creates a git worktree for the selected repository,
stores that worktree path as the thread execution `workspace_dir`, and runs the
agent in that isolated worktree.

This is separate from Codex sandboxing. Codex `workspace-write` controls file
permissions around a process. Garyx worktree mode creates a separate git
working directory and then passes that directory as `cwd` / `workspace_dir`.

## Confirmed Decisions

- Worktree mode is opt-in. Existing thread and task creation continue to use
  direct workspace mode by default.
- Worktree mode applies only to new thread creation and task creation. It does
  not affect sending messages to existing threads.
- A thread's execution directory remains fixed after the thread is created.
  Passing a worktree option later must not create another worktree or mutate
  the thread workspace.
- Task creation does not own or separately manage worktrees. A task only
  forwards the worktree request to the backing thread creation flow.
- The durable relationship is `thread -> worktree`, stored on the thread.
- Garyx continues to use `workspace_dir` as the execution directory source of
  truth. No separate workspace entity is introduced.
- Worktree metadata is stored on the thread record in a top-level `worktree`
  field, not only inside generic provider metadata and not on the task record.
- The selected `workspace_dir` must itself be a git repository root. If the
  selected directory is only a subdirectory inside a git repository, worktree
  mode must fail or be hidden in UI.
- The selected repository must have at least one commit because worktree mode
  is based on the repository's current `HEAD`.
- Dirty source repositories are allowed. The worktree is created from the
  current branch's current `HEAD`; uncommitted changes are not copied into the
  worktree.
- Garyx must record enough metadata to make that behavior clear, including
  source repository root, source branch, and base commit.
- Worktrees are not automatically deleted when a thread is deleted or a task is
  completed.
- Worktree creation does not set upstream tracking and does not push branches.
- If worktree creation fails, the whole create operation fails. Garyx should
  roll back the newly created thread/task where possible, and surface rollback
  errors instead of silently swallowing them.

## CLI And API

API should use an explicit workspace mode field:

```json
{
  "workspace_mode": "direct"
}
```

Supported values:

- `direct`
- `worktree`

Default is `direct`.

CLI should expose worktree mode as a boolean flag on explicit create commands:

```bash
garyx thread create --workspace-dir /path/to/repo --worktree
garyx task create --workspace-dir /path/to/repo --worktree
```

Bot channel accounts can also set `workspace_mode=worktree`; newly created
inbound threads and bot-local `/newthread` commands use that account default.
`thread send` does not change an existing thread's workspace mode.

If `--worktree` / `workspace_mode=worktree` is used and `workspace_dir` is not
a git repository root, or the repository has no `HEAD` commit, Garyx returns an
error.

## Git Repository Detection

Mac app needs a lightweight API to decide whether to show worktree mode for the
currently selected workspace. Proposed endpoint:

```http
GET /api/workspaces/git-status?workspace_dir=/path/to/repo
```

Proposed response:

```json
{
  "workspace_dir": "/path/to/repo",
  "is_git_repo": true,
  "repo_root": "/path/to/repo",
  "current_branch": "main",
  "is_dirty": false
}
```

Important semantics:

- `is_git_repo=true` only when the requested `workspace_dir` is the repository
  root.
- If `workspace_dir` is inside a parent git repo but is not the repo root,
  return `is_git_repo=false` for worktree UI purposes.
- The Mac app uses this to decide whether to expose worktree mode.

## Worktree Path

Managed worktrees should live under Garyx-owned storage:

```text
~/.garyx/worktrees/<repo-hash>/<thread-id-safe>
```

The final path segment is derived from the thread id using a filesystem-safe
form. Example:

```text
~/.garyx/worktrees/6f3a2d10/thread--21225bfb-9198-4d1b-8e3c-6b5fa45ea9ac
```

The original thread id is stored in metadata.

## Branch Naming

Garyx creates a local branch for the worktree:

```text
garyx/<short-thread-id>
```

`<short-thread-id>` uses the first 8 hex characters from the backing thread id
UUID. Example:

```text
garyx/21225bfb
```

If a branch already exists, Garyx should not reuse it blindly. It may append a
small suffix, for example:

```text
garyx/21225bfb-2
```

No upstream is configured.

## Creation Flow

Thread creation in worktree mode:

1. Generate the thread id without persisting the thread record yet.
2. Validate that the requested `workspace_dir` is a git repository root with a
   valid `HEAD`.
3. Compute the managed worktree path and branch name from the thread id.
4. Run git worktree creation from the selected repository.
5. Store the thread record with `workspace_dir` set to the created worktree
   path and top-level `worktree` metadata.
6. If worktree creation partially creates files and then fails, rollback the
   partial worktree where possible and return an error.

Task creation in worktree mode:

1. Task creation passes the worktree request into backing thread creation.
2. Backing thread creation follows the same thread worktree flow.
3. Task state remains unchanged beyond its existing backing thread relation.

Bot inbound thread creation in worktree mode:

1. The channel account supplies `workspace_dir` and `workspace_mode`.
2. Router thread creation forwards the same worktree request used by explicit
   thread creation.
3. Existing bound bot threads keep their original immutable `workspace_dir`.

## Git Command Semantics

The worktree should be based on the current default behavior of git worktree
creation from the selected repository and current `HEAD`.

Conceptually:

```bash
git -C /path/to/repo worktree add -b garyx/<short-thread-id> <worktree-path>
```

Garyx should record:

- `source_workspace_dir`
- `source_repo_root`
- `source_branch`
- `base_head`
- `path`
- `branch`
- `created_at`
- original `thread_id`

Example thread fields:

```json
{
  "workspace_dir": "/Users/test/.garyx/worktrees/6f3a2d10/thread--21225bfb-9198-4d1b-8e3c-6b5fa45ea9ac",
  "worktree": {
    "mode": "worktree",
    "source_workspace_dir": "/Users/test/repos/example",
    "source_repo_root": "/Users/test/repos/example",
    "source_branch": "main",
    "base_head": "abc123",
    "path": "/Users/test/.garyx/worktrees/6f3a2d10/thread--21225bfb-9198-4d1b-8e3c-6b5fa45ea9ac",
    "branch": "garyx/21225bfb",
    "thread_id": "thread::21225bfb-9198-4d1b-8e3c-6b5fa45ea9ac",
    "created_at": "2026-05-13T00:00:00Z"
  }
}
```

## Mac App UI Status

Confirmed:

- The Mac app should show worktree mode only when the selected workspace is a
  git repository root.
- Default remains direct mode.
- The app should not expose worktree mode for non-git workspaces or git
  subdirectories.
- Empty state layout should align with Codex Mac app: when no thread is
  selected, place the primary composer in the middle of the content area rather
  than keeping it in the normal thread-bottom position.
- The composer and its context controls should follow Codex's structure as
  closely as possible, with Garyx-specific options removed.
- Workspace/mode controls belong below the input box, not inside the input
  itself.
- Garyx has only local execution and worktree mode. It should not show Codex's
  cloud option.
- Garyx should not expose manual branch management in this UI. Branch creation
  remains automatic via the backend rule `garyx/<short-thread-id>`.
- A "resume by id" action should sit below the input on the lower-right side,
  visually symmetric with the context selectors below the input. It should use
  the same small selector/button styling and open a modal when clicked.

Observed Codex Mac app behavior:

- Codex shows project, startup mode, and branch selectors under the composer.
- The startup mode menu includes local processing, new worktree, and cloud.
- Selecting new worktree adds an environment selector and keeps branch visible.
- If no project is selected, the project/runtime selector group disappears.

Garyx should implement a simplified Codex-style version:

```text
[ centered composer ]

[ workspace selector ] [ mode selector: Local | New worktree ]        [ resume by id ]
```

Details:

- Show the workspace selector whenever a workspace can be selected for the new
  thread/task.
- Show the mode selector only when the selected workspace is a git repository
  root.
- Mode selector default is `Local`.
- Mode selector values are `Local` and `New worktree`.
- Hide the mode selector for non-git workspaces and git subdirectories.
- Do not show a branch selector.
- Do not show a cloud selector.
- Do not show environment selection in the first version.
- Before implementation, inspect Codex Mac app through CDP again and match the
  empty-state composer spacing, selector placement, and button treatment as
  closely as possible, then remove unsupported Codex options.
