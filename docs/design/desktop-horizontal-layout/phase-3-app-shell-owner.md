# Phase 3: app-shell CSS ownership

Phase 3 keeps the live layout policy on `legacy` and changes only how the
already-published frame is consumed. It does not enable window expansion,
change the 1180 DIP BrowserWindow minimum, add the 960/961 side-tools
auto-hide behavior, or change the 245 DIP sidebar default.

## One task-tree decision source

`ThreadTaskTreePopover` now receives `taskTreeDocked` from the live layout
frame. The component uses that prop to decide whether the compact header
trigger exists and closes an open compact popover when the frame becomes
docked. Its former DOM-width measurement, `ResizeObserver`, and
`isDockedTaskTree` call are gone.

The docked geometry consumes the matching atomic
`data-task-tree-presentation="docked"` frame attribute. The old
`@container thread-task-tree (min-width: 1088px)` policy is removed. Browser
child-view bounds, terminal fitting, side-tools content measurement, and
composer-height observers remain because they do not decide horizontal shell
policy.

## Always-loaded owner

[`app-shell.css`](../../../desktop/garyx-desktop/src/renderer/src/styles/app-shell.css)
is imported exactly once by the renderer root. It owns the complete horizontal
recipe for:

- the AppShell L1/L2/main tracks;
- the conversation, side-tools, and thread-log tracks;
- collapsed, compact-overlay, and hidden presentation geometry;
- sidebar, side-tools, and thread-log resizers; and
- the shell titlebar drag/no-drag composition.

Every owned horizontal grid column consumes concrete `--gx-*` pixel variables.
No owned column uses `fr`, `minmax()`, percentage, `calc()`, media policy, or a
container query. The temporary Phase 2 variable aliases are removed. Vertical
feature layout and embedded side-chat sizing remain feature-owned.

## Executable contracts

`app-shell-owner-contract.test.mjs` fails if:

- the owner import is missing or duplicated;
- an owned horizontal selector escapes into another feature stylesheet;
- an owned column reintroduces CSS width negotiation or a breakpoint;
- task-tree DOM measurement returns, or the required non-horizontal observers
  are removed; or
- the painted sidebar no-drag carveout becomes conditional or ceases to be the
  final AppShell child.

The packaged acceptance gate remains the Phase 0a normalized rect/computed
track/class/attribute oracle. Native drag regions are additionally checked in
the installed Electron app because they cannot be proven by a headless DOM
test.
