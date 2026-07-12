# Phase 0b: normalized occupancy event log

Phase 0b adds one synchronous shadow path for horizontal panel intent while
leaving the existing React state, resize controller, DOM, CSS, and native
window behavior unchanged. The log emits the final-v4 protocol shape
`LAYOUT_INTENT_CHANGED { previousOccupancy, nextOccupancy, cause,
transactionId }`; no reducer or effect channel consumes it until Phase 1.

## Writer routing

| Surface | Normalized writer | Cause and edge rule | Legacy application |
| --- | --- | --- | --- |
| Global sidebar | Both visible/final drag-carveout toggles call the same AppShell wrapper | Normal-width toggle emits `user-panel`; compact temporary presentation emits nothing | Existing `useLayoutResizeController` callback remains the only UI writer |
| Conversation rail | Recent, bot, and workspace open/close/toggle plus pinned/root navigation | `user-route`; rail-to-rail identity switches emit one replace event even when the four booleans are unchanged | One bridge writes the same three mutually exclusive React states |
| Conversation rail cleanup | Non-thread route, missing bot group, or missing workspace group | `system-cleanup` only when a requested rail is removed | Existing cleanup effects remain in place |
| Side tools header | Inspector toggle and whole-dock close | `user-panel`; only the inspector/capsule union's 0↔1 edge emits | Inspector and capsule React state remain unchanged |
| Workspace preview | Local-link request and modal-open effect | One eventual `user-route` vector; a later legacy effect is a log no-op | The local-link path keeps its original early inspector write and later logs close |
| Capsules | Open/activate, individual tab close, and thread/route cleanup | Navigation is `user-route`; cleanup is `system-cleanup`; tab count changes inside an already occupied union do not emit | The same tab arrays and pending activation state are used |
| Thread logs | Header toggle | One `user-panel` full-vector replacement clears inspector and capsules and toggles logs | Existing mutually exclusive right-panel result is unchanged |
| Thread logs cleanup | Escape, non-thread route, or no selected thread | `system-cleanup` | Existing effects/key handler still perform the visible close |

`AppShell` keeps two shadow snapshots. The desired snapshot is synchronously
logged as one full vector. The applied snapshot feeds the old setters. They are
normally identical; the split exists only where the old UI intentionally lands
state through more than one React commit (workspace preview and new-thread
capsule cleanup). This prevents the event protocol from depending on React
batching without changing the legacy commit order.

## Proof against missed writers

- [`layout-occupancy-events.test.mjs`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/layout-occupancy-events.test.mjs)
  covers the four-panel projection, capsule union edges, rail identity replace,
  logs full-vector replace, causes, transaction sequence, and bounded history.
- [`layout-occupancy-writers.test.mjs`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/layout-occupancy-writers.test.mjs)
  is a source contract: each of the six raw React setters has exactly one call
  site inside the bridge, retired setter names cannot reappear, the compact
  sidebar bypass is pinned, and route/capsule/cleanup entry points must invoke
  the bridge.
- Phase 0a's packaged structural oracle remains the behavior gate. Phase 0b
  must match all eight scenarios exactly after a fresh packaged build.
