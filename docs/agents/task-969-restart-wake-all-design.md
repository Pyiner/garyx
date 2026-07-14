# Restart Wake-All Design

## Goal

`garyx gateway restart --wake all` captures every user-visible thread that was
running before the restart and queues a continuation for each one. After the new
gateway is healthy, startup wake draining sends the configured wake message
(a structured restart notice by default) to each captured thread.

This keeps the existing single-target wake behavior while covering the common
agent-restart case where several runs are interrupted by the same gateway
restart.

## CLI Semantics

Supported restart choices:

- `garyx gateway restart --wake thread <thread_id> --wake-message "continue"`
- `garyx gateway restart --wake task <task_id> --wake-message "continue"`
- `garyx gateway restart --wake bot <channel:account_id> --wake-message "continue"`
- `garyx gateway restart --wake all [--wake-message "continue"]`
- `garyx gateway restart --no-wake`

`--wake all` is a separate wake mode, not a target kind with an extra target.
It conflicts with `--no-wake` the same way single-target wake does. Unlike
single-target wake, it does not require `--wake-message`; omitted message means
the structured restart-notice message.

The clap definition accepts `num_args = 1..=2`, with manual validation in
`resolve_gateway_restart_wake_destination`:

- one token exactly equal to `all` is wake-all;
- two tokens remain the existing single-target form;
- any other arity or target kind is rejected;
- `wake_message` is required only for the two-token single-target form.

The CLI captures the wake-all thread list before it restarts the service. The
queued file therefore represents "threads that were running before restart",
not whatever the new gateway sees after startup reconciliation.

## Inclusion Rules

The source of truth is the gateway SQLite `recent_threads` projection. Before
restarting, the CLI first requests the bounded snapshot from the running
gateway at `GET /api/restart-wake/snapshot`; this keeps the gateway's existing
database service as the only writable owner. The endpoint performs the active
thread predicate and recency ordering in SQL rather than loading and filtering
every recent-thread row.

If the gateway is unavailable or an older gateway does not expose the endpoint,
the CLI resolves `sessions.data_dir` from the selected config and opens
`<data_dir>/garyx-db.sqlite3` with `SQLITE_OPEN_READONLY` plus `query_only=ON`.
The fallback never creates or initializes a database and therefore cannot
become a second writer. This preserves pre-restart recovery when the gateway is
already unhealthy while respecting custom data directories.

A thread is included when:

- `thread_id` is canonical (`thread::...`);
- `run_state == "running"` or `active_run_id` is non-empty;
- it appears in `recent_threads` at queue time.

The list is sorted by `recent_threads` recency order and de-duplicated by
thread id. Capturing by `recent_threads` intentionally matches the mobile list
that currently shows stale typing state.

Generated automation threads are already excluded from `recent_threads` by
projection rules (`exclude_from_recent` or
`automation_thread_mode=generated_thread`). Wake-all does not add a hidden-thread
scan. Visible top-level agent/task threads remain included.

Completed threads are not included unless their projection still claims
`running` or carries a non-empty `active_run_id`. In that stale case wake-all
uses the user's selected recovery behavior: wake the thread so it can continue
or close cleanly.

## Pending Wake File

The existing pending restart wake queue is extended without replacing the
single-target shape:

- `kind=thread|task|bot`: existing file shape with one `target`.
- `kind=all`: stores a `targets` array of canonical thread ids captured before
  restart.

Drain-time handling resolves single-target wakes as it does today. For
`kind=all`, the new gateway dispatches one internal continuation per captured
thread. Each dispatched run id is unique and stable for the wake file:
`restart-wake-<wake_id>-<index>`.

The drain still renames a `.json` file to `.processing.json` before dispatch.
That remains the re-entry guard: a second gateway process will not drain the
same file concurrently. At startup, stale `.processing.json` files older than a
short crash-recovery threshold are renamed back to `.json` before scanning; this
keeps all-mode from amplifying a mid-drain crash into a permanently lost batch.

The scanner must only treat fresh pending files as drainable. A fresh pending
file is a regular file whose final extension is exactly `.json` and whose file
stem contains no dots. That is stricter than the current
`Path::extension() == Some("json")` check, which also matches
`.processing.json` and `.failed.json`. Stale-processing recovery runs before the
fresh-file scan and is the only path that turns an old `.processing.json` back
into a drainable `.json`; `.failed.json` remains terminal unless a human renames
it.

For wake-all, drain attempts each captured thread at most once per drain pass:

- successful targets are removed from the pending set;
- non-retryable failures are recorded into a `.failed.json` file that contains
  only the failed targets and their errors;
- `Overloaded` is retryable, because the bridge run limiter rejects
  synchronously when full instead of queuing.

The current dispatch stack erases `BridgeError` into `String` before
`restart_wake` sees it. The wake-all implementation therefore introduces an
explicit classification helper at the `dispatch_internal_message_to_thread`
boundary. It wraps the string error as a small internal dispatch error enum and
marks it retryable when the trimmed error starts with the stable
`BridgeError::Overloaded` display prefix, `bridge overloaded:`. It must not
match the full error string, because the concurrency count is dynamic.

When `Overloaded` occurs, the current target and all unattempted targets are
written back as a fresh `.json` pending wake file with the same message and
metadata plus an incremented attempt count. The drain schedules a delayed retry
task and returns success for the current pass. Already dispatched targets are
not written back, so the automatic retry is idempotent for the successful
prefix. A retry cap moves the remaining targets to `.failed.json` rather than
spinning forever; the failed file still contains only targets that did not get a
successful dispatch.

Delayed retry is a new in-process startup helper, not an existing scheduler. The
helper writes the remaining wake-all targets to a fresh pending `.json` file,
then spawns a detached task that sleeps for an exponential backoff and calls
the same drain routine for the restart-wake directory again. The retry task uses
the same fresh file scanner described above, so it will not reprocess
`.processing.json` or `.failed.json` files. If the gateway dies before the retry
fires, the fresh pending file remains on disk and will be retried on the next
startup.

## Storm Control

Wake-all applies control at three layers:

- Queue-time cap: capture at most `MAX_RESTART_WAKE_ALL_THREADS` de-duplicated
  thread ids from the ordered recent-thread projection. The initial cap is 16,
  deliberately below the bridge default of 32 so wake-all leaves startup
  headroom.
- Dispatch behavior: the drain sends wake-all targets sequentially and treats
  bridge `Overloaded` as backpressure, not a terminal failure. Remaining
  targets are requeued for delayed retry instead of being silently dropped.
- Runtime admission: the bridge still enforces global provider concurrency
  through its run limiter (`GARYX_BRIDGE_MAX_CONCURRENT_RUNS`). Wake-all does
  not bypass that limiter, and it does not rely on that limiter to queue work.

The captured list is de-duplicated before the cap is applied, so one stale row
cannot create repeated continuations for the same thread. A restart can still
have multiple pending wake files, but each file is processed once by rename and
each wake-all file sends at most one successful continuation per thread.

## Startup Reconciliation

Wake-all is the primary recovery path for interrupted runs because it lets the
run own its normal completion path and update projections naturally. Startup
projection reconciliation stays in place as a data-migration and stale-row
fallback, but this change does not expand it into a broad "mark interrupted
agent runs complete" policy.

That restraint matters because a killed run might still be recoverable and the
desired behavior for this task is to continue it, not erase it. Reconciliation
should continue to repair rows where the transcript/store state already proves
there is no active run; wake-all handles the intentional restart-recovery path.

## Validation Plan

- CLI parsing tests for `--wake all` with default and explicit messages.
- Wake-all snapshot unit tests:
  - includes canonical rows where `run_state=running`;
  - includes rows with non-empty `active_run_id`;
  - excludes idle/completed rows without active run;
  - de-duplicates thread ids;
  - preserves the restarting agent's own running thread when present.
  - prefers the running gateway HTTP snapshot;
  - falls back to a read-only SQLite connection in configured custom
    `sessions.data_dir` when HTTP is unavailable;
  - proves the read-only connection can query while another connection holds a
    write transaction and rejects mutation attempts with SQLite `ReadOnly`.
- Drain tests with a recording provider:
  - dispatches one continuation per captured thread;
  - uses distinct run ids;
  - does not dispatch duplicate threads.
  - does not scan `.processing.json` or `.failed.json` as fresh pending files;
  - requeues only unattempted targets on retryable overload;
  - classifies retryable overload by the stable `bridge overloaded:` prefix;
  - schedules a delayed re-drain for requeued overload work;
  - recovers stale `.processing.json` files.
- Focused cargo gates:
  - `cargo test -p garyx-gateway --all-targets restart_wake`
  - `cargo test -p garyx --all-targets gateway_restart`
  - broader gateway/CLI tests after implementation.

Managed gateway restart is deliberately not part of agent-side validation for
this task handoff; the code change only affects the installed gateway after the
binary is built, installed, and restarted by the owner.
