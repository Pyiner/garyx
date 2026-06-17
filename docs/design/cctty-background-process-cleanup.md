# cctty Background Process Cleanup

## Problem

Garyx runs Claude Code through the embedded `cctty` PTY wrapper by default. A
normal provider run can complete successfully while a user-started background
service is still intended to keep running, for example a `nohup`-style local
server started from a Bash tool. Today that service is killed during successful
run cleanup.

Focused reproduction:

```text
cargo test -p garyx-bridge --test cctty_cleanup \
  cctty_normal_completion_preserves_user_background_process -- --nocapture
```

Current failing output:

```text
user-started background process <pid> should survive normal cctty completion
test cctty_normal_completion_preserves_user_background_process ... FAILED
```

The test lives in `garyx-bridge` as a pinned behavior guard for the embedded
cctty dependency used by the bridge. It drives cctty directly with a synthetic
Claude CLI. The fake CLI starts a SIGHUP-immune background child, writes a
normal Claude transcript result, and exits the run. The child is gone after the
run completes.

## Root Cause

The bridge successful path calls `run.finish()` after receiving a Claude result:
`garyx-bridge/src/claude_provider.rs:1361`.

The SDK finish path closes stdin and waits for the CLI process:
`claude-agent-sdk/src/client.rs:350-360`.

The regression test exercises the cctty cleanup behavior directly; the SDK
stdin-close link above is established by source inspection.

The process that exits is Garyx's embedded cctty runner. In the pinned cctty
revision, normal `run_print` success calls `process.terminate(...)`:
`cctty/src/runner.rs:199`.

`PtyProcess::terminate` calls `kill()`, and `kill()` sends SIGTERM to
`-pid`, the PTY child process group:
`cctty/src/pty.rs:50-57`. If the child does not exit, terminate escalates
SIGKILL to the same process group:
`cctty/src/pty.rs:75-80`.

That means normal success is treated like an abort. Any background process that
is still in the PTY child process group is killed even if it was intentionally
started as a persistent service.

## Cleanup Policy

Normal successful completion:

- Stop and reap only the foreground PTY child process.
- Do not signal the child process group.
- Preserve explicitly backgrounded or detached user processes.

Abnormal cleanup:

- Keep existing process-group termination for startup retries, stale session
  recovery, MCP runtime restarts, errors, interrupts, and dropped unreaped
  `PtyProcess` values.
- These paths are still cleanup for failed or abandoned provider control flow,
  so child-tree teardown is the right leak-prevention behavior.

Leak control:

- The foreground Claude/cctty PTY child must still be terminated and waited on
  during normal completion.
- The fix is not "remove cleanup"; it is "use PID-only foreground cleanup on
  success, process-group cleanup only on abnormal teardown."

## Planned Implementation

1. Update cctty with a small lifecycle split:
   - Add a `PtyProcess::finish(timeout)` method that sends SIGTERM/SIGKILL to
     `pid` only, waits for that child, and sets `pid = 0`.
   - Keep `PtyProcess::terminate(timeout)` unchanged for process-group cleanup.
   - Change the successful `run_print` path from `terminate` to `finish`.
2. Advance Garyx's pinned `cctty` revision to the fixed commit.
3. Keep the `garyx-bridge` regression test as the guard.

## Validation

- Red: the focused bridge test fails on current `origin/main`.
- Green: the same test passes after the cctty revision update.
- Full target: `cargo test -p garyx-bridge --all-targets`.
