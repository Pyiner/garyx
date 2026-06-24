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

Extend `deriveThreadActivityModel` with optional render activity inputs:

- `renderTailActivity`
- `renderActiveToolGroupId`

When a render snapshot is present, `runActive` is derived from the render
snapshot:

- `tailActivity` of `thinking`, `assistant_streaming`, or `tool_active` means
  active.
- A non-null `activeToolGroupId` also means active.
- `tailActivity: "none"` with no active tool group means idle.

When no render snapshot is available yet, keep the existing local runtime
fallback so very early local sends remain gated before the first
`thread_render_frame` arrives.

Then pass the selected thread and side-chat `render_state` fields into
`deriveThreadActivityModel`. `isActiveSendingThread` and side-chat equivalent
continue to use `threadActivity.runActive || showPendingAckLoading`, but
`runActive` is now server-render-state-first instead of local-runtime-only.

Update `interruptThread` so it always calls
`window.garyxDesktop.interruptThread(threadId)` for a valid thread id. If a
local busy runtime exists, keep the current optimistic local interruption
cleanup. If no local runtime exists, skip local intent cleanup and rely on the
gateway interrupt plus the scheduled history refresh to converge the renderer.

## Impact

- Main composer and side-chat composer get the same behavior.
- Cold-open/reconnect/deep-link/task/list opens work because selected-thread
  `render_state` is keyed by thread id, not by local run ownership.
- A stale local runtime no longer keeps the stop button active once a present
  server render snapshot says the run is idle.
- No server, preload, or main-process API change is needed; the existing
  interrupt endpoint already accepts `threadId`.

## Validation

- Red/green renderer logic test:
  `node --experimental-strip-types --test desktop/garyx-desktop/src/renderer/src/app-shell/thread-activity.test.mjs`
- Add active-tool coverage in the same pure test file.
- Run the desktop unit/conformance subset affected by this model:
  `node --experimental-strip-types --test desktop/garyx-desktop/src/renderer/src/app-shell/thread-activity.test.mjs desktop/garyx-desktop/src/renderer/src/conversation-state-conformance.test.mjs`
- Run TypeScript/build validation if the reviewer requests broader proof:
  `npm --prefix desktop/garyx-desktop run build:ui`

Packaged-app validation is not required because this is a renderer state
selection and IPC-call gating fix, not a bundling or installed-app behavior
change.
