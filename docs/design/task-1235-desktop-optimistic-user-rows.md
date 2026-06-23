# Desktop Optimistic User Rows

Task: `#TASK-1235`

## Problem

Desktop solo thread rendering now maps only `render_state.rows` plus seq-backed
message bodies. A sync send still seeds a local user message with a stable
`origin:<intentId>` id, but the message has no `seq` until the gateway commits
it. Because `buildThreadViewRows` resolves user rows only through
`messagesBySeq`, the seeded message stays in `activeMessages` but never becomes
a visible user bubble while the app waits for the committed
`thread_render_frame`.

This is a desktop rendering-layer regression. The existing origin-id strategy
and merge reconciliation already line up with the server and iOS behavior.

## Constraints

- `render_state.rows`, `tailActivity`, `activeToolGroupId`, assistant turn
  grouping, final-answer placement, and tool grouping remain server-owned.
- The desktop may add optimistic local user rows and pending-ack chrome after
  server-derived rows.
- Do not change `seededUserBubble`, `userMessageIdForOrigin`,
  `messageOriginId`, `normalizeTranscriptMessageId`, or remote transcript merge
  semantics unless a test proves they are wrong.
- Failed retryable user messages must remain visible.
- Existing activity derivation must still see the seeded local user message so
  `showPendingAckLoading` is not regressed.

## Proposed Change

Add a pure render-view-model helper that appends unresolved local user messages
after the server-derived solo rows:

```ts
buildThreadViewRowsWithLocalUsers(renderState, messagesBySeq, activeMessages)
```

The helper will:

1. Call the existing `buildThreadViewRows(renderState, messagesBySeq)` first.
2. Compute `representedMessageIds` from the resulting rows and their nested
   message blocks.
3. Compute committed ids from `messagesBySeq` values.
4. Append one `user_turn` row for each `activeMessages` entry where:
   - `role === "user"`;
   - `localState` exists and is not `"remote_final"`;
   - `representedMessageIds` does not contain `message.id`;
   - committed ids do not contain `message.id`;
   - the message is not an internal loop-continuation row.

The appended row shape will reuse the existing user-message rendering path:

```ts
{
  kind: "user_turn",
  key: `user-turn:${message.id}`,
  userBlock: messageBlock(message),
  activityRows: [],
}
```

`ThreadPage` will use the new helper for solo threads only. Team-mode flattening
continues to use `buildThreadViewBlocks` unchanged. The existing
`activePendingAckIntents` and `visibleRemotePendingInputs` overlays stay where
they are; sync sends do not use `activePendingAckIntents`, so the local row
fills only the missing solo-thread sync-send case.

## Dedupe

The dedupe rules are intentionally redundant:

- `representedMessageIds` covers the committed frame once `render_state` points
  at the origin-stable user id.
- committed ids from `messagesBySeq` cover the same transition even if a loaded
  committed body exists before a row body is rendered.
- `mergeRemoteTranscriptWithLocal` already removes the optimistic user when the
  materialized committed message with the same `origin:<intentId>` appears.

No new origin-id mapping is introduced.

## Failure State

Desktop currently records dispatch failures on the assistant placeholder/error
row, while the seeded user row remains local and optimistic. That keeps the
intent data around, but after Block 4 the user text can still disappear because
the local user row is not rendered.

The renderer helper must keep any local non-`remote_final` user row visible, so
the current production failure shape still shows the user's text. To satisfy the
retry-chrome contract, the implementation should also mark the matching seeded
user row with `error: true` and `localState: "error"` or `"interrupted"` when a
dispatch/run failure is recorded. `ThreadPage` already renders retry chrome for
errored user rows with an `intentId`, and `handleRetryFailedMessage` already
clears those user error marks and removes the assistant error row before
re-dispatching.

Do not render local assistant error rows through the new helper. The allowed
local transcript overlay for this fix is the user row; assistant activity,
thinking, tool state, and final-answer placement remain server-owned.

## Storybook

`StorybookApp` currently passes `renderState={null}` for all scenarios. With the
new helper, the existing `optimistic · dispatching_sync` and failure scenarios
render their local user rows through the same row model as production. The
storybook copy already describes the desired behavior and should remain aligned
with the implementation.

## Tests

Use `render-view-model.test.mjs` for no-UI reproduction and validation:

1. Red test: `render_state` has no matching user row, `activeMessages` contains
   an optimistic user with id `origin:intent-1`; output rows must include that
   user bubble.
2. Dedupe test: when `render_state` references `origin:intent-1` and the
   committed body is in `messagesBySeq`, the local optimistic entry is not
   appended a second time.
3. Failure test: a local user with `localState: "error"` or `"interrupted"` and
   `error: true` remains visible, preserving the existing retry chrome path.
4. Optional regression guard: the current production shape, optimistic user plus
   matching local assistant error row, still renders the user row exactly once.

Focused validation:

- First run the red reproduction command and capture the failure before
  implementation.
- After implementation run:
  `cd desktop/garyx-desktop && npm run test:unit -- src/renderer/src/render-view-model.test.mjs`
  if script args are supported; otherwise run the direct node test command.
- Run `cd desktop/garyx-desktop && npm run build:ui`.
- Because this changes renderer UI behavior but not preload, packaging, or
  installed-app resource wiring, a packaged-app check is useful but not the
  only proof. If time permits, run `npm run dist:dir` and attach to the
  installed app for a sync-send smoke.

## Risks

- Appending local rows at the wrong layer could accidentally apply to team mode
  or duplicate pending-ack/remote-pending overlays. Keep the helper wired only
  to solo `turnRows`.
- Filtering only by `representedMessageIds` could leave a transient duplicate
  if committed bodies arrive before rows. Include committed ids as a second
  guard.
- Hiding failed rows by filtering only `"optimistic"` would regress retry
  visibility. Include all local non-`remote_final` user states.
