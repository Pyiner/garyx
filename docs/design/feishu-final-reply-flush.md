# Feishu Final Reply Flush Diagnosis

## Problem

Feishu streaming replies can finish with a visible COT completion card but no
ordinary reply card carrying the assistant's final answer.

Deterministic reproduction:

```bash
cargo test -p garyx-channels test_e2e_feishu_trailing_tool_after_final_answer_keeps_reply_card -- --nocapture
```

Initial red result before the fix:

```text
assertion `left == right` failed: final assistant answer must be sent as a Feishu reply card even when a trailing tool follows it
  left: 0
 right: 1
```

The mock request trace from the same scenario shows `message_cot` create/update
and `message_cot/complete/...` are sent, while `/reply` is not. That isolates
the failure to Feishu channel final flushing, not Feishu network delivery and
not upstream omission of the assistant text.

## Root Cause

`garyx-channels/src/feishu/ws.rs` uses `FeishuResponseStreamState.stream_text`
for two roles:

- pending assistant text that should be converted into Feishu COT text before a
  tool call;
- the final assistant reply text sent as the ordinary Feishu reply card after
  `StreamEvent::Done`.

On `ToolUse` and `ToolResult`, `send_pending_stream_text_cot_events` sends
`state.stream_text` as COT `TEXT_MESSAGE_*` events, then clears
`state.stream_text` when COT succeeds. On `Done`, the worker only sends a final
reply card if `state.stream_text.trim()` is non-empty.

For event order:

```text
Delta(final answer) -> ToolUse -> ToolResult -> Done
```

the final answer is consumed as COT pre-tool text and cleared before `Done`.
`finish_cot_run` still completes the COT run, so Feishu shows the completion
card, but the reply card sees an empty buffer and is skipped.

This is intermittent because it only happens when a provider emits a tool call
after producing the human-readable answer. It appears more often on gateways or
agents that perform trailing side effects such as task updates, result
submission, message sends, or follow-up scheduling after drafting the answer.

## Proposed Fix

Keep Feishu's existing COT policy, but separate final reply retention from the
COT pending buffer.

Add a second Feishu response-state field, for example:

```rust
last_assistant_text_for_reply: String
```

Behavior:

- On `Delta`, merge into `stream_text` as today, and set
  `last_assistant_text_for_reply` to the current merged assistant segment.
- On `AssistantSegment`, apply the same separator semantics to both buffers so
  segment boundaries still render coherently.
- On `UserAck`, keep existing behavior of sending the boundary segment
  immediately, then clear both buffers.
- On `ToolUse` / `ToolResult`, keep sending pending `stream_text` to COT before
  tool events. If COT succeeds and consumes the text, clear only `stream_text`;
  retain `last_assistant_text_for_reply` as the Done fallback. If a later
  `Delta` arrives, it overwrites the fallback with the post-tool assistant
  segment, preserving the existing "pre-tool text in COT, post-tool text in
  final reply" policy.
- On `Done`, send `stream_text` when non-empty; otherwise send
  `last_assistant_text_for_reply` when non-empty. Then clear both buffers and
  remove the processing reaction as today.

Why this is scoped:

- It does not change provider events, committed replay, router state, or
  cross-channel plugin policies.
- It keeps COT failure fallback intact: if COT send fails,
  `send_pending_stream_text_cot_events` does not clear `stream_text`, so Done
  sends the accumulated text as before.
- It preserves the existing successful COT test where text before a normal tool
  moves into COT and the later post-tool assistant text stays in the final
  reply.

## Horizontal Channel Check

Telegram:

- Uses `PluginStreamSendState`.
- Tool calls render placeholders but do not clear accumulated assistant text.
- `Done` clears any runtime-only placeholder and finalizes accumulated text.
- No same-shape bug found.

Discord:

- Uses `PluginStreamSendState` with buffered-until-tool-or-Done policy.
- Tool calls render placeholders against the accumulated text; finalization
  edits/sends the accumulated text on `Done`.
- No same-shape bug found.

Weixin:

- Streaming/live paths close the current live message on `Done`.
- Non-streaming paths may flush text at tool boundaries, but `Done` still
  flushes the current buffer or deliberately preserves it when send budget is
  low.
- It does not have Feishu's single buffer serving both COT text and final reply
  card roles. No same-shape bug found.

## Validation Plan

1. Keep the new red Feishu E2E test:
   `test_e2e_feishu_trailing_tool_after_final_answer_keeps_reply_card`.
2. After implementation, verify that test turns green.
3. Run existing Feishu COT tests covering:
   `test_e2e_feishu_tool_trace_then_assistant_keeps_single_reply_message` and
   `test_e2e_feishu_tool_trace_emits_cot_openapi_events`.
4. Run `cargo test -p garyx-channels --lib`.
5. Run `cargo build`.
