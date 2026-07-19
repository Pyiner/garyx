# iOS Fluid P1-D Feedback Audit

Date: 2026-07-20
Baseline: `origin/main` at `8d2833792`

This is the completion inventory for P1-D. It applies the multimodal feedback
rules from `garyx-product-ui/references/apple-design.md` §13: causality,
same-frame harmony, and utility.

## Haptic inventory

`GaryxHapticEvent` is the testable semantic catalog. `GaryxMobileHaptics` is the
only UIKit adapter and caches one generator per UIKit pattern. Explicit
touch-down, gesture-begin, and operation-begin hooks call `prepare()` on the
same cached generator that later emits; every emission also re-arms that
generator for a repeated action.

| Event | Pattern | Preheat strategy | Visual/causal commit |
| --- | --- | --- | --- |
| Message send | Light impact | Send, Continue, and Retry button touch-down | `GaryxMobileModel.send`: after optimistic user row/run state is accepted, before the first network suspension |
| Thread pin/unpin | Selection | Thread menu label touch-down; row-swipe gesture begin; cached generator re-arm for nested unpin | After optimistic pin membership and placement publish, before transport starts |
| Thread favorite/unfavorite | Selection | Same thread menu/row-action selection generator | After the favorites reducer changes presented membership and publishes it |
| Capsule favorite/unfavorite | Selection | Gallery card or focused-star touch-down | After the capsule favorite reducer applies the visible star state, before transport starts |
| Capsule drag dismiss | Medium impact | First owned drag sample | Reducer returns `.dismiss`; final drag state and haptic land in the release branch before dismissal |
| Interactive back | Light impact | `beginInteractivePop()` | Irreversible `.committed` release branch, alongside commit-phase visual/canonical mutation |
| Message/thread action menu presentation | Light impact | Cached generator is re-armed after each presentation; thread menu presentation also primes its selection actions | Long-press recognition publishes the anchored menu in the same handler |
| Row swipe full reveal | Medium impact | Row-pan begin | First full-reveal threshold crossing, or release directly to the open landing |
| Clipboard copy | Success notification | Message menu presentation primes copy; cached generator re-arms after every copy | Immediately after the pasteboard write succeeds |
| Avatar generation success | Success notification | Generation operation begin | Immediately after the accepted state-machine result publishes the candidate |
| Avatar generation error | Error notification | Generation operation begin | Immediately after the accepted failure result publishes error state |
| Navigation drawer visibility | Light impact | Leading-edge gesture begin or menu-button touch-down | After reveal target and `sidebarVisible` change; nonanimated debug/restoration writes are silent |
| Task-tree visibility | Light impact | Trailing-edge gesture begin | After reveal target and `isTaskTreeSidebarOpen` change |
| Pinned-order drop | Selection | Reorder gesture begin | After the terminal preview is folded and the accepted order publishes |

The Core matrix test (`GaryxHapticFeedbackTests`) covers every semantic event,
UIKit pattern, and intended preparation point. Direct UIKit generator creation,
`.sensoryFeedback`, and raw `impactOccurred`/`notificationOccurred` calls no
longer exist outside `GaryxMobileHaptics`.

## Same-frame contract

Haptics are never attached to animation completion callbacks. Each `play`
call is in the same main-actor call stack as the state write that produces the
visual result:

- sends commit after the optimistic transcript/run-state write and before the
  first `await`;
- pin and favorite changes commit after their synchronous presentation reducer
  publishes and before their network task;
- capsule dismiss and interactive pop commit inside their irreversible release
  outcome branches, not at settle completion;
- row reveal and pinned reorder fire at their threshold/drop landing writes;
- success/error feedback fires beside the accepted result state.

SwiftUI renders those writes on the next display update, so the haptic and
visual mutation share one run-loop transaction rather than being separated by
an animation callback or async transport response.

## Press feedback inventory

`GaryxPressableRowStyle` is the shared style for App-target controls that
previously opted into `.plain`. It resolves the P1-C `GaryxMotion.press` token:
standard mode uses scale `0.96`, opacity `0.78`, and a `0.12 s` ease-out. Under
Reduce Motion the shared resolver removes spatial scale while retaining the
short opacity response; immediate mode removes the animation itself.

The audit found 117 App-target `.buttonStyle(.plain)` sites and two uses of the
old `GaryxItemActionMenuButtonStyle`. All 119 now use the shared style:

| App file | Sites | App file | Sites |
| --- | ---: | --- | ---: |
| `GaryxCapsuleChromePanel.swift` | 4 | `GaryxGatewaySwitcherViews.swift` | 4 |
| `GaryxImagePreview.swift` | 2 | `GaryxMessageActionMenu.swift` | 1 |
| `GaryxMobileAgentPickerComponents.swift` | 11 | `GaryxMobileAgentsViews.swift` | 4 |
| `GaryxMobileAutomationViews.swift` | 8 | `GaryxMobileBotSettingsViews.swift` | 1 |
| `GaryxMobileCapsuleViews.swift` | 5 | `GaryxMobileClaudeCodeAuthViews.swift` | 5 |
| `GaryxMobileComponents.swift` | 6 | `GaryxMobileComposerViews.swift` | 8 |
| `GaryxMobileConversationStatusViews.swift` | 2 | `GaryxMobileConversationViews.swift` | 4 |
| `GaryxMobileFormComponents.swift` | 10 | `GaryxMobileListComponents.swift` | 3 |
| `GaryxMobileMarkdownViews.swift` | 1 | `GaryxMobileMessageBubbleViews.swift` | 2 |
| `GaryxMobileProviderSettingsViews.swift` | 1 | `GaryxMobileSidebarViews.swift` | 12 |
| `GaryxMobileSkillsViews.swift` | 1 | `GaryxMobileStatusComponents.swift` | 4 |
| `GaryxMobileTaskTreeSidebarViews.swift` | 1 | `GaryxMobileThreadRuntimeSettingsViews.swift` | 4 |
| `GaryxMobileToolTraceViews.swift` | 3 | `GaryxMobileTurnViews.swift` | 1 |
| `GaryxMobileViews.swift` | 2 | `GaryxMobileWorkspacePreviewViews.swift` | 1 |
| `GaryxMobileWorkspacesViews.swift` | 5 | `GaryxRecentThreadFilterMenu.swift` | 1 |
| `GaryxThreadListDrilldownViews.swift` | 2 |  |  |

### Explicit exemptions

| File | Site | Reason |
| --- | --- | --- |
| `Widget/GaryxRecentThreadsWidget/GaryxRecentThreadsWidget.swift` | Per-thread `Link` | WidgetKit renders a static timeline and does not provide the live press frames/environment used by the App style. Keep `.plain` to preserve per-row Link routing. |
| `Widget/GaryxRecentThreadsWidget/GaryxCodingUsageWidget.swift` | Whole-card `Link` for medium/large families | Same WidgetKit static-interaction constraint; the small family separately uses its required family-scoped `widgetURL`. |

There are no App-target exemptions. A repository audit now returns zero App
`.buttonStyle(.plain)` sites and exactly the two WidgetKit exceptions above.

## Validation record

- `swift test --filter GaryxHapticFeedbackTests`: pass (2 tests).
- `swift test --filter GaryxMotionTests.testPressFeedbackKeepsOpacityWhenReduceMotionRemovesScale`: pass (1 test).
- `swift test --quiet`: pass (1,417 tests, 0 failures; 271.159 s).
- Standard Debug simulator target build: pass (`xcodebuild`, `GaryxMobile`
  target, `iphonesimulator` SDK, code signing disabled).
- Simulator walkthrough: pass on iPhone 17 Pro / iOS 26.5. Installed and
  launched the final Debug simulator app, then opened and visually inspected
  the deterministic new-thread draft, Capsules, and open navigation-drawer
  routes. All three rendered and the process remained alive.
- Real-device haptic feel/latency: required follow-up. Simulator validates event
  paths and layout while Core validates the press-state token, but Simulator
  has no Taptic Engine.
