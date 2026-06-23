# design-sync notes — garyx-desktop UI

The synced "design system" is the shadcn-style primitive library under
`src/renderer/src/components/ui/` (plus, later, a few app-shell composites).
This repo is an Electron app, not a published library, so the sync runs in
**synth/explicit-entry mode**.

## How the build is wired (why config looks the way it does)
- **`pkg` is `garyx-desktop`** but `package.json` `main` points at the Electron
  main process (`out/main/index.js`). We must NOT let the adapter resolve that.
  So `cfg.entry` points at a hand-written barrel **`.design-sync/entry.tsx`**
  that `export *`s every `components/ui/*` module. PKG_DIR resolves to the repo
  root by walking up from the barrel to the named `package.json`.
- **Component list** comes from `cfg.componentSrcMap` (explicit enumeration) —
  there is no shipped `.d.ts` tree, so discovery can't be automatic. To add a
  component, add it to `entry.tsx` (`export *`) AND to `componentSrcMap`.
- **CSS is Tailwind v4**, compiled deterministically by `cfg.buildCmd`:
  `.ds-sync/node_modules/.bin/tailwindcss -i src/renderer/src/styles.css -o .design-sync/compiled.css`
  `cfg.cssEntry` points at that compiled file. `compiled.css` is **gitignored**
  (regenerate via buildCmd before each sync). The `@tailwindcss/cli` is NOT a
  repo dep — it is installed into `.ds-sync/node_modules` alongside esbuild/
  ts-morph/@types/react.
- **Fonts**: the DS uses the macOS system stack on purpose (SF Pro / SF Mono).
  `cfg.runtimeFontPrefixes` suppresses `[FONT_MISSING]` for SF Pro, SF Mono,
  IBM Plex, Segoe UI, Menlo (all host/system-provided or stack fallbacks). No
  font files ship. This is intentional, not a substitute.

## Preview authoring conventions (proven working)
- Previews import DS components from the specifier `'garyx-desktop'` (shimmed to
  `window.GaryxUI`). lucide-react / @tabler/icons-react bundle fine.
- Use inline `style={{}}` for ALL layout glue — component classes are
  precompiled, but arbitrary layout utility classes you add may not be.
- Checkbox indeterminate: `checked="indeterminate"` (shows MinusIcon).
- Field error styling: `data-invalid="true"` on `Field` + `aria-invalid` on input.
- Label disabled dimming uses `group`/`peer` siblings.
- Separator vertical: needs explicit height from parent (`data-[orientation=vertical]`
  collapses to 0) — give it `style={{ height: N }}`.
- Avatar: NO network in headless chromium — use `AvatarFallback` initials (+ an
  icon fallback), never rely on remote `AvatarImage src`.
- Select: author CLOSED (defaultValue / placeholder) — the radix dropdown portals
  and would escape the card. Closed-trigger-only is correct and counts complete.
- Overlays (Dialog/DropdownMenu/Popover/FloatingActionMenuContent): render with
  `defaultOpen` and set `cfg.overrides.<Name> = {cardMode:"single", viewport:"WxH"}`
  so the open/portaled content renders inside the card.

## Known render warns (triaged legitimate)
- `[TOKENS_MISSING]` ~10 vars (`--sdm-*`, `--streamdown-*`, `--tw`, `--agent-bg`,
  several `--color-token-*`): these come from streamdown / app-shell code that the
  Tailwind content scan sees, NOT from the synced ui primitives. Set at runtime or
  unused by our components. Non-blocking.

## Re-sync risks (watch-list for the next run)
- `compiled.css` is gitignored — a re-sync MUST run `cfg.buildCmd` first or
  `cssEntry` won't exist on a fresh clone (also reinstall `.ds-sync` deps then).
- The repo working tree is sometimes edited concurrently (other sessions). The
  sync only writes under `.design-sync/`, `ds-bundle/`, `.ds-sync/` — leave app
  `src/` changes alone.
- `componentSrcMap` + `entry.tsx` are hand-maintained; new ui components are NOT
  auto-discovered. Add them to both.
- App-shell composites (planned addition) pull i18n/gateway context — some may
  need `cfg.provider` or land on floor cards.
