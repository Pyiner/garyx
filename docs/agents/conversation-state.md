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
  established message-machine fixture files:
  - Desktop: `desktop/garyx-desktop/src/renderer/src/conversation-state-conformance.test.mjs`
  - iOS: `mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxConversationStateConformanceTests.swift`
- `spec/conversation-state/scenarios/durable-delivery.json`: the shared
  durable-send and multi-stage-create fixture definition. iOS and Mac both
  execute every scenario in their conformance suites and implement the same
  transitions rather than introduce a second vocabulary.
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

This is a transport/recovery state only. It must not drive transcript rows,
tail thinking, active tool groups, final-answer visibility, or composer/steer
business gates. Remote rendered activity comes from the server
`render_state`; local business gates use `ThreadRuntimeState`.

### TranscriptEntryState (message local state)

Every locally known transcript message carries one:
`optimistic`, `remote_partial`, `remote_final`, `error`, `interrupted`.

iOS note: this replaces the legacy id-prefix conventions
(`local-user-*`, `stream-assistant-*`, …) as the carrier of message
provenance. Id schemes may remain as identifiers, but logic must branch on
`TranscriptEntryState`, not on id prefixes. iOS keeps `localState` as
immutable birth provenance (`optimistic` local sends, `remote_partial`
streamed/pending content, `remote_final` canonical rows) and carries
failure as a `statusText` overlay; canonical history indexes live in an
explicit `historyIndex` field instead of `history:N` id parsing. Merge
reconciliation reuses the local row id when a remote row materializes it
(`GaryxTranscriptMerge`), mirroring the desktop identity-preservation rule
so reconciles do not churn list row identity.

### ComposerPhase

`empty`, `editing`, `ime_composing`, `locked`. Derived purely:
locked wins, then IME composition, then text presence.

### DurableDeliveryState

`DurableDeliveryState` is the client-side durable outbox lifecycle. It is
orthogonal to `IntentState`: an intent describes the visible chat/run flow,
while this state proves what can safely happen after storage failure, process
death, logout, or a lost transport response.

| State | Meaning |
| --- | --- |
| `notDispatched` | Envelope is durable and transport has not crossed its attempt gate. Safe to retry. |
| `transportAttempted` | The attempt marker committed before transport; the response is not yet known. A relaunch promotes this to `ambiguous`. |
| `ambiguous` | The gateway may have accepted the send. Only evidence or an explicit user exit may settle it. |
| `acknowledged` | Authenticated origin evidence claimed the send. |
| `cancelledByDiscard` | An unattempted send was cancelled by payload/scope discard. |
| `evidence` | An attempted send lost its payload owner, but bounded correlation evidence remains. |
| `terminalEvidence` | An acknowledged send was reduced to its bounded evidence tombstone. |
| `abandoned` | The envelope was restored to composer ownership without another network attempt. |
| `supersededByDuplicate` | The user explicitly created a duplicate-risk copy with a new client intent ID. |

The state carries two independent axes whose raw values are also canonical:

- `DurableDeliveryEvidence`: `none`, `transportAttempted`,
  `serverAcknowledged`.
- `DurableDeliveryUserDisposition`: `none`, `restoredToDraft`,
  `resentAsDuplicate`, `scopeRevoked`, `payloadDiscarded`.

Authenticated delivery evidence accepts only `(scope, correlationID)` and no
message body. Evidence arriving after a user exit advances the evidence axis
without undoing the user's disposition. Scope revocation settles every record
by its own state and preserves only bounded evidence for attempted sends.

`notDispatched` means transport provably did not run; it does not authorize a
silent network retry. On iOS relaunch, a bare message in that state is
terminalized as `abandoned`/`restoredToDraft`, which releases live delivery
quota and atomically returns its immutable envelope to composer ownership. A
blank composer adopts it immediately. A composer with text, attachments, or an
in-flight attachment operation remains unchanged; the recovered envelope stays
as a separate durable deferred payload and is adopted after that newer draft is
committed to the durable delivery pipeline, or on a later activation/relaunch
once the host is blank.
This placement never projects a recovery-choice notice or action. The same
settlement runs when the live attempt-marker commit fails. A record owned by an
unfinished multi-stage create is excluded because the create correlation must
retain its explicit, honest ambiguity exit.

Multi-stage conversation creation uses the companion canonical vocabularies
`durableCreateDeliveryPhase` and `durableCreateUserDisposition` from
`states.json`. `createPending`, `threadCreated`, optional `bindingCompleted`,
and `chatStartAttempted` are separately durable. Losing any response is
honestly `ambiguous`; in particular, a lost create response does not promise
that no server-side conversation exists. Restoring the payload or rebuilding
with an explicit duplicate warning are the two user-terminal exits until the
P0-G gateway uniqueness/query contract exists.

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

## Activity And Render State

The conversation-state activity derivation is intentionally narrow. It drives
only local business gates: composer lock affordance, steer affordance, and the
optimistic pending-ack loading window.

Inputs: transcript messages (role and loop-continuation marker), `runtimeBusy`,
`pendingAckIntentCount`, `remoteAwaitingAckInputCount`, and
`pendingHistoryIntent`.

Outputs:

- `runActive` = `runtimeBusy`.
- `showPendingAckLoading` = pending-ack intents > 0 ∨ remote awaiting-ack
  inputs > 0 ∨ (an intent awaits history ∧ the latest non-loop-continuation
  user message has no assistant/tool progress after it).
- `canSteerQueuedPrompt` = `showPendingAckLoading` ∨ `runActive`.

Mac desktop has one renderer-local carve-out: the selected-thread composer
send/interrupt button is not itself a `ThreadActivityModel.runActive` output.
For an existing thread, desktop may combine this local activity model with the
selected thread's server `render_state.tailActivity` and
`render_state.activeToolGroupId` to decide whether the button shows Interrupt
instead of Send. That keeps interruption controls aligned with the transcript's
server-derived thinking/tool activity without changing the shared
conversation-state contract or iOS twin.

Everything that is rendered inside the transcript comes from the server
`render_state` reducer:

- `render_state.rows` owns user-turn rows, assistant steps, tool groups, final
  message placement, and filtered empty placeholders.
- `render_state.tailActivity` owns thinking / assistant-streaming / tool-active
  tail presentation.
- `render_state.activeToolGroupId` owns active tool highlighting.
- `render_state.based_on_seq` is the committed ledger sequence the snapshot was
  derived from.

The per-thread SSE protocol sends one frame:

```
{ "type": "thread_render_frame", "events": [...], "render_state": { ... } }
```

`events` are the sync channel for cache, run-state, and cursor maintenance.
`render_state` is the render channel. Clients may keep optimistic local user
rows and pending-ack chrome, but must not derive transcript rows, tail thinking,
tool grouping, or final-answer visibility from live stream state, active-run
projection rows, or raw committed messages.

## Platform Mapping

Legacy iOS constructs and their canonical replacements:

| Legacy iOS construct | Canonical replacement |
| --- | --- |
| `isSending` + `activeRunThreadId` | `threadRuntimeByThread[threadId].state` (`dispatching_sync` / `running_remote`) |
| `remoteBusyThreadIds` | `runtimeBusy` for business gates; `render_state.tailActivity` for rendered transcript activity |
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
`{ role, internal?, internalKind? }` with transcript role strings
(`user`, `assistant`, `tool_use`, `tool_result`, `system`).

`scenarios/function-cases.json` — table cases for `findPendingAckIntentIndex`,
`shouldTrackProviderAckAfterStreamInputResponse`, and `nextComposerPhase`.

`scenarios/durable-delivery.json` — action sequences and exact snapshots for
the durable delivery record and multi-stage create record. `platformConsumers`
is rollout metadata, not permission to diverge: both iOS and Mac execute the
canonical scenarios.

When adding behavior: extend the fixtures in the same change, and keep both
conformance suites green (`npm run test:unit` in `desktop/garyx-desktop`,
`swift test` in `mobile/garyx-mobile`).
