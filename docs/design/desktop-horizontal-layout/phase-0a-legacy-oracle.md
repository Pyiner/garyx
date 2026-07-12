# Phase 0a: legacy horizontal-layout inventory and oracle

This inventory characterizes the behavior that Phases 0b–3 must preserve. It
does not enable the `expand-v1` policy or change native window bounds.

## Occupancy entry points

| Panel | Current writers and cleanup paths | Legacy effect | Phase 0b cause |
| --- | --- | --- | --- |
| Global sidebar | `useLayoutResizeController.toggleSidebarCollapsed`; both the focusable toggle and the always-last drag carveout call the same callback | Normal width toggles `garyx.sidebarCollapsed`; compact width toggles only temporary in-window presentation | `user-panel`; compact presentation stays outside occupancy transactions |
| Conversation rail | `onOpenRecent`, `onToggleBotConversationGroup`, `onToggleWorkspaceThreadGroup`; each rail's `onClose`; route/data cleanup effects for unavailable rail content | Recent, bot, and workspace are mutually exclusive occupants of one L2 track; switching is a replacement, not two physical steps | User navigation is `user-route`; route/data invalidation is `system-cleanup` |
| Side tools | Header inspector toggle; workspace preview request/modal; capsule open, tab close, whole-dock close; thread/content cleanup | Occupancy is the union `inspectorOpen || openCapsuleTabs.length > 0`; only its 0↔1 edge changes the track | Header is `user-panel`; preview/capsule navigation is `user-route`; cleanup is `system-cleanup` |
| Thread logs | Header toggle; Escape; no-thread/content-view/new-thread cleanup | Opening logs clears inspector and capsule occupancy in the same user action; logs then use existing dock/overlay policy | Header replacement is `user-panel`; cleanup is `system-cleanup` |

The full writer list lives in
[`AppShell.tsx`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/AppShell.tsx).
Phase 0b must route those writers through one full-vector occupancy event log
while leaving their existing React setters/controller behavior intact.

## Resizing and responsive policy

- Sidebar drag clamps to 245–520 DIP and pointer cancellation commits the last
  preview, matching pointer-up.
- Conversation-rail drag clamps to 220–420 DIP, writes the preview CSS variable
  at most once per animation frame, and commits the last preview on pointer-up
  or pointer cancellation.
- Side tools clamp to 320–1180 DIP and keep a 540 DIP primary-canvas budget.
- Thread logs clamp to 280–760 DIP, use a 10 DIP resizer, and dock only when the
  measured thread canvas retains the 540 DIP comfort width.
- The global sidebar compact breakpoints remain 720/721 without L2 and 980/981
  with L2. Compact manual open does not overwrite the desktop preference.
- The task tree still uses its pre-migration DOM `ResizeObserver` plus the
  1088 DIP container query in this phase. Browser, terminal, composer, and
  other non-horizontal observers are explicitly outside the policy migration.

These values are pinned by table-driven tests in
[`responsive-layout-model.test.mjs`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/responsive-layout-model.test.mjs)
and
[`diagnostics-helpers.test.mjs`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/diagnostics-helpers.test.mjs).

## Packaged structural oracle

The normalized fixture is
[`legacy-horizontal-layout-oracle.json`](../../../desktop/garyx-desktop/src/renderer/src/app-shell/fixtures/legacy-horizontal-layout-oracle.json).
It records seven 1480×940 packaged scenarios:

1. baseline;
2. sidebar collapsed;
3. side tools;
4. thread logs;
5. Recent L2 rail;
6. Recent L2 plus side tools;
7. Recent L2 plus thread logs.

For each scenario it records rectangles, computed grid tracks, semantic class
tokens, state-bearing ARIA attributes, the four-panel desired occupancy, and
the drag/no-drag carveout. Dynamic text, element IDs, workspace paths, thread
IDs, and task-tree content height are deliberately excluded.

Reproduce it against a freshly installed packaged app:

```bash
cd desktop/garyx-desktop
npm run dist:dir
# Quit any stale Garyx process, launch the newly installed app, and attach CDP.
npm run layout:oracle -- --compare
```

Use `GARYX_LAYOUT_CDP_ENDPOINT` when the packaged app is not using the project
default endpoint. `--write` is only for intentionally recording a new Phase 0
baseline; Phases 0b–3 use `--compare` and must match exactly.
