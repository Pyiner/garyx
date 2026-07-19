# iOS Fluid P2-E Dynamic Type Audit

Date: 2026-07-20
Baseline: `origin/main` at `aa9fd416b`

This is the completion inventory for P2-E. It applies the Dynamic Type and
typographic optical-sizing rules from
`garyx-product-ui/references/apple-design.md` §15 to the iOS 26 app and its
WidgetKit extension.

## Reading-role contract

`GaryxTypographyRole` and `GaryxTypographyScaleBoundary` live in
`GaryxMobileCore`, so the platform-independent role table and every permitted
cap are testable. `GaryxFont` is the SwiftUI/UIKit adapter. Reading roles use
system text styles (`Font.TextStyle` / `UIFont.preferredFont`) and have no
Dynamic Type ceiling.

| Role | System text style | Large baseline | Tracking intent | Leading intent | Scale policy |
| --- | --- | ---: | --- | --- | --- |
| `largeTitle` | `largeTitle` | 34 pt | tightened | tight | unbounded |
| `title` | `title` / `title1` | 28 pt | tightened | tight | unbounded |
| `title2` | `title2` | 22 pt | tightened | tight | unbounded |
| `title3` | `title3` | 20 pt | tightened | tight | unbounded |
| `headline` | `headline` | 17 pt | neutral | standard | unbounded |
| `body` | `body` | 17 pt | neutral | relaxed | unbounded |
| `callout` | `callout` | 16 pt | neutral | relaxed | unbounded |
| `subheadline` | `subheadline` | 15 pt | neutral | relaxed | unbounded |
| `footnote` | `footnote` | 13 pt | opened | relaxed | unbounded |
| `caption` | `caption` / `caption1` | 12 pt | opened | relaxed | unbounded |
| `caption2` | `caption2` | 11 pt | opened | relaxed | unbounded |

The adapter deliberately adds no global `.tracking`, `.kerning`, or fixed
line-spacing rule. System text styles supply Apple's size-specific optical
tables: display text tightens, body stays near neutral with comfortable
leading, and small reading text opens. Monospaced transcript/code text is also
relative to a semantic role.

The Core test fixes the role-to-style/base-size mapping, optical intent, the
unbounded reading policy, and the requirement that every non-reading boundary
has an XXL cap and a nonempty rationale.

## Reading-surface inventory

The App target contains 252 `GaryxFont` semantic-role uses plus 21 native
SwiftUI semantic-style uses. The Widget target contains 13 custom point-size
uses routed through a shared `@ScaledMetric(relativeTo:)` adapter. The complete
App file inventory is below; the number is the semantic typography call-site
count in that file.

| App file | Sites | App file | Sites |
| --- | ---: | --- | ---: |
| `GaryxCapsuleChromePanel.swift` | 3 | `GaryxGatewaySwitcherViews.swift` | 7 |
| `GaryxImagePreview.swift` | 4 | `GaryxMessageActionMenu.swift` | 2 |
| `GaryxMessageTextSelectionViews.swift` | 2 | `GaryxMobileAgentPickerComponents.swift` | 24 |
| `GaryxMobileAgentsViews.swift` | 8 | `GaryxMobileAutomationViews.swift` | 12 |
| `GaryxMobileBotSettingsViews.swift` | 7 | `GaryxMobileCapsuleViews.swift` | 5 |
| `GaryxMobileClaudeCodeAuthViews.swift` | 10 | `GaryxMobileCommandsViews.swift` | 2 |
| `GaryxMobileComponents.swift` | 1 | `GaryxMobileComposerViews.swift` | 12 |
| `GaryxMobileConversationStatusViews.swift` | 6 | `GaryxMobileConversationViews.swift` | 1 |
| `GaryxMobileDesignSystem.swift` | 4 | `GaryxMobileFormComponents.swift` | 20 |
| `GaryxMobileListComponents.swift` | 2 | `GaryxMobileMarkdownViews.swift` | 12 |
| `GaryxMobileMcpViews.swift` | 2 | `GaryxMobileMessageBubbleViews.swift` | 11 |
| `GaryxMobileProviderSettingsViews.swift` | 14 | `GaryxMobileSettingsViews.swift` | 4 |
| `GaryxMobileSidebarViews.swift` | 17 | `GaryxMobileSkillsViews.swift` | 8 |
| `GaryxMobileStatusComponents.swift` | 10 | `GaryxMobileTaskTreeSidebarViews.swift` | 6 |
| `GaryxMobileThreadRuntimeSettingsViews.swift` | 16 | `GaryxMobileToolTraceViews.swift` | 11 |
| `GaryxMobileTurnViews.swift` | 2 | `GaryxMobileViews.swift` | 6 |
| `GaryxMobileWorkspacePreviewViews.swift` | 9 | `GaryxMobileWorkspacesViews.swift` | 10 |
| `GaryxThreadListDrilldownViews.swift` | 3 |  |  |

This covers the transcript and Markdown/code/table renderer; message, thread,
and row menus; settings and provider/runtime screens; all shared form rows and
selection sheets; navigation and task drawers; agent, automation, workspace,
skill, command, MCP, bot, and capsule management; image/file previews; status
and empty states; and the composer reading/editor surfaces.

The UIKit `Select Text` transcript sheet uses the same Core `body` role through
`UIFont.preferredFont`. Its line/paragraph spacing and text-container insets
derive from the scaled body font. The previous fixed 17-point attributed font
no longer exists.

| Widget file | Relative sites | Reading coverage |
| --- | ---: | --- |
| `GaryxCodingUsageWidget.swift` | 5 | empty state, heading, age/detail text |
| `GaryxRecentThreadsWidget.swift` | 4 | empty state, thread title, workspace |
| `GaryxUsageGaugeView.swift` | 4 | gauge value, title, detail, shared header |

Repository scans after the migration report:

- zero direct App `.font(.system(size:))` reading calls;
- zero App `UIFont.systemFont(ofSize:)` reading calls;
- zero App `.tracking` or `.kerning` overrides;
- zero exact one/two/three-line reading limits; 129 sites use the adaptive
  reading-line helper instead;
- Widget raw point-size calls consist of the one `@ScaledMetric` adapter and
  four fixed glyph/avatar-geometry uses, not prose.

`GaryxFont.fixedSystem` is intentionally named and documented as a
non-reading API. Its App uses are SF Symbols and fixed icon controls, avatar
fallback initials, or DEBUG probe geometry. The two explicit fixed `Text`
sites are initials centered inside fixed-diameter avatars.

## Fixed-geometry boundary inventory

Every boundary below grows through XXL and then stops. The Core enum owns the
durable rationale, and each application site has a local comment naming its
specific geometry. Reading content surrounding these controls remains
unbounded.

| Boundary | Sites | Application points | Reason |
| --- | ---: | --- | --- |
| Navigation chrome | 6 | Runtime morph header; static and interactive gateway identities; capsule morph header; shared panel-title capsule; home `Garyx` wordmark | These labels share a 44-point bar, neighboring icon controls, or matched morph anchors. XXL preserves one-line endpoint geometry without freezing ordinary sizes. |
| Composer accessory chrome | 3 | Workspace-mode accessory; attachment file chip; compact agent picker | These labels live in the fixed composer apron/tray. The editor and its reading text remain unbounded. |
| Segmented-control chrome | 5 | Bot root behavior; agent environment mode; automation target type; capsule gallery tab; Claude login method | Both labels must remain reachable in one fixed system segmented track. |
| Compact badge chrome | 3 | Shared status pill; workspace-option badge; task-tree state badge | These badges must remain inline with their parent reading row instead of consuming it. |
| Compact data visualization | 1 | Shared quota speedometer center/title group | Embedded labels must stay within the gauge arc; surrounding usage explanations remain unbounded. |
| Widget family chrome | 2 | Recent Threads root; Coding Usage root | WidgetKit family snapshots are fixed, non-scrolling canvases, so an XXL ceiling keeps every static row/gauge reachable. |

Fixed SF Symbols, the 56-point FAB glyph, avatar initials, and icon-only
44-point controls do not receive a text-size cap because they are not readable
language content; their point size is the geometry itself.

## Layout synchronization

- `garyxReadingLineLimit` removes compact line limits at XXXL and all
  accessibility categories. Compact categories retain the existing density.
- Shared form rows switch from trailing values to stacked values at XXXL;
  text fields become vertically expanding, leading-aligned fields.
- Transcript Markdown block, paragraph, list, bullet, line, table-cell, and
  table-width spacing uses `@ScaledMetric` relative to the matching reading
  role. Code blocks use the relative monospaced footnote role.
- Message/thread action menus use scaled row heights and vertical padding and
  widen to the available compact-iPhone width for expanded reading sizes.
- Drawer/thread rows scale spacing and move trailing metadata below the title;
  the navigation drawer uses 96% of the phone width at accessibility sizes.
- Agent cards place availability badges below identity at accessibility sizes;
  provider/runtime/settings rows and automation cards likewise wrap or stack
  instead of compressing their reading labels.
- Message bubbles and composer text grow vertically; fixed heights that owned
  reading text were changed to minimum heights or scaled padding.
- Widget custom sizes use `@ScaledMetric`; only the explicitly listed fixed
  family/gauge boundaries cap their final category.

## Screenshot matrix

The final Debug app was installed on an iPhone 17 Pro simulator running iOS
26.5. Each route used the deterministic DEBUG snapshot; no live gateway data
appears in the evidence. Screenshots remain in `/tmp` rather than being
committed or packaged as an Artifact.

| Size | Transcript | Drawer | Gateway settings | Automations | Agents | Automation form | Action menu |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Default (`large`) | `final-default/01-transcript.png` PASS | `02-drawer.png` PASS | `03-gateway-settings.png` PASS | `04-automations.png` PASS | `05-agents.png` PASS | `06-automation-form.png` PASS | `07-action-menu.png` PASS |
| Maximum standard (`extra-extra-extra-large`) | `final-xxxl/01-transcript.png` PASS | `02-drawer.png` PASS | `03-gateway-settings.png` PASS | `04-automations.png` PASS | `05-agents.png` PASS | `06-automation-form.png` PASS | `07-action-menu.png` PASS |
| Accessibility maximum (`accessibility-extra-extra-extra-large`) | `final-axxxl/01-transcript.png` PASS | `02-drawer.png` PASS | `03-gateway-settings.png` PASS | `04-automations.png` PASS | `05-agents.png` PASS | `06-automation-form.png` PASS | `07-action-menu.png` PASS |

All paths above are below `/tmp/garyx-task-2461/`. Visual inspection found no
overlap, hard truncation, per-character title collapse, or unreachable fixed
control in the captured viewport. Reading rows wrap and scroll naturally;
navigation, segmented, composer-accessory, badge, and menu geometry stays
stable.

Because the simulator is shared by concurrent tasks, every screenshot command
installed the final local app and then compared the installed code-bearing
binary with the build output after capture. This build is not dylib-backed, so
the checked file on both sides was `Gary X`; all 21 checks matched SHA-256
`1874960a94b7ed70d329437eea80d34607dacade6038b57cbfc74f0c8521468b`.
No capture survived a mismatched ownership check.

MD5 manifest:

```text
40e4ce88714993eee9ed1bce901e839d  final-default/01-transcript.png
b63dbb40ea6324bbc3b5c81435ae650d  final-default/02-drawer.png
305fa16e291cde8a7a0dc53ba18bb29a  final-default/03-gateway-settings.png
4156c2a4e825c89f63cb8a7f64d520d2  final-default/04-automations.png
3483ef0347a10bc6c2f00b85c6deb7b1  final-default/05-agents.png
21bc837abd518faa8658872efed7a3c4  final-default/06-automation-form.png
e4f5a0f07edbb8ce4d7a44daac8ddb0c  final-default/07-action-menu.png
d8d376681a64d451850fd37440bc1e35  final-xxxl/01-transcript.png
f9355b603d39035036d3afd9e22fcb2b  final-xxxl/02-drawer.png
ae0a30278057bc84dec398fb99ea8a58  final-xxxl/03-gateway-settings.png
145509c8f4d2e67fdc6e01455727ef10  final-xxxl/04-automations.png
d2adec80e6b55ba57a80b3885b1244c0  final-xxxl/05-agents.png
af7626bed1bf0505919b134d61b01f8f  final-xxxl/06-automation-form.png
7e834d77dd868dfc77717de825bf5490  final-xxxl/07-action-menu.png
fad62dfc3a4a9a151c951c95f5d4cfa6  final-axxxl/01-transcript.png
232b0856354a0895bb6b762daf2204ef  final-axxxl/02-drawer.png
497a21667eab7da07da40512fa6d80af  final-axxxl/03-gateway-settings.png
0fe32c6d6dc8fbb2c9ad23abfa09b9d3  final-axxxl/04-automations.png
22c112f5c797a436366e909a752a10e0  final-axxxl/05-agents.png
64a336691214823ba9e30c7355b6d63d  final-axxxl/06-automation-form.png
a7a16210d378dcc6975902fdf14880dd  final-axxxl/07-action-menu.png
```

## Validation record

- `xcodegen generate`: pass; `GaryxTypography.swift` is included in the app,
  Core package, and Widget extension where required.
- `swift test --filter GaryxTypographyTests`: pass (3 tests, 0 failures).
- `swift test --quiet`: pass (1,420 tests, 0 failures; 363.250 s).
- `xcodebuild -project GaryxMobile.xcodeproj -scheme GaryxMobile -destination
  'generic/platform=iOS Simulator' -configuration Debug
  CODE_SIGNING_ALLOWED=NO build -quiet`: pass for the App and Widget targets.
  The build retains existing warnings for `UIScreen.main`, `Text + Text`, and
  one redundant `await`; no new warning is emitted by the typography adapter.
- `git diff --check`: pass. Final invariant scans: 0 direct App
  `.font(.system(size:))`, 0 fixed App `UIFont` reading sizes, 0
  `.tracking`/`.kerning`, 0 exact one/two/three-line reading limits, 129
  adaptive reading-line sites, and 20 documented cap applications.
