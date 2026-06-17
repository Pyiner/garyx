# Task Ready Final Message Design

## Problem

`task_ready_for_review` notification cards can show a stale or truncated
`finalMessage`.

The failing cases are covered by headless tests added before the fix:

- `cargo test -p garyx-bridge last_assistant_segment_keeps_tool_split_final_answer_together`
  currently returns only `The code review is queued...`, dropping the preceding
  `CONFIRMED...` summary from the same normalized provider tool-split final
  answer.
- `cargo test -p garyx-gateway final_text_keeps_tool_split_final_answer_together`
  currently returns only the final short assistant segment after a
  Claude/Anthropic-style `assistant` tool-use block followed by a `user`
  tool-result block.
- `cargo test -p garyx-gateway dispatch_defers_snapshot_notification_while_task_run_is_active`
  currently sends a notification from an active-run transcript snapshot instead
  of waiting for the stopped-run final message.

All fixture text uses synthetic task/thread/run IDs and synthetic assistant
content.

## Root Causes

1. Timing: a manual or agent-driven transition to `in_review` emits
   `task_ready_for_review` immediately with `final_message = None`. The gateway
   falls back to the transcript snapshot at that moment. If the provider run
   keeps producing assistant text afterward, the card is already wrong.
2. Granularity: the bridge stopped-run path sends only the last assistant
   segment. A final answer split by tool activity, such as `CONFIRMED...` text,
   a review/task tool call, then a short closing assistant segment, loses the
   substantive first segment.

## Proposed Fix

Keep the existing event contract:

```json
{
  "type": "task_ready_for_review",
  "thread_id": "thread::synthetic",
  "task_id": "#TASK-1",
  "run_id": "run-synthetic",
  "final_message": "optional text"
}
```

No frontend change is required. The card still renders `title + finalMessage`
without a follow-up fetch.

### Timing

Move the authoritative notification body to the stopped-run path.

- In `dispatch_task_ready_notification`, when `final_message` is absent and the
  task thread still has `history.active_run_snapshot`, skip delivery instead of
  formatting from the live snapshot. This handles manual `in_review` transitions
  that happen inside an active agent run.
- In `mark_task_ready_for_review_after_stopped_run`, keep the existing
  `InProgress -> InReview` transition path for successful runs with a non-empty
  final response.
- Add a second path for tasks that are already `InReview` when the run stops:
  if the latest task event is `InProgress -> InReview` and that event happened
  after the current run started, emit the same notification event with the
  stopped-run `final_message` when available. This path runs after both success
  and non-success provider results. A manual/agent mid-run transition must not
  be dropped just because the remaining provider run failed or was interrupted.
- Thread `run_started_at` into `mark_task_ready_for_review_after_stopped_run` as
  an argument. Compare the latest task event timestamp to `run_started_at` as
  parsed RFC3339 timestamps. Both values come from `chrono::Utc::now()`, so they
  are directly comparable after parsing.
- This avoids a new persisted notification state. Duplicate prevention is based
  on the latest task event timestamp falling within the stopped run. A task that
  was already `InReview` before a later run started will not be re-notified by
  that later run.

If a successful run produces no final response, preserve the current gate and do
not transition an in-progress task to review. If a task was already moved to
`InReview` during the run, still emit the deferred notification even when the
provider result is non-successful; the body can be the extracted stopped-run
text or `None`, allowing the gateway fallback to use the completed transcript.

### Granularity

Replace `last_assistant_segment` with a helper whose contract is "best
notification text from the final assistant turn":

- When a human-user boundary exists in the input, scan after the last human user
  message.
- When no human-user boundary exists in the input, preserve the existing
  provider-session semantics: take the trailing assistant text island after
  skipping trailing tool traces, so early run narration before prior tools is
  not pulled into notifications.
- Collect non-empty assistant text in the selected window.
- Treat tool trace rows inside the selected window as transparent, not as text
  boundaries.
- Reset on a later human user message when human-user rows exist.
- Join collected assistant text with blank lines.

The human-user discriminator is explicit:

- Bridge `ProviderMessage` path: `ProviderMessageRole::ToolUse` and
  `ProviderMessageRole::ToolResult` are tool trace rows. `ProviderMessageRole::User`
  resets the turn only when the row is not structurally tool-related.
- Gateway `Value` path: classify a message as tool-related with
  `garyx_models::is_tool_related_message(role, object)`. A `role: "user"` row
  whose content is only a structured `tool_result` block is not a human user
  boundary and must not clear the accumulated assistant text.

This split is necessary because the bridge helper is called on provider
`session_messages`, which often do not include the triggering user message. The
persisted transcript later synthesizes the user row in `build_run_messages`, but
`last_assistant_segment` sees the provider slice before that synthesis. Existing
no-user bridge tests should retain their "only the closing summary, not the
whole run narration" expectation.

Apply the same extraction rule to the gateway fallback
`final_text_after_last_user` so stopped-run events and fallback snapshots share
the same text shape.

This is deterministic rather than semantic: without explicit turn IDs the
backend cannot infer whether pre-tool assistant text was progress narration or
the substantive final answer. The new contract intentionally favors preserving
the full visible final assistant turn over dropping important text.

The stopped-run event is authoritative when the helper extracts non-empty text.
If a run ends on a tool call and extraction returns `None`, the event continues
to carry `final_message: null`; the gateway fallback then uses the completed
transcript with the same extraction rule.

## Impact

- Gateway notifications for a task thread with an active run snapshot are
  deferred until the run stops.
- Stopped-run notifications can include more than the final tiny segment when
  a tool call split the final answer.
- Existing notification target routing and frontend rendering stay unchanged.
- The event contract remains backward-compatible because `final_message` stays
  optional.

## Validation

Focused red tests before implementation:

- `cargo test -p garyx-bridge last_assistant_segment_keeps_tool_split_final_answer_together`
- `cargo test -p garyx-gateway final_text_keeps_tool_split_final_answer_together`
- `cargo test -p garyx-gateway dispatch_defers_snapshot_notification_while_task_run_is_active`

Additional implementation tests:

- A gateway fallback fixture with `role: "assistant"` structured `tool_use`
  content followed by `role: "user"` structured `tool_result` content must keep
  the preceding assistant summary.
- A stopped-run timing test must cover a task that moved to `InReview` during
  the run and then completed with `success = false`; the notification must be
  delivered after the run stops rather than dropped.

Post-fix validation:

- `cargo test -p garyx-bridge last_assistant_segment`
- `cargo test -p garyx-gateway task_notifications`
- `cargo test -p garyx-bridge --all-targets`
- `cargo test -p garyx-gateway --all-targets`
