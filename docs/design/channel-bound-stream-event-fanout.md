# Bound Channel Stream Event Fanout

## Status

Proposed.

This document records the target design for delivering a thread run's committed
stream events to every channel endpoint bound to that thread. The current
compatibility work item is tracked in `TODO.md`.

## Problem

Garyx lets a thread bind to channel endpoints such as Feishu, Telegram,
Discord, Weixin, or subprocess plugin channels. Once a thread is bound, the
desired product contract is simple:

- A run emits committed `StreamEvent` rows.
- Garyx fans those events out to every endpoint bound to the thread.
- Delivery does not depend on where the run was triggered.

The current implementation is not clean enough for that contract. Some gateway
entry paths attach a bound-response callback. Built-in channel inbound paths can
also subscribe to committed replay directly and dispatch through their own
channel-native logic. That means the fanout behavior is partly determined by
the run's origin.

There is also a compatibility fallback that converts `StreamEvent` values into
older outbound messages. That fallback is useful for legacy subprocess plugins,
but it is the wrong abstraction for the main system. It loses protocol shape:
tool events, boundaries, delivery ordering, and channel-specific rendering rules
do not belong in gateway-level text conversion.

## Goals

- Bound delivery is origin-agnostic: API chat, internal/plugin entry, or a
  channel-originated run all use the same fanout contract once a `thread_id` is
  known.
- Gateway/router only decide which committed events should be delivered to
  which bound endpoints.
- Channel-specific rendering stays inside `garyx-channels`.
- Built-in channels and subprocess plugins are represented by the same
  dispatcher-level `dispatch_stream_event` contract.
- Legacy plugin compatibility is isolated behind an adapter in the channel
  layer, not leaked into gateway behavior.
- The design preserves event ordering and keeps target identity precise enough
  for channel-native thread/topic scopes.

## Non-Goals

- Do not preserve gateway markdown parsing, image stripping, Telegram-specific
  error checks, or other channel presentation logic in gateway.
- Do not define the full subprocess plugin wire format here.
- Do not require old subprocess plugins to become fully stream-event native in
  one release.
- Do not change how an individual channel chooses to render markdown, tool
  calls, images, or acknowledgements.

## Proposed Model

The architectural boundary is:

```text
committed StreamEvent row
  -> bound endpoint lookup by thread_id
  -> garyx-channels dispatcher dispatch_stream_event
  -> channel-native renderer/sender
```

Gateway does not construct channel messages. It only pushes committed events
into the dispatcher for the selected targets.

### Shared Fanout Attachment

Introduce one shared fanout service for run orchestration, for example
`BoundStreamFanout` or `BoundEventFanout`.

The service is attached after the run has a stable `thread_id` and `run_id`.
At that moment it snapshots the thread's bound endpoints and creates one
stream-event callback per target.

The snapshot behavior is intentional. If a user changes bot/channel binding
while a run is already active, the change applies to the next run. This keeps a
single run's delivery set deterministic.

All run origins call the same attachment point:

- API chat.
- Internal/plugin-created runs.
- Built-in channel inbound handlers.
- Subprocess plugin inbound handlers.
- Future workflow or automation entry paths that create thread runs.

### Dispatch Target Identity

The fanout target must carry enough identity for channels to address their
native destination without gateway understanding the channel.

Required fields should include:

- `channel_id` or channel kind.
- `account_id`.
- `thread_id`.
- `target_thread_id` when the channel has a mapped Garyx thread target.
- Channel delivery target type and id.
- Channel-native chat or conversation id.
- Channel-native topic/thread scope when the channel has one.
- `run_id`.

De-duplication must use an exact endpoint identity, not just a chat id. Two
bindings in the same chat but different native topics must not collapse into
one target.

The origin target is not semantically special. The only special handling is
practical duplicate prevention if an existing direct channel callback for the
same exact endpoint is already attached to the run.

### Dispatcher Contract

`garyx-channels` should expose a dispatcher-level stream event API. A sketch:

```rust
fn build_stream_event_callback(
    &self,
    target: StreamDispatchTarget,
) -> Result<StreamEventCallback, StreamEventCallbackError>;
```

The callback receives the original `StreamEvent` shape plus delivery metadata.
It must preserve:

- Event order for a single target.
- Boundaries such as segment flushes and `Done`.
- Structured `ToolUse` and `ToolResult` events.
- Channel-specific acknowledgement behavior.

For subprocess plugins with a new protocol capability, the host forwards events
with a host-to-plugin RPC such as:

```json
{
  "type": "dispatch_stream_event",
  "target": {
    "account_id": "account-synthetic",
    "delivery_target_type": "chat",
    "delivery_target_id": "chat-synthetic",
    "thread_id": "thread-synthetic",
    "run_id": "run-synthetic"
  },
  "seq": 42,
  "event": {}
}
```

`seq` is per target and monotonic. It gives channels and plugins a simple hook
for ordering, idempotency, and diagnostics.

### Built-In Channels

Built-in channels should implement `dispatch_stream_event` natively.

They can reuse their existing channel-native stream consumers, but the
dispatcher should be the single entry point. Gateway should not know that
Telegram, Feishu, Discord, or Weixin have different rendering rules.

This also means structured tool events stay structured until they reach the
channel renderer. If Feishu can render a tool call and Telegram chooses a
different presentation, that difference belongs in each channel implementation.

### Legacy Subprocess Plugin Adapter

Old subprocess plugins only know `dispatch_outbound`. They need a compatibility
adapter, but the adapter is deliberately a downgrade path.

The adapter lives in `garyx-channels`, receives the same `StreamEvent` callback
as every other target, and converts only what the old protocol can represent.

Adapter rules:

- Text deltas and assistant segments may be buffered into outbound text.
- Segment boundaries and `Done` flush buffered text.
- A single worker queue per target preserves event order.
- `ToolUse` and `ToolResult` must not be silently dropped as successful sends.
  The adapter should either convert them into an explicit visible fallback,
  report them as unsupported, or use a declared structured capability.
- `UserAck` behavior must be documented. It can be treated as a boundary for
  old plugins only if that matches the old plugin contract.

This adapter is the compatibility debt listed in `TODO.md`. It should disappear
once subprocess plugins support `dispatch_stream_event`.

## Observability

Gateway should remain channel-agnostic, but delivery diagnostics still matter.

The channel layer should report per-target delivery outcomes in a structured
way. That can be a callback result, a delivery outcome event, or a diagnostics
sink owned by `garyx-channels`. The important rule is that observability moves
with delivery ownership; it does not justify putting channel rendering logic
back in gateway.

## Migration Plan

1. Add tests for exact endpoint identity, especially same chat with different
   native thread/topic scopes.
2. Add the dispatcher-level `dispatch_stream_event` contract in
   `garyx-channels`.
3. Implement native stream-event dispatch for built-in channels through that
   contract.
4. Add subprocess plugin capability detection for native `dispatch_stream_event`.
5. Add the legacy subprocess adapter from `StreamEvent` to `dispatch_outbound`.
6. Move all run origins to the shared bound fanout attachment point.
7. Remove gateway-side message rendering, markdown parsing, image stripping, and
   channel-specific branching from bound delivery paths.
8. Delete the compatibility TODO once all supported plugin paths are
   stream-event native.

## Acceptance Criteria

- A bound thread fans out committed events to every bound endpoint regardless of
  whether the run started from API chat, an internal/plugin entry path, or a
  built-in channel inbound message.
- Gateway bound delivery contains no channel-specific markdown, image, tool, or
  Telegram/Feishu/Discord/Weixin rendering logic.
- Built-in channels receive structured `StreamEvent` values and render
  `ToolUse` / `ToolResult` through their own channel presentation paths.
- Legacy subprocess plugins continue to receive old `dispatch_outbound` calls
  through an explicit adapter, with documented lossy behavior.
- Unsupported structured events are observable and are not reported as silent
  successful sends.
- Same-chat different-topic bindings are not accidentally de-duplicated.
- Per-target event order is preserved.

## Test Plan

- Unit tests for exact target identity and de-duplication keys.
- Gateway or orchestration tests proving every run entry path attaches the same
  bound fanout service.
- Channel tests proving built-in stream callbacks receive structured tool
  events instead of fallback text conversion.
- Legacy plugin adapter tests for ordering, flush behavior, and unsupported
  structured event reporting.
- Regression tests for router outbound id persistence and delivery target
  lookup.

## Open Questions

- Should `dispatch_stream_event` be fire-and-forget with async diagnostics, or
  should every event require a per-target acknowledgement?
- What is the minimum acceptable visible fallback for `ToolUse` / `ToolResult`
  in legacy plugins that cannot render structured tool calls?
- Should the shared fanout service live in gateway runtime assembly, router run
  orchestration, or bridge/gateway glue code? The ownership should follow the
  place where `thread_id`, `run_id`, committed replay, and bound endpoint lookup
  are all available without channel-specific knowledge.
