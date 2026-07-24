# Thread Runtime Picker — Review Debt

Adjacent problems found while fixing the iOS per-thread model picker (the
follow-default row swallowed the provider default's own row, so choosing
"Claude Opus 5" cleared the thread cell instead of pinning Opus 5).

Both are pre-existing and outside that fix's path. Recorded here instead of
being folded into it; each needs its own task.

## 1. The Mac composer model menu has the same structural defect

`desktop/garyx-desktop/src/renderer/src/ComposerForm.tsx:405-424`

The menu renders a follow-default item labelled `defaultModelLabel` — the
default model's own label — and then filters that model out of the real rows:

```tsx
{models
  .filter((option) => option.id !== defaultModelId)
  .map((option) => ...)}
```

Selecting that item calls `selectModelSanitizingTier(null)`, i.e. clears the
selection. So the only row reading e.g. "Claude Opus 5" does not pin Opus 5.
The same shape repeats for thinking levels at `:436-460`
(`defaultReasoningEffortId`).

Why it does not reproduce today: `resolveComposerModelControlState`
(`src/renderer/src/composer-model-control.ts:50-51`) resolves
`configuredDefaultModelId` as `agentConfiguredModel || providerModels.defaultModel`,
so a thread bound to an agent with an explicit model uses that agent model as
the default basis, leaving the provider default with its own row. A thread whose
agent configures no model falls through to the provider default and loses that
model's row exactly like iOS did.

The iOS fix put the row contract in
`GaryxThreadModelOverridePresentation.pickerOptions`
(`mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxThreadModelOverridePresentation.swift`):
row 0 carries a fixed follow-default label and never borrows a concrete value's
label; every advertised option keeps its own real-id row. Desktop should adopt
the same contract, which also means deciding whether the desktop default basis
(agent model first) or the iOS one (provider default, agent model applied
server-side) is the intended cross-platform rule.

## 2. Choosing a model can silently clear thinking level and speed

`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileThreadRuntimeSettingsViews.swift`
`selectModel(_:)`

When the thread's current thinking level or service tier is not supported by the
newly chosen model, the same request sends `""` for those cells, clearing them
without telling the user. Desktop does the equivalent through
`selectModelSanitizingTier`.

Sanitizing is correct — an unsupported level must not stay pinned — but the
silent part is the debt: the user picks a model and separately loses a thinking
level they had set. Needs a product decision (surface it, or re-pin the nearest
supported level) rather than a code tweak.
