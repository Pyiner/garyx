# Phase 1: pure horizontal-layout state machine

Phase 1 implements the protocol in [`final.md`](./final.md) without wiring it
to React, Electron, persistence, timers, or native window effects. The live
desktop app still uses the legacy controller and keeps `expand-v1` disabled.

## Pure module boundary

[`responsive-layout-model.ts`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/responsive-layout-model.ts)
owns:

- `createHorizontalLayoutState`, `reduceHorizontalLayout`, and
  `projectHorizontalLayout`;
- separate requested occupancy, acknowledged funding, and in-flight
  transaction state;
- `legacy` and `expand-v1` policy records;
- stable, pending, and rejected frame unions with exact in-flow tracks, pixel
  variables, and presentation attributes;
- `UserCauseToken` and `RepayProof` authority, full checkpoint/bounds/claim
  command contracts, fixed-mode deferral, deadline events, supersession, and
  physical-revision folding.

[`window-layout-protocol.ts`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/window-layout-protocol.ts)
is the headless executor oracle. It has no layout policy. It validates renderer
epoch, queue sequence, the checkpoint gate, both CAS revisions, authority,
fresh-session claims, current work-area containment, and actual bounds
readback before returning a complete acknowledged session. It is not imported
by the live app.

Phase 0b's `LayoutOccupancyEvent` is now the exact
`LAYOUT_INTENT_CHANGED` member of the machine event union, so the normalized
writer bridge and the reducer cannot silently drift into parallel protocols.

## Projection gates

The legacy projection is shadow-compared with all eight packaged Phase 0
scenarios, including the L2 conversation hairline and both logs dock/overlay
samples. Intentional `expand-v1` differences are isolated in
[`expand-v1-horizontal-layout-golden.json`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/fixtures/expand-v1-horizontal-layout-golden.json).

The invariant matrix covers both policies at 480, 720/721, 960/961, 980/981,
1116, 1280, 1480, and 1920 DIP, every valid four-panel occupancy combination,
and normal/maximized/fullscreen modes. It asserts:

- flattened in-flow tracks fill the content viewport;
- accepted stable primary width is at least 350 DIP;
- docked logs leave at least 540 DIP;
- responsive presentation never rewrites requested intent;
- only user/display snapshots update the responsive basis;
- snapshot-only events never mint bounds authority;
- acknowledged funding stays attached to its normal base;
- funded open/close returns to the original bounds; and
- accepted physical facts fold monotonically by window revision, independent
  of transaction sequence.

## Verification

Focused coverage lives in:

- [`horizontal-layout-projection.test.mjs`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/horizontal-layout-projection.test.mjs);
- [`horizontal-layout-machine.test.mjs`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/horizontal-layout-machine.test.mjs); and
- [`window-layout-protocol.test.mjs`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/window-layout-protocol.test.mjs).

The writer source contract scans all renderer TypeScript sources, pins all nine
`system-cleanup` call sites, and asserts that Phase 1 reducer/effect symbols are
not live-wired. Packaged verification continues to use Phase 0's exact
eight-scenario structural oracle.
