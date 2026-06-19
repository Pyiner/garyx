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
- Desktop interruption controls must be gateway-backed.
- The local Mac app process may not own the active WebSocket for runs started
  elsewhere or after a reload; after trying any local active socket, call the
  gateway chat interrupt endpoint so the bridge can interrupt or abort the
  active thread run.

## Workspace File Tree

- The workspace file browser should read directories on demand.
- Do not pre-scan child directories just to decide whether to show expansion
  affordances, especially on macOS where probing protected folders can trigger
  privacy prompts.

## Product UI Skill

Desktop chat, transcript, workspace selector, and file-tree interaction details
live in the `garyx-product-ui` skill. Use that skill for non-trivial desktop UI
implementation or review.
