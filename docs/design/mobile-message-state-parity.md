# Mobile Message-State Parity: Diagnosis & Alignment Design

Task: #TASK-1449. Status: **implemented** — designs ① ② PASSed (codex #TASK-1450)
and ③ revised + PASSed (codex #TASK-1451); all three fixes landed on branch
`garyx/90e323ef`. See "Implementation status" at the end.

## Framing (oracle-first, both ends)

The Mac desktop app is **not** treated as the source of truth here; it can be
wrong too. The baseline for every symptom is the **objectively-correct
semantics of the state**, anchored to the real source of truth — the
**server** (`render_state` / per-thread SSE run state / server-derived run
truth). Clients (desktop *and* iOS) are dumb renderers that must faithfully
reflect server truth. For each symptom we (1) state the correct semantics
(oracle) independently of any client, (2) check desktop against it, (3) check
iOS against it, (4) give a both-ends alignment plan. A conclusion may be "both
ends change" or "only one end is wrong".

This does **not** relax the `render_state` dumb-render red line
(`docs/agents/repository-contracts.md` → Transcript Rendering): no fix may add
local recomputation of transcript rows, tool grouping, tail thinking,
active-tool state, or final-answer placement on the client. Every fix below
reads an existing server-derived signal; none derives transcript structure
locally.

## Server-derived truth reference (the oracle foundation)

Established by reading `garyx-gateway` / `garyx-router` / `garyx-models`
(verified against current code):

- **"Does a thread have an active run right now?"** has two server surfaces with
  a deliberate precision difference — both are valid, they answer slightly
  different questions:
  - **Recent-threads projection** (`run_state` ∈ {`idle`,`running`,`completed`}
    + `active_run_id`, `recent_thread_run_state`,
    `recent_thread_projection.rs:534-548`): the **orphan-vetoed** truth.
    `run_state == running` requires `transcript.busy == true` AND `active_run_id`
    present AND the bridge confirms the run still executes in memory
    (`resolve_active_run_id`, `recent_thread_projection.rs:57-68`, gated by
    `BridgeActiveRunProbe`), so a crash-orphaned run reads `idle`. Snapshot,
    pulled.
  - **Per-thread SSE `thread_render_frame`**: live, **committed-ledger/render
    derived** (not itself bridge-probe-gated). `render_state` (`RenderSnapshot`,
    `garyx-models/src/transcript_render_state.rs`) carries `tailActivity` ∈
    {`none`,`thinking`,`assistant_streaming`,`tool_active`}, `activeToolGroupId`,
    `based_on_seq`; the committed control ledger in the same frame carries
    `run_start` / `run_complete` / `run_interrupted`, from which the open/closed
    run boolean is reduced. Pushed live to any subscribed client — no polling.
  - This run truth is background context for ② (foreground resync must converge
    the open thread's committed messages **and** run state). It is deliberately
    **not** what drives the top spinner: per the boss, the spinner is a loading
    indicator, not a run indicator (see symptom 3). The orphan-veto vs
    committed-ledger distinction above only matters to the recent-list `run_state`
    projection, not to the loading chrome.
- **Thread "kind"** is an objective field. `thread_type` is present in **every**
  thread-summary-returning endpoint (recent-threads, list, get-thread, history
  summary) and **defaults to `"chat"`** for a normal thread — it is never
  empty/null for a legitimate thread (`thread_summary_type_from_record`,
  `garyx-gateway/src/thread_type.rs:4-6`). `workflow_run_id` is present only in
  the full get-thread object, not in the list projections.

iOS already consumes both truths today:

- Run truth: `runStateByThread[id].busy` is reduced from committed control
  records by `GaryxTranscriptRunStateReducer`
  (`Sources/GaryxMobileCore/GaryxTranscriptRunState.swift:154+`,
  `run_start`→busy, `run_complete`/`run_interrupted`→idle) and applied by
  `applyTranscriptRunState` from the SSE render-frame flush
  (`App/GaryxMobile/GaryxMobileModel+ThreadStream.swift:316`). Exposed as
  `isThreadBusy(_:)` (`GaryxMobileModel+Presentation.swift:317-320`) and
  `showsTailThinkingIndicator` (`:304-307`, `tailActivity == .thinking`).
- Kind truth: `GaryxThreadSummary.threadType` / `workflowRunId`, classified
  purely by `GaryxWorkflowRunDestination.destination(...)`
  (`Sources/GaryxMobileCore/GaryxWorkflowRunPanelState.swift:8-30`).

The bugs below are **not** missing server truth: ① binds the surface kind to
mutable session state instead of the objective `threadType`; ② gates the
foreground resync on a stale cached connection state; ③ uses a loading-complete
predicate that is stricter than the render predicate, so the loading chrome
never settles. Each fix binds to an existing objective signal (or aligns a
predicate with the renderer) — none invents new server truth or recomputes
transcript structure locally.

---

## Symptom 1 — Widget-opened normal thread shows a "Workflow run" header

### Correct semantics (oracle)

The conversation top-bar "kind / header identity" must be a **pure function of
the thread's own objective type** (`thread_type == "workflow_run"` ⇒ workflow
surface; anything else ⇒ normal chat). It must be **independent of**:

- the entry point (widget link / recent-list row / deep link / task), and
- the previously-viewed thread (no residual carry-over).

A thread whose type is not yet known (opened by id before its summary is
loaded) is **not** a workflow run; the correct default while resolving is a
neutral chat/loading surface, never a workflow surface. The server guarantees
this is decidable: `thread_type` is always present and defaults to `"chat"`.

### Desktop: does it deviate? **No.**

Desktop decides the surface as a pure function of the thread record:

```ts
// AppShell.tsx:3226-3229
const activeWorkflowRunThreadId =
  contentView === "thread" && activeThread?.threadType === "workflow_run"
    ? activeThread.id : null;
```

`activeThread` is looked up fresh from in-memory state on each render; there is
no "resolving" intermediate surface, no entry-path branch, and the header title
is always `activeThread?.title` for both kinds. A residual
`selectedWorkflowRunId` from a prior view does not feed the thread-view
decision. Desktop is correct per the oracle.

### iOS: does it deviate? **Yes — root cause.**

iOS derives the surface kind from **mutable session state**, not the thread's
type:

```swift
// GaryxMobileModel+WorkflowRuns.swift:7-14
var isWorkflowRunSurfaceActive: Bool {
    switch workflowRunPanelState.mode { case .idle: false; case .resolving, .run: true }
}
// GaryxMobileSidebarViews.swift:231-235 / 245-248
if model.isWorkflowRunSurfaceActive { GaryxWorkflowRunView() } else { GaryxConversationView() }
```

Two concrete deviations:

1. **Entry-path pollution.** Opening a thread *by id* with no in-memory summary
   (the widget cold-start path: `garyx://mobile/thread?threadId=…` →
   `openMobileRouteFromLink` → `openThread(id:)`) routes through
   `showResolvingWorkflowThread` (`GaryxMobileModel+AgentsWorkspaces.swift:206`,
   also `:162`, and `queuePendingThreadLink:173`), which calls
   `workflowRunPanelState.beginResolving(…)` → `mode = .resolving` →
   `isWorkflowRunSurfaceActive == true` **before the thread type is known**
   (`GaryxWorkflowRunPanelState.swift:80-90`). The conversation then renders
   `GaryxWorkflowRunView`, whose panel title falls back to **"Workflow Run"**
   (`GaryxMobileWorkflowRunViews.swift:51-55`) rendered at the top by
   `GaryxPanelScaffold`→`GaryxPanelHeaderTitle`
   (`GaryxMobileComponents.swift:324,357`). For a normal chat thread the surface
   is wrong for the entire resolve window (connect + `refreshThreads` on cold
   start — seconds, and indefinitely if resolution is superseded or the gateway
   is slow/unreachable). The **recent-list row tap takes a different path** —
   `openThreadImmediately(_ thread:)`
   (`GaryxMobileModel+AgentsWorkspaces.swift:129-153`, call site
   `GaryxMobileViews.swift:51`) already has the summary, computes
   `.chat`, and goes straight to `showSelectedThread` — so it **never** shows
   the workflow surface. Same thread, two entry points, two surfaces ⇒ oracle
   violation. (This is exactly symptom 4: re-entering from the list = the
   by-summary path = correct.)

2. **Residual-state pollution.** `popToHome()`
   (`GaryxMobileModel+Navigation.swift:56-65`) cancels workflow polling but does
   **not** `clearWorkflowRunSurface()` and does not reset `draftThreadTitle`
   (set to `"Workflow run"` at `GaryxMobileModel+WorkflowRuns.swift:72`) or
   `selectedWorkflowRunThread`. So `workflowRunPanelState.mode` and the leaked
   title survive navigation. Most subsequent opens happen to clear it
   (`showSelectedThread` clears when active,
   `GaryxMobileModel+Threads.swift:755-757`), so it self-heals on the common
   path, but the surface kind is still a function of "what you looked at before"
   — a latent oracle violation and a defense-in-depth gap.

The underlying classifier `GaryxWorkflowRunDestination.destination` is already
correct (pure over `threadType`). The bug is entirely that the App layer
renders the **surface** from `workflowRunPanelState.mode` (an entry-path- and
history-dependent mutable state) and maps "resolving an unknown thread" to the
**workflow** surface instead of a neutral chat-loading surface.

### Alignment plan

- Introduce a Core pure decider
  `GaryxConversationSurfaceKind.resolve(summary:isResolvingById:) -> .chat | .workflowRun | .loadingUnknown`
  keyed only on the resolved `threadType`/`workflowRunId`. `.loadingUnknown`
  (by-id, summary not yet loaded) renders the **chat** surface with a neutral
  loading state — never the workflow panel. The view branch reads this decider's
  output, not `workflowRunPanelState.mode`.
- Keep `workflowRunPanelState` strictly for *actual* workflow-run drilldown
  content (the `.run` data), not as the surface-kind switch. Stop using
  `.resolving` as a workflow-presenting mode; a thread being fetched by id is a
  chat-loading state.
- Reset the workflow surface on every navigation that leaves a thread
  (`popToHome` must `clearWorkflowRunSurface()` and clear the leaked
  `draftThreadTitle`), so kind never carries over.
- Net effect: a normal thread shows the normal header on **every** open path,
  matching the (already-correct) desktop semantics and the oracle. No
  `render_state` recomputation involved.

### Deterministic reproduction

Core, no UI. Because the buggy decision currently lives in `@MainActor`
`GaryxMobileModel` methods (App target, untestable), the reproduction is (a) a
**precise state sequence** over those methods, plus (b) a spec test against the
proposed Core decider (red until implemented), plus (c) a compiling
characterization of the entry-path divergence at the existing classifier level.

State sequence (cold start from widget, normal thread `T` with
`thread_type=="chat"`):

```
onOpenURL(garyx://mobile/thread?threadId=T)
  → openMobileRouteFromLink(.thread(T))
  → queuePendingThreadLink(T) → showResolvingWorkflowThread(T)
      ⇒ workflowRunPanelState.mode == .resolving
      ⇒ isWorkflowRunSurfaceActive == true            // ASSERT (bug): surface = workflow
      ⇒ selectedThread == nil, GaryxWorkflowRunView shown, panel title "Workflow Run"
  → (connecting…) connectAndRefresh → openPendingThreadLinkIfNeeded → openThread(id:T)
  → destination(for: T) == .chat → clearWorkflowRunSurface + selectThread
      ⇒ isWorkflowRunSurfaceActive == false           // only now correct
```

Contrast (recent-list tap of the same `T`): `openThreadImmediately(T)` →
`destination(for: T) == .chat` → `showSelectedThread` ⇒
`isWorkflowRunSurfaceActive == false` throughout. Same thread, surface differs
by entry path ⇒ reproduces the oracle violation deterministically.

The red acceptance test for `GaryxConversationSurfaceKind` (the gate for this
fix) is listed under `Acceptance tests`.

A checked-in characterization test (`…ReproTests`, compiles today, green) pins
the present divergence at the classifier level and the `.resolving`/workflow
conflation; see the test file referenced in "Reproduction artifacts".

---

## Symptom 2 — Background→foreground does not refresh the current thread

### Correct semantics (oracle)

When the app returns to the foreground (or the window becomes visible again),
the currently-open thread must **converge to the server's latest committed
state** — both transcript messages and run state — so the result is identical
to having stayed in the foreground the whole time. Concretely: re-establish the
live signal (per-thread SSE) and backfill anything missed while suspended.

### Desktop: does it deviate? **Partially, by a different mechanism.**

Desktop keeps the per-thread SSE alive in the **main process** across renderer
window-hide (it is torn down on thread *deselection*, not on window-hide), and
recovers dropped connections via main-process backoff + renderer gap detection.
So while hidden it generally stays synced without an explicit foreground
refresh. On `visibilitychange` it calls `refreshSilently()` →
`refreshDesktopState()` (`AppShell.tsx:5128-5134, 5109-5122`) which refreshes
the **global** thread list but does **not** explicitly re-fetch the active
thread's history or restart its stream. This is mostly fine *given* the
persistent main-process connection, but it is a latent gap: if the connection
silently dies without firing the gap/backoff path, return-to-visibility will
not force a current-thread resync. Desktop's model leans on "the connection
never stops"; it should still defensively resync the active thread on visibility
regain.

### iOS: does it deviate? **Yes — root cause.**

iOS cannot keep a persistent connection: the OS suspends the app and iOS
explicitly tears the stream down on background
(`handleScenePhase(.background)` → `stopSelectedThreadStream()`,
`GaryxMobileModel+Gateway.swift:310-319`). So iOS **must** resync on foreground.
It tries, but the resync is **gated on a cached `connectionState == .ready`**
and does nothing otherwise:

```swift
// GaryxMobileModel+Gateway.swift:279-308
case .active:
    sceneRefreshTask = Task {
        switch connectionState {
        case .ready:
            … refreshThreads(); if same selected thread { loadSelectedThreadHistory();
            startSelectedThreadStream(for: selectedThreadId) } …
        case .checking, .disconnected, .failed:
            break            // ⇐ no reconnect, no resync, nothing
        }
    }
```

`startSelectedThreadStream` is itself `.ready`-gated
(`GaryxMobileModel+ThreadStream.swift:45`). Consequences:

- If the connection dropped or changed during background (network switch,
  gateway restart, long suspend that killed the socket) such that
  `connectionState` is `.disconnected`/`.failed`, foreground does **nothing** —
  no reconnect attempt — and the open thread stays frozen at its pre-background
  state until the user manually re-opens it. Re-opening works because
  `selectThread`→`loadSelectedThreadHistory`+stream restart runs through a
  different path that is reached on user action (symptom 4).
- Even in the `.ready` branch, the resync depends on the cached `.ready` being
  truthful; a stale `.ready` whose first `refreshThreads` fails does not
  escalate to a reconnect, so the thread can still end up stale.

So iOS has the right *intent* (resync on foreground, which is the only correct
model for a suspended app) but the wrong *gate*: it should ensure connectivity
(reconnect when not ready) and then unconditionally resync the open thread +
restart its stream, rather than no-op'ing whenever the cached state isn't
`.ready`.

### Alignment plan

- Extract the foreground decision into a Core pure planner, e.g.
  `GaryxForegroundSyncPlan.plan(connectionState:selectedThreadId:) -> { reconnect, resyncOpenThread, restartStream }`.
  Oracle: a selected thread always yields `resyncOpenThread` + `restartStream`,
  and a non-ready connection always yields `reconnect` first. The App layer
  drives effects from the plan.
- iOS `handleScenePhase(.active)`: when not `.ready`, attempt
  `connectAndRefresh()` (which already opens pending routes and refreshes), then
  resync the open thread. When `.ready`, keep the existing refresh but make the
  open-thread resync + stream restart unconditional on "a thread is selected"
  rather than implicitly relying on the cached `.ready`.
- Desktop: add a defensive current-thread resync on `visibilitychange` /
  reconnect so it does not rely solely on the connection never stopping.
- All of this reuses existing sync primitives (`loadSelectedThreadHistory`, the
  resumable per-thread SSE, the committed reconcile loop). No new transcript
  derivation; the red line is untouched.

### Deterministic reproduction

The decision lives in `handleScenePhase` (App target). Reproduction = precise
state sequence + spec test against the proposed planner.

State sequence:

```
state: connectionState = .ready, selectedThread = T, stream running
scenePhase → .background   ⇒ stopSelectedThreadStream()   (stream stopped)
[background: socket dies; connectionState becomes .disconnected via a failed bg probe
 OR remains a stale .ready]
scenePhase → .active
  current behavior:
    if .disconnected/.failed → break       ⇒ NO reconnect, NO resync   (BUG)
    if stale .ready          → refreshThreads may fail, no reconnect escalation (BUG)
  oracle:
    plan(.disconnected, T) == { reconnect:true, resyncOpenThread:true, restartStream:true }
```

The red acceptance test for `GaryxForegroundSyncPlan` (the gate for this fix)
is listed under `Acceptance tests`. Because `handleScenePhase` is App-target,
the in-target reproduction is the state sequence above; the planner test is the
acceptance gate.

---

## Symptom 3 — Top loading spinner does not settle (stuck spinning)

> **Revised per boss (2026-06-29).** The earlier draft set the oracle to "spinner
> = active-run truth" and proposed a `GaryxConversationHeaderActivity` decider to
> rebind the spinner to run state. **That direction is void and removed.** The
> boss clarified: *"转圈就是用来表示加载的,不是表示进行中,我也不需要它表示进行中。
> 只要加载好了能正常取消转圈就行。"* The spinner IS a loading indicator and that
> role is correct; it must carry **no** running/run-in-progress semantics. The
> real bug is that the loading indicator does not **settle** after loading
> completes.

### Correct semantics (oracle)

The top spinner is a **loading indicator** (initial history / render
resolution). It must faithfully reflect "is the current thread still loading its
initial content" and **deterministically settle to `false`** once that load
completes (or fails / is cancelled) — on every path, without needing a re-enter.
It must **not** reflect whether a run is in progress. The existing pairing
`isSelectedThreadLoadingInitialHistory = isLoadingSelectedThreadHistory ||
isAwaitingInitialHistory` (`GaryxMobileModel+Presentation.swift:271-273`) is the
right loading signal in principle — the requirement is that it can settle.

### Desktop: does it deviate? **No equivalent stuck-spinner.**

Desktop shows history-loading as a placeholder in the message body, not a header
spinner (`AppShell.tsx` header block has no loading spinner). Its load lifecycle
is driven by the persistent stream + `historyLoading` flag, with no
render-snapshot-resolution gate that can wedge. No stuck-loading equivalent
observed. (Re-verify during implementation review; this symptom is iOS-specific.)

### iOS: does it deviate? **Yes — root cause: loading-complete predicate is stricter than the render predicate.**

The mapper renders **every** snapshot row, substituting a placeholder for an
unresolved ref:

```swift
// GaryxMobileRenderState.swift:797 (and :847 for assistant steps)
let userBlock = user.map { lookup.mobileMessage(for: $0) ?? .userStepPlaceholder(for: $0) } …
```

So the transcript is never blank once a snapshot has rows. But the
loading-complete predicate `isAwaitingInitialHistory`
(`GaryxMobileRenderState.swift:667-712`) returns `true` whenever **any** snapshot
row ref is unresolved (`hasUnresolvedVisibleRefs`, `:699-712`), where "resolved"
means the ref's `id`/`historyIndex` is among `cachedMessages`
(`GaryxMobileModel+Presentation.swift:254-261`). Render-time placeholders are
**not** in `cachedMessages`. So when a snapshot references an out-of-window /
not-yet-materialized message (e.g. a row below the client's `render_floor`, or a
control/internal row), the transcript renders fully (placeholder) while
`isAwaitingInitialHistory` stays `true` — the spinner spins **over a
fully-rendered transcript** and never settles. Re-entering the thread
(`selectThread` → `resetSelectedThreadHistoryPagination` + reload) resets it,
which is exactly the reported "必须重进列表才好". In short: the loading-complete
predicate is **stricter than the render predicate**, so loading is "done" on
screen but "still awaiting" in state.

Two coupled secondary paths keep the indicator stuck:

- The in-flight flag `isLoadingSelectedThreadHistory` is flipped true at the
  start of every `loadSelectedThreadHistory()` (`Threads.swift:1117`) and only
  settled in its `defer` when the fetch returns. On `scenePhase .active` the
  reload runs through ② — if ②'s resync never completes (connection not ready /
  no reconnect), the fetch (and the flag) does not settle. **②'s fix lets the
  fetch run and settle.**
- The no-snapshot + `historyLoaded` + committed-messages branch (`:690-696` →
  `true`) waits for a render frame; if the per-thread SSE is not connected (②),
  the frame never arrives and it stays awaiting. **②'s reconnect lets the frame
  arrive and settle.**

So after ② is fixed, the remaining iOS-only stuck path is the
`hasUnresolvedVisibleRefs` over-strictness above.

### Alignment plan (revised — loading lifecycle settle, NOT run truth)

- Settle `isAwaitingInitialHistory` on the **window-applied fact**, aligning it
  with the mapper's render tolerance: once `historyLoaded == true` (the committed
  window was applied via `markThreadHistoryLoaded`), a present render snapshot
  means the initial history has loaded — remaining unresolved refs are
  out-of-window / live-delta rows the mapper already placeholders, so the
  predicate must return `false`. Keep the **pre-`historyLoaded`** path
  (`hasUnresolvedVisibleRefs`) for the genuine "first window not yet resolved /
  blank screen" case. This is a tightly-scoped change to the existing Core pure
  function (no new decider, no run-truth binding, no `GaryxConversationHeaderActivity`).
- The existing test `testLiveRenderSnapshotWithUnresolvedRefsStillAwaitsInitialHistory`
  (`GaryxSelectedThreadHistoryPresentationTests.swift:68-74`) currently asserts
  this stuck state as *correct* (historyLoaded=true + unresolved ref ⇒ awaiting
  `true`). Its asserted behavior **is** the bug; the fix flips it to `false` and
  renames it. The neighboring tests
  (`…ResolvedByMobileMessages…`, `…CachedRenderSnapshotStops…`) already expect
  `false` and stay green.
- Keep `isLoadingSelectedThreadHistory` as the in-flight loading signal; it
  settles via its `defer`. ②'s foreground resync guarantees the fetch actually
  runs and completes so the flag and the awaited frame both settle.
- The spinner keeps its current binding to
  `isSelectedThreadLoadingInitialHistory` — **no** rebinding to run truth.
- Red line intact: this only changes *when* the loading chrome hides; it does
  not recompute transcript rows — it aligns the hide-condition WITH the mapper's
  existing row output.

### Deterministic reproduction (checked-in Core characterization test)

`GaryxMobileMessageStateParityReproTests.testLoadingIndicatorStaysStuckWhileTranscriptIsAlreadyRendered`
(green today) drives the real Core asymmetry: it feeds a windowed snapshot whose
single user-turn row references an unresolved seq to **both**
`GaryxMobileRenderStateMapper.rows` (⇒ ≥1 placeholder row, transcript rendered)
and `GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(historyLoaded:
true, …)` (⇒ `true`, still awaiting). Rendered-but-stuck = the bug. The
acceptance gate (red until the fix) is the settle spec under `Acceptance tests`.

---

## Symptom 4 — "Re-enter the thread to fix it" (common cause, not separate)

Re-entering via the recent-list row always runs the by-summary `openThread` /
`showSelectedThread` path, which (a) classifies kind from the summary (fixes
symptom 1), (b) runs `loadSelectedThreadHistory` + restarts the stream (fixes
symptom 2), and (c) resets pagination and reloads, re-resolving the snapshot
refs so `isAwaitingInitialHistory` settles (fixes symptom 3's stuck spinner). So
"re-enter fixes it" confirms the diagnosis: **initial open is correct; the
incremental / lifecycle update chain is what's broken** — entry-path-dependent
surface kind (1), `.ready`-gated foreground resync (2), and a loading-complete
predicate stricter than the renderer so the spinner never settles (3). The fix
is to make those three states settle / derive correctly on the steady-state
path, not only on a fresh re-open.

---

## Cross-cutting: move the decisions into Core so they are testable

The ① and ② root causes live in `@MainActor GaryxMobileModel` App-target methods
(`isWorkflowRunSurfaceActive` / `showResolvingWorkflowThread`,
`handleScenePhase`), which is why they escaped unit coverage. Per
`docs/agents/mobile-ui.md` (route state, presentation mapping, business rules
belong in `GaryxMobileCore` with SwiftPM tests), the alignment introduces **two**
small pure Core deciders — `GaryxConversationSurfaceKind` (① kind from objective
`threadType`) and `GaryxForegroundSyncPlan` (② foreground reconnect/resync) —
each covered by SwiftPM tests; the App target shrinks to compute inputs → call
decider → drive effects. ③ is **not** a new decider: its logic is already the
Core pure function `GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory`,
and the fix is a tightly-scoped settle change to it (covered by updating that
function's existing SwiftPM tests). This keeps every state decision testable in
Core.

## Reproduction artifacts

Two kinds, kept distinct:

**A. Checked-in green characterizations (reproduction, not acceptance).**
`mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxMobileMessageStateParityReproTests.swift`
compiles against current Core and is green today. Each test pins the *current*
behavior deterministically with real Core data and models any App-target formula
locally (citing its source line). These **do not flip on the fix** — they model
the old formulas locally, so they document the bug but do not gate it.

  - Symptom 1: pins (i) the entry-path classification divergence
    (`destination(threadId:summary:nil) == .unresolved` vs
    `destination(for: chatSummary) == .chat`) and (ii) the
    `beginResolving`→non-idle-mode conflation that the App reads (via
    `state.mode != .idle`) as a workflow surface.
  - Symptom 3: feeds one windowed snapshot with an unresolved row ref to **both**
    `GaryxMobileRenderStateMapper.rows` (⇒ ≥1 placeholder row — transcript
    rendered) and `GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(historyLoaded:
    true,…)` (⇒ `true` — still awaiting), pinning the rendered-but-stuck
    asymmetry.

**B. Red acceptance tests (the gate).** See `Acceptance tests` below. ① and ②
are red specs for the two new deciders (`GaryxConversationSurfaceKind`,
`GaryxForegroundSyncPlan`) — not committed in this phase (they would not compile
without the new types). ③ is a red spec against the **existing**
`GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory` (it compiles
today and fails today — the fix flips it green) plus the existing test
`testLiveRenderSnapshotWithUnresolvedRefsStillAwaitsInitialHistory` flipping from
`true` to `false`.

## Acceptance tests (red specs)

① and ② fail today because the new deciders do not exist yet; ③ fails today
against the existing Core function. The fix implements ①②'s deciders, rewires
the App layer, and applies ③'s settle change — turning all of these green. They
are the acceptance gate (distinct from the green reproduction characterizations
above).

```swift
// Symptom 1 — GaryxConversationSurfaceKind: kind is a pure function of type,
// entry-path- and history-independent; unknown defaults to chat-loading.
XCTAssertEqual(GaryxConversationSurfaceKind.resolve(summary: nil, isResolvingById: true), .loadingUnknown)
XCTAssertEqual(GaryxConversationSurfaceKind.resolve(summary: chatSummary("T"), isResolvingById: false), .chat)
XCTAssertEqual(GaryxConversationSurfaceKind.resolve(summary: chatSummary("T"), isResolvingById: true), .chat)
XCTAssertEqual(GaryxConversationSurfaceKind.resolve(summary: workflowSummary("W"), isResolvingById: false), .workflowRun)
// none of these is the workflow surface except the real workflow_run thread:
XCTAssertNotEqual(GaryxConversationSurfaceKind.resolve(summary: nil, isResolvingById: true), .workflowRun)

// Symptom 2 — GaryxForegroundSyncPlan: a selected thread always resyncs +
// restarts its stream; a non-ready connection reconnects first.
XCTAssertEqual(GaryxForegroundSyncPlan.plan(connectionState: .disconnected, selectedThreadId: "T"),
               .init(reconnect: true, resyncOpenThread: true, restartStream: true))
XCTAssertEqual(GaryxForegroundSyncPlan.plan(connectionState: .ready, selectedThreadId: "T"),
               .init(reconnect: false, resyncOpenThread: true, restartStream: true))
XCTAssertEqual(GaryxForegroundSyncPlan.plan(connectionState: .ready, selectedThreadId: nil),
               .init(reconnect: false, resyncOpenThread: false, restartStream: false))

// Symptom 3 — isAwaitingInitialHistory settles once the window is applied; an
// out-of-window / unresolved snapshot ref must NOT keep loading on (the mapper
// placeholders it). NO run-truth semantics. (Tested against the existing Core
// function — compiles today; the first assertion FAILS today, the fix flips it.)
let snap = snapshotWithUnresolvedRef() // a windowed snapshot whose row ref is not in cache
XCTAssertFalse(GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
    threadId: "T", historyLoaded: true, liveRenderSnapshot: snap, cachedTranscript: nil))   // settles after window applied
XCTAssertTrue(GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
    threadId: "T", historyLoaded: false, liveRenderSnapshot: snap, cachedTranscript: nil))  // pre-window: still resolving
// Existing test flips: testLiveRenderSnapshotWithUnresolvedRefsStillAwaitsInitialHistory
// (Tests/GaryxMobileCoreTests/GaryxSelectedThreadHistoryPresentationTests.swift:68-74)
// asserted TRUE (the bug) → becomes FALSE (renamed) after the fix.
```

Implementation acceptance also includes: the view branch reads
`GaryxConversationSurfaceKind` (not `workflowRunPanelState.mode`); `popToHome`
clears the workflow surface; `handleScenePhase(.active)` drives effects from
`GaryxForegroundSyncPlan`; and the top spinner keeps its binding to
`isSelectedThreadLoadingInitialHistory` (now settling) — it is **not** rebound to
any run-truth signal.

## Non-goals / constraints honored

- This document is the design reference; implementation proceeds per it (① ②
  on the PASSed design, ③ after this revision's re-review).
- No `render_state` dumb-render violation: ① binds to objective `threadType`,
  ② re-establishes the resumable SSE + reload, ③ aligns the loading-complete
  predicate WITH the mapper's existing placeholder output (it only changes
  *when* the loading chrome hides). None recomputes transcript rows, grouping,
  tail, or active-tool state locally.
- No new top-level mobile concept; kinds/states stay the server's.
- The top spinner stays a **loading** indicator (③) — no run/progress semantics,
  per the boss's clarification.
- Conclusions are asymmetric where the evidence is: symptom 1 = iOS-only fix
  (desktop correct); symptom 2 = iOS primary + desktop defensive; symptom 3 =
  iOS loading-settle only (no run-truth rebind; desktop has no equivalent
  stuck-spinner).

## Implementation status (#TASK-1449)

Implemented on branch `garyx/90e323ef` (design ① ② PASSed by codex #TASK-1450;
③ revision PASSed by codex #TASK-1451). Each Core decider is pure + unit-tested;
the App/renderer wiring computes inputs → calls the decider → drives effects.

| Symptom | Commit | Change | Validation |
| --- | --- | --- | --- |
| ① surface kind | `iOS: derive conversation surface kind…` | `GaryxConversationSurfaceKind` (Core) + `showsWorkflowRunSurface` view branch + `popToHome` clears the surface | Core tests + `xcodebuild` |
| ② foreground (iOS) | `iOS: reconnect + resync the open thread…` | `GaryxForegroundSyncPlan` (Core) + `handleScenePhase(.active)` reconnects when not ready, then resync + restart stream | Core tests + `xcodebuild` |
| ② foreground (desktop) | `desktop: defensively resync…` | visibility-regain schedules a canonical history refresh for the open thread | `tsc --noEmit` + `test:unit` |
| ③ loading settle | `iOS: settle the loading indicator…` | `isAwaitingInitialHistory` settles once `historyLoaded` (window applied); out-of-window refs are placeholdered, not loading. No run-truth binding. | Core tests + `xcodebuild` |

Validation snapshot: `swift test` 554 / 0 failures (incl.
`GaryxConversationSurfaceKindTests`, `GaryxForegroundSyncPlanTests`, the flipped
`GaryxSelectedThreadHistoryPresentationTests`, and the updated
`GaryxMobileMessageStateParityReproTests`); `xcodebuild -scheme GaryxMobile`
BUILD SUCCEEDED; desktop `tsc --noEmit` clean + `test:unit` 258 / 0.

The earlier "Acceptance tests" red specs for ① and ② are now the committed
green decider tests; ③'s settle is the flipped predicate test. No gateway /
router / bridge changes (server truth unchanged).
