# Task Review Handoff Final Answer Design

## Problem

Task-ready notifications currently use the stopped run's accumulated assistant
response as the handoff body. Providers that emit progress narration as
assistant text can therefore send every intermediate status update to the
review target when the task enters `in_review`, burying the actual final
summary.

The notification path is:

- bridge stopped-run code marks the task `in_review` with a `handoff`;
- bridge emits `task_ready_for_review` with that `handoff`;
- gateway `deliver_task_review_handoff` formats and sends the notification.

The bug is in the body chosen before the event is emitted, not in channel
delivery or notification routing.

## Chosen Layer

Choose the bridge stopped-run layer for agent runs.

The gateway can derive render snapshots from committed transcript records. The
bridge is the place that creates the wrong value by passing the accumulated
provider response as the transient `handoff` carried by `EnterReview`; the task
record itself does not store that handoff.

The bridge layer has the stopped run id, persists the terminal committed
records before moving the task to review, and is the place where the wrong
handoff value is created. It can replace the handoff with the final-answer
segment before both the task record and the event are written.

## Final Answer Extraction

Add a reusable helper in `garyx-models::transcript_render_state`:

- input: committed transcript record values;
- reducer: call `reduce_transcript_render_state`;
- selection: walk render rows from newest to oldest and choose the first visible
  final assistant reference:
  - descend through `RenderRow::UserTurn.activity`;
  - `RenderStepRow.final_message` when present;
  - otherwise `RenderAssistantReplyRow.message` for single-reply turns;
- text: resolve the chosen `seq` back to the original assistant message and
  extract visible text from `text` first, then `content`.

This reuses the existing final-answer placement used by clients. It also keeps
the behavior deterministic: assistant boundaries matter because the streaming
snapshot turns each post-boundary assistant segment into a separate committed
assistant message.

The record set fed to the helper must include the stopped run's terminal control
record. The render reducer defers final-answer placement while the run state is
busy, so omitting `run_complete` would make `RenderStepRow.final_message` stay
empty and fall back to the accumulated provider response.

If a provider does not emit assistant boundaries, the helper will see one
assistant message and keep that full message. That is the best available
backend signal without semantic guessing.

## Bridge Flow

After terminal persistence, compute a task handoff for successful agent runs:

1. Read committed transcript records for the stopped run, preferably from the
   run tail rather than scanning unrelated long-thread history.
2. Filter records to the stopped run id, including terminal control records.
3. Feed those run records to the model helper.
4. If a non-empty final answer is found, use it as the handoff.
5. Otherwise fall back to the existing non-empty provider response gate so pure
   tool or malformed runs do not crash.

The task should still only move from `in_progress` to `in_review` when the run
is successful and there is non-empty handoff text, preserving the current gate.

## Gateway Contract

Keep the event contract unchanged:

```json
{
  "type": "task_ready_for_review",
  "thread_id": "thread::synthetic",
  "run_id": "run-synthetic",
  "task_id": "#TASK-1",
  "handoff": "optional text"
}
```

## Length Limit

Apply a notification-body cap to external bot targets after final-answer
extraction. It should be large enough for a normal final summary but prevent
progress dumps or pasted logs from dominating a review channel. Thread targets
keep the complete handoff because the notification is also input to the review
agent; presentation concerns on first-party clients belong in collapsible UI,
not destructive gateway truncation.

Logging can keep its existing short summaries because logs are not the user
notification body.

## Tests

Reproduce first:

- add a focused bridge test that simulates a successful task run whose provider
  emits two assistant segments separated by `assistant_boundary`;
- first assert the current event handoff contains both the progress segment and
  final segment, proving the bug deterministically.

Then change the test expectation for the fix:

- the task-ready event handoff is only the final assistant segment;
- a single assistant segment keeps the previous handoff value;
- a run with no assistant final answer does not panic and does not create a
  misleading notification body.

Add model tests for the helper:

- multi-segment assistant records return only the final segment;
- single assistant reply returns that reply;
- no assistant reply returns `None`.

Add gateway coverage for the bot-target cap and complete thread-target
handoffs.

## Validation

Focused validation:

- `cargo test -p garyx-models transcript_render_state`
- `cargo test -p garyx-bridge task_ready`
- `cargo test -p garyx-gateway task_notifications`

Final validation for touched crates:

- `cargo test -p garyx-models --all-targets`
- `cargo test -p garyx-bridge --all-targets`
- `cargo test -p garyx-gateway --all-targets`
