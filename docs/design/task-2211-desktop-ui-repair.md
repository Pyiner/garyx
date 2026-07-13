# TASK-2211 Desktop UI Repair Design

Status: approved by design review #TASK-2215

## Scope

Repair the active Mac workspace picker, remove local color literals from
`GatewaySettingsPanel`, centralize provider labels in the existing
`providerLabel` helper, resolve the audited orphan class hooks, and give the
shared popover primitive a safe stacking fallback. The implementation is
limited to `desktop/garyx-desktop` plus this design record.

This work does not change transcript rendering, Gateway mirror state, routing,
or any server `render_state` contract.

## Reproduction Baseline

The baseline was captured from commit
`e734a243c96c3c5362494200a78826a9412fd380` after running
`desktop/garyx-desktop/npm run dist:dir`, installing the resulting package,
quitting the stale installed process, and reopening `/Applications/Garyx.app`.

The target dialog was opened deterministically by navigating the installed app
to `#/new`, clicking `.new-thread-workspace-trigger`, waiting for
`.workspace-picker-dialog`, and capturing that dialog through CDP on port
`39222`.

- Worktree-local evidence (intentionally untracked because it contains real
  local app state): `.garyx-evidence/task-2211/before-workspace-picker.png`
- Dimensions: `840 x 788`
- SHA-256:
  `21f4b702c37d8c2ec8cd12ce526966045f3ebb6e816198e383bbe4258427daf2`
- The screenshot shows the broken layout: the search icon occupies its own
  line, workspace buttons shrink to their inline content, names and paths run
  together, and the selected check mark is placed as a separate block.

CDP computed-style measurements make the visual failure deterministic:

| Element | Broken baseline |
| --- | --- |
| `.workspace-picker-search` | `display: block`, height `55px` |
| `.workspace-picker-list` | `display: block`, `overflow: visible` |
| `.workspace-picker-row` | `display: inline-block`, width about `216px` |
| `.workspace-picker-path` | inline, `overflow: visible`, no ellipsis |
| `.workspace-picker-check` | `display: block`, static position |

Static reproduction also confirms:

- All ten audited workspace-picker hooks have zero rules in `styles/*.css`.
- All nine audited orphan hooks and `.popover-content` have zero rules.
- `GatewaySettingsPanel.tsx` contains exactly 19 hard-coded hex occurrences.
- `WorkspaceSelectDialog` / `WorkspacePathPicker` has seven external TSX
  consumers, so the missing recipe is on live shared code.
- `providerOptionLabel` and `providerLabel` duplicate the same mapping. The
  wire provider enum currently has four values, while `gemini` exists in the
  adjacent icon-key/runtime-string domain; the parent task requires a
  defensive explicit `Gemini` label so a future or untyped value cannot fall
  through to `Claude`.

Git history identifies the root cause. The picker recipe lived at the end of
the unrelated `styles/workflows.css`; commit `c3499f4db` correctly removed the
workflow feature and stylesheet, but the still-live picker component retained
its class hooks. The repair must move the recipe to its real owner rather than
restore the removed workflow stylesheet.

## Design

### 1. Restore the workspace picker in `styles/dialogs.css`

Restore the previously working layout recipe under the always-loaded dialog
owner, adapted to current semantic tokens:

- The compact dialog becomes a vertical flex layout with the existing compact
  dialog spacing and close-button geometry.
- The search wrapper is a positioned flex row; its icon is absolutely placed
  and the shared input receives matching left padding.
- The list is a bounded vertical scroller (`max-height: 320px`).
- Rows are full-width flex rows with `min-width: 0`, token-based hover/selected
  fill, disabled state, and left-aligned text.
- Workspace names and paths independently ellipsize; the path flexes into the
  remaining width and aligns right.
- Icons, empty text, primary text, and the footer divider use
  `--color-token-*` variables rather than the historical raw colors.

Do not restore `.workspace-picker-trigger`. The only occurrence is an extra
class on `NewThreadEmptyState`; `button.new-thread-workspace-trigger` already
has its base recipe in `dialogs.css` and contextual rules in
`channel-plugins.css`. Remove the redundant token from the component.

Add a focused source contract test that pins the dialog owner import, the nine
required picker selectors, and the layout declarations whose absence caused
this P0. The same test will ensure the retired trigger token does not return.

### 2. Replace `GatewaySettingsPanel` hex utilities with semantic utilities

Use the shadcn/Tailwind semantic utilities already backed by `theme.css` and
Garyx tokens:

| Current intent | Replacement |
| --- | --- |
| primary foreground | `text-foreground` |
| secondary/quiet foreground | `text-secondary-foreground` or `text-muted-foreground` |
| white control surface | `bg-background` or the component variant default |
| quiet grouped surface | `bg-muted` / `bg-secondary` |
| border | `border-border` or the component variant default |
| primary button colors | shared default `Button` variant |
| outline button colors | shared `outline` `Button` variant |

Keep geometry-only utilities such as `rounded-xl`, height, padding, and
`shadow-none`. Do not create a panel-specific color abstraction. A source
contract assertion will require zero hex literals in
`GatewaySettingsPanel.tsx`.

### 3. Make the existing `providerLabel` the single audited mapping

Keep `app-shell/components/agents-hub-helpers.ts::providerLabel` as the single
presentation helper; do not add another module or table.

- Make the helper explicit for `claude_code`, `codex_app_server`,
  `antigravity`, `traex`, and defensive runtime/icon-key string `gemini`, whose
  expected label is `Gemini`. Keep the local signature widening in this helper;
  do not widen `DesktopApiProviderType` or the wire provider enum.
- Delete `ComposerForm::providerOptionLabel`, import the shared helper under an
  alias, and call it for `composerProviderType`. The alias avoids colliding
  with Composer's existing local `providerLabel` display variable.
- Replace only the fallback label expression in `AppShell` with
  `providerLabel("claude_code")`, plus the required import. No other AppShell
  behavior or layout code changes.
- Add a pure unit test for all five mappings and source assertions that the
  Composer duplicate and AppShell literal fallback stay removed.

This prevents a defensive Gemini value from being mislabeled without
broadening the desktop API provider contract or changing agent creation
choices. Other label tables in `RateLimitBanner`, `ProviderSettingsPanel`, and
`NewThreadEmptyState` consume different domains (usage IDs, settings rows, and
resume hints) and remain intentionally out of scope. Source assertions must
target only the audited helper and consumers, not same-named helpers in those
other domains.

### 4. Resolve every audited orphan hook

| Hook | Decision | Reason |
| --- | --- | --- |
| `gateway-add-dialog` | delete token | Generic compact `DialogContent` owns geometry; `gateway-add-fields` owns body layout. |
| `browser-side-panel-loading` | keep and style | It is the live `Suspense` fallback. Add a centered token-colored spinner in `browser.css`, reusing the existing `browser-tab-spin` keyframe. |
| `tasks-agent-menu-item` | delete token | Shared floating menu item owns the row; `AgentOptionRow` owns avatar/text layout. |
| `skills-create-field-group` | delete token | Shared `FieldGroup` utilities already own flex direction, width, and gap. |
| `settings-update-row` | delete token | `SettingsControlRow` and its update child classes own the complete layout. |
| `composer-bot-submenu` | delete token | Shared submenu content must retain the single `menus.css` surface recipe. |
| `composer-menu-item` | delete token | Shared floating menu item owns the row recipe. |
| `composer-attachments` | delete token | Keep the live `px-3 pt-2` utilities; the semantic class has no independent behavior. |
| `bot-console-empty-card` | delete token | Shared `UICard`, header, and content recipes fully style the empty card. |

The focused style contract test will assert that the eight deleted hooks are
absent and that the retained browser loading hook has a real owner rule.

### 5. Give the shared popover primitive a stacking fallback

Add `.popover-content { z-index: var(--z-app-floating); }` inside the existing
`@layer components` block in `styles/menus.css`. This is only a base stacking
contract; consumers that opt into `.menu-popover-surface` still get the single
shared surface recipe. Pin the fallback in `menu-design-system.test.mjs`.

## Validation

During implementation:

1. Run the focused provider/style/menu/owner contract tests through
   `npm run test:unit -- ...`.
2. Require deterministic source counts:
   - nine picker rules present in `dialogs.css`;
   - eight deleted orphan tokens absent;
   - the browser loading and popover hooks each have an owner rule;
   - zero hex occurrences in `GatewaySettingsPanel.tsx`;
   - no `providerOptionLabel` remains.
3. Run `npm run dist:dir`, quit/reopen the installed package, repeat the exact
   `#/new` + trigger CDP flow, and capture
   `.garyx-evidence/task-2211/after-workspace-picker.png` immediately.
4. Record the after image dimensions and SHA-256. Re-measure computed styles:
   search/list/rows must be flex layouts, rows must fill the dialog width, and
   path overflow must ellipsize.
5. Run the complete required gates:
   - `cd desktop/garyx-desktop && npm run test:unit`
   - `cd desktop/garyx-desktop && npx tsc --noEmit`
   - explicitly confirm the app-shell owner, menu design-system, and
     `gateway-mirror/mirror-contract` tests are in the green full-suite output.

The screenshots remain untracked local review evidence and must never be
staged into the public repository. Because the evidence directory is not
ignored and contains real local paths, stage the task files individually and
verify the staged file list before every commit; never use `git add -A`.

## Implementation Evidence

The packaged repair was rebuilt, installed, restarted, and captured through
the same `#/new` CDP flow:

- Worktree-local evidence:
  `.garyx-evidence/task-2211/after-workspace-picker.png`
- Dimensions: `840 x 752`
- SHA-256:
  `27ec26961fbcf3c6e82264143915ee1b36bb2b22d191552461e241700018f6b7`
- Computed layout: dialog/search/list/row are flex; the row is `402px` wide
  inside the `420px` compact dialog; the path has `min-width: 0`, hidden
  overflow, `text-overflow: ellipsis`, and `white-space: nowrap`; the selected
  check is a `15px` non-shrinking flex item.

## Rebase and Integration

After code review passes, fetch the latest `origin/main`, inspect overlapping
upstream changes, and rebase. Reconcile `AppShell.tsx` from upstream semantics;
the task-owned change there is limited to the provider-label import and the
single fallback label expression. Re-run the required validation after the
rebase, commit any reconciliation separately if needed, then merge and push to
remote `main`.
