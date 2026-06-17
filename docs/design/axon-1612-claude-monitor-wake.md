# AXON-1612 Claude Monitor Wake

## Bug

Claude Code monitor/background-task events can fail to wake the active Garyx run.
The reproducible sequence is:

1. A Claude Code run starts a monitor task.
2. Claude emits a `ResultMessage` while the monitor remains active.
3. Later, the monitor condition is satisfied and Claude emits a terminal
   `task_updated` system message, such as `status: completed`.
4. The monitor wake notification and assistant response arrive after the
   bridge's short post-result drain window.

On `origin/main`, `garyx-bridge` closes the run before step 4 can be processed.
The focused reproducer is:

```bash
cargo test -p garyx-bridge test_process_messages_streaming_keeps_monitor_wake_open_after_terminal_task_update -- --nocapture
```

It fails because `run_pending_inputs` is removed after the terminal
`task_updated` message, before the synthetic monitor wake can enter the stream.

## Root Cause

The break is in `garyx-bridge/src/claude_provider.rs`.

- `update_claude_background_tasks` removes a background task when a
  `task_updated` or `task_notification` carries a terminal status.
- `process_messages_streaming` switches to the 2 second
  `POST_RESULT_DRAIN_TIMEOUT_SECS` whenever `result_seen` is true and no active
  background tasks remain.
- The system-message branch only clears `result_seen` for
  `task_notification`.

That means a terminal `task_updated` removes the last active background task but
does not clear `result_seen`. If the follow-up `task_notification` or assistant
response is not available inside the 2 second drain window, the bridge calls
`try_close_pending_inputs`, removes the run input queue, exits the message loop,
and `execute_sdk_run` finishes the Claude process. The watcher has triggered,
but the bridge closes the wake path before Claude can react.

## Design

Treat terminal Claude background-task lifecycle updates as wake-bearing system
events, just like `task_notification`.

Concrete change:

1. Add a small helper near the background-task helpers that returns true for:
   - `task_notification`
   - `task_updated` with a terminal status accepted by
     `is_terminal_claude_background_task_status`
2. In the `Message::System` branch, compute that helper from the raw
   `SystemMessage`, update active task tracking as today, and clear
   `result_seen` when the helper returns true.
3. Leave `ResultMessage` as the only successful completion marker.
4. Keep all existing active-task tracking and pending-input cleanup semantics.

This preserves the current contract: active monitors still keep the stream open,
terminal monitor events still remove active task state, and normal runs still
close through the existing post-result drain when there is no wake-bearing
background event.

The helper should not require a task id. Today `task_notification` clears
`result_seen` based on subtype alone, even if active-task bookkeeping cannot find
`task_id` / `tool_use_id`; terminal `task_updated` should follow the same
subtype-plus-status rule so malformed-but-actionable SDK lifecycle messages do
not silently miss the wake path.

## Leak Boundary

The fix does not introduce an unbounded process leak. A terminal monitor event
moves the stream back to the normal live-read state so Claude can consume the
wake and emit an assistant response / final result. If Claude does not emit
anything and the stream does not close, the existing `STREAM_IDLE_TIMEOUT_SECS`
dead-run guard still breaks the loop.

The intentional tradeoff is that all terminal statuses use this path, not only
`completed`. A killed/cancelled/failed monitor may not produce a follow-up wake,
so such a run can remain alive until the normal idle guard instead of closing
after the 2 second post-result drain. That is bounded, and it keeps failure
notifications compatible with `task_notification`, where Claude may still need
to wake the agent to report the failed monitor.

## Validation

Before fix:

- `cargo test -p garyx-bridge test_process_messages_streaming_keeps_monitor_wake_open_after_terminal_task_update -- --nocapture`
  fails at the assertion that the monitor wake path should remain open.

After fix:

- The same reproducer should pass and assert that the assistant wake response is
  included in the final stream result.
- Full package gate: `cargo test -p garyx-bridge --all-targets`.
