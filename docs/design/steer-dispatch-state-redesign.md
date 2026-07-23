# Steer Dispatch State Redesign

Status: approved direction (owner decision 2026-07-23); implementation pending.

## Owner Verdict

Steer is a first-class dispatch path. A steer-mode composer send must never
transit through the client-side queue (`queued_local`, `queueByThread`, queue
UI) on its way to the gateway. "Enqueue locally, then immediately dispatch
from the queue" is rejected as a state design.

The client queue exists for exactly one product concept: follow-ups the user
explicitly chose to hold until the current run finishes (queue mode). It is
not a staging buffer, not a retry parking lot for steer, and not a transit
state for immediate dispatch.

## Defects In The Current Design

Both defects live on the same dispatch chain and are fixed together.

### D1 — Steer borrows the queue as a transit state

Current flow (`useMessageDispatchController.handleQueueCurrentPrompt` +
`dispatch-orchestrator.steerQueuedIntent`):

1. Every follow-up while a run is active is built as `queued_local` and
   enqueued (`intent/created, enqueue: true`) — even in steer mode.
2. Steer mode then immediately dispatches that intent *from the queue*
   (`source: queue_steer`), removing it only on gateway ack.
3. Any steer failure does `intent/requeue-front`: the message silently
   becomes a queued follow-up, changing its delivery timing semantics
   ("inject now" degrades to "send after the run") and flashing queue UI.

Consequences: queue chrome flashes for every steer send; steer failures are
invisible in the message stream and mutate into queue entries; the queue
carries two unrelated meanings.

### D2 — Attempt-scoped timestamp poisons the admission fingerprint

The gateway deduplicates dispatches per `client_intent_id` by requiring an
exact request-fingerprint match on replay (`conversation_admission.rs`,
durable `dispatch_admission` table). The desktop main process
(`garyx-client/stream.ts::openChatStream`) generates
`metadata.client_timestamp_local` **at every HTTP attempt** (second
resolution). The fingerprint strips only `client_intent_id` and the
server-owned agent keys, so any re-dispatch of the same intent that crosses a
second boundary produces a different fingerprint and is rejected with
`clientIntentId was reused with a different request` — permanently, for that
intent. The idempotent-replay design can effectively never engage.

## Target Design

### Intent lifecycle

No new `IntentState` values. The existing vocabulary already models the
correct machine; the fix is which transitions each entry path uses.

- **Composer steer send** (steer mode, run active):
  `dispatch_requested` (mode `async_steer`, source `composer_steer`) at
  creation — `enqueue: false`, never in `queueByThread` — then `dispatching`
  → gateway `/api/chat/stream-input` ack → `awaiting_provider_ack` →
  provider `user_ack` materializes the row → `completed`.
- **Composer queue send** (queue mode, run active): unchanged.
  `queued_local` + enqueue; drained as `sync_send` (`queue_send`) when the
  run ends.
- **Manual steer of a queued entry** (existing queue-row affordance):
  unchanged in shape. The intent is a queue member (`queued_local`,
  source `queue_steer` on dispatch); it leaves the queue only on gateway
  ack. On failure it never left the queue: restore `queued_local` in place
  (no `requeue-front` reshuffling, no field resets beyond clearing the
  in-flight dispatch fields).
- **Steer failure (composer steer send)**: terminal `failed` on the intent,
  surfaced *inside the message scroll stream* as the optimistic user row in
  `TranscriptEntryState.error` with the existing retry affordance — exactly
  the `sync_send` failure semantics. Never requeue, never convert to a
  queued follow-up. (Transcript red line 2026-07-21: conversation elements
  live in the scroll stream.)
- **Run-ended race**: a steer dispatch whose `stream-input` answer reports
  no active run continues *the same dispatch* as `sync_send`
  (`/api/chat/start`) with the same intent id — an in-dispatch mode
  resolution, not a queue operation. If that send also fails → `failed`, as
  above.

### Vocabulary change

Add `composer_steer` to `IntentSource` (spec `states.json`, desktop
`INTENT_SOURCES`, iOS enum). `queue_steer` remains, now meaning only the
manual queue-row steer. `async_steer` dispatch-mode semantics in
`conversation-state.md` are reworded: "queued by the **gateway** as a pending
input" stays (protocol truth); the client-side queue is explicitly not part
of the steer path.

### Fingerprint hygiene (D2 fix)

`client_timestamp_local` is the moment the user committed the message — an
intent-scoped value. Stamp it **once at intent creation** in the renderer,
store it on the `MessageIntent`, thread it through the IPC `SendMessageInput`
contract, and have the main process send the stored value verbatim on every
attempt (chat/start and stream-input alike, and the retry path). Request
bodies for the same intent become byte-identical, so the gateway's strict
fingerprint match now yields idempotent join instead of
`FingerprintConflict`. The gateway fingerprint stays strict — no server-side
exemptions.

Check iOS: if the mobile send path also generates a per-attempt local
timestamp, apply the same stamp-once rule there.

### What does not change

- Gateway admission, fingerprint composition, and `dispatch_admission`
  schema.
- Queue-mode product behavior, queue reordering, cancel, drain order.
- `awaiting_provider_ack` tracking, `findPendingAckIntentIndex`,
  provider-ack materialization.
- `ThreadRuntimeState`, activity derivation (`canSteerQueuedPrompt` inputs).
- Server `render_state` ownership of transcript rendering.

## Affected Surfaces

| Surface | Change |
| --- | --- |
| `docs/agents/conversation-state.md` | steer path semantics, `composer_steer` source, failure semantics |
| `spec/conversation-state/states.json` + `scenarios/*` | fixtures first: composer-steer scenarios (direct dispatch, failure→failed, run-ended fallback), queue-row steer failure stays-in-queue scenario |
| Desktop `message-machine.ts` | `composer_steer` source; steer-failure transition (no requeue path for composer steer) |
| Desktop `dispatch-orchestrator.ts` | split `steerQueuedIntent` into composer-steer dispatch (no queue) and queue-row steer; failure paths per above |
| Desktop `useMessageDispatchController.ts` | steer mode branches to direct steer dispatch; queue mode unchanged; timestamp stamped at `buildIntent` |
| Desktop `ComposerQueue.tsx` | no steer-send flash-through (behavioral consequence, minimal code) |
| Desktop `shared/contracts` + preload + `main/garyx-client/stream.ts` | `SendMessageInput.clientTimestampLocal`; remove per-attempt `formatLocalChatTimestamp()` call at request build |
| iOS `GaryxConversationStateMachine.swift` + conformance tests | same transitions via shared fixtures; timestamp rule if applicable |

## Validation

- Fixtures first (`spec/conversation-state/scenarios/`), then both
  conformance suites green: `npm run test:unit` (desktop),
  `swift test` (GaryxMobileCore).
- Deterministic reproduction for D2: unit/integration test that dispatches
  the same intent twice across a wall-clock second — before: gateway
  `FingerprintConflict`; after: byte-identical body, idempotent join.
  (Client-side test asserts the built request body is attempt-invariant for
  a fixed intent; no UI.)
- Steer-failure test: composer steer dispatch with failing transport →
  intent `failed`, queue for the thread stays empty, error row present in
  mapped output (headless, no UI).
- Queue-row steer failure test: intent remains in queue at original
  position, state restored to `queued_local`.

## Out Of Scope (debt, separate items)

- Any broader durable-outbox unification between desktop queue and the iOS
  `DurableDeliveryState` pipeline.
- Gateway admission-row lifecycle/GC.
