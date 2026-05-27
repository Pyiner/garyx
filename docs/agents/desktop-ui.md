# Desktop UI Rules

## Transcript And Threads

- Treat transcript history as user-turn based: one user message plus the
  following agent activity until the next user message.
- Pagination, prefetch, folding, and final-answer visibility should use the
  user-turn unit rather than raw provider messages or tool-call counts.
- Keep completed user-turn final answers visible when turns collapse.
- While a thread is still running, keep active turn containers stable and
  reserve Working/Worked rows for real tool activity.
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
