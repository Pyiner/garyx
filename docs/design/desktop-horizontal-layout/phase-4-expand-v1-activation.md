# Phase 4: expand-v1 activation

Phase 4 switches the complete desktop horizontal-layout stack to `expand-v1`.
It is the first phase that executes the window/session effects prepared in
Phases 1–3.

## One policy gate

`GARYX_DESKTOP_EXPAND_V1` defaults to enabled. `0`, `false`, `off`, or
`legacy` selects the complete legacy path:

- BrowserWindow `minWidth` is 480 for `expand-v1` and 1180 for `legacy`;
- preload exposes the main-selected policy to the renderer;
- `expand-v1` creates the live effect runner and native session executor;
- `legacy` keeps the Phase 2 local-checkpoint store and never consumes native
  effects.

This makes feature-off a policy rollback, not an IPC kill switch layered over
new responsive constants.

## Main-process executor

Each BrowserWindow owns one `WindowLayoutExecutor` for its lifetime. The
executor keeps the opaque acknowledged session across renderer reloads and
serializes:

1. sender/window and renderer-epoch validation;
2. queue-sequence coalescing;
3. desired-occupancy checkpoint or fresh-session claim;
4. window/session dual CAS and `BoundsAuthority` validation;
5. current mode and work-area validation;
6. one `setBounds` call, actual bounds readback, and acknowledged-session
   commit.

Accepted results return the complete session and authoritative snapshot. A
new renderer epoch supersedes queued old work but cannot erase a command that
already changed physical bounds. `will-resize` / `will-move` mark native user
geometry sessions; display, mode, panel-machine, and hydrate snapshots retain
their distinct origins.

## Renderer adapter

The frame store now accepts either policy. Under `expand-v1`, the effect runner
executes checkpoint, claim, and bounds commands and dispatches the 100 ms open
fallback. A funded close publishes its closed frame immediately and requests
the native shrink on the next animation frame; there is no synthetic close
watchdog without a matching visual transition. Snapshot pushes coalesce by
origin class once per animation frame while preserving the reducer's
user/display versus panel-machine `responsiveBasisWidth` semantics.

AppShell consumes the frame's presented/effective occupancy for track
mounting. Side tools stay mounted with zero bounds while auto-hidden, and the
task tree consumes effective side-tools visibility. A non-fresh bootstrap
restores renderer panel intent before asynchronous DesktopState hydration, so
reload does not show an empty funded placeholder or repay valid funding during
the boot gap.

## Automated verification

- the existing reducer/projection suites cover the final-v4 sequence matrix
  and nine invariants;
- `window-layout-executor.test.mjs` covers sender binding, fresh claim, dual
  CAS, coalescing, work-area TOCTOU, fixed mode, one-call `setBounds`, actual
  readback, delayed acknowledgement, epoch takeover, and external rebasing;
- `horizontal-layout-effect-runner.test.mjs` drives the production main
  executor through a fake host and closes the live open/checkpoint/bounds and
  close/frame/repay loops, including snapshot burst coalescing;
- the complete desktop unit suite and production renderer build pass;
- packaged feature-off matches all eight legacy oracle scenarios.

Packaged CDP verification also covers cold-claim first close, funded
open/close, right-panel primary-rect preservation, full-vector replacement,
reload without a second expansion, reload-during-close orphan repayment,
constrained intent reload, the 480 minimum/350 protection chain, programmatic
resize basis isolation, and a 150 ms late-ack sequence.

## Native-input follow-up

If native pointer input is unavailable, finish the platform-only portion on an
unlocked Mac with the same installed build:

1. Attach CDP on port 39222 and record, on every resize frame,
   `innerWidth`, `sum(columns) - innerWidth`, primary width, presentation, and
   layout revision.
2. With no L2 rail, physically drag through outer widths 721 and 720. With an
   L2 rail open, repeat at 981 and 980.
3. Open side tools above 961, physically drag through 961 and 960, then back;
   confirm hidden is not closed and its selected internal tool survives.
4. Drag to the 480 minimum with competing panels and confirm primary width
   never drops below 350 and the documented degradation order is used.
5. In sidebar, L2, and right-panel states, click every header toggle and drag
   the blank title strip; confirm buttons remain no-drag and the strip moves
   the native window.
