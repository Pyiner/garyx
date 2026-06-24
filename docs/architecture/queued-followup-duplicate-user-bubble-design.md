# Queued Follow-Up Duplicate User Bubble Design

## Reproduction

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

## Proposed Fix

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

Do not add `origin_id` handling in this fix. Gateway history currently does not
project `origin_id` on `pending_user_inputs`; `queued_input_id` is already the
stronger queued-input identity for this reconciliation.

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
