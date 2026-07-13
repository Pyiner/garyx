# TASK-2199: iOS Old User Row Tail Reconciliation

## Problem

In a long iOS conversation, committed user messages from much earlier in the
thread can reappear after the newest server-rendered rows. They then remain at
the bottom while later committed messages render above them.

The captured thread makes the failure deterministic:

- an older mobile user input committed at seq 934 with a stable
  `metadata.origin_id`;
- two queued mobile follow-ups committed at seq 1058 and seq 1064, also with
  distinct stable `metadata.origin_id` values;
- after the render window advanced to later records, those three old user
  bubbles appeared after the current server rows;
- a newer user input still inside the render window stayed in the correct
  server-owned position.

`GaryxMobileOldUserTailReproTests` reduces that captured shape to synthetic
identifiers. Before the fix, the mapper returns newer server rows followed by
the old user row:

```text
[origin:newer-1, origin:newer-2, origin:old]
```

The authoritative window contains only the two newer rows, so the expected
output is:

```text
[origin:newer-1, origin:newer-2]
```

The old row's canonical seq places it above the render floor; it must not be
reintroduced at the tail by local optimistic state.

## Root Cause

The server contract and reducer output are correct. Captured committed user
records carry the expected `metadata.origin_id`, and render refs use the
matching `origin:*` identity.

The identity is lost on the iOS live-stream path:

1. `GaryxTranscriptMessage` derives a user message id from
   `metadata.origin_id`, falling back to `history:<index>` only when no origin
   exists.
2. `GatewayStreamFrameProcessor.processRenderFrame` receives a committed event,
   assigns its ledger index, and then unconditionally overwrites its id with
   `history:<index>`.
3. `GaryxTranscriptMerge` is intentionally id-only. It therefore sees the
   remote `history:*` message and the optimistic `origin:*` message as two
   different messages, retaining the optimistic copy.
4. While the old server render ref remains in the window,
   `GaryxMobileRenderStateMapper` hides that local copy because its `origin:*`
   id is represented by the snapshot.
5. Once the render floor moves past the old turn, the ref disappears. The
   retained optimistic copy is no longer represented, so the mapper appends it
   after all server rows. It has no `historyIndex`, so window-pruning logic also
   treats it as unsettled local state and keeps it.

The visible bad ordering is therefore a delayed consequence of live committed
identity corruption, not a server row-sort defect.

## Design

Make committed-index assignment preserve the canonical message identity rule.

Add one operation to `GaryxTranscriptMessage` that applies a committed history
index and re-derives `id` through the type's existing identity function:

- user + non-empty `metadata.origin_id` -> `origin:<origin_id>`;
- every other committed message -> `history:<index>`.

Use that operation in `GatewayStreamFrameProcessor` instead of mutating
`index` and then forcing a `history:*` id independently.

This keeps all ingress paths aligned:

- REST transcript decode;
- persisted cache decode;
- live committed stream events.

After the change, the first live flush merges the remote committed user row
with the optimistic row by their shared `origin:*` id. The surviving row is
`remoteFinal` and has a `historyIndex`. When it later falls above the render
floor, the mapper does not append it and the normal window-pruning path can
discard it.

## Scope

Production changes are limited to `GaryxMobileCore`:

- canonical committed-index/identity assignment on `GaryxTranscriptMessage`;
- live-frame processing calls that operation.

Tests cover:

- the captured end-to-end transition from optimistic send, through live
  commit, to a later render window;
- direct stream-frame identity behavior for origin-bearing users;
- existing send-to-thinking expectations updated to the canonical origin id;
- origin-less user and non-user committed events retaining `history:*` ids.

No server reducer, gateway wire contract, SwiftUI view, row sorting, or local
render ordering changes are required. The TASK-2190 render-floor tool ownership
fix remains untouched.

## Rejected Alternatives

### Filter local optimistic rows using `based_on_seq` or age

This cannot distinguish a genuinely pending queued input from a committed row
whose identity was lost. It can hide valid optimistic sends and turns the
mapper into a lifecycle heuristic.

### Match by message text or timestamp

Repeated prompts are valid, attachments complicate text projection, and clock
ordering is not a stable identity. This would revive the ambiguous
reconciliation removed by the origin-id design.

### Locally sort appended rows by guessed seq

The stale optimistic row has no committed history index. More importantly,
clients must not reconstruct server transcript ordering. Sorting the symptom
would violate the dumb-render contract while leaving duplicate state alive.

### Change the server reducer

The captured transcript already produces the correct origin-bearing refs and
canonical order. Changing the server would move the fix away from the layer
that corrupts the identity.

## Validation

1. Preserve the current failing output from
   `GaryxMobileOldUserTailReproTests` as the before-fix proof.
2. Run the same test after the implementation and require it to pass.
3. Run focused stream actor, render mapper, merge, cache, and prepared
   transcript tests.
4. Run the full `swift test` suite.
5. Build the iOS app target with `xcodebuild` against the simulator SDK.
6. Run `garyx-models` tests to guard the unchanged server render contract.
