# TASK-1786 — iOS: three audited architecture-compliance fixes

Fixes three audit-confirmed compliance problems in `mobile/garyx-mobile`:
behavior-preserving path unification plus two small UI-compliance corrections.
Base: `origin/main` @ `664319b0` (includes #TASK-1751 — cold-open async
restore, turn-rows window, LRU residency; see
`docs/design/task-1751-ios-chat-pipeline-fixes.md`).

Audit line numbers drifted after #TASK-1751; all evidence below was relocated
and re-verified against the current tree.

## 1 — Open-thread dual path (P2)

### Relocated evidence

Row taps call `openThreadImmediately` (direct `showSelectedThread` + detached
`Task`), while deep links / routes / tasks / automations / bots go through
`openThread`:

- `App/GaryxMobile/GaryxMobileViews.swift:51` — home list row tap →
  `model.openThreadImmediately(thread, source: .replace)`.
- `App/GaryxMobile/GaryxMobileSidebarViews.swift:1114` — bot-conversation
  drilldown row → `model.openThreadImmediately(thread, source: .current)`.
- `App/GaryxMobile/GaryxMobileSidebarViews.swift:1338` —
  `GaryxSidebarThreadButton` default `onSelect` →
  `model.openThreadImmediately(thread, source: openSource)` (used by the
  automation-threads drilldown `:945` and workspace drilldown `:1257`, both
  with `openSource: .current`).
- `App/GaryxMobile/GaryxMobileModel+AgentsWorkspaces.swift:129` and `:155` —
  the two `openThreadImmediately` overloads (summary / id).

`docs/agents/mobile-ui.md` rule: *"Mobile entry points that open an existing
thread by row tap, widget link, task, automation, bot conversation, or deep
link should route through the shared `GaryxMobileModel.openThread` path;
home-list behavior is the baseline."* Row taps are the one entry family not on
the shared path.

### What each path does today (verified in source)

**Shared path** (`openThread(id:source:)` → `openThread(id:requestId:source:)`
→ `openThreadDestination` → `selectThread`), used by deep links
(`+Navigation.swift:206`), tasks (`+TaskTree.swift:134`), automations
(`+DreamsAutomations.swift:68/80/82`), bots (`+Bots.swift:9`,
`+ThreadLifecycle.swift:278`), workflow children
(`GaryxMobileWorkflowRunViews.swift:333`):

```
openThreadDestination(thread, requestId, invalidates, source):
  .chat        → clearWorkflowRunSurface(); await selectThread(...)
  .workflowRun → await openWorkflowRun(...)
  .unresolved  → false (caller resolves by id: refreshThreads + getThread;
                 showResolvingWorkflowThread is a no-op for direct opens — D5)

selectThread(thread, invalidates, source):                    // +ThreadLifecycle.swift:10
  reopening = isHomeVisible && selectedThread?.id == thread.id
  showSelectedThread(..., startsSelectedThreadStream: !reopening)
  await loadSelectedThreadHistory()
  if reopening { ensureSelectedThreadStreamForVisibleConversation() }
```

The `reopening` deferral (suppress the stream at show; connect only after the
history refresh) was introduced by the M3 gateway-stream-actor commit
(`3abfead5`, 2026-06-22) — the same commit that made `popToHome()` stop the
selected-thread stream — and **postdates** row taps' `openThreadImmediately`
(`55aee675`, 2026-06-20). It has therefore only ever applied to the id-based
entries, never to the home-list baseline.

**Immediate path** (`openThreadImmediately(_ thread:source:)`,
`+AgentsWorkspaces.swift:129`), row taps only:

```
merge summaryWithCommittedRunState(thread) into threads
switch destination:
  .chat        → showSelectedThread(thread, invalidates: true, source)   // sync, stream policy always on
                 Task { await loadSelectedThreadHistory() }              // detached
  .workflowRun → Task { await openWorkflowRun(..., invalidates: true, source) }
  .unresolved  → openThreadImmediately(id:) → beginDirectThreadOpen()
                 + showResolvingWorkflowThread(...)   // no-op for direct opens (D5)
                 + Task { await openThread(id:requestId:source:) }
```

Stream-start machinery fact used throughout: `startSelectedThreadStream`
(`+ThreadStream.swift:44-51`) early-returns when `streamOwnedThreadId ==
threadId && selectedThreadStreamTask != nil` — the `selectedThread` didSet's
`.start` action is *ensure* semantics (start only when dead), never a live
teardown. At home the stream is always dead (`popToHome` →
`stopSelectedThreadStreamForHome`, `+Navigation.swift:59`).

`openThread(_ thread: GaryxThreadSummary)` (summary variant,
`+AgentsWorkspaces.swift:91`) exists but has **zero callers** and silently
no-ops for `.unresolved` (`openResolvedThread` discards the
`openThreadDestination` result) — it is dead, incomplete code.

### Design: one funnel, `openThread` is the only entry

Rewrite the summary variant as the real entry and delete the immediate path:

```swift
/// Single summary-based open entry (mobile-ui.md: row taps, widget links,
/// tasks, automations, bot conversations, and deep links all route through
/// the shared openThread path).
func openThread(
    _ thread: GaryxThreadSummary,
    source: GaryxMobilePanelOpenSource = .replace
) async {
    let resolvedThread = summaryWithCommittedRunState(thread)
    threads = Self.mergedThreadSummaries(threads + [resolvedThread])
    if await openThreadDestination(
        resolvedThread,
        requestId: nil,
        invalidatesPendingThreadOpen: true,
        source: source
    ) {
        return
    }
    // Unknown/missing threadType (e.g. bot-conversation fallback summaries):
    // resolve by id through the shared resolving flow, like id-based opens.
    await openThread(id: resolvedThread.id, source: source)
}
```

- Delete `openThreadImmediately(_ thread:source:)`,
  `openThreadImmediately(id:source:)`, and `openResolvedThread` (folded in).
  No test references exist.
- Row-tap call sites become `Task { await model.openThread(thread, source:
  …) }` (the established sibling pattern: `onOpenBotGroup` at
  `GaryxMobileViews.swift:66`). Closure signatures stay synchronous.
- `openThreadDestination` is untouched.

**Converged `selectThread` semantics = the home-list baseline (design-review
v1, finding F1).** Routing row taps through the v1 `selectThread` would have
adopted the M3 reopen deferral for them: on a same-thread home reopen of a
*running* thread with a slow history response, the old row tap attaches the
live stream immediately at show, while the deferral delays live output by a
full history roundtrip — a visible regression, and backwards per the
mobile-ui rule (*home-list behavior is the baseline*). The deferral is
therefore **removed** rather than propagated:

```swift
func selectThread(_ thread:, invalidatesPendingThreadOpen: = true, source: = .replace) async {
    showSelectedThread(thread, invalidatesPendingThreadOpen:, source:)
    await loadSelectedThreadHistory()
    // Recovery net, not the primary start: no-op while the stream is owned
    // and alive; picks the stream up when the show-time start was skipped
    // (connection not yet ready at show).
    ensureSelectedThreadStreamForVisibleConversation()
}
```

- `showSelectedThread` loses the `startsSelectedThreadStream` parameter and
  its `suppressesSelectedThreadStreamPolicy` toggle; `openConversation`
  loses the parameter (no other non-default caller exists —
  `+WorkflowRuns.swift:64/:107` use the default); the
  `suppressesSelectedThreadStreamPolicy` stored flag and the `selectedThread`
  didSet guard (`GaryxMobileModel.swift:105`) are deleted outright. The
  deferral was the flag's only producer; killing the flag removes the switch
  future code could use to re-diverge.
- Safety: the didSet `.start` is ensure-semantics (idempotency guard above),
  so removing suppression can never tear down a live stream; at home the
  stream is always dead, so the same-id reopen becomes a plain fresh start —
  byte-identical to what the home row tap does today.
- Effect on id-based entries (widget link / deep link / task / automation /
  bot) for the **same-thread home reopen** sub-case only: the live stream now
  attaches at show time instead of after the history roundtrip (strictly
  earlier live output; content freshness unchanged — the bounded history
  refresh still runs and the stream resumes from the held committed cursor).
  Different-thread opens already started the stream at show in both paths
  (`reopening == false`), so they are unchanged.
- The trailing `ensureSelectedThreadStreamForVisibleConversation()` becomes
  unconditional (previously reopen-only): a pure recovery net that no-ops
  when the stream is alive (`GaryxVisibleConversationStreamPolicy.shouldStart`
  requires `owned != selected || !hasStreamTask`) and re-reads the *current*
  `selectedThread`, so a mid-load thread switch can never start a wrong-thread
  stream. Id-based entries keep their post-history recovery; row taps and
  different-thread opens gain it (invisible when healthy).

After this, every open funnels through `openThreadDestination → selectThread →
showSelectedThread` with **one** stream-start rule: ensure at show (didSet
`.start` + `openConversation`'s ensure), recover after history. The only other
`showSelectedThread` caller is `showPendingThreadLink`
(`+AgentsWorkspaces.swift:377`), the pending-link resolving overlay inside the
same shared flow.

### Interaction with #TASK-1751 (required analysis)

The cold-open async restore, cold-open generation, and turn-rows window reset
all live **inside `showSelectedThread`** (`+ThreadLifecycle.swift:48-102`):
on thread-id change it bumps `selectedThreadColdOpenGeneration`, calls
`resetSelectedTurnRowsWindow()`, and on an in-memory miss sets `messages = []`
(loading presentation) and `spawnColdOpenTranscriptRestore(threadId:)`. This
change does not modify that function, `spawnColdOpenTranscriptRestore`,
`applyColdOpenTranscriptRestore`, or `GaryxColdOpenRestorePolicy` — both old
and new row-tap flows reach the identical code.

The only 1751-relevant delta is **scheduling order of the two racers**:

- Today (immediate): the restore `Task` (enqueued by `showSelectedThread`) and
  the detached history `Task` are enqueued in that order; the restore's disk
  load starts first, then the history HTTP fires.
- After (shared): `selectThread` runs `loadSelectedThreadHistory()` inline
  after `showSelectedThread` returns, so the history HTTP fires within the
  same main-actor turn, *before* the enqueued restore task starts its disk
  load.

Both orderings are concurrent races with arbitrary completion order, which is
exactly what `GaryxColdOpenRestorePolicy` adjudicates (1751 design: *"the
network refresh below (loadSelectedThreadHistory) races and wins"*; the
six `shouldApply` conditions — thread id, cold-open generation, mirror
generation, `threadHistoryLoaded`, render snapshot, messages — abort a stale
restore regardless of which racer lands first, and a restore that lands first
is overwritten by the fetch through the existing
`setPreparedMessages`/`applyThreadRenderSnapshot` paths). No policy condition
observes enqueue order. Cold-open loading presentation
(`isAwaitingInitialHistory`) is produced by `showSelectedThread` itself,
unchanged.

The F1 deferral removal is 1751-orthogonal: a same-thread reopen keeps
`previousThreadId == thread.id`, so `showSelectedThread` skips the entire
id-change block — no cold-open generation bump, no turn-rows window reset, no
restore spawn (`+ThreadLifecycle.swift:48/:86`). The cold-open machinery only
runs for different-thread opens, whose stream-at-show + history + restore
triple race is precisely what every home row tap already executes today; live
stream mirror writes bump the 1751 transcript-mirror generation
(`applyStreamedCommittedMessages`/`applyThreadRenderSnapshot`), which is the
condition the restore policy uses to yield to them.

### Semantic deltas, enumerated (row taps: immediate → shared)

| # | Aspect | Before (immediate) | After (shared) | Visible? |
|---|---|---|---|---|
| D1 | Main-actor hops to `showSelectedThread` (navigation push) | 0 — sync in the tap action | 1 — `Task` hop, then sync | No; sub-frame hop, push animation unchanged. `.workflowRun`/`.unresolved` already paid 1 hop before, and still do (hop moves from after to before the destination switch) |
| D2 | `.chat` history load | detached `Task` enqueued after show | awaited inline after show (same actor, same `loadSelectedThreadHistory` with per-request-id + selected-thread guards) | No; starts marginally earlier, same guards, same loading flags |
| D3 | Same-thread reopen from home | Row tap: didSet `.start` → fresh stream at show (dead at home); history races in parallel | **Identical for row taps** — deferral removed (F1), stream starts at show for every entry; trailing ensure is a no-op | No for row taps (byte-identical sequence: didSet `.start` → `openConversation` ensure no-op → history). Id-based entries change in the *other* direction: live output attaches one history roundtrip **earlier** (see design above) |
| D4 | `.workflowRun` | `Task { openWorkflowRun }` — body starts at hop 1 | `await openWorkflowRun` inside the tap task — starts at hop 1 | No; identical hop count and arguments |
| D5 | `.unresolved` | `beginDirectThreadOpen()` + `showResolvingWorkflowThread` at hop 0, resolution task at hop 1 | same pair inside `openThread(id:)` at hop 1 | No. `showResolvingWorkflowThread` is a **no-op for direct opens in both paths**: `beginDirectOpen()` sets `pendingThreadId = nil` (`GaryxMobileNavigationState.swift:280-286`) and `markShown` requires `pendingThreadId == threadId` (`:300-307`), so the resolving surface only ever presents for URL-queued pending links. Both paths resolve silently via `refreshThreads`/`getThread` → `openThreadDestination` |
| D6 | Workflow-surface clearing | conditional (`isWorkflowRunSurfaceActive`) inside `showSelectedThread` | unconditional `clearWorkflowRunSurface()` in `openThreadDestination` first | No; `clearWorkflowRunSurface` is idempotent (cancel + clear + nil) |
| D7 | Rapid successive taps | last sync call wins; detached history loads race, request-id guarded | tasks interleave at suspension points; same request-id guards converge | No |

The one deliberate semantic change is confined to id-based same-thread home
reopens (D3, earlier live attach); row-tap visible behavior is conserved
exactly, per the hard constraint.

### Decision-logic inventory (Core sinking)

The F1 resolution *removes* a decision branch instead of adding one, so there
is no new decision logic to sink. Every rule the unified path decides with is
already a Core-tested pure policy:

- destination selection — `GaryxWorkflowRunDestination.destination`
  (`GaryxWorkflowRunPanelState.swift:3-31`);
- didSet stream action — `GaryxSelectedThreadStreamPolicy.action`;
- ensure gate — `GaryxVisibleConversationStreamPolicy.shouldStart`
  (`owned != selected || !hasStreamTask` — this is the exact predicate that
  makes the converged "ensure at show, recover after history" rule safe, and
  it already has SwiftPM tests);
- cold-open restore freshness — `GaryxColdOpenRestorePolicy`;
- pending-open request state — `GaryxMobileThreadOpenState`
  (direct-open/markShown semantics locked by
  `GaryxMobileThreadOpenStateTests.swift:68`).

The remaining diff is thin orchestration in the App layer, per the mobile-ui
layering rule. No new files ⇒ no xcodegen needed.

## 2 — Coding Usage widget: container-level `widgetURL` (P2)

### Relocated evidence

`Widget/GaryxRecentThreadsWidget/GaryxCodingUsageWidget.swift:127-134` — the
whole widget body ends with
`.widgetURL(GaryxMobileProviderSettingsLink.make())` for **all** families
(`systemSmall/systemMedium/systemLarge`) with an in-code comment defending the
container-level URL. The widget rule (mobile-ui.md) requires row/element taps
to be expressed as per-element `Link`s; a container `widgetURL` swallows
element-level tap semantics (it is exactly the pattern the recent-threads
widget rules forbid).

### Platform constraint

WidgetKit ignores `Link` controls in `systemSmall` — `widgetURL` is the *only*
supported tap mechanism there. In-repo precedent:
`GaryxRecentThreadsWidget.swift:130-131` (`supportsRowLinks == false` for
small, comment: *"systemSmall ignores per-row Links; a tap opens the app
instead"*). Removing `widgetURL` outright would downgrade the small family
from "deep link to provider settings" to "just open the app" — a click-target
regression the task forbids.

### Design

Express the card's single destination as an explicit per-element `Link` for
every family that supports Link semantics; keep the equivalent URL for
`systemSmall` only, as a documented platform exception:

```swift
var body: some View {
    linkedContent
        .containerBackground(for: .widget) {
            ContainerRelativeShape().fill(.thinMaterial)
        }
}

@ViewBuilder
private var linkedContent: some View {
    // One tap target, one destination (provider settings / Quota hero), for
    // content and empty state alike. Medium/large express it as an explicit
    // whole-card Link element (widget rule: no container-level widgetURL
    // where Link semantics exist). systemSmall is the platform exception:
    // WidgetKit ignores Link there, so the small family keeps the equivalent
    // family-scoped widgetURL — the only supported tap mechanism, with no
    // per-element links it could steal.
    if widgetFamily == .systemSmall {
        paddedContent.widgetURL(GaryxMobileProviderSettingsLink.make())
    } else if let url = GaryxMobileProviderSettingsLink.make() {
        Link(destination: url) { paddedContent }
            .buttonStyle(.plain)
    } else {
        paddedContent
    }
}

private var paddedContent: some View {
    Group { /* existing emptyState / gauges body, unchanged */ }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(metrics.contentPadding)
}
```

Behavior conservation:

- Tap target unchanged: the `Link` label is the full padded content
  (`frame(maxWidth:maxHeight:) + padding` **inside** the Link), so the
  tappable region stays the whole card, as `widgetURL` gave; `systemSmall`
  keeps `widgetURL` verbatim. Empty state stays linked in all families.
- Visuals unchanged: `.buttonStyle(.plain)` (the recent-threads row
  precedent, `GaryxRecentThreadsWidget.swift:97`) prevents Link tinting; all
  text/gauge styling is explicit `foregroundStyle`, and the modifier chain
  (`frame → padding → containerBackground`) keeps the same geometry with one
  transparent wrapper inserted.
- Destination unchanged: same `GaryxMobileProviderSettingsLink.make()` URL
  (`garyx://mobile/settings/provider`).
- `URL?` handling: `make()` is effectively infallible (static components) but
  optional-typed; the `else` branch renders bare content, matching the
  recent-threads pattern (`if let url … Link … else row`).

## 3 — Root view bypasses the shared page-background chrome (P2)

### Relocated evidence

- `App/GaryxMobile/GaryxMobileViews.swift:18` — `GaryxRootView`'s ZStack first
  child is a raw `GaryxTheme.background.ignoresSafeArea()`.
- `App/GaryxMobile/GaryxMobileDesignSystem.swift:300-302` — the shared helper
  being bypassed: `garyxPageBackground()` =
  `background(GaryxTheme.background.ignoresSafeArea(edges:
  GaryxSafeAreaChrome.pageBackgroundEdges))`, with `pageBackgroundEdges =
  .all` (`:35`). The helper itself is correct and unchanged; the violation is
  only the root view's raw pattern. This is the sole raw
  `GaryxTheme.background.ignoresSafeArea()` in the App target (all other
  `ignoresSafeArea` hits are non-page-background chrome: image previewers,
  full-bleed panels, sheets — out of audit scope).

### Design

```swift
var body: some View {
    ZStack {
        if …ready… { GaryxShellView(…) } else { GaryxGatewaySetupView() }
    }
    .garyxPageBackground()
    .overlay(alignment: .top) { … }   // unchanged
    …
}
```

Visual-equivalence argument (no full-screen special case needed):

- Paint region identical: raw `ignoresSafeArea()` ≡ `ignoresSafeArea(.all,
  edges: .all)` ≡ the helper's `ignoresSafeArea(edges: .all)` (same default
  `SafeAreaRegions.all`, keyboard included).
- Layout identical: both ZStack branches are self-expanding full-screen roots
  (`GaryxShellView` hosts the root `NavigationStack`;
  `GaryxGatewaySetupView` renders a `NavigationStack`), so the ZStack fills
  the proposal with or without the removed Color child; `background(…)` then
  paints the same full-bleed region the ZStack child painted.

## Out of scope

- Any change to the TASK-1751 surface (cold-open restore, turn-rows window,
  residency): `showSelectedThread` only loses the M3
  `startsSelectedThreadStream` plumbing; its id-change block, cold-open spawn,
  and warm-open floor lock are untouched.
- The stream-start idempotency/ensure semantics themselves
  (`startSelectedThreadStream` early-return, didSet `.start`,
  `GaryxVisibleConversationStreamPolicy`) — relied on, not modified.
- Sweeping non-page-background `ignoresSafeArea` uses (previewers, panels).
- Recent-threads widget (already compliant).

## Validation

1. Full `swift test` in `mobile/garyx-mobile` before and after (baseline
   865/0), no pipe-tail.
2. `xcodebuild -scheme GaryxMobile -sdk iphonesimulator build` (builds app +
   widget extension).
3. rg proofs of zero residue:
   - `rg openThreadImmediately mobile/` → empty;
   - `rg "startsSelectedThreadStream|suppressesSelectedThreadStreamPolicy|reopeningSelectedThreadFromHome" mobile/`
     → empty (deferral fully removed, no re-divergence switch left);
   - `rg -n "widgetURL" mobile/garyx-mobile/Widget/` → only the
     family-gated `systemSmall` branch in `GaryxCodingUsageWidget.swift`;
   - `rg -n "GaryxTheme.background.ignoresSafeArea" mobile/…/App/` → only the
     `garyxPageBackground()` helper definition.
4. No new files ⇒ no xcodegen/pbxproj churn; verify `git status` stays clean
   of project files.
