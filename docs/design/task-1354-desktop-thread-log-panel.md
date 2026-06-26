# Task 1354: Desktop Thread Log Panel Cleanup

## Goal

Garyx Mac desktop thread logs should be a single service-side log tail. The
panel must not expose or retain the former renderer/client stream-event log
source, and opening logs must not compete for horizontal space with the task
tree popover.

## Current Seams

- `desktop/garyx-desktop/src/renderer/src/app-shell/components/ThreadLogPanel.tsx`
  renders the source `ToggleGroup`, client log list, gateway log lines, and the
  `Latest` action.
- `desktop/garyx-desktop/src/renderer/src/app-shell/components/ThreadPage.tsx`
  accepts and forwards all log panel props, and calls
  `shouldShowThreadTaskTreePopover` before rendering `ThreadTaskTreePopover`.
- `desktop/garyx-desktop/src/renderer/src/app-shell/AppShell.tsx` owns the log
  panel state, gateway log polling, active-tab state, client log buffering, and
  scroll/unread handlers.
- `desktop/garyx-desktop/src/renderer/src/app-shell/diagnostics-helpers.ts`
  contains both gateway log helpers and client stream log builders/trimmers.
- `desktop/garyx-desktop/src/renderer/src/app-shell/types.ts` defines
  `ClientLogEntry` and `ThreadLogTab`.
- `desktop/garyx-desktop/src/renderer/src/app-shell/diagnostics-helpers.test.mjs`
  currently tests client log entry building/trimming.
- `desktop/garyx-desktop/src/renderer/src/app-shell/components/thread-task-tree-popover-model.ts`
  is the pure visibility rule for the task tree popover.
- `desktop/garyx-desktop/src/renderer/src/i18n/index.tsx` contains log source
  labels that become unused once the source switcher is gone.
- `desktop/garyx-desktop/src/renderer/src/styles.css` includes log panel tab and
  client-list styles. The `thread-log-client-*` styles are also used by
  `RendererPerformancePanel`, so only truly orphaned panel styles should be
  removed.

## Client Log Removal Plan

Remove the renderer/client log feature end to end:

- Delete `ClientLogEntry` and `ThreadLogTab` from `types.ts`, and remove the
  now-unused `DesktopChatStreamEvent` type import from that file.
- Delete client stream log helpers from `diagnostics-helpers.ts`:
  `MAX_CLIENT_STREAM_LOG_ENTRIES`, `buildClientStreamLogEntry`,
  `appendClientStreamLogEntry`, and their private formatting/stringifying
  helpers. Keep gateway log line parsing, panel width helpers, side-tools width
  helpers, and gateway indicator helpers.
- In `diagnostics-helpers.ts`, delete the now-unused `ClientLogEntry` and
  `DesktopChatStreamEvent` imports; keep `ConnectionStatus`,
  `GatewayIndicatorTone`, and `ThreadLogLine`.
- Update `diagnostics-helpers.test.mjs` to remove client log tests and imports;
  keep the side tools width tests.
- In `AppShell.tsx`, remove:
  - `threadLogsActiveTab` state/ref and selection handler.
  - `threadLogsOpenRef` and its sync effect, because it is only used by the
    client-log flush path.
  - `clientLogsByThread`, `clientLogsHasUnread`,
    `expandedClientLogEntries`.
  - `clientLogSequenceRef`, `pendingClientLogEventsRef`,
    `clientLogFlushFrameRef`.
  - `flushPendingClientLogEvents` and `enqueueClientLogEvent`.
  - the `enqueueClientLogEvent(event)` call in the chat stream subscription.
  - tab/client branches in `activeThreadLogsPath`,
    `activeThreadLogsHasUnread`, header unread state, content scroll handler,
    and latest jump handler.
- In side-chat and storybook `ThreadPage` call sites, stop passing removed
  client log props and tab callbacks.
- In `ThreadPage.tsx`, remove client log props and tab callbacks from the prop
  type/destructuring and from the `ThreadLogPanel` invocation. Rename the stale
  `mobileThreadLogLines` prop/derived value to `threadLogLines` so the old tab
  value does not survive as frontend API naming.
- In `ThreadLogPanel.tsx`, remove the ToggleGroup import, client log imports,
  client log props, source-switch handler, client log list rendering, and
  client empty state. Render only gateway log lines.
- Remove unused i18n keys: `Client Logs`, `Log sources`, `Hide`, `Show`, and
  `No client stream events yet.` Keep `Gateway Logs` as the static panel label
  and intentionally set the Chinese label to `服务端日志` so the UI reads as a
  server-side log surface after the client log source is removed.
- Remove `.thread-log-panel-tabs` if it has no remaining users. Keep
  `.thread-log-client-*` structural styles because the renderer performance
  panel reuses them. Remove orphaned stream-event-only modifiers if no remaining
  renderer code references them: `.thread-log-client-entry-error`,
  `.thread-log-client-entry-type.type-assistant-delta`,
  `.thread-log-client-entry-type.type-tool-use`,
  `.thread-log-client-entry-type.type-tool-result`, and
  `.thread-log-client-entry-type.type-error`.

The chat stream subscription itself must remain; only the client-log side effect
is removed.

## Single Source Panel Rendering

`ThreadLogPanel` will become a service-log panel:

- Keep `activeThreadTitle`, `selectedThreadId`, `activeThreadLogsPath`,
  `activeThreadLogsHasUnread`, `threadLogsError`, `threadLogsLoading`,
  `threadLogLines`, `threadLogsRef`, `onJumpToLatest`, and `onContentScroll`.
- Replace the source ToggleGroup with a quiet static toolbar label using
  `t("Gateway Logs")`; keep the `Latest` button only as the resume-to-tail
  affordance when `activeThreadLogsHasUnread` is true.
- Always render `threadLogsError` when present.
- Always render `threadLogLines`, otherwise show `Loading logs...` or
  `No logs yet.`.

## Tail-Follow Behavior

The existing gateway log polling loop already has the right primitive:
`threadLogsNearBottom()` checks the scroll position before a poll result is
applied, and `scrollThreadLogsToLatest()` scrolls the log container.

The revised single-source behavior:

- On panel open or selected-thread change, clear `threadLogsHasUnread` and
  request an immediate scroll to the bottom.
- On a reset response from `getThreadLogs`, replace the text with the retained
  tail and request an immediate bottom scroll. This covers opening the panel and
  log rotation/reset.
- On appended chunks, capture `wasNearBottom` before applying text:
  - if true, clear unread and request an immediate bottom scroll after React
    paints;
  - if false, set `threadLogsHasUnread` and do not move the viewport.
- On content scroll, if the container is near the bottom, clear
  `threadLogsHasUnread`. The next appended chunk will follow again because
  `threadLogsNearBottom()` becomes true.
- Keep `Latest` as a manual resume button: clear unread and smooth-scroll to the
  bottom. This replaces the former cross-tab unread jump behavior with a
  service-log-only tail affordance.

Implementation note from packaged-app verification: the parent polling loop's
requestAnimationFrame scroll can run before the mounted panel has its final
layout. `ThreadLogPanel` should therefore also keep a local non-durable
`shouldFollowTailRef`, update it from the scroll container, and run a
layout/post-frame bottom scroll on mount and new line keys only while
tail-following is active. The service log tail is capped at 100 lines, so
`.thread-log-line` should participate in normal layout instead of using
`content-visibility`; lazy row heights make the initial `scrollHeight` unstable
and break open-at-bottom behavior. This preserves the standard "user scrolls up
to pause, returns to bottom or clicks Latest to resume" behavior without
reintroducing durable tab/client state.

## Task Tree Popover

Use the existing pure visibility model instead of adding JSX-only guards:

- Add `threadLogsOpen: boolean` to
  `shouldShowThreadTaskTreePopover(...)`.
- Return false when `threadLogsOpen` is true.
- Pass `threadLogsOpen` from `ThreadPage.tsx`.
- Extend `thread-task-tree-popover-model.test.mjs` with a case proving logs
  open hides the task tree.

This mirrors the existing `inspectorOpen` exclusion and keeps the layout
mutual-exclusion policy testable.

## Validation

Focused checks:

- `cd desktop/garyx-desktop && npm run build:ui`
- `cd desktop/garyx-desktop && npm run test:unit`
- `cd desktop/garyx-desktop && npm run test:i18n-literals`

Repository has no `npm run lint` script in `desktop/garyx-desktop/package.json`;
if no root lint script exists, record that explicitly in the final evidence
instead of inventing one.

Residual-code scan:

- `rg "ClientLogEntry|ThreadLogTab|clientLogs|clientThreadLogEntries|expandedClientLogEntries|onToggleClientLogEntry|onSelectThreadLogsTab|threadLogsOpenRef|mobileThreadLogLines|Client Logs|No client stream events|Log sources" desktop/garyx-desktop/src/renderer/src`

Real Mac app validation:

- `cd desktop/garyx-desktop && npm run dist:dir`
- Quit stale `Garyx` processes.
- Open the installed `Garyx.app`.
- Attach with `playwright-cli -s=<session> attach --cdp=http://127.0.0.1:39222`.
- Capture screenshots proving:
  - log panel shows only service/gateway logs and no client tab;
  - no source ToggleGroup is present;
  - opening logs leaves the content at the bottom / `Latest` only appears when
    manually scrolled away from the tail and new logs arrive;
  - `ThreadTaskTreePopover` is not visible while logs are open.
