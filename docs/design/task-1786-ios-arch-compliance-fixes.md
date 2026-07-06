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
  .unresolved  → false (caller resolves by id: resolving surface +
                 refreshThreads + getThread)

selectThread(thread, invalidates, source):                    // +ThreadLifecycle.swift:10
  reopening = isHomeVisible && selectedThread?.id == thread.id
  showSelectedThread(..., startsSelectedThreadStream: !reopening)
  await loadSelectedThreadHistory()
  if reopening { ensureSelectedThreadStreamForVisibleConversation() }
```

**Immediate path** (`openThreadImmediately(_ thread:source:)`,
`+AgentsWorkspaces.swift:129`), row taps only:

```
merge summaryWithCommittedRunState(thread) into threads
switch destination:
  .chat        → showSelectedThread(thread, invalidates: true, source)   // sync, stream policy always on
                 Task { await loadSelectedThreadHistory() }              // detached
  .workflowRun → Task { await openWorkflowRun(..., invalidates: true, source) }
  .unresolved  → openThreadImmediately(id:) → beginDirectThreadOpen()
                 + showResolvingWorkflowThread(...) (sync)
                 + Task { await openThread(id:requestId:source:) }
```

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
- `openThreadDestination` / `selectThread` / `showSelectedThread` are
  **untouched**.

After this, every open funnels through `openThreadDestination → selectThread →
showSelectedThread`; the only other `showSelectedThread` caller is
`showPendingThreadLink` (`+AgentsWorkspaces.swift:377`), the pending-link
resolving overlay inside the same shared flow.

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

### Semantic deltas, enumerated (row taps: immediate → shared)

| # | Aspect | Before (immediate) | After (shared) | Visible? |
|---|---|---|---|---|
| D1 | Main-actor hops to `showSelectedThread` (navigation push) | 0 — sync in the tap action | 1 — `Task` hop, then sync | No; sub-frame hop, push animation unchanged. `.workflowRun`/`.unresolved` already paid 1 hop before, and still do (hop moves from after to before the destination switch) |
| D2 | `.chat` history load | detached `Task` enqueued after show | awaited inline after show (same actor, same `loadSelectedThreadHistory` with per-request-id + selected-thread guards) | No; starts marginally earlier, same guards, same loading flags |
| D3 | Same-thread reopen from home (tap the selected thread's row on the home list) | `selectedThread` didSet always fires `.start` → stream (re)starts at show; history races in parallel | `startsSelectedThreadStream: false` + stream ensured **after** history load (`selectThread` reopen rule) | Not visually. Warm reopen (messages retained; P4 pins the selected thread), so no loading state either way; live stream attaches after one history roundtrip instead of concurrently — the established semantics every widget/deep-link/task reopen already has |
| D4 | `.workflowRun` | `Task { openWorkflowRun }` — body starts at hop 1 | `await openWorkflowRun` inside the tap task — starts at hop 1 | No; identical hop count and arguments |
| D5 | `.unresolved` | resolving surface shown at hop 0, resolution task at hop 1 | resolving surface + resolution at hop 1 (`openThread(id:)` finds the just-merged summary, destination still unresolved → `showResolvingWorkflowThread` in the same turn) | No; surface appears one hop later, same turn as all other UI work |
| D6 | Workflow-surface clearing | conditional (`isWorkflowRunSurfaceActive`) inside `showSelectedThread` | unconditional `clearWorkflowRunSurface()` in `openThreadDestination` first | No; `clearWorkflowRunSurface` is idempotent (cancel + clear + nil) |
| D7 | Rapid successive taps | last sync call wins; detached history loads race, request-id guarded | tasks interleave at suspension points; same request-id guards converge | No |

D3 is the one deliberate semantic convergence: it removes the row-tap-only
divergence (row taps restarted the stream even on a same-thread home reopen,
bypassing the reopen rule every other entry point gets). Content freshness is
unaffected — the awaited history fetch returns the newest window and the
stream resumes from the held committed cursor.

### Core sink (testable decision logic)

The reopen rule — the exact sub-case where the two old paths diverged — is
inline in `selectThread` today. Sink it as a pure predicate next to the
existing stream policies (no new file):

```swift
// Sources/GaryxMobileCore/GaryxSelectedThreadStreamPolicy.swift
public enum GaryxSelectedThreadReopenPolicy {
    /// True when opening `openingThreadId` re-opens the already-selected
    /// thread from the home list. Such an open must not tear down / restart
    /// the selected-thread stream at show time; the caller ensures the stream
    /// after the history refresh.
    public static func reopensSelectedThreadFromHome(
        isHomeVisible: Bool,
        selectedThreadId: String?,
        openingThreadId: String
    ) -> Bool {
        isHomeVisible && selectedThreadId == openingThreadId
    }
}
```

`selectThread` adopts it (raw equality preserved — no trimming added, byte-for
-byte the current predicate). Tests in the existing
`GaryxSelectedThreadStreamPolicyTests.swift`: nil selected ⇒ false; different
id ⇒ false; same id while conversation presented ⇒ false; same id + home ⇒
true. The destination switch, stream policies, and cold-open policy are
already Core-tested (`GaryxWorkflowRunDestination`,
`GaryxSelectedThreadStreamPolicy`, `GaryxVisibleConversationStreamPolicy`,
`GaryxColdOpenRestorePolicy`); the remaining diff is thin orchestration in the
App layer, per the mobile-ui layering rule. No new files ⇒ no xcodegen needed.

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

- Any change to `showSelectedThread` / cold-open restore / turn-rows window /
  residency (TASK-1751 surface) beyond routing row taps into the existing
  shared entry.
- The pre-existing shared-path behavior that selecting the same thread while
  the conversation is presented restarts the stream (`selectedThread` didSet →
  `.start`) — identical before/after for row taps.
- Sweeping non-page-background `ignoresSafeArea` uses (previewers, panels).
- Recent-threads widget (already compliant).

## Validation

1. Full `swift test` in `mobile/garyx-mobile` before and after (baseline
   865/0), no pipe-tail.
2. `xcodebuild -scheme GaryxMobile -sdk iphonesimulator build` (builds app +
   widget extension).
3. rg proofs of zero residue:
   - `rg openThreadImmediately mobile/` → empty;
   - `rg -n "widgetURL" mobile/garyx-mobile/Widget/` → only the
     family-gated `systemSmall` branch in `GaryxCodingUsageWidget.swift`;
   - `rg -n "GaryxTheme.background.ignoresSafeArea" mobile/…/App/` → only the
     `garyxPageBackground()` helper definition.
4. No new files (Core predicate + tests land in existing files) ⇒ no
   xcodegen/pbxproj churn; verify `git status` stays clean of project files.
