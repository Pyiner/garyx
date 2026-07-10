# Perf slice 2 — two-phase thread hydration for desktop `getState`

Status: draft for review (perf round, slice 2)
Owner: desktop + gateway
Related: perf round baseline (boot marks, 7ddfba8e / 9b71a45b, #TASK-1666)

## Problem

Steady-state cold boot of the packaged Mac app spends ~80% of its time in
`state-hydrated` (getState IPC): renderer-entry 111ms → state-hydrated
717ms → first-interactive-frame 737ms. The dominant cost is the `threads`
slice of `mergeRemoteDesktopState`:

- `fetchThreads` requests `/api/threads?limit=1000` on every
  `getDesktopState()` (boot and every refresh).
- Measured against a live gateway with ~2100 threads:

| request | time | payload |
| --- | --- | --- |
| `/api/threads?limit=1000` | 532–1570ms | 1.6MB |
| `/api/threads?limit=200` | 88–137ms | 318KB |
| `/api/threads?limit=100` | 77ms | 151KB |
| every other state slice | <2ms each | <90KB total |

- Per-row server cost is structural: `list_threads` runs
  `attach_thread_runtime_summary` per row → `thread_store.get()` (deep
  clone + mtime stat on cache hit, disk JSON read on miss) **plus**
  `current_agent_runtime_metadata`, which clones the full custom-agent
  catalog (`list_agents().await`) — per row,
  per request.

## Why NOT a plain `limit=200` truncation

A consumer audit of `state.threads`/`state.sessions` found truncation is
not transparent; the full set is load-bearing in three ways:

1. **Main process hard-throws.** `requireThread` (`main/store.ts:900`)
   throws `"Thread not found."` for ids absent from `state.threads`.
   Callers: `setDesktopThreadPinned` (pin an open-but-old thread),
   `resolveThreadWorkspace` (terminal/browser/workspace features on an old
   thread), `recordOutgoingThreadPrompt` (sending a message into an old
   thread). All three would break for any thread outside the window.
2. **Renderer repair is transient.** `ensureThreadOpenable` merges a
   synthesized summary into `sessions`
   (`transcript-lifecycle.ts:688-712`), but every
   `refreshDesktopState()` replaces `desktopState` wholesale with
   `sessions == threads` from IPC, clobbering the repair on the next
   background refresh.
3. **Full-set enumerations.** Per-workspace thread groups
   (`thread-model.ts:353-409`), the worktree-exclusion set for the visible
   workspace list (`thread-model.ts:280-316` — truncation would leak
   managed worktree dirs into the workspace list), the bot-conversation
   visibility gate (`AppShell.tsx:1818-1842`), the automation
   target picker (`AppShell.tsx:5246`), the pinned rail
   (`threadSummaryById`, `AppShell.tsx:1936-1956`), and the
   startup/route-restore fallback `isKnownThreadId(...) ? id : threads[0]`
   (`route-effect-bridge.ts:236`).

Changing all of those to by-id/on-demand semantics is a product-behavior
change (workspace lists and automation pickers would stop showing old
threads) and a much larger surface. Not this slice.

## Change

### 2a — desktop: two-phase hydration (fast page first, full set follows)

Boot keeps the exact end-state semantics; only the critical path changes.

- `fetchThreads(settings, options?)` gains `{ limit }` (default 1000,
  unchanged for existing callers).
- `getDesktopState()` gains a fast variant used ONLY for the renderer's
  initial hydration: `getDesktopStateFast()` (IPC `garyx:get-state-fast`):
  - threads slice fetched with `limit=200`;
  - afterwards, `pinnedThreadIds` missing from the page (pins are few)
    are repaired by id via the single-thread GET (`/api/threads/{key}`)
    and appended to `threads` before `withSortedEntities`. Failures fall
    back silently (same as a thread deleted while the app was closed:
    the pin row stays hidden until the full set lands).
  - the selected thread needs NO by-id repair: an external
    `#/thread/<id>` route keeps an unknown id selected by design (batch
    4b policy — the selected-thread loader is the single error surface)
    and the transcript loader fetches by id regardless of the summary
    page; a thread-home boot selects `threads[0]`, which is always in
    the page.
  - all other slices unchanged (they are already <2ms).
- Renderer hydration (`useGatewayConnectionController` startup path)
  becomes: `getDesktopStateFast(...)` → `setDesktopState` (state-hydrated
  mark fires here) → immediately `void getState()` in the background →
  wholesale `setDesktopState` when the full set arrives (~1s later, off
  the interaction path). Every non-boot `refreshDesktopState()` keeps
  calling the full `getState()` — semantics identical to today.

Window analysis (between fast and full states, ~1s): route restore and
pinned rows are covered by the by-id repair; workspace thread groups, the
worktree exclusion set, bot conversation gates, and the automation picker
briefly see the 200-thread view and self-heal when the full set lands.
None of those surfaces is interactively reachable within the window
without deliberate speed-running, and all are pure derivations with no
persisted side effects. The main-process hard-throw paths are not
reachable in the window for threads that matter: the only selected thread
is repaired by id.

`hasRemoteDesktopContent`/`shouldRetryStartupHydration` treat the fast
state like any hydrated state (threads.length > 0), which is correct.

### 2b — gateway: hoist catalog lookups out of the per-row loop

In `list_threads` (and the shared summary path used by
`list_recent_threads`), resolve the custom-agent catalog once
per request:

- add `build_thread_runtime_summary_with_catalog(state, thread_value,
  agents: &[...])`;
- keep `build_thread_runtime_summary` as a thin wrapper resolving the
  catalogs itself (single-thread callers unchanged);
- the list loops call the `_with_catalog` variant.

This cuts N catalog clones per request for every `/api/threads` client
(desktop full fetch, mobile) independent of limit. The per-row
`thread_store.get()` stays (runtime summary needs thread metadata that is
not in the `thread_meta` projection); moving those fields into the
projection is a possible future slice if numbers still warrant it.

### Non-goals

- No real truncation of the steady-state `threads` set (product
  semantics stay).
- No new list endpoint, no projection-schema change.
- No `/api/recent-threads` (mobile) changes.
- No `sessions` de-aliasing (`sessions` is already the same array
  reference in main-process memory; Electron structured clone preserves
  duplicate references, so the alias is near-free on the wire).

## Validation

- Unit (desktop): fast-state merge — missing selected/pinned ids are
  repaired by id and present in `threads`; full refresh replaces state
  wholesale.
- `cargo test -p garyx-gateway` (existing list route tests must stay
  green; summary output byte-identical vs. main for a catalog-bearing
  thread).
- Packaged app before/after boot marks: `state-hydrated` target ≈150–250ms
  (from ~717ms); restore-selected-old-thread on boot must keep the
  selection (no threads[0] kick); pin/unpin + terminal open + send message
  on an old thread after boot.
- `npm run test:unit`, smoke.

## Measured after implementation

Packaged app, steady-state cold start (V8 code cache warm), live gateway
with 2672 threads:

| boot mark | before | after |
| --- | --- | --- |
| renderer-entry | 111ms | 102ms |
| shell-mounted | 164ms | 164ms |
| state-hydrated | 717ms | 425ms |
| first-interactive-frame | **737ms** | **431ms (−41%)** |

Gateway after 2b (catalog hoist): `/api/threads?limit=1000` steady state
532–771ms → 380–470ms; `limit=200` (fast path) 71–86ms. Remaining
full-fetch cost is the per-row `thread_store.get()` deep clone + mtime
stat (future slice candidate: projection-backed runtime summary).

Walkthrough on the packaged app: pinned thread at recency rank ~800
(outside the fast page) present in the fast state (`fastThreadCount=202`,
repaired by id) and rendered in the pinned rail after boot; follow-up full
state landed with 1000 threads; workspace list and bot groups intact.
