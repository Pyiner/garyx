# Desktop Composer Interrupt State

## Problem

The desktop transcript can show a server-derived active tail (`thinking`,
`assistant_streaming`, or an active tool group) while the composer still renders
the send arrow. The composer currently derives `isActiveSendingThread` from the
local message-machine runtime, so cold-open, reconnect, and cross-entry thread
opens can miss runs that were started elsewhere.

That splits the visible transcript state from the interrupt affordance:

- Transcript rows and tail activity come from server `render_state`.
- The composer stop/send toggle comes from local runtime state.
- The interrupt handler returns before calling the gateway if local runtime is
  missing or idle.

## Goals

- Make the composer stop/send toggle follow the same server-render activity
  snapshot that drives transcript working presentation.
- Preserve the existing pending-ack loading window for newly sent local input.
- Keep interruption gateway-backed so a renderer that did not start the run can
  still interrupt the selected thread.
- Keep the change in pure renderer logic with focused tests.

## Design

Keep `deriveThreadActivityModel` unchanged. It is the cross-platform
conversation-state activity contract used by desktop, iOS, and shared fixtures;
its `runActive` output remains the local runtime business gate.

Add a narrow desktop renderer selector in `app-shell/thread-activity.ts`, for
example `deriveThreadComposerControlModel`, with inputs:

- `hasThread`: whether this composer is bound to an existing selected thread.
- `renderTailActivity`: selected thread `render_state.tailActivity`, or `null`
  when no render snapshot exists yet.
- `renderActiveToolGroupId`: selected thread `render_state.activeToolGroupId`,
  or `null`.
- `runtimeBusy`: the existing local message-machine runtime busy value.
- `showPendingAckLoading`: the existing `deriveThreadActivityModel` output for
  the optimistic pre-ack window.

The selector returns `isActiveSendingThread`, the exact value passed to
`ComposerForm`/`ComposerQueue`.

Remote run activity is derived from the server render snapshot:

- `tailActivity` of `thinking`, `assistant_streaming`, or `tool_active` means
  active.
- A non-null `activeToolGroupId` also means active.

The race-safe composer rule is:

```
hasThread && (showPendingAckLoading || runtimeBusy || renderActive)
```

This is an OR, not a render snapshot replacement. It prevents a stale idle
render snapshot from making the stop button flash back to send while a local
runtime is already busy and the new `thread_render_frame` has not arrived yet.
It also covers cold-open/reconnect/cross-entry opens because `renderActive` is
true even when local runtime is idle.

In all cases, `showPendingAckLoading` also keeps the composer in stop mode for
newly submitted local input before server render activity appears. This means
`deriveThreadActivityModel` is still reused for the local pending-ack window and
queue-steering gates, while server `render_state` owns the durable running
truth for an opened thread.

Do not make this selector responsible for clearing stale local runtime. Runtime
convergence remains owned by the existing committed-event/run-state paths that
drive `thread/runtime` back to idle. Once runtime and render activity are both
idle, the selector restores the send arrow.

Pass the selected thread and side-chat render activity fields into this selector
and replace the local inline `Boolean(selectedThreadId && ...)` computation in
both call sites. Do not add ad hoc booleans in `AppShell`.

Update `interruptThread` so it always calls
`window.garyxDesktop.interruptThread(threadId)` for a valid thread id. If a
local busy runtime exists, keep the current optimistic local interruption
cleanup. If no local runtime exists, skip local intent cleanup and rely on the
gateway interrupt plus the scheduled history refresh to converge the renderer.

## Impact

- Main composer and side-chat composer get the same behavior.
- Cold-open/reconnect/deep-link/task/list opens work because selected-thread
  `render_state` is keyed by thread id, not by local run ownership.
- The shared conversation-state activity contract, fixtures, and iOS twin are
  not changed. Add a carve-out to `docs/agents/conversation-state.md` that the
  Mac composer interrupt/send toggle is a desktop renderer control, not a
  `ThreadActivityModel.runActive` output.
- No server, preload, or main-process API change is needed; the existing
  interrupt endpoint already accepts `threadId`.

## Validation

- Red/green renderer logic test:
  `node --experimental-strip-types --test desktop/garyx-desktop/src/renderer/src/app-shell/thread-activity.test.mjs`
- Cover thinking, active tool group, stale idle render plus local runtime busy,
  and no-thread idle cases in the same pure test file.
- Run the desktop unit/conformance subset affected by this model:
  `node --experimental-strip-types --test desktop/garyx-desktop/src/renderer/src/app-shell/thread-activity.test.mjs desktop/garyx-desktop/src/renderer/src/conversation-state-conformance.test.mjs`
- The shared fixture file and iOS implementation are intentionally unchanged;
  the desktop conformance test proves `deriveThreadActivityModel` still matches
  the existing contract.
- Run TypeScript/build validation if the reviewer requests broader proof:
  `npm --prefix desktop/garyx-desktop run build:ui`

Packaged-app validation is not required because this is a renderer state
selection and IPC-call gating fix, not a bundling or installed-app behavior
change.
