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
- History `push` semantics for in-app view switches. Today internal
  switches create no history entries (the state-to-hash effect always
  replaces); this cutover preserves that exactly — **every navigation in
  this design is replace**. Making rail/view switches pushable is a
  separate product decision with its own back-stack design (draft-consumed
  `#/new`, feature-gate fallbacks, and automation-run redirects would all
  need per-site treatment), out of scope here.

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

**thread-home redirect.** Today `#/` never rests when threads exist: the
thread-home branch selects `threads[0]` and the state-to-hash effect
rewrites the hash to `#/thread/<id>`. The end state keeps that as an
explicit redirect: applying `{kind:'thread-home'}` with threads resolves
the default thread and immediately `navigate({kind:'thread', threadId},
{replace:true})`. **With no threads it rests at thread-home** (`#/thread`,
selection null) — matching today's startup fold (`threads[0] || null` and
`buildDesktopRouteHash({kind:'thread-home'}) === '#/thread'`), not the
new-thread draft. Back semantics of the redirect: replace consumes the
`#/` entry, so back returns to whatever preceded `#/`, never to `#/`
itself — identical to today's state-to-hash normalization.

**Route effects.** `applyDesktopRoute` stops being external-only and
becomes the route-change effect for every commit (internal and external).
It owns: view-scoped resets (leaving capsules clears the preview), data
side effects (automation select IPC, settings tab resources, workflow-task
fetch), and the thread-open path. Internal callers stop pre-setting state;
they navigate and let the effect apply.

**Commit event contract.** The route effect must know each commit's
origin (an external failure must keep the 4b no-counter-write behavior; an
internal failure converges). Today `subscribe()` carries nothing and
`subscribeExternal()` fires after the plain listeners, so a full-commit
subscriber cannot classify reliably. 2a therefore adds one store API:

```
subscribeCommits(listener: (event: {
  route: DesktopRoute;      // canonical, as committed
  version: number;          // store version of this commit
  origin: 'navigate' | 'external' | 'sync';
}) => void): Unsubscribe
```

`commit()` emits it synchronously with the origin the caller passed
(`navigate` for the internal writer, `external` for hashchange/popstate
application, `sync` for `syncRoute` below). The bridge's route effect
moves onto `subscribeCommits`; `subscribeExternal` is absorbed by it
(delete after the move). The plain `subscribe()` stays as the uSES
notification face.

**`sync` commits are never applied** (implementation finding, round 4):
the state-to-hash pass commits a route the state *already reflects* — a
fold-driven commit (e.g. picking an agent in the draft changes
`pendingAgentId`, folding a new `new-thread` route). Re-applying it would
re-run entry side effects against live state (the new-thread branch's
`clearComposerDraft` would wipe the draft being typed). The state-to-hash
pass therefore uses `syncRoute(route)` — identical to
`navigate({replace:true})` except the commit carries origin `sync`, which
the route effect ignores. This also settles the convergence commit: the
post-failure fold-and-replace is a `sync` commit, so it cannot re-trigger
an application (no second `openExistingThread`, no loop).

**Route application transaction.** Equal-route no-ops alone do NOT break
the feedback loop while the state-to-hash effect still exists: an
application is multi-step (`setContentView` lands before the async
`ensureThreadOpenable` resolves the selection), and the fold over that
intermediate state folds back a *different* route (thread A→B folds
`#/thread/A` mid-flight; a failed automation select folds `#/automation`).
The 4b suppression generalizes — but NOT as a bare boolean, which an
overlapping application's earlier `finally` would clear while a later one
is still in flight (automation A awaiting, user switches to settings B,
A's finally un-suppresses while B still awaits its tab). The transaction
is version-keyed with a pending counter:

- Each application runs with its commit `version` as its token and
  increments a pending counter on entry, decrements on settle.
- The state-to-hash effect is suppressed while the counter is non-zero.
- Settle convergence (the one fold-and-replace pass) runs only when the
  settling application's token still equals
  `routeStore.getSnapshot().version` — a superseded application decrements
  the counter but never triggers convergence, and never lands late state
  (the same version guard, applied to finalization as well as to state
  writes).
- Convergence itself distinguishes origin: for an `external` commit whose
  application failed, it does not fold back (4b no-counter-write — the
  entered hash stays addressable); for a `navigate` commit it converges
  the hash to where the state actually ended (one replace, not an
  oscillation) — byte-identical to today's terminal hash for the same
  failure, because today's call sites pre-set the same partial state and
  fold from it.

**Async guard (uniform criterion).** Every route application and every
controller side effect that writes route-folded state captures
`routeStore.getSnapshot().version` before its first await and re-checks it
before landing state: `version` unchanged ⇒ still the current route; moved
⇒ drop the landing (the newer application owns the state). This replaces
per-site ad-hoc guards: the workflow-task fetch's `cancelled` flag upgrades
to it, and `handleSelectAutomation` — today unguarded, so a slow select A
resolving after select B can clobber `desktopState` and fold the hash back
to A — gains it in 2a. The 6c-1 request-sequence guard for thread opens is
the same shape against the thread-selection sequence and stays.

## Migration steps (each lands + reviews separately)

### 6c-2a — single write path (state unchanged)

Every A/B call site becomes `desktopRouteStore.navigate(route, {replace:
true})` — replace everywhere, preserving today's zero-history-entry
behavior byte-for-byte (see non-goals). The bridge subscribes to **all**
commits via the new `subscribeCommits` event (origin-carrying — this step
adds it and deletes `subscribeExternal`) and runs `applyDesktopRoute`
inside the version-keyed route application transaction (suppressing
state-to-hash while any application is pending);
the state-to-hash effect stays as the post-application convergence
backstop for the ten states it still watches. `contentView` remains
useState but its only writer is `applyDesktopRoute`.

Compound async transitions translate as "do the work, then navigate the
result": thread creation navigates `{kind:'thread', threadId: created.id}`
after the create resolves; `openExistingThread` keeps its imperative body
(the route effect calls it for thread routes; direct callers are rewired to
navigate). `handleSelectAutomation` gains the version guard (it is
unguarded today — a pre-existing race this step must not widen).

**Draft entry is a command (review #TASK-1621, round 5):** entering the
new-thread draft must run its side effects (composer clear, pending
resets, bot rebinding) even when the target route equals the current one
— navigate's equal-route dedupe would swallow them. Draft openers
therefore call a single shared `enterNewThreadDraft` command directly
(the bridge's new-thread application delegates to it for route-only
entries), and the hash syncs from the state fold as before. `navigate`
remains the entry for addressable targets (thread/view/automation/
settings/capsule/workflow-task), where equal-route no-op is the correct
semantics. Bot drafts pass the binding as a command argument (the
mailbox died with this); agent/workflow picks are kept by omission so
async fallbacks cannot write stale closure values back.

**2a scope note (implementation round):** the five `setContentView: () =>
...` seams feed thread-controller compound helpers that keep setting
companion state right after the seam fires — closing a navigate over the
seam would race the route application against the helper's remaining
writes. Those seams and the startup seeding branch therefore stay direct
writers through 2a/2b (an explicitly listed transitional state) and are
dissolved by the 2c selectedThreadId/new-thread flips, whose migration
table already owns them. Deep links keep their 6c-1 shape (the handler IS
the application) and write the hash via `syncRoute`.

Per-kind convergence table (application intermediate/failure states vs the
settled fold — each must terminate in one settled hash equal to today's):

| Scenario | Mid-flight fold (suppressed) | Settled state | Settled hash |
| --- | --- | --- | --- |
| thread A→B, open succeeds | `#/thread/A` | selected B | `#/thread/B` (= committed, no-op) |
| thread A→B, B missing | `#/thread/A` | selection unchanged + error | external commit: stays `#/thread/B` (4b addressable); internal navigate: converges back to `#/thread/A` — today's terminal too |
| automation select, IPC fails | `#/automation` | view automation, id unset | `#/automation` (one replace; today identical) |
| settings tab, resource load fails | `#/settings/<prev>` | tab state landed anyway (load is display-only) | `#/settings/<tab>` (no-op) |

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

**selectedThreadId write-site migration table** (all 24 sites; the flip is
mechanical only once every writer is a navigation):

| Sites | Today | After |
| --- | --- | --- |
| `selectExistingThreadInPlace` (AppShell:2428) | the selection write | becomes the route-effect landing itself (selector reads the committed thread route) |
| startup seeding (2630/2638/2643/2649) | manual per-branch set | absorbed by the store's initial-route commit + thread-home application |
| draft entry clears (3080; bridge new-thread 194; thread-controller 59 via its draft-open seam) | `setSelectedThreadId(null)` | implied by committing the new-thread route (selector yields null) |
| thread-home default (bridge 240) | conditional default set | the thread-home redirect (`navigate({kind:'thread', threadId}, replace)`) |
| created / started threads (2920, 3593; thread-controller 95/414 via seams 3127/3406) | direct set after create | `navigate({kind:'thread', threadId: created}, replace)` after the create resolves |
| delete / archive / workspace-remove fallbacks (2697/2703/3251/3347; thread-controller 225; automation 302) | set fallback `threads[0] || null` | `navigate(fallback ? {kind:'thread', threadId: fallback} : {kind:'thread-home'}, replace)` |
| automation run → thread (automation 371/401) | set + `setContentView('thread')` | `navigate({kind:'thread', threadId: latest}, replace)` |

**Synchronous readability (draft promotion).** `ensureThread` today sets
`selectedThreadId` synchronously before returning the created id, and
same-tick consumers read `selectedThreadIdRef`. Selector-writes stay
synchronous: `navigate` commits the canonical route into the store before
returning (4a), so the selector value is immediately consistent — but the
`selectedThreadIdRef` shadow must stop being fed by a post-commit React
effect and instead be written by a store subscription (synchronous notify)
in the same flip, or same-tick readers (dispatch orchestrator deps,
transcript controller, scroll requests) would observe the previous thread
for one frame. This ref re-feed lands in the same commit as the
`selectedThreadId` flip, with a unit test pinning navigate → ref
visibility in the same tick.

`settingsActiveTab` keeps one non-route nicety: entering `#/settings`
(null tab) shows the last active tab. The selector reads
`route.tabId ?? lastSettingsTabRef.current ?? 'labs'`; selecting a tab
navigates `{kind:'settings', tabId}` (replace) — hash and UI stay 1:1
after the first click, and plain `#/settings` stays addressable.

## Invariants (each verified per step)

1. External hash edits and back/forward behave exactly as 4b shipped
   (commit-first, no counter-write, unknown thread stays addressable).
2. `#/` redirects to the default thread when threads exist; with no
   threads it rests at thread-home (`#/thread`, selection null).
3. Equal-route navigation is a strict no-op (no loops between the route
   effect and navigate).
4. Deep-link semantics from 6c-1 (readiness ladder, supersede guard).
5. A/B sites produce byte-identical hashes to today for the same actions
   (state-to-hash previously normalized them; navigate now writes the same
   canonical form directly).
6. No navigation in this cutover creates a history entry (replace
   everywhere) — back/forward depth is unchanged for every flow.
7. Failure convergence is single-step: a failed application settles with
   at most one hash replace to today's terminal hash, never an
   oscillation (per-kind table in 2a).

## Validation

- `desktop-route` unit tests extend with selector round-trips
  (route → selectors → currentDesktopRoute → same canonical route) while
  the fold still exists (2a/2b), pinning equivalence before the fold dies.
- CDP route matrix per step: for every route kind — internal navigation,
  external hash edit, history back/forward — assert hash, view, and (for
  thread) transcript render; plus the 6c-1 deep-link walkthrough on the
  packaged app after 2c.
- Race and promotion cases get dedicated coverage: slow-A/fast-B automation
  select (version guard drops the late landing), draft promotion
  (same-tick `selectedThreadIdRef` visibility after navigate),
  delete-fallback navigation, and a mid-application hash sample proving
  the state-to-hash suppression (no `#/thread/A` flicker during A→B).
- The electron-smoke baseline stays green after each step.
