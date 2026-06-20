# TASK-938 Committed Terminal Detection Design

## Goal

Fix committed-stream consumers that wait for top-level `run_complete` or
`run_error` bus records even though the producer emits terminal lifecycle facts
as `type="committed_message"` with `message.control.kind` set to
`run_complete` or `run_error`.

## In Scope

- `garyx-channels/src/committed_replay.rs`
  - Replace top-level lifecycle detection in `CommittedReplayState::on_bus_message`
    with committed-message control-kind detection.
  - Keep `control_record_to_stream_events` unchanged: `run_complete` and
    `run_error` remain non-content lifecycle facts, not `StreamEvent::Done`.
  - Update tests so terminal fixtures use the real committed-message envelope.
  - Add coverage for interrupted/error runs that have terminal control but no
    `done`, proving buffered consumers still receive one synthetic `Done`.
  - Return/manage the replay task handle from `committed_callback` so callers can
    abort the replay if dispatch fails before any terminal committed record can
    arrive.

- `garyx-gateway/src/chat.rs`
  - Make `is_terminal_bus_record_for_run` inspect
    `message.control.kind` inside committed messages.
  - In the WS committed stream loop, forward a terminal committed record to the
    client and then run terminal cleanup/break. The terminal record must not be
    swallowed by the break path.
  - Stop the WS stream task when forwarding to `out_tx` fails, so a disconnected
    client does not keep a broadcast receiver alive.
  - Abort the WS stream task if `start_chat_run` fails after subscribing but
    before the provider run starts.

## Out Of Scope

- Do not collapse `done` and `run_complete` into one signal.
- Do not map `run_complete` or `run_error` to `StreamEvent::Done` in
  `control_record_to_stream_events`; that would double-flush on normal runs.
- Do not alter ordinary assistant/tool/done delivery behavior.
- Do not change iOS or desktop UI behavior.

## Test-First Plan

1. Replace `run_lifecycle_line("run_complete")` test fixtures with a real
   committed-message control envelope. Existing terminal tests should fail
   before code changes because `on_bus_message` still reads top-level `type`.
2. Fix `on_bus_message` by detecting terminal control kind inside accepted
   committed messages. Remove the top-level lifecycle branch.
3. Add a spawn-level interrupted-run regression: live receives assistant content
   and a committed `run_complete` control with no `done`; the replay task exits
   and the collected events include exactly one synthetic `Done`.
4. Add a replay-task leak regression for pre-dispatch failure by exercising the
   returned guard/handle path and asserting the task exits.
5. Add gateway WS unit coverage for:
   - terminal committed controls are recognized;
   - terminal committed controls are forwarded once and then cause the loop to
     break;
   - failed `out_tx.send` makes forwarding report failure so the loop can stop.

## Implementation Notes

- Add a small helper, for example `committed_control_kind`, to centralize
  extraction of `message.control.kind` from committed bus values.
- Terminal controls must still pass the existing run/thread acceptance rules.
  This preserves filtering for other runs.
- For channels, introduce a small guard type around `JoinHandle<()>`. It aborts
  on drop unless disarmed. `committed_callback` can return a replay attachment
  that derefs/converts to the existing callback option and lets callers disarm
  only after dispatch/start succeeds.
- For gateway WS, make `spawn_chat_ws_committed_stream` return `JoinHandle<()>`.
  The start task stores the handle from the callback builder and aborts it on
  `start_chat_run` error.
- Change forwarding helpers to return a distinct outcome for sent/skipped, gap,
  and closed-client states. The stream loop treats closed clients as a break,
  while gaps still use durable backfill and retry.

## Validation

- Red step: focused `cargo test -p garyx-channels committed_replay --all-targets`
  after fixture-shape correction should fail on terminal detection.
- Green steps:
  - `cargo test -p garyx-channels --all-targets`
  - `cargo test -p garyx-gateway --all-targets`

## Risk

Risk is low because the change is limited to terminal detection and task
lifetime. Content mapping stays unchanged, including happy-path `done` flush.
The main behavioral change is that interrupted or failed runs now trigger the
existing terminal backfill/synthetic-done path and stream tasks exit promptly.
