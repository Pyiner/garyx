# Conversation State Contract

This is the canonical, platform-neutral contract for conversation (chat
message / run / stream) state. The Mac desktop renderer's
`message-machine.ts` is the reference implementation; the iOS
`GaryxConversationStateMachine` in `GaryxMobileCore` must implement the same
semantics. Neither platform may add, rename, or repurpose lifecycle states
without updating this document and the shared fixtures.

## Source Of Truth Layout

- This document: semantics, transition rules, and derivation rules.
- `spec/conversation-state/states.json`: the machine-readable vocabulary.
  Each platform's enums must match these string values exactly; conformance
  tests assert this.
- `spec/conversation-state/scenarios/*.json`: shared behavior fixtures
  (action sequences with expected state snapshots). Both platforms run the
  same fixture files:
  - Desktop: `desktop/garyx-desktop/src/renderer/src/conversation-state-conformance.test.mjs`
  - iOS: `mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxConversationStateConformanceTests.swift`
- There is no cross-language code generation. Implementations are
  hand-written; the fixtures are the drift guard. When behavior changes,
  change the fixtures first, then make both implementations pass.

## Vocabulary

### IntentState (send lifecycle)

One user-initiated send ("intent") moves through:

| State | Meaning |
| --- | --- |
| `queued_local` | Created and held in the client-side queue, not yet dispatched. |
| `dispatch_requested` | Selected for dispatch; API call not started yet. |
| `dispatching` | API call in flight. |
| `remote_accepted` | Gateway accepted the send; run started. |
| `awaiting_provider_ack` | Queued downstream input waiting for the provider `user_ack` (async steer). |
| `awaiting_response` | Reserved intermediate state; the reference flow normally goes straight from `remote_accepted` to `awaiting_history`. Do not build new logic on it. |
| `awaiting_history` | Stream finished; waiting for the canonical transcript to confirm the turn. |
| `completed` | Intent and its response are visible in canonical history. |
| `failed` | Terminal failure (carries `error`). |
| `interrupted` | Run was interrupted (user abort or recovery abort). |
| `cancelled` | Removed from the queue before dispatch. |

### IntentSource

`composer_send`, `composer_queue`, `queue_send`, `queue_steer`, `retry`.

### IntentDispatchMode

- `sync_send` — primary send; the thread is busy until the run finishes.
- `async_steer` — follow-up input sent while a run is active; queued by the
  gateway as a pending input and acknowledged via `user_ack`.

### ThreadRuntimeState

Per-thread client runtime: `idle`, `dispatching_sync`, `running_remote`,
`reconciling_history`, `interrupting`, `failed`.

`isRuntimeBusy(state)` is true for every state except `idle`, `failed`, and
missing.

### LiveStreamStatus

Per-run stream lifecycle: `connecting`, `streaming`, `reconciling`,
`disconnected` (transient, recovery scheduled), `failed` (permanent),
`interrupted`.

A stream counts as active when its status is `connecting`, `streaming`, or
`reconciling`.

### TranscriptEntryState (message local state)

Every locally known transcript message carries one:
`optimistic`, `remote_partial`, `remote_final`, `error`, `interrupted`.

iOS note: this replaces the legacy id-prefix conventions
(`local-user-*`, `stream-assistant-*`, …) as the carrier of message
provenance. Id schemes may remain as identifiers, but logic must branch on
`TranscriptEntryState`, not on id prefixes.

### ComposerPhase

`empty`, `editing`, `ime_composing`, `locked`. Derived purely:
locked wins, then IME composition, then text presence.

## Machine Semantics

The machine state is:

```
{
  composerPhase: ComposerPhase
  intentsById: { [intentId]: Intent }
  queueByThread: { [threadId]: [intentId] }
  threadRuntimeByThread: { [threadId]: { state, activeIntentId?, remoteRunId?, lastError? } }
}
```

Action semantics (reference: `message-machine.ts` reducer). The notable
non-obvious rules, all covered by fixtures:

- `intent/request-dispatch` on an unknown intent is a complete no-op — it
  must not touch the queue either.
- `intent/cancelled` always removes the id from the thread queue, even when
  the intent record is unknown.
- `intent/remote-accepted` preserves the current state when the intent is
  already `awaiting_provider_ack`, `awaiting_history`, or `completed`
  (duplicate/late accepted events must not regress the lifecycle). Otherwise
  it moves to `awaiting_provider_ack` when `awaitProviderAck` is set, else
  `remote_accepted`. `pendingInputId`/`responseText` keep their existing
  values when the action does not carry them.
- `intent/awaiting-history` overwrites `responseText` with the action value,
  including clearing it when absent.
- `intent/requeue-front` resets `dispatchMode`, `remoteRunId`,
  `remoteThreadKey`, `pendingInputId`, and `responseText`, sets `source`
  (default `queue_send`), and prepends the id to the queue unless already
  present.
- `intent/reorder` clamps the target index to the queue bounds and is a
  no-op when the id is absent or the index does not change.
- `thread/runtime` overwrites `activeIntentId`, `remoteRunId`, and
  `lastError` on every application — omitting a field clears it.
- `thread/replace-id` moves intents, queue, and runtime from the draft
  thread id to the real id. Queues merge as `to ++ (from - to)`. An existing
  runtime on the target id wins field-by-field over the draft runtime.
- `thread/delete` drops the thread's intents, queue, and runtime.

Provider-ack helpers (also fixture-covered):

- `findPendingAckIntentIndex(pendingAckIntentIds, ackedPendingInputId, intents)`
  matches by exact `pendingInputId`; falls back to the single unresolved
  intent (no `pendingInputId` yet); falls back to index 0 when all are
  unresolved or the ack carries no id; otherwise `-1`.
- `shouldTrackProviderAckAfterStreamInputResponse(intent)` is false once the
  intent is in `awaiting_history`, `completed`, `failed`, `interrupted`, or
  `cancelled`.

## Derived Activity Model

UI activity indicators must come from one pure derivation
(`deriveThreadActivityModel` on desktop, `GaryxThreadActivityModel.derive`
on iOS), never from ad-hoc flags:

Inputs: transcript messages (role, pending, loop-continuation marker),
active run id from thread runtime info, live stream status, `runtimeBusy`,
`pendingAckIntentCount`, `remoteAwaitingAckInputCount`,
`pendingHistoryIntent`.

Outputs:

- `runActive` = stream active ∨ runtime busy ∨ server reports an active run.
- `showPendingAckLoading` = pending-ack intents > 0 ∨ remote awaiting-ack
  inputs > 0 ∨ (an intent awaits history ∧ the latest non-loop-continuation
  user message has no assistant/tool progress after it).
- `showRunLoading` = `runActive` ∧ ¬`showPendingAckLoading` ∧ no assistant
  message is pending (streaming text counts as visible progress).
- `canSteerQueuedPrompt` = `showPendingAckLoading` ∨ stream active ∨
  `runActive`.

Render-layer refinements (e.g. iOS `showsTailThinkingIndicator`, which also
hides the indicator while a tool group is live) sit on top of
`showRunLoading`; they must not re-derive `runActive` from transport state.

## Platform Mapping

Legacy iOS constructs and their canonical replacements:

| Legacy iOS construct | Canonical replacement |
| --- | --- |
| `isSending` + `activeRunThreadId` | `threadRuntimeByThread[threadId].state` (`dispatching_sync` / `running_remote`) |
| `remoteBusyThreadIds` | `runActive` from the derived activity model |
| `pendingChatStartThreadIds` | intent in `dispatch_requested` / `dispatching` |
| `statusText` on a message | intent `failed` + message `TranscriptEntryState.error` |
| id prefixes (`local-user-*`, …) | `TranscriptEntryState` |
| `terminatedActiveRunIdsByThread` | runtime `reconciling_history` → `idle` transition gating |
| `pendingQueuedInputsByIntentId` | intents in `awaiting_provider_ack` + `pendingAckIntentIds` |

## Canonical Constants

| Constant | Value | Notes |
| --- | --- | --- |
| History page size | 100 messages | iOS currently 120 — converge to 100. |
| History user-turn limit | 10 user turns per page | Already aligned. |
| Tail prefetch distance | max(640pt, 1.5 × viewport) | Already aligned. |
| Assistant delta flush | coalesce ≤ 50ms before materializing | Desktop flushes per frame/boundary; both must stay within 50ms. |

## Fixture Protocol

`scenarios/machine.json` — `{ "scenarios": [ { "name", "steps": [ {"action": …} | {"expect": …} ] } ] }`.
Actions use the reducer action shapes with string enum values from
`states.json`. Expectations may assert, per step:

- `intents`: map of intentId → partial `{ state, threadId, source, dispatchMode, remoteRunId, pendingInputId, responseText, error }`. A JSON `null` field value asserts the field is absent/cleared; a `null` in place of the whole object asserts the intent record does not exist.
- `queues`: map of threadId → exact ordered intent-id array.
- `runtimes`: map of threadId → `{ state, busy, activeIntentId, remoteRunId, exists }` (all optional; `exists: false` asserts no runtime record).
- `composerPhase`: string.

`scenarios/activity.json` — `{ "cases": [ { "name", "input", "expect" } ] }`
for the derived activity model. `input.messages` entries are
`{ role, pending?, internal?, internalKind? }` with transcript role strings
(`user`, `assistant`, `tool_use`, `tool_result`, `system`).

`scenarios/function-cases.json` — table cases for `findPendingAckIntentIndex`,
`shouldTrackProviderAckAfterStreamInputResponse`, and `nextComposerPhase`.

When adding behavior: extend the fixtures in the same change, and keep both
conformance suites green (`npm run test:unit` in `desktop/garyx-desktop`,
`swift test` in `mobile/garyx-mobile`).
