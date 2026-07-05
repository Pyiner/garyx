# AppShell Route-Selector Cutover (Endgame Batch 6c-2)

Parent design: `appshell-endgame-architecture.md` ("Route as URL single
source of truth", destination-map row for useDeepLinkRouteController).
Batches 4a/4b delivered the `DesktopRouteStore` (only hash writer, external
edits commit-first, no counter-writes); 6c-1 shaped the bridge
(`route-effect-bridge.ts`). What remains is the ownership flip this
document designs: today the route is **derived from** ten pieces of React
state (`currentDesktopRoute`), and view changes write that state directly
(49 `setContentView` sites). The end state inverts it: `route` is the only
navigation state, `contentView` and the routed selection ids are selectors,
and `navigate(route)` is the only way to change page.

## Non-goals

- `mirror.openThread` (thread-open orchestration into the mirror) — a
  batch-6 transcript cut, not route work.
- Colocating layout/settings/composer/side-chat state — separate 5b cuts.
- Any hash grammar change: `desktop-route.ts` parse/build stays as is.

## Current state (verbatim inventory)

Route derivation: `currentDesktopRoute` folds ten states — `contentView`,
`newThreadDraftActive`, `pendingAgentId`, `pendingWorkflowId`,
`pendingWorkspacePath`, `selectedAutomationId`, `selectedWorkflowTaskId`,
`selectedThreadId`, `settingsActiveTab`, `capsulePreviewId` — and a
state-to-hash effect in the bridge navigates (replace) on every change.
External hash edits go the other way through `subscribeExternal` →
`applyDesktopRoute`.

`setContentView` write sites (49) fall into three classes:

- **A. Pure view switches (14)** — rail buttons (`agents`, `skills`,
  `capsules` + clear preview, `tasks`, `dreams`), `onBackToThreads`,
  `onOpenThreads`, `onOpenTasks`, `onOpenRecent` (plus rail-open local
  state), the dreams feature-gate fallback, `openSettingsView` (plus tab
  resource loading side effect).
- **B. Compound transitions (19)** — `openExistingThread`,
  `openWorkflowTask`, thread-created success, new-thread draft entry,
  workflow start, `onOpenCapsule`, automation select / dialog-save /
  run-to-thread (x2), the startup route seeding branch, and five
  `setContentView: () => setContentView("thread")` seams passed to shared
  helpers.
- **C. Route application (7 + thread-home)** — `applyDesktopRoute` in the
  bridge, today reached only by external hash edits and deep links.

## End-state model

```
DesktopRouteStore.route            — the ONLY navigation state
contentView                        = contentViewForDesktopRoute(route)
selectedThreadId                   = route.kind==='thread' ? route.threadId : null
selectedAutomationId               = route.kind==='automation' ? route.automationId : null
selectedWorkflowTaskId             = route.kind==='workflow-task' ? route.taskId : null
capsulePreviewId                   = route.kind==='capsule' ? route.capsuleId : null
settingsActiveTab                  = route.kind==='settings' ? route.tabId ?? 'labs' : (sticky last)
new-thread pendings                = route.kind==='new-thread' ? route params : reset
```

Stays React state (not route): `selectedWorkflowRunId` and the loaded
`selectedWorkflowTask` object (fetched projections of the routed task id),
`threadEntrySelectionSource`, `recentThreadsRailOpen`,
`botConversationGroupId`, `workspaceConversationPath`,
`newThreadDraftActive`'s companion composer state, and every dialog/panel
local state. They are UI or data caches keyed by the route, not route.

**thread-home redirect.** Today `#/` never rests: the thread-home branch
selects `threads[0]` and the state-to-hash effect rewrites the hash to
`#/thread/<id>`. The end state keeps that as an explicit redirect: applying
`{kind:'thread-home'}` resolves the default thread and immediately
`navigate({kind:'thread', threadId}, {replace:true})`; with no threads it
rests on the new-thread draft route (today's behavior via
`initialContentView`). `#/` is transitional by design, unchanged.

**Route effects.** `applyDesktopRoute` stops being external-only and
becomes the route-change effect for every commit (internal and external).
It owns: view-scoped resets (leaving capsules clears the preview), data
side effects (automation select IPC, settings tab resources, workflow-task
fetch — all already idempotent or keyed), and the thread-open path.
Internal callers stop pre-setting state; they navigate and let the effect
apply. Equal-route navigations stay no-ops (4a canonical equality), which
is what breaks feedback cycles.

## Migration steps (each lands + reviews separately)

### 6c-2a — single write path (state unchanged)

Every A/B call site becomes `desktopRouteStore.navigate(route)` (push for
user-initiated view changes — a real behavior change, today's internal
switches don't create history entries — except in-place normalizations,
which stay replace). The bridge subscribes to **all** commits and runs
`applyDesktopRoute`; the state-to-hash effect stays as the convergence
backstop for the ten states it still watches. `contentView` remains
useState but its only writer is `applyDesktopRoute`.

Compound async transitions translate as "do the work, then navigate the
result": thread creation navigates `{kind:'thread', threadId: created.id}`
after the create resolves; `openExistingThread` keeps its imperative body
(the route effect calls it for thread routes; direct callers are rewired to
navigate). The five `setContentView: () => ...` helper seams become
`navigate`-closing seams unchanged in shape.

Risks handled here: double-application (call site no longer pre-sets, so
the effect is the single applier); async effects racing user navigation
(the 6c-1 request-sequence pattern generalizes: a route effect checks its
route is still current before landing late state).

### 6c-2b — contentView becomes a selector

Delete the `contentView` useState; `contentView =
contentViewForDesktopRoute(useRouteSnapshot().route)` (uSES on the store —
AppShell subscribes directly, not via context, per the Provider-renderer
rule). `initialContentView` folds into the store's initial route.
`applyDesktopRoute` loses its `setContentView` calls; what remains of each
branch is data side effects only.

### 6c-2c — routed ids become selectors; the fold dies

Flip the remaining routed ids one per commit (`capsulePreviewId` →
`settingsActiveTab` → `selectedAutomationId` → `selectedWorkflowTaskId` →
new-thread pendings → `selectedThreadId` last, carrying the thread-home
redirect). Each flip deletes the corresponding input from
`currentDesktopRoute`; after the last one, `currentDesktopRoute` and the
state-to-hash effect are deleted — the route store no longer has anything
to converge from. `isKnownThreadId` fallback selection (startup unknown
threads) already lives in route-application land since 4b.

`settingsActiveTab` keeps one non-route nicety: entering `#/settings`
(null tab) shows the last active tab. The selector reads
`route.tabId ?? lastSettingsTabRef.current ?? 'labs'`; selecting a tab
navigates `{kind:'settings', tabId}` (replace) — hash and UI stay 1:1
after the first click, and plain `#/settings` stays addressable.

## Invariants (each verified per step)

1. External hash edits and back/forward behave exactly as 4b shipped
   (commit-first, no counter-write, unknown thread stays addressable).
2. `#/` redirects to the default thread; empty state lands on the draft.
3. Equal-route navigation is a strict no-op (no loops between the route
   effect and navigate).
4. Deep-link semantics from 6c-1 (readiness ladder, supersede guard).
5. A/B sites produce byte-identical hashes to today for the same actions
   (state-to-hash previously normalized them; navigate now writes the same
   canonical form directly).
6. In-place data mutations that today re-derive the same route (rename,
   refresh) must not create history entries: only user-initiated view
   changes push.

## Validation

- `desktop-route` unit tests extend with selector round-trips
  (route → selectors → currentDesktopRoute → same canonical route) while
  the fold still exists (2a/2b), pinning equivalence before the fold dies.
- CDP route matrix per step: for every route kind — internal navigation,
  external hash edit, history back/forward — assert hash, view, and (for
  thread) transcript render; plus the 6c-1 deep-link walkthrough on the
  packaged app after 2c.
- The electron-smoke baseline stays green after each step.
