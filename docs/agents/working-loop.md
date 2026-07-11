# Working Loop

## Default Loop

1. Read the local code around the change before editing.
2. Keep the change scoped to the smallest correct surface.
3. Prefer existing crate and UI patterns over new abstractions.
4. Run focused deterministic tests for the touched area.
5. Update `docs/configuration.md` when user-facing configuration, CLI flags,
   environment variables, config-file schema, or setup behavior changes.
6. Commit every completed code change before handoff. Stage only files changed
   for the current task and leave unrelated user work untouched.

## Development Worktree Cleanup

Dedicated worktrees created to implement or review a Garyx task are temporary.
The task creator or orchestrator owns their cleanup; a worker cannot reliably
remove the worktree that is still its own process directory.

After the result is explicitly approved:

1. Confirm no agent, reviewer, Cargo command, or other process is still using
   the worktree.
2. Inspect the worktree status and preserve every required change as a commit.
   Resolve dirty state instead of forcing deletion over unpreserved work.
3. From the parent checkout, run `git worktree remove <worktree-path>`. This
   removes the checkout and its local build artifacts, including Rust
   `target/`, while retaining the task branch.
4. Run `git worktree prune`, then verify the path no longer appears in
   `git worktree list --porcelain`.
5. Only after that verification, mark the task `done`.

Do not leave an approved/done task's worktree around as a build cache. Shared
compiler caching must be handled separately; task worktrees remain ephemeral.

## Desktop Dev Mode

For fast macOS UI iteration, run the Electron dev build:

```bash
cd desktop/garyx-desktop && npm run dev
```

This launches the Garyx Mac app in development mode. Renderer changes are
visible directly in the running Mac app as you edit, so use this mode for quick
visual and interaction feedback.

Prefer showing the user this dev client during normal UI iteration because code
changes take effect without rebuilding a package. Packaging is optional unless
the user asks for a packaged app or the change needs installed-app validation.

## Runtime Changes

For runtime changes that affect the managed gateway's behavior, install the new
binary and run a synthetic end-to-end CLI smoke test against the running gateway
after unit tests pass.

## iOS TestFlight

iOS TestFlight releases are independent from the macOS/gateway release flow.
Do not wire iOS uploads into version-tag release jobs unless the user explicitly
asks for that coupling. Do not trigger an iOS TestFlight build or upload unless
the user explicitly asks for TestFlight packaging/upload in the current turn.
