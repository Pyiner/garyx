# TASK-1753: iOS settings business logic → GaryxMobileCore (behavior-preserving)

Status: design for review
Scope: `mobile/garyx-mobile` only. Pure refactor — no UI structure, copy,
interaction, or wire-payload change. Oracle: existing full `swift test` stays
green, new Core tests pin current behavior, app target builds.

## Verified evidence (current worktree, `d66fd109`)

### P2-1: provider settings business rules in the view

`App/GaryxMobile/GaryxMobileProviderSettingsViews.swift`

- `:499-551 authenticationSection` — the view decides the auth-section variant
  by `provider.providerType == "claude_code"` / `provider.isNative`, and which
  menu options exist by `provider.providerType == "gpt"`.
- `:606-608 supportsServiceTier` — `providerType == "gpt" &&
  catalog?.supportsServiceTierSelection == true` decides whether the Speed row
  exists and whether the tier is sent on save.
- `:686-708 saveDefaults` — the view assembles the save parameters:
  `serviceTier: supportsServiceTier ? serviceTier : nil`, `authSource/baseUrl:
  provider.isNative ? ... : nil`, `apiKey: provider.isNative ?
  GaryxProviderApiKeyUpdate.make(draft:existing:) : .keep`.
- Same cluster (not separately cited but part of the same decision surface):
  `:621-647` `effectiveAuthSource` (empty draft → provider default source),
  `authSourceLabel`, `showsApiKeyField` (gpt hides the key field while the
  shared token is selected), `apiKeyPlaceholder` (env-name map fallback), and
  `defaultModelLabel` (configured → catalog default → static fallback chain);
  `:665-672 selectAuthSource` (switching gpt back to the shared token clears
  the drafted key); `:674-684 fillDraft` (echo rules, native-only fields);
  `:121-167` row status/detail derivation in `GaryxProviderModelsRow`
  (effective-model fallback chain, capability counts, error/loading states).

### P2-2: bot settings business transforms in the view

`App/GaryxMobile/GaryxMobileBotSettingsViews.swift`

- `:380-399 defaultAccountId(for:)` — channel slug + `-main` + `-2…-99` /
  `-new` uniquification against existing account ids.
- `:454-478 normalizedConfigValues(fields:)` — save-time typed coercion
  (boolean/number/string), drop-empty-optional rules.
- `:563-624 GaryxBotSchemaField` (private, in the view file) — JSON-schema
  parsing: `properties`/`required`/`enum`/`default`/`description`/
  `placeholder`/`x-garyx.secret`, kind mapping, label derivation from
  snake_case keys, required-first case-insensitive sort.
- Same family: `:626-679` private `garyxBotObjectValue/ArrayValue/
  StringValue(IfPresent)/BoolValue` JSON coercion helpers; `:401-421`
  `applySchemaDefaults` + editor default value (`binding(for:)`); `:538-559`
  editor text→typed-value mapping (`.number(Double($0) ?? 0)`).

Both files live in the app target only. `project.yml` compiles
`Sources/GaryxMobileCore` **directly into the app target** (no module import),
so moved code stays same-module for the app; SwiftPM (`Package.swift`) builds
`GaryxMobileCore` + `GaryxMobileCoreTests` (`@testable import`), which is the
new test surface. The widget target cherry-picks 4 unrelated Core files —
untouched.

## Design

Two new Core files, logic moved **verbatim** (code motion + mechanical
parameterization of view state into function arguments). No behavior edits,
including pre-existing quirks (kept deliberately, listed under Non-goals).

### A. `Sources/GaryxMobileCore/GaryxProviderSettingsPresentation.swift`

```swift
/// Plans the provider-defaults sheet and provider-list row: which auth
/// variant shows, which fields exist, picker labels, echoed draft values,
/// and the save payload. Pure functions of provider descriptor + catalog +
/// settings document + draft state; the SwiftUI layer binds fields and calls
/// save.
public enum GaryxProviderSettingsPresentation {

    // Section planning
    public enum AuthSection: Equatable, Sendable {
        case claudeCode      // login-entry row driving the OAuth sheet
        case native          // auth-source menu, optional API key, base URL
        case managedOAuth    // read-only "Managed on the Mac app"
    }
    public static func authSection(for provider: GaryxModelProviderDefault) -> AuthSection
    /// gpt is the only provider with a second auth source (shared Codex token).
    public static func offersGptTokenAuthSource(_ provider: GaryxModelProviderDefault) -> Bool
    public static func supportsServiceTier(
        provider: GaryxModelProviderDefault, catalog: GaryxProviderModels?) -> Bool

    // Field-level rules (current computed vars, verbatim)
    public static func effectiveAuthSource(
        provider: GaryxModelProviderDefault, draft: String) -> String
    public static func authSourceLabel(effectiveAuthSource: String) -> String
    public static func showsApiKeyField(
        provider: GaryxModelProviderDefault, effectiveAuthSource: String) -> Bool
    public static func apiKeyPlaceholder(for provider: GaryxModelProviderDefault) -> String
    /// Selecting the shared gpt token clears the drafted key (save then blanks
    /// a previously stored key); selecting api_key keeps the draft.
    public static func apiKeyDraft(afterSelectingAuthSource source: String, current: String) -> String
    public static func defaultModelLabel(
        provider: GaryxModelProviderDefault, catalog: GaryxProviderModels?) -> String

    // Draft echo (fillDraft, verbatim incl. the native-only guard)
    public struct Draft: Equatable, Sendable {
        public var modelName, reasoningEffort, serviceTier: String
        public var authSource, baseUrl, apiKey: String   // "" for non-native
        public static func make(
            settings: [String: GaryxJSONValue],
            provider: GaryxModelProviderDefault) -> Draft
    }

    // Save payload assembly (saveDefaults parameter rules, verbatim)
    public struct SaveRequest: Equatable, Sendable {
        public var modelName: String
        public var reasoningEffort: String
        public var serviceTier: String?      // gpt+catalog-supported only
        public var authSource: String?       // native only, effective source
        public var baseUrl: String?          // native only
        public var apiKey: GaryxProviderApiKeyUpdate  // native: make(draft:existing:), else .keep
        public static func make(
            provider: GaryxModelProviderDefault,
            catalog: GaryxProviderModels?,
            modelName: String, reasoningEffort: String, serviceTier: String,
            authSourceDraft: String, baseUrl: String,
            apiKeyDraft: String, originalApiKey: String) -> SaveRequest
    }

    // Provider-list row (status pill + detail line, verbatim)
    public struct RowModel: Equatable, Sendable {
        public enum Tone: Equatable, Sendable { case good, muted, danger }
        public var statusText: String        // "Ready" / "Loading" / "Error"
        public var statusTone: Tone
        public var detailText: String
        public static func make(
            provider: GaryxModelProviderDefault,
            catalog: GaryxProviderModels?,
            settings: [String: GaryxJSONValue]) -> RowModel
    }
}
```

View changes (`GaryxMobileProviderSettingsViews.swift`):

- `authenticationSection` switches on `authSection(for:)` — the three
  branches keep their exact current bodies; the gpt-only menu item is gated by
  `offersGptTokenAuthSource`.
- `supportsServiceTier` / `effectiveAuthSource` / `authSourceLabel` /
  `showsApiKeyField` / `apiKeyPlaceholder` / `defaultModelLabel` computed vars
  become one-line Core calls.
- `selectAuthSource` sets `authSource = source; apiKey =
  apiKeyDraft(afterSelectingAuthSource:current:)`.
- `fillDraft` assigns from `Draft.make(...)` (all six fields plus
  `originalApiKey = draft.apiKey`; assigning "" to already-"" state for
  non-native is identical to today's early return).
- `saveDefaults` builds `SaveRequest.make(...)` and calls the model.
- The two `providerType == "claude_code"` reset-flow gates
  (`onDisappear`/`closeSheet`) test `authSection(for:) == .claudeCode`.
- `GaryxProviderModelsRow` renders `RowModel`; a small
  `RowModel.Tone → GaryxStatusPill.Tone` mapping stays in the view (same
  pattern as `GaryxUsageLevel.garyxTint` — pill tone is a view type).

Model change (`GaryxMobileModel+AgentsWorkspaces.swift`):

- `updateModelProviderDefaults(provider:modelName:reasoningEffort:serviceTier:
  authSource:baseUrl:apiKey:)` becomes
  `updateModelProviderDefaults(provider:request: SaveRequest)` and forwards
  `request.*` into the two existing `GaryxModelProviderDefaults.update` calls
  and the trims, unchanged. (Single caller today is this sheet.)

### B. `Sources/GaryxMobileCore/GaryxBotAccountConfigSchema.swift`

```swift
/// One manual channel-auth field from a plugin's JSON schema
/// (properties/required/enum/default/x-garyx), as shown in the bot form.
public struct GaryxBotSchemaField: Identifiable, Equatable, Sendable {
    public enum Kind: Equatable, Sendable { case string, boolean, number }
    public var id: String { key }
    public var key, label: String
    public var kind: Kind
    public var required, secret: Bool
    public var enumValues: [String]
    public var defaultValue: GaryxJSONValue?
    public var description: String?
    public var placeholder: String
    public static func fields(from schema: [String: GaryxJSONValue]) -> [GaryxBotSchemaField]
}

/// Config-value coercion shared by the bot-form editors and save path.
public enum GaryxBotConfigValues {
    public static func stringValue(_ value: GaryxJSONValue?) -> String   // was garyxBotStringValue
    public static func boolValue(_ value: GaryxJSONValue?) -> Bool?      // was garyxBotBoolValue
    /// Editor initial value: config → schema default → kind fallback.
    public static func editorValue(
        for field: GaryxBotSchemaField, config: [String: GaryxJSONValue]) -> GaryxJSONValue
    /// Text-editor input mapped into the field's JSON type.
    public static func fieldValue(fromEditorText text: String, kind: GaryxBotSchemaField.Kind) -> GaryxJSONValue
    /// Seed schema defaults into a draft (replacing: reset on channel switch).
    public static func applyingSchemaDefaults(
        to config: [String: GaryxJSONValue],
        fields: [GaryxBotSchemaField], replacing: Bool) -> [String: GaryxJSONValue]
    /// Save-time normalization: typed coercion + drop-empty-optional rules.
    public static func normalized(
        config: [String: GaryxJSONValue],
        fields: [GaryxBotSchemaField]) -> [String: GaryxJSONValue]
}

/// Default bot account id: channel slug + "-main", uniquified -2…-99 / -new.
public enum GaryxBotAccountIdDefaults {
    public static func defaultAccountId(
        channel: String, existingAccountIds: Set<String>) -> String
}
```

View changes (`GaryxMobileBotSettingsViews.swift`):

- Private `GaryxBotSchemaField`, the four `garyxBot*Value` helpers,
  `defaultAccountId`, `normalizedConfigValues` bodies are deleted; call sites
  become Core calls (`schemaFields(for:)` keeps its shape, now returning the
  Core type).
- `binding(for:)` get → `editorValue(for:config:)`; `applySchemaDefaults`
  keeps its plugin guard and delegates to `applyingSchemaDefaults`;
  `save()` uses `normalized(config:fields:)`;
  `resetGeneratedAccountIdIfNeeded` passes
  `Set(model.configuredBotAccountSettings.map(\.accountId))`.
- `GaryxBotConfigFieldEditor` bindings call `stringValue`/`boolValue`/
  `fieldValue(fromEditorText:kind:)`; enum picker set stays `.string($0)`
  (verbatim today).

## Behavior-preservation argument

1. Every extracted rule is verbatim code motion; the only edits are
   `private func`/computed-var → `static func` with view state passed as
   arguments. String literals ("Use GPT token", "Standard", "Provider
   default: …", "Ready"/"Loading"/"Error", "Required"/"Optional", label
   capitalization, join separators) are copied character-for-character.
2. The save wire payload is unchanged: for every (providerType, catalog,
   draft) combination `SaveRequest.make` reproduces exactly the old inline
   expressions — pinned by tests that enumerate each branch (see below). The
   model still calls the same `GaryxModelProviderDefaults.update` /
   `saveGatewaySettings(merge: true)`; bot saves still validate + write via
   `GaryxConfiguredBotAccountsDocument.setAccount` with an identical
   `GaryxConfiguredBotAccountInput`.
3. Hydration/edit-authority rules are untouched: the sheet still gates
   editing on `refreshAuthoritativeGatewaySettings()` success and echoes
   drafts only from the fetched document (mobile-ui "fetch authoritative data
   before saving" preserved); the bot save path still re-fetches settings and
   validates before writing.
4. New Core tests are characterization tests: expectations are derived from
   the current implementation before the move, then the implementation is
   copy-pasted. Existing suites (`GaryxModelProviderDefaultsTests`,
   `GaryxMobileGatewaySettingsModelsTests`, full package) keep guarding the
   layers underneath.

## Test plan (new, `Tests/GaryxMobileCoreTests/`)

`GaryxProviderSettingsPresentationTests.swift`

- `authSection` for every entry in `GaryxModelProviderDefaults.providers`:
  claude_code → claudeCode; gpt/anthropic/google → native;
  codex_app_server/antigravity/traex → managedOAuth.
- `offersGptTokenAuthSource` true only for gpt.
- `effectiveAuthSource`: empty/whitespace draft → "codex" (gpt) / "api_key"
  (anthropic, google); non-empty draft trimmed and passed through.
- `authSourceLabel`: "codex" → "Use GPT token"; "api_key"/other → "Use API key".
- `showsApiKeyField`: false for non-native; gpt+codex false; gpt+api_key
  true; anthropic/google true regardless of source.
- `apiKeyPlaceholder`: OPENAI_API_KEY / ANTHROPIC_API_KEY / GEMINI_API_KEY /
  "API key" fallback for CLI providers.
- `apiKeyDraft(afterSelectingAuthSource:)`: codex clears, api_key keeps.
- `supportsServiceTier`: gpt×(catalog true/false/nil) and non-gpt×true.
- `defaultModelLabel`: catalog default wins; whitespace catalog default falls
  to provider fallback (antigravity); both empty → "Provider default"
  (claude_code).
- `Draft.make`: full settings doc echoes all six fields (api key from
  `agents.<key>.env[<ENV>]`); non-native providers echo defaults but leave
  auth fields ""; empty settings → all "".
- `SaveRequest.make` matrix: claude_code (all nil/.keep), gpt with tier
  support (tier passed, empty auth draft → "codex", apiKey set/blank/keep
  triple: non-empty draft → .set(trimmed); empty draft + existing → .blank;
  both empty → .keep), gpt without tier support (tier nil), anthropic
  (tier nil, default "api_key", baseUrl passed).
- `RowModel`: catalog error → Error/danger + "Model metadata unavailable"
  when no parts; nil catalog → Loading/muted + "Loading metadata"; loaded →
  Ready/good; detail composition: configured model beats catalog default
  beats fallback; "Thinking …" suffix; models/reasoning/tiers counts only
  when the supports flags are true; separator " · ".

`GaryxBotAccountConfigSchemaTests.swift`

- `fields(from:)`: kinds string/boolean/number/integer/unknown→string; enum
  extraction; required set; `x-garyx.secret`; default passthrough;
  description; explicit placeholder vs Required/Optional; label
  `bot_token → "Bot Token"`; non-object property skipped; sort =
  required-first then case-insensitive key.
- `stringValue`: string passthrough; number 2 → "2", 2.5 → "2.5"; bools;
  null/array/object/nil → "".
- `boolValue`: bool passthrough; "true"/"yes"/"1" → true, "false"/"no"/"0" →
  false, other string → nil; number 0 → false, nonzero → true;
  null/array/object/nil → nil.
- `editorValue`: config wins → default → kind fallback (.bool(false) /
  .string("")).
- `fieldValue(fromEditorText:)`: number "1.5" → .number(1.5), junk →
  .number(0); string kind keeps raw text.
- `applyingSchemaDefaults`: replacing=true reseeds from defaults only;
  replacing=false keeps user values and fills only missing defaults.
- `normalized`: boolean coercion (string/number/absent → false); number
  empty optional removed, empty/junk required → 0, parsed otherwise; string
  trimmed, empty optional removed, empty required kept as "";
  keys not declared in fields preserved verbatim; absent value falls back to
  schema default before coercion.
- enum-valued number field (schema `type: number`/`integer` with `enum`, a
  reachable combination): the picker writes `.string(option)` into the
  editor draft, and save-time `normalized` coerces it to
  `.number(Double(option) ?? 0)` — pinned explicitly as a characterization
  test (editor state stays string, save normalizes as number).
- `defaultAccountId`: "Telegram" → "telegram-main"; slugging collapses
  non-alphanumerics (" My_Channel! " → "my-channel-main"); empty slug →
  "bot-main"; collision walk -2…-99; exhaustion → "-new".

## Validation

- `cd mobile/garyx-mobile && swift test` (full, no pipe) before and after.
- `xcodegen generate` (new Core files are globbed into both the SwiftPM
  target and the app target; regenerated `project.pbxproj` is committed).
- `cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj
  -target GaryxMobile -sdk iphonesimulator -configuration Debug build`.
- Rebase onto latest origin/main before merging; pbxproj conflicts resolved
  by taking either side and re-running `xcodegen generate`; full validation
  re-run after rebase.

## Non-goals (pre-existing quirks kept verbatim, on purpose)

- `stringValue` integer formatting uses `String(Int(number))`, which would
  trap on absurdly large doubles — pre-existing; a safer `Int(exactly:)`
  variant exists in `GaryxMobileGatewaySettingsModels` but converging them
  would change formatting edge cases. Follow-up candidate, not this task.
- Number editors coerce unparseable text to `0` on each keystroke;
  save-time `normalized` re-coerces — unchanged.
- Enum pickers write `.string(option)` into the editor draft regardless of
  field kind; for number-kind enum fields the saved payload is then
  `.number(Double(option) ?? 0)` via `normalized` — both halves unchanged
  and pinned by the characterization test above.
- Bot form field-required error copy, display-name echo rule, and agent
  default selection stay in the view/model exactly as today (not part of the
  audited findings).
- No visual, copy, or interaction changes anywhere.
