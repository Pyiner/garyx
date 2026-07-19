# iOS Fluid P0-A A6 Acceptance Record

Date: 2026-07-19

A6 closes the v41 P0-A delivery with a repository-wide deletion audit. The
only production changes remove code that cannot execute under the iOS 26
deployment policy; the surviving branch is the same branch that already ran
on supported devices. No route, gesture, animation, layout, or data contract
was changed.

## Environment

- Xcode 26.6 (17F113), Apple Swift 6.3.3
- iPhone 17 Pro simulator, iOS 26.5 SDK/runtime
- Deployment target iOS 26.0 in XcodeGen, app, tests, and widget targets
- Audit base: `origin/main@f356e1324`

## Three-batch repository audit

The scan started at the repository root, used `--hidden`, excluded only Git's
object database, and therefore covered app/Core sources, tests, comments,
documentation, generated project files, shared schemes, plists, and other
tracked hidden configuration.

Identifier checks are word-anchored. Unrelated desktop APIs whose longer names
describe appending composer files are not the retired iOS storage symbol and
remain valid consumers in their own subsystem.

| §8a family | Scope | Matches |
|---|---|---:|
| A4b identifiers | Five retired attachment/draft/runtime families | 0 |
| A4b write sites | Retired composer lifecycle draft writes | 0 |
| A4c | Content path owner, leading-edge semantics, private panel back path, sidebar route write | 0 |
| A5 global | Drawer/task-tree local gesture and drag-state families | 0 |
| A5 row | Fixed-spring row settle path | 0 |

The commands reconstruct retired names at shell evaluation time so this
historical record does not itself reintroduce them:

```sh
cd <repository-root>

a4b_patterns=(
  'composer''Attachments'
  'clearAllComposer''Drafts'
  'gatewayRuntime''Generation'
  'GaryxComposerDraft''Store'
  'setComposer''Draft'
)
a4c_patterns=(
  'GaryxRootNavigationPath''Store'
  'rootPath''Binding'
  'applyRootNavigation''Path'
  'GaryxMobileLeadingEdge''Action'
  'mainPanelLeadingEdge''Action'
  'mainPanelBack''Stack'
  'goBackInMain''Panel'
  'performMainPanelLeadingEdge''Action'
)
a5_patterns=(
  'openingSidebar''Gesture'
  'closingSidebar''Gesture'
  'decideSidebar''Axis'
  'canStartOpening''Drag'
  'sidebarDrag''Offset'
  'sidebarDrag''Axis'
  'sidebarDrag''Live'
  'taskTreeDrag''Offset'
  'taskTreeDrag''Axis'
  'taskTreeDrag''Live'
  'resetSidebar''Drag'
  'reset''Drag'
  'predictedEnd''Translation'
  'GaryxMobileMotion\.row''Swipe'
)

for identifier in "${a4b_patterns[@]}" "${a4c_patterns[@]}" "${a5_patterns[@]}"; do
  ! rg --hidden -ni -g '!.git/**' "\b${identifier}\b" .
done

! rg --hidden -n -g '!.git/**' \
  'Navigation''Stack\(path:|\.(popTo''Home|mainPanel''Back|settings''Overview|workspaceBots''Overview)\b' .
! rg --hidden -n -g '!.git/**' \
  '\.on(Change|Disappear).*composer|\.onChange\(of: draftText\)' .
! rg --hidden -n -g '!.git/**' \
  '\.onDisappear[[:space:]]*\{[^}]*workspaceBotsDrilldown' .
! rg -n \
  'withAnimation\([^)]*row''Swipe|interactive''Spring\(' \
  mobile/garyx-mobile/App/GaryxMobile/GaryxMobileListComponents.swift
```

Every assertion exits zero with no match.

## Dead-code and fixture audit

Only zero-consumer compatibility code was deleted:

| Item | Base | A6 | Proof |
|---|---:|---:|---|
| Version-gate annotations/branches below or equal to the deployment floor | 11 | 0 | Root scan of app, Core, tests, UI tests, project, schemes, and plists |
| Legacy Glass material argument references | 38 | 0 | All references were confined to the unreachable compatibility API |
| Glass fallback renderer | 1 | 0 | No iOS 26 call path; symbol scan is empty |
| Unused adaptive-Glass overloads | 2 | 0 | Call-shape scan found definitions only before deletion |
| Unused adaptive-Glass style case | 1 | 0 | Definition-only before deletion |
| Retired row/sidebar motion entries | 0 | 0 | A5 had already removed them; A6 full-root rescan stayed empty |

The migrated helper files were also scanned for `private func` declarations
whose symbol occurred only at the declaration. The candidate list was empty.
The remaining `sidebarDrilldown` motion token has exactly one definition and
one production consumer, so it is retained.

The same exploration included fixtures and their project/config consumers:

| Fixture | Confirmed consumers | Decision |
|---|---|---|
| Durable-delivery debug fixture | App launch switch, generated project source phase, durable UI interaction suite | Retain |
| Fluid fake-route fixture | App launch switch, app-host configuration tests, 18 route interaction tests | Retain |
| Agent-avatar parity JSON | Two SwiftPM suites and the desktop parity test | Retain |

No orphan fixture was found.

## Build hygiene

The `origin/main` Debug build and final clean Debug build were normalized to
repository-relative source warning lines and sorted before comparison:

| Warning measure | Base | A6 | Delta |
|---|---:|---:|---:|
| Compiler warning lines (two simulator architectures) | 20 | 20 | 0 |
| Unique source warning sites | 10 | 10 | 0 |
| New warning identities | 0 | 0 | 0 |

The normalized files are byte-identical. A fresh `xcodegen generate` followed
by a tracked project/scheme diff also exits zero: the generated pbxproj has no
drift.

## Performance and behavior non-regression

The retained route-performance attachment reports:

```text
performance=pass; settleFrames=40; maxGapMs=18.06; backwards=0; bodyDelta=0; peakMounted=2
```

This improves on the A5 maximum frame gap of 18.16 ms. The home-list gate also
passed with an explicit probe hitch-time ratio of 0.03563, maximum frame
interval of 60.91 ms, and worst frame delta of 44.25 ms. Its four measured
iterations reported 0.903 s average app CPU time (3.10% RSD) and 7.323 s
average monotonic time (0.15% RSD), within the suite's 10% variability gate.

The iOS 26 cleanup selects the exact API branch previously selected at runtime:
native Glass, scroll-edge effects, scroll anchors/phases, UIKit gesture
representables, display-link frame-rate range, and the hitch metric. No
fallback value was substituted.

## Full validation

| Gate | Result |
|---|---|
| `swift test` | 1,406 passed, 0 failed; includes 264-second real kill/relaunch harness |
| `GaryxMobile` app-host tests | 142 passed, 0 failed |
| `GaryxMobileFluidRoutes` complete UI scheme | 53 passed, 0 failed in 801 seconds |
| Generic simulator Debug clean build | PASS |
| Generic simulator Release build | PASS |
| XcodeGen project regeneration | PASS, zero pbxproj/scheme drift |
| Diff whitespace check | PASS |

The design's implementation anchor, the mother plan's “current state” audit,
and older Capsule, Recent, sidebar, task-tree, and home-FAB implementation
documents now carry explicit historical notes. Section 8b of the authoritative
design links every retained per-slice acceptance record and this A6 closeout.
