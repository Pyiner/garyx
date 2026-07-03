# Provider Pages Overhaul — Canonical Three-Platform Design

Status: **Frozen.** This is the single authoritative design for the "model
provider three-platform overhaul" feature. It supersedes the two concurrent
draft designs (claude baseline + codex partition). Every open question from the
drafts has been resolved by the product owner; the resolutions are recorded
verbatim in §2 and threaded through every downstream section. Implementation
agents follow this document phase by phase (§11) and do not re-open the locked
decisions.

Scope: the model-provider surface on **Mac desktop, iOS, and the CLI** — field
display and default-config editing, per-model remaining-quota (usage)
visualization, the iOS widget → provider deep link, and the mobile form-row
click-target fix.

Ground rules honored here (CLAUDE.md):

- The **Mac app is the source of truth** for information architecture, labels,
  field meaning, icon semantics, and Gateway-backed data models. iOS adapts to
  native grouped-list patterns; the CLI adapts to text/JSON. Neither invents new
  top-level concepts.
- Provider/agent identity presentation resolves through the shared presentation
  helpers (`ProviderAgentIcon` on Mac, `GaryxProviderPresentation` in Core),
  never a new local switch table.
- Pure iOS route/presentation/formatting logic lives in `GaryxMobileCore` with
  SwiftPM tests.
- The usage token-acquisition layer is already shipped and is **not** touched.
- **No server schema change.** Every wire/snapshot field needed here already
  ships; all additions are client-side and additive so the installed widget
  contract keeps decoding.
- Public repository: no real personal data. All examples use synthetic
  placeholders (`Test User`, `sk-ant-EXAMPLE`, `${OPENAI_API_KEY}`,
  `/Users/test`).

---

## 0. TL;DR — what ships

- **Mac** keeps its fixed 8-row table but rationalizes columns
  (`Provider · Auth · Default · Status · Actions`), drops the redundant
  `Type` column to a subtitle, wires the `ProviderAgentIcon` glyph, turns Auth
  into a badge and Default into `model · reasoning · tier` chips, upgrades the
  page top with an independent Quota hero for metered providers, keeps the
  Configure modal's full Usage section,
  and **fixes the real bug where Antigravity default config never persists**.
- **iOS** keeps the whole-row-tappable provider list and **gains plaintext
  editing** for the value-typed defaults it lacks — API key (plaintext, echoed),
  base URL, auth source, service tier for the three native providers, plus
  model/reasoning for all — writing the **same `agents.<key>` shape the Mac app
  writes**. Host-environment fields stay read-only and "Managed on the Mac app"
  (semantic reason, not security). The provider list gains a **Quota hero** so
  the widget deep-link lands on a quota-forward surface.
- **Usage visualization** is one shared spec (§4) across all three ends: dual
  `Session`+`Weekly` meters for Claude/Codex, per-model mini-bars for
  Antigravity, plan pill, reset countdown, stale dimming, and a single shared
  remaining-% severity color scale (`≥50 green / 20–50 amber / <20 red`). The
  five non-metered providers show "No quota data" rather than hiding.
- **CLI** gets a new `garyx provider {list,show,set}` group and a `garyx usage`
  command with human + `--json` output; `garyx config provider-model` stays as a
  **deprecated alias** that hints at `garyx provider set`.
- **Widget** stays minimal: Claude + Codex only (no Antigravity), and a single
  whole-widget `.widgetURL(garyx://mobile/settings/provider)` that opens the
  provider **list** page. No per-gauge `Link`, no `?provider=` param.
- **Form rows** get a hardened shared primitive layer (opt-in `onTap`, a new
  `GaryxFormMenuRow`, focus-on-tap text rows); the enumerated dead-click rows
  (§9.3) migrate; toggles/steppers are untouched.
- **Data layer**: no server schema change; only client-side additions (extend
  the iOS display model, shared constants) and no widget snapshot change.

---

## 1. Current-state diagnosis (verified against source)

All file:line references below were verified against the tree at the time of
writing (main `f1c1b390`).

### 1.1 Data layer (shipped — do not rebuild)

`GET /api/usage/coding` (`garyx-gateway/src/coding_usage.rs`) returns
`CodingUsageResponse { providers, refreshed_at }` (`:195`, `:198`), always
exactly three providers: `claude_code`, `codex`, `antigravity`. Per provider,
`ProviderUsage` (`:149`):

- `id`, `name`, `available` (`:155`), `stale` (`:159`), `error?`,
  `plan?` (`:162`)
- `weekly?` (`:165`) / `session?` (`:168`) — `UsageWindow { used_percent,
  remaining_percent (:73), resets_at? (:76), reset_after_seconds? (:79) }`
  (Claude/Codex only)
- `models[]` (`:171`) — `ModelUsage { id, name, remaining_fraction (:106),
  remaining_percent (:108), used_percent, resets_at? (:113),
  reset_after_seconds? (:116), description? (:119) }` (**Antigravity only** — one
  bucket per model)

Per-provider ~20 s cache with stale-on-error fallback. The five non-metered
providers (`traex`, `gemini_cli`, `gpt`, `anthropic`, `google`) have no usage
source and never appear in this response.

**Everything "nice usage" needs is already on the wire** — `session`, `plan`,
`stale`, `resets_at`, `reset_after_seconds`, per-model `description`. This is the
basis for the "no server schema change" rule (§10).

Provider defaults live at `agents.<key>` in `~/.garyx/garyx.json` as
`AgentProviderConfig` (`garyx-models/src/config.rs:183`), a flat superset. The
fields this feature touches:

| Field | Source of truth | Notes |
|---|---|---|
| `provider_type` | `:188` | fixed per row |
| `default_model` | `:194` | editable everywhere |
| `model` | `:215` | legacy; writers `delete` it and use `default_model` |
| `model_reasoning_effort` | `:217` | editable everywhere |
| `model_service_tier` | `:219` | native (`gpt`) only |
| `auth_source` | `:237` | native providers; default `codex` |
| `base_url` | `:239` | native providers |
| `env` (`HashMap`) | `:201` | **API keys live here** (see §1.2) |
| `claude_cli_mode` / `claude_cli_path` | `:205` / `:207` | host, read-only on iOS |
| `codex_home`, `gemini_bin`, `antigravity_bin`, `approval_mode`, `permission_mode` | various | host, read-only on iOS |

The eight product providers and their config keys:

| Provider type | Config key | Group | Usage id |
|---|---|---|---|
| `claude_code` | `claude` | default (CLI) | `claude_code` |
| `codex_app_server` | `codex` | default (CLI) | `codex` |
| `antigravity` | `antigravity` | default (CLI) | `antigravity` |
| `traex` | `traex` | default (CLI) | — |
| `gemini_cli` | `gemini` | default (CLI) | — |
| `gpt` | `gpt` | native loop | — |
| `anthropic` | `anthropic` | native loop | — |
| `google` | `google` | native loop | — |

### 1.2 How the Mac app stores API keys (the iOS alignment target)

This is the source-of-truth mechanic decision #1 (§2) requires iOS to match.
Verified in `ProviderSettingsPanel.tsx`:

- **The API key is written into `env[<ENV_NAME>]`, not a dedicated field.**
  `apiKeyEnvName` (`:347`) maps provider type → env var name:
  `gpt → OPENAI_API_KEY`, `anthropic`/`claude_llm → ANTHROPIC_API_KEY`,
  `google`/`gemini_llm → GEMINI_API_KEY`.
- `mutateGatewayProviderModelDefaults` (`:842`) reads the existing `env`
  (`providerConfigEnv`, `:400`), sets `env[envName] = apiKey.trim()` (or deletes
  the key when blank, `:869–874`), and writes `auth_source` (`:881`) and
  `base_url` (`:883`) as **top-level `agents.<key>` fields**.
- For `gpt`, `auth_source` is `codex` (OAuth shared with Codex) or `api_key`; the
  API-key field only shows when `authSource === 'api_key'` (`:1292`). For
  `anthropic`/`google` the key field always shows.

So the iOS write target is: **`agents.<key>.env[<ENV_NAME>]` for the key**,
**`agents.<key>.auth_source`** and **`agents.<key>.base_url`** as top-level
fields, exactly mirroring Mac. iOS does not introduce a separate key field.

### 1.3 Mac — `ProviderSettingsPanel.tsx`

Fixed 8-row shadcn table (`MODEL_PROVIDER_ROWS`, ~`:97–167`) with columns
Provider / Type / Auth / Model / Usage / Status / Actions, plus one shared
Configure `Dialog` (`:1096–1495`). Diagnosis:

- **`Type` column is redundant** — the row *is* the provider type (fixed rows);
  it duplicates the name for zero added meaning.
- **No provider identity glyphs** — labels are hardcoded; the ready-made
  `ProviderAgentIcon` (`app-shell/components/ProviderAgentIcon.tsx`,
  `@lobehub/icons`) is not wired in.
- **Usage cell under-uses the data** — `renderProviderUsageCell` (`:766–828`)
  renders `remaining_percent` only and ignores `session` and `plan`; Antigravity
  shows only the tightest bucket (rest in a hover tooltip).
- **Bug — Antigravity default config never persists.** `handleSaveProviderConfig`
  (`:901–995`) has explicit branches for `claude_code`, `codex_app_server`,
  `gemini_cli`, `traex`, and `group === 'native'` (gpt/anthropic/google).
  **Antigravity matches none** — its key is not special-cased and its group is
  `default` (CLI), not `native` — so saving Antigravity defaults falls through to
  the `finally` and is a **silent no-op**. Confirmed real. Clean fix: add an
  `antigravity` branch mirroring the `traex` branch (CLI provider, model
  defaults only, routed through `mutateGatewayProviderModelDefaults`).
- Persistence goes through the existing optimistic gateway-draft flow
  (`onMutateGatewayDraft` → `onSaveGatewaySettings`).

Net: the Mac panel is the richest editor but the presentation is dense, the
usage cell is weak, and one provider silently fails to save.

### 1.4 iOS — provider tab

`GaryxSettingsProviderContent` (`GaryxMobileSettingsViews.swift:476`) renders the
eight providers from Core `GaryxModelProviderDefaults.providers` (8 entries,
`GaryxModelProviderDefaults.swift:24–48`); each row is already a whole-row
`Button` (`:486–500`, `.contentShape(Rectangle())` `:562`) opening
`GaryxModelProviderDefaultsSheet` (`:668`). Diagnosis:

- **iOS can only edit `default_model` + `model_reasoning_effort`.** Core
  `GaryxModelProviderDefaults.update()` (`:88–113`) writes exactly those two
  keys plus `provider_type` — no service tier, base URL, API key, or auth source.
  A real parity gap vs. Mac.
- **Write path is clean and additive-friendly:** sheet `saveDefaults()`
  (`GaryxMobileSettingsViews.swift:761`) → `updateModelProviderDefaults`
  (`GaryxMobileModel+AgentsWorkspaces.swift:941`) → builds the patch via
  `GaryxModelProviderDefaults.update` → `saveGatewaySettings(patch, merge: true)`
  (`GaryxGatewayClient.swift:295`) → `PUT /api/settings?merge=true`. iOS already
  writes `agents.<key>` — it just writes too few keys.
- **The editor sheet is a dead-click offender.** `GaryxProviderDefaultPickerRow`
  (`:780`) wraps a `Menu` inside `GaryxFormRow` (`:788–811`), so only the
  trailing label is tappable; used at Model (`:690`) and Thinking (`:699`).
- **The inline usage display drops data.** `GaryxProviderUsageDisplayModel`
  (`GaryxMobileUsageWidgetData.swift:481`) has only
  `providerId/summaryText/detailText/available/models`; `make` (`:502`) reads
  `available/error/weekly/models/stale` and **never reads `plan`, never reads
  `session`**, folding `stale` into a string. The richer data survives on the
  wire model `GaryxProviderUsage` (`:125–197`: `plan`, `weekly`, `session`,
  `models`, `stale`) and in the App Group snapshot — only the display model
  loses it.

### 1.5 CLI

There is **no `garyx provider` and no `garyx usage` command** (top-level
`Commands` enum, `garyx/src/cli.rs:34`). Provider editing is scattered:

- `garyx config provider-model <provider>` (`ConfigAction::ProviderModel`,
  `cli.rs:326`; impl `cmd_config_provider_model`, `commands.rs:1014`) supports
  only `--model` (`:331`), `--model-reasoning-effort` (`:337`),
  `--claude-cli-mode` (`:347`), `--claude-cli-path` (`:353`) (+ `--clear-*`,
  `--json`), and PUTs `{ "agents": { "<key>": … } }` to
  `/api/settings?merge=true` (`commands.rs:1081`).
- `garyx agent create|update|upsert --provider*` (`AgentAction`, `cli.rs:1048`;
  impls `commands.rs:3899/3948/4008`, POST/PUT `/api/custom-agents`) edits
  **custom agents**, not provider defaults. Untouched by this feature.
- Reusable pieces: `provider_model_config_key(&ProviderType)` (`commands.rs:1000`)
  already maps every provider type → config key; gateway HTTP helpers
  `fetch_gateway_json` (`:2016`), `put_gateway_json` (`:2136`); and
  `cmd_automation_list` (`:2420`) is the canonical `--json`-or-aligned-table
  pattern to mirror.

### 1.6 Widget — "Garyx Quota"

`Widget/GaryxRecentThreadsWidget/GaryxCodingUsageWidget.swift`. A speedometer arc
per provider (`GaryxUsageSpeedometer`, `:104`; arc `GaryxUsageGaugeArc`, `:84`),
families small/medium/large, reading the App Group snapshot
(`GaryxUsageWidgetStore.loadSnapshot`, `GaryxMobileUsageWidgetData.swift:283`).
Diagnosis:

- **No `widgetURL` / `Link` anywhere** — tapping cold-launches to the default
  page. (The sibling recent-threads widget deep-links per row via `Link`,
  `GaryxRecentThreadsWidget.swift:94`.)
- Shows **Claude Code + Codex only** (`widgetProviderIds`,
  `GaryxMobileUsageWidgetData.swift:608`; filtered in `widgetModels`, `:399`),
  even though the snapshot carries the full `GaryxCodingUsage` (all providers,
  incl. Antigravity — the store never drops data, only the render layer filters).
- `GaryxUsageSpeedometer` is a **`private struct` in the Widget target** — it is
  not importable from the App target. Its view-model `GaryxUsageGaugeModel`
  (`GaryxMobileUsageWidgetData.swift:311`) *is* Core-public and reusable. This
  matters for the iOS Quota hero (§6.4): the gauge **view** must be lifted to a
  shared source, not imported as-is.

### 1.7 Mobile form rows — the dead-click root cause

`GaryxFormRow` (`GaryxMobileFormComponents.swift:145`) lays out a bare
`HStack { title · Spacer · content }` (`trailingRow` `:176–187`; `stackedRow`
`:192–205`) with **no `Button`, no `.contentShape`, no tap target**. Only the
trailing `content` (a `Menu`/`Picker`/compact button) is hittable; the title +
`Spacer` (~60 % of the row) is dead.

Two correct primitives already exist and are the migration reference:
`GaryxFormSelectionRow` (`:588` — `Button` `:595` + `.contentShape(Rectangle())`
`:616` + `.buttonStyle(.plain)` `:618`) and `GaryxDisclosureListRow`
(`GaryxMobileListComponents.swift:61` — same shape at `:75/:114/:116`).

Editing the base layout has real blast radius: five subclass wrappers render
through `GaryxFormRow` — `GaryxFormReadOnlyRow` (`:208`),
`GaryxFormReadOnlyMultilineRow` (`:222`), `GaryxFormTextFieldRow` (`:279`),
`GaryxFormSecureFieldRow` (`:324`), `GaryxFormTextAreaRow` (`:344`). The fix must
therefore be **opt-in per interaction kind**, not a blanket "wrap everything in a
Button" (which would break `Menu` popovers, text-field focus, and toggle
semantics).

---

## 2. Locked product decisions (frozen — do not re-open)

These twelve resolutions replace the "open questions" of both drafts. They are
authoritative; the rest of this document is their expansion.

**D1 — iOS key editing = direct plaintext, echoed.** No multi-user, no
link-security concern; an API key is shown in its original form. Therefore:
- **Delete** the draft-B `GET/PATCH /api/providers` sanitized-secret API, and
  do **not** ship a masked / `Saved` / `Not set` replacement-field UX. iOS reads
  and writes plaintext directly.
- iOS provider detail: the three native providers (`gpt`/`anthropic`/`google`)
  can edit **API key (plaintext), base URL, auth source, service tier**; all
  providers can edit model / reasoning. Writes go through the existing
  `PUT /api/settings` to `agents.<key>`, **identical to Mac** — API key into
  `env[<ENV_NAME>]`, `auth_source`/`base_url` as top-level fields (§1.2).
- iOS stays **read-only, labeled "Managed on the Mac app"** for: OAuth sign-in,
  CLI path/bin, `claude_cli_mode`, `codex_home`, free-form `env`,
  approval/permission modes. The reason is **semantic** (these are gateway-host
  filesystem concepts, meaningless to set from a phone), not security.

**D2 — Mac keeps the rationalized table.** Columns
`Provider · Auth · Default · Usage · Status · Actions`; drop the `Type` column to
a subtitle under the name; wire `ProviderAgentIcon`; Auth becomes a badge;
Default becomes `model · reasoning · tier` chips; the Usage cell embeds the
compact dual-bar viz; the Configure modal gains a top Usage section; and the
**Antigravity save no-op bug is fixed** (`handleSaveProviderConfig` gains an
`antigravity` branch routed through `mutateGatewayProviderModelDefaults`).

**D3 — Quota color scale** is remaining-% `≥50 green / 20–50 amber / <20 red`,
defined once as a shared constant and consumed by all three ends.

**D4 — Plan name is shown by default** (no gating behind detail/JSON).

**D5 — CLI legacy command kept as alias.** `garyx config provider-model` stays
as a **deprecated alias** that prints a "use `garyx provider set`" hint; it is
not hard-removed. `garyx agent … --provider*` (custom agents) is untouched.

**D6 — The five non-metered providers show "No quota data"**, not hidden.

**D7 — Widget stays as-is, minimal.** Claude + Codex only (**no Antigravity**);
tapping the **whole widget** opens `garyx://mobile/settings/provider` (the
provider **list** page) via a single root `.widgetURL`. **No** per-provider
focus, **no** `?provider=` param, **no** per-gauge `Link`. Smallest possible
change.

**D8 — Usage lands inside the provider page**, no new top-level usage page. The
iOS provider list gains a **Quota hero** at the top (reusing the widget
`GaryxUsageSpeedometer` gauge visual) as the widget deep-link anchor.

**D9 — Usage visualization spec (cross-platform, §4):** Claude/Codex dual-window
meters (Session 5h + Weekly 7d); Antigravity per-model mini-bars (tightest
first) + `description`; plan pill; relative reset countdown
(`reset_after_seconds` preferred, fallback parse `resets_at`); stale → dimmed +
"updated Nm ago"; no source → "No quota data".

**D10 — Form fix = shared primitive layer, opt-in by interaction kind:**
`GaryxFormRow` gains an optional `onTap` (default `nil`, zero regression); a new
`GaryxFormMenuRow` makes the whole row the `Menu` label; text/secure rows gain a
`FocusState` tap-to-focus; toggles/steppers are **not** wrapped. Migrate the
enumerated dead-click rows (§9.3). The already-correct `GaryxFormSelectionRow` /
`GaryxDisclosureListRow` are left alone.

**D11 — Data layer = no server schema change** (session/plan/stale/resets/
description already ship). Client-side additions only: extend iOS
`GaryxProviderUsageDisplayModel` to surface `plan`/`session`/`stale` (Core,
unit-testable); one shared severity + countdown constant per end; widget App
Group snapshot unchanged.

**D12 — Phasing:** `P0 spec+constants → P1 CLI (first, pure headless) ∥ P2 form
∥ P3 Mac+bug fix → P4 iOS provider page (expanded editing + rich usage) → P5 iOS
hero + widget deep link`. Each phase is an independent task: implement →
self-review → merge. §11 gives per-phase files and verification.

---

## 3. Shared IA — one field model, one section order

Every surface presents a provider through the same semantic groups, in the same
order. This is the contract Mac / iOS / CLI each adapt (Mac table + modal, iOS
grouped form, CLI table + `show`). Section order (adopted from the draft-B
partition):

1. **Overview / Identity** — glyph + display name + provider type subtitle +
   runtime family (`Built-in CLI` / `Native model loop`). Resolve via the shared
   helpers, never a local switch table.
2. **Usage** — the §4 visualization (only the three metered providers; others
   show "No quota data"). Placed high because quota + auth readiness + defaults
   are one troubleshooting workflow (D8).
3. **Defaults** — default model, reasoning effort, and (native only) service
   tier. Empty value = "provider default" (existing config semantics).
4. **Authentication** — auth source/mode; for native providers the editable API
   key (plaintext on both Mac and iOS, D1) + base URL. OAuth sign-in for the CLI
   providers is status-only ("Signed in") and Mac-managed.
5. **Endpoint** — base URL for native providers.
6. **CLI Runtime (host)** — CLI mode, CLI/bin paths, free-form env,
   approval/permission. **Editable on Mac, read-only "Managed on the Mac app" on
   iOS** (D1). Rendered only for providers where the fields apply.
7. **Advanced** — timeouts, max turns, and other low-frequency runtime options,
   only if surfaced in this pass.

Field rationalization matrix (the concrete per-end deltas):

| Field | Mac today | Mac proposed | iOS today | iOS proposed | CLI |
|---|---|---|---|---|---|
| Identity glyph | none | `ProviderAgentIcon` | Core symbol | keep | name only |
| `Type` column | dedicated column | subtitle | read-only row | keep | `TYPE` col |
| Auth | free text | badge | not shown | badge + editable (native) | `AUTH` col |
| `default_model` | Select/Input | keep | menu row | keep | `--model` |
| `model_reasoning_effort` | Select | keep | menu row | keep | `--reasoning` |
| `model_service_tier` | Select (gpt) | keep | — | **add** (native) | `--service-tier` |
| `base_url` | Input (native) | keep | — | **add** (native) | `--base-url` |
| API key (`env[…]`) / `auth_source` | Input/Select (native) | keep | — | **add, plaintext** | `--api-key` / `--auth-source` |
| `claude_cli_mode` / path | Select/Input | keep | — | read-only | `--claude-cli-mode` / `--claude-cli-path` |
| env / bins / approval | editor/Textarea | keep | — | read-only | `--env` / `config set` |

---

## 4. Usage "remaining quota" visualization spec (D9)

### 4.1 The cross-platform contract

Each end implements this identically (TS on Mac, `GaryxMobileCore` on
iOS/widget, Rust formatting on CLI), reading only fields already on the wire.

- **Metric = `remaining_percent`** (fill = remaining, not used). Clamp 0–100.
- **Severity color (D3):** `healthy ≥ 50 %`, `warning 20–50 %`,
  `critical < 20 %`, `unavailable` when `!available`. One shared source per end
  (§10). Semantic color for usage state only; ordinary selected states stay
  monochrome.
- **Window shape:**
  - **Claude / Codex →** two meters, `Session` (5-hour) and `Weekly` (7-day),
    each a remaining-% fill + numeric + reset caption. Surfacing `session` is new
    on every end.
  - **Antigravity →** a list of per-model mini-bars (`name` + `% left` + reset),
    tightest first; `description` (e.g. "Quota resets in 1 hour") as caption when
    present. Never collapse `models[]` into a single provider aggregate in the
    detail view.
- **Plan pill (D4):** show `plan` (e.g. `Max`, `Pro`) next to the provider name
  when present, by default.
- **Reset countdown:** `reset_after_seconds` preferred; fallback parse
  `resets_at`. Format `resets in 2d 4h` / `1h 12m` / `<1m`. If the two disagree,
  prefer the shorter conservative value and keep the raw fields in JSON/hover.
- **Stale / freshness:** when `stale`, dim the meters, tag `stale`, and show
  `updated Nm ago` from `refreshed_at` (or the widget snapshot's fetch time). A
  stale-but-high meter must never look fresh-green.
- **Empty / error:** `!available` → muted "Unavailable" + `error` tooltip; **no
  data → "No quota data"** — the five non-metered providers always land here
  (D6).

### 4.2 Per-surface rendering

- **Mac Quota hero (top of provider page):** product-owner rework after P3:
  usage is no longer embedded in the provider table. The top of the page shows
  three compact cards for the metered providers only (Claude Code, Codex,
  Antigravity). Claude/Codex use a weekly primary gauge plus session secondary
  meter; Antigravity uses its tightest model bucket plus "N models". Each card
  carries plan, reset countdown, stale dimming/tag, and freshness text when
  stale. A follow-up product-owner pass removed the outer Quota container card:
  render a plain section heading row (`Quota`, freshness, refresh) followed
  directly by the three provider cards on the page background.
- **Mac Configure modal (full):** a top **Usage** section — both windows as
  labeled bars, all Antigravity buckets, plan, per-window reset countdowns,
  `updated Nm ago`. Replaces the current hover-only breakdown.
- **iOS list row (inline):** dual mini-bars (session+weekly) or a small ring +
  plan pill + reset caption; Antigravity keeps the per-model list with mini-bars.
  Requires extending `GaryxProviderUsageDisplayModel` (§6.3).
- **iOS Quota hero (top of provider list):** a horizontal row of gauges reusing
  the widget's `GaryxUsageSpeedometer` visual DNA (§6.4) — the "好看" centerpiece
  and the widget deep-link landing.
- **CLI:** `garyx usage` prints an aligned table (§7); `--json` dumps the raw
  `CodingUsageResponse`. Optional unicode bar `[██████░░░░]` with an ASCII-safe
  fallback.

---

## 5. Mac provider page (D2)

Keep the table (it is the source of truth and works); rationalize it.

- **Columns → `Provider · Auth · Default · Status · Actions`.** Drop the
  standalone `Type` column; render `provider_type` as a monospaced subtitle under
  the provider name (and in the Configure header).
- **Provider cell** gains the `ProviderAgentIcon` glyph + name + type subtitle.
- **Auth cell** becomes a `Badge` (`data-state` ready/empty/error) computed from
  the existing `providerRowDetails` logic, replacing the free-text column.
- **Default cell** shows `model · reasoning · tier` chips (effective defaults),
  truncating with a tooltip.
- **Usage is not a table column.** Product-owner rework after P3 moved quota out
  of rows to the page-top Quota hero (§4.2), restoring compact table height.
  The Quota heading row is unframed, without a background or border, and the
  three provider cards sit directly on the page background. The full breakdown
  remains in the Configure modal's Usage section.
- **Configure modal** keeps today's full field set, reordered to the §3 section
  order, adds the top **Usage** section for metered providers, and **fixes the
  Antigravity save path**: add an `antigravity` case in `handleSaveProviderConfig`
  (`:901`) that calls `mutateGatewayProviderModelDefaults(providerConfigRow,
  providerConfigDraft)` then the existing `onSaveGatewaySettings`, mirroring the
  `traex` branch (`:971`) — a CLI provider that persists model defaults only.

This is field rationalization + one bug fix + a usage upgrade, not a rewrite.

---

## 6. iOS provider page (D1, D8)

### 6.1 Structure (native grouped form, all rows whole-row-tappable per §9)

Provider list rows are unchanged (already whole-row `Button`). The detail sheet
`GaryxModelProviderDefaultsSheet` (`:668`) expands to the §3 section order:

- **Overview** — name, type (read-only rows).
- **Usage** — the §4 inline viz (metered providers).
- **Defaults** — Model, Thinking level, Service tier (native) as
  `GaryxFormMenuRow`s (from P2).
- **Authentication** — for native providers: `Auth source` menu row, `Base URL`
  field, **API key field (plaintext, echoing the current value)** (D1). For CLI
  providers: read-only "Signed in / Managed on the Mac app".
- **CLI Runtime / Advanced (host)** — read-only mirror of CLI mode, paths, env,
  approval with the "Managed on the Mac app" note (D1).

### 6.2 Editing = write the same `agents.<key>` shape as Mac

Extend Core `GaryxModelProviderDefaults.update()` (`:88–113`) so that, for native
providers, it also writes:

- `auth_source`, `base_url`, `model_service_tier` — top-level `agents.<key>`
  fields (empty string = provider default).
- **API key → `env[<ENV_NAME>]`**, using the same provider-type → env-name map as
  Mac's `apiKeyEnvName` (`gpt → OPENAI_API_KEY`, `anthropic → ANTHROPIC_API_KEY`,
  `google → GEMINI_API_KEY`). Port this map into Core so it is unit-testable.

**Merge/clobber correctness (important).** iOS saves via
`PUT /api/settings?merge=true`, which the gateway applies with `deep_merge_json`
(`garyx-gateway/src/api.rs:1876`). Deep-merge recursively overrides object keys,
so writing `agents.<key>.env = { ANTHROPIC_API_KEY: "…" }` **merges into** the
existing `env` and preserves sibling env keys — good for set/update. But
deep-merge **cannot express deletion** (`api.rs:2038, 2073`). Therefore:

- Per the mobile-ui contract ("edit paths that preserve hidden gateway fields
  must fetch authoritative data before saving"), P4 **fetches the authoritative
  `agents.<key>` config** (`GET /api/settings`) before opening the editor, so it
  echoes the real current key (D1) and does not rely on a stale projection.
- Set/update of `base_url` / `auth_source` / `model_service_tier`: write the
  value; empty string clears to provider default (scalar deep-merge is safe).
- Set/update of the API key: deep-merge `env[<ENV_NAME>] = value`.
- **Clearing** an env-backed API key (true key removal) is out of P4 scope on
  iOS — deep-merge cannot delete, and a `merge=false` full-document write from a
  phone is unsafe. Writing an empty `env[<ENV_NAME>] = ""` blanks it (equivalent
  to "no key" for native auth); a hard removal remains a Mac-app action. Note
  this in the sheet copy.

### 6.3 Rich inline usage — extend the display model (D11)

Extend `GaryxProviderUsageDisplayModel` (`GaryxMobileUsageWidgetData.swift:481`)
to surface `plan`, `session`, and `stale` as **distinct fields** (today `make`
at `:502` drops them). The wire model `GaryxProviderUsage` (`:125–197`) already
carries them, so this is a Core-only, additive, **unit-testable** change with no
wire impact. The row/hero then renders dual meters + plan pill + stale treatment
per §4.

### 6.4 Quota hero (D8)

Add a Quota hero at the top of `GaryxSettingsProviderContent` (`:476`): a
horizontal row of gauges for the three metered providers, reusing the widget
gauge visual. Because `GaryxUsageSpeedometer` is a **private struct in the Widget
target** (§1.6), P5 **lifts** the gauge view (`GaryxUsageSpeedometer` +
`GaryxUsageGaugeArc`) into a shared source file compiled into **both** the App
target and the Widget extension, driven by the already-Core-public
`GaryxUsageGaugeModel` (`:311`). One gauge implementation, two consumers. The
hero is the widget deep-link landing (D7/§8).

---

## 7. CLI provider & usage commands (D5)

Add a top-level `Provider { action }` variant + `Usage { … }` to `Commands`
(`cli.rs:34`), dispatched in `main.rs`, handlers mirroring `cmd_automation_list`
(`commands.rs:2420`) for the `--json`-or-aligned-table shape. Reuse
`provider_model_config_key` (`commands.rs:1000`) and the gateway helpers
`fetch_gateway_json` (`:2016`) / `put_gateway_json` (`:2136`).

**`garyx provider list [--usage] [--json]`** — all 8 providers:

```
PROVIDER      TYPE               KEY          AUTH        DEFAULT MODEL          USAGE          STATUS
Claude Code   claude_code        claude       signed in   (provider default)    73% wk         ready
Codex         codex_app_server   codex        api key     (provider default)    11% wk         ready
Antigravity   antigravity        antigravity  signed in   claude-opus-4-6       99% top model  ready
GPT           gpt                gpt          api key      gpt-5.5               —              ready
…
```

**`garyx provider show <provider> [--json]`** — the full `AgentProviderConfig`
defaults for one provider (`agents.<key>`), grouped by the §3 sections.

**`garyx provider set <provider> [flags] [--json]`** — the canonical default
editor, covering the value-typed superset:
`--model` / `--clear-model`, `--reasoning` / `--clear-reasoning`,
`--service-tier`, `--base-url`, `--api-key`, `--auth-source`,
`--claude-cli-mode`, `--claude-cli-path`, `--env KEY=VALUE` (repeatable),
`--clear-env KEY`. Builds `{ "agents": { "<key>": { … } } }` and PUTs
`/api/settings?merge=true` (same path as `config provider-model`). `--api-key`
writes `env[<ENV_NAME>]` to match Mac/iOS.

**`garyx usage [<provider>] [--json]`** — §4:

```
PROVIDER      PLAN   SESSION            WEEKLY             STATUS
Claude Code   max    98% · resets 2h    73% · resets 5d    ok
Codex         pro    98% · resets 3h    11% · resets 2d    ok
Antigravity   —      —                  —                  ok
  claude-opus-4-6        99% · resets 1h
  gemini-3-flash         84% · resets 5h
```

`--json` dumps the raw `CodingUsageResponse`; stale rows tag `stale (updated 3m
ago)`; unavailable shows the error. Connection failure prints a clear error and
still supports `--json` error output.

**Deprecation (D5):** `garyx provider set` is a strict superset of `config
provider-model`. Keep `config provider-model` working but print a one-line
"deprecated — use `garyx provider set`" hint on use. `agent … --provider*`
(custom agents) is untouched. Use synthetic values only in tests/docs.

---

## 8. Widget deep link (D7)

The route already resolves end-to-end: `garyx://mobile/settings/provider` parses
(`GaryxMobileRouteLink.swift:84`, settings branch `:113`) to `.settings(.provider)`
(`GaryxMobileNavigationState.swift:107`) and drives `openSettings(tab: .provider)`
(`GaryxMobileModel+Navigation.swift:213 → :138`). The only change is in the
widget.

**Add a single whole-widget `.widgetURL`** to the root of
`GaryxCodingUsageWidgetView`:
`.widgetURL(GaryxMobileRouteLink.make(.settings(.provider)))`. Apply the same URL
to every family (empty/placeholder/small/medium/large). This lands on the
provider list (whose top is the Quota hero, §6.4).

Explicitly **not** doing (D7): no per-gauge `Link`, no `?provider=` focus param,
no Antigravity in the widget. The "container `widgetURL` steals row taps" hazard
applies only to widgets with competing per-row `Link`s; this usage widget has
none, so a root `.widgetURL` is correct and minimal here.

---

## 9. Mobile form-row fix (D10)

### 9.1 Root cause

`GaryxFormRow` renders a bare `HStack`/`VStack` with no tap target; only the
trailing control is hittable (§1.7).

### 9.2 Primitive changes (shared layer, opt-in by interaction kind)

Three primitives, not one blunt wrapper:

1. **`GaryxFormRow` gains an optional `onTap: (() -> Void)? = nil`.** When set,
   wrap the existing layout in `Button(action:) { … }.contentShape(Rectangle())`
   `.buttonStyle(.plain)` — identical to `GaryxFormSelectionRow`. When `nil`,
   behavior is byte-for-byte unchanged (zero regression for the five subclass
   wrappers and every existing row). Covers navigation/present rows.
2. **New `GaryxFormMenuRow`** — a `Menu` whose `label:` *is the full-width row*.
   A `Menu` cannot be nested inside an outer `Button`, so the correct full-row
   fix for menu rows is to make the whole row the menu label. Replaces the
   `GaryxFormRow { Menu { … } }` anti-pattern.
3. **Focus-aware text rows** — `GaryxFormTextFieldRow` / `GaryxFormSecureFieldRow`
   take a `FocusState` binding and add
   `.contentShape(Rectangle()).onTapGesture { focus = field }` so tapping the
   label focuses the field. Do **not** wrap text rows in a `Button` (breaks caret
   placement / selection).

**Toggles/steppers keep their embedded control and are left un-wrapped** (D10) —
SwiftUI toggles already expose their own label hit area; a whole-row `Button`
would double-fire.

### 9.3 Migration inventory (verified)

Migrate these dead-click rows (all `GaryxFormRow { Menu/Picker/DatePicker }`):

| Feature file | Rows → target primitive |
|---|---|
| `GaryxMobileAutomationViews.swift` | `:666` Agent (picker-ctrl → `onTap`/present), `:678` Repeat (Menu → `GaryxFormMenuRow`), `:701` Date (DatePicker → `onTap`/present), `:715` Day (Menu → MenuRow), `:732` Date-monthly (Menu → MenuRow) |
| `GaryxMobileBotSettingsViews.swift` | `:296` Channel (Picker.menu → MenuRow), `:311` Agent (picker-ctrl → `onTap`), `:488` enum field (Picker → MenuRow) |
| `GaryxMobileTasksViews.swift` | `:397` Agent (picker-ctrl → `onTap`), `:414` Target (Menu → MenuRow) |
| `GaryxMobileSettingsViews.swift` | `GaryxProviderDefaultPickerRow` (`:780`) → `GaryxFormMenuRow`, used at Model (`:690`) + Thinking (`:699`) |

That is **12 concrete dead-click call sites** across four files (Automation 5,
BotSettings 3, Tasks 2, Settings 2). (The product memo's "13" counts this set;
the exact enumerable Menu/Picker/DatePicker rows are these 12 — migrate all of
them.) `GaryxAgentTargetPickerControl` (`GaryxMobileAgentPickerComponents.swift:428`)
is already a `Button` popover, so wrapping its host `GaryxFormRow` with `onTap`
triggering the same present action makes the whole row tappable.

**Not migrated (per D10):** toggle rows (Automation Enabled `:242`-style,
BotSettings `:480`, Tasks Start-immediately `:446`) and steppers (Automation
Hours `:694`). **Already correct, left alone:** `GaryxFormSelectionRow`,
`GaryxWorkspacePathSelectionRow`, `GaryxDisclosureListRow`. Read-only rows get a
no-op tap.

### 9.4 Migration strategy & regression checks

- **Phase A — land primitives only** (`GaryxFormRow.onTap`, `GaryxFormMenuRow`,
  focus-aware text rows). No call-site changes → zero behavior change, all green.
- **Phase B — migrate call sites per feature file** (Settings/provider-defaults
  first, then Automation, BotSettings, Tasks). Each file is a self-contained,
  verifiable migration.

Per-row simulator matrix: menu opens on a tap anywhere in the row; a
present/push fires from anywhere in the row; a text field focuses on a label tap
**and** still places the caret on a field tap; a toggle flips exactly once; multi-
control rows don't get a greedy whole-row tap; VoiceOver still exposes the
control + value; Dynamic Type does not clip (≥ 44 pt). SwiftUI interaction is not
headless-testable, so this is simulator-verified; the Core-testable slivers (menu
option lists, focus-field enums, deep-link URL build/parse) get SwiftPM tests.

---

## 10. Data layer (D11 — additive only)

**No server schema change.** Everything "nice usage" needs is already serialized
(§1.1). Additions are client-side:

- **iOS Core:** extend `GaryxProviderUsageDisplayModel` to surface `plan`,
  `session`, `stale` as distinct fields (§6.3). Additive, unit-testable, no wire
  change.
- **Shared constants:** the §4 severity thresholds (D3) + countdown formatter,
  factored to **one source per platform** — a Core enum on iOS/widget (unify with
  the existing `GaryxUsageGaugeModel.level`), one TS util on Mac (replacing the
  ad-hoc `usageLevelClass`), one Rust helper on the CLI.
- **iOS write map:** the provider-type → env-var-name map (§6.2) ported into Core
  for unit tests.
- **Widget App Group snapshot: unchanged.** It already stores the full
  `GaryxCodingUsage`; the widget stays Claude+Codex by render-layer filter (D7),
  no snapshot change. Any future field must remain optional-with-default so the
  installed widget keeps decoding.

No `?provider=` deep-link parsing is added (D7).

---

## 11. Phased implementation plan (D12)

Each phase is an independent, separately reviewable task: implement →
self-review → merge. Dependency order:
**P0 → (P1 ∥ P2 ∥ P3) → P4 → P5.** P4 needs P2's form primitives + P0's spec; P5's
hero needs P4, while P5's widget `.widgetURL` is independent and could pair with
any phase.

| Phase | Deliverable | End | Depends | No-UI testable? |
|---|---|---|---|---|
| P0 | §4 spec + shared severity/countdown constants | all | — | **yes** |
| P1 | `garyx provider {list,show,set}` + `garyx usage` | CLI | P0 | **yes** |
| P2 | Form-row hardening (primitives + 12 migrations) | iOS | — | partial |
| P3 | Mac panel: columns/glyphs/usage cell + **Antigravity save fix** | Mac | P0 | partial |
| P4 | iOS provider page: plaintext editing + rich inline usage | iOS | P2, P0 | partial |
| P5 | iOS Quota hero + widget `.widgetURL` | iOS/widget | P4 | partial |

### P0 — Spec + shared constants

- **Files:** Core (new severity enum + countdown formatter in
  `GaryxMobileCore`, unifying `GaryxUsageGaugeModel.level`,
  `GaryxMobileUsageWidgetData.swift:311`); Mac TS util (new shared helper
  replacing `usageLevelClass` in `ProviderSettingsPanel.tsx`); Rust helper in the
  `garyx` crate.
- **Verification:** SwiftPM test (thresholds + countdown formatting incl.
  `reset_after_seconds`-vs-`resets_at` fallback), desktop unit test, Rust unit
  test. Fully headless.

### P1 — CLI (land first)

- **Files:** `garyx/src/cli.rs` (add `Provider { action }` + `Usage { … }` to
  `Commands`, `:34`; new `ProviderAction` enum), `garyx/src/main.rs` (dispatch
  near `:389`), `garyx/src/commands.rs` (new `cmd_provider_list/show/set` +
  `cmd_usage`, mirroring `cmd_automation_list` `:2420`, reusing
  `provider_model_config_key` `:1000`, `fetch_gateway_json` `:2016`,
  `put_gateway_json` `:2136`; add the deprecation hint to
  `cmd_config_provider_model` `:1014`).
- **Verification:** Rust unit — arg parse, patch-object build, config-key
  mapping, clear semantics, golden aligned-table + `--json` snapshots against a
  synthetic `CodingUsageResponse`; a live `garyx usage --json` diff against
  `/api/usage/coding`. Fully headless — this makes the whole surface
  scriptable/reviewable before any UI moves.

### P2 — Form-row hardening

- **Files:** `GaryxMobileFormComponents.swift` (`GaryxFormRow` `:145` optional
  `onTap`; new `GaryxFormMenuRow`; `GaryxFormTextFieldRow` `:279` /
  `GaryxFormSecureFieldRow` `:324` `FocusState`). Migrations (§9.3):
  `GaryxMobileSettingsViews.swift` (`GaryxProviderDefaultPickerRow` `:780`),
  `GaryxMobileAutomationViews.swift` (`:666/:678/:701/:715/:732`),
  `GaryxMobileBotSettingsViews.swift` (`:296/:311/:488`),
  `GaryxMobileTasksViews.swift` (`:397/:414`).
- **Verification:** `xcodebuild` (App target) + simulator matrix (§9.4) + SwiftPM
  Core slivers. If any new file is added to the App target, run `xcodegen
  generate` and commit the updated `.pbxproj` (else the app won't compile the new
  file even though `swift test` is green).

### P3 — Mac panel + Antigravity save fix

- **Files:** `desktop/garyx-desktop/src/renderer/src/settings/ProviderSettingsPanel.tsx`
  — columns/rows (`MODEL_PROVIDER_ROWS` ~`:97–167`), `renderProviderUsageCell`
  (`:766`), the **`antigravity` branch** in `handleSaveProviderConfig` (`:901`),
  the Configure `Dialog` Usage section (`:1096–1495`), and wiring
  `ProviderAgentIcon` (`app-shell/components/ProviderAgentIcon.tsx`). Consume the
  P0 shared TS severity util.
- **Verification:** `npm run build:ui` + `npm run test:unit` + TS util unit test;
  a packaged-app CDP screenshot pass of the table + Configure modal; an
  **Antigravity save round-trip** (save a default, read back `agents.antigravity`
  to prove it persists — the bug fix). Follow the packaged-app renderer check in
  `docs/agents/validation.md` (build `dist:dir`, restart the app, attach CDP).

### P4 — iOS provider page (expanded editing + rich usage)

- **Files:** `GaryxMobileCore/GaryxModelProviderDefaults.swift` (`update()`
  `:88–113` — write `auth_source`/`base_url`/`model_service_tier` +
  `env[<ENV_NAME>]`; add the Core env-name map);
  `GaryxMobileCore/GaryxMobileUsageWidgetData.swift`
  (`GaryxProviderUsageDisplayModel` `:481` / `make` `:502` — surface
  `plan`/`session`/`stale`); `GaryxMobileSettingsViews.swift`
  (`GaryxModelProviderDefaultsSheet` `:668` — Authentication/Endpoint sections +
  read-only host section, using `GaryxFormMenuRow` from P2);
  `GaryxMobileModel+AgentsWorkspaces.swift` (`updateModelProviderDefaults` `:941`
  — fetch authoritative `agents.<key>` first, build merged `env`). Note the
  deep-merge / no-delete caveat (§6.2) against `saveGatewaySettings`
  (`GaryxGatewayClient.swift:295`).
- **Verification:** SwiftPM Core tests (the additive `update()` patch shape for
  native providers incl. `env[<ENV_NAME>]`; the extended display model surfacing
  plan/session/stale) — these are the headless core; plus `xcodebuild` and a
  simulator pass of the editor + inline usage. `xcodegen generate` + commit
  `.pbxproj` if files are added.

### P5 — iOS Quota hero + widget deep link

- **Files:** lift `GaryxUsageSpeedometer` + `GaryxUsageGaugeArc`
  (`GaryxCodingUsageWidget.swift:104/:84`) into a shared source compiled into
  **both** App + Widget targets, driven by Core `GaryxUsageGaugeModel` (`:311`);
  add the Quota hero to `GaryxSettingsProviderContent`
  (`GaryxMobileSettingsViews.swift:476`); add the root
  `.widgetURL(GaryxMobileRouteLink.make(.settings(.provider)))` to
  `GaryxCodingUsageWidgetView` for all families. No route change (no `?provider=`,
  D7).
- **Verification:** SwiftPM Core route round-trip test (`.settings(.provider)`
  make/parse); `xcodebuild` (App + Widget); a widget preview/snapshot check that
  every family carries the `widgetURL`; a cold-start check that the tap opens the
  Provider tab. `xcodegen generate` + commit `.pbxproj` for the shared gauge file
  added to two targets.

---

## 12. Risks & trade-offs

- **SwiftUI interaction is not headless-testable** — the form-row fix (P2) and
  the iOS editor (P4) lean on simulator verification; mitigate by landing
  primitives first (zero-regression Phase A) and Core-testing everything
  factorable (patch shape, display model, route, severity/countdown).
- **iOS host/remote mental model** — editing "auth" on a phone mutates the remote
  gateway host's config. The "Managed on the Mac app" framing (D1) keeps
  host-only fields off the phone; plaintext key editing is intentional (D1) and
  scoped to the native providers.
- **Deep-merge cannot delete** (§6.2) — iOS key/field editing is set/update-first;
  true env-key removal stays a Mac action. Called out in the sheet copy so users
  aren't surprised.
- **Mac full-document save** — the Antigravity fix (P3) slots into the existing
  optimistic gateway-draft flow, not a new write path.
- **Widget budget** — the widget only renders the app-warmed snapshot; the
  `.widgetURL` is render-only and adds no fetch (D7).
- **Blast radius of `GaryxFormRow`** — five subclasses render through it; the
  opt-in `onTap` (default `nil`) keeps the change regression-free.
- **`.pbxproj` sync** — P2/P4/P5 may add files/targets; `xcodegen generate` must
  run and the `.pbxproj` be committed, or TestFlight CI (which does not run
  xcodegen) and the app build will miss them even when `swift test` is green.
