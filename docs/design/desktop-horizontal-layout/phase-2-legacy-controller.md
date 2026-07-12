# Phase 2: live legacy-policy frame adapter

Phase 2 replaces the horizontal policy inside
[`useLayoutResizeController.ts`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/useLayoutResizeController.ts)
without enabling window expansion or any `expand-v1` presentation behavior.
The BrowserWindow minimum remains 1180 DIP, the sidebar remains 245 DIP, and
side tools do not gain the 960/961 auto-hide behavior.

## One live frame

[`horizontal-layout-frame-store.ts`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/horizontal-layout-frame-store.ts)
is the renderer adapter around the Phase 1 pure module. It owns the single
live `HorizontalLayoutState`, reduces normalized occupancy/viewport/width
events under the `legacy` policy, projects one stable frame, and exposes that
frame through `useSyncExternalStore`. React derives compact, collapsed,
dock/overlay, and preferred-width values from that frame; it does not keep a
second horizontal policy state.

The Phase 0b bridge now sends its exact `LAYOUT_INTENT_CHANGED` event into the
store before applying the existing component-state writers. The state
machine's native bounds effect remains deliberately disconnected. The legacy
store folds the ordered desired-occupancy checkpoint locally and then compacts
the settled transaction, because no native Phase 2 result can refer back to
it. There is no renderer call to a native bounds executor in Phase 2.

## Atomic DOM publication

`applyFrame(root, frame)` synchronously writes every `--gx-*` pixel variable,
every presentation `data-*` attribute, and the temporary legacy CSS aliases.
`data-layout-revision` is written last as the commit marker. The aliases keep
the Phase 2 styles byte-for-byte compatible while Phase 3 moves their recipes
into the always-loaded `styles/app-shell.css` owner:

- `--spacing-token-sidebar` and `--app-sidebar-width`;
- `--spacing-token-rail`;
- `--side-tools-panel-width` and its resizer width; and
- `--thread-log-panel-width` and its resizer width.

The AppShell, conversation, and thread-layout elements no longer carry
independent inline horizontal variables. A callback ref attaches the frame
store to each AppShell mount, so loading/setup branch changes cannot leave a
new shell root without the current pre-paint frame.

## Legacy interaction contract

All four resizers dispatch `PANEL_WIDTH_CHANGED` into the same store. The L2
preview remains capped at one requestAnimationFrame, pointer cancellation
still commits the last preview, and only thread-log width is persisted.
`garyx.sidebarCollapsed` remains the only sidebar preference. Window resize
events synchronously reduce and publish the numeric variables and presentation
attributes together; the controller no longer uses a DOM `ResizeObserver` to
decide logs dock versus overlay.

Phase 3 still owns the task-tree frame-prop migration, removal of its horizontal
DOM observer/container query, CSS ownership consolidation, and exact px track
recipes. Browser, terminal, composer, and other non-horizontal observers are
outside this migration and remain untouched.
