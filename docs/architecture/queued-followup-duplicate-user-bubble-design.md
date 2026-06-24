# Queued Follow-Up Duplicate User Bubble Design

## TASK-1282 Update

The first fix for this area hid stale remote pending-input bubbles when the
committed user record carried `metadata.queued_input_id`. The current report is
a second race in the same desktop-only overlay layer:

1. An active run is already busy.
2. The user queues a follow-up. Desktop creates a local intent
   (`intent:test-follow-up`) and receives a gateway pending input
   (`queued_input:test-follow-up`), so local pending-ack chrome suppresses the
   remote pending-input chrome.
3. A live `thread_render_frame` arrives whose `render_state` includes the
   committed user row. The committed user row is keyed by the same origin id
   (`metadata.origin_id = intent:test-follow-up` / `origin:intent:test-follow-up`).
4. The local pending-ack bubble is correctly hidden because the committed user
   row represents the intent origin.
5. The stale remote pending-input entry can still be present in desktop state.
   If the committed user record does not expose `queued_input_id` in the loaded
   message shape, the remote helper cannot connect
   `queued_input:test-follow-up` back to `intent:test-follow-up`, so it renders
   a second user bubble.

This keeps the data-layer observation intact: the transcript has one committed
user record. The duplicate is introduced after server `render_state.rows`, in
desktop pending chrome.

### Deterministic Red Repro

The red test models the timing sequence instead of a single static record:

```bash
cd desktop/garyx-desktop
node --experimental-strip-types --test src/renderer/src/app-shell/pending-inputs.test.mjs
```

Expected current failure before the fix:

```text
AssertionError [ERR_ASSERTION]: committed origin row should suppress the stale remote pending bubble
actual: 2
expected: 1
```

The test passes an optional pending-input-to-origin reconciliation tuple:
`queued_input:test-follow-up -> intent:test-follow-up`. Current code ignores
that tuple, so the sequence deterministically renders one committed user row
plus one stale remote-pending row.

### Proposed TASK-1282 Fix

Keep server-render-state first. Do not change `render_state`, turn grouping,
message merge, or final-answer placement.

Extend the desktop pending-input visibility helper so a remote pending input is
considered represented when either:

- an active user message has `metadata.queued_input_id` equal to the pending
  input id; or
- the pending input id maps to an in-flight local intent origin and an active
  user message has that same origin id (`origin:<intentId>` or
  `metadata.origin_id`).

The mapping comes from desktop message-machine state, not text matching:

```ts
{
  pendingInputId: "queued_input:test-follow-up",
  originId: "intent:test-follow-up",
}
```

Build this reconciliation list per thread from known `MessageIntent`s that have
both `pendingInputId` and `intentId`, then pass it to both main-thread and
side-chat `visibleRemotePendingInputsForThread` calls. Include intents beyond
the currently visible pending-ack set, because the ack can be removed before the
stale remote pending input cache has been cleared.

### #1235 Regression Guard

This fix must not change `buildThreadViewRowsWithLocalUsers`. The #1235 sync
send path is still guarded by `render-view-model.test.mjs`: local optimistic
origin rows render before materialization and dedupe once `render_state`
represents the same origin.

### Mobile Check

The mobile path does not have the same desktop `pendingRemoteInputsByThread`
overlay helper. `GaryxMobileRenderStateMapper.rows(...)` appends local
non-final user rows only while the server snapshot refs do not already contain
the same `origin:*` id, and existing Swift tests cover both optimistic
visibility and committed-origin suppression. No mobile code change is planned
unless review identifies another remote-pending overlay path.

## Prior TASK-1281 Reproduction

The remainder of this document records the first queued-follow-up fix, which
handled stale pending-input bubbles when committed user records carried
`metadata.queued_input_id`. It is superseded for TASK-1282 by the origin-aware
update above.

Local captured data from the reported run was inspected before any fix:

- The committed transcript ledger contains one user record for the reported
  queued follow-up, not two.
- That record carries `metadata.origin_id` and `metadata.queued_input_id`.
- The final persisted thread state has no remaining `pending_user_inputs`.

The failing live window is therefore a desktop render/chrome reconciliation
issue: a stale remote pending input can remain in desktop state after the live
`thread_render_frame` has made the committed user row visible.

Deterministic red test:

```bash
cd desktop/garyx-desktop
node --experimental-strip-types --test src/renderer/src/app-shell/pending-inputs.test.mjs
```

Current failure:

```text
AssertionError [ERR_ASSERTION]: committed user row should suppress the stale remote pending bubble
actual: [queued_input:test-follow-up]
expected: []
```

The test fixture is a scrubbed version of the captured state shape: one
`pending_user_inputs` entry and one committed user message whose metadata
contains the same `queued_input_id`.

## Root Cause

`ThreadPage` renders in this order:

1. `turnRows` from server `render_state.rows`.
2. desktop-only pending ack intent bubbles.
3. remote pending input bubbles from `pendingRemoteInputsByThread`.

`pendingRemoteInputsByThread` is refreshed by transcript fetches via
`pending_user_inputs`. Live `thread_render_frame` events apply committed
messages and replace the render snapshot, but they do not necessarily clear the
separate remote pending-input cache in the same frame. During that window, the
same queued user message is represented twice:

- once as the committed user row selected by server `render_state`;
- once as a stale remote-pending bubble keyed by the queued input id.

This does not violate the server reducer contract. The duplicate is outside
`render_state.rows`, in desktop-only pending-ack chrome.

## Superseded TASK-1281 Proposed Fix

Keep transcript structure server-owned. Do not add client grouping or row
reduction.

Add a pure desktop helper for pending-input visibility:

- Keep the existing rule that local active pending-ack intents hide remote
  pending inputs.
- Otherwise show only `awaiting_ack` remote inputs that are not represented by
  any loaded user message.
- A pending input is represented when a loaded user message has
  `metadata.queued_input_id` matching the pending input id. This key is the
  stable bridge between gateway `pending_user_inputs[].id` and the committed
  user record metadata after provider ack.

TASK-1282 supersedes the following constraint: origin-aware reconciliation is
now required for live frames whose committed user row is represented only by
`origin_id` and a desktop-local `pendingInputId -> originId` mapping.

Wire both desktop call sites through the helper:

- the main thread surface's `visibleRemotePendingInputs`;
- the side-chat surface's equivalent pending-input derivation.

The helper must receive the already-filtered visible local pending-ack count
(`visiblePendingAckIntents.length` on the main surface, and the side-chat
equivalent), not the raw active pending-ack list.

## Validation Plan

- Red/green focused desktop unit test:
  `node --experimental-strip-types --test src/renderer/src/app-shell/pending-inputs.test.mjs`
- Desktop unit suite:
  add `src/renderer/src/app-shell/pending-inputs.test.mjs` to the explicit
  `npm run test:unit` file list, then run `npm run test:unit`
- Renderer build:
  `npm run build:ui`
- Packaged-app check because renderer resources change:
  `npm run dist:dir`

## Impact

The change only affects desktop-only pending input chrome. It does not alter
server `render_state`, message body resolution, user-turn grouping, tool
grouping, final-answer placement, or SSE cursor handling.
