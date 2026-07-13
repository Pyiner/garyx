# Desktop UI Rules

## Transcript And Threads

- Render transcript history from server `render_state.rows`. User-turn
  grouping, assistant steps, tool groups, filtered placeholders, and final
  answer placement are reducer output, not desktop heuristics.
- Pagination and cache sync may read committed ledger events and cursors, but
  folding and visible row structure must follow `render_state`.
- Tail thinking uses `render_state.tailActivity`; active tool highlighting uses
  `render_state.activeToolGroupId`.
- Keep completed user-turn final answers visible when `render_state` places them
  in a collapsed turn.
- While a thread is still running, keep active turn containers stable and
  reserve Working/Worked rows for real tool activity from `render_state`.
- Pure assistant/reasoning text remains normal assistant text.
- Expanded tool activity grows to its natural height. The transcript owns
  vertical scrolling; tool groups and nested activity must not introduce
  max-height caps or inner vertical scrollbars.
- Desktop interruption controls must be gateway-backed.
- The local Mac app process may not own the active WebSocket for runs started
  elsewhere or after a reload; after trying any local active socket, call the
  gateway chat interrupt endpoint so the bridge can interrupt or abort the
  active thread run.

## Menus And Popovers

- All dropdown menus, select popups, menu-like popovers, and icon menu
  triggers render from the shared recipe in
  `desktop/garyx-desktop/src/renderer/src/styles/menus.css` (extracted 1:1
  from the ChatGPT/Codex Mac app via CDP-measured computed styles).
- Per-surface CSS may set menu widths or add semantic variants, but must not
  fork the surface: no local menu backgrounds, borders, radii, shadows, item
  typography, or hover washes.
- Menu shortcut hints use the shared `DropdownMenuShortcut` component, not
  local spans.
- Persistent sidebar popovers that sit over high-contrast navigation content
  may use the shared `.menu-popover-surface-opaque` variant. It changes only
  the surface fill; radius, ring, shadow, typography, rows, and spacing still
  come from `styles/menus.css`.

## App-Shell Chrome Ownership

- Global shell components must own one complete, always-loaded CSS recipe.
  Do not scatter required layout declarations across unrelated feature files.
- The bottom-left gateway identity, settings control, presence glyph, and
  switcher popover are owned by `styles/gateway-status.css`. A contract test
  must fail if those selectors escape that file or its root import disappears.
- Deleting an optional feature must not remove styles required by navigation,
  the conversation header, the sidebar footer, or other persistent chrome.

## Responsive Conversation Layout

- Keep the desktop app shell horizontal at every supported window width.
  Narrow windows collapse a rail; they must never stack the whole sidebar
  above the conversation or turn the viewport into a document-height page.
- Auto-hide Garyx's secondary conversation rail at 980px and below before
  auto-collapsing the global rail at 720px and below. Preserve both desired
  states so widening restores them. An explicit compact global-rail open must
  enter the native expansion transaction and remain in flow; do not present it
  as an overlay. Responsive auto-hide itself must never write the user's
  desktop-width collapse preference.
- A funded rail close publishes the closed layout frame immediately, waits one
  animation frame for paint, and only then shrinks the native window. Do not
  add a fixed close delay unless a real transition reports its completion.
- The transcript owns a 768px outer scroller with 16px internal gutters, which
  yields a 736px reading edge. The composer uses the same 736px edge directly;
  both become `available width - 32px` when constrained.
- On an existing thread, the composer surface stays 16px above the window
  bottom at every supported height. Workspace/branch context belongs inside
  the composer footer; it must not create a second row that pushes the input
  surface upward. The thread body starts directly below the 46px toolbar.
- A task tree may reserve its 320px rail only when the thread canvas is at
  least 1088px wide, leaving the full reading column intact. Below that width,
  expose the tree through the 28px conversation-header control and show it as
  a dismissible overlay with Escape, outside-click, and focus-return behavior.
- Side tools have one presentation: a right-docked rail, approximately 320px
  by default. The conversation-header control toggles that rail directly; do
  not add a second inset/overlay form. The rail may still be resized and must
  keep its contents responsive within the available width.
- Thread logs are a built-in side-tools item and replace the former side-tools
  Tasks item. They inherit the right rail's open, close, resize, and responsive
  behavior; they have no independent occupancy, funding, placement, resizer, or
  width preference. The global Tasks entry and `#/tasks` route and the thread
  task tree remain unchanged.
- Floating task trees use the available height between their top and bottom
  insets. Do not cap them to a percentage of viewport height on tall windows or
  turn the app shell into document-height vertical stacking.
- Validate responsive changes in the installed app around the actual seams:
  640, 720/721, 980/981, 1332/1333, and a wide desktop width. Restore the
  user's original window size after CDP measurement.

## Workspace File Tree

- The workspace file browser should read directories on demand.
- Do not pre-scan child directories just to decide whether to show expansion
  affordances, especially on macOS where probing protected folders can trigger
  privacy prompts.

## Product UI Skill

Desktop chat, transcript, workspace selector, and file-tree interaction details
live in the `garyx-product-ui` skill. Use that skill for non-trivial desktop UI
implementation or review.
